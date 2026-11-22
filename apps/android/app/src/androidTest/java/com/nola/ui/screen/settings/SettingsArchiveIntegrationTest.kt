// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.settings

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.datastore.MemoryProfileDataStore
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.local.security.SecureTokenStore
import com.nola.domain.model.McpServerConfig
import com.nola.ui.screen.chat.requireIsolatedProductTestUser
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.UUID
import java.util.zip.ZipFile

class SettingsArchiveIntegrationTest {
    @Test fun exportReadsCommittedWalTextAcrossUnicodeRangesAndLeavesCredentialsPrivate() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        requireIsolatedProductTestUser(context)
        val root = File(context.cacheDir, "archive-test-${UUID.randomUUID()}").apply { mkdirs() }
        val credential = "fixture-auth-${UUID.randomUUID()}"
        val text = "正文🌙".repeat(4_500)
        try {
            val db = File(root, "nola.db")
            AndroidApplicationRuntime(context, db).use { runtime ->
                runtime.createConversation("export-chat", "Review notes", 1, 1, null)
                runtime.createMcpServer(McpServerConfig(name = "fixture", url = "https://example.org/mcp?api_key=$credential"))
                runtime.execute(buildJsonObject {
                    put("type", "import_conversation_transcript")
                    put("input", buildJsonObject {
                        put("import_id", "archive-test"); put("source", "external_archive")
                        put("conversation_id", "export-chat"); put("expected_message_count", 0); put("imported_at_ms", 2)
                        put("messages", buildJsonArray { add(buildJsonObject {
                            put("id", "user"); put("role", "user"); put("status", "completed")
                            put("ordinal", 1); put("created_at_ms", 2); put("completed_at_ms", 2)
                            put("parts", buildJsonArray { add(buildJsonObject {
                                put("id", "body"); put("kind", "text"); put("ordinal", 1)
                                put("state", "completed"); put("created_at_ms", 2); put("updated_at_ms", 2)
                                put("content_text", text)
                                put("content_json", buildJsonObject { put("Authorization", credential) }.toString())
                            }) })
                        }) })
                    })
                })
                val assistantText = "回答🌟".repeat(4_500)
                runtime.execute(buildJsonObject {
                    put("type", "import_conversation_transcript")
                    put("input", buildJsonObject {
                        put("import_id", "assistant-import"); put("source", "external_archive")
                        put("conversation_id", "export-chat"); put("expected_message_count", 1); put("imported_at_ms", 3)
                        put("messages", buildJsonArray { add(buildJsonObject {
                            put("id", "reply"); put("role", "assistant"); put("status", "completed"); put("ordinal", 2)
                            put("created_at_ms", 3); put("completed_at_ms", 3)
                            put("parts", buildJsonArray {
                                fun part(id: String, kind: String, ordinal: Int, data: JsonObject) = buildJsonObject {
                                    put("id", id); put("kind", kind); put("ordinal", ordinal); put("state", "completed")
                                    put("created_at_ms", 3); put("updated_at_ms", 3); put("content_text", "")
                                    put("content_json", data.toString())
                                }
                                add(part("reply-text", "text", 1, buildJsonObject {
                                    put("kind", "text"); put("id", "reply-text"); put("message_id", "reply"); put("turn_id", "turn")
                                    put("text", assistantText); put("is_complete", true); put("private_metadata", credential)
                                }))
                                add(part("tool-result", "tool", 2, buildJsonObject {
                                    put("Authorization", credential); put("text", credential)
                                }))
                            })
                        }) })
                    })
                })
                assertTrue(File(db.path + "-wal").length() > 0)
                val secure = SecureTokenStore(context)
                val viewModel = SettingsViewModel(context, PreferenceDataStore(context, secure),
                    MemoryProfileDataStore(context, secure, runtime), ModelParametersStore(runtime), runtime)
                val archive = viewModel.createExportArchive()
                try {
                    ZipFile(archive).use { zip ->
                        val entries = zip.entries().asSequence().map { it.name }.toSet()
                        assertEquals(setOf("manifest.json", "conversations/1.md", "profile/memory.json",
                            "settings/model_parameters.json", "settings/preferences.json", "README.txt"), entries)
                        fun read(name: String) = zip.getInputStream(zip.getEntry(name)).bufferedReader().use { it.readText() }
                        assertEquals("# Review notes\n\n## user\n\n$text\n\n## assistant\n\n$assistantText\n", read("conversations/1.md"))
                        assertEquals(2, Json.parseToJsonElement(read("manifest.json")).jsonObject.getValue("version").jsonPrimitive.int)
                        for (name in entries) assertFalse("Credential found in $name", read(name).contains(credential))
                    }
                } finally {
                    archive.delete()
                }
            }
        } finally {
            root.deleteRecursively()
        }
    }
}
