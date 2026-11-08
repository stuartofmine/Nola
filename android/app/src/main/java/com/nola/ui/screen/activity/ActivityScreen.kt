// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.activity

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.unit.sp
import com.nola.ui.components.nolaPageGutter
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.ui.components.MarkdownRenderer
import java.text.DateFormat
import java.util.Date

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
    LifecycleResumeEffect(source, needsResponse, search) {
        refresh()
        onPauseOrDispose { }
    }
    Box(Modifier.fillMaxSize().imePadding()) {
        LazyColumn(Modifier.align(Alignment.TopCenter).widthIn(max = 720.dp).fillMaxSize(),
            contentPadding = PaddingValues(horizontal = gutter, vertical = 24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)) {
            item {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.activity_title), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                    if (onOpenPlans != null) IconButton(onClick = onOpenPlans) {
                        Icon(androidx.compose.ui.res.painterResource(R.drawable.nola_calendar), stringResource(R.string.scheduled_title))
                    }
                }
            }
            item { Text(stringResource(R.string.activity_local_scope), style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant) }
            item {
                OutlinedTextField(value = searchDraft, onValueChange = { searchDraft = it },
                    modifier = Modifier.fillMaxWidth(), singleLine = true,
                    label = { Text(stringResource(R.string.activity_search_title)) },
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { submitSearch() }),
                    trailingIcon = { TextButton(onClick = ::submitSearch) { Text(stringResource(R.string.activity_search)) } })
            }
            item {
                Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf(null to R.string.activity_all, "agent" to R.string.activity_agent, "automation" to R.string.activity_plan).forEach { (value, label) ->
                        FilterChip(selected = source == value, onClick = { source = value }, label = { Text(stringResource(label)) })
                    }
                }
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween) {
                    FilterChip(selected = needsResponse, onClick = { needsResponse = !needsResponse },
                        label = { Text(stringResource(R.string.activity_needs_response)) })
                    TextButton(onClick = ::refresh, enabled = !state.loading) { Text(stringResource(R.string.common_refresh)) }
                }
            }
            if (state.failed) item {
                Text(stringResource(R.string.activity_read_failed), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = { if (state.entries.isEmpty()) refresh() else viewModel.loadMore() }) { Text(stringResource(R.string.common_retry)) }
            }
            if (state.entries.isEmpty() && !state.loading && !state.failed) item {
                Text(stringResource(if (needsResponse || source != null || search.isNotBlank()) R.string.activity_empty_filter else R.string.activity_empty),
                    color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            items(state.entries.filter { it.needs_response } + state.entries.filter { !it.needs_response }, key = { it.run_id }) { entry ->
                Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable { viewModel.open(entry) }.padding(vertical = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(entry.title, maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.titleMedium.copy(fontSize = 16.sp, lineHeight = 24.sp))
                    Text(stringResource(activitySource(entry.source)) + " · " + stringResource(activityStatus(entry.status)),
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(entry.created_at_ms)),
                        style = MaterialTheme.typography.labelSmall.copy(fontSize = 12.sp, lineHeight = 18.sp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (entry.needs_response) Text(stringResource(R.string.activity_needs_response), style = MaterialTheme.typography.labelMedium)
                }
                HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
            }
            if (state.loading) item { CircularProgressIndicator(Modifier.size(24.dp)) }
            if (state.cursor != null && !state.loading) item {
                TextButton(onClick = viewModel::loadMore) { Text(stringResource(R.string.activity_load_more)) }
            }
        }
    }
    detail?.let { selected ->
        AlertDialog(onDismissRequest = viewModel::closeDetail,
            title = { Text(selected.detail?.entry?.title ?: selected.entry.title) },
            text = {
                Column(Modifier.heightIn(max = 480.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    if (selected.loading) CircularProgressIndicator(Modifier.size(24.dp))
                    else if (selected.failed) {
                        Text(stringResource(R.string.activity_read_failed), color = MaterialTheme.colorScheme.error)
                        TextButton(onClick = { viewModel.open(selected.entry) }) { Text(stringResource(R.string.common_retry)) }
                    } else selected.detail?.let { value ->
                        Text(stringResource(R.string.activity_execution), style = MaterialTheme.typography.titleSmall)
                        Text(stringResource(R.string.activity_execution_boundary), style = MaterialTheme.typography.bodySmall)
                        Text(stringResource(R.string.activity_execution_host), style = MaterialTheme.typography.labelMedium)
                        Text(value.execution_host_id ?: stringResource(R.string.activity_host_unknown))
                        Text(stringResource(R.string.activity_context_scope), style = MaterialTheme.typography.labelMedium)
                        Text(stringResource(when (value.context_scope) {
                            "task" -> R.string.activity_context_task
                            "conversation" -> R.string.activity_context_conversation
                            else -> R.string.activity_context_unavailable
                        }), style = MaterialTheme.typography.bodySmall)
                        Text(stringResource(activitySource(value.entry.source)) + " · " + stringResource(activityStatus(value.entry.status)))
                        Text(DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(value.entry.created_at_ms)))
                        value.explanation?.let { Text(it) }
                        value.result_text?.takeIf { it.isNotBlank() }?.let { MarkdownRenderer(it) }
                        if (value.result_has_more) Text(stringResource(R.string.activity_result_more), style = MaterialTheme.typography.bodySmall)
                        if (value.source_conversation_id != null) {
                            if (value.review_in_source) Text(stringResource(R.string.activity_review_source), style = MaterialTheme.typography.bodySmall)
                            TextButton(onClick = { viewModel.closeDetail(); onOpenConversation(value.source_conversation_id) }) {
                                Text(stringResource(R.string.activity_open_source))
                            }
                        } else Text(stringResource(R.string.activity_source_unavailable))
                    } ?: Text(stringResource(R.string.activity_unavailable))
                }
            },
            confirmButton = { TextButton(onClick = viewModel::closeDetail) { Text(stringResource(R.string.common_close)) } })
    }
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
