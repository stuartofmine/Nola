package com.nola.data.repository

import com.nola.domain.model.ModelParameters
import com.nola.domain.model.ModelInfo
import kotlinx.serialization.encodeToString
import com.nola.ui.screen.chat.ChatUiState
import com.nola.ui.screen.chat.conversationPermissionMode
import com.nola.core.AndroidAgentPermissionMode
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.*
import org.junit.Test

class ConversationPromptStyleTest {
    @Test fun conversationWritesUseTheSameStrictTwoProfilesAsCore() {
        for (profile in listOf("cautious", "allow_all")) {
            assertEquals(profile, requireValidConversationPermissionMode(profile))
        }
        val changed = transformConversationSettings("""{"permissionModeRawValue":"allow_all"}""") {
            copy(permissionModeRawValue = "cautious")
        }
        assertEquals("cautious", Json.parseToJsonElement(changed).jsonObject
            .getValue("permissionModeRawValue").jsonPrimitive.content)
        for (invalid in listOf("safe", "recommended", "allow-all", " cautious ", "", "unknown")) {
            assertThrows(IllegalArgumentException::class.java) { requireValidConversationPermissionMode(invalid) }
        }
    }

    @Test fun permissionModeIsConversationScopedAndUnrelatedEditsPreserveIt() {
        val first = """{"permissionModeRawValue":"cautious","modelOverrides":{"a":{"temperature":0}},"sessionMemory":"first"}"""
        val second = """{"permissionModeRawValue":"allow_all","sessionMemory":"second"}"""
        val changed = transformConversationSettings(first) { copy(sessionMemory = "changed") }
        assertEquals("cautious", Json.parseToJsonElement(changed).jsonObject.getValue("permissionModeRawValue").jsonPrimitive.content)
        assertEquals("allow_all", Json.parseToJsonElement(second).jsonObject.getValue("permissionModeRawValue").jsonPrimitive.content)
        assertEquals(
            Json.parseToJsonElement(first).jsonObject.getValue("modelOverrides"),
            Json.parseToJsonElement(changed).jsonObject.getValue("modelOverrides"),
        )
        val selected = transformConversationSettings(changed) { copy(permissionModeRawValue = "allow_all") }
        assertEquals("allow_all", Json.parseToJsonElement(selected).jsonObject.getValue("permissionModeRawValue").jsonPrimitive.content)
    }

    @Test fun unknownPermissionModesCannotBeSavedOrEscalateOnRead() {
        assertThrows(IllegalArgumentException::class.java) {
            transformConversationSettings("""{"permissionModeRawValue":"cautious"}""") { copy(permissionModeRawValue = "allowEverything") }
        }
        assertThrows(IllegalArgumentException::class.java) {
            conversationPermissionMode("allowEverything", AndroidAgentPermissionMode.AllowAll)
        }
        assertEquals(AndroidAgentPermissionMode.Cautious,
            conversationPermissionMode(null, AndroidAgentPermissionMode.Cautious))
    }

    @Test fun twoConversationModelSelectionsRoundTripWithoutChangingEachOther() {
        val modelA = "configured:profile-a"
        val modelB = "configured:profile-b"
        val first = Json.encodeToString(AndroidConversationSettings(modelSelectionID = modelA, promptResponseStyle = "playful"))
        val second = Json.encodeToString(AndroidConversationSettings(modelSelectionID = modelB))
        assertEquals(modelA, Json.decodeFromString<AndroidConversationSettings>(first).modelSelectionID)
        assertEquals(modelB, Json.decodeFromString<AndroidConversationSettings>(second).modelSelectionID)
        val changed = transformConversationSettings(first) { copy(sessionMemory = "new memory") }
        assertEquals(modelA, Json.decodeFromString<AndroidConversationSettings>(changed).modelSelectionID)
        assertEquals(modelB, Json.decodeFromString<AndroidConversationSettings>(second).modelSelectionID)
    }

    @Test fun onlySharedPromptStyleWireValuesAndInheritanceAreAccepted() {
        listOf(null, "professional", "casual", "playful").forEach { value ->
            assertEquals(value, com.nola.domain.model.requireValidPromptResponseStyle(value))
        }
        listOf("", "balanced", "Professional", " playful ", "unknown").forEach { value ->
            assertThrows(IllegalArgumentException::class.java) {
                com.nola.domain.model.requireValidPromptResponseStyle(value)
            }
        }
    }

    @Test fun invalidPromptStyleCannotProduceSettingsToPersist() {
        val original = """{"promptResponseStyle":"professional","extensionValue":"preserve","extension":{"keep":true}}"""
        listOf("", "balanced", "Professional", " playful ", "unknown").forEach { invalid ->
            var persisted = original
            assertThrows(IllegalArgumentException::class.java) {
                persisted = transformConversationSettings(persisted) { copy(promptResponseStyle = invalid) }
            }
            assertEquals(original, persisted)
        }
    }

    @Test fun newConversationInheritsModelDefaultAndOverrideDoesNotMutateIt() {
        val defaults = ModelParameters(responseStyle = "professional")
        val first = ChatUiState(currentParameters = defaults)
        assertEquals("professional", first.responseStyle)
        val overridden = first.copy(responseStyleOverride = "playful")
        assertEquals("playful", overridden.responseStyle)
        assertEquals("professional", overridden.currentParameters.responseStyle)
        assertEquals("casual", first.copy(currentParameters = defaults.copy(responseStyle = "casual")).responseStyle)
        assertEquals("playful", overridden.copy(currentParameters = defaults.copy(responseStyle = "casual")).responseStyle)
        assertEquals("professional", overridden.copy(responseStyleOverride = null).responseStyle)
    }

    @Test fun promptStylePatchPreservesOtherPlatformsSettingsAndCanRestoreInheritance() {
        val original = """{"extensionValue":"preserve","modelOverrides":{"m":{"temperature":0}},"sessionMemory":"keep"}"""
        val changed = transformConversationSettings(original) { copy(promptResponseStyle = "playful") }
        val objectValue = Json.parseToJsonElement(changed).jsonObject
        assertEquals("playful", objectValue.getValue("promptResponseStyle").jsonPrimitive.content)
        assertEquals("preserve", objectValue.getValue("extensionValue").jsonPrimitive.content)
        assertEquals(Json.parseToJsonElement(original).jsonObject.getValue("modelOverrides"), objectValue["modelOverrides"])
        val restored = transformConversationSettings(changed) { copy(promptResponseStyle = null) }
        assertEquals(Json.parseToJsonElement(original), Json.parseToJsonElement(restored))
    }

    @Test fun unrelatedSettingsEditKeepsConversationPromptStyle() {
        val changed = transformConversationSettings("""{"promptResponseStyle":"playful"}""") {
            copy(sessionMemory = "memory")
        }
        assertEquals("playful", Json.parseToJsonElement(changed).jsonObject.getValue("promptResponseStyle").jsonPrimitive.content)
    }

    @Test(expected = IllegalArgumentException::class)
    fun malformedSettingsAreNotSilentlyReplaced() {
        transformConversationSettings("invalid json") { copy(promptResponseStyle = "playful") }
    }
}
