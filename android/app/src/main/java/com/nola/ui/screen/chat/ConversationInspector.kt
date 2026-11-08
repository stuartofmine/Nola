package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Checklist
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.data.repository.ConversationProgress
import com.nola.data.repository.ConversationProgressRepository
import com.nola.data.repository.ConversationProgressItem
import com.nola.ui.components.NolaTopAppBar
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*
import androidx.compose.runtime.saveable.rememberSaveable
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal enum class ConversationInspector(val label: Int) {
    Progress(R.string.inspector_progress), Resources(R.string.inspector_resources), Settings(R.string.inspector_settings)
}

internal fun toggleInspector(current: ConversationInspector?, target: ConversationInspector): ConversationInspector? = target.takeUnless { it == current }

@Composable
internal fun ConversationInspectorButtons(selected: ConversationInspector?, select: (ConversationInspector) -> Unit) {
    Row {
        ConversationInspector.entries.forEach { item ->
            IconButton(
                onClick = { select(item) },
                modifier = Modifier.size(48.dp).semantics { this.selected = selected == item },
                colors = IconButtonDefaults.iconButtonColors(
                    containerColor = if (selected == item) MaterialTheme.colorScheme.primaryContainer else androidx.compose.ui.graphics.Color.Transparent
                )
            ) {
                Icon(when(item) {
                    ConversationInspector.Progress -> Icons.Filled.Checklist
                    ConversationInspector.Resources -> Icons.Filled.FolderOpen
                    ConversationInspector.Settings -> Icons.Filled.Tune
                }, stringResource(item.label), Modifier.size(20.dp),
                    tint = if (selected == item) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
internal fun ConversationProgressPanel(state: ChatUiState, load: suspend (String) -> ConversationProgress,
    pendingControl: suspend (String) -> Boolean,
    submitControl: suspend (String, JsonObject?) -> ConversationProgress, onClose: () -> Unit) {
    val owner = state.conversationId
    val host = state.executionHostBinding
    var value by remember(owner, host) { mutableStateOf<ConversationProgress?>(null) }
    var failed by remember(owner, host) { mutableStateOf(false) }
    var loading by remember(owner, host) { mutableStateOf(owner != null) }
    val scope = rememberCoroutineScope()
    var objective by rememberSaveable(owner) { mutableStateOf("") }
    var pending by remember(owner, host) { mutableStateOf(false) }
    var submitting by remember(owner, host) { mutableStateOf(false) }
    var controlError by remember(owner, host) { mutableStateOf<String?>(null) }
    fun submit(action: String?) {
        val capturedOwner = owner ?: return
        val observed = value?.goalControl
        val control = action?.let {
            com.nola.data.local.remoteGoalControlInput(java.util.UUID.randomUUID().toString(), it,
                observed?.goalId, observed?.controlToken, if (it == "start") objective else "")
        }
        submitting = true; controlError = null
        scope.launch {
            try { value = submitControl(capturedOwner, control) }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) { controlError = error.message }
            finally { pending = pendingControl(capturedOwner); submitting = false }
        }
    }
    var refresh by remember(owner, host) { mutableIntStateOf(0) }
    LaunchedEffect(owner, host, refresh, state.agentCoreSnapshot?.iteration, state.coreTaskProjection.revision) {
        value = null; failed = false; loading = owner != null
        if (owner != null) {
            try {
                value = load(owner)
                pending = pendingControl(owner)
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (_: Exception) { failed = true }
        }
        loading = false
    }
    Column(Modifier.fillMaxSize()) {
        NolaTopAppBar(title = stringResource(R.string.inspector_progress), onBack = onClose) {
            IconButton(onClick = { refresh++ }, enabled = !loading) { Icon(Icons.Filled.Refresh, stringResource(R.string.common_refresh)) }
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (failed) {
                Text(stringResource(R.string.inspector_progress_unavailable), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = { refresh++ }) { Text(stringResource(R.string.common_refresh)) }
            }
            value?.let { progress ->
                if (host != null || state.currentModel?.source != com.nola.domain.model.ModelInfo.ModelSource.WebHost) {
                    controlError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                    if (pending) {
                        Text(stringResource(R.string.goal_operation_pending))
                        TextButton(onClick = { submit(null) }, enabled = !submitting) {
                            Text(stringResource(R.string.goal_operation_retry))
                        }
                    } else if (progress.status == null || progress.status in setOf("completed", "cancelled")) {
                        OutlinedTextField(value = objective, onValueChange = { objective = it },
                            label = { Text(stringResource(R.string.goal_objective)) }, modifier = Modifier.fillMaxWidth())
                        Button(onClick = { submit("start") }, enabled = !submitting && objective.isNotBlank()) {
                            Text(stringResource(R.string.goal_start))
                        }
                    } else {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            if (progress.goalControl?.canPause == true) TextButton(onClick = { submit("pause") }, enabled = !submitting) { Text(stringResource(R.string.goal_pause)) }
                            if (progress.goalControl?.canResume == true) TextButton(onClick = { submit("resume") }, enabled = !submitting) { Text(stringResource(R.string.goal_resume)) }
                            if (progress.goalControl?.canCancel == true) TextButton(onClick = { submit("cancel") }, enabled = !submitting) { Text(stringResource(R.string.goal_cancel)) }
                        }
                    }
                }
                progress.objective?.let { Text(it, style = MaterialTheme.typography.titleMedium) }
                progress.status?.let { Text(it, style = MaterialTheme.typography.labelMedium) }
                if (progress.requirements.isEmpty() && progress.todos.isEmpty()) Text(stringResource(R.string.inspector_no_plan), color = MaterialTheme.colorScheme.onSurfaceVariant)
                ProgressItems(stringResource(R.string.inspector_requirements), progress.requirements)
                ProgressItems(stringResource(R.string.inspector_plan), progress.todos)
            }
            if (owner == null) Text(stringResource(R.string.inspector_no_plan))
            state.coreTaskProjection.orderedTasks.filter { owner != null && it.ownerSessionId == state.coreTaskProjection.sessionId }.forEach { task ->
                HorizontalDivider()
                Text(task.currentStep ?: task.id, style = MaterialTheme.typography.bodyMedium)
                Text(task.state.wireValue, style = MaterialTheme.typography.labelMedium)
            }
        }
    }
}

@Composable
internal fun ProgressItems(title: String, entries: List<ConversationProgressItem>) {
    if (entries.isEmpty()) return
    Text(title, style = MaterialTheme.typography.titleSmall)
    Text(stringResource(R.string.inspector_completed_count, entries.count { it.status == "completed" }, entries.size))
    entries.forEach { item ->
        Column { Text(item.text, style = MaterialTheme.typography.bodyLarge); Text(item.status, style = MaterialTheme.typography.labelMedium) }
    }
}

@Composable
internal fun ConversationResourcesPanel(state: ChatUiState, onClose: () -> Unit) {
    val messages = state.messages.filter { it.conversationId == state.conversationId }
    val parts = messages.flatMap { it.canonicalParts }.filter {
        it is com.nola.domain.projection.ConversationProjectionV1.Part.Source ||
            it is com.nola.domain.projection.ConversationProjectionV1.Part.Citation ||
            it is com.nola.domain.projection.ConversationProjectionV1.Part.Attachment
    }
    val sources = messages.flatMap { com.nola.domain.model.decodeAnswerSourcesFromMeta(it.meta) }.distinctBy { it.id }
    val attachments = messages.flatMap { decodeMessageAttachments(it.meta) }.distinctBy { it.uri }
    Column(Modifier.fillMaxSize()) {
        NolaTopAppBar(title = stringResource(R.string.inspector_resources), onBack = onClose)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (parts.isEmpty() && sources.isEmpty() && attachments.isEmpty()) Text(stringResource(R.string.inspector_no_resources), color = MaterialTheme.colorScheme.onSurfaceVariant)
            messages.forEach { message -> key(message.conversationId, message.id) {
                val messageParts = message.canonicalParts.filter {
                    it is com.nola.domain.projection.ConversationProjectionV1.Part.Source ||
                        it is com.nola.domain.projection.ConversationProjectionV1.Part.Citation ||
                        it is com.nola.domain.projection.ConversationProjectionV1.Part.Attachment
                }
                CanonicalPartReaderBoundary(message.conversationId, message.id, messageParts) { onReadText ->
                    CanonicalSourceCards(messageParts, onReadText)
                }
            } }
            if (sources.isNotEmpty()) AnswerSourcesList(sources)
            attachments.forEach { attachment ->
                Text(attachment.name, style = MaterialTheme.typography.bodyMedium)
                Text(attachment.sizeLabel, style = MaterialTheme.typography.labelMedium)
            }
            state.dataSources.filter { it.id in state.selectedDataSourceIds }.forEach { source ->
                Text(source.name, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}
