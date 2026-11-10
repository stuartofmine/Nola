package com.nola.ui.screen.chat

import android.view.WindowManager
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import com.nola.R
import com.nola.domain.projection.ConversationProjectionV1

/** Public field identity, not a file reference or authority to read another body. */
internal enum class CanonicalPartReadField {
    ErrorIdentity, ErrorMetadata, ToolPermission, ToolInput, ToolResult, ToolPreview, ToolOrigin,
    ToolPermissionIdentity, ToolStarted, ToolFinished, ToolOutputOwner,
    SourceIdentity, SourceUri, SourceOrigin, SourceEvidence, SourceSupersedes,
}

internal data class CanonicalPartTextSnapshot(
    val partId: String,
    val field: CanonicalPartReadField,
    val label: String,
    val text: String,
    val code: Boolean,
)

/** Ephemeral message-owned reader. Opening it never performs IO or follows a URI. */
@Composable
internal fun CanonicalPartReaderBoundary(
    conversationId: String,
    messageId: String,
    parts: List<ConversationProjectionV1.Part>,
    content: @Composable ((CanonicalPartTextSnapshot) -> Unit) -> Unit,
) {
    val owner = conversationId to messageId
    val activation = remember(owner) { Any() }
    var snapshot by remember(owner) { mutableStateOf<CanonicalPartTextSnapshot?>(null) }
    val partIds = parts.mapTo(mutableSetOf()) { it.partId }
    val latestOwner by rememberUpdatedState(owner)
    val latestActivation by rememberUpdatedState(activation)
    val latestPartIds by rememberUpdatedState(partIds)
    LaunchedEffect(owner, partIds) {
        if (snapshot?.partId?.let { it !in latestPartIds } == true) snapshot = null
    }
    content { requested ->
        if (latestActivation === activation && latestOwner == owner && requested.partId in latestPartIds) snapshot = requested
    }
    // Hide synchronously on removal; the effect also discards the retained string.
    snapshot?.takeIf { it.partId in partIds }?.let { displayed ->
        Dialog(onDismissRequest = { if (latestActivation === activation && latestOwner == owner && snapshot == displayed) snapshot = null },
            properties = DialogProperties(decorFitsSystemWindows = false)) {
            val window = (LocalView.current.parent as DialogWindowProvider).window
            SideEffect { window.setLayout(WindowManager.LayoutParams.MATCH_PARENT, WindowManager.LayoutParams.MATCH_PARENT) }
            Surface(Modifier.fillMaxSize().testTag("canonical.reader.${displayed.partId}.${displayed.field.name}"),
                color = MaterialTheme.colorScheme.surface, contentColor = MaterialTheme.colorScheme.onSurface) {
                Column(Modifier.windowInsetsPadding(WindowInsets.safeDrawing).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = androidx.compose.ui.Alignment.CenterVertically) {
                        Text(displayed.label, Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                        TextButton(onClick = { if (latestActivation === activation && latestOwner == owner && snapshot == displayed) snapshot = null }) {
                            Text(stringResource(R.string.common_close))
                        }
                    }
                    Text(stringResource(R.string.canonical_reader_snapshot), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                    SelectionContainer(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
                        Text(displayed.text, Modifier.fillMaxWidth(), style = MaterialTheme.typography.bodyMedium,
                            fontFamily = if (displayed.code) FontFamily.Monospace else null)
                    }
                }
            }
        }
    }
}
