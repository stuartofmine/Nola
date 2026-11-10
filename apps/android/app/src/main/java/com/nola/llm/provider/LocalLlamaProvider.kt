package com.nola.llm.provider

import android.content.Context
import com.arm.aichat.AiChat
import com.arm.aichat.InferenceEngine
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelOutput
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.providerMessages
import com.nola.llm.LlmClient
import com.nola.llm.local.DeviceModelRecord
import com.nola.llm.local.DeviceModelStore
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.time.Duration
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.sync.Mutex
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

sealed interface LocalModelRuntimeState {
    data object Stopped : LocalModelRuntimeState
    data class Starting(val modelId: String) : LocalModelRuntimeState
    data class Ready(val modelId: String) : LocalModelRuntimeState
    data class Generating(val modelId: String) : LocalModelRuntimeState
    data class Failed(val modelId: String?, val message: String) : LocalModelRuntimeState
}

@Singleton
class LocalLlamaProvider @Inject constructor(
    @ApplicationContext context: Context,
    private val modelStore: DeviceModelStore,
) : LlmClient {
    private val engine: InferenceEngine = AiChat.getInferenceEngine(context)
    private val json = Json { prettyPrint = false }
    private val engineOperation = Mutex()
    private val _state = MutableStateFlow<LocalModelRuntimeState>(LocalModelRuntimeState.Stopped)
    val state: StateFlow<LocalModelRuntimeState> = _state.asStateFlow()

    var activeModel: DeviceModelRecord? = null
        private set
    var parameters: ModelParameters = ModelParameters()
        private set

    suspend fun start(model: DeviceModelRecord, parameters: ModelParameters) {
        require(parameters.maxContextTokens > 0 && parameters.maxContextTokens <= model.contextWindow) {
            "The requested context exceeds the installed model capacity"
        }
        check(engineOperation.tryLock()) { "The local model is executing another operation" }
        try {
            if (activeModel == model && this.parameters == parameters && _state.value is LocalModelRuntimeState.Ready) return
            stopLocked()
            _state.value = LocalModelRuntimeState.Starting(model.id)
            try {
                engine.loadModel(
                    pathToModel = modelStore.modelPath(model),
                    contextSize = parameters.maxContextTokens,
                    temperature = parameters.temperature,
                    topK = parameters.topK ?: 40,
                    topP = parameters.topP,
                )
                engine.setSystemPrompt(
                    "You are Nola. Follow the complete policy, conversation, context, and tool contracts supplied in each user turn."
                )
                this.activeModel = model
                this.parameters = parameters
                _state.value = LocalModelRuntimeState.Ready(model.id)
            } catch (error: Throwable) {
                engine.cleanUp()
                activeModel = null
                _state.value = LocalModelRuntimeState.Failed(model.id, error.message ?: error::class.java.simpleName)
                throw error
            }
        } finally { engineOperation.unlock() }
    }

    fun stop() {
        check(engineOperation.tryLock()) { "Wait for the local model operation before stopping" }
        try { stopLocked() } finally { engineOperation.unlock() }
    }

    private fun stopLocked() {
        val engineState = engine.state.value
        check(engineState !is InferenceEngine.State.Generating) { "Wait for generation to finish before stopping the model" }
        if (engineState is InferenceEngine.State.ModelReady || engineState is InferenceEngine.State.Error) {
            engine.cleanUp()
        }
        activeModel = null
        _state.value = LocalModelRuntimeState.Stopped
    }

    override fun generateSummaryStream(input: ModelInput): Flow<String> = generateBoundStream(input, null, null)

    private fun generateBoundStream(input: ModelInput, expectedModel: String?, expectedParameters: ModelParameters?): Flow<String> = flow {
        check(engineOperation.tryLock()) { "The local model is executing another operation" }
        try {
            if (expectedModel != null) check(activeModel?.id == expectedModel && parameters == expectedParameters) {
                "The captured model or parameters are no longer loaded"
            }
            val model = requireReady()
            _state.value = LocalModelRuntimeState.Generating(model.id)
            try {
                engine.sendUserPrompt(buildPrompt(input), parameters.maxCompletionTokens)
                    .collect { emit(it) }
                _state.value = LocalModelRuntimeState.Ready(model.id)
            } catch (error: Throwable) {
                _state.value = LocalModelRuntimeState.Failed(model.id, error.message ?: error::class.java.simpleName)
                throw error
            }
        } finally { engineOperation.unlock() }
    }

    /** Identity is rechecked while holding the same lock as reload and generation. */
    internal fun clientForTurn(modelId: String, parameters: ModelParameters): LlmClient = object : LlmClient {
        override fun generateSummaryStream(input: ModelInput) = generateBoundStream(input, modelId, parameters)
        override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
            val generate: suspend () -> String = { buildString { generateSummaryStream(input).collect(::append) } }
            return ModelOutput(text = if (timeout == null) generate() else withTimeout(timeout.inWholeMilliseconds) { generate() })
        }
        override suspend fun chat(message: String, systemPrompt: String?, enableThinking: Boolean, timeout: Duration?): String =
            generateSummary(ModelInput(messages = listOf(mapOf("role" to "user", "content" to message)), systemPrompt = systemPrompt), timeout).text
    }

    override suspend fun generateSummary(input: ModelInput, timeout: Duration?): ModelOutput {
        val generate: suspend () -> String = {
            buildString { generateSummaryStream(input).collect(::append) }
        }
        val text = if (timeout != null) withTimeout(timeout.inWholeMilliseconds) { generate() } else generate()
        return ModelOutput(text = text)
    }

    override suspend fun chat(
        message: String,
        systemPrompt: String?,
        enableThinking: Boolean,
        timeout: Duration?,
    ): String = generateSummary(
        ModelInput(messages = listOf(mapOf("role" to "user", "content" to message)), systemPrompt = systemPrompt),
        timeout,
    ).text

    internal fun buildPrompt(input: ModelInput): String = buildString {
        append("<nola_request>\n")
        append("<messages>")
        append(json.encodeToString(input.providerMessages()))
        append("</messages>\n")
        if (input.tools.isNotEmpty()) {
            append("<tools>")
            append(json.encodeToString(input.tools))
            append("</tools>\n")
        }
        append("Respond to the final user message. Use only the supplied tool contract when a tool is needed.\n")
        append("</nola_request>")
    }

    private fun requireReady(): DeviceModelRecord {
        val model = activeModel
        check(model != null && _state.value is LocalModelRuntimeState.Ready) {
            "No on-device model is running"
        }
        return model
    }
}
