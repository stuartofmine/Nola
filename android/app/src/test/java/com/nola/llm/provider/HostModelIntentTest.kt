package com.nola.llm.provider

import com.nola.data.remote.dto.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.*
import org.junit.Test

class HostModelIntentTest {
    private val settings = AgentSettingsResponse(
        hostIdentity = HostIdentityDto("source", HOST_PERMISSION_PROTOCOL_VERSION),
        modelProfiles = listOf(HostModelProfileDto("configured-model", 7, "Model", "upstream")),
        defaultChatModelResolution = HostModelResolutionDto("resolved", "configured-model"),
    )

    @Test fun configProjectionFreezesOnlyHostProfileIdentityAndRevision() {
        val decoded = Json { ignoreUnknownKeys = true }.decodeFromString<AgentSettingsResponse>("""
            {"host_identity":{"id":"source","permission_protocol_version":4},
             "model_profiles":[{"profile_id":"configured-model","revision":7,"display_name":"Model","model_name":"upstream"}],
             "default_chat_model_resolution":{"kind":"resolved","profile_id":"configured-model","origin":"default"}}
        """.trimIndent())
        val frozen = decoded.freezeDefaultHostModelIntent("source")
        assertEquals(setOf("profile_id", "expected_revision", "tier", "overrides"), frozen.keys)
        assertEquals("7", frozen["expected_revision"]!!.jsonPrimitive.content)
        assertEquals("balanced", frozen["tier"]!!.jsonPrimitive.content)
        assertEquals(settings.freezeDefaultHostModelIntent("source"), frozen)
        assertEquals("7", frozen["expected_revision"]!!.jsonPrimitive.content)
    }

    @Test fun defaultRevisionChangesDoNotSilentlyRetargetPreparedSubmission() {
        settings.requireUnchangedHostDefault(settings, "source")
        assertTrue(runCatching { settings.requireUnchangedHostDefault(settings.copy(defaultChatModelRevision = 2), "source") }.isFailure)
        assertTrue(runCatching { settings.requireUnchangedHostDefault(settings.copy(modelProfiles = settings.modelProfiles.map { it.copy(revision = 8) }), "source") }.isFailure)
    }

    @Test fun unavailableStaleIdentityAndAmbiguousCatalogNeverUseRawModelFallback() {
        for (candidate in listOf(
            settings.copy(defaultChatModelResolution = HostModelResolutionDto("unavailable", "configured-model")),
            settings.copy(defaultChatModelResolution = null),
            settings.copy(modelProfiles = emptyList()),
            settings.copy(modelProfiles = settings.modelProfiles + settings.modelProfiles),
            settings.copy(modelProfiles = settings.modelProfiles.map { it.copy(revision = 0) }),
            settings.copy(hostIdentity = HostIdentityDto("other", HOST_PERMISSION_PROTOCOL_VERSION)),
        )) {
            assertTrue(runCatching { candidate.freezeDefaultHostModelIntent("source") }.isFailure)
        }
    }
}
