package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.res.stringResource
import com.nola.R
import kotlinx.coroutines.launch
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.ConversationProjectionWindowV1
import com.nola.ui.components.MarkdownRenderer

internal typealias TaskRecoveryCommand = suspend (com.nola.data.local.UnknownEffectRecoveryDecision?, Boolean) -> Pair<UnknownEffectRecoveryPresentation?, Boolean>

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TaskChildDetailSheet(load: suspend (Long?) -> ConversationProjectionWindowV1, recovery: TaskRecoveryCommand? = null, close: () -> Unit) {
    val scope = rememberCoroutineScope()
    var recoverySnapshot by remember { mutableStateOf<UnknownEffectRecoveryPresentation?>(null) }
    var recoveryBusy by remember { mutableStateOf(false) }
    var recoveryStatus by remember { mutableStateOf<String?>(null) }
    val noRecoveryText = stringResource(R.string.task_recovery_none)
    val blockedText = stringResource(R.string.chat_unknown_blocked)
    fun recover(decision: com.nola.data.local.UnknownEffectRecoveryDecision?, submit: Boolean) {
        val action = recovery ?: return
        if (recoveryBusy) return
        recoveryBusy = true
        scope.launch {
            try {
                val result = action(decision, submit)
                recoverySnapshot = result.first
                recoveryStatus = if (result.second) blockedText else if (result.first == null) noRecoveryText else null
            } catch (failure: Exception) {
                if (failure is kotlinx.coroutines.CancellationException) throw failure
                recoveryStatus = failure.message
                // Refresh only the original saved intent; this read never submits another decision.
                try { recoverySnapshot = action(null, false).first } catch (_: Exception) { }
            } finally { recoveryBusy = false }
        }
    }
    var messages by remember { mutableStateOf(emptyList<ConversationProjectionV1.Message>()) }
    var conversationId by remember { mutableStateOf<String?>(null) }
    var before by remember { mutableStateOf<Long?>(null) }
    var hasMore by remember { mutableStateOf(false) }
    var request by remember { mutableIntStateOf(0) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(request) {
        loading = true; error = null
        try {
            val window = load(before)
            require(conversationId == null || conversationId == window.projection.conversationId)
            conversationId = window.projection.conversationId
            messages = (window.projection.messages + messages).distinctBy { it.messageId }
            require(!window.hasMore || (window.beforeOrdinal != null && window.beforeOrdinal != before))
            before = window.beforeOrdinal; hasMore = window.hasMore
        } catch (failure: Exception) {
            if (failure is kotlinx.coroutines.CancellationException) throw failure
            error = failure.message ?: "Task detail unavailable"
        } finally { loading = false }
    }
    ModalBottomSheet(onDismissRequest = close, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        modifier = Modifier.testTag("chat.task.detail")) {
        Column(Modifier.fillMaxWidth().weight(1f, false).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            TextButton(onClick = close, modifier = Modifier.testTag("chat.task.detail.back")) { Text(stringResource(R.string.chat_task_details_back)) }
            Text(stringResource(R.string.chat_task_details), style = MaterialTheme.typography.titleMedium)
            if (recovery != null) {
                TextButton(onClick = { recover(null, false) }, enabled = !recoveryBusy) { Text(stringResource(R.string.chat_unknown_check)) }
                recoverySnapshot?.let { snapshot ->
                    UnknownEffectRecoveryCard(snapshot, recoveryBusy, { _, decision -> recover(decision, true) }, allowsRiskRetry = false)
                }
                recoveryStatus?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
            }
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth())
            error?.let { Text(it, color = MaterialTheme.colorScheme.error); TextButton(onClick = { request++ }) { Text(stringResource(R.string.common_retry)) } }
            if (hasMore && !loading) TextButton(onClick = { request++ }) { Text(stringResource(R.string.chat_task_details_earlier)) }
            messages.forEach { message -> key(conversationId, message.messageId) {
                var activityChoices by remember { mutableStateOf(CanonicalActivityDisclosureChoices()) }
                CanonicalPartReaderBoundary(checkNotNull(conversationId), message.messageId, message.parts) { onReadText ->
                Column(Modifier.testTag("chat.task.detail.message.${message.messageId}"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(message.role.name, style = MaterialTheme.typography.labelMedium)
                    canonicalMessageBlocks(message.parts).forEach { block -> key(block.key) {
                        when (block) {
                            is CanonicalMessageBlock.Activity -> CanonicalActivityCards(block.parts, isLive = false, choices = activityChoices, onChoicesChanged = { activityChoices = it }, onReadText = onReadText)
                            is CanonicalMessageBlock.Part -> when (val part = block.part) {
                                is ConversationProjectionV1.Part.Text -> if (part.disclosedMarkdown.isNotEmpty()) MarkdownRenderer(part.disclosedMarkdown)
                                is ConversationProjectionV1.Part.Error -> CanonicalErrorPart(part, onReadText)
                                is ConversationProjectionV1.Part.UiAction -> if (part.summary.isNotBlank()) Text(part.summary)
                                is ConversationProjectionV1.Part.UiSurface -> com.nola.data.local.uiReferencePresentation(
                                    part, conversationId, message.messageId)?.let { CanonicalUiReferenceUnavailable(it) }
                                is ConversationProjectionV1.Part.Historical -> {
                                    HistoricalPartActions(listOf(part))
                                    part.projection.timelineText?.takeIf { it.isNotBlank() }?.let { MarkdownRenderer(it) }
                                    com.nola.data.local.uiReferencePresentation(part, conversationId, message.messageId)?.let { CanonicalUiReferenceUnavailable(it) }
                                }
                                is ConversationProjectionV1.Part.Source, is ConversationProjectionV1.Part.Citation,
                                is ConversationProjectionV1.Part.Attachment -> CanonicalSourceCards(listOf(part), onReadText)
                                else -> Unit
                            }
                        }
                    } }
                }
                }
            } }
            Spacer(Modifier.height(24.dp))
        }
    }
}
