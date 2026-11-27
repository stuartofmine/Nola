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
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
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
import com.nola.llm.local.DeviceModelCatalog
import com.nola.llm.local.DeviceModelCatalogEntry
import com.nola.llm.local.DeviceModelFit
import com.nola.llm.local.LocalModelDownloadSource
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


        val tabHaptics = rememberNolaHaptics()
        val isDark = NolaColors.isDark
        Row(
            Modifier.fillMaxWidth().selectableGroup()
                .background(NolaColors.segmentTrack, RoundedCornerShape(10.dp))
                .then(if (isDark) Modifier.border(0.75.dp, NolaColors.hairlineBorder, RoundedCornerShape(10.dp)) else Modifier)
                .padding(3.dp),
            horizontalArrangement = Arrangement.spacedBy(2.dp)
        ) {
            listOf(R.string.models_service_tab, R.string.models_local_models, R.string.models_web_host).forEachIndexed { index, title ->
                val selected = sourceTab == index
                val tabBg by animateColorAsState(
                    targetValue = if (selected) NolaColors.segmentSelected else Color.Transparent,
                    animationSpec = com.nola.ui.components.nolaTween(NolaMotion.fast, reduceMotion),
                    label = "models-source-tab-bg",
                )
                val tabFg by animateColorAsState(
                    targetValue = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                    animationSpec = com.nola.ui.components.nolaTween(NolaMotion.fast, reduceMotion),
                    label = "models-source-tab-fg",
                )
                Box(
                    modifier = Modifier.weight(1f).heightIn(min = 48.dp).clip(RoundedCornerShape(8.dp))
                        .background(tabBg)
                        .selectable(selected = selected, role = Role.Tab, onClick = {
                            if (!selected) {
                                tabHaptics.selection()
                                viewModel.clearConnectionStatus()
                            }
                            sourceTab = index
                        })
                        .padding(horizontal = 6.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(stringResource(title), style = MaterialTheme.typography.labelLarge,
                        color = tabFg,
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

        AnimatedContent(
            targetState = sourceTab,
            transitionSpec = {
                fadeIn(com.nola.ui.components.nolaTween(NolaMotion.controlState, reduceMotion)) togetherWith
                    fadeOut(com.nola.ui.components.nolaTween(NolaMotion.fast, reduceMotion))
            },
            label = "models-source-tab-body",
        ) { activeTab ->
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(NolaSpacing.xl),
            ) {
        if (activeTab == 0) {
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

            val hasConfiguredApiModels = state.officialModels.isNotEmpty() || state.configuredApiModels.isNotEmpty()
            var showAddApiForm by rememberSaveable(hasConfiguredApiModels) { mutableStateOf(!hasConfiguredApiModels) }
            NolaCard {
                if (hasConfiguredApiModels) {
                    NolaDisclosureBar(
                        title = stringResource(R.string.models_add_api_model),
                        expanded = showAddApiForm,
                        onToggle = { showAddApiForm = !showAddApiForm },
                        modifier = Modifier.testTag(MODEL_API_CONNECTION_TEST_TAG),
                    )
                    AnimatedVisibility(
                        visible = showAddApiForm,
                        enter = nolaDisclosureEnter(reduceMotion),
                        exit = nolaDisclosureExit(reduceMotion),
                    ) {
                        EndpointConfigFields(
                            initialEndpoint = "", initialApiKey = "", initialModel = "",
                            initialWireProtocol = ModelWireProtocol.ChatCompletions,
                            saving = state.savingEndpoint, error = state.endpointSaveError, draft = viewModel.connectionDraft,
                            onSave = { endpoint, key, model, protocol -> viewModel.saveApiEndpoint(endpoint, key, model, protocol) }
                        )
                    }
                } else {
                    NolaSectionTitle(icon = Icons.Filled.Add, title = stringResource(R.string.models_add_api_model))
                    EndpointConfigFields(
                        initialEndpoint = "", initialApiKey = "", initialModel = "",
                        initialWireProtocol = ModelWireProtocol.ChatCompletions,
                        saving = state.savingEndpoint, error = state.endpointSaveError, draft = viewModel.connectionDraft,
                        onSave = { endpoint, key, model, protocol -> viewModel.saveApiEndpoint(endpoint, key, model, protocol) }
                    )
                }
            }

            ModelConnectionsSection(state, viewModel::loadConnectionInventory, connectHost = {
                sourceTab = 2
                showWebHostSheet = true
            }, manageHost = manageHostModels)
        }
        if (activeTab == 1) {
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

            val deviceRamBytes = remember(context) { DeviceModelCatalog.deviceTotalRamBytes(context) }
            NolaCard {
                NolaSectionTitle(icon = Icons.Filled.Download, title = stringResource(R.string.models_download_models))
                Row(
                    Modifier.fillMaxWidth().selectableGroup()
                        .background(NolaColors.segmentTrack, RoundedCornerShape(10.dp))
                        .then(if (isDark) Modifier.border(0.75.dp, NolaColors.hairlineBorder, RoundedCornerShape(10.dp)) else Modifier)
                        .padding(3.dp),
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    LocalModelDownloadSource.entries.forEach { source ->
                        val selected = state.downloadSource == source
                        Box(
                            modifier = Modifier.weight(1f).heightIn(min = 40.dp).clip(RoundedCornerShape(8.dp))
                                .background(if (selected) NolaColors.segmentSelected else Color.Transparent)
                                .selectable(selected = selected, role = Role.Tab, onClick = {
                                    if (!selected) tabHaptics.selection()
                                    viewModel.setDownloadSource(source)
                                })
                                .padding(horizontal = 8.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                source.label,
                                style = MaterialTheme.typography.labelMedium,
                                color = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                                textAlign = TextAlign.Center,
                            )
                        }
                    }
                }
                state.deviceCatalog.forEach { entry ->
                    DeviceModelCatalogRow(
                        entry = entry,
                        deviceRamBytes = deviceRamBytes,
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
        if (activeTab == 2) {
            ModelActionTile(title = stringResource(R.string.models_web_host),
                modifier = Modifier.testTag(MODEL_HOST_CONNECTION_TEST_TAG),
                subtitle = state.webHostUrl.ifBlank { stringResource(R.string.models_desktop_bridge) },
                icon = Icons.Filled.Security, tint = MaterialTheme.colorScheme.primary,
                onClick = { showWebHostSheet = true })
            state.verifiedHostTransport?.let { route ->
                Text(
                    stringResource(if (route == com.nola.data.remote.dto.VerifiedHostTransport.Tailcat)
                        R.string.models_verified_via_tailcat else R.string.models_verified_via_direct),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("models.host.verified-transport"),
                )
            }
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
                HorizontalDivider(color = NolaColors.hairlineBorder)
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
        onDismissRequest = requestDismiss,
        contentWindowInsets = { androidx.compose.foundation.layout.WindowInsets(0) },
        sheetState = com.nola.ui.components.rememberGuardedEditorSheetState(saving, dirty) { showDiscardDialog = true }) {
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
                ChatOutputTokenField.MaxTokens to stringResource(R.string.models_execution_output_standard),
                ChatOutputTokenField.MaxCompletionTokens to stringResource(R.string.models_execution_output_completion),
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
    deviceRamBytes: Long,
    installed: Boolean,
    downloading: Boolean,
    progress: Float?,
    downloadsBlocked: Boolean,
    onDownload: () -> Unit,
    onCancelDownload: () -> Unit,
) {
    val fit = entry.fitForRam(deviceRamBytes)
    val isZh = java.util.Locale.getDefault().language.equals("zh", ignoreCase = true)
    val fitLabel = when (fit) {
        DeviceModelFit.Best -> if (isZh) "最适合本机" else "Best fit"
        DeviceModelFit.Light -> if (isZh) "轻量流畅" else "Fast & light"
        DeviceModelFit.Large -> if (isZh) "大参数" else "Large"
    }
    val fitTint = when (fit) {
        DeviceModelFit.Best -> MaterialTheme.colorScheme.primary
        DeviceModelFit.Light -> NolaColors.success
        DeviceModelFit.Large -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text(entry.displayName, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                NolaBadge(text = fitLabel, tint = fitTint)
            }
            Text(
                "${DeviceModelCatalog.formatGb(entry.byteSize)} · ${entry.quantization} · ${entry.contextWindow / 1024}K",
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
    val reduceMotion = rememberReduceMotionEnabled()
    val rowBackground by animateColorAsState(
        targetValue = if (isSelected) {
            MaterialTheme.colorScheme.primary.copy(alpha = 0.07f)
        } else {
            Color.Transparent
        },
        animationSpec = com.nola.ui.components.nolaTween(NolaMotion.base, reduceMotion),
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
                .clickable(enabled = !isAnyActivating) { onClick() }
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
    var overflowExpanded by remember(model.id) { mutableStateOf(false) }
    val hasOverflowActions = model.source != ModelInfo.ModelSource.WebHost && (
        true || onTest != null || onCapabilities != null || onDeleteLocal != null
    )
    FlowRow(
        modifier = Modifier
            .testTag(MODEL_ROW_ACTIONS_TEST_TAG_PREFIX + model.id)
            .widthIn(min = 120.dp),
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
        val actionState = when {
            isActivating -> 0
            isSelected -> 1
            else -> 2
        }
        val reduceMotion = rememberReduceMotionEnabled()
        AnimatedContent(
            targetState = actionState,
            transitionSpec = {
                fadeIn(com.nola.ui.components.nolaTween(NolaMotion.base, reduceMotion)) togetherWith
                    fadeOut(com.nola.ui.components.nolaTween(NolaMotion.fast, reduceMotion))
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
        if (hasOverflowActions) {
            Box {
                IconButton(
                    onClick = { overflowExpanded = true },
                    enabled = !isAnyActivating,
                    modifier = Modifier.size(48.dp)
                ) {
                    Icon(
                        Icons.Filled.MoreVert,
                        contentDescription = stringResource(R.string.intelligence_more),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                DropdownMenu(
                    expanded = overflowExpanded,
                    onDismissRequest = { overflowExpanded = false }
                ) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.common_parameters)) },
                        leadingIcon = {
                            Icon(
                                painterResource(R.drawable.nola_settings),
                                contentDescription = null,
                                modifier = Modifier.size(18.dp)
                            )
                        },
                        onClick = {
                            overflowExpanded = false
                            onParams()
                        }
                    )
                    if (onTest != null) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.models_inference_test)) },
                            leadingIcon = {
                                Icon(
                                    painterResource(R.drawable.nola_execution),
                                    contentDescription = null,
                                    modifier = Modifier.size(18.dp)
                                )
                            },
                            onClick = {
                                overflowExpanded = false
                                onTest()
                            }
                        )
                    }
                    if (onCapabilities != null) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.models_execution_capabilities)) },
                            leadingIcon = {
                                Icon(
                                    Icons.Filled.Security,
                                    contentDescription = null,
                                    modifier = Modifier.size(18.dp)
                                )
                            },
                            onClick = {
                                overflowExpanded = false
                                onCapabilities()
                            }
                        )
                    }
                    if (onDeleteLocal != null) {
                        DropdownMenuItem(
                            text = {
                                Text(
                                    stringResource(R.string.common_delete),
                                    color = if (!isRunning) MaterialTheme.colorScheme.error
                                    else MaterialTheme.colorScheme.error.copy(alpha = 0.38f)
                                )
                            },
                            leadingIcon = {
                                Icon(
                                    Icons.Filled.Delete,
                                    contentDescription = null,
                                    tint = if (!isRunning) MaterialTheme.colorScheme.error
                                    else MaterialTheme.colorScheme.error.copy(alpha = 0.38f),
                                    modifier = Modifier.size(18.dp)
                                )
                            },
                            enabled = !isRunning,
                            onClick = {
                                overflowExpanded = false
                                haptics.warning()
                                onDeleteLocal()
                            }
                        )
                    }
                }
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
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary
            )
        ) {
            if (testing) {
                CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.onPrimary
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
    NolaCard(modifier = modifier.clickable(enabled = enabled, onClick = onClick)) {
        Row(
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
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
            Icon(
                painterResource(R.drawable.nola_chevron),
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant
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

private data class ApiProviderPreset(
    val id: String,
    val label: String?,
    val baseUrl: String,
    val defaultModel: String,
    val protocol: ModelWireProtocol,
)

private val API_PROVIDER_PRESETS = listOf(
    ApiProviderPreset("openai", "OpenAI", "https://api.openai.com/v1", "gpt-4o-mini", ModelWireProtocol.Responses),
    ApiProviderPreset("anthropic", "Anthropic", "https://api.anthropic.com/v1", "claude-sonnet-4-20250514", ModelWireProtocol.ChatCompletions),
    ApiProviderPreset("deepseek", "DeepSeek", "https://api.deepseek.com/v1", "deepseek-chat", ModelWireProtocol.ChatCompletions),
    ApiProviderPreset("openrouter", "OpenRouter", "https://openrouter.ai/api/v1", "openai/gpt-4o-mini", ModelWireProtocol.ChatCompletions),
    ApiProviderPreset("ollama", "Ollama", "http://127.0.0.1:11434/v1", "llama3.2", ModelWireProtocol.ChatCompletions),
    ApiProviderPreset("custom", null, "", "", ModelWireProtocol.ChatCompletions),
)

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
    val reduceMotion = rememberReduceMotionEnabled()
    var endpoint by draft::endpoint
    var key by draft::key
    var model by draft::model
    var protocol by draft::protocol
    var showApiKey by remember { mutableStateOf(false) }
    var showAdvanced by remember { mutableStateOf(false) }
    val endpointInvalid = endpoint.isNotBlank() && !RemoteModelEndpointURL.isValid(endpoint)
    val selectedPresetId = remember(endpoint) {
        val trimmed = endpoint.trim().trimEnd('/')
        API_PROVIDER_PRESETS.firstOrNull { preset ->
            preset.baseUrl.isNotEmpty() && trimmed.equals(preset.baseUrl.trimEnd('/'), ignoreCase = true)
        }?.id ?: "custom"
    }
    LaunchedEffect(endpoint, key, model, protocol) {
        onDirtyChange(endpoint != initialEndpoint || key != initialApiKey || model != initialModel || protocol != initialWireProtocol)
    }
    Column(Modifier.widthIn(max = 560.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Text(
            stringResource(R.string.models_provider_presets),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(
            modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            API_PROVIDER_PRESETS.forEach { preset ->
                val label = preset.label ?: stringResource(R.string.models_provider_custom)
                FilterChip(
                    selected = selectedPresetId == preset.id,
                    enabled = !saving,
                    onClick = {
                        if (preset.id != "custom") {
                            endpoint = preset.baseUrl
                            if (model.isBlank() || API_PROVIDER_PRESETS.any { it.defaultModel == model }) {
                                model = preset.defaultModel
                            }
                            protocol = preset.protocol
                        } else {
                            endpoint = ""
                        }
                    },
                    label = { Text(label) },
                )
            }
        }
        OutlinedTextField(
            endpoint,
            { endpoint = it },
            label = { Text(stringResource(R.string.models_base_url)) },
            placeholder = { Text("https://api.example.com/v1") },
            singleLine = true,
            enabled = !saving,
            isError = endpointInvalid,
            supportingText = if (endpointInvalid) {
                { Text(stringResource(R.string.models_endpoint_invalid_url)) }
            } else null,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrect = false, imeAction = ImeAction.Next),
            keyboardActions = KeyboardActions(onNext = { focusManager.moveFocus(FocusDirection.Next) }),
            modifier = Modifier.fillMaxWidth().testTag("model.setup.endpoint")
        )
        OutlinedTextField(
            key,
            { key = it },
            label = { Text(stringResource(R.string.models_api_key)) },
            visualTransformation = if (showApiKey) VisualTransformation.None else PasswordVisualTransformation(),
            trailingIcon = {
                IconButton(onClick = { showApiKey = !showApiKey }, enabled = !saving) {
                    Icon(
                        imageVector = if (showApiKey) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                        contentDescription = stringResource(if (showApiKey) R.string.models_hide_secret else R.string.models_show_secret),
                    )
                }
            },
            singleLine = true,
            enabled = !saving,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrect = false, imeAction = ImeAction.Next),
            keyboardActions = KeyboardActions(onNext = { focusManager.moveFocus(FocusDirection.Next) }),
            modifier = Modifier.fillMaxWidth().testTag("model.setup.key")
        )
        OutlinedTextField(
            model,
            { model = it },
            label = { Text(stringResource(R.string.models_default_model_id)) },
            singleLine = true,
            enabled = !saving,
            keyboardOptions = KeyboardOptions(autoCorrect = false, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { focusManager.clearFocus(); keyboard?.hide() }),
            modifier = Modifier.fillMaxWidth().testTag("model.setup.modelID")
        )
        NolaDisclosureBar(
            title = stringResource(R.string.models_advanced_settings),
            subtitle = stringResource(
                if (protocol == ModelWireProtocol.Responses) R.string.models_protocol_responses
                else R.string.models_protocol_chat_completions
            ),
            expanded = showAdvanced,
            onToggle = { showAdvanced = !showAdvanced },
            enabled = !saving,
            modifier = Modifier.testTag("model.setup.advanced"),
        )
        AnimatedVisibility(
            visible = showAdvanced,
            enter = nolaDisclosureEnter(reduceMotion),
            exit = nolaDisclosureExit(reduceMotion),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
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
            }
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
        contentWindowInsets = { androidx.compose.foundation.layout.WindowInsets(0) },
        sheetState = sheetState
    ) {
        val focus = LocalFocusManager.current
        val keyboard = LocalSoftwareKeyboardController.current
        val tokenFocus = remember { FocusRequester() }
        val pinFocus = remember { FocusRequester() }
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
                .heightIn(max = androidx.compose.ui.platform.LocalConfiguration.current.screenHeightDp.dp * 0.92f)
                .imePadding()
                .navigationBarsPadding()
                .padding(horizontal = 24.dp)
        ) {
            Spacer(Modifier.height(20.dp))
            Text(stringResource(R.string.models_web_host), Modifier.testTag("models.host.heading"), style = MaterialTheme.typography.titleLarge)
            Spacer(Modifier.height(16.dp))

            Column(
                modifier = Modifier
                    .weight(1f, fill = false)
                    .verticalScroll(rememberScrollState()).testTag("models.host.fields"),
                verticalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                OutlinedButton(modifier = Modifier.fillMaxWidth(), enabled = !isConnecting, onClick = {
                    val setup = clipboard.getText()?.text?.let { WebHostConnectionSetup.decode(it) }
                    if (setup == null) inlineError = context.getString(R.string.models_web_host_invalid_setup)
                    else {
                        focus.clearFocus()
                        keyboard?.hide()
                        importedSetup = setup
                        url = setup.origin
                        token = setup.token
                        certificateSha256 = ""
                        inlineError = null
                    }
                }) { Text(stringResource(R.string.models_web_host_paste_setup)) }

                if (!hasExistingConnection) {
                    var showSetupHelp by rememberSaveable { mutableStateOf(false) }
                    NolaDisclosureBar(
                        title = stringResource(R.string.models_web_host_setup_title),
                        expanded = showSetupHelp,
                        onToggle = { showSetupHelp = !showSetupHelp },
                    )
                    if (showSetupHelp) {
                        Column(verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)) {
                            Text(stringResource(R.string.models_web_host_setup_start))
                            Text(stringResource(R.string.models_web_host_setup_copy))
                        }
                    }
                }
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
                    keyboardActions = KeyboardActions(onNext = { tokenFocus.requestFocus() }),
                    modifier = Modifier.fillMaxWidth().testTag("models.host.url"),
                    singleLine = true
                )
                var showToken by remember { mutableStateOf(false) }
                var showAdvancedTls by remember { mutableStateOf(certificateSha256.isNotBlank()) }
                val tlsReduceMotion = rememberReduceMotionEnabled()
                OutlinedTextField(
                    value = token,
                    enabled = !isConnecting,
                    onValueChange = { token = it; inlineError = null },
                    label = { Text(stringResource(R.string.models_access_token)) },
                    supportingText = if (token.isBlank()) {
                        { Text(stringResource(if (canUseSavedToken) R.string.models_host_token_keep else R.string.models_host_token_new)) }
                    } else null,
                    trailingIcon = {
                        IconButton(onClick = { showToken = !showToken }, enabled = !isConnecting) {
                            Icon(
                                imageVector = if (showToken) Icons.Filled.VisibilityOff else Icons.Filled.Visibility,
                                contentDescription = stringResource(if (showToken) R.string.models_hide_secret else R.string.models_show_secret),
                            )
                        }
                    },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrect = false,
                        imeAction = if (showCertificate && showAdvancedTls) ImeAction.Next else ImeAction.Done),
                    keyboardActions = KeyboardActions(onNext = { pinFocus.requestFocus() }, onDone = { submit() }),
                    modifier = Modifier.fillMaxWidth().focusRequester(tokenFocus).testTag("models.host.token"),
                    singleLine = true,
                    visualTransformation = if (showToken) VisualTransformation.None else PasswordVisualTransformation()
                )
                if (showCertificate) {
                    NolaDisclosureBar(
                        title = stringResource(R.string.models_advanced_tls),
                        expanded = showAdvancedTls,
                        onToggle = { showAdvancedTls = !showAdvancedTls },
                        modifier = Modifier.testTag("models.host.advancedTls"),
                        enabled = !isConnecting,
                    )
                    AnimatedVisibility(
                        visible = showAdvancedTls,
                        enter = nolaDisclosureEnter(tlsReduceMotion),
                        exit = nolaDisclosureExit(tlsReduceMotion),
                    ) {
                        OutlinedTextField(
                            value = certificateSha256,
                            enabled = !isConnecting,
                            onValueChange = {
                                certificateSha256 = it
                                inlineError = null
                                if (it.isBlank() && !Uri.parse(url.trim()).scheme.equals("https", ignoreCase = true)) {
                                    focus.clearFocus()
                                    keyboard?.hide()
                                }
                            },
                            label = { Text(stringResource(R.string.models_certificate_sha256_optional)) },
                            supportingText = { Text(stringResource(R.string.models_certificate_sha256_help)) },
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, autoCorrect = false, imeAction = ImeAction.Done),
                            keyboardActions = KeyboardActions(onDone = { submit() }),
                            modifier = Modifier.fillMaxWidth().focusRequester(pinFocus).testTag("models.host.certificateSha256"),
                            singleLine = true
                        )
                    }
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
            val cancelLabel = stringResource(if (isConnecting) R.string.models_cancel_host_verification else R.string.common_cancel)
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
                            Text(stringResource(actionRes), textAlign = TextAlign.Center)
                        }
                        TextButton(
                            onClick = onCancelClick,
                            modifier = Modifier.fillMaxWidth()
                        ) { Text(cancelLabel, textAlign = TextAlign.Center) }
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
                        ) { Text(cancelLabel, textAlign = TextAlign.Center) }
                        Button(
                            onClick = ::submit,
                            enabled = canSubmit,
                            modifier = Modifier.weight(1f)
                        ) {
                            if (isConnecting) {
                                CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                                Spacer(Modifier.width(6.dp))
                            }
                            Text(stringResource(actionRes), textAlign = TextAlign.Center)
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
