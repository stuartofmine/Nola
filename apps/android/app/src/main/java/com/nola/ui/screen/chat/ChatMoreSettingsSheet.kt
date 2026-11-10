package com.nola.ui.screen.chat

import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.semantics.Role

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.core.AndroidAgentPermissionMode
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.HostComputerUseStatus
import com.nola.domain.model.HostNativeBrowserStatus
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.ConversationGenerationOverride
import com.nola.domain.model.ModelParameterLimits
import com.nola.ui.components.ModelParametersSheet
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.key
import com.nola.domain.model.UploadedContent
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaDivider
import com.nola.ui.components.NolaSectionTitle
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaTopAppBar
import kotlinx.coroutines.launch

private fun compactChatTokenCount(value: Int): String = when {
    value >= 1_000_000 -> "%.1fM".format(value / 1_000_000f)
    value >= 1_000 -> "%.1fK".format(value / 1_000f)
    else -> value.toString()
}

private fun permissionModeLabel(mode: AndroidAgentPermissionMode): Int = when (mode) {
    AndroidAgentPermissionMode.Cautious -> R.string.chat_permission_mode_cautious
    AndroidAgentPermissionMode.AllowAll -> R.string.chat_permission_mode_allow_all
}

private fun permissionModeDescription(mode: AndroidAgentPermissionMode): Int = when (mode) {
    AndroidAgentPermissionMode.Cautious -> R.string.chat_permission_mode_cautious_detail
    AndroidAgentPermissionMode.AllowAll -> R.string.chat_permission_mode_allow_all_detail
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun PermissionModeSelector(
    selected: AndroidAgentPermissionMode,
    onSelected: (AndroidAgentPermissionMode) -> Unit,
    enabled: Boolean = true,
) {
    Text(
        stringResource(R.string.chat_permission_mode),
        style = MaterialTheme.typography.labelMedium,
    )
    Spacer(Modifier.height(NolaSpacing.sm))
    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
        AndroidAgentPermissionMode.entries.forEachIndexed { index, mode ->
            SegmentedButton(
                enabled = enabled,
                selected = selected == mode,
                onClick = { onSelected(mode) },
                shape = SegmentedButtonDefaults.itemShape(
                    index,
                    AndroidAgentPermissionMode.entries.size,
                ),
                modifier = Modifier
                    .weight(1f)
                    .testTag("permission-mode-${mode.wireValue}"),
            ) {
                Text(
                    stringResource(permissionModeLabel(mode)),
                    style = MaterialTheme.typography.labelSmall,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
    Text(
        text = stringResource(permissionModeDescription(selected)),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = NolaSpacing.sm),
    )
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun ChatMoreSettingsScreen(
    selectedModel: ModelInfo?,
    contextUsage: ContextUsage?,
    hostUploadedSkills: List<UploadedContent> = emptyList(),
    hostUploadedDocuments: List<UploadedContent> = emptyList(),
    hostMcpServers: List<McpServerConfig> = emptyList(),
    hostComputerUseStatus: HostComputerUseStatus? = null,
    hostNativeBrowserStatus: HostNativeBrowserStatus? = null,
    agentModeEnabled: Boolean,
    permissionMode: AndroidAgentPermissionMode,
    hybridDeviceToolsEnabled: Boolean,
    hybridDeviceCapabilities: Set<String>,
    pairedHostIdentity: String,
    autoCompactTriggerPercent: Int,
    responseStyleOverride: String?,
    sessionMemory: String,
    longTermMemory: String,
    userPreferences: String,
    uploadedSkills: List<UploadedContent> = emptyList(),
    draftParameters: ModelParameters,
    reasoningEfforts: List<String> = emptyList(),
    conversationId: String?,
    generationOverride: ConversationGenerationOverride,
    onAgentModeChange: (Boolean) -> Unit,
    onPermissionModeChange: suspend (AndroidAgentPermissionMode) -> Unit,
    onHybridDeviceToolsChange: (Boolean) -> Unit,
    onHybridDeviceCapabilityChange: (String, Boolean) -> Unit,
    onForgetHostPairing: () -> Unit,
    onAutoCompactPercentChange: (Int) -> Unit,
    onResponseStyleChange: suspend (String?) -> Unit,
    onSessionMemoryChange: (String) -> Unit,
    onLongTermMemoryChange: (String) -> Unit,
    onUserPreferencesChange: (String) -> Unit,
    onSkillToggle: (UploadedContent, Boolean) -> Unit,
    onSaveParameters: suspend (ModelParameters) -> Unit,
    onSaveGenerationOverride: suspend (ConversationGenerationOverride) -> Unit,
    onReloadParameters: () -> Boolean,
    onSaveMemoryProfile: suspend () -> Unit,
    onOpenContextSettings: () -> Unit,
    onOpenSkillLibrary: () -> Unit,
    onBack: () -> Unit
) {
    var showModelParameters by remember(selectedModel?.selectionId) { mutableStateOf(false) }
    var showConversationLimits by remember(conversationId, selectedModel?.selectionId) { mutableStateOf(false) }
    var conversationLimitsSaving by remember(conversationId, selectedModel?.selectionId) { mutableStateOf(false) }
    var conversationLimitsError by remember(conversationId, selectedModel?.selectionId) { mutableStateOf<String?>(null) }
    var contextLimitText by remember(conversationId, selectedModel?.selectionId, generationOverride) {
        mutableStateOf(generationOverride.maxContextLength?.toString().orEmpty())
    }
    var outputLimitText by remember(conversationId, selectedModel?.selectionId, generationOverride) {
        mutableStateOf(generationOverride.maxOutputTokens?.toString().orEmpty())
    }
    var parameterSaving by remember(selectedModel?.selectionId) { mutableStateOf(false) }
    var parameterError by remember(selectedModel?.selectionId) { mutableStateOf<String?>(null) }
    var parameterDraftVersion by remember(selectedModel?.selectionId) { mutableStateOf(0L) }
    val currentModelId by rememberUpdatedState(selectedModel?.selectionId)
    var localResponseStyle by remember(responseStyleOverride) { mutableStateOf(responseStyleOverride) }
    var localAgentMode by remember(agentModeEnabled) { mutableStateOf(agentModeEnabled) }
    var localPermissionMode by remember(permissionMode) { mutableStateOf(permissionMode) }
    var permissionSaving by remember { mutableStateOf(false) }
    var localHybridDeviceTools by remember(hybridDeviceToolsEnabled) { mutableStateOf(hybridDeviceToolsEnabled) }
    var localHybridCapabilities by remember(hybridDeviceCapabilities) { mutableStateOf(hybridDeviceCapabilities) }
    var localAutoCompactPercent by remember(autoCompactTriggerPercent) { mutableIntStateOf(autoCompactTriggerPercent) }
    var localSessionMemory by remember(sessionMemory) { mutableStateOf(sessionMemory) }
    var localLongTermMemory by remember(longTermMemory) { mutableStateOf(longTermMemory) }
    var localUserPreferences by remember(userPreferences) { mutableStateOf(userPreferences) }

    var isSaving by remember { mutableStateOf(false) }
    var statusMessage by remember { mutableStateOf<String?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var showForgetPairingConfirmation by remember { mutableStateOf(false) }
    var showPersonalIntelligence by remember { mutableStateOf(false) }

    val scope = rememberCoroutineScope()
    val responseStyles = listOf("professional", "casual", "playful")
    val responseStyleLabels = listOf(
        stringResource(R.string.chat_style_professional),
        stringResource(R.string.chat_style_casual),
        stringResource(R.string.chat_style_playful)
    )
    val isWebHostModel = selectedModel?.source == ModelInfo.ModelSource.WebHost
    val noModelSelected = stringResource(R.string.chat_no_model_selected)
    val settingsSaved = stringResource(R.string.chat_settings_saved)
    val saveFailed = stringResource(R.string.chat_save_failed)
    val snackbarHostState = remember { SnackbarHostState() }
    val enabledSkillCount = uploadedSkills.count { it.enabled } + hostUploadedSkills.count { it.enabled }
    val enabledMcpCount = hostMcpServers.count { it.enabled }
    val enabledDocumentCount = hostUploadedDocuments.count { it.enabled }
    val measuredContextUsage = contextUsage?.takeIf { it.maxTokens > 0 }
    val contextUsageSummary = measuredContextUsage?.let { usage ->
        val used = compactChatTokenCount(usage.totalTokens)
        val limit = compactChatTokenCount(usage.maxTokens)
        "$used / $limit · ${usage.usagePercent.toInt().coerceIn(0, 100)}%"
    }

    LaunchedEffect(errorMessage, statusMessage) {
        val message = errorMessage ?: statusMessage ?: return@LaunchedEffect
        snackbarHostState.showSnackbar(message)
        if (errorMessage == message) errorMessage = null
        if (statusMessage == message) statusMessage = null
    }

    if (showModelParameters && selectedModel != null) {
        val editingModel = selectedModel
        key(editingModel.selectionId) {
            ModelParametersSheet(
                modelName = editingModel.name,
                supportsTopK = editingModel.source == ModelInfo.ModelSource.Local,
                supportsStreamOutput = editingModel.source != ModelInfo.ModelSource.WebHost,
                parameters = draftParameters,
                reasoningEfforts = reasoningEfforts,
                saving = parameterSaving,
                error = parameterError,
                draftVersion = parameterDraftVersion,
                dismissOnSubmit = false,
                onDismiss = { if (!parameterSaving) showModelParameters = false },
                onReload = {
                    if (!parameterSaving) {
                        if (onReloadParameters()) {
                            parameterError = null
                            parameterDraftVersion += 1
                        } else { parameterError = saveFailed }
                    }
                },
                onUpdate = { parameters ->
                    if (!parameterSaving) {
                        parameterSaving = true
                        parameterError = null
                        scope.launch {
                            try {
                                onSaveParameters(parameters)
                                if (currentModelId == editingModel.selectionId) showModelParameters = false
                            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                                throw cancelled
                            } catch (error: Exception) {
                                if (currentModelId == editingModel.selectionId) parameterError = error.localizedMessage ?: saveFailed
                            } finally {
                                if (currentModelId == editingModel.selectionId) parameterSaving = false
                            }
                        }
                    }
                }
            )
        }
    }

    if (showConversationLimits) {
        val parsedContext = contextLimitText.trim().takeIf(String::isNotEmpty)?.toIntOrNull()
        val parsedOutput = outputLimitText.trim().takeIf(String::isNotEmpty)?.toIntOrNull()
        val contextValid = contextLimitText.isBlank() || parsedContext in ModelParameterLimits.remoteContextTokens
        val outputValid = outputLimitText.isBlank() || parsedOutput in ModelParameterLimits.outputTokens
        AlertDialog(
            onDismissRequest = { if (!conversationLimitsSaving) showConversationLimits = false },
            title = { Text(stringResource(R.string.chat_conversation_limits_title)) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    Text(stringResource(R.string.chat_conversation_limits_hint), style = MaterialTheme.typography.bodySmall)
                    Spacer(Modifier.height(NolaSpacing.md))
                    OutlinedTextField(
                        value = contextLimitText,
                        onValueChange = { contextLimitText = it },
                        label = { Text(stringResource(R.string.models_max_context)) },
                        placeholder = { Text(draftParameters.maxContextTokens.toString()) },
                        isError = !contextValid,
                        enabled = !conversationLimitsSaving,
                        modifier = Modifier.fillMaxWidth().testTag("conversation-context-limit"),
                    )
                    OutlinedTextField(
                        value = outputLimitText,
                        onValueChange = { outputLimitText = it },
                        label = { Text(stringResource(R.string.models_max_output)) },
                        placeholder = { Text(draftParameters.maxCompletionTokens.toString()) },
                        isError = !outputValid,
                        enabled = !conversationLimitsSaving,
                        modifier = Modifier.fillMaxWidth().testTag("conversation-output-limit"),
                    )
                    conversationLimitsError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                }
            },
            dismissButton = {
                TextButton(onClick = { showConversationLimits = false }, enabled = !conversationLimitsSaving) {
                    Text(stringResource(R.string.common_cancel))
                }
            },
            confirmButton = {
                TextButton(
                    enabled = contextValid && outputValid && !conversationLimitsSaving,
                    onClick = {
                        conversationLimitsSaving = true
                        conversationLimitsError = null
                        scope.launch {
                            try {
                                onSaveGenerationOverride(ConversationGenerationOverride(parsedContext, parsedOutput))
                                showConversationLimits = false
                            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                                throw cancelled
                            } catch (error: Exception) {
                                conversationLimitsError = error.localizedMessage ?: saveFailed
                            } finally {
                                conversationLimitsSaving = false
                            }
                        }
                    },
                ) { Text(stringResource(R.string.common_save)) }
            },
        )
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            NolaTopAppBar(
                title = stringResource(R.string.chat_more_settings_title),
                subtitle = selectedModel?.name ?: noModelSelected,
                onBack = { if (!isSaving && !parameterSaving) onBack() }
            ) {
                    IconButton(
                        modifier = Modifier.testTag("chat-settings-save"),
                        enabled = !isSaving && !parameterSaving && !permissionSaving,
                        onClick = {
                            scope.launch {
                                isSaving = true
                                errorMessage = null
                                statusMessage = null

                                try {

                                // Notify parent of changes
                                onResponseStyleChange(localResponseStyle)
                                onAutoCompactPercentChange(localAutoCompactPercent)
                                onSessionMemoryChange(localSessionMemory)
                                onLongTermMemoryChange(localLongTermMemory)
                                onUserPreferencesChange(localUserPreferences)
                                onAgentModeChange(localAgentMode)
                                onHybridDeviceToolsChange(localHybridDeviceTools)

                                // Save memory profile
                                    onSaveMemoryProfile()
                                    statusMessage = settingsSaved
                                } catch (cancelled: kotlinx.coroutines.CancellationException) {
                                    throw cancelled
                                } catch (e: Exception) {
                                    errorMessage = e.message ?: saveFailed
                                } finally {
                                    isSaving = false
                                }
                            }
                        }
                    ) {
                        if (isSaving) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(20.dp),
                                strokeWidth = 2.dp
                            )
                        } else {
                            Icon(Icons.Filled.Check, stringResource(R.string.common_save))
                        }
                    }
            }
        }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentAlignment = Alignment.TopCenter
        ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .widthIn(max = 840.dp)
                .verticalScroll(rememberScrollState())
                .padding(NolaSpacing.lg)
        ) {
            NolaCard {
                Column(modifier = Modifier.padding(vertical = 4.dp)) {
                    Column(
                        modifier = Modifier.fillMaxWidth(),
                        verticalArrangement = Arrangement.spacedBy(6.dp)
                    ) {
                        Text(
                            stringResource(R.string.chat_scope_current_conversation),
                            style = MaterialTheme.typography.labelSmall,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.primary
                        )
                        Text(
                            stringResource(R.string.chat_more_settings_desc),
                            modifier = Modifier.fillMaxWidth().testTag("chat-settings-scope-description"),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    if (pairedHostIdentity.isNotBlank()) {
                        NolaDivider()
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(stringResource(R.string.chat_host_pairing_identity), Modifier.weight(1f))
                            Text(
                                pairedHostIdentity.takeLast(12),
                                style = MaterialTheme.typography.labelSmall,
                                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        TextButton(onClick = { showForgetPairingConfirmation = true }) {
                            Text(
                                stringResource(R.string.chat_host_pairing_forget),
                                color = MaterialTheme.colorScheme.error
                            )
                        }
                        if (showForgetPairingConfirmation) {
                            AlertDialog(
                                onDismissRequest = { showForgetPairingConfirmation = false },
                                title = { Text(stringResource(R.string.chat_host_pairing_forget_confirm_title)) },
                                text = { Text(stringResource(R.string.chat_host_pairing_forget_confirm_message)) },
                                dismissButton = {
                                    TextButton(onClick = { showForgetPairingConfirmation = false }) {
                                        Text(stringResource(R.string.common_cancel))
                                    }
                                },
                                confirmButton = {
                                    TextButton(onClick = {
                                        showForgetPairingConfirmation = false
                                        onForgetHostPairing()
                                    }) {
                                        Text(
                                            stringResource(R.string.chat_host_pairing_forget_confirm_action),
                                            color = MaterialTheme.colorScheme.error
                                        )
                                    }
                                }
                            )
                        }
                    }
                }
            }

            OutlinedButton(
                onClick = onOpenContextSettings,
                modifier = Modifier.fillMaxWidth()
            ) {
                Icon(Icons.Filled.Storage, null, Modifier.size(18.dp))
                Spacer(Modifier.width(NolaSpacing.sm))
                Column(
                    modifier = Modifier.weight(1f),
                    horizontalAlignment = Alignment.Start
                ) {
                    Text(stringResource(R.string.chat_open_context_settings), fontWeight = FontWeight.SemiBold)
                    contextUsageSummary?.let { summary -> Text(
                        summary,
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    ) }
                    measuredContextUsage?.let { usage ->
                        Spacer(Modifier.height(5.dp))
                        LinearProgressIndicator(
                            progress = { (usage.usagePercent / 100f).coerceIn(0f, 1f) },
                            modifier = Modifier.fillMaxWidth(),
                            trackColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.10f)
                        )
                    }
                }
            }

            NolaDivider()

            // ---- Model and Output ----
            NolaSectionTitle(title = stringResource(R.string.chat_model_output))
            NolaCard {
                // Natural-width choices wrap as whole controls at compact widths.
                Text(stringResource(R.string.settings_response_style), style = MaterialTheme.typography.labelMedium)
                TextButton(onClick = { localResponseStyle = null }, enabled = localResponseStyle != null) {
                    Text(stringResource(R.string.chat_use_model_default))
                }
                Spacer(Modifier.height(NolaSpacing.sm))
                FlowRow(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm),
                    verticalArrangement = Arrangement.spacedBy(NolaSpacing.xs),
                ) {
                    responseStyles.forEachIndexed { index, style ->
                        FilterChip(
                            selected = localResponseStyle == style,
                            onClick = { localResponseStyle = style },
                            modifier = Modifier.heightIn(min = 48.dp),
                            label = { Text(responseStyleLabels[index]) },
                        )
                    }
                }

                Spacer(Modifier.height(NolaSpacing.md))

                TextButton(
                    modifier = Modifier.testTag("chat-conversation-generation-limits"),
                    enabled = conversationId != null && selectedModel != null &&
                        ConversationGenerationOverride.modelKeyOrNull(selectedModel) != null && !isSaving,
                    onClick = { conversationLimitsError = null; showConversationLimits = true },
                ) { Text(stringResource(R.string.chat_conversation_limits_title)) }
                Text(
                    stringResource(R.string.chat_conversation_limits_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                Spacer(Modifier.height(NolaSpacing.md))

                TextButton(
                    modifier = Modifier.testTag("chat-model-defaults"),
                    enabled = selectedModel != null && !isSaving && !parameterSaving,
                    onClick = {
                        if (onReloadParameters()) {
                            parameterError = null
                            parameterDraftVersion += 1
                            showModelParameters = true
                        } else {
                            errorMessage = saveFailed
                        }
                    }
                ) { Text(stringResource(R.string.chat_model_defaults_entry)) }
                Text(
                    stringResource(R.string.chat_model_defaults_scope),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            NolaDivider()

            // ---- Execution, host, and context window ----
            NolaSectionTitle(title = stringResource(R.string.chat_execution_context_section))
            NolaCard {
                ToggleRow(stringResource(R.string.settings_agent_mode), localAgentMode) {
                    localAgentMode = it
                }

                Spacer(Modifier.height(NolaSpacing.md))
                PermissionModeSelector(
                    selected = localPermissionMode,
                    enabled = !permissionSaving,
                    onSelected = { mode ->
                        if (!permissionSaving) {
                            localPermissionMode = mode
                            permissionSaving = true
                            scope.launch {
                                try { onPermissionModeChange(mode) }
                                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                                catch (error: Exception) { errorMessage = error.localizedMessage ?: saveFailed }
                                finally { permissionSaving = false }
                            }
                        }
                    },
                )

                if (isWebHostModel) {
                    NolaDivider()
                    ToggleRow(stringResource(R.string.chat_hybrid_device_tools), localHybridDeviceTools) {
                        localHybridDeviceTools = it
                        onHybridDeviceToolsChange(it)
                    }
                    Text(
                        text = stringResource(R.string.chat_hybrid_device_tools_hint),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )

                    if (localHybridDeviceTools) {
                        Spacer(Modifier.height(NolaSpacing.sm))
                        listOf(
                            "files" to stringResource(R.string.chat_hybrid_files),
                            "clipboard" to stringResource(R.string.chat_hybrid_clipboard),
                            "location" to stringResource(R.string.chat_hybrid_location),
                            "calendar" to stringResource(R.string.chat_hybrid_calendar)
                        ).forEach { (capability, label) ->
                            val enabled = when (capability) {
                                "files" -> "files_read" in localHybridCapabilities && "files_write" in localHybridCapabilities
                                else -> capability in localHybridCapabilities
                            }
                            ToggleRow(label, enabled) { selected ->
                                val updated = localHybridCapabilities.toMutableSet()
                                val ids = if (capability == "files") {
                                    listOf("files_read", "files_write")
                                } else {
                                    listOf(capability)
                                }
                                ids.forEach { id ->
                                    if (selected) updated += id else updated -= id
                                    onHybridDeviceCapabilityChange(id, selected)
                                }
                                localHybridCapabilities = updated
                            }
                        }
                        Text(
                            text = stringResource(R.string.chat_hybrid_capabilities_hint),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }

                    Text(
                        text = stringResource(R.string.chat_execution_host_footer),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )

                    if (hostComputerUseStatus != null) {
                        Spacer(Modifier.height(NolaSpacing.sm))
                        HostComputerUseStatusBlock(hostComputerUseStatus)
                    }
                    if (hostNativeBrowserStatus != null) {
                        Spacer(Modifier.height(NolaSpacing.sm))
                        HostNativeBrowserStatusBlock(hostNativeBrowserStatus)
                    }

                    Spacer(Modifier.height(NolaSpacing.sm))
                }

                // Auto-compact trigger stepper
                StepperRow(
                    label = stringResource(R.string.chat_auto_compact_trigger),
                    value = "${localAutoCompactPercent}%",
                    onIncrement = {
                        if (localAutoCompactPercent < 95) {
                            localAutoCompactPercent += 5
                            onAutoCompactPercentChange(localAutoCompactPercent)
                        }
                    },
                    onDecrement = {
                        if (localAutoCompactPercent > 20) {
                            localAutoCompactPercent -= 5
                            onAutoCompactPercentChange(localAutoCompactPercent)
                        }
                    }
                )

            }

            NolaDivider()
            NolaSectionTitle(title = stringResource(R.string.chat_global_libraries_title))
            Text(
                stringResource(R.string.chat_global_libraries_desc),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(NolaSpacing.sm))
            NolaCard {
                SettingsNavigationRow(
                    icon = Icons.Filled.GridView,
                    title = stringResource(R.string.chat_open_skill_library),
                    detail = stringResource(
                        R.string.chat_global_libraries_summary,
                        enabledSkillCount,
                        enabledMcpCount,
                        enabledDocumentCount
                    ),
                    onClick = onOpenSkillLibrary
                )
                NolaDivider()
                SettingsNavigationRow(
                    icon = Icons.Filled.Storage,
                    title = stringResource(R.string.personal_intelligence_title),
                    detail = stringResource(R.string.chat_personal_intelligence_manage_desc),
                    onClick = { showPersonalIntelligence = true }
                )
            }

            Spacer(Modifier.height(NolaSpacing.xxl))
        }
        }
    }

    if (showPersonalIntelligence) {
        PersonalIntelligenceDialog(onDismiss = { showPersonalIntelligence = false })
    }
}

@Composable
private fun SettingsNavigationRow(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    title: String,
    detail: String,
    onClick: () -> Unit
) {
    Surface(
        onClick = onClick,
        color = MaterialTheme.colorScheme.surface.copy(alpha = 0f)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Surface(
                shape = MaterialTheme.shapes.medium,
                color = MaterialTheme.colorScheme.primary.copy(alpha = 0.09f)
            ) {
                Icon(
                    icon,
                    contentDescription = null,
                    modifier = Modifier.padding(9.dp).size(18.dp),
                    tint = MaterialTheme.colorScheme.primary
                )
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                Text(
                    detail,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )
            }
            Text(
                "›",
                style = MaterialTheme.typography.titleLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun HostComputerUseStatusBlock(status: HostComputerUseStatus) {
    val statusText = when {
        !status.featureEnabled -> stringResource(R.string.chat_computer_use_disabled)
        !status.tokenConfigured -> stringResource(R.string.chat_computer_use_token_missing)
        status.connected -> stringResource(R.string.chat_computer_use_connected)
        else -> stringResource(R.string.chat_computer_use_disconnected)
    }
    Column(verticalArrangement = Arrangement.spacedBy(NolaSpacing.xs)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(stringResource(R.string.chat_computer_use_host), style = MaterialTheme.typography.bodyMedium)
            Text(
                statusText,
                style = MaterialTheme.typography.labelMedium,
                color = if (status.connected) NolaColors.success else MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Text(
            stringResource(R.string.chat_computer_use_capacity, status.hosts.size, status.tools.size),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        status.hosts.firstOrNull()?.let { host ->
            Text(
                host.hostId,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Composable
private fun HostNativeBrowserStatusBlock(status: HostNativeBrowserStatus) {
    val statusText = when {
        !status.tokenConfigured -> stringResource(R.string.chat_native_browser_token_missing)
        status.connected -> stringResource(R.string.chat_native_browser_connected)
        else -> stringResource(R.string.chat_native_browser_disconnected)
    }
    Column(verticalArrangement = Arrangement.spacedBy(NolaSpacing.xs)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(stringResource(R.string.chat_native_browser_host), style = MaterialTheme.typography.bodyMedium)
            Text(
                statusText,
                style = MaterialTheme.typography.labelMedium,
                color = if (status.connected) NolaColors.success else MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Text(
            stringResource(
                R.string.chat_native_browser_capacity,
                status.hosts.size,
                status.pendingCommands,
                status.inFlightCommands
            ),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        status.hosts.firstOrNull()?.let { host ->
            Text(
                host.hostId,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Composable
fun StepperRow(
    label: String,
    value: String,
    onIncrement: () -> Unit,
    onDecrement: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(label, style = MaterialTheme.typography.bodyMedium)
            Text(
                value,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        TextButton(onClick = onDecrement) {
            Text("−", style = MaterialTheme.typography.titleMedium)
        }
        TextButton(onClick = onIncrement) {
            Text("+", style = MaterialTheme.typography.titleMedium)
        }
    }
}


@Composable
fun ToggleRow(label: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .toggleable(value = checked, role = Role.Switch, onValueChange = onCheckedChange)
            .heightIn(min = 48.dp)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(label, modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
        Switch(checked = checked, onCheckedChange = null)
    }
}
