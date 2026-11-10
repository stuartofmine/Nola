package com.nola.core

import com.nola.data.repository.ApiModelConfig
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.domain.model.ModelParameters
import com.nola.llm.LlmClient
import com.nola.llm.provider.OpenAiProvider
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flow
import kotlinx.serialization.json.JsonElement
import kotlin.time.Duration

/** A turn owns its model route, parameters and client for every tool-loop iteration. */
data class AndroidTurnModelSnapshot(
    val model: ModelInfo,
    val parameters: ModelParameters,
    val client: LlmClient,
    internal val route: kotlinx.serialization.json.JsonObject? = null,
    internal val resolvedRuntime: kotlinx.serialization.json.JsonObject? = null,
    internal val editedResendModel: com.nola.data.local.AndroidFrozenResendModel? = null,
)

/** Tool authority remains in Core; the model sees schemas only when its route supports calls. */
internal fun AndroidTurnModelSnapshot.modelFacingTools(plannedTools: List<JsonElement>): List<JsonElement> =
    if (!model.executionCapabilities.toolCalls) emptyList()
    else plannedTools

internal fun cloudTurnModelSnapshot(
    model: ModelInfo,
    parameters: ModelParameters,
    endpoint: ApiModelConfig,
    provider: OpenAiProvider,
): AndroidTurnModelSnapshot {
    require(model.source == ModelInfo.ModelSource.Cloud && model.id.isNotBlank())
    val config = OpenAiProvider.Config(
        baseUrl = endpoint.endpoint,
        apiKey = endpoint.apiKey,
        model = model.id,
        wireProtocol = endpoint.wireProtocol,
        executionCapabilities = model.executionCapabilities,
        temperature = parameters.temperature,
        topP = parameters.topP,
        maxCompletionTokens = parameters.maxCompletionTokens,
    ).withParameters(parameters).withResolvedRuntime(endpoint.resolvedRuntime)
    return AndroidTurnModelSnapshot(model, parameters, provider.clientForTurn(config), androidHttpModelRoute(config), resolvedRuntime=endpoint.resolvedRuntime)
}

/** Shared on-device engines cannot silently retarget a captured turn. */
internal class GuardedTurnModelClient(
    private val delegate: LlmClient,
    private val validate: suspend () -> Unit,
) : LlmClient {
    override val supportsNativeEventStream get() = delegate.supportsNativeEventStream
    override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
        validate()
        return delegate.generateSummary(input, timeout)
    }
    override fun generateSummaryStream(input: ModelInput) = flow {
        validate()
        emitAll(delegate.generateSummaryStream(input))
    }
    override fun generateEventStream(input: ModelInput) = flow {
        validate()
        emitAll(delegate.generateEventStream(input))
    }
    override suspend fun chat(message: String, systemPrompt: String?, enableThinking: Boolean, timeout: Duration?): String {
        validate()
        return delegate.chat(message, systemPrompt, enableThinking, timeout)
    }
}
