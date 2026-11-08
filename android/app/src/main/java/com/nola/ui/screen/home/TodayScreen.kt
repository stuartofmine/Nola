// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.home

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nola.ui.components.nolaPageGutter
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.local.TodayConversationFact
import java.text.DateFormat
import java.util.Date

/** Core-authored facts from one snapshot; no client-made daily report or authorization. */
@Composable
internal fun TodayScreen(
    onOpenWorkspace: (Int) -> Unit,
    onOpenConversations: () -> Unit,
    onOpenConversation: (String) -> Unit,
    responses: TodayPendingResponsesViewModel = hiltViewModel(),
) {
    val state by responses.state.collectAsStateWithLifecycle()
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { now = System.currentTimeMillis(); responses.refresh() }
    val date = remember(now) { DateFormat.getDateInstance(DateFormat.FULL).format(Date(now)) }
    val snapshot = state.snapshot
    LazyColumn(
        modifier = Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).statusBarsPadding().testTag("workspace.today"),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item {
            Column(Modifier.fillMaxWidth().padding(horizontal = nolaPageGutter(), vertical = 24.dp)) {
                Text(date, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Text(stringResource(R.string.today_title), style = MaterialTheme.typography.headlineMedium.copy(fontSize = 28.sp, lineHeight = 36.sp),
                    fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 8.dp))
                Text(stringResource(R.string.today_core_description), style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
            }
        }
        item {
            Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                TextButton(onClick = onOpenConversations, modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.tab_chat)) }
                TextButton(onClick = { onOpenWorkspace(0) }, modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.today_manage)) }
                Spacer(Modifier.weight(1f))
                TextButton(onClick = responses::refresh, enabled = !state.loading, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.common_refresh))
                }
            }
        }
        if (state.loading) item { CircularProgressIndicator(Modifier.padding(horizontal = nolaPageGutter()).testTag("today.loading")) }
        if (state.failed) item {
            Text(stringResource(if (snapshot == null) R.string.today_waiting_error else R.string.today_refresh_error),
                color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = nolaPageGutter(), vertical = 8.dp).testTag("today.error"))
        }
        // A failed/unfinished first read is never rendered as a successful empty list.
        if (snapshot != null) {
            if (snapshot.needsAttention) item {
                Column(Modifier.padding(horizontal = nolaPageGutter(), vertical = 8.dp)) {
                    Text(stringResource(R.string.today_expert_attention), color = MaterialTheme.colorScheme.error)
                    snapshot.unrepresentedExpertContexts.forEach { context ->
                        Text(context.displayName, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            item { TodaySectionTitle(stringResource(R.string.today_waiting)) }
            val waiting = snapshot.conversations.filter { it.needsResponse }
            if (waiting.isEmpty() && !snapshot.needsAttention) item { TodayEmpty(stringResource(R.string.today_core_waiting_empty)) }
            items(waiting, key = { "waiting:${it.conversationId}" }) { fact ->
                TodayConversationRow(fact, onOpenConversation = onOpenConversation)
            }
            if (snapshot.hasMoreConversations) item {
                Column(Modifier.padding(horizontal = nolaPageGutter())) {
                    Text(stringResource(R.string.today_more_conversations), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("today.moreConversations"))
                    TextButton(onClick = onOpenConversations, modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.tab_chat)) }
                }
            }
            item { TodaySectionTitle(stringResource(R.string.today_next)) }
            if (snapshot.queuedInstances.isEmpty()) item { TodayEmpty(stringResource(R.string.today_no_queued_instances)) }
            items(snapshot.queuedInstances, key = { "queued:${it.occurrenceId}" }) { queued ->
                Column(Modifier.fillMaxWidth().padding(horizontal = nolaPageGutter(), vertical = 12.dp)) {
                    Text(queued.title?.takeIf(String::isNotBlank) ?: stringResource(R.string.today_queued_instance), style = MaterialTheme.typography.titleMedium)
                    Text(DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(queued.scheduledForMs)),
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(stringResource(R.string.today_queued_status), style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(onClick = { onOpenWorkspace(2) }, modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.today_view_plan)) }
                }
            }
            if (snapshot.hasMoreQueuedInstances) item {
                Column(Modifier.padding(horizontal = nolaPageGutter())) {
                    Text(stringResource(R.string.today_more_queued_instances), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("today.moreQueued"))
                    TextButton(onClick = { onOpenWorkspace(2) }, modifier = Modifier.heightIn(min = 48.dp)) { Text(stringResource(R.string.today_view_plan)) }
                }
            }
            item { TodaySectionTitle(stringResource(R.string.today_recent_conversations)) }
            val recent = snapshot.recentConversations.filterNot { it.needsResponse }
            if (recent.isEmpty()) item { TodayEmpty(stringResource(R.string.today_no_recent_conversations)) }
            items(recent, key = { "recent:${it.conversationId}" }) { fact ->
                TodayConversationRow(fact, onOpenConversation = onOpenConversation)
            }
        }
        item { Spacer(Modifier.height(24.dp)) }
    }
}

@Composable
private fun TodayConversationRow(fact: TodayConversationFact, onOpenConversation: (String) -> Unit) {
    Column(Modifier.fillMaxWidth().heightIn(min = 64.dp).clickable { onOpenConversation(fact.conversationId) }
        .padding(horizontal = nolaPageGutter(), vertical = 12.dp).testTag("today.conversation.${fact.conversationId}")) {
        Text(fact.title.ifBlank { stringResource(R.string.tab_chat) }, style = MaterialTheme.typography.titleMedium)
        val experts = fact.expertContexts.map { it.displayName }.distinct()
        if (experts.isNotEmpty()) Text(experts.joinToString(" · "),
            style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (fact.needsResponse) Text(stringResource(R.string.today_core_needs_response), style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(fact.updatedAtMs)),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        TextButton(onClick = { onOpenConversation(fact.conversationId) }, modifier = Modifier.heightIn(min = 48.dp)) {
            Text(stringResource(R.string.today_open_conversation))
        }
    }
}

@Composable
private fun TodaySectionTitle(title: String) {
    Text(title, style = MaterialTheme.typography.titleLarge.copy(fontSize = 20.sp, lineHeight = 28.sp), fontWeight = FontWeight.SemiBold,
        modifier = Modifier.fillMaxWidth().padding(start = nolaPageGutter(), end = nolaPageGutter(), top = 24.dp, bottom = 4.dp))
}

@Composable
private fun TodayEmpty(text: String) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.fillMaxWidth().padding(horizontal = nolaPageGutter(), vertical = 12.dp))
}
