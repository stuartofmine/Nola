package com.nola.ui.screen.chat

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class ExpertEditorDefaultsTest {
    private fun value(text: String) = Json.parseToJsonElement(text).jsonObject

    @Test fun metadataEditKeepsOriginalNullsAndPreferencesExactly() {
        val original = value("""{"modelSelectionID":null,"selectedPromptTemplateID":null,"language":"zh","memoryEnabledOverride":false,"modelOverrides":{"configured:a":{"temperature":0.21}}}""")
        assertSame(original, mergeExpertEditorDefaults(original, false, null, false, null))
    }

    @Test fun selectingAnotherModelChangesOnlyItsPointer() {
        val original = value("""{"modelSelectionID":"configured:a","selectedPromptTemplateID":"template","retrievalEnabledOverride":false,"modelOverrides":{"configured:a":{"temperature":0.21},"configured:b":{"maxOutputTokens":900}}}""")
        val updated = mergeExpertEditorDefaults(original, true, value("""{"modelSelectionID":"configured:b"}"""), false, null)
        assertEquals(JsonPrimitive("configured:b"), updated["modelSelectionID"])
        assertEquals(original.filterKeys { it != "modelSelectionID" }, updated.filterKeys { it != "modelSelectionID" })
    }

    @Test fun inheritingModelDoesNotEraseStoredPerModelParameters() {
        val original = value("""{"modelSelectionID":"configured:a","selectedPromptTemplateID":null,"modelOverrides":{"configured:a":{"temperature":0.21}}}""")
        val updated = mergeExpertEditorDefaults(original, true, null, false, null)
        assertFalse(updated.containsKey("modelSelectionID"))
        assertEquals(original["modelOverrides"], updated["modelOverrides"])
        assertEquals(JsonNull, updated["selectedPromptTemplateID"])
    }

    @Test fun defaultTemplateRemovesItsKeyRatherThanWritingNoneOrNull() {
        val original = value("""{"modelSelectionID":null,"selectedPromptTemplateID":"template","language":"zh"}""")
        val updated = mergeExpertEditorDefaults(original, false, null, true, null)
        assertFalse(updated.containsKey("selectedPromptTemplateID"))
        assertEquals(JsonNull, updated["modelSelectionID"])
        assertEquals(original["language"], updated["language"])
    }

    @Test fun creationInitializesOnlyAbsentParameterProfiles() {
        val original = value("""{"modelOverrides":{"configured:a":{"temperature":0.21}}}""")
        val generated = value("""{"modelSelectionID":"configured:b","modelOverrides":{"configured:a":{"temperature":0.7},"configured:b":{"temperature":0.8}}}""")
        val updated = mergeExpertEditorDefaults(original, true, generated, false, null, initializeModelParameters = true)
        val profiles = updated.getValue("modelOverrides").jsonObject
        assertEquals(original.getValue("modelOverrides").jsonObject["configured:a"], profiles["configured:a"])
        assertEquals(generated.getValue("modelOverrides").jsonObject["configured:b"], profiles["configured:b"])
    }
}
