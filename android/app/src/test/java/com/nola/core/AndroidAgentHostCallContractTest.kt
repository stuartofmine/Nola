package com.nola.core

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertThrows
import org.junit.Assert.assertEquals
import org.junit.Test

class AndroidAgentHostCallContractTest {
    @Test
    fun nativeReceiptsNeverInventLocalStopEvidence() {
        listOf("not_started", "completed", "unknown", "partial", "deferred").forEach { state ->
            val result = androidToolResult(JsonObject(emptyMap()), state)
            assertEquals("nola.tool_result.v2", result.getValue("contract").jsonPrimitive.content)
            val receipt = result.getValue("execution").jsonObject
            assertEquals(state, receipt.getValue("state").jsonPrimitive.content)
            assertEquals(JsonNull, receipt.getValue("local_stop"))
        }
    }

    @Test
    fun parsesCompleteV2ExecuteToolPayload() {
        val request = androidAgentHostCallRequest(
            JsonObject(
                mapOf(
                    "type" to JsonPrimitive("execute_tool"),
                    "call_id" to JsonPrimitive("call-1"),
                    "capability_id" to JsonPrimitive("device_read_file"),
                    "input" to JsonObject(mapOf("uri" to JsonPrimitive("content://document/1"))),
                    "grant" to JsonObject(mapOf("uri" to JsonPrimitive("content://document/1")))
                )
            ),
            capability = "files_read",
        )

        assertEquals("call-1", request.callId)
        assertEquals("device_read_file", request.toolName)
        assertEquals("files_read", request.capability)
        assertEquals("content://document/1", request.arguments["uri"]?.toString()?.trim('"'))
        assertEquals("content://document/1", request.grant?.get("uri")?.toString()?.trim('"'))
    }

    @Test
    fun rejectsIncompleteV2ExecuteToolPayloads() {
        assertThrows(IllegalStateException::class.java) {
            androidAgentHostCallRequest(
                JsonObject(
                    mapOf(
                        "type" to JsonPrimitive("execute_tool"),
                        "capability_id" to JsonPrimitive("device_read_file"),
                    )
                ),
                capability = "files_read",
            )
        }
        assertThrows(IllegalStateException::class.java) {
            androidAgentHostCallRequest(
                JsonObject(
                    mapOf(
                        "type" to JsonPrimitive("execute_tool"),
                        "call_id" to JsonPrimitive("call-2"),
                        "capability_id" to JsonPrimitive(""),
                    )
                ),
                capability = "files_read",
            )
        }
        assertThrows(IllegalArgumentException::class.java) {
            androidAgentHostCallRequest(
                JsonObject(
                    mapOf(
                        "type" to JsonPrimitive("execute_tool"),
                        "call_id" to JsonPrimitive("call-3"),
                        "capability_id" to JsonPrimitive("device_read_file"),
                    )
                ),
                capability = "",
            )
        }
    }

    @Test
    fun rejectedUnknownToolPreservesIdentityAndArgumentsWithoutKeywordRerouting() {
        val originalArguments = JsonObject(
            mapOf(
                "query" to JsonPrimitive("搜索 current 架构，不要改选工具"),
                "scope" to JsonPrimitive("follow-up"),
            )
        )
        val result = androidRejectedToolCallPayload(
            payload = JsonObject(
                mapOf(
                    "type" to JsonPrimitive("execute_tool"),
                    "call_id" to JsonPrimitive("call-unknown-7"),
                    "capability_id" to JsonPrimitive("search"),
                    "input" to originalArguments,
                )
            ),
            code = "capability_unavailable",
            message = "Tool is not available in the active Android plan.",
        )

        assertEquals("tool_completed", result["type"]?.jsonPrimitive?.content)
        assertEquals("call-unknown-7", result["call_id"]?.jsonPrimitive?.content)
        assertEquals("failed", result["outcome"]?.jsonPrimitive?.content)
        assertEquals(0, result["artifacts"]?.jsonArray?.size)
        val envelope = result["output"]!!.jsonObject
        assertEquals("nola.tool_result.v2", envelope["contract"]?.jsonPrimitive?.content)
        assertEquals("not_started", envelope["execution"]!!.jsonObject["state"]?.jsonPrimitive?.content)
        assertEquals(kotlinx.serialization.json.JsonNull, envelope["execution"]!!.jsonObject.getValue("local_stop"))
        val output = envelope["data"]!!.jsonObject
        assertEquals("capability_unavailable", output["error"]!!.jsonObject["code"]?.jsonPrimitive?.content)
        val requestedCall = output["requested_call"]!!.jsonObject
        assertEquals("call-unknown-7", requestedCall["call_id"]?.jsonPrimitive?.content)
        assertEquals("search", requestedCall["name"]?.jsonPrimitive?.content)
        assertEquals(originalArguments, requestedCall["arguments"])
    }

    @Test
    fun rejectedNonObjectArgumentsRemainVisibleToTypedRecovery() {
        val originalArguments = JsonPrimitive("当前内容")
        val result = androidRejectedToolCallPayload(
            payload = JsonObject(
                mapOf(
                    "call_id" to JsonPrimitive("call-invalid-input"),
                    "capability_id" to JsonPrimitive("device_read_file"),
                    "input" to originalArguments,
                )
            ),
            code = "invalid_tool_input",
            message = "Tool input must be an object.",
        )

        val requestedCall = result["output"]!!.jsonObject["data"]!!.jsonObject["requested_call"]!!.jsonObject
        assertEquals(originalArguments, requestedCall["arguments"])
    }

    @Test
    fun rejectedMissingArgumentsRemainExplicitlyAbsent() {
        val result = androidRejectedToolCallPayload(
            payload = JsonObject(
                mapOf(
                    "call_id" to JsonPrimitive("call-missing-input"),
                    "capability_id" to JsonPrimitive("device_read_file"),
                )
            ),
            code = "invalid_tool_input",
            message = "Tool input must be an object.",
        )

        val requestedCall = result["output"]!!.jsonObject["data"]!!.jsonObject["requested_call"]!!.jsonObject
        assertEquals(JsonNull, requestedCall["arguments"])
    }
}
