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

    @Test fun selectingModelInheritsItsProfileUntilTheUserExplicitlyResetsExistingParameters() {
        val selected = value("""{"modelSelectionID":"configured:b"}""")
        val created = mergeExpertEditorDefaults(value("{}"), true, selected, false, null)
        assertEquals(selected, created)

        val original = value("""{"modelSelectionID":"configured:b","modelOverrides":{"configured:a":{"temperature":0.21},"configured:b":{"temperature":0.7,"topP":0.9}}}""")
        val saved = mergeExpertEditorDefaults(original, false, null, false, null, reasoningEffort = "automatic")
        assertEquals(original, saved)
        val reset = mergeExpertEditorDefaults(original, false, null, false, null, resetModelParameters = "configured:b")
        assertEquals(value("""{"configured:a":{"temperature":0.21}}"""), reset["modelOverrides"])
        assertEquals(original["modelSelectionID"], reset["modelSelectionID"])
    }
    @Test fun effortIsExactAndAutomaticRemovesOnlyItsOverride() {
        val original = value("""{"expertReasoningEffort":"high","modelOverrides":{"configured:a":{"temperature":0.21}},"selectedPromptTemplateID":"template"}""")
        val inherited = mergeExpertEditorDefaults(original, false, null, false, null, reasoningEffort = "automatic")
        assertFalse(inherited.containsKey("expertReasoningEffort"))
        assertEquals(original.filterKeys { it != "expertReasoningEffort" }, inherited)
        val maximum = mergeExpertEditorDefaults(original, false, null, false, null, reasoningEffort = "max")
        assertEquals(JsonPrimitive("max"), maximum["expertReasoningEffort"])
        assertEquals(original["modelOverrides"], maximum["modelOverrides"])
        assertThrows(IllegalArgumentException::class.java) {
            mergeExpertEditorDefaults(original, false, null, false, null, reasoningEffort = "balanced")
        }
    }
    @Test fun executorVariantsKeepProviderAndPreserveModelDefaults() {
        val original = buildJsonObject { put("modelSelectionID", "old") }
        val variant = com.nola.data.remote.dto.ExecutionBackendModelDto("x", "X", "p")
        val binding = expertExecutorBinding("codex", variant)
        val updated = mergeExpertEditorDefaults(original, false, null, false, null, changeExecutor = true, executor = binding)
        assertEquals(JsonPrimitive("p"), (updated["expertExecutor"] as JsonObject)["model_provider"])
        assertEquals(JsonPrimitive("open_code"), expertExecutorBinding("opencode")["backend"])
        assertEquals(original, mergeExpertEditorDefaults(updated, false, null, false, null, changeExecutor = true))
    }
}
