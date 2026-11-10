package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.ui.components.DiscardChangesDialog
import com.nola.domain.model.UploadedContent
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

@Composable
internal fun ChatSkillEditorDialog(
    original: UploadedContent?,
    onSave: suspend (UploadedContent?, String, String) -> Unit,
    onDismiss: () -> Unit,
) {
    val templateName = stringResource(R.string.chat_new_skill)
    val templateContent = stringResource(R.string.chat_new_skill_template)
    val failureText = stringResource(R.string.chat_save_failed)
    val baseline = remember { original }
    var name by remember { mutableStateOf(baseline?.name ?: templateName) }
    var content by remember { mutableStateOf(baseline?.content ?: templateContent) }
    var saving by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var discard by remember { mutableStateOf(false) }
    val dirty = name != (baseline?.name ?: templateName) || content != (baseline?.content ?: templateContent)
    val requestDismiss = { if (!saving) { if (dirty) discard = true else onDismiss() } }
    if (discard) DiscardChangesDialog(onDiscard = onDismiss, onKeepEditing = { discard = false })
    val scope = rememberCoroutineScope()
    AlertDialog(
        onDismissRequest = requestDismiss,
        title = { Text(stringResource(R.string.chat_skill_content)) },
        text = {
            Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState())) {
                OutlinedTextField(name, { name = it }, enabled = !saving, singleLine = true,
                    label = { Text(stringResource(R.string.chat_skill_name)) },
                    modifier = Modifier.fillMaxWidth().testTag("chat.skill.editor.name"))
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(content, { content = it }, enabled = !saving, minLines = 6, maxLines = 12,
                    label = { Text(stringResource(R.string.chat_skill_content)) },
                    modifier = Modifier.fillMaxWidth().testTag("chat.skill.editor.content"))
                if (failed) {
                    Text(failureText, color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.testTag("chat.skill.editor.error"))
                }
            }
        },
        confirmButton = {
            TextButton(enabled = !saving && name.isNotBlank() && content.isNotBlank(),
                modifier = Modifier.testTag("chat.skill.editor.save"), onClick = {
                    saving = true
                    failed = false
                    scope.launch {
                        try {
                            onSave(baseline, name.trim(), content)
                            onDismiss()
                        } catch (error: Exception) {
                            if (error is CancellationException) throw error
                            failed = true
                        } finally {
                            saving = false
                        }
                    }
                }) {
                if (saving) CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                else Text(stringResource(R.string.common_save))
            }
        },
        dismissButton = {
            TextButton(onClick = requestDismiss, enabled = !saving,
                modifier = Modifier.testTag("chat.skill.editor.cancel")) {
                Text(stringResource(R.string.common_cancel))
            }
        },
    )
}
