package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.domain.model.StructuredMemoryRecord
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import kotlinx.serialization.json.*

class AndroidMemoryCommandEndToEndTest {
    @Test
    fun currentOwnerAwareSelectionAbiIsAvailable() {
        assertTrue(com.nola.core.AgentCoreMemorySelection.isAvailable)
    }

    @Test
    fun textEditorPreservesDistinctCanonicalNamespacesWithIdenticalDisplayValues() = kotlinx.coroutines.runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "memory-namespace-${System.nanoTime()}")
        val database = File(root, "product.db")
        var expected: JsonArray? = null
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                runtime.upsertMemory(StructuredMemoryRecord(
                    id = "personal-native", namespace = "personal", kind = "reference",
                    owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
                    content = "Same visible content", confidence = 0.8,
                    provenanceKind = "explicit_user", sourceReference = "fixture://memory"
                ))
                val original = runtime.memorySnapshot().canonical.single().jsonObject.getValue("record").jsonObject
                val other = JsonObject(original.toMutableMap().apply {
                    put("id", JsonPrimitive("personal-custom"))
                    put("namespace", buildJsonObject { put("kind", "custom"); put("name", "personal") })
                })
                runtime.submitMutation(buildJsonObject {
                    put("type", "memory")
                    put("command", buildJsonObject {
                        put("type", "commit"); put("intent", "explicit_user")
                        put("input", buildJsonObject {
                            put("record", other); put("quality", JsonNull); put("proposed_at_ms", 1)
                        })
                    })
                })
                expected = runtime.memorySnapshot().canonical
                assertEquals(2, expected!!.size)
                val store = com.nola.data.local.datastore.MemoryProfileDataStore(
                    context, com.nola.data.local.security.SecureTokenStore(context), runtime
                )
                val saved = store.saveMemoryText(store.getSnapshot().copy(longTermMemory = listOf("Editable memory")))
                store.saveMemoryText(saved.copy(longTermMemory = emptyList()))
                assertEquals(expected, runtime.memorySnapshot().canonical)
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertEquals(expected, runtime.memorySnapshot().canonical)
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun memoryEditorUsesCommittedSnapshotInsteadOfLaterWriterState() = kotlinx.coroutines.runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "memory-editor-${System.nanoTime()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val store = com.nola.data.local.datastore.MemoryProfileDataStore(
                    context, com.nola.data.local.security.SecureTokenStore(context), runtime
                )
                val committed = store.saveMemoryText(
                    store.getSnapshot().copy(longTermMemory = listOf("First editor memory"))
                )
                assertEquals(committed.canonicalMemorySnapshot, runtime.memorySnapshot().canonical)
                val secondCommit = store.saveMemoryText(
                    committed.copy(userPreferences = listOf("Prefer short answers"))
                )
                assertEquals(listOf("Prefer short answers"), secondCommit.userPreferences)
                val anotherEditor = store.saveMemoryText(
                    store.getSnapshot().copy(longTermMemory = listOf("Other editor revision"))
                )
                assertEquals(listOf("First editor memory"), secondCommit.longTermMemory)
                val failure = runCatching {
                    store.saveMemoryText(secondCommit.copy(userPreferences = listOf("Stale edit")))
                }.exceptionOrNull()
                assertTrue(failure?.message.orEmpty().contains("memory_snapshot_conflict"))
                val expected = requireNotNull(anotherEditor.canonicalMemorySnapshot)
                val actual = runtime.memorySnapshot().canonical
                fun byId(records: kotlinx.serialization.json.JsonArray) = records.associateBy {
                    val envelope = it as kotlinx.serialization.json.JsonObject
                    val record = envelope.getValue("record") as kotlinx.serialization.json.JsonObject
                    record.getValue("id")
                }
                assertEquals(expected.size, byId(expected).size)
                assertEquals(actual.size, byId(actual).size)
                assertEquals(byId(expected), byId(actual))
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun profileReplacementRejectsStaleEditorAndPersistsExplicitRemoval() = kotlinx.coroutines.runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "memory-profile-${System.nanoTime()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val store = com.nola.data.local.datastore.MemoryProfileDataStore(
                    context, com.nola.data.local.security.SecureTokenStore(context), runtime
                )
                val initial = store.getSnapshot()
                store.saveMemoryText(initial.copy(longTermMemory = listOf("Original durable memory")))
                val stale = store.getSnapshot()
                store.saveMemoryText(stale.copy(longTermMemory = stale.longTermMemory + "Added by another editor"))
                val failure = runCatching { store.saveMemoryText(stale) }.exceptionOrNull()
                assertNotNull(failure)
                assertTrue(failure!!.message.orEmpty().contains("memory_snapshot_conflict"))
                val current = store.getSnapshot()
                assertTrue(current.longTermMemory.contains("Added by another editor"))
                store.saveMemoryText(current.copy(longTermMemory = emptyList()))
                assertTrue(store.getSnapshot().memoryRecords.isEmpty())
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertTrue(runtime.listMemories().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun semanticMemoryCommitReadAndDeleteSurviveJniAndReopen() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "memory-command-${System.nanoTime()}")
        val database = File(root, "product.db")
        val input = StructuredMemoryRecord(
            id = "preference", namespace = "user_preferences", kind = "preference",
            owner = kotlinx.serialization.json.buildJsonObject { put("kind", kotlinx.serialization.json.JsonPrimitive("profile")); put("profile_id", kotlinx.serialization.json.JsonPrimitive("local")) },
            content = "Prefer concise technical answers", confidence = 0.8,
            provenanceKind = "explicit_user", sourceReference = "profile://android",
            observedAt = 1_700_000_000_000, retention = "expires_at",
            expiresAt = 4_000_000_000_000, tags = listOf("language", "style"),
        )
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val saved = runtime.upsertMemory(input)
                assertEquals(input.content, saved.content)
                assertEquals(input.namespace, saved.namespace)
                assertEquals(input.observedAt, saved.observedAt)
                assertEquals(input.expiresAt, saved.expiresAt)
                assertEquals(input.tags, saved.tags)
                assertEquals(saved, runtime.listMemories().single())
                assertTrue(runCatching { runtime.upsertMemory(input.copy(id = "invalid", kind = "semantic")) }.isFailure)
                assertTrue(runCatching { runtime.upsertMemory(input.copy(id = "automatic", provenanceKind = "agent_inference")) }.isFailure)
                assertEquals(1, runtime.listMemories().size)
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val saved = runtime.listMemories().single()
                assertEquals(input.content, saved.content)
                assertEquals(input.retention, saved.retention)
                runtime.deleteMemory(saved.id)
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertTrue(runtime.listMemories().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun savingMemoryFromOldCapabilityListPreservesSkillsMcpAndDocuments() = kotlinx.coroutines.runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "memory-capability-boundary-${System.nanoTime()}")
        val database = File(root, "product.db")
        val documentJson = """{"schemaVersion":1,"value":[]}"""
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val store = com.nola.data.local.datastore.MemoryProfileDataStore(
                    context, com.nola.data.local.security.SecureTokenStore(context), runtime
                )
                val baseline = store.getSnapshot()
                runtime.createUserSkill(com.nola.domain.model.UploadedContent(
                    name = "Review", content = "Keep this workflow", resourceId = "review"
                ))
                runtime.createMcpServer(com.nola.domain.model.McpServerConfig(
                    name = "Files", url = "https://example.com/mcp", serverId = "files"
                ))
                runtime.putProductSetting("assistant.uploaded_documents", documentJson, 1)
                val skills = runtime.listSkills()
                val servers = runtime.listMcpServers()
                val documents = runtime.productSetting("assistant.uploaded_documents")
                val saved = store.saveMemoryText(baseline.copy(longTermMemory = listOf("Remember only this")))
                store.saveMemoryText(saved.copy(longTermMemory = emptyList()))
                assertEquals(skills, runtime.listSkills())
                assertEquals(servers, runtime.listMcpServers())
                assertEquals(documents, runtime.productSetting("assistant.uploaded_documents"))
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertTrue(runtime.listMemories().isEmpty())
                assertEquals("Keep this workflow", runtime.listSkills().single().content)
                assertEquals("files", runtime.listMcpServers().single().serverId)
                assertEquals(documentJson, runtime.productSetting("assistant.uploaded_documents")?.valueJson)
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun displayedSkillSnapshotCannotDeleteOrToggleANewerVersion() = kotlinx.coroutines.runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "displayed-skill-${System.nanoTime()}")
        try {
            AndroidApplicationRuntime(context, File(root, "product.db")).use { runtime ->
                val store = com.nola.data.local.datastore.MemoryProfileDataStore(
                    context, com.nola.data.local.security.SecureTokenStore(context), runtime
                )
                runtime.createUserSkill(com.nola.domain.model.UploadedContent(
                    name = "Review", content = "Original instructions", resourceId = "review"
                ))
                val displayed = store.skills().single()
                val baseline = store.skillEditor(displayed)
                val newer = runtime.configureUserSkill(baseline, "Renamed", "New instructions")
                assertTrue(runCatching { store.deleteSkill(store.skillEditor(displayed)) }.isFailure)
                assertTrue(runCatching { store.setSkillEnabled(store.skillEditor(displayed), false) }.isFailure)
                assertEquals(newer, runtime.skillEditor("review"))
                assertTrue(runCatching { store.skillEditor(displayed.copy(resourceId = "different")) }.isFailure)
                store.deleteSkill(store.skillEditor(store.skills().single()))
                assertTrue(store.skills().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
