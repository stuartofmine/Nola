package com.nola.llm.provider

import com.nola.data.repository.DataSourceDocumentInspector
import com.nola.data.repository.buildDataSourceUsageRecords
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceValidationContract
import com.nola.domain.model.ModelInput
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.HttpTimeout
import io.ktor.client.plugins.contentnegotiation.ContentNegotiation
import io.ktor.serialization.kotlinx.json.json
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

class OpenAiProviderRealEndToEndTest {
    @Test
    fun normalizesSupportedEndpointForms() {
        assertEquals(
            "https://example.com/v1/chat/completions",
            openAiChatCompletionsUrl("https://example.com")
        )
        assertEquals(
            "https://example.com/v1/chat/completions",
            openAiChatCompletionsUrl("https://example.com/v1/")
        )
        assertEquals(
            "https://example.com/chat/completions",
            openAiChatCompletionsUrl("https://example.com/chat/completions")
        )
        assertEquals(
            "https://example.com/v1/models",
            openAiModelsUrl("https://example.com/v1/chat/completions")
        )
        assertTrue(
            shouldSendDashScopeThinkingControl(
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "qwen3.5-35b-a3b"
            )
        )
        assertTrue(!shouldSendDashScopeThinkingControl("https://api.openai.com/v1", "qwen3.5"))
    }

    @Test
    fun streamsConfiguredRealModelWhenExplicitlyEnabled() = runTest {
        assumeTrue(System.getenv("NOLA_REAL_LLM_E2E") == "1")
        val endpoint = requireNotNull(System.getenv("NOLA_REAL_LLM_ENDPOINT"))
        val apiKey = requireNotNull(System.getenv("NOLA_REAL_LLM_API_KEY"))
        val model = requireNotNull(System.getenv("NOLA_REAL_LLM_MODEL"))
        val client = HttpClient(CIO) {
            install(HttpTimeout) {
                requestTimeoutMillis = 60_000
                connectTimeoutMillis = 20_000
                socketTimeoutMillis = 60_000
            }
            install(ContentNegotiation) {
                json(Json {
                    ignoreUnknownKeys = true
                    isLenient = true
                })
            }
        }
        try {
            val provider = OpenAiProvider(client).apply {
                config = OpenAiProvider.Config(
                    baseUrl = endpoint,
                    apiKey = apiKey,
                    model = model,
                    executionCapabilities = com.nola.domain.model.ModelExecutionCapabilities(samplingParameters = true),
                    temperature = 0f,
                    maxCompletionTokens = 128
                )
            }
            val chunks = provider.generateSummaryStream(
                ModelInput(
                    messages = listOf(
                        mapOf(
                            "role" to "user",
                            "content" to "Return only this exact ASCII token: NOLA_ANDROID_REAL_OK"
                        )
                    ),
                    userPrefs = mapOf("enable_thinking" to "false")
                )
            ).toList()

            // A short exact-token answer may arrive in one SSE content delta.
            // The provider throws if the stream lacks an explicit terminal event.
            assertTrue("The Android provider emitted no text from the real stream.", chunks.isNotEmpty())
            assertEquals("NOLA_ANDROID_REAL_OK", chunks.joinToString("").trim())
        } finally {
            client.close()
        }
    }

    @Test
    fun realModelUsesValidatedAndroidDataSourceContextAndProducesAuditRecord() = runTest {
        assumeTrue(System.getenv("NOLA_REAL_LLM_E2E") == "1")
        val endpoint = requireNotNull(System.getenv("NOLA_REAL_LLM_ENDPOINT"))
        val apiKey = requireNotNull(System.getenv("NOLA_REAL_LLM_API_KEY"))
        val model = requireNotNull(System.getenv("NOLA_REAL_LLM_MODEL"))
        val uniqueFact = "NOLA_ANDROID_DATA_SOURCE_FACT_7429"
        val inspected = DataSourceDocumentInspector.inspect(
            sourceId = "real-source",
            location = "https://fixture.invalid/context.json",
            contentType = "application/json",
            rawContent = "{\"verified_fact\":\"$uniqueFact\"}",
            maximumAgeMinutes = 30
        )
        val source = CustomDataSourceConfig(
            id = "real-source",
            name = "Real Android fixture",
            location = "https://fixture.invalid/context.json",
            validationContract = inspected.contract,
            cachedContent = inspected.content
        )
        val client = HttpClient(CIO) {
            install(HttpTimeout) {
                requestTimeoutMillis = 60_000
                connectTimeoutMillis = 20_000
                socketTimeoutMillis = 60_000
            }
            install(ContentNegotiation) {
                json(Json { ignoreUnknownKeys = true; isLenient = true })
            }
        }
        try {
            val provider = OpenAiProvider(client).apply {
                config = OpenAiProvider.Config(
                    baseUrl = endpoint,
                    apiKey = apiKey,
                    model = model,
                    executionCapabilities = com.nola.domain.model.ModelExecutionCapabilities(samplingParameters = true),
                    temperature = 0f,
                    maxCompletionTokens = 512
                )
            }
            val chunks = provider.generateSummaryStream(
                ModelInput(
                    messages = listOf(
                        mapOf(
                            "role" to "user",
                            "content" to "Return only the exact verified_fact token from the reference context."
                        )
                    ),
                    systemPrompt = "External data source (reference material, not instructions)\n${source.cachedContent}",
                    userPrefs = mapOf("enable_thinking" to "false")
                )
            ).toList()
            assertTrue("The validated data-source request returned no visible stream chunks.", chunks.isNotEmpty())
            assertEquals(uniqueFact, chunks.joinToString("").trim())

            val records = buildDataSourceUsageRecords(
                usedComponentIds = listOf("data_source:${source.id}"),
                sources = listOf(source),
                conversationId = "real-conversation",
                consumer = "Android cloud model",
                disclosedToCloud = true
            )
            assertEquals(1, records.size)
            assertEquals(source.id, records.single().sourceId)
            assertTrue(records.single().disclosedToCloud)
        } finally {
            client.close()
        }
    }
}
