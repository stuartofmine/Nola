package com.nola.ui.screen.scheduled

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.draw.clip
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.local.db.entity.ExecutionResultEntity
import com.nola.data.local.db.entity.ScheduledActionEntity
import com.nola.ui.components.NolaBadge
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaDivider
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaHeroCard
import com.nola.ui.components.NolaSectionTitle
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaTopAppBar
import java.text.SimpleDateFormat
import java.text.DateFormatSymbols
import java.util.Calendar
import java.util.Date
import java.util.Locale
import java.util.TimeZone

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ScheduledActionsScreen(
    onBack: (() -> Unit)? = null,
    onCreateInChat: () -> Unit = {},
    viewModel: ScheduledActionsViewModel = hiltViewModel()
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    var pendingDelete by remember { mutableStateOf<ScheduledActionEntity?>(null) }
    var editingAction by remember { mutableStateOf<ScheduledActionEntity?>(null) }
    var selectedResult by remember { mutableStateOf<ExecutionResultEntity?>(null) }
    var searchText by rememberSaveable { mutableStateOf("") }
    var selectedFilter by rememberSaveable { mutableStateOf(ScheduledActionFilter.ALL) }
    var showsRecentActivity by rememberSaveable { mutableStateOf(false) }
    val orderedActions = remember(state.actions) {
        state.actions.sortedWith(
            compareBy<ScheduledActionEntity> { !it.enabled }
                .thenBy { nextScheduledAt(it) ?: Long.MAX_VALUE }
                .thenBy { it.name.lowercase() }
        )
    }
    val nextAction = orderedActions.firstOrNull { it.enabled && nextScheduledAt(it) != null }
    val latestResultsByAction = remember(state.recentResults) {
        state.recentResults
            .groupBy { it.actionId }
            .mapValues { (_, results) -> results.maxBy { it.createdAt } }
    }
    val attentionActionIds = latestResultsByAction.values
        .filter { it.status.lowercase(Locale.ROOT) in setOf("failed", "partial") }
        .mapTo(mutableSetOf()) { it.actionId }
    val normalizedQuery = searchText.trim()
    val visibleActions = orderedActions.filter { action ->
        val matchesQuery = normalizedQuery.isEmpty()
            || action.name.contains(normalizedQuery, ignoreCase = true)
            || action.url.contains(normalizedQuery, ignoreCase = true)
            || action.userPrompt.orEmpty().contains(normalizedQuery, ignoreCase = true)
        val matchesFilter = when (selectedFilter) {
            ScheduledActionFilter.ALL -> true
            ScheduledActionFilter.ENABLED -> action.enabled
            ScheduledActionFilter.PAUSED -> !action.enabled
            ScheduledActionFilter.ATTENTION -> action.id in attentionActionIds
        }
        matchesQuery && matchesFilter
    }

    pendingDelete?.let { action ->
        AlertDialog(
            onDismissRequest = { pendingDelete = null },
            title = { Text(stringResource(R.string.scheduled_delete_confirm_title)) },
            text = { Text(stringResource(R.string.scheduled_delete_confirm_message, action.name)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        viewModel.delete(action.id)
                        pendingDelete = null
                    }
                ) {
                    Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingDelete = null }) {
                    Text(stringResource(R.string.common_cancel))
                }
            }
        )
    }

    selectedResult?.let { result ->
        ExecutionResultDetailSheet(
            result = result,
            onDismiss = { selectedResult = null }
        )
    }

    editingAction?.let { action ->
        ScheduledActionEditorSheet(
            existing = action,
            onSave = {
                viewModel.save(it) { editingAction = null }
            },
            onDismiss = { editingAction = null },
            isSaving = state.savingActionId != null,
            saveErrorMessage = state.saveErrorMessage,
            savePersistedBeforeFailure = state.savePersistedBeforeFailure,
        )
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            NolaTopAppBar(
                title = stringResource(R.string.scheduled_title),
                onBack = onBack
            ) {
                if (state.actions.isNotEmpty()) {
                    TextButton(onClick = onCreateInChat) {
                        Icon(Icons.Filled.AutoAwesome, contentDescription = null)
                        Spacer(Modifier.width(6.dp))
                        Text(stringResource(R.string.scheduled_create_in_chat))
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
            LazyColumn(
                modifier = Modifier
                    .widthIn(max = 840.dp)
                    .fillMaxSize()
                    .padding(horizontal = NolaSpacing.lg),
                verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
            ) {
            if (state.actions.isNotEmpty()) item {
                Spacer(Modifier.height(NolaSpacing.sm))
                val enabledCount = state.actions.count { it.enabled }
                NolaCard {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.Top,
                        horizontalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        Box(
                            modifier = Modifier
                                .size(42.dp)
                                .clip(CircleShape)
                                .background(NolaColors.warning.copy(alpha = 0.12f)),
                            contentAlignment = Alignment.Center
                        ) {
                            Icon(
                                Icons.Filled.Schedule,
                                contentDescription = null,
                                tint = NolaColors.warning
                            )
                        }
                        Column(
                            modifier = Modifier.weight(1f),
                            verticalArrangement = Arrangement.spacedBy(4.dp)
                        ) {
                            Text(
                                stringResource(R.string.scheduled_next_title),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                            Text(
                                nextAction?.name ?: stringResource(R.string.scheduled_next_none),
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.SemiBold
                            )
                            nextAction?.let { action ->
                                nextScheduledAt(action)?.let { timestamp ->
                                    Text(
                                        java.text.DateFormat.getDateTimeInstance(
                                            java.text.DateFormat.MEDIUM,
                                            java.text.DateFormat.SHORT,
                                            Locale.getDefault()
                                        ).format(Date(timestamp)),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.primary
                                    )
                                }
                            }
                        }
                        if (state.actions.isNotEmpty()) {
                            NolaBadge(
                                stringResource(R.string.scheduled_enabled_total_count, enabledCount, state.actions.size),
                                tint = MaterialTheme.colorScheme.primary
                            )
                        }
                    }
                }
            }

            if (state.isLoading && state.actions.isEmpty()) {
                item {
                    NolaCard {
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(vertical = NolaSpacing.md),
                            horizontalArrangement = Arrangement.Center,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                            Spacer(Modifier.width(10.dp))
                            Text(
                                stringResource(R.string.scheduled_loading),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                }
            }

            state.errorMessage?.let { error ->
                item {
                    NolaCard {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp)
                        ) {
                            Icon(Icons.Filled.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error)
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    stringResource(R.string.scheduled_load_failed),
                                    style = MaterialTheme.typography.bodyMedium,
                                    fontWeight = FontWeight.SemiBold,
                                    color = MaterialTheme.colorScheme.error
                                )
                                if (error.isNotBlank()) {
                                    Text(
                                        error,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        maxLines = 2,
                                        overflow = TextOverflow.Ellipsis
                                    )
                                }
                            }
                            TextButton(onClick = viewModel::loadActions) {
                                Text(stringResource(R.string.common_refresh))
                            }
                        }
                    }
                }
            }

            if (state.actions.isNotEmpty()) {
                item {
                    ScheduledActionsFilterBar(
                        searchText = searchText,
                        onSearchTextChange = { searchText = it },
                        selectedFilter = selectedFilter,
                        onFilterChange = { selectedFilter = it },
                        totalCount = state.actions.size,
                        enabledCount = state.actions.count { it.enabled },
                        attentionCount = attentionActionIds.size
                    )
                }
            }

            when {
            state.isLoading && state.actions.isEmpty() -> {
                item {
                    NolaCard {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 14.dp, vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(18.dp),
                                strokeWidth = 2.dp,
                            )
                            Text(
                                stringResource(R.string.scheduled_loading),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
            !state.isLoading && state.errorMessage == null && state.actions.isEmpty() -> {
                item {
                    Column(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(stringResource(R.string.scheduled_no_automations), style = MaterialTheme.typography.titleMedium)
                        Text(stringResource(R.string.scheduled_no_automations_message), style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                        TextButton(onClick = onCreateInChat) { Text(stringResource(R.string.scheduled_create_in_chat)) }
                    }
                }
            }
            state.actions.isEmpty() -> Unit
            visibleActions.isEmpty() -> {
                item {
                    NolaCard {
                        NolaEmptyState(
                            icon = Icons.Filled.Search,
                            title = stringResource(R.string.scheduled_filter_empty_title),
                            message = stringResource(R.string.scheduled_filter_empty_message)
                        )
                        TextButton(
                            onClick = {
                                searchText = ""
                                selectedFilter = ScheduledActionFilter.ALL
                            },
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            Text(stringResource(R.string.scheduled_filter_reset))
                        }
                    }
                }
            }
            else -> {
                items(visibleActions, key = { it.id }) { action ->
                    val latestResult = latestResultsByAction[action.id]
                    ScheduledActionCard(
                        action = action,
                        latestResult = latestResult,
                        isBusy = action.id in state.busyActionIds,
                        onToggle = { viewModel.toggle(action.id, it) },
                        onRequestEdit = { viewModel.beginEditing(); editingAction = action },
                        onRequestDelete = { pendingDelete = action },
                        onResultClick = { latestResult?.let { selectedResult = it } }
                    )
                }
            }
            }

            if (state.recentResults.isNotEmpty()) {
                item {
                    Surface(
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("scheduled.recent.toggle")
                            .clickable(role = Role.Button) {
                                showsRecentActivity = !showsRecentActivity
                            },
                        shape = MaterialTheme.shapes.large,
                        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f)
                    ) {
                        Row(
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    stringResource(R.string.scheduled_recent_activity),
                                    style = MaterialTheme.typography.titleSmall,
                                    fontWeight = FontWeight.SemiBold
                                )
                                Text(
                                    stringResource(
                                        R.string.scheduled_recent_activity_count,
                                        state.recentResults.size
                                    ),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                            Icon(
                                if (showsRecentActivity) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                                contentDescription = stringResource(R.string.scheduled_recent_activity)
                            )
                        }
                    }
                }
                if (showsRecentActivity) {
                    item {
                        NolaCard {
                            state.recentResults.take(10).forEachIndexed { index, result ->
                                if (index > 0) NolaDivider()
                                ExecutionResultRow(
                                    result = result,
                                    onClick = { selectedResult = result }
                                )
                            }
                        }
                    }
                }
            }

                item { Spacer(Modifier.height(NolaSpacing.xxl)) }
            }
        }
    }
}

private enum class ScheduledActionFilter {
    ALL,
    ENABLED,
    PAUSED,
    ATTENTION
}

@Composable
private fun ScheduledActionsFilterBar(
    searchText: String,
    onSearchTextChange: (String) -> Unit,
    selectedFilter: ScheduledActionFilter,
    onFilterChange: (ScheduledActionFilter) -> Unit,
    totalCount: Int,
    enabledCount: Int,
    attentionCount: Int
) {
    Column(verticalArrangement = Arrangement.spacedBy(NolaSpacing.sm)) {
        OutlinedTextField(
            value = searchText,
            onValueChange = onSearchTextChange,
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null) },
            trailingIcon = {
                if (searchText.isNotEmpty()) {
                    IconButton(onClick = { onSearchTextChange("") }) {
                        Icon(Icons.Filled.Close, stringResource(R.string.scheduled_filter_clear_search))
                    }
                }
            },
            placeholder = { Text(stringResource(R.string.scheduled_filter_search)) }
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
        ) {
            ScheduledActionFilter.entries.forEach { filter ->
                val count = when (filter) {
                    ScheduledActionFilter.ALL -> totalCount
                    ScheduledActionFilter.ENABLED -> enabledCount
                    ScheduledActionFilter.PAUSED -> totalCount - enabledCount
                    ScheduledActionFilter.ATTENTION -> attentionCount
                }
                val label = when (filter) {
                    ScheduledActionFilter.ALL -> R.string.scheduled_filter_all
                    ScheduledActionFilter.ENABLED -> R.string.scheduled_filter_enabled
                    ScheduledActionFilter.PAUSED -> R.string.scheduled_filter_paused
                    ScheduledActionFilter.ATTENTION -> R.string.scheduled_filter_attention
                }
                FilterChip(
                    selected = selectedFilter == filter,
                    onClick = { onFilterChange(filter) },
                    label = { Text("${stringResource(label)} ($count)") }
                )
            }
        }
    }
}

@Composable
fun ScheduledActionCard(
    action: ScheduledActionEntity,
    latestResult: ExecutionResultEntity?,
    isBusy: Boolean = false,
    onToggle: (Boolean) -> Unit,
    onRequestEdit: () -> Unit,
    onRequestDelete: () -> Unit,
    onResultClick: () -> Unit = {}
) {
    val dateFormat = remember { SimpleDateFormat("MMM dd, HH:mm", Locale.getDefault()) }
    var showMenu by remember(action.id) { mutableStateOf(false) }
    val needsAttention = latestResult?.status?.lowercase(Locale.ROOT) in setOf("failed", "partial")
    val isRunning = latestResult?.status?.lowercase(Locale.ROOT) in setOf("pending", "running")
    Box(modifier = Modifier.testTag("scheduled.card.${action.id}")) {
    NolaCard {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                Box(
                    modifier = Modifier
                        .size(38.dp)
                        .clip(MaterialTheme.shapes.medium)
                        .background(
                            (if (action.enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
                                .copy(alpha = 0.1f)
                        ),
                    contentAlignment = Alignment.Center
                ) {
                    Icon(
                        if (action.enabled) Icons.Filled.Schedule else Icons.Filled.Pause,
                        contentDescription = null,
                        modifier = Modifier.size(18.dp),
                        tint = if (action.enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        action.name,
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis
                    )
                    Text(
                        scheduleDescription(action),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                }
                Box {
                    IconButton(onClick = { showMenu = true }, enabled = !isBusy && !isRunning) {
                        Icon(
                            Icons.Filled.MoreHoriz,
                            stringResource(R.string.scheduled_more_options),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    DropdownMenu(expanded = showMenu, onDismissRequest = { showMenu = false }) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error) },
                            leadingIcon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                            onClick = {
                                showMenu = false
                                onRequestDelete()
                            }
                        )
                    }
                }
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                when {
                    isRunning && latestResult != null -> NolaBadge(
                        resultStatusLabel(latestResult.status),
                        tint = resultStatusColor(latestResult.status)
                    )
                    needsAttention && latestResult != null -> NolaBadge(
                        resultStatusLabel(latestResult.status),
                        tint = resultStatusColor(latestResult.status)
                    )
                    !action.enabled -> NolaBadge(
                        stringResource(R.string.scheduled_paused),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    else -> NolaBadge(
                        stringResource(R.string.scheduled_filter_enabled),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
                Spacer(Modifier.weight(1f))
                Box(
                    modifier = Modifier.size(width = 52.dp, height = 48.dp),
                    contentAlignment = Alignment.Center
                ) {
                    if (isBusy) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    } else {
                        Switch(
                            checked = action.enabled,
                            onCheckedChange = onToggle,
                            modifier = Modifier.testTag("scheduled.toggle.${action.id}")
                        )
                    }
                }
            }

            if (isRunning) {
                Surface(
                    shape = MaterialTheme.shapes.medium,
                    color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.42f)
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(17.dp), strokeWidth = 2.dp)
                        Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text(
                                stringResource(R.string.scheduled_result_running),
                                style = MaterialTheme.typography.bodySmall,
                                fontWeight = FontWeight.SemiBold,
                                color = MaterialTheme.colorScheme.primary
                            )
                            Text(
                                action.url,
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis
                            )
                        }
                    }
                }
            }

            action.userPrompt?.trim()?.takeIf { it.isNotEmpty() }?.let { prompt ->
                Text(
                    prompt,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Text(
                    action.format.uppercase(Locale.getDefault()),
                    modifier = Modifier
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.55f), MaterialTheme.shapes.small)
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Text(
                    action.url,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }

            Column(verticalArrangement = Arrangement.spacedBy(7.dp)) {
                nextScheduledAt(action)?.let { timestamp ->
                    Text(
                        "${stringResource(R.string.scheduled_next_title)} · ${dateFormat.format(Date(timestamp))}",
                        style = MaterialTheme.typography.labelSmall,
                        fontWeight = FontWeight.Medium,
                        color = if (action.enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                latestResult?.let { result ->
                    val detail = result.failureReason ?: result.llmSummaryText
                    Surface(
                        modifier = Modifier
                            .fillMaxWidth()
                            .testTag("scheduled.result.${action.id}")
                            .clickable(role = Role.Button, onClick = onResultClick),
                        shape = MaterialTheme.shapes.medium,
                        color = resultStatusColor(result.status).copy(alpha = 0.08f)
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                            verticalArrangement = Arrangement.spacedBy(4.dp)
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(7.dp)
                            ) {
                                Box(
                                    Modifier
                                        .size(7.dp)
                                        .clip(CircleShape)
                                        .background(resultStatusColor(result.status))
                                )
                                Text(
                                    stringResource(R.string.scheduled_recent_results),
                                    style = MaterialTheme.typography.labelMedium,
                                    fontWeight = FontWeight.SemiBold,
                                    modifier = Modifier.weight(1f)
                                )
                                Text(
                                    stringResource(
                                        R.string.scheduled_last_run_status_format,
                                        dateFormat.format(Date(result.createdAt)),
                                        resultStatusLabel(result.status)
                                    ),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                            if (!detail.isNullOrBlank()) {
                                Text(
                                    detail,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis
                                )
                            }
                        }
                    }
                }
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End
            ) {
                TextButton(
                    onClick = onRequestEdit,
                    enabled = !isBusy && !isRunning,
                    modifier = Modifier.testTag("scheduled.edit.${action.id}")
                ) {
                    Icon(Icons.Filled.Edit, contentDescription = null, modifier = Modifier.size(17.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(stringResource(R.string.scheduled_edit))
                }
            }
        }
    }
    }
}

@Composable
fun ExecutionResultRow(result: ExecutionResultEntity, onClick: () -> Unit = {}) {
    val dateFormat = remember { SimpleDateFormat("MMM dd, HH:mm", Locale.getDefault()) }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(5.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(7.dp)
                    .clip(CircleShape)
                    .background(resultStatusColor(result.status))
            )
            Spacer(Modifier.width(9.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    result.sourceName ?: stringResource(R.string.scheduled_action_default),
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    dateFormat.format(Date(result.createdAt)),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            NolaBadge(resultStatusLabel(result.status), tint = resultStatusColor(result.status))
        }
        val detail = result.failureReason ?: result.llmSummaryText?.take(180)
        if (!detail.isNullOrBlank()) {
            Text(
                detail,
                modifier = Modifier.padding(start = 16.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ExecutionResultDetailSheet(
    result: ExecutionResultEntity,
    onDismiss: () -> Unit,
) {
    val dateFormat = remember { SimpleDateFormat("MMM dd, yyyy · HH:mm", Locale.getDefault()) }
    val uriHandler = LocalUriHandler.current
    var showsTechnicalDetails by rememberSaveable(result.id) { mutableStateOf(false) }
    val hasTechnicalDetails = result.backendResultId?.isNotBlank() == true
        || result.eventsJson.trim() !in setOf("", "[]", "{}")

    ModalBottomSheet(onDismissRequest = onDismiss) {
        LazyColumn(
            modifier = Modifier
                .widthIn(max = 760.dp)
                .fillMaxWidth()
                .padding(horizontal = NolaSpacing.lg),
            verticalArrangement = Arrangement.spacedBy(NolaSpacing.md)
        ) {
            item {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.Top,
                    horizontalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Box(
                        modifier = Modifier
                            .size(42.dp)
                            .clip(CircleShape)
                            .background(resultStatusColor(result.status).copy(alpha = 0.12f)),
                        contentAlignment = Alignment.Center
                    ) {
                        Icon(
                            Icons.Filled.Schedule,
                            contentDescription = null,
                            tint = resultStatusColor(result.status)
                        )
                    }
                    Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(
                            result.sourceName ?: stringResource(R.string.scheduled_action_default),
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.SemiBold
                        )
                        Text(
                            stringResource(R.string.scheduled_result_detail_title),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    NolaBadge(resultStatusLabel(result.status), tint = resultStatusColor(result.status))
                }
            }

            result.failureReason?.takeIf { it.isNotBlank() }?.let { failure ->
                item {
                    NolaCard {
                        Text(
                            stringResource(R.string.scheduled_result_failure_reason),
                            style = MaterialTheme.typography.titleSmall,
                            color = MaterialTheme.colorScheme.error
                        )
                        SelectionContainer {
                            Text(failure, style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                }
            }

            result.llmSummaryText?.takeIf { it.isNotBlank() }?.let { summary ->
                item {
                    NolaCard {
                        Text(
                            stringResource(R.string.scheduled_result_summary),
                            style = MaterialTheme.typography.titleSmall
                        )
                        SelectionContainer {
                            com.nola.ui.components.MarkdownRenderer(summary)
                        }
                    }
                }
            }

            item {
                NolaCard {
                    Text(
                        stringResource(R.string.scheduled_result_run_information),
                        style = MaterialTheme.typography.titleSmall
                    )
                    ExecutionResultInfoRow(
                        stringResource(R.string.scheduled_result_source),
                        result.sourceName ?: stringResource(R.string.scheduled_action_default)
                    )
                    result.sourceFormat?.takeIf { it.isNotBlank() }?.let {
                        ExecutionResultInfoRow(stringResource(R.string.scheduled_result_format), resultFormatLabel(it))
                    }
                    result.scheduledAt?.let {
                        ExecutionResultInfoRow(stringResource(R.string.scheduled_result_scheduled_at), dateFormat.format(Date(it)))
                    }
                    ExecutionResultInfoRow(
                        stringResource(R.string.scheduled_result_completed_at),
                        dateFormat.format(Date(result.fetchedAt ?: result.createdAt))
                    )
                    result.sourceUrl?.takeIf { it.isNotBlank() }?.let { url ->
                        if (url.startsWith("https://", ignoreCase = true) || url.startsWith("http://", ignoreCase = true)) {
                            TextButton(onClick = { runCatching { uriHandler.openUri(url) } }) {
                                Text(stringResource(R.string.scheduled_result_open_source))
                            }
                        } else {
                            ExecutionResultInfoRow(
                                stringResource(R.string.scheduled_result_local_location),
                                url
                            )
                        }
                    }
                }
            }

            if (hasTechnicalDetails) {
                item {
                    NolaCard {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable(role = Role.Button) {
                                    showsTechnicalDetails = !showsTechnicalDetails
                                }
                                .padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(
                                stringResource(R.string.scheduled_result_technical_details),
                                style = MaterialTheme.typography.titleSmall,
                                modifier = Modifier.weight(1f)
                            )
                            Icon(
                                if (showsTechnicalDetails) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                                contentDescription = stringResource(
                                    if (showsTechnicalDetails) R.string.scheduled_result_hide_technical
                                    else R.string.scheduled_result_show_technical
                                )
                            )
                        }
                        if (showsTechnicalDetails) {
                            ExecutionResultInfoRow(stringResource(R.string.scheduled_result_id), result.id)
                            result.backendResultId?.takeIf { it.isNotBlank() }?.let {
                                ExecutionResultInfoRow(stringResource(R.string.scheduled_result_backend_id), it)
                            }
                            if (result.eventsJson.trim() !in setOf("", "[]", "{}")) {
                                Text(
                                    stringResource(R.string.scheduled_result_raw_events),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                SelectionContainer {
                                    Text(
                                        result.eventsJson,
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                            }
                        }
                    }
                }
            }

            item {
                TextButton(onClick = onDismiss, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.common_close))
                }
                Spacer(Modifier.height(NolaSpacing.xl))
            }
        }
    }
}

@Composable
private fun ExecutionResultInfoRow(label: String, value: String) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.Top
    ) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.weight(0.38f)
        )
        SelectionContainer(modifier = Modifier.weight(0.62f)) {
            Text(
                value,
                style = MaterialTheme.typography.bodySmall,
                fontWeight = FontWeight.Medium
            )
        }
    }
}

@Composable
private fun resultStatusColor(status: String): Color =
    when (status.lowercase()) {
        "answered", "action_completed" -> NolaColors.success
        "incomplete", "blocked", "needs_attention" -> NolaColors.warning
        "skipped", "delegation_accepted" -> MaterialTheme.colorScheme.onSurfaceVariant
        "pending", "running" -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.error
    }

@Composable
private fun resultStatusLabel(status: String): String =
    when (status.lowercase()) {
        "answered", "action_completed" -> stringResource(R.string.scheduled_result_success)
        "delegation_accepted" -> stringResource(R.string.scheduled_result_delegated)
        "incomplete" -> stringResource(R.string.scheduled_result_partial)
        "blocked", "needs_attention" -> stringResource(R.string.scheduled_result_needs_attention)
        "cancelled" -> stringResource(R.string.scheduled_result_cancelled)
        "skipped" -> stringResource(R.string.scheduled_result_skipped)
        "pending", "running" -> stringResource(R.string.scheduled_result_running)
        else -> stringResource(R.string.scheduled_result_failed)
    }

@Composable
private fun resultFormatLabel(format: String): String =
    when (format.lowercase(Locale.ROOT)) {
        "html" -> stringResource(R.string.scheduled_result_format_web)
        "search" -> stringResource(R.string.scheduled_result_format_search)
        "script" -> stringResource(R.string.scheduled_result_format_script)
        "file" -> stringResource(R.string.scheduled_result_format_file)
        "folder" -> stringResource(R.string.scheduled_result_format_folder)
        "rss" -> stringResource(R.string.scheduled_result_format_rss)
        "json" -> stringResource(R.string.scheduled_result_format_json)
        "icalendar", "calendar" -> stringResource(R.string.scheduled_result_format_calendar)
        else -> format
    }

internal fun nextScheduledAt(
    action: ScheduledActionEntity,
    now: Long = System.currentTimeMillis()
): Long? {
    if (!action.enabled) return null
    val rawTime = action.scheduledTimeIso?.trim().orEmpty()
    if (action.scheduleFrequency == "once") {
        val patterns = listOf("yyyy-MM-dd'T'HH:mm:ssXXX", "yyyy-MM-dd'T'HH:mm", "yyyy-MM-dd HH:mm")
        return patterns.firstNotNullOfOrNull { pattern ->
            runCatching {
                SimpleDateFormat(pattern, Locale.US).parse(rawTime)?.time
            }.getOrNull()?.takeIf { it > now }
        }
    }
    if (action.scheduleFrequency != "daily" && action.scheduleFrequency != "weekly") return null
    val match = Regex("""(\d{1,2}):(\d{2})""").findAll(rawTime).lastOrNull() ?: return null
    val hour = match.groupValues[1].toIntOrNull()?.takeIf { it in 0..23 } ?: return null
    val minute = match.groupValues[2].toIntOrNull()?.takeIf { it in 0..59 } ?: return null
    val zone = TimeZone.getTimeZone(action.timezone.ifBlank { TimeZone.getDefault().id })
    val next = Calendar.getInstance(zone).apply {
        timeInMillis = now
        set(Calendar.HOUR_OF_DAY, hour)
        set(Calendar.MINUTE, minute)
        set(Calendar.SECOND, 0)
        set(Calendar.MILLISECOND, 0)
        if (action.scheduleFrequency == "weekly") {
            val isoWeekday = action.scheduledWeekday
                ?.coerceIn(1, 7)
                ?: calendarWeekdayToIso(firstDayOfWeek)
            set(Calendar.DAY_OF_WEEK, isoWeekdayToCalendarWeekday(isoWeekday))
        }
        if (timeInMillis <= now) {
            add(
                if (action.scheduleFrequency == "weekly") Calendar.WEEK_OF_YEAR else Calendar.DAY_OF_YEAR,
                1
            )
        }
    }
    return next.timeInMillis
}

@Composable
private fun scheduleDescription(action: ScheduledActionEntity): String {
    return scheduleFrequencyLabel(action.scheduleFrequency)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ScheduledActionEditorSheet(
    existing: ScheduledActionEntity?,
    onSave: (ScheduledActionEntity) -> Unit,
    onDismiss: () -> Unit,
    isSaving: Boolean = false,
    saveErrorMessage: String? = null,
    savePersistedBeforeFailure: Boolean = false,
) {
    val initialName = existing?.name ?: ""
    val initialUrl = existing?.url ?: ""
    val format = existing?.format ?: "url"
    val initialPrompt = existing?.userPrompt.orEmpty()
    var userPrompt by remember(existing?.id) { mutableStateOf(initialPrompt) }
    val initialFrequency = existing?.scheduleFrequency?.takeUnless { it == "none" } ?: "once"
    val initialWeekday = existing?.scheduledWeekday?.toString() ?: ""
    val initialTime = existing?.scheduledTimeIso ?: ""
    val initialTimezone = existing?.timezone?.takeIf { it.isNotBlank() } ?: TimeZone.getDefault().id
    val draftId = remember(existing?.id) { existing?.id ?: java.util.UUID.randomUUID().toString() }
    var name by remember(existing?.id) { mutableStateOf(initialName) }
    var url by remember(existing?.id) { mutableStateOf(initialUrl) }
    var scheduleFrequency by remember(existing?.id) {
        mutableStateOf(initialFrequency)
    }
    var scheduledWeekday by remember(existing?.id) { mutableStateOf(initialWeekday) }
    var scheduledTimeIso by remember(existing?.id) { mutableStateOf(initialTime) }
    var timezone by remember(existing?.id) { mutableStateOf(initialTimezone) }
    var showDiscardConfirmation by remember(existing?.id) { mutableStateOf(false) }
    val isDirty = name != initialName
        || userPrompt != initialPrompt
        || url != initialUrl
        || scheduleFrequency != initialFrequency
        || scheduledWeekday != initialWeekday
        || scheduledTimeIso != initialTime
        || timezone != initialTimezone
    val timeValidation = validateScheduleTime(scheduleFrequency, scheduledTimeIso, timezone)
    val isTimeValid = timeValidation is ScheduleTimeValidation.Valid
    val isTimezoneValid = runCatching { java.time.ZoneId.of(timezone) }.isSuccess
    val weekdayLabels = remember {
        val names = DateFormatSymbols.getInstance(Locale.getDefault()).shortWeekdays
        (1..7).map { isoDay -> isoDay to names[isoWeekdayToCalendarWeekday(isoDay)] }
    }
    val requestDismiss = {
        if (!isSaving) {
            if (isDirty) showDiscardConfirmation = true else onDismiss()
        }
    }

    if (showDiscardConfirmation) {
        AlertDialog(
            onDismissRequest = { showDiscardConfirmation = false },
            title = { Text(stringResource(R.string.scheduled_discard_title)) },
            text = { Text(stringResource(R.string.scheduled_discard_message)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showDiscardConfirmation = false
                        onDismiss()
                    }
                ) {
                    Text(
                        stringResource(R.string.scheduled_discard_changes),
                        color = MaterialTheme.colorScheme.error
                    )
                }
            },
            dismissButton = {
                TextButton(onClick = { showDiscardConfirmation = false }) {
                    Text(stringResource(R.string.scheduled_keep_editing))
                }
            }
        )
    }

    androidx.compose.material3.ModalBottomSheet(
        onDismissRequest = requestDismiss,
        sheetState = com.nola.ui.components.rememberGuardedEditorSheetState(isSaving, isDirty) { showDiscardConfirmation = true },
    ) {
        LazyColumn(modifier = Modifier.imePadding().navigationBarsPadding().padding(24.dp)) {
            item {
                Text(
                    if (existing != null) stringResource(R.string.scheduled_action_editor_edit) else stringResource(R.string.scheduled_action_editor_new),
                    style = MaterialTheme.typography.titleLarge
                )
                Spacer(Modifier.height(16.dp))
            }

            item {
                androidx.compose.material3.OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    enabled = !isSaving,
                    label = { Text(stringResource(R.string.scheduled_name)) },
                    placeholder = { Text(stringResource(R.string.scheduled_name_placeholder)) },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true
                )
                Spacer(Modifier.height(8.dp))
            }

            item {
                androidx.compose.material3.OutlinedTextField(
                    value = userPrompt,
                    onValueChange = { userPrompt = it },
                    enabled = !isSaving,
                    label = { Text(stringResource(R.string.scheduled_task_instruction)) },
                    supportingText = { Text(stringResource(R.string.scheduled_task_instruction_help)) },
                    modifier = Modifier.fillMaxWidth().testTag("scheduled-task-instruction"),
                    minLines = 3,
                )
                Spacer(Modifier.height(8.dp))
            }

            item {
                androidx.compose.material3.OutlinedTextField(
                    value = url,
                    onValueChange = { url = it },
                    enabled = !isSaving,
                    label = { Text(stringResource(R.string.scheduled_url)) },
                    placeholder = { Text(stringResource(R.string.scheduled_url_placeholder)) },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true
                )
                Spacer(Modifier.height(8.dp))
            }

            item {
                Text(stringResource(R.string.scheduled_schedule), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.height(8.dp))
                Row(
                    modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    listOf(
                        "once" to stringResource(R.string.scheduled_once),
                        "daily" to stringResource(R.string.scheduled_daily),
                        "weekly" to stringResource(R.string.scheduled_weekly)
                    ).forEach { (value, label) ->
                        FilterChip(
                            selected = scheduleFrequency == value,
                            enabled = !isSaving,
                            onClick = {
                                if (scheduleFrequency != value) {
                                    scheduledTimeIso = if (value == "once") {
                                        val time = scheduleLocalTime(scheduledTimeIso, timezone)
                                        time?.let { java.time.LocalDate.now(runCatching { java.time.ZoneId.of(timezone) }
                                            .getOrDefault(java.time.ZoneId.systemDefault())).atTime(it).toString() }.orEmpty()
                                    } else {
                                        scheduleLocalTime(scheduledTimeIso, timezone)
                                            ?.format(java.time.format.DateTimeFormatter.ofPattern("HH:mm")).orEmpty()
                                    }
                                    scheduleFrequency = value
                                }
                            },
                            label = { Text(label) }
                        )
                    }
                }
                if (scheduleFrequency in setOf("once", "daily", "weekly")) {
                    Spacer(Modifier.height(8.dp))
                    if (scheduleFrequency == "weekly") {
                        Text(
                            stringResource(R.string.scheduled_weekday_label),
                            style = MaterialTheme.typography.labelLarge
                        )
                        Row(
                            modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                            horizontalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            weekdayLabels.forEach { (day, label) ->
                                FilterChip(
                                    selected = scheduledWeekday == day.toString(),
                                    enabled = !isSaving,
                                    onClick = { scheduledWeekday = day.toString() },
                                    label = { Text(label) }
                                )
                            }
                        }
                        Spacer(Modifier.height(8.dp))
                    }
                    ScheduleTimeFields(scheduleFrequency, scheduledTimeIso, timezone, enabled = !isSaving) { time, zone ->
                        scheduledTimeIso = time
                        timezone = zone
                    }
                } else {
                    Text(stringResource(R.string.scheduled_frequency_unsupported, scheduleFrequency),
                        color = MaterialTheme.colorScheme.error)
                }
                Spacer(Modifier.height(16.dp))
            }

            item {
                saveErrorMessage?.let { message ->
                    Text(stringResource(if (savePersistedBeforeFailure) R.string.scheduled_save_partial else R.string.scheduled_save_failed),
                        color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("schedule-save-error"))
                    if (message.isNotBlank()) Text(message, style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error)
                    Spacer(Modifier.height(8.dp))
                }
                androidx.compose.material3.Button(
                    onClick = {
                        val action = ScheduledActionEntity(
                            id = draftId,
                            localUserId = existing?.localUserId ?: "local",
                            name = name,
                            url = url,
                            format = format,
                            userPrompt = userPrompt.trim().takeIf { it.isNotBlank() },
                            scheduleFrequency = scheduleFrequency,
                            scheduledWeekday = scheduledWeekday.toIntOrNull(),
                            scheduledTimeIso = (timeValidation as? ScheduleTimeValidation.Valid)?.wireValue,
                            timezone = timezone,
                            enabled = existing?.enabled ?: true,
                            backendSourceId = existing?.backendSourceId,
                            createdAt = existing?.createdAt ?: System.currentTimeMillis()
                        )
                        onSave(action)
                    },
                    modifier = Modifier.fillMaxWidth().testTag("schedule-save"),
                    enabled = !isSaving && name.isNotBlank()
                        && userPrompt.isNotBlank()
                        && url.isNotBlank()
                        && isTimeValid
                        && isTimezoneValid
                        && (scheduleFrequency != "weekly" || scheduledWeekday.toIntOrNull() in 1..7)
                ) { Text(stringResource(if (isSaving) R.string.scheduled_saving else R.string.common_save)) }
                TextButton(onClick = requestDismiss, enabled = !isSaving, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.common_cancel))
                }
                Spacer(Modifier.height(32.dp))
            }
        }
    }
}

private fun isoWeekdayToCalendarWeekday(isoWeekday: Int): Int = (isoWeekday % 7) + 1

private fun calendarWeekdayToIso(calendarWeekday: Int): Int =
    if (calendarWeekday == Calendar.SUNDAY) 7 else calendarWeekday - 1

@Composable
private fun scheduleFrequencyLabel(value: String): String =
    when (value) {
        "daily" -> stringResource(R.string.scheduled_daily)
        "weekly" -> stringResource(R.string.scheduled_weekly)
        else -> stringResource(R.string.scheduled_once)
    }
