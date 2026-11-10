// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
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
    val outcomePending: Boolean = false)

@Composable
internal fun MessageEditAction(message: ConversationMessage, generating: Boolean,
    onResend: ((ConversationMessage) -> Unit)? = null) {
    TextButton(enabled = !generating && onResend != null,
        onClick = { onResend?.invoke(message) }, modifier = Modifier.heightIn(min = 48.dp).testTag("chat.message.edit." + (message.meta?.get("nola.canonicalMessageID") ?: message.id))) {
        Text(stringResource(R.string.chat_edit_resend), color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
internal fun MessageEditDialog(draft: AndroidMessageEditDraft, onText: (String) -> Unit,
    onSubmit: () -> Unit, onCancel: () -> Unit) {
    AlertDialog(containerColor = MaterialTheme.colorScheme.background,
        onDismissRequest = { if (!draft.busy && !draft.outcomePending) onCancel() },
        title = { Text(stringResource(R.string.chat_edit_resend)) },
        text = { Column(Modifier.heightIn(max = 440.dp).verticalScroll(rememberScrollState())) {
            OutlinedTextField(value = draft.input.editedText, onValueChange = onText, enabled = !draft.busy && !draft.outcomePending,
                label = { Text(stringResource(R.string.chat_edit_text)) }, minLines = 3, modifier = Modifier.fillMaxWidth().testTag("chat.message.editor.input"))
            Text(stringResource(R.string.chat_edit_preserves_original), style = MaterialTheme.typography.bodySmall)
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
