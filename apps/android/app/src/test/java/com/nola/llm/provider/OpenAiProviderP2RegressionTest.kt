package com.nola.llm.provider

import com.nola.llm.LlmRequestException
import io.ktor.client.HttpClient
import io.ktor.client.engine.mock.MockEngine
import io.ktor.client.engine.mock.respond
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.headersOf
import io.ktor.serialization.kotlinx.json.json
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.milliseconds

class OpenAiProviderP2RegressionTest {

    @Test
    fun responsesStreamFailureExtractsNestedResponseErrorCodeAndMessage() {
        val event = Json.parseToJsonElement(
            """
            {
              "type": "response.failed",
              "response": {
                "error": {
                  "code": "context_length_exceeded",
                  "message": "Your input exceeds the context window"
                }
              }
            }
            """.trimIndent()
        ).jsonObject

        val failure = responsesStreamFailure(event)
        assertTrue(failure is LlmRequestException)
        val requestError = failure as LlmRequestException
        assertEquals("context_length_exceeded", requestError.code)
        assertTrue(requestError.message.orEmpty().contains("context_length_exceeded"))
        assertTrue(requestError.message.orEmpty().contains("Your input exceeds the context window"))
    }

    @Test
    fun responsesStreamFailureExtractsTopLevelErrorDetails() {
        val event = Json.parseToJsonElement(
            """
            {
              "type": "error",
              "error": {
                "code": "rate_limit_exceeded",
                "message": "Too many requests"
              }
            }
            """.trimIndent()
        ).jsonObject

        val failure = responsesStreamFailure(event) as LlmRequestException
        assertEquals("rate_limit_exceeded", failure.code)
        assertEquals(429, failure.statusCode)
        assertTrue(failure.retryable)
        assertTrue(failure.message.orEmpty().contains("Too many requests"))
    }

    @Test
    fun openAiProviderChatEnforcesTimeout() = runBlocking {
        val engine = MockEngine {
            delay(5_000)
            respond(
                content = """{"choices":[{"message":{"content":"late"}}]}""",
                status = HttpStatusCode.OK,
                headers = headersOf(HttpHeaders.ContentType, "application/json"),
            )
        }
        val client = HttpClient(engine) {
            install(ContentNegotiation) { json() }
        }
        val provider = OpenAiProvider(client).apply {
            config = OpenAiProvider.Config(
                baseUrl = "https://example.test/v1",
                apiKey = "test-key",
                model = "gpt-4o",
            )
        }

        val outcome = runCatching {
            provider.chat("hello", timeout = 50.milliseconds)
        }
        assertTrue(outcome.exceptionOrNull() is TimeoutCancellationException)
    }
}
