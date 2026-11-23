package com.nola.ui.screen.chat

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Checklist
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.RadioButtonUnchecked
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Schedule
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.data.repository.ConversationProgress
import com.nola.data.repository.ConversationProgressRepository
import com.nola.data.repository.ConversationProgressItem
import com.nola.ui.components.NolaBadge
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaSectionTitle
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

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun ConversationProgressPanel(
    state: ChatUiState,
    load: suspend (String) -> ConversationProgress,
    pendingControl: suspend (String) -> Boolean,
    submitControl: suspend (String, JsonObject?) -> ConversationProgress,
    onClose: () -> Unit,
    discardControl: suspend (String) -> Unit = {},
    expertChildren: @Composable (String) -> Unit = {}
) {
    val owner = state.conversationId
    val host = state.executionHostBinding
    var value by remember(owner, host) { mutableStateOf<ConversationProgress?>(null) }
    var failed by remember(owner, host) { mutableStateOf(false) }
    var loading by remember(owner, host) { mutableStateOf(owner != null) }
    val scope = rememberCoroutineScope()
    var objective by rememberSaveable(owner) { mutableStateOf("") }
    var constraintsText by rememberSaveable(owner) { mutableStateOf("") }
    var pending by remember(owner, host) { mutableStateOf(false) }
    var submitting by remember(owner, host) { mutableStateOf(false) }
    var cancelRequest by remember(owner, host) { mutableStateOf<JsonObject?>(null) }
    val operationFailedMessage = stringResource(R.string.goal_operation_failed)
    var controlError by remember(owner, host) { mutableStateOf<String?>(null) }
    fun submit(action: String?, prepared: JsonObject? = null) {
        if (submitting || (action == "start" && objective.isBlank())) return
        val capturedOwner = owner ?: return
        val observed = value?.goalControl?.takeUnless { action == "start" }
        val parsedConstraints = if (action == "start") {
            constraintsText.split('\n', ',').map { it.trim() }.filter { it.isNotEmpty() }
        } else {
            emptyList()
        }
        val control = prepared ?: action?.let {
            com.nola.data.local.remoteGoalControlInput(java.util.UUID.randomUUID().toString(), it,
                observed?.goalId, observed?.controlToken, if (it == "start") objective else "", parsedConstraints)
        }
        submitting = true; controlError = null
        scope.launch {
            try { value = submitControl(capturedOwner, control) }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) { controlError = error.message?.takeIf { it.isNotBlank() } ?: operationFailedMessage }
            finally {
                try { pending = pendingControl(capturedOwner) }
                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (_: Exception) { pending = true; controlError = operationFailedMessage }
                finally { submitting = false }
            }
        }
    }
    var refresh by remember(owner, host) { mutableIntStateOf(0) }
    LaunchedEffect(owner, host, refresh, state.agentCoreSnapshot?.iteration, state.coreTaskProjection.revision) {
        failed = false; loading = owner != null
        if (owner != null) {
            try {
                value = load(owner)
                pending = pendingControl(owner)
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (_: Exception) { failed = true }
        } else {
            value = null
        }
        loading = false
    }
    // A paired Host may acknowledge the command before its worker applies it.
    // Follow the original receipt while this panel is visible, without sending another control.
    LaunchedEffect(owner, host, pending, submitting) {
        if (owner == null || host == null || !pending || submitting) return@LaunchedEffect
        for (delayMillis in listOf(500L, 1_000L, 2_000L, 4_000L, 6_000L, 8_000L, 10_000L, 12_000L, 15_000L)) {
            kotlinx.coroutines.delay(delayMillis)
            try {
                value = load(owner)
                pending = pendingControl(owner)
                if (!pending) {
                    controlError = null
                    return@LaunchedEffect
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                pending = try { pendingControl(owner) }
                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (_: Exception) { true }
                controlError = failure.message ?: operationFailedMessage
                return@LaunchedEffect
            }
        }
    }
    cancelRequest?.let { observed ->
        AlertDialog(onDismissRequest = { cancelRequest = null },
            title = { Text(stringResource(R.string.goal_cancel)) },
            text = { Text(stringResource(R.string.goal_cancel_explanation)) },
            confirmButton = { TextButton(onClick = { cancelRequest = null; submit("cancel", observed) }) { Text(stringResource(R.string.goal_cancel), color = MaterialTheme.colorScheme.error) } },
            dismissButton = { TextButton(onClick = { cancelRequest = null }) { Text(stringResource(R.string.common_cancel)) } })
    }
    Column(Modifier.fillMaxSize()) {
        NolaTopAppBar(title = stringResource(R.string.inspector_progress), onBack = onClose) {
            IconButton(onClick = { refresh++ }, enabled = !loading) { Icon(Icons.Filled.Refresh, stringResource(R.string.common_refresh)) }
        }
        Column(Modifier.weight(1f).imePadding().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (loading || submitting) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (failed) {
                Text(stringResource(R.string.inspector_progress_unavailable), color = MaterialTheme.colorScheme.error)
                TextButton(onClick = { refresh++ }) { Text(stringResource(R.string.common_refresh)) }
            }
            value?.let { progress ->
                if (host != null || state.currentModel?.source != com.nola.domain.model.ModelInfo.ModelSource.WebHost) {
                    controlError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                    if (pending) {
                        Text(stringResource(R.string.goal_operation_pending))
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            TextButton(onClick = { submit(null) }, enabled = !submitting) {
                                Text(stringResource(R.string.goal_operation_retry))
                            }
                            TextButton(
                                onClick = {
                                    val capturedOwner = owner ?: return@TextButton
                                    submitting = true
                                    controlError = null
                                    scope.launch {
                                        try {
                                            discardControl(capturedOwner)
                                            pending = pendingControl(capturedOwner)
                                        } catch (cancelled: kotlinx.coroutines.CancellationException) {
                                            throw cancelled
                                        } catch (error: Exception) {
                                            controlError = error.message?.takeIf { it.isNotBlank() } ?: operationFailedMessage
                                        } finally {
                                            submitting = false
                                        }
                                    }
                                },
                                enabled = !submitting
                            ) {
                                Text(stringResource(R.string.goal_discard_pending))
                            }
                        }
                    } else if (
                        progress.status == null ||
                        (progress.status in setOf("completed", "failed", "cancelled") && progress.goalControl?.canResume != true)
                    ) {
                        Text(stringResource(R.string.goal_guidance), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        OutlinedTextField(value = objective, onValueChange = { objective = it },
                            label = { Text(stringResource(R.string.goal_objective)) },
                            placeholder = { Text(stringResource(R.string.goal_example)) }, minLines = 3, maxLines = 8,
                            enabled = !submitting, modifier = Modifier.fillMaxWidth())
                        OutlinedTextField(
                            value = constraintsText,
                            onValueChange = { constraintsText = it },
                            label = { Text(stringResource(R.string.goal_constraints)) },
                            placeholder = { Text(stringResource(R.string.goal_constraints_hint)) },
                            minLines = 1,
                            maxLines = 4,
                            enabled = !submitting,
                            modifier = Modifier.fillMaxWidth()
                        )
                        Button(onClick = { submit("start") }, enabled = !submitting && objective.isNotBlank()) {
                            Text(stringResource(R.string.goal_start))
                        }
                    } else {
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            if (progress.goalControl?.canPause == true) TextButton(onClick = { submit("pause") }, enabled = !submitting) { Text(stringResource(R.string.goal_pause)) }
                            if (progress.goalControl?.canResume == true) TextButton(onClick = { submit("resume") }, enabled = !submitting) { Text(stringResource(R.string.goal_resume)) }
                            if (progress.goalControl?.canCancel == true) TextButton(onClick = {
                                progress.goalControl?.let { observed ->
                                    cancelRequest = com.nola.data.local.remoteGoalControlInput(java.util.UUID.randomUUID().toString(), "cancel",
                                        observed.goalId, observed.controlToken, "")
                                }
                            }, enabled = !submitting) { Text(stringResource(R.string.goal_cancel)) }
                        }
                    }
                }
                if (progress.objective != null || progress.status != null || progress.latestContractRun != null || progress.nextWakeAtEpochMs != null) {
                    NolaCard {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            progress.objective?.let {
                                Text(it, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                            }
                            progress.status?.let {
                                NolaBadge(text = progressStatusText(it), tint = progressStatusTint(it))
                            }
                        }
                        progress.latestContractRun?.let { run ->
                            Column(
                                Modifier.testTag("goal.contract.${run.runId}"),
                                verticalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                NolaSectionTitle(title = stringResource(R.string.goal_latest_step))
                                if (run.objective.isNotBlank() && run.objective != progress.objective) {
                                    Text(run.objective, style = MaterialTheme.typography.bodyMedium)
                                }
                                NolaBadge(text = progressStatusText(run.status), tint = progressStatusTint(run.status))
                            }
                        }
                        val additionalConstraints = progress.constraints.filterNot { constraint -> progress.requirements.any { it.text == constraint } }
                        if (additionalConstraints.isNotEmpty()) {
                            NolaSectionTitle(title = stringResource(R.string.goal_saved_constraints))
                            for (constraint in additionalConstraints) {
                                Text(constraint, style = MaterialTheme.typography.bodyMedium)
                            }
                        }
                        progress.nextWakeAtEpochMs?.let { wakeMs ->
                            val formattedWake = remember(wakeMs) {
                                java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.SHORT, java.text.DateFormat.SHORT)
                                    .format(java.util.Date(wakeMs))
                            }
                            Text(
                                stringResource(R.string.goal_next_wake_format, formattedWake),
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                }
                if (progress.requirements.isEmpty() && progress.todos.isEmpty()) Text(stringResource(R.string.inspector_no_plan), color = MaterialTheme.colorScheme.onSurfaceVariant)
                ProgressItems(stringResource(R.string.inspector_requirements), progress.requirements)
                ProgressItems(stringResource(R.string.inspector_plan), progress.todos)
            }
            if (owner == null) Text(stringResource(R.string.inspector_conversation_needed))
            else expertChildren(owner)
            val activeTasks = state.coreTaskProjection.orderedTasks.filter { owner != null && it.ownerSessionId == state.coreTaskProjection.sessionId }
            if (activeTasks.isNotEmpty()) {
                NolaCard {
                    activeTasks.forEachIndexed { index, task ->
                        if (index > 0) HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(
                                task.currentStep?.takeIf { it.isNotBlank() } ?: stringResource(R.string.canonical_running),
                                style = MaterialTheme.typography.bodyMedium,
                                modifier = Modifier.weight(1f),
                            )
                            NolaBadge(
                                text = progressStatusText(task.state.wireValue),
                                tint = progressStatusTint(task.state.wireValue),
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun progressStatusText(status: String): String = when (status) {
    "executing", "running" -> stringResource(R.string.canonical_running)
    "completed", "succeeded" -> stringResource(R.string.canonical_completed)
    "interrupted" -> stringResource(R.string.goal_step_interrupted)
    "awaiting_audit" -> stringResource(R.string.goal_step_review_pending)
    "audited" -> stringResource(R.string.goal_step_reviewed)
    "cancelled" -> stringResource(R.string.canonical_cancelled)
    "failed" -> stringResource(R.string.canonical_failed)
    "paused", "suspended" -> stringResource(R.string.scheduled_paused)
    "pending", "queued", "planned" -> stringResource(R.string.canonical_pending)
    "needs_user", "waiting_input" -> stringResource(R.string.goal_needs_user)
    "waiting_permission" -> stringResource(R.string.canonical_waiting_permission)
    else -> status.replace('_', ' ').trim().replaceFirstChar { it.uppercase() }
        .ifBlank { stringResource(R.string.chat_cache_unknown) }
}

@Composable
private fun progressStatusTint(status: String): Color = when (status) {
    "completed", "succeeded", "audited" -> NolaColors.success
    "executing", "running" -> MaterialTheme.colorScheme.primary
    "failed" -> MaterialTheme.colorScheme.error
    "needs_user", "waiting_input", "waiting_permission", "awaiting_audit", "interrupted" -> NolaColors.warning
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

@Composable
internal fun ProgressItems(title: String, entries: List<ConversationProgressItem>) {
    if (entries.isEmpty()) return
    NolaCard {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            NolaSectionTitle(title = title, modifier = Modifier.weight(1f))
            NolaBadge(
                text = stringResource(R.string.inspector_completed_count, entries.count { it.status == "completed" }, entries.size),
                tint = MaterialTheme.colorScheme.primary,
            )
        }
        entries.forEachIndexed { index, item ->
            if (index > 0) HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
            val tint = progressStatusTint(item.status)
            val icon = when (item.status) {
                "completed", "succeeded", "audited" -> Icons.Filled.CheckCircle
                "executing", "running" -> Icons.Filled.Schedule
                else -> Icons.Filled.RadioButtonUnchecked
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.Top,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.padding(top = 2.dp).size(18.dp))
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(item.text, style = MaterialTheme.typography.bodyMedium)
                    NolaBadge(text = progressStatusText(item.status), tint = tint)
                }
            }
        }
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
    val selectedDataSources = state.dataSources.filter { it.id in state.selectedDataSourceIds }
    Column(Modifier.fillMaxSize()) {
        NolaTopAppBar(title = stringResource(R.string.inspector_resources), onBack = onClose)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (parts.isEmpty() && sources.isEmpty() && attachments.isEmpty() && selectedDataSources.isEmpty()) {
                NolaEmptyState(
                    icon = Icons.Filled.FolderOpen,
                    title = stringResource(R.string.inspector_no_resources),
                )
            }
            if (parts.isNotEmpty() || sources.isNotEmpty()) {
                NolaCard {
                    NolaSectionTitle(icon = Icons.Filled.FolderOpen, title = stringResource(R.string.chat_sources_title))
                    messages.forEach { message -> key(message.conversationId, message.id) {
                        val messageParts = message.canonicalParts.filter {
                            it is com.nola.domain.projection.ConversationProjectionV1.Part.Source ||
                                it is com.nola.domain.projection.ConversationProjectionV1.Part.Citation ||
                                it is com.nola.domain.projection.ConversationProjectionV1.Part.Attachment
                        }
                        if (messageParts.isNotEmpty()) {
                            CanonicalPartReaderBoundary(message.conversationId, message.id, messageParts) { onReadText ->
                                CanonicalSourceCards(messageParts, onReadText)
                            }
                        }
                    } }
                    if (sources.isNotEmpty()) AnswerSourcesList(sources)
                }
            }
            if (attachments.isNotEmpty()) {
                val context = LocalContext.current
                NolaCard {
                    NolaSectionTitle(icon = Icons.Filled.AttachFile, title = stringResource(R.string.inspector_resources))
                    attachments.forEachIndexed { index, attachment ->
                        if (index > 0) HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
                        val canOpen = canPreviewAttachmentUri(attachment.uri)
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .heightIn(min = 48.dp)
                                .clickable(enabled = canOpen, role = Role.Button) {
                                    openAttachmentPreview(context, attachment.uri, attachment.mimeType, attachment.name)
                                },
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Text(
                                attachment.name,
                                style = MaterialTheme.typography.bodyMedium,
                                color = if (canOpen) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
                                modifier = Modifier.weight(1f),
                            )
                            NolaBadge(text = attachment.sizeLabel, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
            }
            if (selectedDataSources.isNotEmpty()) {
                NolaCard {
                    NolaSectionTitle(icon = Icons.Filled.Storage, title = stringResource(R.string.data_sources_screen_title))
                    selectedDataSources.forEachIndexed { index, source ->
                        if (index > 0) HorizontalDivider(thickness = 0.5.dp, color = MaterialTheme.colorScheme.outlineVariant)
                        Text(source.name, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }
    }
}
