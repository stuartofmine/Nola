package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.core.AgentCoreNative
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import com.nola.domain.model.UploadedContent
import com.nola.domain.model.McpServerConfig

class AndroidApplicationRuntimeRealEndToEndTest {
    @Test
    fun coreConfigRejectsHostMetadataBeforeAnyStoreMutation() {
        assertTrue(AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "core-config-schema-${System.nanoTime()}").apply { mkdirs() }
        val database = File(root, "product.db")
        val handle = AgentCoreNative.nativeApplicationOpen(database.absolutePath.encodeToByteArray())
        check(handle != 0L)
        try {
            val envelope = buildJsonObject {
                put("protocol_version", AgentCoreNative.nativeApplicationProtocolVersion())
                put("client_id", "strict-config-fixture"); put("request_id", java.util.UUID.randomUUID().toString())
                put("command", buildJsonObject {
                    put("type", "begin_assistant_turn"); put("arguments", buildJsonObject {
                        put("input", buildJsonObject {
                            put("conversation_id", "must-not-exist"); put("conversation_title", "Schema fixture")
                            put("user_message_id", "fixture-user"); put("user_part_id", "fixture-user:text"); put("user_text", "Hello")
                            put("assistant_message_id", "fixture-assistant"); put("created_at_ms", 1)
                            put("core_config", buildJsonObject { put("host_id", "not-a-Core-field") })
                            put("submission", buildJsonObject { put("type", "user_input"); put("delivery", "current_turn")
                                put("items", kotlinx.serialization.json.JsonArray(emptyList()))
                                put("turn_context_items", kotlinx.serialization.json.JsonArray(emptyList()))
                                put("turn_context_components", kotlinx.serialization.json.JsonArray(emptyList()))
                                put("turn_context_platform", buildJsonObject {})
                            })
                        })
                    })
                })
            }
            val bytes = checkNotNull(AgentCoreNative.nativeApplicationSubmit(handle, envelope.toString().encodeToByteArray()))
            val response = kotlinx.serialization.json.Json.parseToJsonElement(bytes.decodeToString()).jsonObject
            assertEquals("invalid_command", response.getValue("error").jsonObject.getValue("code").jsonPrimitive.content)
            assertTrue(response.getValue("error").jsonObject.getValue("message").jsonPrimitive.content.contains("unknown field `host_id`"))
        } finally { AgentCoreNative.nativeApplicationFree(handle) }
        AndroidApplicationRuntime(context, database).use { assertEquals(null, it.loadConversation("must-not-exist")) }
        root.deleteRecursively()
    }

    @Test
    fun applicationCommandSurvivesNativeHandleAndSqliteColdReopen() {
        assertTrue("This test requires the real Rust JNI library", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "canonical-source-e2e-${System.nanoTime()}")
        val database = File(root, "product.db")
        root.mkdirs()
        try {
            val inspected = com.nola.data.repository.DataSourceDocumentInspector.inspect(
                sourceId = "android-real-source", location = "https://example.test/source.json",
                contentType = "application/json", rawContent = "{\"title\":\"Native lifecycle\"}",
                maximumAgeMinutes = 60,
            )
            val draft = com.nola.domain.model.CustomDataSourceConfig(
                id = "android-real-source", name = "Android real source",
                location = "https://example.test/source.json", validationContract = inspected.contract,
                cachedContent = inspected.content,
            )
            AndroidApplicationRuntime(context, database).use { store ->
                val createdAt = System.currentTimeMillis()
                store.createConversation("source-conversation", "Native source override", createdAt, createdAt, null)
                store.saveDataSource(draft)
                val saved = store.listDataSources().single()
                assertTrue(saved.enabled)
                assertEquals(inspected.content, saved.cachedContent)
                assertTrue(saved.canonicalRevision != null)
                val overrides = CanonicalDataSourceOverrides { store.execute(it.jsonObject) }
                assertEquals(CanonicalDataSourceOverride.Inherit, overrides.get("source-conversation", saved.id))
                overrides.set("source-conversation", saved.id, CanonicalDataSourceOverride.Enabled, System.currentTimeMillis())
                assertEquals(CanonicalDataSourceOverride.Enabled, overrides.get("source-conversation", saved.id))
                overrides.set("source-conversation", saved.id, CanonicalDataSourceOverride.Disabled, System.currentTimeMillis())
                assertEquals(CanonicalDataSourceOverride.Inherit, overrides.get("other-conversation", saved.id))
                store.appendDataSourceUsage(com.nola.domain.model.DataSourceUsageRecord(
                    id = "usage-1", sourceId = saved.id, sourceTitle = saved.name,
                    consumer = "context", purpose = "Native persistence test", conversationId = null,
                ))
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                val saved = reopened.listDataSources().single()
                assertEquals("android-real-source", saved.id)
                val overrides = CanonicalDataSourceOverrides { reopened.execute(it.jsonObject) }
                assertEquals(CanonicalDataSourceOverride.Disabled, overrides.get("source-conversation", saved.id))
                overrides.set("source-conversation", saved.id, CanonicalDataSourceOverride.Inherit, System.currentTimeMillis())
                assertEquals(CanonicalDataSourceOverride.Inherit, overrides.get("source-conversation", saved.id))
                assertEquals(inspected.content, saved.cachedContent)
                val expectedRecipe = requireNotNull(inspected.contract.readRecipe)
                val restoredRecipe = requireNotNull(saved.validationContract?.readRecipe)
                assertEquals(expectedRecipe.method, restoredRecipe.method)
                assertEquals(expectedRecipe.operationId, restoredRecipe.operationId)
                assertEquals(expectedRecipe.configurationKeys, restoredRecipe.configurationKeys)
                assertEquals(
                    kotlinx.serialization.json.Json.parseToJsonElement(expectedRecipe.outputSchemaJson),
                    kotlinx.serialization.json.Json.parseToJsonElement(restoredRecipe.outputSchemaJson),
                )
                assertEquals(null, reopened.listDataSourceUsage().single().conversationId)
                reopened.setDataSourceEnabled(saved.id, false)
                assertFalse(reopened.listDataSources().single().enabled)
                assertTrue("An outdated source must not overwrite the new revision",
                    runCatching { reopened.saveDataSource(saved) }.isFailure)
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                assertFalse(reopened.listDataSources().single().enabled)
                reopened.deleteDataSource("android-real-source")
                assertTrue(reopened.listDataSources().isEmpty())
            }
        } finally { root.deleteRecursively() }
    }

    @Test
    fun sourceCacheFailuresDoNotLeavePendingReadsOrDeleteCommittedContent() {
        assertTrue("This test requires real JNI", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "source-fault-e2e-${System.nanoTime()}")
        root.mkdirs()
        try {
            val inspected = com.nola.data.repository.DataSourceDocumentInspector.inspect(
                sourceId = "fault-source", location = "https://example.test/data",
                contentType = "application/json", rawContent = "{\"title\":\"Committed content\"}",
                maximumAgeMinutes = 60,
            )
            val draft = com.nola.domain.model.CustomDataSourceConfig(
                id = "fault-source", name = "Fault fixture", location = "https://example.test/data",
                validationContract = inspected.contract, cachedContent = inspected.content,
            )
            val database = File(root, "product.db")
            val cache = File(root, "cache")
            cache.writeText("Not a directory")
            AndroidApplicationRuntime(context, database).use { runtime ->
                var failAfterReadCommit = false
                val client = CanonicalDataSourceClient { command ->
                    val result = runtime.execute(command)
                    if (failAfterReadCommit && command["command"]!!.jsonObject["type"]!!.jsonPrimitive.content == "record_read") {
                        failAfterReadCommit = false
                        throw IllegalStateException("Injected response loss after commit")
                    }
                    result
                }
                val adapter = AndroidCanonicalDataSources(client, cache)
                assertTrue(runCatching { adapter.save(draft) }.isFailure)
                val afterDirectoryFailure = requireNotNull(client.get(draft.id))
                assertEquals("never_read", afterDirectoryFailure.readState["status"]!!.jsonPrimitive.content)
                assertEquals(kotlinx.serialization.json.JsonNull, afterDirectoryFailure.readState["read_token"])
                assertTrue(cache.delete())
                failAfterReadCommit = true
                assertTrue(runCatching {
                    adapter.save(draft.copy(canonicalRevision = afterDirectoryFailure.revision))
                }.isFailure)
                val afterResponseLoss = requireNotNull(client.get(draft.id))
                assertEquals("ready", afterResponseLoss.readState["status"]!!.jsonPrimitive.content)
                assertEquals(kotlinx.serialization.json.JsonNull, afterResponseLoss.readState["read_token"])
                assertEquals(inspected.content, adapter.list().single().cachedContent)
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                val adapter = AndroidCanonicalDataSources(CanonicalDataSourceClient(reopened::execute), cache)
                assertEquals(inspected.content, adapter.list().single().cachedContent)
            }
        } finally { root.deleteRecursively() }
    }

    @Test
    fun concurrentSourceEditorsCannotOverwriteCommittedConfiguration() {
        assertTrue("This test requires real JNI", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "source-concurrency-e2e-${System.nanoTime()}")
        root.mkdirs()
        val database = File(root, "product.db")
        val cache = File(root, "cache")
        try {
            val inspected = com.nola.data.repository.DataSourceDocumentInspector.inspect(
                sourceId = "concurrent-source",
                location = "https://example.test/data",
                contentType = "application/json",
                rawContent = "{\"title\":\"Original content\"}",
                maximumAgeMinutes = 60
            )
            AndroidApplicationRuntime(context, database).use { firstRuntime ->
                AndroidApplicationRuntime(context, database).use { secondRuntime ->
                    val first = AndroidCanonicalDataSources(
                        CanonicalDataSourceClient(firstRuntime::execute), cache
                    )
                    val second = AndroidCanonicalDataSources(
                        CanonicalDataSourceClient(secondRuntime::execute), cache
                    )
                    first.save(com.nola.domain.model.CustomDataSourceConfig(
                        id = "concurrent-source",
                        name = "Original name",
                        location = "https://example.test/data",
                        validationContract = inspected.contract,
                        cachedContent = inspected.content
                    ))
                    val firstDraft = first.list().single()
                    val staleDraft = second.list().single()
                    first.save(firstDraft.copy(name = "Committed name"))
                    val filesBeforeConflict = cache.listFiles()!!.map { it.name }.toSet()
                    assertTrue(runCatching {
                        second.save(staleDraft.copy(name = "Stale overwrite"))
                    }.isFailure)
                    assertEquals(filesBeforeConflict, cache.listFiles()!!.map { it.name }.toSet())
                    assertEquals("Committed name", second.list().single().name)
                    assertEquals(inspected.content, second.list().single().cachedContent)
                }
            }
            AndroidApplicationRuntime(context, database).use { reopened ->
                val source = AndroidCanonicalDataSources(
                    CanonicalDataSourceClient(reopened::execute), cache
                ).list().single()
                assertEquals("Committed name", source.name)
                assertEquals(inspected.content, source.cachedContent)
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun skillAndMcpSurviveNativeHandleAndSqliteColdReopen() {
        assumeTrue("Rust JNI library must be packaged for this device", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "canonical-extensions-e2e-${System.nanoTime()}")
        val database = File(root, "product.db")
        root.mkdirs()
        AndroidApplicationRuntime(context, database).use { store ->
            store.createUserSkill(UploadedContent(name = "Review", content = "Review carefully"))
            store.createMcpServer(McpServerConfig(name = "Files", url = "stdio://files", serverId = "files"))
        }
        AndroidApplicationRuntime(context, database).use { reopened ->
            assertEquals("Review carefully", reopened.listSkills().single().content)
            assertEquals("files", reopened.listMcpServers().single().serverId)
        }
        root.deleteRecursively()
    }

    @Test
    fun productSettingRevisionAndDeletionSurviveNativeColdReopen() {
        assumeTrue("Rust JNI library must be packaged for this device", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "canonical-setting-e2e-${System.nanoTime()}")
        val database = File(root, "product.db")
        root.mkdirs()

        AndroidApplicationRuntime(context, database).use { store ->
            val saved = store.putProductSetting(
                "assistant.memory.enabled",
                "{\"schemaVersion\":1,\"enabled\":false}",
                100,
            )
            assertEquals(1, saved.revision)
        }
        AndroidApplicationRuntime(context, database).use { reopened ->
            assertTrue(reopened.productSetting("assistant.memory.enabled")?.valueJson?.contains("false") == true)
            val updated = reopened.putProductSetting(
                "assistant.memory.enabled",
                "{\"schemaVersion\":1,\"enabled\":true}",
                200,
            )
            assertEquals(2, updated.revision)
            assertTrue(reopened.deleteProductSetting("assistant.memory.enabled"))
        }
        AndroidApplicationRuntime(context, database).use { final ->
            assertEquals(null, final.productSetting("assistant.memory.enabled"))
        }
        root.deleteRecursively()
    }

    @Test
    fun automationOccurrenceLeaseSurvivesColdReopenWithoutFabricatedResult() {
        assumeTrue("Rust JNI library must be packaged for this device", AgentCoreNative.isApplicationAvailable)
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "canonical-automation-e2e-${System.nanoTime()}")
        val database = File(root, "product.db")
        root.mkdirs()
        val now = 1_800_000_000_000L

        AndroidApplicationRuntime(context, database).use { store ->
            val definition = store.saveAutomationDefinition("android-automation", "Android automation", true,
                buildJsonObject {}, AndroidAutomationSpec("Explain briefly", "android-test-host", "cautious", emptyList(), emptyList()))
            val first = store.enqueueAutomationRun(
                automationId = "android-automation",
                definitionRevision = definition.revision,
                occurrenceKey = "scheduled:$now",
                trigger = "schedule",
                scheduledForMs = now,
            )
            val duplicate = store.enqueueAutomationRun(
                automationId = "android-automation",
                definitionRevision = definition.revision,
                occurrenceKey = "scheduled:$now",
                trigger = "schedule",
                scheduledForMs = now,
            )
            assertEquals(first.id, duplicate.id)
            assertTrue(store.claimAutomationRun(first.id, "android-a", "lease-a", now, now + 60_000))
            assertFalse(store.claimAutomationRun(first.id, "android-b", "lease-b", now + 1, now + 60_001))
            assertEquals(definition.revision, first.definitionRevision)
            // A lease is not execution evidence. Current protocol reconciliation
            // must not manufacture success without an admitted execution.
            assertEquals(null, store.reconcileAutomationExecution(first.id))
            assertEquals(kotlinx.serialization.json.JsonNull,
                store.observeAutomationExecution(first.id)["binding"])
        }

        AndroidApplicationRuntime(context, database).use { reopened ->
            val run = reopened.loadAutomationRun("automation-run-missing")
            assertEquals(null, run)
            val persisted = reopened.execute(buildJsonObject {
                put("type", "automation")
                put("command", buildJsonObject {
                    put("type", "run")
                    put("command", buildJsonObject {
                        put("type", "list")
                        put("automation_id", "android-automation")
                        put("limit", 10)
                    })
                })
            }).jsonArray.single().jsonObject
            assertEquals("running", persisted["status"]?.jsonPrimitive?.content)
            assertEquals(
                kotlinx.serialization.json.JsonNull,
                persisted["result_json"],
            )
        }
        root.deleteRecursively()
    }
}
