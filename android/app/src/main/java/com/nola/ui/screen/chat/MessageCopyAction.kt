// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.ui.graphics.Color
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R

/** Persistent external action; copy failures retain selectable text instead of claiming success. */
@Composable
internal fun MessageCopyAction(text: String, modifier: Modifier = Modifier, compact: Boolean = false) {
    val context = LocalContext.current
    var manualText by remember(text) { mutableStateOf<String?>(null) }
    var copied by remember(text) { mutableStateOf(false) }
    var copyAttempt by remember(text) { mutableStateOf(0) }
    LaunchedEffect(copied, copyAttempt) {
        if (copied) { kotlinx.coroutines.delay(1800); copied = false }
    }
    TextButton(onClick = {
        copyAttempt += 1
        val captured = text
        val result = runCatching {
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("Nola message", captured))
            check(clipboard.primaryClip?.getItemAt(0)?.text?.toString() == captured)
        }
        copied = result.isSuccess
        if (result.isFailure) manualText = captured
    }, enabled = text.isNotBlank(), modifier = modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp),
        colors = ButtonDefaults.textButtonColors(containerColor = Color.Transparent, disabledContainerColor = Color.Transparent)) {
        Icon(if (copied) Icons.Filled.Check else Icons.Filled.ContentCopy,
            contentDescription = if (compact) stringResource(if (copied) R.string.chat_message_copied else R.string.chat_message_action_copy) else null,
            modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        if (!compact) {
            Spacer(Modifier.width(6.dp))
            Text(stringResource(if (copied) R.string.chat_message_copied else R.string.chat_message_action_copy), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    manualText?.let { captured ->
        AlertDialog(onDismissRequest = { manualText = null },
            title = { Text(stringResource(R.string.chat_copy_manually)) },
            text = { Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState())) {
                SelectionContainer { Text(captured, style = MaterialTheme.typography.bodyLarge) }
            } },
            confirmButton = { TextButton(onClick = { manualText = null }) { Text(stringResource(R.string.common_done)) } })
    }
}
