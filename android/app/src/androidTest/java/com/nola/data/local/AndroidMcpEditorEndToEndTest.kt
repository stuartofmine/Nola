package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.domain.model.McpServerConfig
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.*

class AndroidMcpEditorEndToEndTest {
    @Test
    fun editingPreservesUnknownConfigurationAndCanonicalIdentity() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "mcp-config-${System.nanoTime()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val embedded = McpServerConfig(name = "Old embedded name", url = "https://example.test/mcp", serverId = "wrong-embedded-id")
                val extras = buildJsonObject {
                    put("headers", buildJsonObject { put("X-Fixture", "keep") })
                    put("futureProtocol", buildJsonObject { put("timeout", 42) })
                }
                val configuration = JsonObject(Json.parseToJsonElement(Json.encodeToString(embedded)).jsonObject + extras)
                runtime.submitMutation(buildJsonObject {
                    put("type", "mcp")
                    put("command", buildJsonObject {
                        put("type", "create")
                        put("id", "canonical-id")
                        put("display_name", "Canonical name")
                        put("enabled", false)
                        put("configuration_json", configuration.toString())
                        put("created_at_ms", 1)
                    })
                })
                val opened = runtime.mcpEditorSnapshots().single()
                assertEquals("canonical-id", opened.server.serverId)
                assertEquals("Canonical name", opened.server.name)
                assertFalse(opened.server.enabled)
                val saved = runtime.configureMcpServer(opened, opened.server.copy(name = "Renamed", url = "https://example.test/new"))
                assertEquals("Renamed", saved.server.name)
                assertEquals("https://example.test/new", saved.server.url)
                extras.forEach { (key, value) -> assertEquals(value, saved.server.canonicalConfiguration?.get(key)) }
                assertTrue(runCatching { runtime.configureMcpServer(opened, opened.server.copy(name = "Stale")) }.isFailure)
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val saved = runtime.mcpEditorSnapshots().single()
                assertEquals("Renamed", saved.server.name)
                assertEquals("canonical-id", saved.server.serverId)
                assertEquals("keep", saved.server.canonicalConfiguration?.get("headers")?.jsonObject?.get("X-Fixture")?.jsonPrimitive?.content)
                runtime.deleteMcpServer(saved)
                assertTrue(runtime.mcpEditorSnapshots().isEmpty())
            }
        } finally { root.deleteRecursively() }
    }

    @Test
    fun creationRejectsExistingAndDeletedIdentityThroughJni() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "mcp-create-${System.nanoTime()}")
        val database = File(root, "product.db")
        val input = McpServerConfig(name = "Files", url = "http://localhost:9876/mcp", serverId = "files")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val saved = runtime.createMcpServer(input)
                val duplicate = runCatching { runtime.createMcpServer(input.copy(name = "Overwrite")) }.exceptionOrNull()
                assertTrue(duplicate?.message.orEmpty().contains("mcp_identity_conflict"))
                assertEquals(saved, runtime.mcpEditorSnapshots().single())
                runtime.deleteMcpServer(saved)
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val resurrection = runCatching { runtime.createMcpServer(input) }.exceptionOrNull()
                assertTrue(resurrection?.message.orEmpty().contains("mcp_identity_conflict"))
                assertTrue(runtime.mcpEditorSnapshots().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun singleServerEditingPreservesOtherServersAndMemory() = kotlinx.coroutines.runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "mcp-profile-editor-${System.nanoTime()}")
        try {
            AndroidApplicationRuntime(context, File(root, "product.db")).use { runtime ->
                val store = com.nola.data.local.datastore.MemoryProfileDataStore(
                    context, com.nola.data.local.security.SecureTokenStore(context), runtime
                )
                store.saveMemoryText(store.getSnapshot().copy(longTermMemory = listOf("Keep this memory")))
                val memoryBefore = runtime.memorySnapshot().canonical
                store.saveMcpServer(null, McpServerConfig(name = "Same name", url = "http://localhost:9876/one"))
                store.saveMcpServer(null, McpServerConfig(name = "Same name", url = "http://localhost:9876/two"))
                val opened = store.getSnapshot().mcpServers
                assertEquals(2, opened.map { it.serverId }.toSet().size)
                val first = opened.first()
                val other = opened.last()
                store.saveMcpServer(first, first.copy(name = "ZZ renamed"))
                val failure = runCatching { store.deleteMcpServer(first) }.exceptionOrNull()
                assertTrue(failure?.message.orEmpty().contains("mcp_revision_conflict"))
                val current = store.getSnapshot().mcpServers
                assertEquals(other, current.single { it.serverId == other.serverId })
                store.deleteMcpServer(current.single { it.serverId == first.serverId })
                assertEquals(listOf(other), store.getSnapshot().mcpServers)
                assertEquals(memoryBefore, runtime.memorySnapshot().canonical)
            }
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun staleEditorCannotOverwriteDeleteOrResurrectMcpServer() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "mcp-editor-${System.nanoTime()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                runtime.createMcpServer(McpServerConfig(name = "Files", url = "http://localhost:9876/mcp"))
                val opened = runtime.mcpEditorSnapshots().single()
                val committed = runtime.configureMcpServer(opened, opened.server.copy(name = "Renamed"))
                assertTrue(committed.revision > opened.revision)
                assertEquals("Renamed", committed.server.name)
                assertEquals(committed, runtime.mcpEditorSnapshots().single())
                val editFailure = runCatching {
                    runtime.configureMcpServer(opened, opened.server.copy(enabled = false))
                }.exceptionOrNull()
                assertTrue(editFailure?.message.orEmpty().contains("mcp_revision_conflict"))
                val deleteFailure = runCatching { runtime.deleteMcpServer(opened) }.exceptionOrNull()
                assertTrue(deleteFailure?.message.orEmpty().contains("mcp_revision_conflict"))
                assertEquals(committed, runtime.mcpEditorSnapshots().single())
                runtime.deleteMcpServer(committed)
                val resurrection = runCatching {
                    runtime.configureMcpServer(committed, committed.server.copy(name = "Resurrected"))
                }.exceptionOrNull()
                assertTrue(resurrection?.message.orEmpty().contains("missing or deleted"))
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                assertTrue(runtime.mcpEditorSnapshots().isEmpty())
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
