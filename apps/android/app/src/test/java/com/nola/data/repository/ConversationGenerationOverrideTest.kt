package com.nola.data.repository

import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.data.local.CanonicalModelProfile
import com.nola.ui.screen.chat.ChatUiState
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.*
import org.junit.Test

class ConversationGenerationOverrideTest {
    @Test fun sparseOverridesInheritAndUseCanonicalModelKeys() {
        val defaults = ModelParameters(maxContextTokens = 32_000, maxCompletionTokens = 8_192)
        assertEquals(defaults, ConversationGenerationOverride().applyingTo(defaults))
        assertEquals(4_096, ConversationGenerationOverride(maxOutputTokens = 4_096).applyingTo(defaults).maxCompletionTokens)
        assertEquals(32_000, ConversationGenerationOverride(maxOutputTokens = 4_096).applyingTo(defaults).maxContextTokens)
        assertEquals("local:device", ConversationGenerationOverride.modelKey(ModelInfo("device", "Device", ModelInfo.ModelSource.Local)))
        val profile = CanonicalModelProfile(
            id = "profile-123", displayName = "Cloud", provider = "openai-compatible",
            modelName = "wire-model-7", runtime = "remote", sourceJson = "{}", parametersJson = "{}",
            selected = true, enabled = true, revision = 3, createdAtMs = 1, updatedAtMs = 1,
        )
        val cloud = profile.toConfiguredModelInfo()
        assertEquals("wire-model-7", cloud.id)
        assertEquals("profile-123", cloud.selectionId)
        assertEquals("profile-123", cloud.parameterProfileId)
        assertEquals("configured:profile-123", ConversationGenerationOverride.modelKey(cloud))
        assertNull(ConversationGenerationOverride.modelKeyOrNull(ModelInfo("wire-model-7", "Official", ModelInfo.ModelSource.Cloud)))
        val state = ChatUiState(currentParameters = defaults, generationOverride = ConversationGenerationOverride(48_000, 4_096))
        assertEquals(48_000, state.effectiveGenerationParameters.maxContextTokens)
        assertEquals(4_096, state.effectiveGenerationParameters.maxCompletionTokens)
        assertEquals(defaults, state.currentParameters)
    }

    @Test fun sparsePatchOnlyNamesAndroidOwnedFieldsAndUnrelatedSettingsStayIntact() {
        val original = """{"permissionProfile":"cautious","modelOverrides":{"configured:a":{"wireProtocol":"responses","reasoningPreference":"high","temperature":0,"maxOutputTokens":4096},"configured:b":{"topP":0.5}}}"""
        val patch = conversationGenerationPatch("configured:a", ConversationGenerationOverride(65_536, null))
        val models = patch.getValue("modelOverrides").jsonObject
        assertEquals(setOf("configured:a"), models.keys)
        val selected = models.getValue("configured:a").jsonObject
        assertEquals(setOf("maxContextLength"), selected.keys)
        assertEquals("65536", selected.getValue("maxContextLength").toString())
        assertFalse(selected.containsKey("maxOutputTokens"))
        assertEquals(4_096, readConversationGenerationOverrides(original).getValue("configured:a").maxOutputTokens)
        val unrelatedEdit = transformConversationSettings(original) { copy(sessionMemory = "updated") }
        assertEquals(
            Json.parseToJsonElement(original).jsonObject.getValue("modelOverrides"),
            Json.parseToJsonElement(unrelatedEdit).jsonObject.getValue("modelOverrides"),
        )
    }

    @Test fun invalidLimitsCannotBePersisted() {
        assertThrows(IllegalArgumentException::class.java) {
            conversationGenerationPatch("configured:a", ConversationGenerationOverride(maxOutputTokens = 0))
        }
    }
}
