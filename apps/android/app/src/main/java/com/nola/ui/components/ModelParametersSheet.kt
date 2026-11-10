package com.nola.ui.components

import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.ui.semantics.Role

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nola.R

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModelParametersSheet(
    modelName: String,
    supportsTopK: Boolean,
    parameters: com.nola.domain.model.ModelParameters,
    onUpdate: (com.nola.domain.model.ModelParameters) -> Unit,
    onDismiss: () -> Unit,
    saving: Boolean = false,
    error: String? = null,
    dismissOnSubmit: Boolean = true,
    draftVersion: Long = 0,
    onReload: (() -> Unit)? = null,
    supportsStreamOutput: Boolean = true,
    reasoningEfforts: List<String> = emptyList(),
) {
    // A draft and its comparison/submission baseline are one editing transaction.
    // Observing refreshed parameters must not silently rebase an open editor.
    val baseline = remember(draftVersion) { parameters }
    var tempText by remember(draftVersion) { mutableStateOf(parameters.temperature.toString()) }
    var topPText by remember(draftVersion) { mutableStateOf(parameters.topP.toString()) }
    var topKText by remember(draftVersion) { mutableStateOf((parameters.topK ?: 40).toString()) }
    var maxTokensText by remember(draftVersion) { mutableStateOf(parameters.maxCompletionTokens.toString()) }
    var maxContextText by remember(draftVersion) { mutableStateOf(parameters.maxContextTokens.toString()) }
    var reasoningPreference by remember(draftVersion) { mutableStateOf(parameters.reasoningPreference ?: "automatic") }
    var enableThinking by remember(draftVersion) { mutableStateOf(parameters.enableThinking) }
    var streamOutput by remember(draftVersion) { mutableStateOf(parameters.streamOutput) }
    var customPrompt by remember(draftVersion) { mutableStateOf(parameters.customSystemPrompt) }
    var showDiscardDialog by remember { mutableStateOf(false) }
    var reloadAfterDiscard by remember { mutableStateOf(false) }
    val initialTemp = baseline.temperature.toString()
    val initialTopP = baseline.topP.toString()
    val initialTopK = (baseline.topK ?: 40).toString()
    val initialMaxTokens = baseline.maxCompletionTokens.toString()
    val initialMaxContext = baseline.maxContextTokens.toString()
    val dirty = tempText != initialTemp || topPText != initialTopP || topKText != initialTopK
        || maxTokensText != initialMaxTokens || maxContextText != initialMaxContext
        || reasoningPreference != (baseline.reasoningPreference ?: "automatic") || enableThinking != baseline.enableThinking || streamOutput != baseline.streamOutput || customPrompt != baseline.customSystemPrompt
    val tempValue = tempText.toFloatOrNull()
    val topPValue = topPText.toFloatOrNull()
    val topKValue = topKText.toIntOrNull()
    val maxTokensValue = maxTokensText.toIntOrNull()
    val maxContextValue = maxContextText.toIntOrNull()
    val valid = tempValue != null && tempValue in 0f..2f
        && topPValue != null && topPValue in 0f..1f
        && (!supportsTopK || (topKValue != null && topKValue in 1..200))
        && maxTokensValue != null && maxTokensValue in com.nola.domain.model.ModelParameterLimits.outputTokens
        && maxContextValue != null && maxContextValue in com.nola.domain.model.ModelParameterLimits.remoteContextTokens
    val requestDismiss = { if (!saving) { reloadAfterDiscard = false; if (dirty) showDiscardDialog = true else onDismiss() } }
    val currentSaving by rememberUpdatedState(saving)
    val currentDirty by rememberUpdatedState(dirty)
    val listState = rememberLazyListState()
    LaunchedEffect(error, saving) {
        if (error != null && !saving) listState.animateScrollToItem(0)
    }
    val sheetState = rememberModalBottomSheetState(
        skipPartiallyExpanded = true,
        confirmValueChange = { target ->
            if (target != SheetValue.Hidden) true
            else if (currentSaving) false
            else if (currentDirty) { reloadAfterDiscard = false; showDiscardDialog = true; false }
            else true
        },
    )

    ModalBottomSheet(
        onDismissRequest = requestDismiss,
        sheetState = sheetState
    ) {
        Column(Modifier.fillMaxHeight(0.92f).imePadding().navigationBarsPadding()) {
            LazyColumn(
                state = listState,
                modifier = Modifier
                    .testTag("model-parameters-list")
                    .weight(1f)
                    .padding(horizontal = 24.dp, vertical = 20.dp)
            ) {
                item {
                    Text(stringResource(R.string.models_parameters_title, modelName), style = MaterialTheme.typography.titleLarge)
                    if (error != null) {
                        Text(error, color = MaterialTheme.colorScheme.error,
                            modifier = Modifier.testTag("model-parameters-error")
                                .semantics { liveRegion = LiveRegionMode.Polite })
                        if (onReload != null) {
                            TextButton(enabled = !saving, onClick = {
                                if (dirty) { reloadAfterDiscard = true; showDiscardDialog = true }
                                else onReload()
                            }) { Text(stringResource(R.string.common_refresh)) }
                        }
                    }
                    Spacer(Modifier.height(16.dp))
                }

                item {
                    Text(stringResource(R.string.models_sampling), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.height(8.dp))
                    NumericParameterField(stringResource(R.string.models_temperature), tempText, "0.00 ... 2.00", true, enabled = !saving, isError = tempValue == null || tempValue !in 0f..2f) { tempText = it }
                    NumericParameterField(stringResource(R.string.models_top_p), topPText, "0.00 ... 1.00", true, enabled = !saving, isError = topPValue == null || topPValue !in 0f..1f) { topPText = it }
                    if (supportsTopK) {
                        NumericParameterField(stringResource(R.string.models_top_k), topKText, "1 ... 200", false, enabled = !saving, isError = topKValue == null || topKValue !in 1..200) { topKText = it }
                    }
                }

                item { Spacer(Modifier.height(16.dp)) }
                item {
                    Text(stringResource(R.string.models_limits), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.height(8.dp))
                    NumericParameterField(stringResource(R.string.models_max_output), maxTokensText, "${com.nola.domain.model.ModelParameterLimits.outputTokens.first} ... ${com.nola.domain.model.ModelParameterLimits.outputTokens.last}", false, enabled = !saving, isError = maxTokensValue == null || maxTokensValue !in com.nola.domain.model.ModelParameterLimits.outputTokens) { maxTokensText = it }
                    NumericParameterField(stringResource(R.string.models_max_context), maxContextText, "${com.nola.domain.model.ModelParameterLimits.remoteContextTokens.first} ... ${com.nola.domain.model.ModelParameterLimits.remoteContextTokens.last}", false, enabled = !saving, isError = maxContextValue == null || maxContextValue !in com.nola.domain.model.ModelParameterLimits.remoteContextTokens) { maxContextText = it }
                }

                if (reasoningEfforts.isNotEmpty()) {
                    item {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(stringResource(R.string.models_reasoning_effort), style = MaterialTheme.typography.titleSmall)
                            for (effort in listOf("automatic") + reasoningEfforts) {
                                NolaRadioOption(com.nola.ui.screen.chat.reasoningEffortLabel(effort), reasoningPreference == effort,
                                    enabled = !saving, modifier = Modifier.testTag("model-reasoning-$effort")) { reasoningPreference = effort }
                            }
                        }
                    }
                }
                item { Spacer(Modifier.height(16.dp)) }
                item {
                    Row(
                        modifier = Modifier.fillMaxWidth().toggleable(value = enableThinking, enabled = !saving, role = Role.Switch, onValueChange = { enableThinking = it }).heightIn(min = 48.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(stringResource(R.string.models_show_thinking), style = MaterialTheme.typography.bodyMedium)
                            Text(
                                stringResource(R.string.models_show_thinking_desc),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        Switch(checked = enableThinking, onCheckedChange = null, enabled = !saving)
                    }
                }
                item { Spacer(Modifier.height(16.dp)) }
                item {
                    OutlinedTextField(
                        value = customPrompt,
                        enabled = !saving,
                        onValueChange = { customPrompt = it },
                        label = { Text(stringResource(R.string.models_custom_system_prompt)) },
                        modifier = Modifier.fillMaxWidth(),
                        minLines = 3,
                        maxLines = 6
                    )
                }
                if (supportsStreamOutput) {
                    item {
                        Row(modifier = Modifier.fillMaxWidth().testTag("model-parameters-stream").toggleable(value = streamOutput, enabled = !saving, role = Role.Switch, onValueChange = { streamOutput = it }).heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text(stringResource(R.string.chat_stream_output), modifier = Modifier.weight(1f))
                            Switch(checked = streamOutput, onCheckedChange = null, enabled = !saving)
                        }
                    }
                }
                item { Spacer(Modifier.height(16.dp)) }

            }
            Button(
                onClick = {
                    onUpdate(
                        baseline.copy(
                            temperature = tempValue ?: baseline.temperature,
                            topP = topPValue ?: baseline.topP,
                            topK = if (supportsTopK) topKValue ?: baseline.topK else baseline.topK,
                            maxCompletionTokens = maxTokensValue ?: baseline.maxCompletionTokens,
                            maxContextTokens = maxContextValue ?: baseline.maxContextTokens,
                            reasoningTier = if (reasoningPreference == (baseline.reasoningPreference ?: "automatic")) baseline.reasoningTier else null,
                            reasoningPreference = if (reasoningPreference == (baseline.reasoningPreference ?: "automatic")) baseline.reasoningPreference else reasoningPreference,
                            enableThinking = enableThinking,
                            streamOutput = streamOutput,
                            customSystemPrompt = customPrompt.trim()
                        )
                    )
                    if (dismissOnSubmit) onDismiss()
                },
                modifier = Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 12.dp).testTag("model-parameters-save"),
                enabled = dirty && valid && !saving
            ) {
                Text(stringResource(R.string.models_save_parameters))
            }
        }
    }
    if (showDiscardDialog) {
        DiscardChangesDialog(
            onDiscard = {
                if (!saving) {
                    showDiscardDialog = false
                    if (reloadAfterDiscard) onReload?.invoke() else onDismiss()
                    reloadAfterDiscard = false
                }
            },
            onKeepEditing = { showDiscardDialog = false; reloadAfterDiscard = false }
        )
    }
}

@Composable
internal fun DiscardChangesDialog(onDiscard: () -> Unit, onKeepEditing: () -> Unit) {
    AlertDialog(
        onDismissRequest = onKeepEditing,
        title = { Text(stringResource(R.string.models_discard_changes_title)) },
        text = { Text(stringResource(R.string.ui_unsaved_changes)) },
        confirmButton = {
            TextButton(onClick = onDiscard) {
                Text(
                    stringResource(R.string.models_discard_changes_action),
                    color = MaterialTheme.colorScheme.error
                )
            }
        },
        dismissButton = {
            TextButton(onClick = onKeepEditing) {
                Text(stringResource(R.string.models_keep_editing))
            }
        }
    )
}

@Composable
fun NumericParameterField(
    label: String,
    value: String,
    rangeText: String,
    allowDecimal: Boolean,
    enabled: Boolean = true,
    isError: Boolean = false,
    onValueChange: (String) -> Unit
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        enabled = enabled,
        isError = isError,
        label = { Text(label) },
        supportingText = { Text(rangeText) },
        singleLine = true,
        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
            keyboardType = if (allowDecimal) {
                androidx.compose.ui.text.input.KeyboardType.Decimal
            } else {
                androidx.compose.ui.text.input.KeyboardType.Number
            }
        ),
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
    )
}
