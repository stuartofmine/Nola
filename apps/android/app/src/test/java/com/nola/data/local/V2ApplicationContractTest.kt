package com.nola.data.local

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class V2ApplicationContractTest {

    @Test
    fun v2ReadAndWriteCommandsClassifyCorrectly() {
        val reads = listOf(
            "health", "sessions_list", "events_after", "projection_snapshot",
            "module_list", "memory_search",
        )
        for (cmd in reads) {
            assertTrue("$cmd should be read-only", ApplicationCommandEffect.isObservation(buildJsonObject { put("type", cmd) }))
        }
        val writes = listOf(
            "session_create", "session_delete", "module_upsert", "module_delete", "memory_save", "memory_delete",
            "branch_conversation", "truncate_conversation", "compact_conversation",
        )
        for (cmd in writes) {
            assertFalse("$cmd should be a mutation", ApplicationCommandEffect.isObservation(buildJsonObject { put("type", cmd) }))
        }
    }

    @Test
    fun v2SessionScopeIsolatesById() {
        val delScope = ApplicationCommandScope.command(buildJsonObject {
            put("type", "session_delete")
            put("id", "sess-42")
        })
        assertEquals(ApplicationCommandScope.Conversation("sess-42"), delScope)

        val evScope = ApplicationCommandScope.command(buildJsonObject {
            put("type", "events_after")
            put("scope", "sess-42")
        })
        assertEquals(ApplicationCommandScope.Conversation("sess-42"), evScope)

        for (cmd in listOf("branch_conversation", "truncate_conversation", "compact_conversation")) {
            val scope = ApplicationCommandScope.command(buildJsonObject {
                put("type", cmd)
                put("conversation_id", "conv-99")
            })
            assertEquals("$cmd should scope by conversation_id", ApplicationCommandScope.Conversation("conv-99"), scope)
        }
    }

    @Test
    fun v2WorkspaceReviewParserHandlesGitAndNonGitPayloads() {
        val gitPayload = buildJsonObject {
            put("workspace_root", "/repo/nola")
            put("is_git_repository", true)
            put("truncated", false)
            put("review", buildJsonObject {
                put("inventory", buildJsonObject {
                    put("entries", JsonArray(listOf(buildJsonObject {
                        put("path", "src/main.rs")
                        put("index_status", "M")
                        put("worktree_status", " ")
                    })))
                })
                put("staged", buildJsonObject { put("text", "+fn hello() {}") })
                put("unstaged", buildJsonObject { put("text", "-old\n+new") })
            })
        }
        val parsedGit = com.nola.ui.screen.chat.parseV2WorkspaceReview(gitPayload)
        assertTrue(parsedGit.isGitRepository)
        assertEquals("/repo/nola", parsedGit.workspaceRoot)
        assertEquals(1, parsedGit.entries.size)
        assertEquals("src/main.rs", parsedGit.entries[0].path)
        assertEquals("M", parsedGit.entries[0].status)
        assertEquals("+fn hello() {}", parsedGit.stagedDiff)
        assertEquals("-old\n+new", parsedGit.unstagedDiff)

        val dirPayload = buildJsonObject {
            put("workspace_root", "/tmp/scratch")
            put("is_git_repository", false)
            put("truncated", true)
            put("files", JsonArray(listOf(buildJsonObject {
                put("path", "notes.md")
                put("is_dir", false)
                put("size", 128)
            })))
        }
        val parsedDir = com.nola.ui.screen.chat.parseV2WorkspaceReview(dirPayload)
        assertFalse(parsedDir.isGitRepository)
        assertTrue(parsedDir.truncated)
        assertEquals(1, parsedDir.entries.size)
        assertEquals("notes.md", parsedDir.entries[0].path)
        assertEquals(128L, parsedDir.entries[0].sizeBytes)
    }

    @Test
    fun rustV2CommittedAndRejectedReceiptsPassStrictValidation() {
        val request = buildJsonObject {
            put("protocol_version", 1)
            put("client_id", "android-v2")
            put("request_id", "req-v2-1")
            put("command", buildJsonObject {
                put("type", "session_create")
                put("arguments", buildJsonObject { put("title", "Hello v2") })
            })
        }

        val committedWire = buildJsonObject {
            put("ok", true)
            put("result", buildJsonObject {
                put("protocol_version", 1)
                put("client_id", "android-v2")
                put("request_id", "req-v2-1")
                put("received_at", "2026-11-25T12:00:00+00:00")
                put("completed_at", "2026-11-25T12:00:00+00:00")
                put("status", "committed")
                put("committed_revision", 1)
                put("result", buildJsonObject { put("id", "sess-1"); put("title", "Hello v2") })
            })
        }
        val decodedCommitted = ApplicationReceiptValidation.decode(
            request,
            committedWire.toString().encodeToByteArray(),
            readOnly = false,
        )
        assertEquals("committed", decodedCommitted["status"]?.let { (it as JsonPrimitive).content })
        assertEquals("sess-1", decodedCommitted["result"]?.jsonObject?.get("id")?.let { (it as JsonPrimitive).content })

        val rejectedWire = buildJsonObject {
            put("ok", true)
            put("result", buildJsonObject {
                put("protocol_version", 1)
                put("client_id", "android-v2")
                put("request_id", "req-v2-1")
                put("received_at", "2026-11-25T12:00:00+00:00")
                put("completed_at", "2026-11-25T12:00:00+00:00")
                put("status", "rejected")
                put("error", buildJsonObject {
                    put("code", "not_found")
                    put("message", "conversation not found")
                    put("recoverable", false)
                })
            })
        }
        val decodedRejected = ApplicationReceiptValidation.decode(
            request,
            rejectedWire.toString().encodeToByteArray(),
            readOnly = false,
        )
        assertEquals("rejected", decodedRejected["status"]?.let { (it as JsonPrimitive).content })
        assertEquals(JsonArray(emptyList()).size, 0)
        assertEquals(JsonObject(emptyMap()).size, 0)
    }
}
