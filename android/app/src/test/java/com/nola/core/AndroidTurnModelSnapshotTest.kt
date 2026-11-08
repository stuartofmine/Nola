package com.nola.core

import com.nola.data.repository.ApiModelConfig
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelParameters
import com.nola.llm.provider.OpenAiProvider
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
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.*
import org.junit.Test

class AndroidTurnModelSnapshotTest {
    @Test fun frozenGenerationPreservesEveryConfiguredOption() {
        val http = HttpClient(MockEngine { respond("{}") })
        try {
            val parameters = ModelParameters(topK = 40, repetitionPenalty = 1.0, presencePenalty = 0.3,
                frequencyPenalty = -0.2, seed = 0, stopSequences = "END\nSTOP")
            val snapshot = AndroidTurnModelSnapshot(ModelInfo("cloud", "Cloud", ModelInfo.ModelSource.Cloud),
                parameters, OpenAiProvider(http))
            val binding = buildJsonObject { put("generation", snapshot.invocationGeneration(true)) }
            val context = buildJsonObject {
                put("reserved_output_tokens", parameters.maxCompletionTokens)
                put("context_window_tokens", parameters.maxContextTokens)
            }
            assertEquals(parameters, frozenAndroidParameters(binding, context))
            assertEquals(JsonNull, snapshot.invocationGeneration(true).getValue("reasoning_summary_supported"))
            val frozen = snapshot.copy(resolvedRuntime = buildJsonObject {
                put("reasoning_tier", "detailed")
                put("model_capabilities", buildJsonObject { put("reasoning_summary_supported", false) })
            })
            assertEquals(JsonPrimitive("detailed"), frozen.invocationGeneration(true).getValue("reasoning_tier"))
            assertEquals(JsonPrimitive(false), frozen.invocationGeneration(true).getValue("reasoning_summary_supported"))
        } finally { http.close() }
    }

    @Test fun defaultToolSupportExposesSchemasAndExplicitDisableOverridesCloudAndLocalModels() {
        val http = HttpClient(MockEngine { respond("{}") })
        try {
            val schema = buildJsonObject { put("name", "fixture") }
            val client = OpenAiProvider(http)
            val cloud = AndroidTurnModelSnapshot(
                ModelInfo("cloud", "Cloud", ModelInfo.ModelSource.Cloud), ModelParameters(), client)
            assertEquals(JsonPrimitive("balanced"), cloud.invocationGeneration(thinking = false)["reasoning_tier"])
            assertEquals(JsonPrimitive("balanced"), cloud.invocationGeneration(thinking = true)["reasoning_tier"])
            assertEquals(listOf(schema), cloud.modelFacingTools(listOf(schema)))
            val disabledCloud = cloud.copy(model = cloud.model.copy(
                executionCapabilities = ModelExecutionCapabilities(toolCalls = false)))
            assertTrue(disabledCloud.modelFacingTools(listOf(schema)).isEmpty())
            val local = cloud.copy(model = ModelInfo("local", "Local", ModelInfo.ModelSource.Local))
            assertEquals(listOf(schema), local.modelFacingTools(listOf(schema)))
            val disabledLocal = local.copy(model = local.model.copy(
                executionCapabilities = ModelExecutionCapabilities(toolCalls = false)))
            assertTrue(disabledLocal.modelFacingTools(listOf(schema)).isEmpty())
        } finally { http.close() }
    }

    @Test fun restoredModelAndEveryToolIterationKeepCapturedRouteCredentialsAndParameters() = runBlocking {
        val bodies = mutableListOf<JsonObject>()
        val hosts = mutableListOf<String>()
        val authorizations = mutableListOf<String?>()
        val http = HttpClient(MockEngine { request ->
            bodies += Json.parseToJsonElement((request.body as OutgoingContent.ByteArrayContent).bytes().decodeToString()).jsonObject
            hosts += request.url.host
            authorizations += request.headers["Authorization"]
            respond("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-fixture\",\"type\":\"function\",\"function\":{\"name\":\"echo\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n",
                headers = headersOf("Content-Type", "text/event-stream"))
        }) { install(ContentNegotiation) { json() } }
        try {
            val shared = OpenAiProvider(http).apply {
                config = OpenAiProvider.Config("https://b.invalid/v1", "fixture-b", "model-b")
            }
            val execution = ModelExecutionCapabilities(toolCalls = true, samplingParameters = true)
            val a = cloudTurnModelSnapshot(ModelInfo("model-a", "A", ModelInfo.ModelSource.Cloud, profileId = "profile-a", executionCapabilities = execution),
                ModelParameters(temperature = 0f, topP = 0.5f, maxCompletionTokens = 256),
                ApiModelConfig("https://a.invalid/v1", "fixture-a", "global-default-b"), shared)
            val b = cloudTurnModelSnapshot(ModelInfo("model-b", "B", ModelInfo.ModelSource.Cloud, executionCapabilities = execution),
                ModelParameters(temperature = 1.5f, topP = 0.75f, maxCompletionTokens = 512),
                ApiModelConfig("https://b.invalid/v1", "fixture-b", "global-default-b"), shared)
            // Switching the mutable application provider after Send cannot retarget either turn.
            shared.config = OpenAiProvider.Config("https://c.invalid/v1", "fixture-c", "model-c")
            val aClient = TextAgentModelClient(a.client)
            suspend fun iteration(client: TextAgentModelClient, index: Int) {
                client.generateAgentTurn(AgentModelRequest(
                    sessionId = "fixture-session", turnId = "fixture-turn", iteration = index,
                    messages = listOf(AgentCoreMessage("user", "continue")),
                    tools = listOf(buildJsonObject {
                        put("type", "function")
                        put("function", buildJsonObject { put("name", "echo") })
                    }),
                    reservedOutputTokens = 128,
                    invocationPlan = AgentModelInvocationPlan(nativeToolCalls = true, streamToolCalls = true),
                )).toList()
            }
            iteration(aClient, 0)
            iteration(TextAgentModelClient(b.client), 0)
            iteration(aClient, 1)
            assertEquals(listOf("model-a", "model-b", "model-a"), bodies.map { it.getValue("model").jsonPrimitive.content })
            assertEquals(listOf("0.0", "1.5", "0.0"), bodies.map { it.getValue("temperature").jsonPrimitive.content })
            assertEquals(listOf("0.5", "0.75", "0.5"), bodies.map { it.getValue("top_p").jsonPrimitive.content })
            assertEquals(listOf("256", "512", "256"), bodies.map { it.getValue("max_tokens").jsonPrimitive.content })
            assertEquals(listOf("a.invalid", "b.invalid", "a.invalid"), hosts)
            assertEquals(listOf("Bearer fixture-a", "Bearer fixture-b", "Bearer fixture-a"), authorizations)
            assertEquals("model-c", shared.config?.model)
        } finally { http.close() }
    }

    @Test fun changedSharedEngineFailsBeforeAnyRequestCanUseSuccessorModel() = runBlocking {
        var requests = 0
        val http = HttpClient(MockEngine {
            requests++
            respond("""{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}""",
                headers = headersOf("Content-Type", "application/json"))
        }) { install(ContentNegotiation) { json() } }
        try {
            val provider = OpenAiProvider(http).apply {
                config = OpenAiProvider.Config("https://fixture.invalid/v1", "fixture", "a")
            }
            var activeModel = "a"
            val bound = GuardedTurnModelClient(provider) { check(activeModel == "a") { "Captured model is no longer loaded" } }
            bound.generateSummary(ModelInput())
            activeModel = "b"
            provider.config = provider.config?.copy(model = "b")
            assertTrue(runCatching { bound.generateSummary(ModelInput()) }.exceptionOrNull() is IllegalStateException)
            assertTrue(runCatching { bound.generateSummaryStream(ModelInput()).toList() }.exceptionOrNull() is IllegalStateException)
            assertTrue(runCatching { bound.generateEventStream(ModelInput()).toList() }.exceptionOrNull() is IllegalStateException)
            assertEquals(1, requests)
        } finally { http.close() }
    }
}
