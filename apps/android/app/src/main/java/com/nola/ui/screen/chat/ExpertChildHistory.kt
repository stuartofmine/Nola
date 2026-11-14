// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.nola.R
import com.nola.data.repository.ExpertChildConversation
import com.nola.data.repository.ExpertChildConversationPage
import com.nola.domain.model.ConversationMessage
import com.nola.domain.projection.CanonicalConversationProjector
import com.nola.domain.projection.ConversationProjectionWindowV1
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

internal data class ExpertHistoryWindow(val window: ConversationProjectionWindowV1, val senders: Map<String, com.nola.data.repository.ExpertHistorySender>)

internal data class ExpertHistoryTarget(val parentId: String, val roomId: String, val sourceIdentity: String, val child: ExpertChildConversation)

/** Lazy observations are disposed with the exact parent row, never shared across Hosts. */
@Composable
internal fun ExpertChildTree(parentId: String, load: suspend (Int) -> ExpertChildConversationPage, onOpen: (ExpertHistoryTarget) -> Unit) {
    var expanded by remember(parentId) { mutableStateOf(false) }
    var page by remember(parentId) { mutableStateOf<ExpertChildConversationPage?>(null) }
    var children by remember(parentId) { mutableStateOf(emptyList<ExpertChildConversation>()) }
    var loading by remember(parentId) { mutableStateOf(false) }
    var failed by remember(parentId) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    fun fetch(reset: Boolean = false) {
        if (loading) return
        val offset = if (reset) 0 else page?.nextOffset ?: if (page == null) 0 else return
        loading = true; failed = false
        scope.launch {
            try {
                val incoming = load(offset).validated(parentId, offset)
                check(reset || incoming.children.none { child -> children.any { it.conversationId == child.conversationId } })
                check(page == null || incoming.roomId == page?.roomId && incoming.sourceIdentity == page?.sourceIdentity)
                children = if (reset) incoming.children else children + incoming.children; page = incoming
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { failed = true }
            finally { loading = false }
        }
    }
    Column(Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp)) {
        TextButton(onClick = { expanded = !expanded; if (expanded && page == null) fetch() },
            modifier = Modifier.testTag("chat.expertHistory.$parentId").fillMaxWidth(), contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp)) {
            Text(stringResource(R.string.expert_child_history), Modifier.weight(1f), color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(if (expanded) "−" else "+")
        }
        if (expanded) {
            children.forEach { child ->
                TextButton(onClick = { page?.roomId?.let { onOpen(ExpertHistoryTarget(parentId, it, page!!.sourceIdentity, child)) } },
                    enabled = page?.roomId != null, modifier = Modifier.fillMaxWidth().testTag("chat.expertChild.${child.conversationId}"),
                    contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
                    Column(Modifier.fillMaxWidth()) {
                        Text(child.title.ifBlank { stringResource(R.string.expert_child_history) })
                        Text(stringResource(if (child.isCurrent) R.string.expert_child_current else R.string.expert_child_archived) + " · ${child.messageCount}",
                            style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            if (failed) Text(stringResource(R.string.expert_child_load_failed), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth())
            else if (failed || page == null || page?.hasMore == true) TextButton(onClick = { fetch(reset = failed) }) { Text(stringResource(if (failed) R.string.expert_child_retry else R.string.expert_child_more)) }
            else if (children.isEmpty()) Text(stringResource(R.string.expert_child_empty), Modifier.padding(12.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ExpertChildHistoryDialog(target: ExpertHistoryTarget,
    load: suspend (Long?) -> ExpertHistoryWindow, onContinue: suspend () -> Unit, onClose: () -> Unit) {
    var messages by remember(target) { mutableStateOf(emptyList<ConversationMessage>()) }
    var window by remember(target) { mutableStateOf<ConversationProjectionWindowV1?>(null) }
    var senders by remember(target) { mutableStateOf(emptyMap<String, com.nola.data.repository.ExpertHistorySender>()) }
    var loading by remember(target) { mutableStateOf(false) }
    var error by remember(target) { mutableStateOf<String?>(null) }
    var continuing by remember(target) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val failureText = stringResource(R.string.expert_child_load_failed)
    suspend fun read(reset: Boolean = false) {
        if (loading) return
        loading = true; error = null
        val before = if (reset) null else window?.beforeOrdinal
        try {
            val result = load(before)
            val incoming = result.window
            if (before != null) check(incoming.projection.revision == window?.projection?.revision && incoming.eventSequence == window?.eventSequence)
            val incomingMessages = CanonicalConversationProjector.project(incoming.projection)
            messages = if (before == null) incomingMessages else incomingMessages + messages.filter { old -> incomingMessages.none { it.id == old.id } }
            senders = if (before == null) result.senders else senders + result.senders
            window = incoming
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { error = failureText }
        finally { loading = false }
    }
    LaunchedEffect(target) { read() }
    Dialog(onDismissRequest = { if (!continuing) onClose() }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Scaffold(modifier = Modifier.safeDrawingPadding(), topBar = {
                Column {
                    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text(target.child.title, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                        TextButton(enabled = !continuing && !loading, onClick = { scope.launch { read(reset = true) } }) { Text(stringResource(R.string.common_refresh)) }
                        TextButton(enabled = !continuing, onClick = onClose) { Text(stringResource(R.string.expert_child_close)) }
                    }
                    Text(stringResource(R.string.expert_child_read_only), Modifier.padding(horizontal = 24.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (target.child.active && target.child.isCurrent && target.child.agentRevision != null) TextButton(enabled = !continuing && !loading && window != null && error == null,
                        onClick = { continuing = true; scope.launch {
                            try { onContinue(); onClose() } catch (cancelled: CancellationException) { throw cancelled }
                            catch (failure: Exception) { error = failure.localizedMessage ?: failureText } finally { continuing = false }
                        } }, modifier = Modifier.padding(horizontal = 12.dp)) { Text(stringResource(R.string.expert_child_continue)) }
                }
            }) { padding ->
                LazyColumn(Modifier.fillMaxSize().padding(padding).testTag("chat.expertHistory.transcript"), contentPadding = PaddingValues(12.dp)) {
                    item {
                        if (loading || continuing) LinearProgressIndicator(Modifier.fillMaxWidth())
                        error?.let { Text(it, color = MaterialTheme.colorScheme.error); TextButton(enabled = !loading, onClick = { scope.launch { read(reset = true) } }) { Text(stringResource(R.string.expert_child_retry)) } }
                        if (window?.hasMore == true && error == null) TextButton(enabled = !loading, onClick = { scope.launch { read() } }) { Text(stringResource(R.string.expert_child_more)) }
                        if (!loading && error == null && window != null && messages.isEmpty()) Text(stringResource(R.string.expert_child_empty))
                    }
                    items(messages, key = { it.id }) { message -> Column {
                        if (senders.containsKey(message.id)) Text(senders[message.id]?.name.orEmpty().ifBlank { stringResource(R.string.chat_experts) },
                            Modifier.padding(horizontal = 12.dp, vertical = 4.dp), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        MessageBubble(message = message, readOnlyHistory = true)
                    } }
                }
            }
        }
    }
}
