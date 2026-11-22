// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.activity

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.unit.sp
import com.nola.ui.components.NolaBadge
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaDisclosureBar
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaPrimaryButton
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.nolaPageGutter
import com.nola.ui.components.rememberReduceMotionEnabled
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.ui.components.MarkdownRenderer
import com.nola.ui.components.NolaRootPageTitle
import java.text.DateFormat
import java.util.Date

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
internal fun ActivityScreen(onOpenConversation: (String) -> Unit, onOpenPlans: (() -> Unit)? = null, viewModel: ActivityViewModel = hiltViewModel()) {
    val keyboard = LocalSoftwareKeyboardController.current
    val gutter = nolaPageGutter()
    val state by viewModel.state.collectAsStateWithLifecycle()
    val detail by viewModel.detail.collectAsStateWithLifecycle()
    var source by rememberSaveable { mutableStateOf<String?>(null) }
    var needsResponse by rememberSaveable { mutableStateOf(false) }
    var search by rememberSaveable { mutableStateOf("") }
    var searchDraft by rememberSaveable { mutableStateOf("") }
    fun refresh() = viewModel.refresh(source, needsResponse, search)
    fun submitSearch() { search = searchDraft; keyboard?.hide() }
    LaunchedEffect(searchDraft) {
        if (searchDraft != search) {
            kotlinx.coroutines.delay(300)
            search = searchDraft
        }
    }
    LifecycleResumeEffect(source, needsResponse, search) {
        viewModel.resume(source, needsResponse, search)
        onPauseOrDispose { }
    }
    Box(Modifier.fillMaxSize().imePadding()) {
        LazyColumn(Modifier.align(Alignment.TopCenter).widthIn(max = 720.dp).fillMaxSize(),
            contentPadding = PaddingValues(start = gutter, end = gutter, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            item {
                NolaRootPageTitle(stringResource(R.string.activity_title)) {
                    IconButton(onClick = ::refresh, enabled = !state.loading) {
                        Icon(Icons.Filled.Refresh, stringResource(R.string.common_refresh))
                    }
                    if (onOpenPlans != null) IconButton(onClick = onOpenPlans) {
                        Icon(androidx.compose.ui.res.painterResource(R.drawable.nola_calendar), stringResource(R.string.scheduled_title))
                    }
                }
            }
            item {
                OutlinedTextField(
                    value = searchDraft, onValueChange = { searchDraft = it },
                    modifier = Modifier.fillMaxWidth(), singleLine = true,
                    placeholder = { Text(stringResource(R.string.activity_search_title), maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    leadingIcon = { Icon(painterResource(R.drawable.nola_search), null, Modifier.size(20.dp)) },
                    trailingIcon = if (searchDraft.isNotEmpty()) { {
                        IconButton(onClick = { searchDraft = ""; search = "" }) {
                            Icon(Icons.Filled.Close, stringResource(R.string.notifications_clear_search))
                        }
                    } } else null,
                    shape = RoundedCornerShape(12.dp),
                    colors = OutlinedTextFieldDefaults.colors(
                        unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                        focusedContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                        unfocusedBorderColor = Color.Transparent,
                        focusedBorderColor = MaterialTheme.colorScheme.primary,
                    ),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { submitSearch() }),
                )
            }
            item {
                val colors = FilterChipDefaults.filterChipColors(
                    selectedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                    selectedLabelColor = MaterialTheme.colorScheme.onPrimaryContainer,
                )
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    listOf(null to R.string.activity_all, "agent" to R.string.activity_agent, "automation" to R.string.activity_plan).forEach { (value, label) ->
                        FilterChip(selected = source == value, onClick = { source = value }, colors = colors, border = null,
                            shape = MaterialTheme.shapes.extraLarge, label = { Text(stringResource(label)) })
                    }
                    FilterChip(selected = needsResponse, onClick = { needsResponse = !needsResponse }, colors = colors, border = null,
                        shape = MaterialTheme.shapes.extraLarge, label = { Text(stringResource(R.string.activity_needs_response)) })
                }
            }
            if (state.hasLoaded && state.entries.isEmpty()) item {
                val filtered = needsResponse || source != null || search.isNotBlank()
                NolaEmptyState(
                    icon = if (filtered) Icons.Filled.Search else Icons.Filled.History,
                    title = stringResource(if (filtered) R.string.activity_empty_filter else R.string.activity_empty),
                )
            }
            items(state.entries.filter { it.needs_response } + state.entries.filter { !it.needs_response }, key = { it.run_id }) { entry ->
                Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable { viewModel.open(entry) }.padding(vertical = 14.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(entry.title, maxLines = 2, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.titleMedium.copy(fontSize = 16.sp, lineHeight = 24.sp))
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        if (entry.needs_response) {
                            NolaBadge(
                                text = stringResource(R.string.activity_needs_response),
                                tint = NolaColors.warning,
                            )
                        }
                        NolaBadge(
                            text = stringResource(activityStatus(entry.status)),
                            tint = activityStatusTint(entry.status),
                        )
                        NolaBadge(
                            text = stringResource(activitySource(entry.source)),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Text(DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(entry.created_at_ms)),
                        style = MaterialTheme.typography.labelSmall.copy(fontSize = 12.sp, lineHeight = 18.sp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
            }
            if (state.failed) item {
                Text(stringResource(R.string.activity_read_failed), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = viewModel::retry) { Text(stringResource(R.string.common_retry)) }
            }
            if (state.loading || (!state.hasLoaded && !state.failed)) item { CircularProgressIndicator(Modifier.size(24.dp)) }
            if (state.cursor != null && !state.loading && !state.failed) item {
                TextButton(onClick = viewModel::loadMore) { Text(stringResource(R.string.activity_load_more)) }
            }
        }
    }
    detail?.let { selected ->
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        val reduceMotion = rememberReduceMotionEnabled()
        var showTechnicalDetails by remember(selected.entry.run_id) { mutableStateOf(false) }
        ModalBottomSheet(
            onDismissRequest = viewModel::closeDetail,
            sheetState = sheetState,
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .fillMaxHeight(0.88f)
                    .navigationBarsPadding()
                    .padding(horizontal = 20.dp),
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth().padding(bottom = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        selected.detail?.entry?.title ?: selected.entry.title,
                        style = MaterialTheme.typography.titleLarge,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(onClick = viewModel::closeDetail) {
                        Icon(Icons.Filled.Close, contentDescription = stringResource(R.string.common_close))
                    }
                }
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (selected.loading) {
                        CircularProgressIndicator(Modifier.size(24.dp))
                    } else if (selected.failed) {
                        Text(stringResource(R.string.activity_read_failed), color = MaterialTheme.colorScheme.error)
                        TextButton(onClick = { viewModel.open(selected.entry) }) { Text(stringResource(R.string.common_retry)) }
                    } else selected.detail?.let { value ->
                        FlowRow(
                            horizontalArrangement = Arrangement.spacedBy(6.dp),
                            verticalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            if (value.entry.needs_response) {
                                NolaBadge(text = stringResource(R.string.activity_needs_response), tint = NolaColors.warning)
                            }
                            NolaBadge(text = stringResource(activityStatus(value.entry.status)), tint = activityStatusTint(value.entry.status))
                            NolaBadge(text = stringResource(activitySource(value.entry.source)), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        Text(
                            DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(value.entry.created_at_ms)),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        value.explanation?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                        value.result_text?.takeIf { it.isNotBlank() }?.let { MarkdownRenderer(it) }
                        if (value.result_has_more) {
                            Text(stringResource(R.string.activity_result_more), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        if (value.source_conversation_id != null && value.review_in_source) {
                            Text(stringResource(R.string.activity_review_source), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        if (value.source_conversation_id == null) {
                            Text(stringResource(R.string.activity_source_unavailable), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        NolaDisclosureBar(
                            title = stringResource(R.string.activity_technical_details),
                            expanded = showTechnicalDetails,
                            onToggle = { showTechnicalDetails = !showTechnicalDetails },
                        )
                        AnimatedVisibility(
                            visible = showTechnicalDetails,
                            enter = nolaDisclosureEnter(reduceMotion),
                            exit = nolaDisclosureExit(reduceMotion),
                        ) {
                            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                Text(stringResource(R.string.activity_execution_boundary), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                Text(stringResource(R.string.activity_execution_host), style = MaterialTheme.typography.labelMedium)
                                Text(value.execution_host_id ?: stringResource(R.string.activity_host_unknown), style = MaterialTheme.typography.bodySmall)
                                Text(stringResource(R.string.activity_context_scope), style = MaterialTheme.typography.labelMedium)
                                Text(
                                    stringResource(when (value.context_scope) {
                                        "task" -> R.string.activity_context_task
                                        "conversation" -> R.string.activity_context_conversation
                                        else -> R.string.activity_context_unavailable
                                    }),
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                        }
                        Spacer(Modifier.height(12.dp))
                    } ?: Text(stringResource(R.string.activity_unavailable))
                }
                val sourceConvId = selected.detail?.source_conversation_id
                if (sourceConvId != null) {
                    HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
                    Box(modifier = Modifier.fillMaxWidth().padding(vertical = 12.dp)) {
                        NolaPrimaryButton(
                            text = stringResource(R.string.activity_open_source),
                            onClick = { viewModel.closeDetail(); onOpenConversation(sourceConvId) },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun activityStatusTint(status: String): Color = when (status) {
    "answered", "completed", "action_completed" -> NolaColors.success
    "running" -> MaterialTheme.colorScheme.primary
    "waiting_input", "blocked", "incomplete" -> NolaColors.warning
    "failed" -> MaterialTheme.colorScheme.error
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

private fun activitySource(source: String): Int = when (source) {
    "agent" -> R.string.activity_agent
    "automation" -> R.string.activity_plan
    else -> R.string.tab_chat
}

private fun activityStatus(status: String): Int = when (status) {
    "running" -> R.string.activity_status_running
    "waiting_input" -> R.string.activity_status_waiting
    "answered", "completed" -> R.string.activity_status_completed
    "action_completed" -> R.string.activity_status_action_completed
    "delegation_accepted" -> R.string.activity_status_delegated
    "queued" -> R.string.activity_status_queued
    "cancelled" -> R.string.activity_status_cancelled
    "blocked" -> R.string.activity_status_blocked
    "incomplete" -> R.string.activity_status_incomplete
    "failed" -> R.string.chat_host_tool_status_failed
    else -> R.string.activity_status_other
}
