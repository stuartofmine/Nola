package com.nola.domain.model

import kotlinx.serialization.Serializable

/** Sparse per-conversation limits. Null inherits the selected model profile. */
@Serializable
data class ConversationGenerationOverride(
    val maxContextLength: Int? = null,
    val maxOutputTokens: Int? = null,
) {
    fun applyingTo(defaults: ModelParameters): ModelParameters = defaults.copy(
        maxContextTokens = maxContextLength ?: defaults.maxContextTokens,
        maxCompletionTokens = maxOutputTokens ?: defaults.maxCompletionTokens,
    )

    fun validate(): ConversationGenerationOverride {
        require(maxContextLength == null || maxContextLength in ModelParameterLimits.remoteContextTokens)
        require(maxOutputTokens == null || maxOutputTokens in ModelParameterLimits.outputTokens)
        return this
    }

    companion object {
        fun modelKeyOrNull(model: ModelInfo): String? = when (model.source) {
            ModelInfo.ModelSource.Local -> "local:${model.id}"
            ModelInfo.ModelSource.Cloud -> model.profileId?.takeIf(String::isNotBlank)?.let { "configured:$it" }
            ModelInfo.ModelSource.WebHost -> null
        }

        fun modelKey(model: ModelInfo): String = requireNotNull(modelKeyOrNull(model)) {
            "Configured model profile identity is required for conversation overrides"
        }
    }
}
