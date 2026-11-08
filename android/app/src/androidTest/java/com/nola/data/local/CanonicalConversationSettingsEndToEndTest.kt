package com.nola.data.local

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import kotlinx.serialization.json.*

@RunWith(AndroidJUnit4::class)
class CanonicalConversationSettingsEndToEndTest {
    @Test
    fun settingsSurviveRealJniSqliteColdReopen() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val directory = File(context.cacheDir, "canonical-settings-${UUID.randomUUID()}").apply { mkdirs() }
        java.nio.file.Files.setPosixFilePermissions(directory.toPath(), java.nio.file.attribute.PosixFilePermissions.fromString("rwx------"))
        val database = File(directory, "application.db")
        val initial = """{"version":1,"executionBackend":"nola","dataSourceIds":["health"]}"""
        AndroidApplicationRuntime(context, database).use { store ->
            for ((tier, effort) in listOf("concise" to "low", "balanced" to "low", "detailed" to "high")) {
                val resolved = store.executeQuery(buildJsonObject {
                    put("type", "resolve_model_reasoning")
                    put("declaration", buildJsonObject {
                        put("control", "effort")
                        put("efforts", buildJsonArray { add("low"); add("high") })
                    })
                    put("protocol", "siwc_responses"); put("output_limit", 16384)
                    put("supports_reasoning", true); put("requested", JsonNull); put("tier", tier)
                }).jsonObject
                assertEquals(effort, resolved.getValue("selection").jsonObject.getValue("value").jsonPrimitive.content)
            }
            val profile = CanonicalModelProfile(
                id = "tier-fixture", displayName = "Tier fixture", provider = "openai-compatible", modelName = "tier-fixture",
                runtime = "remote", sourceJson = """{"baseURL":"https://fixture.invalid/v1","wireProtocol":"responses","capabilities":["chat"],"credentialRequired":false,"executionCapabilities":{"toolCalls":false,"imageInput":false,"reasoning":true,"samplingParameters":true},"reasoning":{"control":"effort","efforts":["low","high"]}}""",
                parametersJson = "{}", selected = true, enabled = true, revision = 0,
                createdAtMs = 1, updatedAtMs = 1, deletedAtMs = null)
            store.upsertModelProfile(profile)
            val editor = com.nola.data.local.datastore.ModelParametersStore(store)
            val snapshot = editor.parameterSnapshot(profile.id)
            assertEquals(listOf("low", "high"), snapshot.reasoningEfforts)
            kotlinx.coroutines.runBlocking { editor.saveParameters(snapshot, snapshot.parameters.copy(reasoningTier = "detailed")) }
            val frozen = store.executeQuery(buildJsonObject {
                put("type", "resolve_remote_model_profile"); put("profile_id", profile.id)
                put("expected_revision", requireNotNull(store.modelProfile(profile.id)).revision)
                put("request", buildJsonObject {})
            }).jsonObject.getValue("resolved").jsonObject.getValue("runtime").jsonObject
            assertEquals("high", frozen.getValue("reasoning_selection").jsonObject.getValue("value").jsonPrimitive.content)
            val override = store.executeQuery(buildJsonObject {
                put("type", "resolve_remote_model_profile"); put("profile_id", profile.id)
                put("expected_revision", requireNotNull(store.modelProfile(profile.id)).revision)
                put("request", buildJsonObject { put("reasoning_tier", "concise") })
            }).jsonObject.getValue("resolved").jsonObject.getValue("runtime").jsonObject
            assertEquals("low", override.getValue("reasoning_selection").jsonObject.getValue("value").jsonPrimitive.content)
            store.createConversation(
                id = "settings-e2e",
                title = "Settings",
                createdAtMs = 1_787_000_000_000,
                updatedAtMs = 1_787_000_000_000,
                settingsJson = initial,
            )
            store.patchConversationConfiguration(
                id = "settings-e2e",
                expectedRevision = requireNotNull(store.loadConversation("settings-e2e")?.configurationRevision),
                patchJson = """{"executionBackend":"codex","externalExecutionSessions":{"codex":"thread-42"},"dataSourceIds":["health","calendar"]}""",
            ).requireCommitted()
        }
        AndroidApplicationRuntime(context, database).use { reopened ->
            assertEquals("detailed", com.nola.data.local.datastore.ModelParametersStore(reopened).parameterSnapshot("tier-fixture").parameters.reasoningTier)
            val restored = reopened.loadConversation("settings-e2e")
            assertEquals("Settings", restored?.title)
            assertTrue(restored?.settingsJson?.contains("\"executionBackend\":\"codex\"") == true)
            assertTrue(restored?.settingsJson?.contains("\"thread-42\"") == true)
            assertTrue(restored?.settingsJson?.contains("\"calendar\"") == true)
        }
        directory.deleteRecursively()
    }
}
