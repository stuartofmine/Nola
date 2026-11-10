package com.nola.core

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class CompactionSummaryExecutorTest {
    private fun request(state: String, text: String) = buildJsonObject {
        put("action", "request"); put("state", JsonPrimitive(state)); put("target_tokens", 100)
        put("messages", buildJsonArray { add(buildJsonObject { put("role", "user"); put("content", text) }) })
    }

    @Test fun advancesOpaqueStateOnlyAfterAcceptedSegment() = runBlocking {
        val physical = mutableListOf<String>()
        val states = mutableListOf<JsonElement>()
        val executor = CompactionSummaryExecutor(execute = { command ->
            if (command["type"]?.jsonPrimitive?.content == "evaluate_compaction_maintenance") {
                buildJsonObject { put("action", "accept") }
            } else {
                val step = command.getValue("step").jsonObject
                if (step["type"]?.jsonPrimitive?.content == "start") request("opaque-1", "first")
                else {
                    states += step.getValue("state")
                    if (states.size == 1) request("opaque-2", "second")
                    else buildJsonObject { put("action", "complete"); put("summary", "Core final checkpoint") }
                }
            }
        }, generate = { messages, _, _ ->
            physical += messages.single().content
            AgentModelCompletion(content = "accepted", finishReason = "stop")
        })
        val result = executor.summarize(emptyList(), 16000, 2000, 100, "instructions")
        assertEquals(listOf("first", "second"), physical)
        assertEquals(listOf(JsonPrimitive("opaque-1"), JsonPrimitive("opaque-2")), states)
        assertEquals("Core final checkpoint", result.content)
    }

    @Test fun unknownFinishIsNotReportedCompletedOrAcceptedLocally() = runBlocking {
        var observed: JsonObject? = null
        val executor = CompactionSummaryExecutor(execute = { command ->
            if (command["type"]?.jsonPrimitive?.content == "evaluate_compaction_maintenance") {
                observed = command.getValue("facts").jsonObject
                buildJsonObject { put("action", "reject") }
            } else request("opaque", "source")
        }, generate = { _, _, _ -> AgentModelCompletion(content = "partial", finishReason = "unknown") })
        var rejected = false
        try { executor.summarize(emptyList(), 16000, 2000, 100, "instructions") }
        catch (_: IllegalStateException) { rejected = true }
        assertTrue(rejected)
        assertEquals(JsonPrimitive(false), observed?.get("completed"))
    }
}
