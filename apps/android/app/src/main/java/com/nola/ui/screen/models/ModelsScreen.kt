package com.nola.ui.screen.models

import com.nola.ui.components.dismissImeOnBlankTap
import com.nola.ui.components.ForgetHostPairingDialog
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.layout.width
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.stateDescription

import android.content.Intent
import android.net.Uri
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import com.nola.ui.components.NolaRadioOption
import com.nola.ui.components.NolaIconButton
import com.nola.ui.components.NolaSwitch
import com.nola.ui.components.NolaDisclosureBar
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import com.nola.domain.model.RemoteModelEndpointURL
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.remote.dto.MessagingStatusDto
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ChatOutputTokenField
import com.nola.domain.model.ModelWireProtocol
import com.nola.llm.local.DeviceModelCatalogEntry
import com.nola.ui.components.NolaBadge
import com.nola.ui.components.ModelParametersSheet
import com.nola.ui.components.SmoothStreamingMarkdownRenderer
import com.nola.ui.components.DiscardChangesDialog
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaCardCompact
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.NolaPage
import com.nola.ui.components.NolaPageHeader
import com.nola.ui.components.NolaRadius
import com.nola.ui.components.NolaSectionTitle
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.rememberNolaHaptics
import com.nola.ui.components.rememberReduceMotionEnabled

internal const val MODEL_ROW_TEST_TAG_PREFIX = "models.row."
internal const val MODEL_ROW_COPY_TEST_TAG_PREFIX = "models.row.copy."
internal const val MODEL_ROW_ACTIONS_TEST_TAG_PREFIX = "models.row.actions."
internal const val MODEL_API_CONNECTION_TEST_TAG = "models.connection.api"
internal const val MODEL_HOST_MESSAGING_CARD_TEST_TAG = "models.host.messaging.card"
internal const val MODEL_HOST_MESSAGING_MANAGE_TEST_TAG = "models.host.messaging.manage"
internal const val MODEL_HOST_CONNECTION_TEST_TAG = "models.connection.host"

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModelsScreen(
    viewModel: ModelsViewModel = hiltViewModel(),
    onBack: (() -> Unit)? = null,
) {
    val context = LocalContext.current
    val manageHostModels = {
        viewModel.openHostMessagingManagement("/models?tab=api") { target ->
            runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(target))) }
                .onFailure { viewModel.reportHostManagementLaunchFailure(it) }
        }
    }
    val reduceMotion = rememberReduceMotionEnabled()
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    var sourceTab by rememberSaveable { mutableStateOf(0) }
    var confirmDisconnectAddress by rememberSaveable { mutableStateOf<String?>(null) }
    var showWebHostSheet by rememberSaveable { mutableStateOf(false) }
    var capabilityTarget by remember { mutableStateOf<ModelInfo?>(null) }
    var capabilityError by remember { mutableStateOf<String?>(null) }
    var savingCapabilities by remember { mutableStateOf(false) }
    var modelToDelete by remember { mutableStateOf<ModelInfo?>(null) }

    modelToDelete?.let { model ->
        AlertDialog(
            onDismissRequest = { modelToDelete = null },
            title = { Text(stringResource(R.string.common_delete)) },
            text = { Text(stringResource(R.string.ui_delete_model_message, model.name)) },
            confirmButton = {
                TextButton(onClick = { viewModel.deleteModel(model); modelToDelete = null }) {
                    Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { modelToDelete = null }) { Text(stringResource(R.string.common_cancel)) } },
        )
    }

    if (confirmDisconnectAddress != null && confirmDisconnectAddress == state.webHostUrl) {
        ForgetHostPairingDialog(
            onConfirm = { confirmDisconnectAddress = null; viewModel.disconnectWebHost() },
            onDismiss = { confirmDisconnectAddress = null }
        )
    }

    if (showWebHostSheet) {
        HostPairingSheet(onDismiss = { showWebHostSheet = false }, viewModel = viewModel)
    }

    capabilityTarget?.let { target ->
        ModelExecutionCapabilitiesSheet(
            model = target,
            saving = savingCapabilities,
            error = capabilityError,
            onSave = { capabilities ->
                savingCapabilities = true
                capabilityError = null
                viewModel.saveExecutionCapabilities(target, capabilities) { error ->
                    savingCapabilities = false
                    capabilityError = error
                    if (error == null) capabilityTarget = null
                }
            },
            onDismiss = { if (!savingCapabilities) capabilityTarget = null },
        )
    }

    if (state.showParameters) {
        state.parameterTarget?.let { target ->
            ModelParametersSheet(
                modelName = target.name,
                supportsTopK = target.source == ModelInfo.ModelSource.Local,
                supportsStreamOutput = target.source != ModelInfo.ModelSource.WebHost,
                parameters = state.parameters,
                reasoningEfforts = state.parameterSnapshot?.reasoningEfforts.orEmpty(),
                onUpdate = { viewModel.updateParameters(it) },
                saving = state.savingParameters,
                error = state.parameterError,
                draftVersion = state.parameterEditorVersion,
                onReload = viewModel::reloadParameters,
                dismissOnSubmit = false,
                onDismiss = {
                    viewModel.closeParameters()
                }
            )
        }
    }

    state.modelTestTarget?.let { target ->
        ModelInferenceTestSheet(
            model = target,
            prompt = state.modelTestPrompt,
            output = state.modelTestOutput,
            running = state.modelTestRunning,
            firstTokenMs = state.modelTestFirstTokenMs,
            totalMs = state.modelTestTotalMs,
            error = state.modelTestError,
            onPromptChange = viewModel::updateModelTestPrompt,
            onRun = viewModel::runModelTest,
            onStop = viewModel::stopModelTest,
            onDismiss = viewModel::closeModelTest,
        )
    }

    NolaPage(modifier = Modifier.imePadding(), scrollKey = sourceTab) {
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            if (onBack != null) {
                IconButton(onClick = onBack) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = stringResource(R.string.common_back),
                    )
                }
            }
            NolaPageHeader(
                title = stringResource(R.string.models_title),
                subtitle = stringResource(R.string.models_subtitle),
                compactTrailing = true,
                modifier = Modifier.weight(1f),
            ) {
                val refreshLabel = stringResource(R.string.common_refresh)
                val loadingLabel = stringResource(R.string.models_loading)
                NolaIconButton(label = refreshLabel, onClick = { viewModel.loadModels() }, enabled = !state.isLoading,
                    modifier = Modifier.size(48.dp).testTag("models.refresh").semantics {
                        contentDescription = refreshLabel
                        if (state.isLoading) stateDescription = loadingLabel
                    }) {
                    if (state.isLoading) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp
                        )
                    } else {
                        Icon(Icons.Filled.Refresh, null, tint = MaterialTheme.colorScheme.primary)
                    }
                }
            }
        }


        Row(Modifier.fillMaxWidth().selectableGroup()
            .background(MaterialTheme.colorScheme.surfaceContainer, RoundedCornerShape(10.dp)).padding(3.dp),
            horizontalArrangement = Arrangement.spacedBy(2.dp)) {
            listOf(R.string.models_service_tab, R.string.models_local_models, R.string.models_web_host).forEachIndexed { index, title ->
                val selected = sourceTab == index
                Box(
                    modifier = Modifier.weight(1f).heightIn(min = 48.dp).clip(RoundedCornerShape(8.dp))
                        .background(if (selected) MaterialTheme.colorScheme.surface else Color.Transparent)
                        .selectable(selected = selected, role = Role.Tab, onClick = {
                            if (!selected) viewModel.clearConnectionStatus()
                            sourceTab = index
                        })
                        .padding(horizontal = 6.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(stringResource(title), style = MaterialTheme.typography.labelLarge,
                        color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center)
                }
            }
        }
        AnimatedVisibility(
            visible = state.connectionTestResult?.isNotEmpty() == true,
            enter = nolaDisclosureEnter(reduceMotion),
            exit = nolaDisclosureExit(reduceMotion),
        ) {
            state.connectionTestResult?.takeIf { it.isNotEmpty() }?.let { result ->
                val success = state.connectionTestSucceeded != false
                val statusTint = if (success) NolaColors.success else MaterialTheme.colorScheme.error
                Row(
                modifier = Modifier
                    .fillMaxWidth().semantics { liveRegion = androidx.compose.ui.semantics.LiveRegionMode.Polite }
                    .clip(RoundedCornerShape(NolaRadius.lg))
                    .background(
                        statusTint.copy(alpha = 0.08f)
                    )
                    .padding(horizontal = NolaSpacing.md, vertical = NolaSpacing.sm),
                horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm),
                verticalAlignment = Alignment.CenterVertically
                ) {
                Icon(
                    imageVector = if (success) Icons.Filled.Check else Icons.Filled.ErrorOutline,
                    contentDescription = null,
                    tint = statusTint,
                    modifier = Modifier.size(18.dp)
                )
                Text(
                    result,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.weight(1f)
                )
                IconButton(
                    onClick = viewModel::clearConnectionStatus,
                    modifier = Modifier.size(48.dp)
                ) {
                    Icon(
                        Icons.Filled.Close,
                        contentDescription = stringResource(R.string.common_close),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(17.dp)
                    )
                }
                }
            }
        }

        if (sourceTab == 0) {
        EndpointConfigFields(
            initialEndpoint = "", initialApiKey = "", initialModel = "",
            initialWireProtocol = ModelWireProtocol.ChatCompletions,
            saving = state.savingEndpoint, error = state.endpointSaveError, draft = viewModel.connectionDraft,
            onSave = { endpoint, key, model, protocol -> viewModel.saveApiEndpoint(endpoint, key, model, protocol) }
        )

            ModelConnectionsSection(state, viewModel::loadConnectionInventory, connectHost = {
                sourceTab = 2
                showWebHostSheet = true
            }, manageHost = manageHostModels)

            if (state.officialModels.isNotEmpty()) {
                ModelListCard(
                    title = stringResource(R.string.models_api_models),
                    models = state.officialModels,
                    selectedModelId = state.selectedModelId,
                    selectedModelSource = state.selectedModelSource,
                    activatingModelId = state.activatingModelId,
                    activatingModelSource = state.activatingModelSource,
                    onSelect = { viewModel.selectModel(it) },
                    onTest = { viewModel.openModelTest(it) },
                    onParams = { viewModel.openParameters(it) },
                )
            }

            if (state.configuredApiModels.isNotEmpty()) {
                ModelListCard(
                    title = stringResource(R.string.models_configured_endpoint_models),
                    models = state.configuredApiModels,
                    selectedModelId = state.selectedModelId,
                    selectedModelSource = state.selectedModelSource,
                    activatingModelId = state.activatingModelId,
                    activatingModelSource = state.activatingModelSource,
                    onSelect = { viewModel.selectModel(it) },
                    onDeleteLocal = { modelToDelete = it },
                    onTest = { viewModel.openModelTest(it) },
                    onParams = { viewModel.openParameters(it) },
                    onCapabilities = { capabilityError = null; capabilityTarget = it },
                )
            }

        }
        if (sourceTab == 1) {
            if (state.installedLocalModels.isNotEmpty()) {
                ModelListCard(
                    title = stringResource(R.string.models_local_models),
                    models = state.installedLocalModels,
                    selectedModelId = state.selectedModelId,
                    selectedModelSource = state.selectedModelSource,
                    activatingModelId = state.activatingModelId,
                    activatingModelSource = state.activatingModelSource,
                    runningLocalModelId = state.runningLocalModelId,
                    onSelect = { viewModel.selectModel(it) },
                    onStartLocal = { model ->
                        if (state.runningLocalModelId == model.id) viewModel.stopLocalModel()
                        else viewModel.startLocalModel(model)
                    },
                    onDeleteLocal = { modelToDelete = it },
                    onTest = { viewModel.openModelTest(it) },
                    onParams = { viewModel.openParameters(it) }
                )
            }

            NolaCard {
                NolaSectionTitle(icon = Icons.Filled.Download, title = stringResource(R.string.models_download_models))
                state.deviceCatalog.forEach { entry ->
                    DeviceModelCatalogRow(
                        entry = entry,
                        installed = state.installedLocalModels.any { it.id == entry.id },
                        downloading = state.downloadingModelId == entry.id,
                        progress = state.downloadProgress,
                        downloadsBlocked = state.downloadingModelId != null,
                        onDownload = { viewModel.downloadLocalModel(entry) },
                        onCancelDownload = { viewModel.cancelLocalModelDownload() },
                    )
                }
            }

        }
        if (sourceTab == 2) {
            ModelActionTile(title = stringResource(R.string.models_web_host),
                modifier = Modifier.testTag(MODEL_HOST_CONNECTION_TEST_TAG),
                subtitle = state.webHostUrl.ifBlank { stringResource(R.string.models_desktop_bridge) },
                icon = Icons.Filled.Security, tint = MaterialTheme.colorScheme.primary,
                onClick = { showWebHostSheet = true })
            state.webHostModel?.let { webHostModel ->
                ModelListCard(
                    title = stringResource(R.string.models_web_host),
                    models = listOf(webHostModel),
                    selectedModelId = state.selectedModelId,
                    selectedModelSource = state.selectedModelSource,
                    activatingModelId = state.activatingModelId,
                    activatingModelSource = state.activatingModelSource,
                    onSelect = { viewModel.selectModel(it) },
                    onParams = { viewModel.openParameters(it) }
                )
                TextButton(enabled = !state.openingHostManagement, onClick = manageHostModels,
                    modifier = Modifier.fillMaxWidth().testTag("models.web_host.manage")) {
                    Text(stringResource(R.string.model_connections_host_manage))
                }
                TextButton(
                    onClick = { confirmDisconnectAddress = state.webHostUrl },
                    modifier = Modifier.fillMaxWidth().testTag("models.web_host.disconnect")
                ) {
                    Text(
                        stringResource(R.string.models_web_host_disconnect),
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }

        }

        }
    }

@Composable
fun ModelListCard(
    title: String,
    models: List<ModelInfo>,
    selectedModelId: String?,
    selectedModelSource: ModelInfo.ModelSource?,
    activatingModelId: String?,
    activatingModelSource: ModelInfo.ModelSource?,
    runningLocalModelId: String? = null,
    onSelect: (ModelInfo) -> Unit,
    onStartLocal: ((ModelInfo) -> Unit)? = null,
    onDeleteLocal: ((ModelInfo) -> Unit)? = null,
    onTest: ((ModelInfo) -> Unit)? = null,
    onCapabilities: ((ModelInfo) -> Unit)? = null,
    onParams: (ModelInfo) -> Unit
) {
    NolaCard {
        NolaSectionTitle(icon = Icons.AutoMirrored.Filled.List, title = title)
        val anyActivating = activatingModelId != null
        models.forEachIndexed { index, model ->
            ModelRow(
                model = model,
                isSelected = model.selectionId == selectedModelId && model.source == selectedModelSource,
                isActivating = model.selectionId == activatingModelId && model.source == activatingModelSource,
                isAnyActivating = anyActivating,
                isRunning = model.id == runningLocalModelId,
                onClick = { onSelect(model) },
                onStartLocal = onStartLocal?.let { action -> { action(model) } },
                onDeleteLocal = onDeleteLocal?.let { action -> { action(model) } },
                onTest = onTest?.let { action -> { action(model) } },
                onCapabilities = onCapabilities?.let { action -> { action(model) } },
                onParams = { onParams(model) }
            )
            if (index != models.lastIndex) {
                HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.18f))
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ModelExecutionCapabilitiesSheet(
    model: ModelInfo,
    saving: Boolean,
    error: String?,
    onSave: (ModelExecutionCapabilities) -> Unit,
    onDismiss: () -> Unit,
) {
    val initial = model.executionCapabilities
    var toolCalls by remember(model.profileId) { mutableStateOf(initial.toolCalls) }
    var imageInput by remember(model.profileId) { mutableStateOf(initial.imageInput) }
    var samplingParameters by remember(model.profileId) { mutableStateOf(initial.samplingParameters) }
    var outputField by remember(model.profileId) { mutableStateOf(initial.chatOutputTokenField) }
    var showDiscardDialog by remember { mutableStateOf(false) }
    val dirty = toolCalls != initial.toolCalls || imageInput != initial.imageInput ||
        samplingParameters != initial.samplingParameters || outputField != initial.chatOutputTokenField
    val requestDismiss = { if (!saving) { if (dirty) showDiscardDialog = true else onDismiss() } }
    ModalBottomSheet(
        modifier = Modifier.dismissImeOnBlankTap(),
        onDismissRequest = requestDismiss, sheetState = com.nola.ui.components.rememberGuardedEditorSheetState(saving, dirty) { showDiscardDialog = true }) {
        Column(
            modifier = Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).navigationBarsPadding().padding(horizontal = 24.dp, vertical = 20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(stringResource(R.string.models_execution_capabilities), style = MaterialTheme.typography.titleLarge)
            Text(model.name, style = MaterialTheme.typography.bodyMedium)
            Text(stringResource(R.string.models_execution_capabilities_hint),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            listOf(
                Triple(R.string.models_execution_tools, toolCalls, { value: Boolean -> toolCalls = value }),
                Triple(R.string.models_execution_images, imageInput, { value: Boolean -> imageInput = value }),
                Triple(R.string.models_execution_sampling, samplingParameters, { value: Boolean -> samplingParameters = value }),
            ).forEach { (label, checked, update) ->
                Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).toggleable(value = checked, enabled = !saving, role = Role.Switch, onValueChange = update).heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(label), modifier = Modifier.weight(1f))
                    NolaSwitch(checked = checked, onCheckedChange = null, enabled = !saving)
                }
            }
            Text(stringResource(R.string.models_execution_output_field), style = MaterialTheme.typography.titleSmall)
            listOf(
                ChatOutputTokenField.Auto to stringResource(R.string.models_execution_output_auto),
                ChatOutputTokenField.MaxTokens to "max_tokens",
                ChatOutputTokenField.MaxCompletionTokens to "max_completion_tokens",
            ).forEach { (value, label) ->
                NolaRadioOption(label = label, selected = outputField == value, enabled = !saving) {
                    outputField = value
                }
            }
            if (error != null) Text(error, color = MaterialTheme.colorScheme.error)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = requestDismiss, enabled = !saving) { Text(stringResource(R.string.common_cancel)) }
                Button(onClick = { onSave(initial.copy(
                    toolCalls = toolCalls, imageInput = imageInput, samplingParameters = samplingParameters,
                    chatOutputTokenField = outputField,
                )) }, enabled = (dirty || !model.executionCapabilitiesDeclared) && !saving) { Text(stringResource(R.string.common_save)) }
            }
        }
    }
    if (showDiscardDialog) {
        DiscardChangesDialog(onDiscard = onDismiss, onKeepEditing = { showDiscardDialog = false })
    }
}

@Composable
private fun DeviceModelCatalogRow(
    entry: DeviceModelCatalogEntry,
    installed: Boolean,
    downloading: Boolean,
    progress: Float?,
    downloadsBlocked: Boolean,
    onDownload: () -> Unit,
    onCancelDownload: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(entry.displayName, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
            Text(
                stringResource(
                    R.string.models_device_model_detail,
                    entry.quantization,
                    entry.byteSize.toDouble() / (1024.0 * 1024.0 * 1024.0),
                    entry.recommendedRamBytes / (1024 * 1024 * 1024),
                    entry.contextWindow / 1024,
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (downloading) {
                LinearProgressIndicator(
                    progress = { progress ?: 0f },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
        when {
            installed -> NolaBadge(stringResource(R.string.models_installed), icon = Icons.Filled.Check)
            downloading -> TextButton(onClick = onCancelDownload) {
                Text(stringResource(R.string.common_cancel))
            }
            else -> TextButton(onClick = onDownload, enabled = !downloadsBlocked) {
                Text(stringResource(R.string.models_download))
            }
        }
    }
}

@Composable
internal fun HostMessagingRow(
    messaging: MessagingStatusDto,
    openingHostManagement: Boolean,
    onManage: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val connected = messaging.channels.count { it.enabled && it.state == "connected" }
    Row(
        modifier = modifier
            .fillMaxWidth()
            .testTag(MODEL_HOST_MESSAGING_CARD_TEST_TAG),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
    ) {
        Icon(
            Icons.Filled.Security,
            contentDescription = null,
            tint = if (messaging.worker.enabled) NolaColors.success else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(20.dp)
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(stringResource(R.string.models_host_messaging_title), style = MaterialTheme.typography.labelLarge)
            Text(
                stringResource(
                    R.string.models_host_messaging_summary,
                    connected,
                    messaging.channels.size,
                    stringResource(if (messaging.worker.enabled) R.string.models_host_messaging_running else R.string.models_host_messaging_stopped)
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Text(
                stringResource(R.string.models_host_messaging_providers),
                style = MaterialTheme.typography.labelSmall,
                color = NolaColors.primary
            )
        }
        messaging.managementPath?.let { path ->
            TextButton(
                modifier = Modifier.testTag(MODEL_HOST_MESSAGING_MANAGE_TEST_TAG),
                enabled = !openingHostManagement,
                onClick = { onManage(path) },
            ) {
                if (openingHostManagement) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp
                    )
                } else {
                    Text(stringResource(R.string.models_host_messaging_manage))
                }
            }
        }
    }
}

@Composable
fun ModelRow(
    model: ModelInfo,
    isSelected: Boolean,
    isActivating: Boolean,
    isAnyActivating: Boolean = isActivating,
    isRunning: Boolean = false,
    onClick: () -> Unit,
    onStartLocal: (() -> Unit)? = null,
    onDeleteLocal: (() -> Unit)? = null,
    onTest: (() -> Unit)? = null,
    onCapabilities: (() -> Unit)? = null,
    onParams: () -> Unit
) {
    val sourceIcon = when (model.source) {
        ModelInfo.ModelSource.Cloud -> Icons.Filled.Cloud
        ModelInfo.ModelSource.Local -> Icons.Filled.Tune
        ModelInfo.ModelSource.WebHost -> Icons.Filled.Security
    }
    val sourceTint = when (model.source) {
        ModelInfo.ModelSource.Cloud -> MaterialTheme.colorScheme.primary
        ModelInfo.ModelSource.Local -> MaterialTheme.colorScheme.tertiary
        ModelInfo.ModelSource.WebHost -> NolaColors.success
    }
    val rowBackground by animateColorAsState(
        targetValue = if (isSelected) {
            MaterialTheme.colorScheme.primary.copy(alpha = 0.07f)
        } else {
            Color.Transparent
        },
        animationSpec = tween(NolaMotion.base, easing = NolaMotion.emphasizedEasing),
        label = "model-row-selection"
    )

    BoxWithConstraints(modifier = Modifier.fillMaxWidth()) {
        val compact = maxWidth < 380.dp
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .testTag(MODEL_ROW_TEST_TAG_PREFIX + model.id)
                .heightIn(min = 62.dp)
                .clip(RoundedCornerShape(NolaRadius.md))
                .background(rowBackground)
                .padding(horizontal = 10.dp, vertical = 9.dp),
            verticalArrangement = Arrangement.spacedBy(if (compact) 6.dp else 0.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Box(
                    modifier = Modifier
                        .size(34.dp)
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(sourceTint.copy(alpha = 0.10f)),
                    contentAlignment = Alignment.Center
                ) {
                    Icon(
                        sourceIcon,
                        contentDescription = null,
                        tint = sourceTint,
                        modifier = Modifier.size(17.dp)
                    )
                }
                Spacer(Modifier.size(NolaSpacing.sm))
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .clickable(enabled = !isAnyActivating) { onClick() }
                        .padding(horizontal = 4.dp, vertical = 2.dp)
                        .testTag(MODEL_ROW_COPY_TEST_TAG_PREFIX + model.id)
                ) {
                    Text(
                        model.name,
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(
                        when (model.source) {
                            ModelInfo.ModelSource.Cloud -> stringResource(R.string.models_cloud_detail)
                            ModelInfo.ModelSource.Local -> stringResource(R.string.models_local_detail)
                            ModelInfo.ModelSource.WebHost -> stringResource(R.string.models_web_host_detail)
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
                if (!compact) {
                    Spacer(Modifier.size(NolaSpacing.sm))
                    ModelRowActions(
                        model = model,
                        isSelected = isSelected,
                        isActivating = isActivating,
                        isAnyActivating = isAnyActivating,
                        isRunning = isRunning,
                        onClick = onClick,
                        onStartLocal = onStartLocal,
                        onDeleteLocal = onDeleteLocal,
                        onTest = onTest,
                        onCapabilities = onCapabilities,
                        onParams = onParams
                    )
                }
            }
            if (compact) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    ModelRowActions(
                        model = model,
                        isSelected = isSelected,
                        isActivating = isActivating,
                        isAnyActivating = isAnyActivating,
                        isRunning = isRunning,
                        onClick = onClick,
                        onStartLocal = onStartLocal,
                        onDeleteLocal = onDeleteLocal,
                        onTest = onTest,
                        onCapabilities = onCapabilities,
                        onParams = onParams
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ModelRowActions(
    model: ModelInfo,
    isSelected: Boolean,
    isActivating: Boolean,
    isAnyActivating: Boolean,
    isRunning: Boolean,
    onClick: () -> Unit,
    onStartLocal: (() -> Unit)?,
    onDeleteLocal: (() -> Unit)?,
    onTest: (() -> Unit)?,
    onCapabilities: (() -> Unit)?,
    onParams: () -> Unit
) {
    val haptics = rememberNolaHaptics()
    FlowRow(
        modifier = Modifier
            .testTag(MODEL_ROW_ACTIONS_TEST_TAG_PREFIX + model.id)
            .widthIn(min = 142.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.End),
        verticalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        if (model.source == ModelInfo.ModelSource.Local && onStartLocal != null) {
            TextButton(onClick = {
                if (isRunning) haptics.warning() else haptics.primaryAction()
                onStartLocal()
            }, enabled = !isAnyActivating) {
                Text(stringResource(if (isRunning) R.string.models_stop else R.string.models_start))
            }
        }
        if (model.source != ModelInfo.ModelSource.WebHost && onDeleteLocal != null) {
            IconButton(
                onClick = {
                    haptics.warning()
                    onDeleteLocal()
                },
                enabled = !isAnyActivating && !isRunning,
                modifier = Modifier.alpha(if (isRunning) 0.38f else 1f)
            ) {
                Icon(Icons.Filled.Delete, contentDescription = stringResource(R.string.common_delete))
            }
        }
        val actionState = when {
            isActivating -> 0
            isSelected -> 1
            else -> 2
        }
        AnimatedContent(
            targetState = actionState,
            transitionSpec = {
                fadeIn(tween(NolaMotion.base, easing = NolaMotion.emphasizedEasing)) togetherWith
                    fadeOut(tween(NolaMotion.fast))
            },
            contentAlignment = Alignment.CenterEnd,
            label = "model-row-action"
        ) { state ->
            when (state) {
            0 -> {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(15.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.primary
                    )
                    Spacer(Modifier.size(6.dp))
                    Text(
                        stringResource(R.string.models_activating),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary
                    )
                }
            }
            1 -> {
                NolaBadge(
                    stringResource(R.string.models_selected),
                    icon = Icons.Filled.Check,
                    tint = MaterialTheme.colorScheme.primary
                )
            }
            else -> {
                TextButton(onClick = onClick, enabled = !isAnyActivating) {
                    Icon(Icons.Filled.Check, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.size(4.dp))
                    Text(stringResource(R.string.models_use))
                }
            }
        }
        }
        if (model.source != ModelInfo.ModelSource.WebHost) {
            if (onCapabilities != null) {
                IconButton(onClick = onCapabilities, enabled = !isAnyActivating) {
                    Icon(Icons.Filled.Security, contentDescription = stringResource(R.string.models_execution_capabilities))
                }
            }
            NolaIconButton(label = stringResource(R.string.models_inference_test), onClick = { onTest?.invoke() }, enabled = !isAnyActivating && onTest != null) {
                Icon(
                    painterResource(R.drawable.nola_execution),
                    contentDescription = stringResource(R.string.models_inference_test),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            NolaIconButton(label = stringResource(R.string.common_parameters), onClick = onParams, enabled = !isAnyActivating) {
                Icon(
                    painterResource(R.drawable.nola_settings),
                    contentDescription = stringResource(R.string.common_parameters),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ModelInferenceTestSheet(
    model: ModelInfo,
    prompt: String,
    output: String,
    running: Boolean,
    firstTokenMs: Long?,
    totalMs: Long?,
    error: String?,
    onPromptChange: (String) -> Unit,
    onRun: () -> Unit,
    onStop: () -> Unit,
    onDismiss: () -> Unit,
) {
    ModalBottomSheet(
        modifier = Modifier.dismissImeOnBlankTap(),
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .imePadding()
                .navigationBarsPadding()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.md),
            verticalArrangement = Arrangement.spacedBy(NolaSpacing.md),
        ) {
            Text(stringResource(R.string.models_inference_test), style = MaterialTheme.typography.titleLarge)
            Text(model.name, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(
                stringResource(R.string.models_inference_test_hint),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            OutlinedTextField(
                value = prompt,
                onValueChange = onPromptChange,
                label = { Text(stringResource(R.string.models_inference_test_prompt)) },
                modifier = Modifier.fillMaxWidth(),
                minLines = 3,
                enabled = !running,
            )
            Button(onClick = if (running) onStop else onRun, enabled = running || prompt.isNotBlank()) {
                if (running) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(16.dp),
                        strokeWidth = 2.dp,
                        color = MaterialTheme.colorScheme.onPrimary,
                    )
                    Spacer(Modifier.size(8.dp))
                }
                Text(stringResource(if (running) R.string.models_stop else R.string.models_inference_test_run))
            }
            if (firstTokenMs != null || totalMs != null || output.isNotEmpty()) {
                Text(
                    listOfNotNull(
                        firstTokenMs?.let { stringResource(R.string.models_inference_test_first_token, it) },
                        totalMs?.let { stringResource(R.string.models_inference_test_total, it) },
                        output.takeIf(String::isNotEmpty)?.let {
                            stringResource(R.string.models_inference_test_characters, it.codePointCount(0, it.length))
                        },
                    ).joinToString(" · "),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (error != null) {
                Text(error, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            }
            if (output.isNotEmpty()) {
                Text(stringResource(R.string.models_inference_test_output), style = MaterialTheme.typography.titleSmall)
                SmoothStreamingMarkdownRenderer(text = output, isStreaming = running)
            }
            Spacer(Modifier.height(NolaSpacing.lg))
        }
    }
}

@Composable
private fun ConnectionTestRow(
    testing: Boolean,
    enabled: Boolean,
    onClick: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = NolaSpacing.xs),
        horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp)
        ) {
            Text(
                stringResource(R.string.models_test_connection),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold
            )
            Text(
                stringResource(R.string.models_verify_endpoint),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Button(
            onClick = onClick,
            enabled = enabled,
            modifier = Modifier.heightIn(min = 48.dp),
            shape = RoundedCornerShape(12.dp),
            colors = androidx.compose.material3.ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.onSurface,
                contentColor = MaterialTheme.colorScheme.surface
            )
        ) {
            if (testing) {
                CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.surface
                )
                Spacer(Modifier.size(7.dp))
            } else {
                Icon(Icons.Filled.Refresh, contentDescription = null, modifier = Modifier.size(16.dp))
                Spacer(Modifier.size(7.dp))
            }
            Text(
                stringResource(
                    if (testing) R.string.models_testing_connection else R.string.models_test_connection
                )
            )
        }
    }
}

@Composable
fun ModelActionTile(
    title: String,
    subtitle: String,
    icon: ImageVector,
    tint: Color,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    onClick: () -> Unit
) {
    val shape = RoundedCornerShape(NolaRadius.lg)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .clickable(enabled = enabled, onClick = onClick)
            .heightIn(min = 58.dp)
            .padding(horizontal = NolaSpacing.md, vertical = NolaSpacing.sm),
        horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Box(
            modifier = Modifier
                .size(34.dp)
                .clip(RoundedCornerShape(NolaRadius.md))
                .background(tint.copy(alpha = 0.12f)),
            contentAlignment = Alignment.Center
        ) {
            Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(18.dp))
        }
          Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                softWrap = true
            )
            Text(
                subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                softWrap = true
              )
          }
      }
  }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EndpointConfigSheet(
    currentEndpoint: String,
    currentApiKey: String,
    currentModel: String,
    currentWireProtocol: ModelWireProtocol,
    onSave: (String, String, String, ModelWireProtocol) -> Unit,
    onDismiss: () -> Unit,
    saving: Boolean = false,
    error: String? = null,
) {
    var showDiscardDialog by remember { mutableStateOf(false) }
    var editorDirty by remember { mutableStateOf(false) }
    val dirty = editorDirty
    val requestDismiss = { if (!saving) { if (dirty) showDiscardDialog = true else onDismiss() } }
    val sheetState = com.nola.ui.components.rememberGuardedEditorSheetState(saving, dirty) { showDiscardDialog = true }

    ModalBottomSheet(
        modifier = Modifier.dismissImeOnBlankTap(),
        onDismissRequest = requestDismiss, sheetState = sheetState) {
        Column(Modifier.fillMaxWidth().imePadding().navigationBarsPadding().padding(24.dp)
            .verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(stringResource(R.string.models_endpoint_sheet_title), style = MaterialTheme.typography.titleLarge)
            EndpointConfigFields(currentEndpoint, currentApiKey, currentModel, currentWireProtocol,
                saving = saving, error = error, onSave = onSave,
                onDirtyChange = { value -> editorDirty = value })
            TextButton(onClick = requestDismiss, enabled = !saving) { Text(stringResource(R.string.common_cancel)) }
        }
    }

    if (showDiscardDialog) {
        DiscardChangesDialog(
            onDiscard = onDismiss,
            onKeepEditing = { showDiscardDialog = false }
        )
    }
}

internal class EndpointConfigDraft(endpoint: String, key: String, model: String, protocol: ModelWireProtocol) {
    var endpoint by mutableStateOf(endpoint)
    var key by mutableStateOf(key)
    var model by mutableStateOf(model)
    var protocol by mutableStateOf(protocol)
}

@Composable
private fun EndpointConfigFields(
    initialEndpoint: String, initialApiKey: String, initialModel: String,
    initialWireProtocol: ModelWireProtocol, saving: Boolean, error: String?,
    onSave: (String, String, String, ModelWireProtocol) -> Unit,
    onDirtyChange: (Boolean) -> Unit = {},
    draft: EndpointConfigDraft = remember { EndpointConfigDraft(initialEndpoint, initialApiKey, initialModel, initialWireProtocol) },
) {
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    var endpoint by draft::endpoint
    var key by draft::key
    var model by draft::model
    var protocol by draft::protocol
    LaunchedEffect(endpoint, key, model, protocol) {
        onDirtyChange(endpoint != initialEndpoint || key != initialApiKey || model != initialModel || protocol != initialWireProtocol)
    }
    Column(Modifier.widthIn(max = 560.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        OutlinedTextField(endpoint, { endpoint = it }, label = { Text(stringResource(R.string.models_base_url)) },
            placeholder = { Text("https://api.example.com/v1") }, singleLine = true, enabled = !saving,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrect = false, imeAction = ImeAction.Next),
            keyboardActions = KeyboardActions(onNext = { focusManager.moveFocus(FocusDirection.Next) }),
            modifier = Modifier.fillMaxWidth().testTag("model.setup.endpoint"))
        OutlinedTextField(key, { key = it }, label = { Text(stringResource(R.string.models_api_key)) },
            visualTransformation = PasswordVisualTransformation(), singleLine = true, enabled = !saving,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrect = false, imeAction = ImeAction.Next),
            keyboardActions = KeyboardActions(onNext = { focusManager.moveFocus(FocusDirection.Next) }),
            modifier = Modifier.fillMaxWidth().testTag("model.setup.key"))
        OutlinedTextField(model, { model = it }, label = { Text(stringResource(R.string.models_default_model_id)) },
            singleLine = true, enabled = !saving,
            keyboardOptions = KeyboardOptions(autoCorrect = false, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { focusManager.clearFocus(); keyboard?.hide() }),
            modifier = Modifier.fillMaxWidth().testTag("model.setup.modelID"))
        ModelWireProtocol.entries.forEach { value ->
            NolaRadioOption(
                label = stringResource(
                    if (value == ModelWireProtocol.Responses) R.string.models_protocol_responses
                    else R.string.models_protocol_chat_completions
                ),
                selected = protocol == value,
                enabled = !saving
            ) { protocol = value }
        }
        Text(stringResource(R.string.models_manual_setup_summary), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Button(onClick = {
            focusManager.clearFocus(); keyboard?.hide()
            onSave(endpoint.trim(), key, model.trim(), protocol)
        },
            enabled = !saving && RemoteModelEndpointURL.isValid(endpoint) && model.isNotBlank(),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("model.setup.save")) {
            if (saving) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            else Text(stringResource(R.string.models_save_and_use))
        }
    }
}

@Composable
internal fun HostPairingSheet(
    onDismiss: () -> Unit,
    onVerified: (String) -> Unit = { onDismiss() },
    viewModel: ModelsViewModel = hiltViewModel()
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    WebHostConfigSheet(
        currentUrl = state.webHostUrl,
        currentToken = state.webHostToken,
        currentCertificateSha256 = state.webHostCertificateSha256,
        onSave = viewModel::saveWebHostConfig,
        onCancelConnection = viewModel::cancelWebHostConnection,
        onVerified = onVerified,
        onDismiss = onDismiss
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WebHostConfigSheet(
    currentUrl: String,
    currentToken: String,
    currentCertificateSha256: String,
    onSave: (String, String, String, String?, (Boolean, String) -> Unit) -> Unit,
    onDismiss: () -> Unit,
    onCancelConnection: () -> Unit = {},
    onVerified: (String) -> Unit = { onDismiss() }
) {
    var url by remember { mutableStateOf(currentUrl) }
    var token by remember { mutableStateOf("") }
    var certificateSha256 by remember { mutableStateOf(currentCertificateSha256) }
    var showDiscardDialog by remember { mutableStateOf(false) }
    var isConnecting by remember { mutableStateOf(false) }
    var connectionGeneration by remember { mutableIntStateOf(0) }
    var inlineError by remember { mutableStateOf<String?>(null) }
    var importedSetup by remember { mutableStateOf<WebHostConnectionSetup?>(null) }
    val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
    val context = androidx.compose.ui.platform.LocalContext.current
    val showCertificate = certificateSha256.isNotBlank() || Uri.parse(url.trim()).scheme.equals("https", ignoreCase = true)
    val dirty = url != currentUrl || token.isNotBlank() || certificateSha256 != currentCertificateSha256
    val canUseSavedToken = currentToken.isNotBlank() && sameWebHostDestination(currentUrl, url)
    val sheetState = com.nola.ui.components.rememberGuardedEditorSheetState(isConnecting, dirty) { showDiscardDialog = true }
    val hasExistingConnection = currentUrl.isNotBlank() && currentToken.isNotBlank()
    val requestDismiss = { if (dirty) showDiscardDialog = true else onDismiss() }

    ModalBottomSheet(
        modifier = Modifier.dismissImeOnBlankTap(),
        onDismissRequest = { if (!isConnecting) requestDismiss() },
        sheetState = sheetState
    ) {
        val focus = LocalFocusManager.current
        val keyboard = LocalSoftwareKeyboardController.current
        val canSubmit = !isConnecting && url.isNotBlank() && (token.isNotBlank() || canUseSavedToken)
        fun submit() {
            if (isConnecting || !canSubmit) return
            focus.clearFocus()
            keyboard?.hide()
            inlineError = null
            isConnecting = true
            val generation = ++connectionGeneration
            val expectedIdentity = importedSetup?.takeIf { sameWebHostOrigin(it.origin, url) }?.hostId
            onSave(url.trim(), webHostConnectionToken(currentUrl, currentToken, url, token), certificateSha256.trim(), expectedIdentity) { succeeded, message ->
                if (generation == connectionGeneration) {
                    isConnecting = false
                    if (succeeded) onVerified(url.trim()) else inlineError = message
                }
            }
        }
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .fillMaxHeight(0.92f)
                .imePadding()
                .navigationBarsPadding()
                .padding(horizontal = 24.dp)
        ) {
            Spacer(Modifier.height(20.dp))
            Text(stringResource(R.string.models_web_host), Modifier.testTag("models.host.heading"), style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.height(16.dp))

            Column(
                modifier = Modifier
                    .weight(1f)
                    .verticalScroll(rememberScrollState()).testTag("models.host.fields"),
                verticalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                if (!hasExistingConnection) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(NolaRadius.lg))
                            .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.07f))
                            .padding(NolaSpacing.md),
                        verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
                    ) {
                        Text(
                            stringResource(R.string.models_web_host_setup_title),
                            style = MaterialTheme.typography.titleSmall
                        )
                        Text(
                            stringResource(R.string.models_web_host_setup_start),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Text(
                            stringResource(R.string.models_web_host_setup_copy),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }

                TextButton(enabled = !isConnecting, onClick = {
                    val setup = clipboard.getText()?.text?.let { WebHostConnectionSetup.decode(it) }
                    if (setup == null) inlineError = context.getString(R.string.models_web_host_invalid_setup)
                    else {
                        importedSetup = setup
                        url = setup.origin
                        token = setup.token
                        certificateSha256 = ""
                        inlineError = null
                    }
                }) { Text(stringResource(R.string.models_web_host_paste_setup)) }

                OutlinedTextField(
                    value = url,
                    enabled = !isConnecting,
                    onValueChange = {
                        if (!sameWebHostDestination(url, it)) {
                            token = ""
                            certificateSha256 = ""
                            importedSetup = null
                        }
                        url = it
                        inlineError = null
                    },
                    label = { Text(stringResource(R.string.models_host_url)) },
                    placeholder = { Text(stringResource(R.string.models_web_host_url_placeholder)) },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrect = false, imeAction = ImeAction.Next),
                    keyboardActions = KeyboardActions(onNext = { focus.moveFocus(FocusDirection.Next) }),
                    modifier = Modifier.fillMaxWidth().testTag("models.host.url"),
                    singleLine = true
                )
                OutlinedTextField(
                    value = token,
                    enabled = !isConnecting,
                    onValueChange = { token = it; inlineError = null },
                    label = { Text(stringResource(R.string.models_access_token)) },
                    supportingText = if (token.isBlank()) {
                        { Text(stringResource(if (canUseSavedToken) R.string.models_host_token_keep else R.string.models_host_token_new)) }
                    } else null,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrect = false,
                        imeAction = if (showCertificate) ImeAction.Next else ImeAction.Done),
                    keyboardActions = KeyboardActions(onNext = { focus.moveFocus(FocusDirection.Next) }, onDone = { submit() }),
                    modifier = Modifier.fillMaxWidth().testTag("models.host.token"),
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation()
                )
                if (showCertificate) {
                    OutlinedTextField(
                        value = certificateSha256,
                        enabled = !isConnecting,
                        onValueChange = { certificateSha256 = it; inlineError = null },
                        label = { Text(stringResource(R.string.models_certificate_sha256_optional)) },
                        supportingText = { Text(stringResource(R.string.models_certificate_sha256_help)) },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, autoCorrect = false, imeAction = ImeAction.Done),
                        keyboardActions = KeyboardActions(onDone = { submit() }),
                        modifier = Modifier.fillMaxWidth().testTag("models.host.certificateSha256"),
                        singleLine = true
                    )
                }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(NolaRadius.lg))
                        .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.06f))
                        .padding(NolaSpacing.md),
                    horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm),
                    verticalAlignment = Alignment.Top
                ) {
                    Icon(
                        Icons.Filled.Security,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(20.dp)
                    )
                    Text(
                        stringResource(R.string.models_web_host_trust_explanation),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f)
                    )
                }
                Spacer(Modifier.height(6.dp))
            }

            val reduceMotion = rememberReduceMotionEnabled()
            AnimatedVisibility(visible = inlineError != null, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
                inlineError?.let { message ->
                    Text(
                        message,
                        modifier = Modifier.fillMaxWidth().heightIn(max = 96.dp).verticalScroll(rememberScrollState())
                            .semantics { liveRegion = androidx.compose.ui.semantics.LiveRegionMode.Polite }
                            .padding(vertical = NolaSpacing.sm).testTag("models.host.feedback"),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }

            val actionRes = when {
                isConnecting -> R.string.models_web_host_checking
                !dirty && canUseSavedToken -> R.string.models_verify_again
                else -> R.string.models_save_and_verify
            }
            val onCancelClick: () -> Unit = {
                if (isConnecting) {
                    connectionGeneration++
                    onCancelConnection()
                    isConnecting = false
                } else requestDismiss()
            }
            BoxWithConstraints(modifier = Modifier.fillMaxWidth()) {
                if (maxWidth < 340.dp) {
                    Column(
                        modifier = Modifier.fillMaxWidth(),
                        verticalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Button(
                            onClick = ::submit,
                            enabled = canSubmit,
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            if (isConnecting) {
                                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                                Spacer(Modifier.width(6.dp))
                            }
                            Text(stringResource(actionRes), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                        TextButton(
                            onClick = onCancelClick,
                            modifier = Modifier.fillMaxWidth()
                        ) { Text(stringResource(R.string.common_cancel), maxLines = 1, overflow = TextOverflow.Ellipsis) }
                    }
                } else {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        TextButton(
                            onClick = onCancelClick,
                            modifier = Modifier.weight(1f)
                        ) { Text(stringResource(R.string.common_cancel), maxLines = 1, overflow = TextOverflow.Ellipsis) }
                        Button(
                            onClick = ::submit,
                            enabled = canSubmit,
                            modifier = Modifier.weight(1f)
                        ) {
                            if (isConnecting) {
                                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                                Spacer(Modifier.width(6.dp))
                            }
                            Text(stringResource(actionRes), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        }
                    }
                }
            }
            Spacer(Modifier.height(12.dp))
        }
    }
    if (showDiscardDialog) {
        DiscardChangesDialog(
            onDiscard = onDismiss,
            onKeepEditing = { showDiscardDialog = false }
        )
    }
}
