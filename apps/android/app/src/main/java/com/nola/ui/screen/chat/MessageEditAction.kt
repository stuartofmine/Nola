// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.ui.components.dismissImeOnBlankTap
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.input.key.*
import com.nola.data.repository.PersistentAgent
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.domain.model.ConversationMessage
import com.nola.data.local.AndroidPrepareEditedResend

/** Stable presentation draft remains outside transcript rows and across target selection. */
data class AndroidMessageEditDraft(val input: AndroidPrepareEditedResend, val originalText: String,
    val busy: Boolean = false, val error: String? = null, val progress: String? = null,
    val outcomePending: Boolean = false, val mentions: List<InlineExpertMention> = emptyList())

@Composable
internal fun MessageEditAction(message: ConversationMessage, generating: Boolean,
    onResend: ((ConversationMessage) -> Unit)? = null) {
    TextButton(enabled = !generating && onResend != null,
        onClick = { onResend?.invoke(message) }, modifier = Modifier.heightIn(min = 48.dp).testTag("chat.message.edit." + (message.meta?.get("nola.canonicalMessageID") ?: message.id))) {
        Text(stringResource(R.string.chat_edit_resend), color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
internal fun MessageEditDialog(draft: AndroidMessageEditDraft, onText: (String, List<InlineExpertMention>) -> Unit,
    onSubmit: () -> Unit, onCancel: () -> Unit, candidates: List<PersistentAgent> = emptyList(),
    prepare: suspend () -> Boolean = { false }, candidate: (String) -> InlineExpertMention? = { null }) {
    var editor by remember(draft.input.sourceMessageId) { mutableStateOf(TextFieldValue(draft.input.editedText, TextRange(draft.input.editedText.length))) }
    val query = expertQuery(editor, draft.mentions)
    var ready by remember { mutableStateOf(false) }
    var dismissed by remember { mutableStateOf<ExpertQuery?>(null) }
    LaunchedEffect(query != null) { ready = query != null && prepare() }
    var highlighted by remember(query) { mutableIntStateOf(0) }
    val descriptions = remember(candidates) { mentionDescriptions(candidates) }
    val options = if (ready && query != null && query != dismissed) candidates.filter { it.availability == "active" &&
        (it.name.contains(query.text, true) || it.description.contains(query.text, true)) } else emptyList()
    fun update(edit: MentionEdit) { editor = edit.value; onText(edit.value.text, edit.mentions) }
    fun choose(id: String) {
        val range = query ?: return
        val ref = draft.mentions.firstOrNull { it.agentId == id } ?: candidate(id) ?: return
        update(insertExpert(editor, draft.mentions, TextRange(range.start, range.end), ref))
    }
    AlertDialog(containerColor = MaterialTheme.colorScheme.background,
        modifier = Modifier.dismissImeOnBlankTap(),
        onDismissRequest = { if (!draft.busy && !draft.outcomePending) onCancel() },
        title = { Text(stringResource(R.string.chat_edit_resend)) },
        text = { Column(Modifier.heightIn(max = 440.dp).verticalScroll(rememberScrollState())) {
            OutlinedTextField(value = editor, onValueChange = { update(editExpertText(editor, it, draft.mentions)) },
                visualTransformation = expertMentionStyle(draft.mentions), enabled = !draft.busy && !draft.outcomePending,
                label = { Text(stringResource(R.string.chat_edit_text)) }, minLines = 3, modifier = Modifier.fillMaxWidth().testTag("chat.message.editor.input").onPreviewKeyEvent { event ->
                    if (event.type != KeyEventType.KeyDown || editor.composition != null || options.isEmpty()) false else when (event.key) {
                        Key.Escape -> { dismissed = query; true }; Key.Enter -> { choose(options[highlighted.coerceIn(options.indices)].id); true }
                        Key.DirectionUp, Key.DirectionDown -> { highlighted = (highlighted + (if (event.key == Key.DirectionDown) 1 else -1) + options.size) % options.size; true }; else -> false
                    }
                })
            options.forEach { agent -> TextButton(onClick = { choose(agent.id) }) { Text(listOf(agent.name, descriptions[agent.id].orEmpty()).filter(String::isNotBlank).joinToString(" · ")) } }
            Text(stringResource(R.string.chat_edit_preserves_original), style = MaterialTheme.typography.bodySmall)
            if (draft.error != null && draft.mentions.isNotEmpty() && !draft.outcomePending) {
                TextButton(onClick = { onText(editor.text, emptyList()); dismissed = expertQuery(editor, emptyList()) }) {
                    Text(stringResource(R.string.expert_mention_plain))
                }
            }
            if (draft.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            draft.progress?.let { Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall, modifier = Modifier.testTag("chat.message.editor.progress")) }
            draft.error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.testTag("chat.message.editor.error")) }
        } },
        confirmButton = { TextButton(enabled = !draft.busy && draft.input.editedText.isNotBlank(), onClick = onSubmit, modifier = Modifier.testTag("chat.message.editor.submit")) {
            Text(stringResource(if (draft.outcomePending) R.string.expert_host_check_original else R.string.chat_save_resend)) } },
        dismissButton = { TextButton(enabled = !draft.busy && !draft.outcomePending, onClick = onCancel, modifier = Modifier.testTag("chat.message.editor.cancel")) { Text(stringResource(R.string.common_cancel)) } })
}
