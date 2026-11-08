package com.nola.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test

class AgentCoreContextPrepareContractTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun inputUsesVersionedTypedComponentSchema() {
        val bytes = AgentCoreContextPreparation.encodeInput(
            AgentCoreContextPrepareInput(
                mode = "context_limit_recovery",
                components = listOf(
                    AgentCoreContextComponent(
                        kind = "document",
                        id = "doc-1",
                        content = JsonPrimitive("full document"),
                        source = AgentCoreContextComponentSource(
                            platform = "android",
                            uri = "content://document/1",
                            title = "Document",
                            mimeType = "text/plain",
                        ),
                    )
                ),
                messages = listOf(buildJsonObject {
                    put("id", "m1")
                    put("role", "user")
                    put("content", "hello")
                }),
                tools = listOf(buildJsonObject { put("name", "search") }),
                budget = AgentCoreContextPrepareBudget(contextWindowTokens = 128_000),
            )
        )

        val root = json.parseToJsonElement(bytes.decodeToString()).jsonObject
        assertEquals("1", root.getValue("schema_version").jsonPrimitive.content)
        assertEquals("android", root.getValue("platform").jsonPrimitive.content)
        assertNotNull(root["components"])
        assertNotNull(root["messages"])
        assertNotNull(root["tools"])
    }

    @Test
    fun outputPreservesCanonicalPayloadAndArchiveDigests() {
        val output = AgentCoreContextPreparation.decodeOutput(
            """
            {
              "schema_version": 1,
              "action": "summary_required",
              "messages": [{"id":"m1","role":"user","content":"hello","future_field":{"x":1}}],
              "tools": [{"name":"search"}],
              "payload_digest": "payload-digest",
              "metrics": {
                "message_tokens": 90,
                "tool_schema_tokens": 10,
                "total_prompt_tokens": 100,
                "prompt_token_limit": 120,
                "reserved_output_tokens": 8,
                "usage_percent": 83.333,
                "before_tokens": 100,
                "after_tokens": 100,
                "trigger_tokens": 96,
                "target_tokens": 80,
                "archive_message_count": 1,
                "transaction_count": 0
              },
              "component_metrics": [{
                "kind":"document","used_tokens":20,"dropped_tokens":0,"truncated_tokens":0,
                "used_ids":["doc-1"],"dropped_ids":[],"truncated_ids":[]
              }],
              "archive_request": {
                "archive_id":"archive-1","archive_digest":"${"a".repeat(64)}",
                "message_indices":[0],"message_ids":["m1"],
                "messages":[{"id":"m1","role":"user","content":"hello"}],
                "target_summary_tokens":64
              }
            }
            """.trimIndent().encodeToByteArray()
        )

        assertEquals("summary_required", output.action)
        assertEquals("payload-digest", output.payloadDigest)
        assertEquals(JsonPrimitive("m1"), output.messages.single().jsonObject["id"])
        assertNotNull(output.messages.single().jsonObject["future_field"])
        assertEquals("search", output.tools.single().jsonObject["name"]?.jsonPrimitive?.content)
        assertEquals("archive-1", output.archiveRequest?.archiveId)
        assertEquals("a".repeat(64), output.archiveRequest?.archiveDigest)
        assertEquals(20, output.componentMetrics.single().usedTokens)
    }
}
