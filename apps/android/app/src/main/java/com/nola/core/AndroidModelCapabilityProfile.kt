package com.nola.core

import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.*

/** Frozen declaration from model metadata intersected with this adapter's transport. */
data class AndroidModelCapabilityProfile(
    val id: String,
    val revision: Long,
    val nativeToolCalls: Boolean,
    val reasoning: Boolean,
    val visionInput: Boolean,
    val nativeEventStream: Boolean,
    val contextWindowTokens: Int,
    val maxOutputTokens: Int,
) {
    init {
        require(id.isNotBlank() && id == id.trim())
        require(revision > 0 && contextWindowTokens > 0 && maxOutputTokens > 0)
    }

    fun wire(): JsonObject = buildJsonObject {
        put("profile_id", id)
        put("revision", revision)
        put("instruction_profile", "generic")
        put("native_tool_calls", nativeToolCalls)
        // No Android metadata currently proves these transport capabilities.
        put("parallel_tool_calls", false)
        put("reasoning", reasoning)
        put("reasoning_continuity", false)
        put("prompt_cache", false)
        put("vision_input", visionInput)
        put("native_pdf_input", false)
        put("context_window_tokens", contextWindowTokens)
        put("max_output_tokens", maxOutputTokens)
        put("request_continuation", false)
        put("max_request_continuations", 0)
        put("stream_text", nativeEventStream)
        // Native OpenAI Chat/Responses parsers accept emitted reasoning even
        // when the model catalog cannot configure reasoning generation.
        put("stream_reasoning", nativeEventStream)
        put("stream_tool_calls", nativeToolCalls && nativeEventStream)
        put("direct_tool_capability_ids", JsonArray(emptyList()))
    }
}

internal fun androidModelCapabilityProfile(
    model: ModelInfo,
    parameters: ModelParameters,
    supportsNativeEventStream: Boolean,
): AndroidModelCapabilityProfile {
    require(model.selectionId.isNotBlank())
    return AndroidModelCapabilityProfile(
        id = "android-text-adapter-v1:${model.source.name}:${model.selectionId}",
        revision = model.profileRevision ?: 1,
        nativeToolCalls = model.executionCapabilities.toolCalls,
        reasoning = model.executionCapabilities.reasoning,
        visionInput = model.executionCapabilities.imageInput,
        nativeEventStream = supportsNativeEventStream,
        contextWindowTokens = parameters.maxContextTokens,
        maxOutputTokens = parameters.maxCompletionTokens,
    )
}

/** Current Android transport only: reject facts this renderer/adapter cannot preserve. */
internal fun frozenAndroidModelCapability(raw: JsonObject): AndroidModelCapabilityProfile {
    val profile = AndroidModelCapabilityProfile(
        id = raw.getValue("profile_id").jsonPrimitive.content,
        revision = raw.getValue("revision").jsonPrimitive.long,
        nativeToolCalls = raw.getValue("native_tool_calls").jsonPrimitive.boolean,
        reasoning = raw.getValue("reasoning").jsonPrimitive.boolean,
        visionInput = raw.getValue("vision_input").jsonPrimitive.boolean,
        nativeEventStream = raw.getValue("stream_text").jsonPrimitive.boolean,
        contextWindowTokens = raw.getValue("context_window_tokens").jsonPrimitive.int,
        maxOutputTokens = raw.getValue("max_output_tokens").jsonPrimitive.int,
    )
    require(profile.wire() == raw) { "The original model capability is unsupported by this Android Host" }
    return profile
}
