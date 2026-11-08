package com.nola.data.local

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import java.util.UUID
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ChatOutputTokenField
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ModelConfigurationTransactionEndToEndTest {
    @Test
    fun executionCapabilitiesArePerModelRevisionedAndDoNotLeakToNewModels() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "api-model-execution-${UUID.randomUUID()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val configuration = AndroidModelConfiguration(runtime)
                configuration.save("https://fixture.invalid/v1", "first, second")
                val first = requireNotNull(runtime.modelProfile("first"))
                configuration.updateExecutionCapabilities(first.id, first.revision,
                    ModelExecutionCapabilities(toolCalls = true, samplingParameters = true,
                        chatOutputTokenField = ChatOutputTokenField.MaxCompletionTokens))
                assertTrue(requireNotNull(runtime.modelProfile("first")).sourceJson.contains("\"toolCalls\":true"))
                assertTrue(requireNotNull(runtime.modelProfile("first")).sourceJson.contains("\"chatOutputTokenField\":\"max_completion_tokens\""))
                assertFalse(requireNotNull(runtime.modelProfile("second")).sourceJson.contains("\"toolCalls\":true"))
                assertTrue(runCatching { configuration.updateExecutionCapabilities(first.id, first.revision,
                    ModelExecutionCapabilities(imageInput = true)) }.isFailure)
                configuration.save("https://fixture.invalid/v1", "first, second, third")
                assertFalse(requireNotNull(runtime.modelProfile("third")).sourceJson.contains("\"toolCalls\":true"))
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                assertTrue(requireNotNull(reopened.modelProfile("first")).sourceJson.contains("\"toolCalls\":true"))
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun editingSelectedEndpointPreservesOtherEndpointProfilesAndMetadata() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "api-model-groups-${UUID.randomUUID()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val configuration = AndroidModelConfiguration(runtime)
                val firstSource = """{"schemaVersion":1,"endpointID":"first-endpoint","endpointName":"First","baseURL":"https://first.invalid/v1","endpointEnabled":true,"endpointOrder":0,"modelEnabled":true,"modelOrder":0,"priority":70,"capabilities":["chat"],"wireProtocol":"responses"}"""
                val secondSource = """{"schemaVersion":1,"endpointID":"second-endpoint","endpointName":"Second","baseURL":"https://second.invalid/v1","endpointEnabled":true,"endpointOrder":1,"modelEnabled":true,"modelOrder":0,"priority":40,"capabilities":["chat"],"wireProtocol":"chat_completions"}"""
                runtime.upsertModelProfile(CanonicalModelProfile(
                    id = "first-model", displayName = "First model", provider = "openai-compatible",
                    modelName = "shared-name", runtime = "remote", sourceJson = firstSource,
                    parametersJson = "{}", selected = true, enabled = true, revision = 0,
                    createdAtMs = 1, updatedAtMs = 1,
                ))
                runtime.upsertModelProfile(CanonicalModelProfile(
                    id = "second-model", displayName = "Second model", provider = "openai-compatible",
                    modelName = "shared-name", runtime = "remote", sourceJson = secondSource,
                    parametersJson = "{}", selected = false, enabled = true, revision = 0,
                    createdAtMs = 1, updatedAtMs = 1,
                ))
                runtime.upsertModelProfile(CanonicalModelProfile(
                    id = "disabled-model", displayName = "Disabled model", provider = "openai-compatible",
                    modelName = "dormant", runtime = "remote", sourceJson = firstSource,
                    parametersJson = "{}", selected = false, enabled = false, revision = 0,
                    createdAtMs = 1, updatedAtMs = 1,
                ))
                assertEquals("https://first.invalid/v1", configuration.load()?.endpoint)
                assertEquals("shared-name", configuration.load()?.model)
                assertEquals("responses", configuration.load()?.wireProtocol)
                assertEquals(true, configuration.load()?.supportsDeviceProvider)

                configuration.save("https://first-new.invalid/v1", "shared-name, new-model")
                val foreign = requireNotNull(runtime.modelProfile("second-model"))
                assertEquals(secondSource, foreign.sourceJson)
                assertEquals("Second model", foreign.displayName)
                assertEquals(false, runtime.modelProfile("disabled-model")?.enabled)
                assertEquals(3, configuration.configuredProfiles().size)
                val edited = requireNotNull(runtime.modelProfile("first-model"))
                assertTrue(edited.sourceJson.contains("\"wireProtocol\":\"responses\""))
                assertTrue(edited.sourceJson.contains("\"endpointID\":\"first-endpoint\""))
                val added = requireNotNull(configuration.configuredProfiles().firstOrNull { it.modelName == "new-model" })
                assertTrue(added.sourceJson.contains("\"priority\":50"))
                assertTrue(!added.sourceJson.contains("\"reasoning\""))
                assertEquals("https://first-new.invalid/v1", configuration.load()?.endpoint)

                configuration.save("https://first-new.invalid/v1", "shared-name, new-model", "chat_completions")
                assertEquals("chat_completions", configuration.load()?.wireProtocol)
                assertTrue(requireNotNull(runtime.modelProfile("first-model")).sourceJson.contains("\"wireProtocol\":\"chat_completions\""))
                assertEquals(secondSource, requireNotNull(runtime.modelProfile("second-model")).sourceJson)

                configuration.save("", "")
                assertEquals("second-model", configuration.configuredProfiles().single().id)
                assertTrue(configuration.configuredProfiles().single().selected)
                configuration.clearAll()
                assertTrue(configuration.configuredProfiles().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun apiConfigurationIsOneCanonicalProjectionAcrossReplaceAndReopen() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val root = File(context.cacheDir, "api-model-transaction-${UUID.randomUUID()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val configuration = AndroidModelConfiguration(runtime)
                assertNull(configuration.load())
                configuration.save("https://one.invalid/v1", "first, second")
                assertEquals(2, runtime.listModelProfiles().size)
                runtime.resetModelParameters(listOf("second"), "{\"temperature\":0.31}", 20)
                configuration.save("https://two.invalid/v1", "second; third; second")
                assertNull(runtime.modelProfile("first"))
                assertEquals("{\"temperature\":0.31}", runtime.modelProfile("second")?.parametersJson)
                assertEquals(1, runtime.listModelProfiles().count { it.selected })
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val configuration = AndroidModelConfiguration(runtime)
                assertEquals("https://two.invalid/v1", configuration.load()?.endpoint)
                assertEquals("second, third", configuration.load()?.model)
                assertTrue(configuration.load()?.supportsDeviceProvider == true)
                var rejected = false
                try {
                    configuration.save("", "third")
                } catch (_: IllegalArgumentException) {
                    rejected = true
                }
                assertTrue(rejected)
                assertEquals("second, third", configuration.load()?.model)
                configuration.save("", "")
                assertNull(configuration.load())
                assertTrue(runtime.listModelProfiles().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
