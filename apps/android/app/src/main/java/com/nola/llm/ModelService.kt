package com.nola.llm

import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.llm.provider.PlaceholderProvider
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.time.Duration

@Singleton
class ModelService @Inject constructor(
    private val placeholder: PlaceholderProvider
) : LlmClient {

    private var activeProvider: LlmClient = placeholder
    private var activeModelId: String = "placeholder"
    private var activeModelSource: String = "placeholder"
    private var activeProfileId: String? = null

    @Synchronized fun reconfigure(
        provider: LlmClient,
        modelId: String = "custom",
        modelSource: String = "cloud",
        profileId: String? = null,
    ) {
        activeProvider = provider
        activeModelId = modelId
        activeModelSource = modelSource
        activeProfileId = profileId
    }

    @Synchronized fun usePlaceholder() {
        activeProvider = placeholder
        activeModelId = "placeholder"
        activeModelSource = "placeholder"
        activeProfileId = null
    }

    data class SummarySnapshot(val client:LlmClient,val modelId:String,val profileId:String?,val configurationDigest:String)
    @Synchronized fun captureSummarySnapshot():SummarySnapshot {
        val original=activeProvider
        val (client,identity)=when(original) {
            is com.nola.llm.provider.OpenAiProvider -> {
                val configuration=checkNotNull(original.config).copy()
                require(configuration.model==activeModelId) {"Model selection is changing"}
                original.clientForTurn(configuration) to configuration.toString()
            }
            is com.nola.llm.provider.WebAgentProvider -> original.frozenSummaryClient()
            is com.nola.llm.provider.LocalLlamaProvider -> {
                val model=original.activeModel?.id;val parameters=original.parameters.copy()
                com.nola.core.GuardedTurnModelClient(original) {
                    check(original.activeModel?.id==model && original.parameters==parameters) {"The summary model instance changed"}
                } to "$model:$parameters"
            }
            else -> error("The selected provider cannot freeze a summary operation")
        }
        val digest=java.security.MessageDigest.getInstance("SHA-256").digest(("$activeProfileId:$activeModelId:"+identity).toByteArray()).joinToString("") {"%02x".format(it)}
        return SummarySnapshot(client,activeModelId,activeProfileId,digest)
    }

    val currentModelId: String get() = activeModelId
    val currentModelSource: String get() = activeModelSource
    val currentProfileId: String? get() = activeProfileId
    val isPlaceholder: Boolean get() = activeProvider is PlaceholderProvider

    override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
        return retry(2) {
            activeProvider.generateSummary(input, timeout)
        }
    }

    override fun generateSummaryStream(input: ModelInput): Flow<String> {
        return activeProvider.generateSummaryStream(input)
    }

    override val supportsNativeEventStream: Boolean
        get() = activeProvider.supportsNativeEventStream

    override fun generateEventStream(input: ModelInput): Flow<LlmStreamEvent> {
        return activeProvider.generateEventStream(input)
    }

    override suspend fun chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Boolean,
        timeout: Duration?
    ): String {
        return activeProvider.chat(message, systemPrompt, enableThinking, timeout)
    }

    private suspend fun <T> retry(maxRetries: Int, block: suspend () -> T): T {
        var lastError: Throwable? = null
        repeat(maxRetries) {
            try {
                return block()
            } catch (e: Exception) {
                lastError = e
            }
        }
        throw lastError ?: RuntimeException("Retry failed")
    }
}
