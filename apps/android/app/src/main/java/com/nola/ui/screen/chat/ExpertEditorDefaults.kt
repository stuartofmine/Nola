// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import kotlinx.serialization.json.*

/** Complete configure_agent payload, preserving preferences the editor did not change.
 * An explicit default removes the selected ID; it never invents a None value. */
internal fun expertExecutorBinding(backendID: String, model: com.nola.data.remote.dto.ExecutionBackendModelDto? = null): JsonObject =
    buildJsonObject {
        put("backend", if (backendID == "opencode") "open_code" else backendID)
        model?.let {
            put("model", it.id)
            if (backendID == "codex") it.providerId?.let { provider -> put("model_provider", provider) }
        }
    }

internal fun mergeExpertEditorDefaults(
    original: JsonObject,
    changeModel: Boolean,
    selectedModel: JsonObject?,
    changeTemplate: Boolean,
    selectedTemplateId: String?,
    reasoningEffort: String? = null,
    resetModelParameters: String? = null,
    changeExecutor: Boolean = false,
    executor: JsonObject? = null,
): JsonObject {
    if (!changeExecutor && !changeModel && !changeTemplate && reasoningEffort == null && resetModelParameters == null) return original
    val fields = original.toMutableMap()
    if (changeExecutor) {
        if (executor == null) fields.remove("expertExecutor") else fields["expertExecutor"] = executor
    }
    if (changeModel) {
        if (selectedModel == null) fields.remove("modelSelectionID") else {
            require(selectedModel.keys == setOf("modelSelectionID"))
            fields["modelSelectionID"] = selectedModel.getValue("modelSelectionID")
        }
    }
    if (resetModelParameters != null) {
        require(fields["modelSelectionID"] == JsonPrimitive(resetModelParameters))
        val overrides = (fields["modelOverrides"] as? JsonObject).orEmpty() - resetModelParameters
        if (overrides.isEmpty()) fields.remove("modelOverrides") else fields["modelOverrides"] = JsonObject(overrides)
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
    if (fields["expertExecutor"] is JsonObject) fields.remove("expertReasoningEffort")
    return JsonObject(fields)
}
