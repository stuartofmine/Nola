package com.nola.llm.provider

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ChatOutputTokenField
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
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class OpenAiGenerationParametersTest {
    @Test fun summaryRetainsOutputLimitAndDoesNotInventMissingFinishReason() = runBlocking {
        for ((field, expected) in listOf(
            ",\"finish_reason\":\"length\"" to "length", "" to "unknown")) {
            val client = HttpClient(MockEngine {
                respond("{\"choices\":[{\"message\":{\"content\":\"partial checkpoint\"}$field}]}",
                    headers = headersOf("Content-Type", "application/json"))
            }) { install(ContentNegotiation) { json() } }
            try {
                val provider = OpenAiProvider(client).apply { config = config() }
                val result = provider.generateSummary(ModelInput(messages = listOf(mapOf("role" to "user", "content" to "source"))), null)
                assertEquals(expected, result.finishReason)
            } finally { client.close() }
        }
    }

    @Test fun frozenSummaryClientDoesNotFollowProviderReconfiguration() = runBlocking {
        val models = mutableListOf<String>()
        val http = HttpClient(MockEngine { request ->
            val body = Json.parseToJsonElement((request.body as OutgoingContent.ByteArrayContent).bytes().decodeToString()).jsonObject
            models += body.getValue("model").jsonPrimitive.content
            respond("""{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}""",
                headers = headersOf("Content-Type", "application/json"))
        }) { install(ContentNegotiation) { json() } }
        try {
            val provider = OpenAiProvider(http).apply { config = config().copy(model = "original") }
            val frozen = provider.clientForTurn(checkNotNull(provider.config).copy())
            val input = ModelInput(messages = listOf(mapOf("role" to "user", "content" to "segment")))
            frozen.generateSummary(input)
            provider.config = config().copy(model = "replacement", baseUrl = "https://other.invalid/v1")
            frozen.generateSummary(input)
            assertEquals(listOf("original", "original"), models)
        } finally { http.close() }
    }

    @Test fun explicitOptionsReachWireAndClearingRemovesThem() {
        val parameters = com.nola.domain.model.ModelParameters(topK = 40, repetitionPenalty = 1.0,
            presencePenalty = 0.3, frequencyPenalty = -0.1, seed = 0, stopSequences = "END\nSTOP")
        val body = buildJsonObject { putGenerationParameters(config().withParameters(parameters)) }
        assertEquals("40", body["top_k"]?.jsonPrimitive?.content)
        assertEquals("1.0", body["repetition_penalty"]?.jsonPrimitive?.content)
        assertEquals("0", body["seed"]?.jsonPrimitive?.content)
        assertEquals("[\"END\",\"STOP\"]", body["stop"].toString())
        val cleared = buildJsonObject { putGenerationParameters(config().withParameters(com.nola.domain.model.ModelParameters())) }
        assertTrue(listOf("top_k", "repetition_penalty", "seed", "stop").none(cleared::containsKey))
    }

    private fun config() = OpenAiProvider.Config(
        baseUrl = "https://fixture.invalid/v1", apiKey = "fixture", model = "fixture",
        executionCapabilities = ModelExecutionCapabilities(samplingParameters = true),
        temperature = 0f, topP = 0.875f, maxCompletionTokens = 64_000,
    )

    @Test
    fun allRequestModesSendConfiguredValuesWithoutClamping() = runBlocking {
        for (mode in listOf("single", "text", "events")) {
            var requests = 0
            val client = HttpClient(MockEngine { request ->
                requests++
                val raw = (request.body as OutgoingContent.ByteArrayContent).bytes().decodeToString()
                val body = Json.parseToJsonElement(raw).jsonObject
                assertEquals("64000", body["max_tokens"]?.jsonPrimitive?.content)
                assertEquals("0.0", body["temperature"]?.jsonPrimitive?.content)
                assertEquals("0.875", body["top_p"]?.jsonPrimitive?.content)
                assertEquals("/v1/chat/completions", request.url.encodedPath)
                if (mode == "single") respond("""{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}""", headers = headersOf("Content-Type", "application/json"))
                else respond("data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n", headers = headersOf("Content-Type", "text/event-stream"))
            }) { install(ContentNegotiation) { json() } }
            try {
                val provider = OpenAiProvider(client).apply { config = config() }
                val input = ModelInput(messages = listOf(mapOf("role" to "user", "content" to "hello")))
                when (mode) {
                    "single" -> assertEquals("ok", provider.generateSummary(input).text)
                    "text" -> assertEquals("ok", provider.generateSummaryStream(input).toList().joinToString(""))
                    else -> assertTrue(provider.generateEventStream(input).toList().isNotEmpty())
                }
                assertEquals(1, requests)
            } finally { client.close() }
        }
    }

    @Test
    fun invalidParametersFailInsteadOfBeingSilentlyChanged() {
        for (invalid in listOf(
            config().copy(maxCompletionTokens = 0), config().copy(maxCompletionTokens = 1_000_001),
            config().copy(temperature = Float.NaN), config().copy(topP = 1.5f),
            config().copy(sendTopK = true, topK = 0),
        )) {
            val result = runCatching { buildJsonObject { putGenerationParameters(invalid) } }
            assertTrue(result.exceptionOrNull() is IllegalArgumentException)
        }
    }

    @Test
    fun undeclaredSamplingIsOmittedAndUnsupportedOverrideFails() {
        val defaults = config().copy(executionCapabilities = ModelExecutionCapabilities(), temperature = 0.7f, topP = 0.9f)
        val body = buildJsonObject { putGenerationParameters(defaults) }
        assertTrue("temperature" !in body)
        assertTrue("top_p" !in body)
        assertTrue(runCatching { buildJsonObject { putGenerationParameters(defaults.copy(temperature = 0.2f)) } }
            .exceptionOrNull() is IllegalArgumentException)
    }

    @Test
    fun chatOutputLimitFieldUsesExactRouteOrExplicitModelDeclaration() {
        val generic = buildJsonObject { putGenerationParameters(config()) }
        assertTrue("max_tokens" in generic)
        assertTrue("max_completion_tokens" !in generic)
        val official = buildJsonObject { putGenerationParameters(config().copy(baseUrl = "https://api.openai.com/v1")) }
        assertTrue("max_completion_tokens" in official)
        assertTrue("max_tokens" !in official)
        val proxy = buildJsonObject { putGenerationParameters(config().copy(
            executionCapabilities = ModelExecutionCapabilities(
                samplingParameters = true, chatOutputTokenField = ChatOutputTokenField.MaxCompletionTokens))) }
        assertTrue("max_completion_tokens" in proxy)
        val legacyOfficial = buildJsonObject { putGenerationParameters(config().copy(
            baseUrl = "https://api.openai.com/v1",
            executionCapabilities = ModelExecutionCapabilities(
                samplingParameters = true, chatOutputTokenField = ChatOutputTokenField.MaxTokens))) }
        assertTrue("max_tokens" in legacyOfficial)
    }
}
