package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.core.AgentCoreNative
import com.nola.data.repository.ConversationRepositoryImpl
import com.nola.data.repository.DataSourceDocumentInspector
import com.nola.domain.model.CustomDataSourceConfig
import java.io.File
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidConversationDataSourcesRealTest {
    @Test
    fun successfulReadRefreshesContentFreshnessWithoutRevalidatingConfiguration() {
        assertTrue("Real JNI is required", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "read-freshness-${System.nanoTime()}")
        root.mkdirs()
        try {
            AndroidApplicationRuntime(context, File(root, "product.db")).use { runtime ->
                val now = System.currentTimeMillis()
                val inspected = DataSourceDocumentInspector.inspect(
                    sourceId = "freshness-source", location = "https://example.test/data",
                    contentType = "application/json", rawContent = "{\"signal\":7429}",
                    maximumAgeMinutes = 1, now = now - 7_200_000
                )
                runtime.saveDataSource(CustomDataSourceConfig(
                    id = "freshness-source", name = "Freshness source", location = "https://example.test/data",
                    maximumAgeMinutes = 1, validationContract = inspected.contract,
                    cachedContent = inspected.content
                ))
                val sources = CanonicalDataSourceClient(runtime::execute)
                val overrides = CanonicalDataSourceOverrides { runtime.execute(it.jsonObject) }
                val initial = requireNotNull(sources.get("freshness-source"))
                assertTrue(overrides.resolve("freshness-conversation", now).single().shouldRefresh)
                val originalContract = initial.validation.contract
                val originalReference = requireNotNull(initial.readState["content_reference"])
                val pending = sources.requestRefresh(initial.id, initial.revision, "Refresh content", now)
                val token = pending.readState["read_token"]!!.jsonPrimitive.content
                val committed = sources.recordRead(initial.id, token, buildJsonObject {
                    put("status", "succeeded")
                    put("content_reference", originalReference)
                    put("observed_at_ms", now)
                    put("schema_version", 2)
                    put("record_count", 1)
                }, now)
                val previousContract = requireNotNull(originalContract)
                val updatedContract = requireNotNull(committed.validation.contract)
                assertEquals(previousContract.filterKeys { it !in setOf("observedAt", "validationDigest") },
                    updatedContract.filterKeys { it !in setOf("observedAt", "validationDigest") })
                assertEquals(java.time.Instant.ofEpochMilli(now).toString(),
                    updatedContract["observedAt"]!!.jsonPrimitive.content)
                assertTrue(previousContract["validationDigest"] != updatedContract["validationDigest"])
                val selection = overrides.resolve("freshness-conversation", now + 1).single()
                assertTrue("A fresh read must be usable without rewriting validation: ${selection.reason}", selection.usable)
                assertFalse(selection.shouldRefresh)
                assertEquals(committed.revision, selection.sourceRevision)
                val expired = overrides.resolve("freshness-conversation", now + 120_000).single()
                assertFalse(expired.usable)
                assertTrue(expired.shouldRefresh)
            }
        } finally { root.deleteRecursively() }
    }

    @Test
    fun conversationSelectionUsesCanonicalOverridesAcrossColdReopen() = runBlocking {
        assertTrue("Real JNI is required", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "conversation-sources-${System.nanoTime()}")
        root.mkdirs()
        val database = File(root, "product.db")
        val catalogId = "data_source:conversation-source"
        var firstId = ""
        var secondId = ""
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val inspected = DataSourceDocumentInspector.inspect(
                    sourceId = "conversation-source", location = "https://example.test/data",
                    contentType = "application/json", rawContent = "{\"signal\":7429}",
                    maximumAgeMinutes = 60
                )
                runtime.saveDataSource(CustomDataSourceConfig(
                    id = "conversation-source", name = "Conversation source",
                    location = "https://example.test/data",
                    validationContract = inspected.contract, cachedContent = inspected.content
                ))
                val repository = ConversationRepositoryImpl(runtime, com.nola.data.local.AndroidAssistantJournal(runtime), com.nola.llm.local.DeviceModelStore(context))
                firstId = repository.createConversation("First").id
                secondId = repository.createConversation("Second").id
                val overrides = CanonicalDataSourceOverrides { runtime.execute(it.jsonObject) }
                assertTrue(repository.getConversation(firstId)!!.dataSourceIds.contains(catalogId))
                assertTrue(overrides.resolve(firstId, System.currentTimeMillis()).single().usable)
                repository.setDataSourceEnabled(firstId, catalogId, false)
                assertFalse(repository.getConversation(firstId)!!.dataSourceIds.contains(catalogId))
                assertFalse(overrides.resolve(firstId, System.currentTimeMillis()).single().usable)
                assertTrue(repository.getConversation(secondId)!!.dataSourceIds.contains(catalogId))
                assertTrue(overrides.resolve(secondId, System.currentTimeMillis()).single().usable)
                repository.setDataSourceEnabled(firstId, "platform:health", true)
                val settings = Json.parseToJsonElement(runtime.loadConversation(firstId)!!.settingsJson!!).jsonObject
                val storedIds = settings["dataSourceIds"]!!.jsonArray.map { it.jsonPrimitive.content }
                assertEquals(listOf("platform:health"), storedIds)
                assertEquals(CanonicalDataSourceOverride.Disabled, overrides.get(firstId, "conversation-source"))
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val repository = ConversationRepositoryImpl(runtime, com.nola.data.local.AndroidAssistantJournal(runtime), com.nola.llm.local.DeviceModelStore(context))
                val overrides = CanonicalDataSourceOverrides { runtime.execute(it.jsonObject) }
                assertFalse(repository.getConversation(firstId)!!.dataSourceIds.contains(catalogId))
                assertTrue(repository.getConversation(firstId)!!.dataSourceIds.contains("platform:health"))
                assertTrue(repository.getConversation(secondId)!!.dataSourceIds.contains(catalogId))
                repository.setDataSourceEnabled(firstId, catalogId, null)
                assertEquals(CanonicalDataSourceOverride.Inherit, overrides.get(firstId, "conversation-source"))
                assertTrue(repository.getConversation(firstId)!!.dataSourceIds.contains(catalogId))
                assertTrue(overrides.resolve(firstId, System.currentTimeMillis()).single().usable)
                runtime.setDataSourceEnabled("conversation-source", false)
                assertFalse(overrides.resolve(secondId, System.currentTimeMillis()).single().enabled)
                repository.setDataSourceEnabled(firstId, catalogId, true)
                val explicitlyEnabled = overrides.resolve(firstId, System.currentTimeMillis()).single()
                assertTrue(explicitlyEnabled.enabled)
                assertTrue("Explicit conversation enable must override the default: ${explicitlyEnabled.reason}",
                    explicitlyEnabled.usable)
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
