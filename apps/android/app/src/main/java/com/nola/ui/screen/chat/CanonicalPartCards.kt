package com.nola.ui.screen.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.border
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.IconButton
import androidx.compose.material3.AlertDialog
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.Icon
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Warning
import com.nola.ui.components.NolaColors
import androidx.compose.material3.TextButton
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import com.nola.ui.components.hostToolDisplayName
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.compose.foundation.Image
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.core.content.FileProvider
import androidx.compose.ui.semantics.Role
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import com.nola.ui.components.openableToolUri
import com.nola.R
import com.nola.ui.components.ThinkingBody
import com.nola.ui.components.thinkingExpandedAfterStatusChange
import com.nola.ui.components.ThinkingBodyVisibleCharactersKey
import androidx.compose.ui.semantics.semantics
import com.nola.ui.components.NolaDisclosureBar
import com.nola.ui.components.NolaDisclosureChevron
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.rememberReduceMotionEnabled
import com.nola.domain.model.AnswerSource
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.PartArtifact
import com.nola.domain.projection.cardFacts
import com.nola.domain.projection.sourceCardFacts

/** Production cards read typed canonical facts directly, for both live and restored messages. */
@Composable
internal fun CanonicalActivityCards(
    parts: List<ConversationProjectionV1.Part>,
    isLive: Boolean,
    choices: CanonicalActivityDisclosureChoices,
    onChoicesChanged: (CanonicalActivityDisclosureChoices) -> Unit,
    onReadText: (CanonicalPartTextSnapshot) -> Unit,
    thinkingEnabled: Boolean = true,
) {
    // Callers pass a contiguous canonical segment. Keep steps in that order.
    require(parts.all { it is ConversationProjectionV1.Part.Reasoning || it is ConversationProjectionV1.Part.Tool })
    val activity = parts
    val placeholderId = visibleReasoningPlaceholderId(activity, isLive)
    val visible = activity.filter { it !is ConversationProjectionV1.Part.Reasoning ||
        (thinkingEnabled && (!it.summary.isNullOrBlank() || it.partId == placeholderId)) }
    if (visible.isEmpty()) return
    // The live preview never opens earlier calls, including recovered failures.
    val expanded = choices.expandedFor(activity) ?: false
    val reduceMotion = rememberReduceMotionEnabled()
    val singleReasoning = visible.singleOrNull() as? ConversationProjectionV1.Part.Reasoning
    val heading = if (isLive) R.string.chat_processing else if (singleReasoning == null) R.string.chat_execution_process else when {
        reasoningIsFinished(singleReasoning.state, isLive) -> R.string.thinking_process
        singleReasoning.summary.isNullOrBlank() -> R.string.chat_processing
        else -> R.string.chat_thinking
    }
    val current = currentExecutionActivity(activity, isLive)
    val currentTitle = when (current) {
        is ConversationProjectionV1.Part.Tool -> hostToolDisplayName(current.capabilityId, current.title).takeIf {
            current.state in setOf(ConversationProjectionV1.ActivityState.Running, ConversationProjectionV1.ActivityState.Pending)
        }
        is ConversationProjectionV1.Part.Reasoning -> if (current.state == ConversationProjectionV1.ActivityState.Running)
            stringResource(R.string.chat_thinking) else null
        else -> null
    }
    val cardShape = RoundedCornerShape(12.dp)
    val containerColor = if (isLive) {
        MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.55f)
    } else {
        MaterialTheme.colorScheme.surfaceContainer
    }
    val borderColor = if (isLive) {
        MaterialTheme.colorScheme.primary.copy(alpha = 0.38f)
    } else {
        NolaColors.hairlineBorder
    }
    Column(Modifier.fillMaxWidth()
        .clip(cardShape)
        .background(containerColor, cardShape)
        .border(0.75.dp, borderColor, cardShape)
        .padding(horizontal = 12.dp)) {
        TextButton(onClick = { onChoicesChanged(choices.expanded(activity, !expanded)) },
            colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onSurface),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
            if (isLive) {
                CircularProgressIndicator(
                    modifier = Modifier.size(15.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.primary,
                    trackColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.25f),
                )
            } else {
                Icon(
                    imageVector = Icons.Filled.AutoAwesome,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.primary.copy(alpha = 0.85f),
                )
            }
            Spacer(Modifier.width(8.dp))
            Text(
                executionProcessHeading(isLive, currentTitle, stringResource(R.string.chat_processing), stringResource(heading)),
                modifier = Modifier.weight(1f),
                style = MaterialTheme.typography.labelLarge,
                color = if (isLive) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.9f),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            NolaDisclosureChevron(expanded, reduceMotion)
        }
        AnimatedVisibility(visible = expanded, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
            Column(Modifier.padding(bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                visible.forEachIndexed { index, part -> key(part.partId) {
                    if (visible.size > 1) Text("${index + 1}", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    when (part) {
                        is ConversationProjectionV1.Part.Reasoning -> ThinkingBody(
                            thinking = part.summary.orEmpty(),
                            modifier = Modifier.testTag("assistant.reasoning.block").semantics {
                                this[ThinkingBodyVisibleCharactersKey] = part.summary.orEmpty().length
                            },
                            finished = reasoningIsFinished(part.state, isLive),
                        )
                        is ConversationProjectionV1.Part.Tool -> CanonicalActivityRow(part, details = part.partId in choices.detailsPartIds,
                            onDetailsChanged = { onChoicesChanged(choices.details(activity, part.partId, it)) }, onReadText = onReadText)
                        else -> Unit
                    }
                } }
            }
        }
    }
}

private val prettyToolJson = Json { prettyPrint = true }

internal fun formatToolJsonPayload(element: JsonElement): String = when (element) {
    is JsonNull -> ""
    is JsonPrimitive -> if (element.isString) element.content else element.toString()
    else -> runCatching {
        prettyToolJson.encodeToString(JsonElement.serializer(), element)
    }.getOrElse { element.toString() }
}

internal fun formatCanonicalErrorMessage(code: String, displayKey: String?, fallback: String): String {
    fun humanize(raw: String): String {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) return ""
        if (trimmed.any { it.isWhitespace() }) return trimmed
        val leaf = trimmed.substringAfterLast('.')
        val spaced = leaf.replace('_', ' ').replace('-', ' ').trim()
        return spaced.replaceFirstChar { if (it.isLowerCase()) it.titlecase() else it.toString() }
    }
    val primary = displayKey?.let(::humanize).orEmpty().ifBlank { humanize(code) }
    return primary.ifBlank { fallback }
}

/** Error parts expose typed code/display-key facts and surface a readable summary directly. */
@Composable
internal fun CanonicalErrorPart(part: ConversationProjectionV1.Part.Error, onReadText: (CanonicalPartTextSnapshot) -> Unit) {
    var details by remember(part.partId, part.code, part.displayKey) { mutableStateOf(false) }
    val headline = formatCanonicalErrorMessage(part.code, part.displayKey, stringResource(R.string.canonical_part_error))
    Column(Modifier.fillMaxWidth().testTag("chat.error.${part.partId}"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(headline, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
        val rawCode = part.code.trim()
        if (rawCode.isNotEmpty() && !headline.equals(rawCode, ignoreCase = true)) {
            Text(rawCode, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.labelSmall)
        }
        NolaDisclosureBar(
            title = stringResource(if (details) R.string.canonical_hide_details else R.string.canonical_details),
            expanded = details,
            onToggle = { details = !details }
        )
        if (details) {
            Fact(part.partId, CanonicalPartReadField.ErrorIdentity, onReadText, stringResource(R.string.canonical_identity), part.partId)
            val summaryText = listOfNotNull(
                part.code.takeIf { it.isNotBlank() },
                part.displayKey?.takeIf { it.isNotBlank() },
            ).joinToString(" · ")
            if (summaryText.isNotEmpty()) {
                Fact(part.partId, CanonicalPartReadField.ErrorMetadata, onReadText, stringResource(R.string.canonical_preview), summaryText)
            }
        }
    }
}

@Composable
private fun CanonicalActivityRow(part: ConversationProjectionV1.Part.Tool, details: Boolean, onDetailsChanged: (Boolean) -> Unit, onReadText: (CanonicalPartTextSnapshot) -> Unit) {
    val reduceMotion = rememberReduceMotionEnabled()
    val facts = part.cardFacts()
    val visualKind = resolveToolStatusVisualKind(facts.state)
    Column(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            when (visualKind) {
                ToolStatusVisualKind.Running -> CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.primary,
                    trackColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.25f),
                )
                ToolStatusVisualKind.Completed -> Icon(
                    Icons.Filled.CheckCircle,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = NolaColors.success,
                )
                ToolStatusVisualKind.Failed -> Icon(
                    Icons.Filled.Error,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.error,
                )
                ToolStatusVisualKind.Waiting -> Icon(
                    Icons.Filled.Warning,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = NolaColors.warning,
                )
                ToolStatusVisualKind.Neutral -> Icon(
                    Icons.Filled.Info,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(hostToolDisplayName(facts.capabilityId, facts.title), Modifier.weight(1f), style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
            ActivityStateBadge(facts.state)
        }
        facts.summary?.takeIf { it.isNotBlank() }?.let { summary ->
            if (visualKind == ToolStatusVisualKind.Failed) {
                Text(
                    text = summary,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.35f))
                        .border(0.5.dp, MaterialTheme.colorScheme.error.copy(alpha = 0.35f), RoundedCornerShape(8.dp))
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                )
            } else {
                Text(summary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        facts.artifacts.forEach { ArtifactCard(it) }
        NolaDisclosureBar(
            title = stringResource(if (details) R.string.canonical_hide_details else R.string.canonical_details),
            expanded = details,
            onToggle = { onDetailsChanged(!details) }
        )
        AnimatedVisibility(visible = details, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
            var showRawDetails by remember(part.partId) { mutableStateOf(false) }
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                val friendlyOrigin = listOfNotNull(facts.origin.providerTitle, facts.origin.namespace).joinToString(" · ")
                if (friendlyOrigin.isNotEmpty()) {
                    Text(friendlyOrigin, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                facts.permission?.let { permission ->
                    Fact(part.partId, CanonicalPartReadField.ToolPermission, onReadText, stringResource(R.string.canonical_permission_record), listOfNotNull(permission.rationale, permission.permissionDomain, permission.decision).joinToString(" · "))
                }
                facts.result?.let { result ->
                    val formatted = formatToolJsonPayload(result)
                    if (formatted.isNotBlank()) {
                        Fact(part.partId, CanonicalPartReadField.ToolResult, onReadText, stringResource(R.string.canonical_result), formatted, code = true)
                    }
                }
                facts.preview?.let { Fact(part.partId, CanonicalPartReadField.ToolPreview, onReadText, stringResource(R.string.canonical_preview), it, code = true) }
                facts.startedAtMs?.let {
                    Fact(part.partId, CanonicalPartReadField.ToolStarted, onReadText, stringResource(R.string.canonical_started),
                        java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.SHORT, java.text.DateFormat.MEDIUM).format(java.util.Date(it.coerceAtMost(Long.MAX_VALUE.toULong()).toLong())))
                }
                facts.finishedAtMs?.let {
                    Fact(part.partId, CanonicalPartReadField.ToolFinished, onReadText, stringResource(R.string.canonical_finished),
                        java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.SHORT, java.text.DateFormat.MEDIUM).format(java.util.Date(it.coerceAtMost(Long.MAX_VALUE.toULong()).toLong())))
                }
                NolaDisclosureBar(
                    title = stringResource(R.string.canonical_raw_details),
                    expanded = showRawDetails,
                    onToggle = { showRawDetails = !showRawDetails },
                )
                AnimatedVisibility(visible = showRawDetails, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(listOfNotNull(facts.capabilityId, facts.origin.providerTitle, facts.origin.namespace).joinToString(" · "),
                            style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        val formattedInput = formatToolJsonPayload(facts.input)
                        if (formattedInput.isNotBlank()) {
                            Fact(part.partId, CanonicalPartReadField.ToolInput, onReadText, stringResource(R.string.canonical_input), formattedInput, code = true)
                        }
                        Fact(part.partId, CanonicalPartReadField.ToolOrigin, onReadText, stringResource(R.string.canonical_origin), listOfNotNull(facts.origin.kind, facts.origin.providerId, facts.origin.providerTitle, facts.origin.namespace).joinToString(" · "))
                        facts.permission?.let { Fact(part.partId, CanonicalPartReadField.ToolPermissionIdentity, onReadText, stringResource(R.string.canonical_permission_record), it.requestId) }
                        // Owner facts are selectable display only; there is intentionally no open/download action.
                        facts.fullOutput?.let { owner -> Fact(part.partId, CanonicalPartReadField.ToolOutputOwner, onReadText, stringResource(R.string.canonical_retained_output),
                            listOf(owner.messageId, owner.partId, owner.toolCallId).joinToString(" · ")) }
                    }
                }
            }
        }
    }
}

@Composable
internal fun CanonicalSourceCards(parts: List<ConversationProjectionV1.Part>, onReadText: (CanonicalPartTextSnapshot) -> Unit) {
    val context = LocalContext.current
    val references = canonicalBibliography(parts)
    var expanded by remember(parts.firstOrNull()?.partId) { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (references.isNotEmpty()) Text("${stringResource(R.string.chat_sources_title)} ${references.size}",
            style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        references.take(if (expanded) references.size else 3).forEachIndexed { index, (part, facts) ->
            key(part.partId) {
                var details by remember(part.partId) { mutableStateOf(false) }
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text("[${index + 1}] ${facts.title}", Modifier.weight(1f).heightIn(min = 48.dp)
                        .clickable(enabled = isExternalAnswerSource(facts.uri), role = androidx.compose.ui.semantics.Role.Button) {
                            openAnswerSource(context, AnswerSource(id = facts.identity, sourceId = facts.sourceId, title = facts.title, uri = facts.uri, mimeType = facts.mimeType))
                        }.padding(vertical = 10.dp), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    IconButton(onClick = { details = true }) { Icon(Icons.Filled.Info, stringResource(R.string.canonical_details), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
                if (details) AlertDialog(onDismissRequest = { details = false }, title = { Text(facts.title) },
                    text = { SelectionContainer { Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(facts.uri, style = MaterialTheme.typography.bodySmall)
                        facts.location?.let { Text(listOfNotNull(it.section, it.page?.let { "p. $it" }, it.lineStart?.let { start -> "L$start–${it.lineEnd ?: start}" }).joinToString(" · ")) }
                        facts.snippet?.let { Text(it) }
                        facts.evidence?.usage?.let { Text(listOfNotNull(it.title, it.source, it.observedAt).joinToString(" · ")) }
                    } } },
                    confirmButton = { TextButton(onClick = { details = false }) { Text(stringResource(R.string.common_done)) } })
            }
        }
        if (references.size > 3) TextButton(onClick = { expanded = !expanded }, modifier = Modifier.testTag("chat.sources.toggle")) {
            Text(stringResource(if (expanded) R.string.chat_sources_fewer else R.string.chat_sources_all), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        parts.filterIsInstance<ConversationProjectionV1.Part.Attachment>().forEach { part ->
            key(part.partId) { PartCard { ArtifactCard(PartArtifact(part.uri, part.name, part.mimeType, part.operation)) } }
        }
    }
}

internal enum class ToolStatusVisualKind {
    Running,
    Completed,
    Failed,
    Waiting,
    Neutral,
}

internal fun resolveToolStatusVisualKind(state: ConversationProjectionV1.ActivityState): ToolStatusVisualKind = when (state) {
    ConversationProjectionV1.ActivityState.Running,
    ConversationProjectionV1.ActivityState.Pending -> ToolStatusVisualKind.Running
    ConversationProjectionV1.ActivityState.Completed -> ToolStatusVisualKind.Completed
    ConversationProjectionV1.ActivityState.Failed -> ToolStatusVisualKind.Failed
    ConversationProjectionV1.ActivityState.WaitingForPermission,
    ConversationProjectionV1.ActivityState.WaitingForInput -> ToolStatusVisualKind.Waiting
    ConversationProjectionV1.ActivityState.Sealed,
    ConversationProjectionV1.ActivityState.Cancelled -> ToolStatusVisualKind.Neutral
}

@Composable
internal fun ActivityStateBadge(state: ConversationProjectionV1.ActivityState) {
    val label = stringResource(when (state) {
        ConversationProjectionV1.ActivityState.Pending -> R.string.canonical_pending
        ConversationProjectionV1.ActivityState.Running -> R.string.canonical_running
        ConversationProjectionV1.ActivityState.WaitingForPermission -> R.string.canonical_waiting_permission
        ConversationProjectionV1.ActivityState.WaitingForInput -> R.string.canonical_waiting_input
        ConversationProjectionV1.ActivityState.Completed -> R.string.canonical_completed
        ConversationProjectionV1.ActivityState.Sealed -> R.string.canonical_sealed
        ConversationProjectionV1.ActivityState.Cancelled -> R.string.canonical_cancelled
        ConversationProjectionV1.ActivityState.Failed -> R.string.canonical_failed
    })
    val visualKind = resolveToolStatusVisualKind(state)
    val color = when (visualKind) {
        ToolStatusVisualKind.Completed -> NolaColors.success
        ToolStatusVisualKind.Failed -> MaterialTheme.colorScheme.error
        ToolStatusVisualKind.Waiting -> NolaColors.warning
        ToolStatusVisualKind.Running -> MaterialTheme.colorScheme.primary
        ToolStatusVisualKind.Neutral -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Row(
        modifier = Modifier
            .widthIn(max = 180.dp)
            .clip(RoundedCornerShape(6.dp))
            .background(color.copy(alpha = 0.14f))
            .padding(horizontal = 7.dp, vertical = 3.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when (visualKind) {
            ToolStatusVisualKind.Running -> CircularProgressIndicator(
                modifier = Modifier.size(11.dp),
                strokeWidth = 2.dp,
                color = color,
                trackColor = color.copy(alpha = 0.25f),
            )
            ToolStatusVisualKind.Completed -> Icon(
                Icons.Filled.CheckCircle,
                contentDescription = null,
                modifier = Modifier.size(12.dp),
                tint = color,
            )
            ToolStatusVisualKind.Failed -> Icon(
                Icons.Filled.Error,
                contentDescription = null,
                modifier = Modifier.size(12.dp),
                tint = color,
            )
            ToolStatusVisualKind.Waiting -> Icon(
                Icons.Filled.Warning,
                contentDescription = null,
                modifier = Modifier.size(12.dp),
                tint = color,
            )
            ToolStatusVisualKind.Neutral -> Unit
        }
        Text(
            label,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
            color = color,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

internal data class AndroidInlinePreviewRequest(
    val uri: String,
    val mimeType: String?,
    val title: String,
)

internal val LocalAndroidAttachmentPreview =
    androidx.compose.runtime.staticCompositionLocalOf<((AndroidInlinePreviewRequest) -> Unit)?> { null }

internal fun canPreviewAttachmentUri(uriString: String): Boolean {
    val trimmed = uriString.trim()
    if (trimmed.isEmpty()) return false
    return isExternalAnswerSource(trimmed) ||
        trimmed.startsWith("content://", ignoreCase = true) ||
        trimmed.startsWith("file:", ignoreCase = true) ||
        trimmed.startsWith("/")
}

internal fun openAttachmentPreview(
    context: Context,
    uriString: String,
    mimeType: String?,
    title: String,
) {
    val trimmed = uriString.trim()
    if (trimmed.isEmpty()) return
    if (isExternalAnswerSource(trimmed)) {
        openAnswerSource(context, AnswerSource(id = trimmed, sourceId = trimmed, title = title, uri = trimmed, mimeType = mimeType))
        return
    }
    runCatching {
        val targetUri: Uri = when {
            trimmed.startsWith("content://", ignoreCase = true) -> Uri.parse(trimmed)
            trimmed.startsWith("file:", ignoreCase = true) || trimmed.startsWith("/") -> {
                val path = localFilePathFromUri(trimmed) ?: return@runCatching
                FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", File(path))
            }
            else -> return@runCatching
        }
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(targetUri, mimeType?.takeIf { it.isNotBlank() } ?: "*/*")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        context.startActivity(Intent.createChooser(intent, title))
    }
}

internal enum class AttachmentPreviewTarget {
    InlineInspector,
    ExternalBrowser,
    Unsupported,
}

internal fun resolveAttachmentPreviewTarget(
    uriString: String,
    hasInlineInspector: Boolean = true,
): AttachmentPreviewTarget {
    val trimmed = uriString.trim()
    if (!canPreviewAttachmentUri(trimmed)) return AttachmentPreviewTarget.Unsupported
    if (isExternalAnswerSource(trimmed)) return AttachmentPreviewTarget.ExternalBrowser
    return if (hasInlineInspector) AttachmentPreviewTarget.InlineInspector else AttachmentPreviewTarget.ExternalBrowser
}

private const val ARTIFACT_IMAGE_MAX_BYTES = 5 * 1024 * 1024
internal const val ARTIFACT_TEXT_PREVIEW_MAX_BYTES = 128 * 1024
internal const val ARTIFACT_TEXT_TRUNCATION_NOTICE = "\n\n… [truncated at 128 KB]"

internal fun isImagePreviewUri(uriString: String, mimeType: String?): Boolean {
    val mime = mimeType?.trim()?.lowercase(java.util.Locale.ROOT).orEmpty()
    if (mime.startsWith("image/")) return true
    val lowerUri = uriString.trim().lowercase(java.util.Locale.ROOT)
        .substringBefore('?')
        .substringBefore('#')
    return lowerUri.endsWith(".png") || lowerUri.endsWith(".jpg") || lowerUri.endsWith(".jpeg") ||
        lowerUri.endsWith(".webp") || lowerUri.endsWith(".gif") || lowerUri.endsWith(".bmp") ||
        lowerUri.endsWith(".heic") || lowerUri.endsWith(".heif")
}

internal fun isImageArtifact(artifact: PartArtifact): Boolean =
    isImagePreviewUri(artifact.uri, artifact.mimeType)

private fun localFilePathFromUri(uriString: String): String? {
    val trimmed = uriString.trim()
    if (trimmed.startsWith("/")) return trimmed
    if (!trimmed.startsWith("file:", ignoreCase = true)) return null
    return runCatching { java.net.URI(trimmed).path }.getOrNull()
        ?.takeIf { it.isNotBlank() }
        ?: runCatching { Uri.parse(trimmed).path }.getOrNull()?.takeIf { it.isNotBlank() }
        ?: trimmed.removePrefix("file://").removePrefix("file:").substringBefore('?').substringBefore('#').takeIf { it.isNotBlank() }
}

internal suspend fun loadArtifactImageBitmap(context: Context, uriString: String): Bitmap? = withContext(Dispatchers.IO) {
    val trimmed = uriString.trim()
    if (trimmed.isEmpty()) return@withContext null
    runCatching {
        val bytes = when {
            openableToolUri(trimmed) != null -> {
                val safeUrl = openableToolUri(trimmed) ?: return@runCatching null
                val connection = (URL(safeUrl).openConnection() as? HttpURLConnection) ?: return@runCatching null
                connection.connectTimeout = 5_000
                connection.readTimeout = 10_000
                connection.instanceFollowRedirects = false
                try {
                    if (connection.responseCode !in 200..299) return@runCatching null
                    val contentType = connection.contentType?.lowercase(java.util.Locale.ROOT).orEmpty()
                    if (contentType.isNotBlank() && !contentType.startsWith("image/")) return@runCatching null
                    readArtifactBoundedBytes(connection.inputStream, ARTIFACT_IMAGE_MAX_BYTES)
                } finally {
                    connection.disconnect()
                }
            }
            trimmed.startsWith("content://", ignoreCase = true) -> {
                context.contentResolver.openInputStream(Uri.parse(trimmed))?.use {
                    readArtifactBoundedBytes(it, ARTIFACT_IMAGE_MAX_BYTES)
                }
            }
            trimmed.startsWith("file:", ignoreCase = true) || trimmed.startsWith("/") -> {
                val path = localFilePathFromUri(trimmed) ?: return@runCatching null
                val file = File(path)
                if (!file.isFile || file.length() > ARTIFACT_IMAGE_MAX_BYTES) return@runCatching null
                file.inputStream().use { readArtifactBoundedBytes(it, ARTIFACT_IMAGE_MAX_BYTES) }
            }
            else -> null
        } ?: return@runCatching null
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
    }.getOrNull()
}

private fun readArtifactBoundedBytes(stream: java.io.InputStream, maxBytes: Int): ByteArray? {
    val output = ByteArrayOutputStream()
    val buffer = ByteArray(8 * 1024)
    var total = 0
    while (true) {
        val read = stream.read(buffer)
        if (read < 0) break
        total += read
        if (total > maxBytes) return null
        output.write(buffer, 0, read)
    }
    return output.toByteArray()
}

private fun readPreviewTextBoundedBytes(stream: java.io.InputStream, maxBytes: Int): Pair<ByteArray, Boolean> {
    val output = ByteArrayOutputStream()
    val buffer = ByteArray(8 * 1024)
    var total = 0
    var truncated = false
    while (true) {
        val remaining = maxBytes - total
        if (remaining <= 0) {
            if (stream.read() >= 0) truncated = true
            break
        }
        val toRead = minOf(buffer.size, remaining)
        val read = stream.read(buffer, 0, toRead)
        if (read < 0) break
        output.write(buffer, 0, read)
        total += read
    }
    return output.toByteArray() to truncated
}

private fun looksLikeBinaryPayload(bytes: ByteArray): Boolean {
    if (bytes.isEmpty()) return false
    if (bytes.size >= 8 &&
        bytes[0] == 0x89.toByte() && bytes[1] == 0x50.toByte() &&
        bytes[2] == 0x4E.toByte() && bytes[3] == 0x47.toByte() &&
        bytes[4] == 0x0D.toByte() && bytes[5] == 0x0A.toByte() &&
        bytes[6] == 0x1A.toByte() && bytes[7] == 0x0A.toByte()
    ) return true
    if (bytes.size >= 5 &&
        bytes[0] == '%'.code.toByte() && bytes[1] == 'P'.code.toByte() &&
        bytes[2] == 'D'.code.toByte() && bytes[3] == 'F'.code.toByte() &&
        bytes[4] == '-'.code.toByte()
    ) return true
    val sampleSize = minOf(bytes.size, 8 * 1024)
    for (index in 0 until sampleSize) {
        if (bytes[index] == 0.toByte()) return true
    }
    return false
}

private fun trimTrailingIncompleteUtf8Sequence(bytes: ByteArray): ByteArray {
    if (bytes.isEmpty()) return bytes
    var end = bytes.size
    var continuationCount = 0
    while (end > 0 && continuationCount < 4) {
        val b = bytes[end - 1].toInt() and 0xFF
        if ((b and 0xC0) == 0x80) {
            continuationCount++
            end--
            continue
        }
        val expectedLength = when {
            (b and 0x80) == 0 -> 1
            (b and 0xE0) == 0xC0 -> 2
            (b and 0xF0) == 0xE0 -> 3
            (b and 0xF8) == 0xF0 -> 4
            else -> return bytes.copyOf(end - 1)
        }
        val actualLength = continuationCount + 1
        return if (actualLength < expectedLength) bytes.copyOf(end - 1) else bytes
    }
    return bytes.copyOf(end)
}

internal suspend fun loadAttachmentPreviewText(
    context: Context?,
    uriString: String,
    mimeType: String?,
): String? = withContext(Dispatchers.IO) {
    val trimmed = uriString.trim()
    if (trimmed.isEmpty() || isImagePreviewUri(trimmed, mimeType)) return@withContext null
    val mime = mimeType?.trim()?.lowercase(java.util.Locale.ROOT).orEmpty()
    val lowerUri = trimmed.lowercase(java.util.Locale.ROOT).substringBefore('?').substringBefore('#')
    val looksText = mime.startsWith("text/") ||
        mime.contains("json") || mime.contains("xml") || mime.contains("yaml") || mime.contains("javascript") ||
        listOf(".txt", ".md", ".markdown", ".json", ".yaml", ".yml", ".toml", ".xml", ".csv", ".log",
            ".rs", ".kt", ".kts", ".java", ".swift", ".ts", ".tsx", ".js", ".jsx", ".py", ".sh", ".sql", ".html", ".css")
            .any { lowerUri.endsWith(it) }
    if (!looksText) return@withContext null
    runCatching {
        val bounded = when {
            trimmed.startsWith("content://", ignoreCase = true) -> {
                context?.contentResolver?.openInputStream(Uri.parse(trimmed))?.use {
                    readPreviewTextBoundedBytes(it, ARTIFACT_TEXT_PREVIEW_MAX_BYTES)
                }
            }
            trimmed.startsWith("file:", ignoreCase = true) || trimmed.startsWith("/") -> {
                val path = localFilePathFromUri(trimmed) ?: return@runCatching null
                val file = File(path)
                if (!file.isFile) return@runCatching null
                file.inputStream().use { readPreviewTextBoundedBytes(it, ARTIFACT_TEXT_PREVIEW_MAX_BYTES) }
            }
            else -> null
        } ?: return@runCatching null
        val (rawBytes, truncated) = bounded
        if (looksLikeBinaryPayload(rawBytes)) return@runCatching null
        val safeBytes = if (truncated) trimTrailingIncompleteUtf8Sequence(rawBytes) else rawBytes
        val decoded = safeBytes.toString(Charsets.UTF_8)
        if (truncated) decoded + ARTIFACT_TEXT_TRUNCATION_NOTICE else decoded
    }.getOrNull()
}

@Composable
private fun ArtifactCard(artifact: PartArtifact) {
    val context = LocalContext.current
    val inlinePreview = LocalAndroidAttachmentPreview.current
    val canOpen = canPreviewAttachmentUri(artifact.uri)
    val openInInspectorOrExternal = {
        val req = AndroidInlinePreviewRequest(artifact.uri, artifact.mimeType, artifact.name)
        when (resolveAttachmentPreviewTarget(artifact.uri, hasInlineInspector = inlinePreview != null)) {
            AttachmentPreviewTarget.InlineInspector -> inlinePreview?.invoke(req)
            AttachmentPreviewTarget.ExternalBrowser -> openAttachmentPreview(context, artifact.uri, artifact.mimeType, artifact.name)
            AttachmentPreviewTarget.Unsupported -> Unit
        }
    }
    val showImagePreview = canOpen && isImageArtifact(artifact)
    var previewBitmap by remember(artifact.uri) { mutableStateOf<Bitmap?>(null) }
    if (showImagePreview) {
        LaunchedEffect(artifact.uri) {
            previewBitmap = loadArtifactImageBitmap(context, artifact.uri)
        }
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        previewBitmap?.let { bitmap ->
            Image(
                bitmap = bitmap.asImageBitmap(),
                contentDescription = artifact.name,
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 220.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .clickable(role = Role.Button) {
                        openInInspectorOrExternal()
                    },
                contentScale = ContentScale.Fit,
            )
        }
        val titleModifier = if (canOpen) Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = Role.Button) {
            openInInspectorOrExternal()
        } else Modifier
        Text(artifact.name, titleModifier, style = MaterialTheme.typography.labelMedium,
            color = if (canOpen) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface)
        val metadata = listOfNotNull(artifact.mimeType, artifact.operation).filter { it.isNotBlank() }.joinToString(" · ")
        if (metadata.isNotEmpty()) Text(metadata, style = MaterialTheme.typography.labelSmall)
    }
}

@Composable
private fun PartCard(content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f), RoundedCornerShape(12.dp))
        .border(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f), RoundedCornerShape(12.dp)).padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)) { content() }
}

@Composable
private fun Fact(partId: String, field: CanonicalPartReadField, onReadText: (CanonicalPartTextSnapshot) -> Unit,
    label: String, value: String, code: Boolean = false) {
    Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    if (value.length > 2000) {
        TextButton(onClick = { onReadText(CanonicalPartTextSnapshot(partId, field, label, value, code)) }) {
            Text(stringResource(R.string.canonical_reader_open))
        }
    }
    SelectionContainer {
        Text(if (value.length > 2000) value.take(2000) + "…" else value,
            style = MaterialTheme.typography.bodySmall, fontFamily = if (code) FontFamily.Monospace else null)
    }
}
