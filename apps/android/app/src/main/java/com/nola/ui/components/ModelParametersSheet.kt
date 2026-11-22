package com.nola.ui.components

import androidx.compose.foundation.selection.toggleable
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.nola.R
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt

@Composable
fun ModelParametersContent(
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
    showBackTopBar: Boolean = false,
    onDirtyChange: ((Boolean) -> Unit)? = null,
    modifier: Modifier = Modifier,
) {
    val baseline = remember(draftVersion) { parameters }
    val defaults = remember { com.nola.domain.model.ModelParameters() }
    var temperature by remember(draftVersion) { mutableFloatStateOf(parameters.temperature.coerceIn(0f, 2f)) }
    var topP by remember(draftVersion) { mutableFloatStateOf(parameters.topP.coerceIn(0f, 1f)) }
    var topK by remember(draftVersion) { mutableIntStateOf((parameters.topK ?: 40).coerceIn(1, 200)) }
    var maxTokensText by remember(draftVersion) { mutableStateOf(parameters.maxCompletionTokens.toString()) }
    var maxContextText by remember(draftVersion) { mutableStateOf(parameters.maxContextTokens.toString()) }
    var reasoningPreference by remember(draftVersion) { mutableStateOf(parameters.reasoningPreference ?: "automatic") }
    var enableThinking by remember(draftVersion) { mutableStateOf(parameters.enableThinking) }
    var streamOutput by remember(draftVersion) { mutableStateOf(parameters.streamOutput) }
    var customPrompt by remember(draftVersion) { mutableStateOf(parameters.customSystemPrompt) }
    var showDiscardDialog by remember { mutableStateOf(false) }
    var reloadAfterDiscard by remember { mutableStateOf(false) }
    val initialMaxTokens = baseline.maxCompletionTokens.toString()
    val initialMaxContext = baseline.maxContextTokens.toString()
    val dirty = abs(temperature - baseline.temperature) > 0.004f
        || abs(topP - baseline.topP) > 0.004f
        || (supportsTopK && topK != (baseline.topK ?: 40))
        || maxTokensText != initialMaxTokens
        || maxContextText != initialMaxContext
        || reasoningPreference != (baseline.reasoningPreference ?: "automatic")
        || enableThinking != baseline.enableThinking
        || streamOutput != baseline.streamOutput
        || customPrompt != baseline.customSystemPrompt
    LaunchedEffect(dirty) { onDirtyChange?.invoke(dirty) }
    val maxTokensValue = maxTokensText.toIntOrNull()
    val maxContextValue = maxContextText.toIntOrNull()
    val valid = maxTokensValue != null && maxTokensValue in com.nola.domain.model.ModelParameterLimits.outputTokens
        && maxContextValue != null && maxContextValue in com.nola.domain.model.ModelParameterLimits.remoteContextTokens
    val requestDismiss = { if (!saving) { reloadAfterDiscard = false; if (dirty) showDiscardDialog = true else onDismiss() } }
    val resetToDefaults = {
        temperature = defaults.temperature.coerceIn(0f, 2f)
        topP = defaults.topP.coerceIn(0f, 1f)
        topK = (defaults.topK ?: 40).coerceIn(1, 200)
        maxTokensText = defaults.maxCompletionTokens.toString()
        maxContextText = defaults.maxContextTokens.toString()
        reasoningPreference = defaults.reasoningPreference ?: "automatic"
        enableThinking = defaults.enableThinking
        streamOutput = defaults.streamOutput
        customPrompt = defaults.customSystemPrompt
    }
    val listState = rememberLazyListState()
    LaunchedEffect(error, saving) {
        if (error != null && !saving) listState.animateScrollToItem(0)
    }
    if (showBackTopBar) {
        androidx.activity.compose.BackHandler(onBack = requestDismiss)
    }
    Column(modifier) {
        if (showBackTopBar) {
            NolaTopAppBar(
                title = stringResource(R.string.models_parameters_title, modelName),
                onBack = requestDismiss,
            ) {
                TextButton(onClick = resetToDefaults, enabled = !saving) {
                    Text(stringResource(R.string.models_parameters_reset))
                }
            }
        }
        LazyColumn(
            state = listState,
            modifier = Modifier
                .testTag("model-parameters-list")
                .weight(1f)
                .padding(horizontal = 24.dp, vertical = if (showBackTopBar) 12.dp else 20.dp)
        ) {
            item {
                if (!showBackTopBar) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            stringResource(R.string.models_parameters_title, modelName),
                            style = MaterialTheme.typography.titleLarge,
                            modifier = Modifier.weight(1f),
                        )
                        TextButton(onClick = resetToDefaults, enabled = !saving) {
                            Text(stringResource(R.string.models_parameters_reset))
                        }
                    }
                }
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
                if (!showBackTopBar) Spacer(Modifier.height(16.dp))
            }

            item {
                Text(stringResource(R.string.models_sampling), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.height(8.dp))
                SliderParameterField(
                    label = stringResource(R.string.models_temperature),
                    valueText = String.format(Locale.US, "%.2f", temperature),
                    value = temperature,
                    valueRange = 0f..2f,
                    enabled = !saving,
                    onValueChange = { temperature = (it * 20f).roundToInt() / 20f },
                )
                SliderParameterField(
                    label = stringResource(R.string.models_top_p),
                    valueText = String.format(Locale.US, "%.2f", topP),
                    value = topP,
                    valueRange = 0f..1f,
                    enabled = !saving,
                    onValueChange = { topP = (it * 20f).roundToInt() / 20f },
                )
                if (supportsTopK) {
                    SliderParameterField(
                        label = stringResource(R.string.models_top_k),
                        valueText = topK.toString(),
                        value = topK.toFloat(),
                        valueRange = 1f..200f,
                        enabled = !saving,
                        onValueChange = { topK = it.roundToInt().coerceIn(1, 200) },
                    )
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
                    Spacer(Modifier.height(12.dp))
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.models_reasoning_effort), style = MaterialTheme.typography.titleSmall)
                        for (effort in (listOf("automatic") + reasoningEfforts).distinct()) {
                            NolaRadioOption(com.nola.ui.screen.chat.reasoningEffortLabel(effort), reasoningPreference == effort,
                                enabled = !saving, modifier = Modifier.testTag("model-reasoning-$effort")) { reasoningPreference = effort }
                        }
                    }
                }
            }
            item { Spacer(Modifier.height(16.dp)) }
            item {
                Row(
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).toggleable(value = enableThinking, enabled = !saving, role = Role.Switch, onValueChange = { enableThinking = it }).heightIn(min = 48.dp),
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
                    NolaSwitch(checked = enableThinking, onCheckedChange = null, enabled = !saving)
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
                    Spacer(Modifier.height(12.dp))
                    Row(modifier = Modifier.fillMaxWidth().testTag("model-parameters-stream").clip(RoundedCornerShape(8.dp)).toggleable(value = streamOutput, enabled = !saving, role = Role.Switch, onValueChange = { streamOutput = it }).heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(R.string.chat_stream_output), modifier = Modifier.weight(1f))
                        NolaSwitch(checked = streamOutput, onCheckedChange = null, enabled = !saving)
                    }
                }
            }
            item { Spacer(Modifier.height(16.dp)) }
        }
        Button(
            onClick = {
                onUpdate(
                    baseline.copy(
                        temperature = temperature,
                        topP = topP,
                        topK = if (supportsTopK) topK else baseline.topK,
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
private fun SliderParameterField(
    label: String,
    valueText: String,
    value: Float,
    valueRange: ClosedFloatingPointRange<Float>,
    enabled: Boolean,
    onValueChange: (Float) -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(label, style = MaterialTheme.typography.bodyMedium)
            Text(
                valueText,
                style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.primary,
            )
        }
        Slider(
            value = value,
            onValueChange = onValueChange,
            valueRange = valueRange,
            enabled = enabled,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

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
    var dirty by remember(draftVersion) { mutableStateOf(false) }
    var showDiscardDialog by remember { mutableStateOf(false) }
    val requestDismiss = { if (!saving) { if (dirty) showDiscardDialog = true else onDismiss() } }
    val currentSaving by rememberUpdatedState(saving)
    val currentDirty by rememberUpdatedState(dirty)
    val sheetState = rememberModalBottomSheetState(
        skipPartiallyExpanded = true,
        confirmValueChange = { target ->
            if (target != SheetValue.Hidden) true
            else if (currentSaving) false
            else if (currentDirty) { showDiscardDialog = true; false }
            else true
        },
    )

    ModalBottomSheet(
        modifier = Modifier.dismissImeOnBlankTap(),
        onDismissRequest = requestDismiss,
        sheetState = sheetState,
        contentWindowInsets = { androidx.compose.foundation.layout.WindowInsets(0) },
    ) {
        ModelParametersContent(
            modelName = modelName,
            supportsTopK = supportsTopK,
            parameters = parameters,
            onUpdate = onUpdate,
            onDismiss = onDismiss,
            saving = saving,
            error = error,
            dismissOnSubmit = dismissOnSubmit,
            draftVersion = draftVersion,
            onReload = onReload,
            supportsStreamOutput = supportsStreamOutput,
            reasoningEfforts = reasoningEfforts,
            showBackTopBar = false,
            onDirtyChange = { dirty = it },
            modifier = Modifier.fillMaxHeight(0.92f).imePadding().navigationBarsPadding(),
        )
    }
    if (showDiscardDialog) {
        DiscardChangesDialog(
            onDiscard = {
                if (!saving) {
                    showDiscardDialog = false
                    onDismiss()
                }
            },
            onKeepEditing = { showDiscardDialog = false }
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
