package com.nola.core

import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AgentNativeToolCallAccumulatorTest {
    @Test
    fun parallelDeltasBecomeOrderedCompleteCalls() {
        val accumulator = AgentNativeToolCallAccumulator()
        accumulator.append("call-a", "read_file", "{\"path\":\"")
        accumulator.append("call-b", "web_search", "{\"query\":\"current")
        accumulator.append("call-a", "read_file", "/tmp/a\"}")
        accumulator.append("call-b", "web_search", " status\"}")

        val calls = accumulator.completedCalls()

        assertEquals(listOf("call-a", "call-b"), calls.map { it["call_id"]?.toString()?.trim('"') })
        assertEquals(listOf("read_file", "web_search"), calls.map { it["name"]?.toString()?.trim('"') })
        assertEquals(
            JsonPrimitive("/tmp/a"),
            calls[0]["arguments"]?.jsonObject?.get("path"),
        )
        assertEquals(
            JsonPrimitive("current status"),
            calls[1]["arguments"]?.jsonObject?.get("query"),
        )
    }

    @Test
    fun invalidFinalArgumentsAreRejectedInsteadOfSilentlyExecuting() {
        val accumulator = AgentNativeToolCallAccumulator()
        accumulator.append("call-invalid", "read_file", "{\"path\":\"unterminated")

        var rejected = false
        try {
            accumulator.completedCalls()
        } catch (_: IllegalArgumentException) {
            rejected = true
        } catch (_: kotlinx.serialization.SerializationException) {
            rejected = true
        }
        assertTrue(rejected)
    }

    @Test
    fun providerIndexKeepsLateIdentityAndParallelEmptyIdsSeparate() {
        val accumulator = AgentNativeToolCallAccumulator("turn-a-3-tool-call")

        assertEquals(null, accumulator.append(0, "", "", "{\"path\":\""))
        val first = accumulator.append(0, "provider-id-late", "read_file", "a.md\"}")
        val second = accumulator.append(1, "", "web_search", "{\"query\":\"current\"}")

        assertEquals("turn-a-3-tool-call-0", first?.callId)
        assertEquals("turn-a-3-tool-call-1", second?.callId)
        assertEquals(
            listOf("turn-a-3-tool-call-0", "turn-a-3-tool-call-1"),
            accumulator.completedCalls().map { it["call_id"]?.toString()?.trim('"') },
        )
    }

    @Test
    fun memoryWorkerExecutorReturnsOnlyRequiredToolArguments() = runBlocking {
        val expected = buildJsonObject { put("candidates", buildJsonArray {}) }
        val client = CapturingMemoryWorkerClient(expected)
        val plan = buildJsonObject {
            put("job_id", "job-1")
            put("system_instructions", "Extract durable memory.")
            put("model_input", buildJsonObject { put("transcript", buildJsonArray {}) })
            put("response_schema", buildJsonObject {
                put("type", "object")
                put("properties", buildJsonObject {})
            })
        }

        val output = AgentMemoryWorkerModelExecutor.execute(plan, client)

        assertEquals(expected, output)
        assertEquals(AgentMemoryWorkerModelExecutor.submissionToolName, client.request?.requiredToolName)
        assertTrue(client.request?.invocationPlan?.nativeToolCalls == true)
    }

    @Test
    fun structuredOutputExecutorRejectsDuplicateRequiredCalls() = runBlocking {
        val toolName = "submit_fixture"
        val arguments = buildJsonObject { put("value", "first") }
        var capturedRequest: AgentModelRequest? = null
        val client = object : AgentModelClient {
            override fun generateAgentTurn(request: AgentModelRequest) = flow {
                capturedRequest = request
                emit(AgentModelEvent.Completed(AgentModelCompletion(
                    content = "",
                    toolCalls = listOf("call-1", "call-2").map { callId ->
                        buildJsonObject {
                            put("call_id", callId)
                            put("name", toolName)
                            put("arguments", arguments)
                        }
                    },
                    finishReason = "tool_calls",
                )))
            }
        }

        val result = runCatching {
            AgentStructuredOutputExecutor.execute(
                sessionId = "fixture-session",
                turnId = "fixture-turn",
                systemInstructions = "Return structured output.",
                modelInput = buildJsonObject { put("input", "fixture") },
                responseSchema = buildJsonObject { put("type", "object") },
                submissionToolName = toolName,
                submissionToolDescription = "Submit the fixture.",
                client = client,
            )
        }

        assertTrue(result.isFailure)
        assertEquals(toolName, capturedRequest?.requiredToolName)
    }
}

private class CapturingMemoryWorkerClient(
    private val arguments: JsonElement,
) : AgentModelClient {
    var request: AgentModelRequest? = null

    override fun generateAgentTurn(request: AgentModelRequest) = flow {
        this@CapturingMemoryWorkerClient.request = request
        emit(AgentModelEvent.Completed(AgentModelCompletion(
            content = "",
            toolCalls = listOf(buildJsonObject {
                put("call_id", "memory-call")
                put("name", AgentMemoryWorkerModelExecutor.submissionToolName)
                put("arguments", arguments)
            }),
            finishReason = "tool_calls",
        )))
    }
}
