package com.nola.core

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.llm.LlmClient
import com.nola.llm.LlmStreamEvent
import kotlin.time.Duration
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.coroutines.withTimeout

class TextAgentModelClientProtocolTest {
    @Test
    fun onlyCorePresentationPolicyControlsReceivedReasoningProjection() = runTest {
        val provider = object : LlmClient {
            override val supportsNativeEventStream = true
            override fun generateEventStream(input: ModelInput): Flow<LlmStreamEvent> = flow {
                emit(LlmStreamEvent.ReasoningDelta("Provider supplied summary"))
                emit(LlmStreamEvent.TextDelta("done"))
                emit(LlmStreamEvent.Completed("stop"))
            }
            override suspend fun generateSummary(input: ModelInput, timeout: Duration?) = ModelOutput("")
            override fun generateSummaryStream(input: ModelInput): Flow<String> = flow { }
            override suspend fun chat(message: String, systemPrompt: String?, enableThinking: Boolean, timeout: Duration?) = ""
        }
        for (allowProjection in listOf(false, true)) {
            val request = AgentModelRequest(
                sessionId = "session", turnId = "turn", iteration = 1,
                messages = listOf(AgentCoreMessage("user", "Explain the result")), tools = emptyList(),
                reservedOutputTokens = 128,
                invocationPlan = AgentModelInvocationPlan(reasoning = false, streamText = true,
                    streamReasoning = allowProjection),
            )
            // This JVM test covers streaming projection before terminal parsing, which is native Core.
            val events = TextAgentModelClient(provider).generateAgentTurn(request)
                .take(if (allowProjection) 2 else 1).toList()
            assertEquals(if (allowProjection) 1 else 0, events.filterIsInstance<AgentModelEvent.ReasoningDelta>().size)
            assertEquals(1, events.filterIsInstance<AgentModelEvent.TextDelta>().size)
        }
    }

    @Test
    fun outputLimitFinishReasonsAreClassifiedWithoutPromptHeuristics() {
        assertEquals(true, isModelOutputLimitFinishReason("length"))
        assertEquals(true, isModelOutputLimitFinishReason(" MAX_OUTPUT_TOKENS "))
        assertEquals(false, isModelOutputLimitFinishReason("stop"))
        assertEquals(false, isModelOutputLimitFinishReason("tool_calls"))
    }

    @Test
    fun explicitCompletionTerminatesWithoutWaitingForTransportClose() = runTest {
        val transport = flow {
                emit(LlmStreamEvent.TextDelta("hello"))
                emit(LlmStreamEvent.Completed("stop"))
                awaitCancellation()
        }
        val events = mutableListOf<LlmStreamEvent>()

        withTimeout(1_000) {
            transport.collectThroughProtocolCompletion(events::add)
        }

        assertEquals(
            listOf(
                LlmStreamEvent.TextDelta("hello"),
                LlmStreamEvent.Completed("stop"),
            ),
            events,
        )
    }

    @Test
    fun unknownNativeToolIdentityReachesCoreUnchanged() = runTest {
        val provider = object : LlmClient {
            override val supportsNativeEventStream = true
            override fun generateEventStream(input: ModelInput): Flow<LlmStreamEvent> = flow {
                emit(LlmStreamEvent.ToolCallDelta(0, "provider-call-7", "catalog_tool_v2", "{\"路径\":\"当前\"}"))
                emit(LlmStreamEvent.Completed("tool_calls"))
            }
            override suspend fun generateSummary(input: ModelInput, timeout: Duration?) = ModelOutput("")
            override fun generateSummaryStream(input: ModelInput): Flow<String> = flow { }
            override suspend fun chat(
                message: String,
                systemPrompt: String?,
                enableThinking: Boolean,
                timeout: Duration?,
            ) = ""
        }
        val client = TextAgentModelClient(provider)
        val request = AgentModelRequest(
            sessionId = "session",
            turnId = "turn",
            iteration = 1,
            messages = listOf(AgentCoreMessage("user", "继续处理；search 和 current 只是普通文字")),
            tools = listOf(buildJsonObject {
                put("type", "function")
                put("function", buildJsonObject { put("name", "known_tool") })
            }),
            reservedOutputTokens = 128,
            invocationPlan = AgentModelInvocationPlan(nativeToolCalls = true, streamToolCalls = true),
        )

        val completion = client.generateAgentTurn(request).toList()
            .filterIsInstance<AgentModelEvent.Completed>().single().completion
        val call = completion.toolCalls.single()
        assertEquals("provider-call-7", call["call_id"]?.jsonPrimitive?.content)
        assertEquals("catalog_tool_v2", call["name"]?.jsonPrimitive?.content)
        assertEquals("当前", call["arguments"]?.jsonObject?.get("路径")?.jsonPrimitive?.content)
    }

    @Test
    fun providerContinuationPassesThroughFrozenRequestAndCompletion() = runTest {
        val incoming = buildJsonObject {
            put("provider_namespace", "openai.responses")
            put("model_id", "response-model")
        }
        val outgoing = buildJsonObject {
            put("provider_namespace", "openai.responses")
            put("model_id", "response-model")
            put("response_id", "resp_next")
        }
        val provider = object : LlmClient {
            override val supportsNativeEventStream = true
            override fun generateEventStream(input: ModelInput): Flow<LlmStreamEvent> = flow {
                assertEquals(incoming, input.providerContinuation)
                emit(LlmStreamEvent.TextDelta("done"))
                emit(LlmStreamEvent.ToolCallDelta(0, "call_1", "read", "{}"))
                emit(LlmStreamEvent.Continuation(outgoing))
                emit(LlmStreamEvent.Completed("stop"))
            }
            override suspend fun generateSummary(input: ModelInput, timeout: Duration?) = ModelOutput("")
            override fun generateSummaryStream(input: ModelInput): Flow<String> = flow { }
            override suspend fun chat(message: String, systemPrompt: String?, enableThinking: Boolean, timeout: Duration?) = ""
        }
        val request = AgentModelRequest(
            sessionId = "session", turnId = "turn", iteration = 1,
            messages = listOf(AgentCoreMessage("user", "继续")),
            tools = listOf(buildJsonObject {
                put("type", "function")
                put("function", buildJsonObject { put("name", "read") })
            }),
            reservedOutputTokens = 128,
            invocationPlan = AgentModelInvocationPlan(nativeToolCalls = true, streamText = true, streamToolCalls = true),
            providerContinuation = incoming,
        )
        val completion = TextAgentModelClient(provider).generateAgentTurn(request).toList()
            .filterIsInstance<AgentModelEvent.Completed>().single().completion
        assertEquals("done", completion.content)
        assertEquals(outgoing, completion.providerContinuation)
    }
    @Test
    fun queuedTextCoalescesWithoutCrossingReasoningToolOrCompletionBoundaries() = runTest {
        val tool = AgentModelEvent.ToolCallDelta("part", "call", "fixture.read", "{}")
        val done = AgentModelEvent.Completed(AgentModelCompletion("abcd"))
        val actual = kotlinx.coroutines.flow.flowOf(
            AgentModelEvent.TextDelta("a"), AgentModelEvent.TextDelta("b"),
            AgentModelEvent.ReasoningDelta("summary-a"), AgentModelEvent.ReasoningDelta("summary-b"),
            tool, AgentModelEvent.TextDelta("c"), AgentModelEvent.TextDelta("d"), done,
        ).coalesceQueuedTextDeltas().toList()
        assertEquals(listOf(AgentModelEvent.TextDelta("ab"),
            AgentModelEvent.ReasoningDelta("summary-asummary-b"), tool,
            AgentModelEvent.TextDelta("cd"), done), actual)
    }

}
