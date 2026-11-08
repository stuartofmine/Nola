package com.nola.ui.screen.chat

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.data.local.UnknownEffectRecoveryDecision

/** Inline uncertainty, with a user-initiated risk dialog. No launch-time modal. */
@Composable
internal fun UnknownEffectRecoveryCard(snapshot: UnknownEffectRecoveryPresentation, busy: Boolean,
    onDecision: (UnknownEffectRecoveryPresentation, UnknownEffectRecoveryDecision?) -> Unit,
    allowsRiskRetry: Boolean = true) {
    var selected by remember(snapshot.snapshotId) { mutableStateOf<Int?>(null) }
    var reason by remember(snapshot.snapshotId) { mutableStateOf("") }
    var confirmation by remember(snapshot.snapshotId) { mutableStateOf<UnknownEffectRecoveryDecision?>(null) }
    val inspection = snapshot.inspection
    val validReason = reason.isNotBlank() && reason.toByteArray(Charsets.UTF_8).size <= 4096
    OutlinedCard(Modifier.fillMaxWidth().testTag("unknown-effect-recovery")) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.chat_unknown_title), style = MaterialTheme.typography.titleMedium)
            Text(stringResource(if (allowsRiskRetry) R.string.chat_unknown_detail else R.string.chat_unknown_close_detail), style = MaterialTheme.typography.bodyMedium)
            if (snapshot.pending != null) {
                Text(stringResource(R.string.chat_unknown_pending), style = MaterialTheme.typography.bodyMedium)
                OutlinedButton(onClick = { onDecision(snapshot, null) }, enabled = !busy) {
                    Text(stringResource(R.string.chat_unknown_receipt))
                }
            } else if (inspection != null) {
                if (allowsRiskRetry) LazyColumn(Modifier.fillMaxWidth().heightIn(max = 180.dp)) {
                    itemsIndexed(inspection.targets, key = { _, target -> target.effectId }) { index, target ->
                        TextButton(onClick = { selected = index }, enabled = !busy,
                            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                            RadioButton(selected = selected == index, onClick = null)
                            Spacer(Modifier.width(8.dp))
                            Text(target.effectId, style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                }
                OutlinedTextField(value = reason, onValueChange = { reason = it }, enabled = !busy,
                    label = { Text(stringResource(R.string.chat_unknown_reason)) }, modifier = Modifier.fillMaxWidth(), maxLines = 4)
                if (allowsRiskRetry) OutlinedButton(onClick = { confirmation = inspection.decision(selected, reason) },
                    enabled = !busy && inspection.retryAllowed && validReason && selected != null) {
                    Text(stringResource(R.string.chat_unknown_review_retry))
                }
                TextButton(onClick = { confirmation = inspection.decision(null, reason) },
                    enabled = !busy && validReason && inspection.targets.isNotEmpty() && inspection.independentPendingEffects == 0L) {
                    Text(stringResource(R.string.chat_unknown_block))
                }
                if (inspection.independentPendingEffects > 0) Text(stringResource(R.string.chat_unknown_independent),
                    style = MaterialTheme.typography.bodyMedium)
            }
            if (busy) CircularProgressIndicator(Modifier.size(24.dp))
        }
    }
    confirmation?.let { frozen ->
        AlertDialog(onDismissRequest = { confirmation = null },
            title = { Text(stringResource(if (frozen.choice.kind == com.nola.data.local.UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked)
                R.string.chat_unknown_block else R.string.chat_unknown_confirm_title)) },
            text = { Text(stringResource(if (frozen.choice.kind == com.nola.data.local.UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked)
                R.string.chat_unknown_close_detail else R.string.chat_unknown_risk)) },
            confirmButton = {
                TextButton(onClick = { confirmation = null; onDecision(snapshot, frozen) }) {
                    Text(stringResource(if (frozen.choice.kind == com.nola.data.local.UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked)
                        R.string.chat_unknown_block else R.string.chat_unknown_authorize), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { confirmation = null }) { Text(stringResource(R.string.common_cancel)) } })
    }
}
