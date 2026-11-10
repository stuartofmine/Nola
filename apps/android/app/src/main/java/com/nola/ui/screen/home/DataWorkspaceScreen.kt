package com.nola.ui.screen.home

import android.text.format.DateUtils
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Lightbulb
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Psychology
import androidx.compose.material.icons.filled.CalendarMonth
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nola.ui.components.nolaPageGutter
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.health.AndroidHealthPermissionState
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceAccessPolicy
import com.nola.domain.model.DataSourceUsageRecord
import com.nola.domain.model.DATA_SOURCE_MAXIMUM_AGE_MINUTES
import com.nola.domain.model.DATA_SOURCE_MINIMUM_AGE_MINUTES
import com.nola.domain.model.UnifiedDataSourceKind
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.rememberReduceMotionEnabled
import java.text.NumberFormat
import java.time.Instant

@Composable
fun DataWorkspaceScreen(onCreateInChat: () -> Unit = {}, initialPage: Int = 0, onBack: (() -> Unit)? = null,
    onOpenConversation: (String) -> Unit = {}, showSwitcher: Boolean = true,
    backToInformation: Boolean = false) {
    var page by rememberSaveable(initialPage) { mutableIntStateOf(initialPage) }
    val reduceMotion = rememberReduceMotionEnabled()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .statusBarsPadding()
            .testTag("data.workspace")
    ) {
        if (onBack != null) TextButton(onClick = onBack, modifier = Modifier.padding(horizontal = 12.dp).heightIn(min = 48.dp)) {
            Icon(androidx.compose.material.icons.Icons.AutoMirrored.Filled.ArrowBack, contentDescription = null)
            Text(stringResource(if (backToInformation) R.string.tab_information else R.string.common_back))
        }
        if (showSwitcher) WorkspaceSwitcher(
            selected = page,
            onSelected = { page = it },
            modifier = Modifier
                .widthIn(max = 840.dp)
                .fillMaxWidth()
                .align(Alignment.CenterHorizontally)
                .padding(horizontal = 16.dp, vertical = 10.dp)
        )
        AnimatedContent(
            targetState = page,
            transitionSpec = {
                fadeIn(tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)) togetherWith
                    fadeOut(tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing))
            },
            label = "data-workspace",
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
        ) { selected ->
            when (selected) {
                1 -> com.nola.ui.screen.intelligence.IntelligenceScreen(section = com.nola.ui.screen.intelligence.IntelligenceSection.MEMORY)
                2 -> com.nola.ui.screen.scheduled.ScheduledActionsScreen(onCreateInChat = onCreateInChat)
                3 -> com.nola.ui.screen.activity.ActivityScreen(onOpenConversation = onOpenConversation)
                4 -> ConversationKnowledgeScreen(onOpenConversation = onOpenConversation)
                else -> DataSourcesScreen()
            }
        }
    }
}

/** Mobile Information is a destination list; each category opens its existing real screen. */
@Composable
internal fun InformationOverview(onOpen: (Int) -> Unit) {
    val sections = listOf(4 to R.string.workspace_knowledge, 1 to R.string.workspace_memory,
        0 to R.string.workspace_sources)
    LazyColumn(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)
        .statusBarsPadding().testTag("information.overview"),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = nolaPageGutter(), vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            Text(stringResource(R.string.tab_information), style = MaterialTheme.typography.headlineMedium.copy(fontSize = 17.sp, lineHeight = 24.sp),
                modifier = Modifier.padding(bottom = 16.dp))
        }
        items(sections, key = { it.first }) { (page, label) ->
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp)
                .clickable(role = Role.Button) { onOpen(page) }
                .testTag("information.category.$page").background(MaterialTheme.colorScheme.surface, RoundedCornerShape(14.dp)).padding(horizontal = 14.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Box(Modifier.size(36.dp).background(MaterialTheme.colorScheme.surfaceContainer, RoundedCornerShape(8.dp)),
                    contentAlignment = Alignment.Center) {
                    Icon(painterResource(when (page) {
                        4 -> R.drawable.nola_file
                        0 -> R.drawable.nola_folder
                        1 -> R.drawable.nola_person
                        2 -> R.drawable.nola_calendar
                        else -> R.drawable.nola_clock
                    }), contentDescription = null, modifier = Modifier.size(20.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Text(stringResource(label), Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyMedium.copy(fontSize = 16.sp, lineHeight = 24.sp))
                Icon(painterResource(R.drawable.nola_chevron), contentDescription = null, modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
internal fun WorkspaceSwitcher(
    selected: Int,
    onSelected: (Int) -> Unit,
    modifier: Modifier = Modifier
) {
    val scrollState = rememberLazyListState()
    val sections = listOf(4 to R.string.workspace_knowledge, 0 to R.string.workspace_sources,
        1 to R.string.workspace_memory, 2 to R.string.workspace_plans, 3 to R.string.workspace_records)
    LaunchedEffect(selected) { scrollState.animateScrollToItem(sections.indexOfFirst { it.first == selected }.coerceAtLeast(0)) }
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(15.dp),
        color = Color.Transparent
    ) {
        LazyRow(state = scrollState, modifier = Modifier.padding(3.dp)) {
            itemsIndexed(sections) { _, (page, label) ->
                WorkspaceSegment(title = stringResource(label), testTag = "workspace.section.$page",
                    selected = selected == page, onClick = { onSelected(page) })
            }
        }
    }
}

@Composable
internal fun WorkspaceSegment(
    title: String,
    testTag: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    val reduceMotion = rememberReduceMotionEnabled()
    val selectionBackground by animateColorAsState(
        targetValue = if (selected) MaterialTheme.colorScheme.primaryContainer else Color.Transparent,
        animationSpec = tween(if (reduceMotion) 0 else NolaMotion.fast),
        label = "workspace-segment-background"
    )
    val contentColor by animateColorAsState(
        targetValue = if (selected) {
            MaterialTheme.colorScheme.onSurface
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant
        },
        animationSpec = tween(if (reduceMotion) 0 else NolaMotion.fast, easing = NolaMotion.emphasizedEasing),
        label = "workspace-segment-content"
    )
    Surface(
        modifier = modifier
            .testTag(testTag)
            .heightIn(min = 48.dp)
            .clip(RoundedCornerShape(12.dp))
            .selectable(
                selected = selected,
                onClick = onClick,
                role = Role.Tab
            ),
        shape = RoundedCornerShape(12.dp),
        color = selectionBackground
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(
                text = title,
                maxLines = 1,
                modifier = Modifier.width(IntrinsicSize.Max).padding(horizontal = 12.dp, vertical = 8.dp),
                style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold,
                color = contentColor
            )
        }
    }
}

@Composable
private fun DataSourcesScreen(viewModel: DataSourcesViewModel = hiltViewModel()) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val permissionLauncher = rememberLauncherForActivityResult(AndroidHealthDataSource.permissionContract()) {
        viewModel.refresh()
    }
    DataSourcesContent(
        state = state,
        onRefresh = viewModel::refresh,
        onRequestHealthPermission = {
            permissionLauncher.launch(AndroidHealthDataSource.readPermissions)
        },
        onValidateAndSave = viewModel::validateAndSave,
        onDelete = viewModel::delete,
        onSetEnabled = viewModel::setEnabled,
        onClearError = viewModel::clearOperationError
    )
}

@Composable
internal fun DataSourcesContent(
    state: DataSourcesUiState,
    onRefresh: () -> Unit,
    onRequestHealthPermission: () -> Unit,
    onValidateAndSave: (CustomDataSourceConfig, (Boolean) -> Unit) -> Unit = { _, completion -> completion(false) },
    onDelete: (String) -> Unit = {},
    onSetEnabled: (String, Boolean) -> Unit = { _, _ -> },
    onClearError: () -> Unit = {}
) {
    var editingSource by remember { mutableStateOf<CustomDataSourceConfig?>(null) }
    var showEditor by remember { mutableStateOf(false) }
    var showValidationSuccess by remember { mutableStateOf(false) }
    val healthState = remember(state.health, state.isRefreshing) { healthVisualState(state) }
    val readyCount = listOf(
        healthState == SourceVisualState.Ready || healthState == SourceVisualState.Partial,
        state.enabledDocumentCount > 0,
        state.enabledToolCount > 0 || state.enabledSkillCount > 0
    ).count { it }
    val attentionCount = if (healthState in setOf(SourceVisualState.NeedsPermission, SourceVisualState.Error, SourceVisualState.Unavailable)) 1 else 0

    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.TopCenter
    ) {
        LazyColumn(
            modifier = Modifier
                .fillMaxWidth()
                .widthIn(max = 840.dp)
                .testTag("data.sources"),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(start = 18.dp, end = 18.dp, top = 6.dp, bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    text = stringResource(R.string.data_sources_screen_title),
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = stringResource(R.string.data_sources_screen_subtitle),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    TextButton(onClick = onRefresh, enabled = !state.isRefreshing) {
                        if (state.isRefreshing) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                        } else {
                            Icon(Icons.Filled.Refresh, contentDescription = null, modifier = Modifier.size(18.dp))
                        }
                        Spacer(Modifier.size(7.dp))
                        Text(stringResource(R.string.data_sources_action_refresh))
                    }
                    Button(
                        onClick = {
                            editingSource = null
                            showEditor = true
                        },
                        modifier = Modifier.testTag("data.sources.add").heightIn(min = 48.dp),
                        shape = RoundedCornerShape(12.dp),
                        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.onSurface, contentColor = MaterialTheme.colorScheme.surface)
                    ) {
                        Icon(
                            Icons.Filled.Add,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp)
                        )
                        Spacer(Modifier.size(7.dp))
                        Text(stringResource(R.string.data_sources_action_add))
                    }
                }
            }
        }

        item {
            DataReadinessHero(
                readyCount = readyCount + state.customSources.count { it.enabled && it.isUsable },
                attentionCount = attentionCount + state.customSources.count { it.enabled && !it.isUsable },
                lastReadAt = state.lastSuccessfulReadAt
            )
        }

        item { SectionLabel(stringResource(R.string.data_sources_section_device)) }
        item {
            DataSourceCard(
                modifier = Modifier.testTag("data.health.card"),
                icon = Icons.Filled.Favorite,
                title = stringResource(R.string.data_sources_health_title),
                detail = healthDetail(state, healthState),
                state = healthState,
                accessPolicy = DataSourceAccessPolicy.ConversationOptIn,
                actionLabel = when (healthState) {
                    SourceVisualState.NeedsPermission -> stringResource(R.string.data_sources_action_allow)
                    SourceVisualState.Ready, SourceVisualState.Partial, SourceVisualState.Error -> stringResource(R.string.data_sources_action_refresh)
                    else -> null
                },
                onAction = when (healthState) {
                    SourceVisualState.NeedsPermission -> onRequestHealthPermission
                    SourceVisualState.Ready, SourceVisualState.Partial, SourceVisualState.Error -> onRefresh
                    else -> null
                }
            )
        }

        state.operationError?.let { error ->
            item {
                Surface(
                    onClick = onClearError,
                    shape = RoundedCornerShape(15.dp),
                    color = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.72f)
                ) {
                    Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(stringResource(R.string.data_sources_validation_failed), fontWeight = FontWeight.SemiBold)
                        Text(error, style = MaterialTheme.typography.bodySmall)
                        Text(stringResource(R.string.data_sources_action_dismiss), style = MaterialTheme.typography.labelSmall)
                    }
                }
            }
        }

        if (showValidationSuccess) {
            item {
                Surface(
                    onClick = { showValidationSuccess = false },
                    shape = RoundedCornerShape(15.dp),
                    color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.72f)
                ) {
                    Row(
                        Modifier.padding(horizontal = 14.dp, vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        Icon(
                            Icons.Filled.Check,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                            tint = MaterialTheme.colorScheme.primary
                        )
                        Text(
                            stringResource(R.string.data_sources_validation_success),
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.SemiBold
                        )
                        Text(
                            stringResource(R.string.data_sources_action_dismiss),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        }

        item { SectionLabel(stringResource(R.string.data_sources_section_readable)) }
        if (state.customSources.isEmpty()) {
            item {
                EmptyCustomSourcesCard(
                    onAdd = {
                        editingSource = null
                        showEditor = true
                    }
                )
            }
        } else {
            DataSourceAccessPolicy.entries.forEach { policy ->
                val matchingSources = state.customSources.filter { it.accessPolicy == policy }
                if (matchingSources.isNotEmpty()) {
                    item(key = "policy-${policy.name}") {
                        DataSourcePolicyHeader(policy = policy, count = matchingSources.size)
                    }
                    items(matchingSources, key = { it.id }) { source ->
                        CustomDataSourceCard(
                            source = source,
                            validating = state.validatingSourceId == source.id,
                            busy = source.id in state.busySourceIds,
                            onOpen = {
                                editingSource = source
                                showEditor = true
                            },
                            onEnabledChange = { onSetEnabled(source.id, it) }
                        )
                    }
                }
            }
            item { DataSourcePolicyGuide() }
        }

        item { SectionLabel(stringResource(R.string.data_sources_section_knowledge)) }
        item {
            DataSourceCard(
                icon = Icons.Filled.Description,
                title = stringResource(R.string.data_sources_documents_title),
                detail = if (state.enabledDocumentCount > 0) {
                    stringResource(
                        R.string.data_sources_documents_ready_detail,
                        state.enabledDocumentCount,
                        NumberFormat.getIntegerInstance().format(state.documentCharacterCount)
                    )
                } else {
                    stringResource(R.string.data_sources_documents_empty_detail)
                },
                state = if (state.enabledDocumentCount > 0) SourceVisualState.Ready else SourceVisualState.Empty
            )
        }

        item { SectionLabel(stringResource(R.string.data_sources_section_capabilities)) }
        item {
            val hasConnectedCapabilities = state.enabledToolCount > 0 || state.enabledSkillCount > 0
            DataSourceCard(
                icon = Icons.Filled.Storage,
                title = stringResource(R.string.data_sources_tools_title),
                detail = if (hasConnectedCapabilities) {
                    stringResource(
                        R.string.data_sources_tools_ready_detail,
                        state.enabledToolCount,
                        state.enabledSkillCount
                    )
                } else {
                    stringResource(R.string.data_sources_tools_empty_detail)
                },
                state = if (hasConnectedCapabilities) SourceVisualState.Ready else SourceVisualState.Empty
            )
        }

        if (state.recentUsage.isNotEmpty()) {
            item { SectionLabel(stringResource(R.string.data_sources_section_recent_usage)) }
            items(state.recentUsage.take(8), key = { it.id }) { record ->
                DataSourceUsageRow(record)
            }
        }
    }
    }


    if (showEditor) {
        DataSourceEditorDialog(
            source = editingSource,
            validating = state.validatingSourceId != null,
            onDismiss = { showEditor = false },
            onDelete = editingSource?.let { source ->
                {
                    onDelete(source.id)
                    showEditor = false
                }
            },
            onTestAndSave = { draft ->
                onValidateAndSave(draft) { success ->
                    if (success) {
                        showEditor = false
                        showValidationSuccess = true
                    }
                }
            }
        )
    }
}

@Composable
private fun DataSourcePolicyHeader(policy: DataSourceAccessPolicy, count: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .testTag("data.sources.policy.${policy.name}"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(
            text = policyLabel(policy),
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Surface(
            shape = RoundedCornerShape(50),
            color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.62f)
        ) {
            Text(
                text = count.toString(),
                modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun EmptyCustomSourcesCard(onAdd: () -> Unit) {
    Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f)) {
        Column(
            Modifier.padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
            horizontalAlignment = Alignment.Start
        ) {
            Icon(Icons.Filled.Link, null, tint = MaterialTheme.colorScheme.primary)
            Text(stringResource(R.string.data_sources_empty_title), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(stringResource(R.string.data_sources_empty_detail), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Button(onClick = onAdd, modifier = Modifier.testTag("data.sources.empty.add")) {
                Icon(Icons.Filled.Add, null, Modifier.size(17.dp))
                Spacer(Modifier.size(7.dp))
                Text(stringResource(R.string.data_sources_action_add))
            }
        }
    }
}

@Composable
private fun DataSourcePolicyGuide() {
    var isExpanded by rememberSaveable { mutableStateOf(false) }
    val reduceMotion = rememberReduceMotionEnabled()
    val chevronRotation by animateFloatAsState(
        targetValue = if (isExpanded) 180f else 0f,
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.disclosure,
            easing = NolaMotion.disclosureEasing,
        ),
        label = "data-policy-disclosure-chevron",
    )

    Surface(
        onClick = { isExpanded = !isExpanded },
        shape = RoundedCornerShape(18.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.40f)
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(11.dp)
            ) {
                Surface(
                    shape = RoundedCornerShape(11.dp),
                    color = MaterialTheme.colorScheme.primary.copy(alpha = 0.09f)
                ) {
                    Icon(
                        Icons.Filled.Storage,
                        contentDescription = null,
                        modifier = Modifier.padding(8.dp).size(17.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
                Text(
                    stringResource(R.string.data_sources_editor_policy),
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold
                )
                Icon(
                    Icons.Filled.KeyboardArrowDown,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.graphicsLayer { rotationZ = chevronRotation },
                )
            }
            AnimatedVisibility(
                visible = isExpanded,
                enter = nolaDisclosureEnter(reduceMotion),
                exit = nolaDisclosureExit(reduceMotion),
            ) {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    PolicyGuideRow(
                        icon = Icons.Filled.Check,
                        title = policyLabel(DataSourceAccessPolicy.DefaultEnabled),
                        detail = policyDetail(DataSourceAccessPolicy.DefaultEnabled),
                        tint = MaterialTheme.colorScheme.primary
                    )
                    PolicyGuideRow(
                        icon = Icons.AutoMirrored.Filled.Chat,
                        title = policyLabel(DataSourceAccessPolicy.ConversationOptIn),
                        detail = policyDetail(DataSourceAccessPolicy.ConversationOptIn),
                        tint = MaterialTheme.colorScheme.tertiary
                    )
                    PolicyGuideRow(
                        icon = Icons.Filled.Warning,
                        title = policyLabel(DataSourceAccessPolicy.AskWhenRelevant),
                        detail = policyDetail(DataSourceAccessPolicy.AskWhenRelevant),
                        tint = MaterialTheme.colorScheme.tertiary
                    )
                    PolicyGuideRow(
                        icon = Icons.Filled.CloudOff,
                        title = policyLabel(DataSourceAccessPolicy.Blocked),
                        detail = policyDetail(DataSourceAccessPolicy.Blocked),
                        tint = MaterialTheme.colorScheme.error
                    )
                }
            }
        }
    }
}

@Composable
private fun PolicyGuideRow(
    icon: ImageVector,
    title: String,
    detail: String,
    tint: Color
) {
    Row(
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(11.dp)
    ) {
        Surface(
            shape = RoundedCornerShape(11.dp),
            color = tint.copy(alpha = 0.09f)
        ) {
            Icon(
                icon,
                contentDescription = null,
                modifier = Modifier.padding(8.dp).size(17.dp),
                tint = tint
            )
        }
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp)
        ) {
            Text(
                title,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold
            )
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
@OptIn(ExperimentalLayoutApi::class)
private fun CustomDataSourceCard(
    source: CustomDataSourceConfig,
    validating: Boolean,
    busy: Boolean,
    onOpen: () -> Unit,
    onEnabledChange: (Boolean) -> Unit
) {
    val state = when {
        validating || busy -> SourceVisualState.Updating
        !source.enabled -> SourceVisualState.Disabled
        source.isUsable -> SourceVisualState.Ready
        source.lastError != null -> SourceVisualState.Error
        else -> SourceVisualState.Untested
    }
    Surface(
        onClick = onOpen,
        modifier = Modifier.testTag("data.sources.source.${source.id}"),
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(11.dp)) {
            Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Surface(shape = RoundedCornerShape(13.dp), color = sourceTint(state).copy(alpha = 0.10f)) {
                    Icon(Icons.Filled.Link, null, Modifier.padding(10.dp), tint = sourceTint(state))
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        source.name,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(source.location, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Box(
                    modifier = Modifier.size(width = 52.dp, height = 48.dp),
                    contentAlignment = Alignment.Center
                ) {
                    if (validating || busy) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp
                        )
                    } else {
                        Switch(
                            checked = source.enabled,
                            onCheckedChange = onEnabledChange,
                            modifier = Modifier.semantics { contentDescription = source.name }
                        )
                    }
                }
            }
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(7.dp)
            ) {
                SourceStatusBadge(state)
                SourceMetadataPill(policyLabel(source.accessPolicy))
                SourceMetadataPill(
                    text = if (source.allowCloudContext) stringResource(R.string.data_sources_cloud_allowed) else stringResource(R.string.data_sources_cloud_local_only),
                    icon = if (source.allowCloudContext) Icons.Filled.Cloud else Icons.Filled.CloudOff
                )
            }
            source.validationContract?.let {
                DataSourceContractSummary(source)
            }
            source.lastError?.takeIf(String::isNotBlank)?.let { error ->
                Text(
                    error,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
    }
}

@Composable
private fun DataSourceContractSummary(source: CustomDataSourceConfig) {
    val contract = source.validationContract ?: return
    Surface(
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.36f)
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            Icon(
                Icons.Filled.Check,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.primary
            )
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                Text(
                    text = contract.readerDescription.takeIf(String::isNotBlank)
                        ?: stringResource(R.string.data_sources_contract_reader),
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    text = "${contract.format} · ${stringResource(R.string.data_sources_records_count, contract.recordCount)}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
            source.lastSuccessfulReadAt?.let {
                Text(
                    text = relativeTime(Instant.ofEpochMilli(it)),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1
                )
            }
        }
    }
}

@Composable
private fun SourceMetadataPill(text: String, icon: ImageVector? = null) {
    Surface(shape = RoundedCornerShape(50), color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.65f)) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            icon?.let {
                Icon(it, contentDescription = null, modifier = Modifier.size(13.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(text, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun DataSourceUsageRow(record: DataSourceUsageRecord) {
    Surface(
        modifier = Modifier.testTag("data.sources.usage.${record.id}"),
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surface
    ) {
        Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(11.dp)) {
            Icon(
                if (record.disclosedToCloud) Icons.Filled.Cloud else Icons.Filled.CloudOff,
                null,
                Modifier.size(19.dp),
                tint = if (record.disclosedToCloud) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.primary
            )
            Column(Modifier.weight(1f)) {
                Text(record.sourceTitle, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                Text(
                    stringResource(R.string.data_sources_usage_detail, record.consumer, relativeTime(Instant.ofEpochMilli(record.usedAt))),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            Text(
                if (record.disclosedToCloud) stringResource(R.string.data_sources_usage_cloud) else stringResource(R.string.data_sources_usage_local),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun DataSourceEditorDialog(
    source: CustomDataSourceConfig?,
    validating: Boolean,
    onDismiss: () -> Unit,
    onDelete: (() -> Unit)?,
    onTestAndSave: (CustomDataSourceConfig) -> Unit
) {
    var name by remember(source?.id) { mutableStateOf(source?.name.orEmpty()) }
    var location by remember(source?.id) { mutableStateOf(source?.location.orEmpty()) }
    var policy by remember(source?.id) { mutableStateOf(source?.accessPolicy ?: DataSourceAccessPolicy.ConversationOptIn) }
    var allowCloud by remember(source?.id) { mutableStateOf(source?.allowCloudContext ?: false) }
    var maxAge by remember(source?.id) { mutableStateOf((source?.maximumAgeMinutes ?: 60).toString()) }
    var showDeleteConfirmation by remember(source?.id) { mutableStateOf(false) }
    var showDiscardConfirmation by remember(source?.id) { mutableStateOf(false) }
    val valid = name.isNotBlank() && location.isNotBlank() && (maxAge.toIntOrNull() ?: 0) > 0
    val isDirty = name != source?.name.orEmpty()
        || location != source?.location.orEmpty()
        || policy != (source?.accessPolicy ?: DataSourceAccessPolicy.ConversationOptIn)
        || allowCloud != (source?.allowCloudContext ?: false)
        || maxAge != (source?.maximumAgeMinutes ?: 60).toString()

    val requestDismiss = {
        if (!validating) {
            if (isDirty) showDiscardConfirmation = true else onDismiss()
        }
    }

    if (showDeleteConfirmation) {
        AlertDialog(
            onDismissRequest = { if (!validating) showDeleteConfirmation = false },
            title = { Text(stringResource(R.string.data_sources_delete_confirm_title)) },
            text = { Text(stringResource(R.string.data_sources_delete_confirm_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDeleteConfirmation = false
                        onDelete?.invoke()
                    },
                    enabled = !validating
                ) {
                    Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showDeleteConfirmation = false }, enabled = !validating) {
                    Text(stringResource(R.string.common_cancel))
                }
            }
        )
        return
    }

    if (showDiscardConfirmation) {
        AlertDialog(
            onDismissRequest = { showDiscardConfirmation = false },
            title = { Text(stringResource(R.string.data_sources_discard_confirm_title)) },
            text = { Text(stringResource(R.string.data_sources_discard_confirm_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDiscardConfirmation = false
                        onDismiss()
                    }
                ) {
                    Text(
                        stringResource(R.string.data_sources_discard_confirm_action),
                        color = MaterialTheme.colorScheme.error
                    )
                }
            },
            dismissButton = {
                TextButton(onClick = { showDiscardConfirmation = false }) {
                    Text(stringResource(R.string.common_cancel))
                }
            }
        )
        return
    }

    AlertDialog(
        modifier = Modifier.imePadding().testTag("data.sources.editor"),
        onDismissRequest = requestDismiss,
        title = {
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(if (source == null) stringResource(R.string.data_sources_editor_add_title) else stringResource(R.string.data_sources_editor_edit_title))
                Text(stringResource(R.string.data_sources_editor_subtitle), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        },
        text = {
            LazyColumn(
                modifier = Modifier.testTag("data.sources.editor.fields"),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                item {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it },
                        modifier = Modifier.fillMaxWidth().testTag("data.sources.editor.name"),
                        label = { Text(stringResource(R.string.data_sources_editor_name)) },
                        singleLine = true
                    )
                }
                item {
                    OutlinedTextField(
                        value = location,
                        onValueChange = { location = it },
                        modifier = Modifier.fillMaxWidth().testTag("data.sources.editor.location"),
                        label = { Text(stringResource(R.string.data_sources_editor_location)) },
                        supportingText = { Text(stringResource(R.string.data_sources_editor_location_hint)) },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                        singleLine = true
                    )
                }
                item {
                    Text(stringResource(R.string.data_sources_editor_policy), style = MaterialTheme.typography.labelLarge)
                    DataSourceAccessPolicy.entries.forEach { candidate ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(
                                    selected = policy == candidate,
                                    onClick = { policy = candidate },
                                    role = Role.RadioButton
                                )
                                .testTag("data.sources.editor.policy.${candidate.name}"),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            RadioButton(selected = policy == candidate, onClick = null)
                            Column(Modifier.weight(1f)) {
                                Text(policyLabel(candidate), style = MaterialTheme.typography.bodyMedium)
                                Text(policyDetail(candidate), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                }
                item {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(stringResource(R.string.data_sources_editor_cloud), style = MaterialTheme.typography.bodyMedium)
                            Text(stringResource(R.string.data_sources_editor_cloud_hint), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        Switch(checked = allowCloud, onCheckedChange = { allowCloud = it })
                    }
                }
                item {
                    OutlinedTextField(
                        value = maxAge,
                        onValueChange = { maxAge = it.filter(Char::isDigit).take(5) },
                        modifier = Modifier.fillMaxWidth().testTag("data.sources.editor.max_age"),
                        label = { Text(stringResource(R.string.data_sources_editor_max_age)) },
                        supportingText = { Text(stringResource(R.string.data_sources_editor_max_age_hint)) },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        singleLine = true
                    )
                }
                source?.validationContract?.let { contract ->
                    item {
                        Surface(shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.38f)) {
                            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text(stringResource(R.string.data_sources_editor_contract), style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
                                Text(contract.readerDescription, style = MaterialTheme.typography.bodySmall)
                                if (contract.schemaFields.isNotEmpty()) {
                                    Text(contract.schemaFields.joinToString(" · "), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            Button(
                onClick = {
                    onTestAndSave(
                        (source ?: CustomDataSourceConfig(name = name, location = location)).copy(
                            name = name,
                            location = location,
                            kind = UnifiedDataSourceKind.Url,
                            accessPolicy = policy,
                            allowCloudContext = allowCloud,
                            maximumAgeMinutes = maxAge.toIntOrNull()?.coerceIn(
                                DATA_SOURCE_MINIMUM_AGE_MINUTES,
                                DATA_SOURCE_MAXIMUM_AGE_MINUTES
                            ) ?: 60
                        )
                    )
                },
                enabled = valid && !validating,
                modifier = Modifier.testTag("data.sources.editor.test_save")
            ) {
                if (validating) {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.size(7.dp))
                }
                Text(stringResource(R.string.data_sources_editor_test_save))
            }
        },
        dismissButton = {
            Row {
                onDelete?.let {
                    TextButton(onClick = { showDeleteConfirmation = true }, enabled = !validating) {
                        Icon(Icons.Filled.Delete, null, Modifier.size(16.dp))
                        Spacer(Modifier.size(5.dp))
                        Text(stringResource(R.string.common_delete))
                    }
                }
                TextButton(onClick = requestDismiss, enabled = !validating) {
                    Text(stringResource(R.string.common_cancel))
                }
            }
        }
    )
}

@Composable
private fun policyLabel(policy: DataSourceAccessPolicy): String = when (policy) {
    DataSourceAccessPolicy.DefaultEnabled -> stringResource(R.string.data_sources_policy_default)
    DataSourceAccessPolicy.ConversationOptIn -> stringResource(R.string.data_sources_policy_conversation)
    DataSourceAccessPolicy.AskWhenRelevant -> stringResource(R.string.data_sources_policy_ask)
    DataSourceAccessPolicy.Blocked -> stringResource(R.string.data_sources_policy_blocked)
}

@Composable
private fun policyDetail(policy: DataSourceAccessPolicy): String = when (policy) {
    DataSourceAccessPolicy.DefaultEnabled -> stringResource(R.string.data_sources_policy_default_detail)
    DataSourceAccessPolicy.ConversationOptIn -> stringResource(R.string.data_sources_policy_conversation_detail)
    DataSourceAccessPolicy.AskWhenRelevant -> stringResource(R.string.data_sources_policy_ask_detail)
    DataSourceAccessPolicy.Blocked -> stringResource(R.string.data_sources_policy_blocked_detail)
}

internal enum class SourceVisualState {
    Ready,
    Partial,
    NeedsPermission,
    Updating,
    Disabled,
    Empty,
    Error,
    Unavailable,
    Untested
}

internal fun healthVisualState(state: DataSourcesUiState): SourceVisualState {
    val health = state.health ?: return SourceVisualState.Updating
    if (state.isRefreshing && health.updatedAt == null) return SourceVisualState.Updating
    if (health.error != null) return SourceVisualState.Error
    if (health.availability != AndroidHealthAvailability.Available) return SourceVisualState.Unavailable
    if (health.permissionState != AndroidHealthPermissionState.Granted) return SourceVisualState.NeedsPermission
    if (health.missingPermissionCount > 0) return SourceVisualState.Partial
    return SourceVisualState.Ready
}

@Composable
private fun healthDetail(state: DataSourcesUiState, visualState: SourceVisualState): String {
    val health = state.health
    return when (visualState) {
        SourceVisualState.Ready -> stringResource(
            R.string.data_sources_health_ready_detail,
            health?.updatedAt?.let(::relativeTime).orEmpty()
        )
        SourceVisualState.Partial -> stringResource(
            R.string.data_sources_health_partial_detail,
            health?.missingPermissionCount ?: 0,
            health?.updatedAt?.let(::relativeTime).orEmpty()
        )
        SourceVisualState.NeedsPermission -> stringResource(R.string.data_sources_health_permission_detail)
        SourceVisualState.Updating -> stringResource(R.string.data_sources_runtime_updating_detail)
        SourceVisualState.Disabled -> stringResource(R.string.data_sources_status_disabled)
        SourceVisualState.Error -> stringResource(R.string.data_sources_health_error_detail)
        SourceVisualState.Unavailable -> if (health?.availability == AndroidHealthAvailability.ProviderUpdateRequired) {
            stringResource(R.string.data_health_update_required)
        } else {
            stringResource(R.string.data_sources_health_unavailable_detail)
        }
        SourceVisualState.Empty -> stringResource(R.string.data_sources_documents_empty_detail)
        SourceVisualState.Untested -> stringResource(R.string.data_sources_status_not_tested)
    }
}

@Composable
private fun DataReadinessHero(readyCount: Int, attentionCount: Int, lastReadAt: Instant?) {
    Surface(shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
        Column(
            modifier = Modifier
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Surface(
                    shape = RoundedCornerShape(14.dp),
                    color = MaterialTheme.colorScheme.primary.copy(alpha = 0.11f)
                ) {
                    Icon(
                        Icons.Filled.Storage,
                        contentDescription = null,
                        modifier = Modifier.padding(11.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
                Column {
                    Text(stringResource(R.string.data_sources_readiness_title), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                    Text(
                        stringResource(R.string.data_sources_readiness_body),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    ReadinessMetric(
                        stringResource(R.string.data_sources_metric_ready),
                        readyCount.toString(),
                        MaterialTheme.colorScheme.primary,
                        Modifier.weight(1f)
                    )
                    ReadinessMetric(
                        stringResource(R.string.data_sources_metric_attention),
                        attentionCount.toString(),
                        if (attentionCount > 0) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.onSurfaceVariant,
                        Modifier.weight(1f)
                    )
                }
                ReadinessMetric(
                    stringResource(R.string.data_sources_metric_last_read),
                    lastReadAt?.let(::relativeTime) ?: stringResource(R.string.data_sources_never_read),
                    MaterialTheme.colorScheme.secondary,
                    Modifier.fillMaxWidth()
                )
            }
        }
    }
}

@Composable
private fun ReadinessMetric(label: String, value: String, tint: Color, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .padding(horizontal = 4.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(3.dp)
    ) {
        Text(value, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(label, style = MaterialTheme.typography.labelSmall, color = tint, maxLines = 1)
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text = text,
        modifier = Modifier.padding(top = 4.dp, start = 2.dp),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        fontWeight = FontWeight.SemiBold
    )
}

@Composable
private fun DataSourceCard(
    modifier: Modifier = Modifier,
    icon: ImageVector,
    title: String,
    detail: String,
    state: SourceVisualState,
    accessPolicy: DataSourceAccessPolicy? = null,
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null
) {
    Surface(modifier = modifier, shape = RoundedCornerShape(19.dp), color = MaterialTheme.colorScheme.surface, tonalElevation = 1.dp) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Surface(
                    shape = RoundedCornerShape(13.dp),
                    color = sourceTint(state).copy(alpha = 0.10f)
                ) {
                    Icon(icon, contentDescription = null, modifier = Modifier.padding(10.dp), tint = sourceTint(state))
                }
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                    Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                SourceStatusBadge(state)
                accessPolicy?.let { policy ->
                    SourceMetadataPill(policyLabel(policy))
                }
                Spacer(Modifier.weight(1f))
                if (actionLabel != null && onAction != null) {
                    TextButton(onClick = onAction) {
                        if (state == SourceVisualState.Updating) {
                            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.size(7.dp))
                        }
                        Text(actionLabel)
                    }
                }
            }
        }
    }
}

@Composable
private fun SourceStatusBadge(state: SourceVisualState) {
    val label = when (state) {
        SourceVisualState.Ready -> stringResource(R.string.data_sources_status_ready)
        SourceVisualState.Partial -> stringResource(R.string.data_sources_status_partial)
        SourceVisualState.NeedsPermission -> stringResource(R.string.data_sources_status_access_needed)
        SourceVisualState.Updating -> stringResource(R.string.data_sources_status_updating)
        SourceVisualState.Disabled -> stringResource(R.string.data_sources_status_disabled)
        SourceVisualState.Empty -> stringResource(R.string.data_sources_status_not_configured)
        SourceVisualState.Error -> stringResource(R.string.data_sources_status_error)
        SourceVisualState.Unavailable -> stringResource(R.string.data_sources_status_unavailable)
        SourceVisualState.Untested -> stringResource(R.string.data_sources_status_not_tested)
    }
    val icon = when (state) {
        SourceVisualState.Ready -> Icons.Filled.Check
        SourceVisualState.Updating -> Icons.Filled.Refresh
        else -> Icons.Filled.Warning
    }
    val tint = sourceTint(state)
    Surface(shape = RoundedCornerShape(50), color = tint.copy(alpha = 0.09f)) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 5.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(12.dp), tint = tint)
            Text(label, style = MaterialTheme.typography.labelSmall, color = tint, maxLines = 1)
        }
    }
}

@Composable
private fun sourceTint(state: SourceVisualState): Color = when (state) {
    SourceVisualState.Ready -> MaterialTheme.colorScheme.primary
    SourceVisualState.Partial, SourceVisualState.NeedsPermission -> MaterialTheme.colorScheme.tertiary
    SourceVisualState.Updating -> MaterialTheme.colorScheme.secondary
    SourceVisualState.Error -> MaterialTheme.colorScheme.error
    SourceVisualState.Disabled, SourceVisualState.Empty, SourceVisualState.Unavailable, SourceVisualState.Untested -> MaterialTheme.colorScheme.onSurfaceVariant
}

private fun relativeTime(instant: Instant): String = DateUtils.getRelativeTimeSpanString(
    instant.toEpochMilli(),
    System.currentTimeMillis(),
    DateUtils.MINUTE_IN_MILLIS,
    DateUtils.FORMAT_ABBREV_RELATIVE
).toString()
