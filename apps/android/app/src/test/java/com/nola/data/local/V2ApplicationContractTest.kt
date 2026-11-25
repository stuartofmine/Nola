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
