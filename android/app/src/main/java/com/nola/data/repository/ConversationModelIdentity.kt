package com.nola.data.repository

import com.nola.data.local.CanonicalModelProfile
import com.nola.domain.model.ModelInfo

internal fun validateExecutionHostBinding(binding: com.nola.domain.model.ExecutionHostBinding) {
    val uri = java.net.URI(binding.baseURL)
    require(uri.scheme in setOf("http", "https") && !uri.host.isNullOrBlank() && uri.userInfo == null &&
        uri.rawQuery == null && uri.rawFragment == null) { "Host binding must be a credential-free HTTP(S) URL" }
    require(binding.tlsCertificateSHA256 == null || binding.tlsCertificateSHA256.matches(Regex("[a-fA-F0-9]{64}"))) {
        "Host certificate fingerprint is invalid"
    }
}

/** Wire names and connection entry points are not persistent profile identities. */
internal fun conversationModelIdentity(model: ModelInfo): String = when (model.source) {
    ModelInfo.ModelSource.Cloud -> "configured:${requireNotNull(model.profileId?.takeIf { it.isNotBlank() }) { "Select a configured model profile" }}"
    ModelInfo.ModelSource.Local -> "local:${model.id.also { require(it.isNotBlank()) }}"
    ModelInfo.ModelSource.WebHost -> error("Select a concrete Host model; the Host connection is not a model")
}

internal fun resolveConversationModel(identity: String?, profile: (String) -> CanonicalModelProfile?,
    localModels: List<ModelInfo>): ModelInfo? = when {
    identity?.startsWith("configured:") == true -> profile(identity.removePrefix("configured:"))
        ?.takeIf { it.id == identity.removePrefix("configured:") && it.enabled && it.deletedAtMs == null && it.runtime == "remote" }
        ?.toConfiguredModelInfo()
    identity?.startsWith("local:") == true -> localModels.singleOrNull {
        it.source == ModelInfo.ModelSource.Local && it.id == identity.removePrefix("local:")
    }
    else -> null
}
