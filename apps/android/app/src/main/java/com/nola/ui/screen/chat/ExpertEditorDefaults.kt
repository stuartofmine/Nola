// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import kotlinx.serialization.json.*

/** Complete configure_agent payload, preserving preferences the editor did not change.
 * An explicit default removes the selected ID; it never invents a None value. */
internal fun mergeExpertEditorDefaults(
    original: JsonObject,
    changeModel: Boolean,
    selectedModel: JsonObject?,
    changeTemplate: Boolean,
    selectedTemplateId: String?,
    initializeModelParameters: Boolean = false,
    reasoningEffort: String? = null,
): JsonObject {
    if (!changeModel && !changeTemplate && reasoningEffort == null) return original
    val fields = original.toMutableMap()
    if (changeModel) {
        if (selectedModel == null) fields.remove("modelSelectionID") else {
            require(selectedModel.keys == if (initializeModelParameters) setOf("modelSelectionID", "modelOverrides") else setOf("modelSelectionID"))
            fields["modelSelectionID"] = selectedModel.getValue("modelSelectionID")
            if (initializeModelParameters) {
                val existing = fields["modelOverrides"]
                require(existing == null || existing == JsonNull || existing is JsonObject)
                val overrides = (existing as? JsonObject)?.toMutableMap() ?: mutableMapOf()
                selectedModel.getValue("modelOverrides").jsonObject.forEach { (key, value) -> overrides.putIfAbsent(key, value) }
                fields["modelOverrides"] = JsonObject(overrides)
            }
        }
    }
    if (changeTemplate) {
        if (selectedTemplateId == null) fields.remove("selectedPromptTemplateID")
        else { require(selectedTemplateId.isNotBlank()); fields["selectedPromptTemplateID"] = JsonPrimitive(selectedTemplateId) }
    }
    if (reasoningEffort != null) {
        require(reasoningEffort in listOf("automatic", "none", "minimal", "low", "medium", "high", "xhigh", "max"))
        if (reasoningEffort == "automatic") fields.remove("expertReasoningEffort")
        else fields["expertReasoningEffort"] = JsonPrimitive(reasoningEffort)
    }
    return JsonObject(fields)
}
