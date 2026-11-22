// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.components

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import com.nola.R

@Composable
internal fun ForgetHostPairingDialog(onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.chat_host_pairing_forget_confirm_title)) },
        text = { Text(stringResource(R.string.chat_host_pairing_forget_confirm_message)) },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.common_cancel)) } },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(stringResource(R.string.chat_host_pairing_forget_confirm_action), color = MaterialTheme.colorScheme.error)
            }
        }
    )
}
