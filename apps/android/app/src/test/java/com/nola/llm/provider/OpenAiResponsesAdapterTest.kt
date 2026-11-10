package com.nola.llm.provider

import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelWireProtocol
import com.nola.llm.LlmStreamEvent
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.http.content.OutgoingContent
import io.ktor.http.headersOf
import io.ktor.serialization.kotlinx.json.json
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OpenAiResponsesAdapterTest {
    private suspend fun responseEvents(frames: List<String>): List<LlmStreamEvent> {
        val client = HttpClient(MockEngine {
            respond(frames.joinToString("\n\n") { "data: $it" } + "\n\n", headers = headersOf("Content-Type", "text/event-stream"))
        }) { install(ContentNegotiation) { json() } }
        return try { OpenAiResponsesAdapter(client, config()).events(ModelInput(
            messages = listOf(mapOf("role" to "user", "content" to "fixture")))).toList() }
        finally { client.close() }
    }
    private fun terminalReasoning(content: String, encrypted: String = "opaque-fixture", summary: String = "[]") =
        """{"type":"response.completed","response":{"id":"response","status":"completed","output":[{"type":"reasoning","id":"r1","summary":$summary,"content":$content,"encrypted_content":"$encrypted"},{"type":"message","content":[{"type":"output_text","text":"answer"}]}]}}"""

    @Test fun plaintextReasoningDeltasAreLiveAndTerminalSnapshotDoesNotRepeatThem() = runBlocking {
        val events = responseEvents(listOf(
            """{"type":"response.created","response":{"output":[]}}""",
            """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"first "}""",
            """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"second"}""",
            terminalReasoning("""[{"type":"reasoning_text","text":"first second"}]""")))
        assertEquals(listOf("first ", "second"), events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().map { it.text })
        assertTrue(events.indexOfFirst { it is LlmStreamEvent.ReasoningDelta } < events.indexOfFirst { it is LlmStreamEvent.TextDelta })
        assertEquals("answer", events.filterIsInstance<LlmStreamEvent.TextDelta>().joinToString("") { it.text })
        val continuation = events.filterIsInstance<LlmStreamEvent.Continuation>().single().value
        assertEquals("opaque-fixture", continuation.getValue("state").jsonObject.getValue("response_items").jsonArray[0].jsonObject.getValue("encrypted_content").jsonPrimitive.content)
    }

    @Test fun reasoningItemIdentityLinksIdAndIndexAcrossSubsequentDeltas() = runBlocking {
        for (initial in listOf(
            """{"type":"response.reasoning_text.delta","output_index":0,"delta":"A"}""",
            """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"A"}""")) {
            val events = responseEvents(listOf(initial,
                """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"B"}""",
                """{"type":"response.reasoning_text.delta","output_index":0,"delta":"C"}""",
                """{"type":"response.reasoning_text.delta","item_id":"r1","delta":"D"}""",
                terminalReasoning("""[{"type":"reasoning_text","text":"ABCD"}]""")))
            assertEquals("ABCD", events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().joinToString("") { it.text })
        }
    }

    @Test fun unscopedReasoningCannotBeReappliedToMultipleTerminalItems() = runBlocking {
        val result = runCatching { responseEvents(listOf(
            """{"type":"response.reasoning_text.delta","delta":"A"}""",
            """{"type":"response.completed","response":{"id":"response","output":[{"type":"reasoning","id":"r1","summary":[],"content":[{"type":"reasoning_text","text":"A"}]},{"type":"reasoning","id":"r2","summary":[],"content":[{"type":"reasoning_text","text":"B"}]}]}}""")) }
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }

    @Test fun emptySummaryFallsBackToVisiblePlaintextAtTerminal() = runBlocking {
        val events = responseEvents(listOf(terminalReasoning("""[{"type":"reasoning_text","text":"visible reasoning"}]""")))
        assertEquals("visible reasoning", events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().single().text)
        assertEquals("answer", events.filterIsInstance<LlmStreamEvent.TextDelta>().single().text)
    }

    @Test fun terminalReasoningAppendsOnlyMissingSuffix() = runBlocking {
        val events = responseEvents(listOf(
            """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"prefix "}""",
            terminalReasoning("""[{"type":"reasoning_text","text":"prefix suffix"}]""")))
        assertEquals(listOf("prefix ", "suffix"), events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().map { it.text })
    }

    @Test fun encryptedReasoningRemainsContinuationOnly() = runBlocking {
        val events = responseEvents(listOf(terminalReasoning("[]")))
        assertTrue(events.none { it is LlmStreamEvent.ReasoningDelta })
        assertEquals(listOf("answer"), events.filterIsInstance<LlmStreamEvent.TextDelta>().map { it.text })
    }

    @Test fun streamedSummaryAndPlaintextForSameItemAreNotDisplayedTwice() = runBlocking {
        val events = responseEvents(listOf(
            """{"type":"response.reasoning_summary_text.delta","item_id":"r1","output_index":0,"delta":"summary"}""",
            """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"separate raw representation"}""",
            terminalReasoning("""[{"type":"reasoning_text","text":"separate raw representation"}]""", summary = """[{"type":"summary_text","text":"summary"}]""")))
        assertEquals(listOf("summary"), events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().map { it.text })
    }

    @Test fun conflictingReasoningSnapshotIsRejectedRatherThanDuplicated() = runBlocking {
        val result = runCatching { responseEvents(listOf(
            """{"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"first"}""",
            terminalReasoning("""[{"type":"reasoning_text","text":"different"}]"""))) }
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }

    @Test fun observedReasoningIsTransportedEvenWhenReasoningGenerationWasNotRequested() = runBlocking {
        val client = HttpClient(MockEngine {
            respond("data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Provider supplied summary\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n", headers = headersOf("Content-Type", "text/event-stream"))
        }) { install(ContentNegotiation) { json() } }
        try {
            val events = OpenAiProvider(client).apply { config = config().copy(wireProtocol = ModelWireProtocol.ChatCompletions) }
                .generateEventStream(ModelInput(messages = listOf(mapOf("role" to "user", "content" to "fixture")),
                    userPrefs = mapOf("enable_thinking" to "false"))).toList()
            assertEquals("Provider supplied summary", events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().single().text)
            assertEquals(1, events.filterIsInstance<LlmStreamEvent.Completed>().size)
        } finally { client.close() }
    }

    @Test fun unsupportedExplicitGenerationOptionsAreRejectedNotDropped() {
        val input = ModelInput(messages = listOf(mapOf("role" to "user", "content" to "fixture")))
        for (configured in listOf(config().copy(seed = 0), config().copy(repetitionPenalty = 1.0),
            config().copy(presencePenalty = 0.2), config().copy(frequencyPenalty = -0.2),
            config().copy(stopSequences = "END"), config().copy(sendTopK = true, topK = 40))) {
            assertTrue(runCatching { responsesRequestBody(input, configured) }.exceptionOrNull() is IllegalArgumentException)
        }
    }

    @Test
    fun chatCompletionsPartialUsageDoesNotBecomeCompleteTotal() = runBlocking {
        val client = HttpClient(MockEngine {
            respond("data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":null,\"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n", headers = headersOf("Content-Type", "text/event-stream"))
        }) { install(ContentNegotiation) { json() } }
        try {
            val events = OpenAiProvider(client).apply { config = config().copy(wireProtocol = ModelWireProtocol.ChatCompletions) }.generateEventStream(ModelInput(messages = listOf(mapOf("role" to "user", "content" to "fixture")))).toList()
            val usage = events.filterIsInstance<LlmStreamEvent.Usage>().single().usage
            assertEquals(12, usage.promptTokens)
            assertEquals(0, usage.cachedInputTokens)
            assertEquals(null, usage.completionTokens)
            assertEquals(null, usage.totalTokens)
            assertEquals(null, usage.reasoningTokens)
        } finally { client.close() }
    }
    @Test
    fun missingPartialNullAndZeroProviderUsageAreDistinct() = runBlocking {
        for (raw in listOf("null", "{}", "{\"input_tokens\":12}", "{\"input_tokens\":null,\"total_tokens\":null}", "{\"input_tokens\":0,\"output_tokens\":0,\"total_tokens\":0}")) {
            val client = HttpClient(MockEngine {
                respond("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"fixture\",\"status\":\"completed\",\"output\":[],\"usage\":$raw}}\n\n", headers = headersOf("Content-Type", "text/event-stream"))
            }) { install(ContentNegotiation) { json() } }
            try {
                val events = OpenAiProvider(client).apply { config = config() }.generateEventStream(ModelInput(messages = listOf(mapOf("role" to "user", "content" to "fixture")))).toList()
                val usage = events.filterIsInstance<LlmStreamEvent.Usage>().singleOrNull()?.usage
                if (raw.contains("\"total_tokens\":0")) assertEquals(0, usage?.totalTokens)
                else assertEquals(null, usage?.totalTokens)
                assertEquals(null, usage?.reasoningTokens)
                if (raw.contains(":12")) assertEquals(12, usage?.promptTokens)
            } finally { client.close() }
        }
    }
    @Test fun resolvedSummaryCapabilityDoesNotOverrideEffortOrThinkingPreference() {
        for (effortValue in listOf("high", "none")) {
            val effort = buildJsonObject { put("kind", "effort"); put("value", effortValue) }
            for (support in listOf(null, false, true)) {
                val runtime = buildJsonObject {
                    put("reasoning_selection", effort)
                    put("model_capabilities", buildJsonObject {
                        put("reasoning_summary_supported", support?.let { kotlinx.serialization.json.JsonPrimitive(it) }
                            ?: kotlinx.serialization.json.JsonNull)
                    })
                }
                val configured = config().copy(executionCapabilities = ModelExecutionCapabilities(reasoning = true))
                    .withResolvedRuntime(runtime)
                assertEquals(support, configured.reasoningSummarySupported)
                for (thinking in listOf(false, true)) {
                    val body = responsesRequestBody(ModelInput(messages = listOf(mapOf("role" to "user", "content" to "fixture")),
                        userPrefs = mapOf("enable_thinking" to thinking.toString())), configured)
                    val reasoning = body.getValue("reasoning").jsonObject
                    assertEquals(effortValue, reasoning.getValue("effort").jsonPrimitive.content)
                    assertEquals(thinking && support != false && effortValue != "none", "summary" in reasoning)
                }
                assertEquals(null, configured.withResolvedRuntime(buildJsonObject {
                    put("model_capabilities", buildJsonObject { put("reasoning_summary_supported", kotlinx.serialization.json.JsonNull) })
                }).reasoningSummarySupported)
            }
        }
    }

    private fun config() = OpenAiProvider.Config(
        baseUrl = "https://fixture.invalid/v1", apiKey = "fixture", model = "response-model",
        wireProtocol = ModelWireProtocol.Responses,
        executionCapabilities = ModelExecutionCapabilities(toolCalls = true),
    )

    @Test
    fun requestAndStreamPreserveFunctionCallAndOpaqueContinuation() = runBlocking {
        var requests = 0
        val client = HttpClient(MockEngine { request ->
            requests++
            assertEquals("/v1/responses", request.url.encodedPath)
            val body = Json.parseToJsonElement(
                (request.body as OutgoingContent.ByteArrayContent).bytes().decodeToString()
            ).jsonObject
            assertEquals("16384", body["max_output_tokens"]?.jsonPrimitive?.content)
            assertEquals("false", body["store"]?.jsonPrimitive?.content)
            assertEquals("reasoning.encrypted_content", body["include"]?.jsonArray?.single()?.jsonPrimitive?.content)
            assertEquals("function", body["tools"]?.jsonArray?.single()?.jsonObject?.get("type")?.jsonPrimitive?.content)
            assertEquals("false", body["tools"]?.jsonArray?.single()?.jsonObject?.get("strict")?.jsonPrimitive?.content)
            assertFalse(body.containsKey("messages"))
            respond(
                """data: {"type":"response.output_text.delta","delta":"Checking."}

data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"function_call","call_id":"call_1","name":"read","arguments":"{\"path\":\"a\"}"}],"usage":{"input_tokens":12,"output_tokens":4,"total_tokens":16}}}

data: [DONE]

""",
                headers = headersOf("Content-Type", "text/event-stream"),
            )
        }) { install(ContentNegotiation) { json() } }
        try {
            val input = ModelInput(
                messages = listOf(mapOf("role" to "user", "content" to "Read a")),
                tools = listOf(Json.parseToJsonElement("""{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}""")),
            )
            val events = OpenAiProvider(client).apply { config = config() }.generateEventStream(input).toList()
            assertEquals("Checking.", (events.first() as LlmStreamEvent.TextDelta).text)
            val call = events.filterIsInstance<LlmStreamEvent.ToolCallDelta>().single()
            assertEquals("call_1", call.callId)
            assertEquals("read", call.capabilityId)
            val continuation = events.filterIsInstance<LlmStreamEvent.Continuation>().single().value
            assertEquals("openai.responses", continuation["provider_namespace"]?.jsonPrimitive?.content)
            assertEquals("opaque", continuation["state"]?.jsonObject?.get("response_items")?.jsonArray
                ?.first()?.jsonObject?.get("encrypted_content")?.jsonPrimitive?.content)
            assertTrue(events.last() is LlmStreamEvent.Completed)
            assertEquals(1, requests)

            val replay = responsesRequestBody(ModelInput(
                canonicalMessages = listOf(
                    buildJsonObject { put("role", "user"); put("content", "Read a") },
                    buildJsonObject { put("role", "assistant"); put("content", "Checking.") },
                    buildJsonObject { put("role", "tool"); put("tool_call_id", "turn::call_1"); put("content", "done") },
                ),
                providerContinuation = continuation,
            ), config())
            val replayItems = replay["input"]!!.jsonArray
            assertEquals("reasoning", replayItems[1].jsonObject["type"]?.jsonPrimitive?.content)
            assertEquals("function_call_output", replayItems.last().jsonObject["type"]?.jsonPrimitive?.content)
            assertEquals("call_1", replayItems.last().jsonObject["call_id"]?.jsonPrimitive?.content)
        } finally { client.close() }
    }

    @Test
    fun wrongRouteContinuationFailsBeforeNetwork() {
        val bad = buildJsonObject {
            put("provider_namespace", "openai.responses")
            put("model_id", "response-model")
            put("state", buildJsonObject { put("route_identity", "wrong") })
        }
        val result = runCatching { responsesRequestBody(ModelInput(
            messages = listOf(mapOf("role" to "user", "content" to "hello")),
            providerContinuation = bad,
        ), config()) }
        assertTrue(result.isFailure)
    }

    @Test
    fun replayKeepsOnlyOneReasoningRepresentationAndNormalizesEndpointCase() {
        assertEquals("https://fixture.invalid/V1/responses", openAiResponsesUrl("https://fixture.invalid/V1/CHAT/COMPLETIONS"))
        val config = config()
        val original = responsesRequestBody(ModelInput(messages = listOf(mapOf("role" to "user", "content" to "hello"))), config)
        assertEquals("hello", original["input"]?.jsonArray?.single()?.jsonObject?.get("content")?.jsonPrimitive?.content)
        val continuation = buildJsonObject {
            put("provider_namespace", "openai.responses")
            put("model_id", config.model)
            put("state", buildJsonObject {
                put("route_identity", originalRouteIdentityForTest(config))
                put("response_items", Json.parseToJsonElement("""[{"type":"reasoning","encrypted_content":"opaque","summary":[],"content":[{"type":"reasoning_text","text":"plain"}]}]"""))
            })
        }
        val body = responsesRequestBody(ModelInput(canonicalMessages = listOf(
            buildJsonObject { put("role", "user"); put("content", "hello") },
            buildJsonObject { put("role", "assistant"); put("content", "done") },
        ), providerContinuation = continuation), config)
        val replay = body["input"]!!.jsonArray.last().jsonObject
        assertFalse(replay.containsKey("encrypted_content"))
        assertFalse(replay.containsKey("summary"))
    }

    @Test
    fun sequentialCoreToolBatchesRetainTheirCallsWhenLatestResponseIsReplayed() {
        val items = Json.parseToJsonElement("""[
            {"type":"message","role":"user","content":"Read the fictional record"},
            {"type":"tool_call","call_id":"turn::connect","name":"mcp_connect","arguments":{}},
            {"type":"tool_result","call_id":"turn::connect","name":"mcp_connect","output":{"connected":true}},
            {"type":"tool_call","call_id":"turn::lookup","name":"lookup","arguments":{"record":"robot"}},
            {"type":"tool_result","call_id":"turn::lookup","name":"lookup","output":{"value":43}}
        ]""").jsonArray
        val messages = com.nola.core.androidCoreModelMessages(items)
        val config = config()
        val continuation = buildJsonObject {
            put("provider_namespace", "openai.responses"); put("model_id", config.model)
            put("state", buildJsonObject {
                put("route_identity", originalRouteIdentityForTest(config))
                put("response_items", Json.parseToJsonElement("""[{"type":"function_call","call_id":"lookup","name":"lookup","arguments":"{\"record\":\"robot\"}"}]"""))
            })
        }
        val input = com.nola.core.AgentRequestModelBuilder().build(com.nola.core.AgentModelRequest(
            sessionId="session", turnId="turn", iteration=3, messages=messages,
            tools=emptyList(), reservedOutputTokens=1024, providerContinuation=continuation), false).modelInput
        val wire = responsesRequestBody(input, config)["input"]!!.jsonArray.map { it.jsonObject }
        val calls = wire.filter { it["type"]?.jsonPrimitive?.content == "function_call" }
            .map { it["call_id"]!!.jsonPrimitive.content }
        val results = wire.filter { it["type"]?.jsonPrimitive?.content == "function_call_output" }
            .map { it["call_id"]!!.jsonPrimitive.content }
        assertEquals(listOf("turn::connect", "lookup"), calls)
        assertEquals(calls, results)
        assertEquals(listOf("user", "assistant", "tool", "assistant", "tool"), messages.map { it.role })
    }

    private fun originalRouteIdentityForTest(config: OpenAiProvider.Config): String =
        java.security.MessageDigest.getInstance("SHA-256")
            .digest("${config.baseUrl.trim().trimEnd('/')}\n${config.model}\n${config.apiKey}".toByteArray())
            .joinToString("") { "%02x".format(it.toInt() and 0xff) }
}
