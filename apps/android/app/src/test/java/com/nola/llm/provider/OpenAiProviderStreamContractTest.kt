package com.nola.llm.provider

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelWireProtocol
import com.nola.llm.LlmStreamEvent
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import io.ktor.serialization.kotlinx.json.json
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test

class OpenAiProviderStreamContractTest {
    private val sampleInput = ModelInput(
        messages = listOf(mapOf("role" to "user", "content" to "Hello")),
    )

    @Test
    fun mergeStreamedToolNamePreservesRepeatedCharactersAndHandlesFullNameRepeats() {
        assertEquals("add_event", listOf("ad", "d", "_event").fold("", ::mergeStreamedToolName))
        assertEquals("aa_tool", listOf("a", "a", "_tool").fold("", ::mergeStreamedToolName))
        assertEquals("set_timer", listOf("set_", "t", "imer").fold("", ::mergeStreamedToolName))
        assertEquals("get_weather", listOf("get_", "get_weather").fold("", ::mergeStreamedToolName))
        assertEquals("get_weather", listOf("get_weather", "get_weather").fold("", ::mergeStreamedToolName))
    }

    @Test
    fun generateSummaryHandlesEmptyChoicesWithoutCrashing() = runBlocking {
        val client = HttpClient(MockEngine {
            respond(
                content = """{"id":"chatcmpl-1","choices":[],"usage":{"total_tokens":5}}""",
                status = HttpStatusCode.OK,
                headers = headersOf(HttpHeaders.ContentType, ContentType.Application.Json.toString()),
            )
        }) { install(ContentNegotiation) { json() } }
        try {
            val provider = OpenAiProvider(client).apply {
                config = OpenAiProvider.Config(
                    baseUrl = "https://api.example.test/v1",
                    apiKey = "test-key",
                    model = "gpt-test",
                )
            }
            val output = provider.generateSummary(sampleInput)
            assertEquals("", output.text)
            assertEquals("unknown", output.finishReason)
            assertEquals(5, output.tokensUsed)
        } finally {
            client.close()
        }
    }

    @Test
    fun generateSummaryStreamAndEventStreamPreserveLiteralNullTokens() = runBlocking {
        val sse = listOf(
            """data: {"choices":[{"delta":{"content":"return "}}]}""",
            """data: {"choices":[{"delta":{"content":"null"}}]}""",
            """data: {"choices":[{"delta":{"content":";"},"finish_reason":"stop"}]}""",
            """data: [DONE]""",
        ).joinToString("\n") + "\n"
        val client = HttpClient(MockEngine {
            respond(
                content = sse,
                status = HttpStatusCode.OK,
                headers = headersOf(HttpHeaders.ContentType, ContentType.Text.EventStream.toString()),
            )
        }) { install(ContentNegotiation) { json() } }
        try {
            val provider = OpenAiProvider(client).apply {
                config = OpenAiProvider.Config(
                    baseUrl = "https://api.example.test/v1",
                    apiKey = "test-key",
                    model = "gpt-test",
                )
            }
            val summaryText = provider.generateSummaryStream(sampleInput).toList().joinToString("")
            assertEquals("return null;", summaryText)

            val eventsText = provider.generateEventStream(sampleInput)
                .toList()
                .filterIsInstance<LlmStreamEvent.TextDelta>()
                .joinToString("") { it.text }
            assertEquals("return null;", eventsText)
        } finally {
            client.close()
        }
    }

    @Test
    fun generateSummaryStreamWrapsResponsesReasoningInSingleThinkBlock() = runBlocking {
        val sse = listOf(
            """data: {"type":"response.created","response":{"output":[]}}""",
            """data: {"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"Step 1. "}""",
            """data: {"type":"response.reasoning_text.delta","item_id":"r1","output_index":0,"delta":"Step 2."}""",
            """data: {"type":"response.output_text.delta","delta":"Answer"}""",
            """data: {"type":"response.completed","response":{"id":"resp-1","status":"completed","output":[{"type":"reasoning","id":"r1","content":[{"type":"reasoning_text","text":"Step 1. Step 2."}]},{"type":"message","content":[{"type":"output_text","text":"Answer"}]}]}}""",
        ).joinToString("\n\n") + "\n\n"
        val client = HttpClient(MockEngine {
            respond(
                content = sse,
                status = HttpStatusCode.OK,
                headers = headersOf(HttpHeaders.ContentType, ContentType.Text.EventStream.toString()),
            )
        }) { install(ContentNegotiation) { json() } }
        try {
            val provider = OpenAiProvider(client).apply {
                config = OpenAiProvider.Config(
                    baseUrl = "https://api.example.test/v1",
                    apiKey = "test-key",
                    model = "gpt-test",
                    wireProtocol = ModelWireProtocol.Responses,
                )
            }
            val summary = provider.generateSummaryStream(sampleInput).toList().joinToString("")
            assertEquals("<think>Step 1. Step 2.</think>\nAnswer", summary)
        } finally {
            client.close()
        }
    }
}
