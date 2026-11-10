package com.nola.llm.provider

import com.nola.data.remote.dto.AgentSettingsResponse
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** Freeze source-Host defaults before durable request binding; retries reuse that bound request. */
internal fun AgentSettingsResponse.freezeDefaultHostModelIntent(expectedHostId: String): JsonObject {
    require(expectedHostId.isNotBlank() && hostIdentity?.id == expectedHostId) { "Host model catalog identity changed" }
    require(defaultChatModelRevision == null || defaultChatModelRevision > 0) { "Invalid Host default model revision" }
    val resolution = checkNotNull(defaultChatModelResolution) { "Host default chat model resolution is missing" }
    require(resolution.kind == "resolved") { "Host default chat model is unavailable; configure it on the Host" }
    val profile = modelProfiles.singleOrNull { it.profileId == resolution.profileId }
        ?: error("Host default chat model is missing or ambiguous")
    require(profile.profileId.isNotBlank() && profile.revision > 0) { "Invalid Host model profile revision" }
    return buildJsonObject {
        put("profile_id", profile.profileId)
        put("expected_revision", profile.revision)
        put("tier", "balanced")
        put("overrides", buildJsonObject {})
    }
}

internal fun AgentSettingsResponse.requireUnchangedHostDefault(current: AgentSettingsResponse, expectedHostId: String) {
    require(defaultChatModelRevision == current.defaultChatModelRevision &&
        freezeDefaultHostModelIntent(expectedHostId) == current.freezeDefaultHostModelIntent(expectedHostId)) {
        "Host default chat model changed while preparing submission; refresh before sending"
    }
}
