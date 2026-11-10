package com.nola.data.repository

import com.nola.data.local.CanonicalModelProfile
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ConversationModelIdentityTest {
    @Test fun allGenerationFieldsSurviveSessionAndHostRoundTripAndClear() {
        val model = ModelInfo("wire", "Model", profileId = "profile")
        val value = ModelParameters(topK = 40, presencePenalty = 0.4, frequencyPenalty = -0.2,
            repetitionPenalty = 1.0, seed = 0, stopSequences = "END\nSTOP")
        val saved = writeConversationParameters(null, model, value)
        assertEquals(value, readConversationParameters(saved, model))
        val cleared = value.copy(topK = null, repetitionPenalty = null, seed = null, stopSequences = "")
        assertEquals(cleared, readConversationParameters(writeConversationParameters(saved, model, cleared), model))
        val host = initialHostSettings(com.nola.domain.model.ExecutionHostBinding("https://host.test"), value)
        assertEquals(value, readHostParameters(host.toString()))
    }

    @Test fun roomOwnershipProjectsWithoutChangingOrdinaryConversationsOrSettings() {
        val stored = """{"agentRoomOwner":{"room_id":"room","participant_id":"member","agent_id":"agent"},"sessionMemory":"original"}"""
        val settings = Json.decodeFromString<AndroidConversationSettings>(stored)
        assertEquals("room", settings.agentRoomOwner?.roomId)
        assertEquals("member", settings.agentRoomOwner?.participantId)
        val updated = transformConversationSettings(stored) { copy(sessionMemory = "edited") }
        assertEquals(Json.parseToJsonElement(stored).jsonObject["agentRoomOwner"],
            Json.parseToJsonElement(updated).jsonObject["agentRoomOwner"])
        assertNull(Json.decodeFromString<AndroidConversationSettings>("{}").agentRoomOwner)
    }
    private fun profile(id: String, enabled: Boolean = true) = CanonicalModelProfile(id, "Display", "openai",
        "same-wire", "remote", "{}", "{}", false, enabled, 7, 1, 1)

    @Test fun configuredIdentityNeverUsesWireNameOrHostConnection() {
        val first = ModelInfo("same-wire", "One", profileId = "profile-one")
        val second = first.copy(profileId = "profile-two")
        assertEquals("configured:profile-one", conversationParameterKey(first))
        assertEquals("configured:profile-two", conversationParameterKey(second))
        assertThrows(IllegalArgumentException::class.java) { conversationModelIdentity(first.copy(profileId = null)) }
        assertThrows(IllegalStateException::class.java) { conversationModelIdentity(first.copy(source = ModelInfo.ModelSource.WebHost)) }
    }

    @Test fun modelProjectionUsesCanonicalDirectoryNotPlatformSourceSnapshot() {
        val model = resolveConversationModel("configured:profile-one", { profile(it) }, emptyList())
        assertEquals("same-wire", model?.id)
        assertEquals("profile-one", model?.profileId)
        assertEquals(7L, model?.profileRevision)
        assertNull(resolveConversationModel("configured:missing", { null }, emptyList()))
        assertNull(resolveConversationModel("configured:disabled", { profile(it, false) }, emptyList()))
        assertNull(resolveConversationModel("configured:deleted", { profile(it).copy(deletedAtMs = 2) }, emptyList()))
        assertNull(resolveConversationModel("configured:wrong", { profile("different") }, emptyList()))
        assertNull(resolveConversationModel("connectedHost:host:profile", { profile(it) }, emptyList()))
    }

    @Test fun localIdentityRequiresExactInstalledModel() {
        val installed = ModelInfo("installed", "Same display", ModelInfo.ModelSource.Local)
        assertEquals(installed, resolveConversationModel("local:installed", { null }, listOf(installed)))
        assertNull(resolveConversationModel("local:missing", { null }, listOf(installed)))
    }

    @Test fun selectionAndParametersAreOneSparsePatchAndNoSerializedSecondIdentity() {
        val model = ModelInfo("same-wire", "Display", profileId = "profile-one")
        val patch = conversationModelSelection("""{"modelOverrides":{"configured:other":{"temperature":0.9}}}""",
            model, ModelParameters(temperature = 0.2f))
        assertEquals(setOf("kind", "model_id", "parameters", "presentation"), patch.keys)
        assertEquals("configured:profile-one", patch.getValue("model_id").jsonPrimitive.content)
        assertEquals("model", patch.getValue("kind").jsonPrimitive.content)
        assertEquals(0.2f, patch.getValue("parameters").jsonObject.getValue("temperature").jsonPrimitive.float)
        val domain = com.nola.domain.model.Conversation(modelSelectionID = "configured:profile-one", selectedModelInfo = model)
        assertFalse(Json.encodeToString(domain).contains("selectedModelInfo"))
        val decoded = Json { ignoreUnknownKeys = true }.decodeFromString<AndroidConversationSettings>(
            """{"selectedModelInfo":{"id":"stale","name":"Stale"}}""")
        assertNull(decoded.modelSelectionID)
    }

    @Test fun hostSelectionIsCredentialFreeBindingAndHostParametersNotFakeModel() {
        val binding = com.nola.domain.model.ExecutionHostBinding("http://192.168.1.2:8080")
        val parameters = ModelParameters(maxCompletionTokens = 8192, responseStyle = "professional", customSystemPrompt = "instruction")
        val patch = initialHostSettings(binding, parameters)
        assertFalse(patch.containsKey("modelSelectionID"))
        assertFalse(patch.containsKey("modelOverrides"))
        assertEquals(parameters, readHostParameters(patch.toString()))
        assertEquals(setOf("execution-host"), patch.getValue("clientModelPresentation").jsonObject.keys)
        assertFalse(patch.toString().contains("token"))
        assertThrows(IllegalArgumentException::class.java) {
            initialHostSettings(binding.copy(baseURL = "https://user:password@example.com"), parameters)
        }
        assertThrows(IllegalArgumentException::class.java) {
            initialHostSettings(binding.copy(baseURL = "https://example.com?token=secret"), parameters)
        }
    }
}
