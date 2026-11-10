package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.domain.model.UploadedContent
import com.nola.domain.model.McpServerConfig
import java.io.File
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class AndroidDocumentEditorEndToEndTest {
    @Test fun documentEditsRejectStaleListsAndPreserveOtherResources() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val root = File(context.cacheDir, "document-editor-${System.nanoTime()}")
        val database = File(root, "product.db")
        try {
            AndroidApplicationRuntime(context, database).use { runtime ->
                val store = MemoryProfileDataStore(context, SecureTokenStore(context), runtime)
                store.saveMemoryText(store.getSnapshot().copy(longTermMemory = listOf("Keep memory")))
                runtime.createUserSkill(UploadedContent(name = "Skill", content = "Keep skill", resourceId = "skill"))
                runtime.createMcpServer(McpServerConfig(name = "Server", url = "https://example.com/mcp", serverId = "server"))
                val memory = runtime.memorySnapshot()
                val skills = runtime.listSkills()
                val servers = runtime.listMcpServers()
                val empty = store.documentEditor()
                val doc = UploadedContent(name = "Same name", content = "First", resourceId = "first")
                val first = store.saveDocuments(empty, listOf(doc))
                val second = store.saveDocuments(first, first.documents + doc.copy(content = "Second", resourceId = "second"))
                assertTrue(runCatching { store.saveDocuments(first, emptyList()) }.isFailure)
                assertEquals(second, store.documentEditor())
                assertTrue(runCatching { store.saveDocuments(second, listOf(doc, doc)) }.isFailure)
                val deleted = store.saveDocuments(second, second.documents.filterNot { it.resourceId == "first" })
                assertEquals(listOf("second"), deleted.documents.map { it.resourceId })
                assertEquals(memory, runtime.memorySnapshot())
                assertEquals(skills, runtime.listSkills())
                assertEquals(servers, runtime.listMcpServers())
            }
            AndroidApplicationRuntime(context, database).use { runtime ->
                val store = MemoryProfileDataStore(context, SecureTokenStore(context), runtime)
                assertEquals("Second", store.documentEditor().documents.single().content)
                assertEquals("Keep memory", store.getSnapshot().longTermMemory.single())
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
