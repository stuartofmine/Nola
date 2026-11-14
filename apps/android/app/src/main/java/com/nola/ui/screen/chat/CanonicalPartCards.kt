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
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.Icon
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import com.nola.R
import com.nola.ui.components.ThinkingBody
import com.nola.ui.components.thinkingExpandedAfterStatusChange
import com.nola.ui.components.ThinkingBodyVisibleCharactersKey
import androidx.compose.ui.semantics.semantics
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
        is ConversationProjectionV1.Part.Tool -> current.title.takeIf {
            current.state in setOf(ConversationProjectionV1.ActivityState.Running, ConversationProjectionV1.ActivityState.Pending)
        }
        is ConversationProjectionV1.Part.Reasoning -> if (current.state == ConversationProjectionV1.ActivityState.Running)
            stringResource(R.string.chat_thinking) else null
        else -> null
    }
    Column(Modifier.fillMaxWidth()
        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f), RoundedCornerShape(12.dp))
        .padding(horizontal = 12.dp)) {
        TextButton(onClick = { onChoicesChanged(choices.expanded(activity, !expanded)) },
            colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onSurfaceVariant),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
            Text(executionProcessHeading(isLive, currentTitle, stringResource(R.string.chat_processing), stringResource(heading)), modifier = Modifier.weight(1f), style = MaterialTheme.typography.labelLarge,
                maxLines = 1, overflow = TextOverflow.Ellipsis)
            NolaDisclosureChevron(expanded, reduceMotion)
        }
        AnimatedVisibility(visible = expanded, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
            Column(Modifier.padding(bottom = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                visible.forEachIndexed { index, part -> key(part.partId) {
                    if (visible.size > 1) Text("${index + 1}", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    when (part) {
                        is ConversationProjectionV1.Part.Reasoning -> ThinkingBody(part.summary.orEmpty(),
                            Modifier.testTag("assistant.reasoning.block").semantics {
                                this[ThinkingBodyVisibleCharactersKey] = part.summary.orEmpty().length
                            })
                        is ConversationProjectionV1.Part.Tool -> CanonicalActivityRow(part, details = part.partId in choices.detailsPartIds,
                            onDetailsChanged = { onChoicesChanged(choices.details(activity, part.partId, it)) }, onReadText = onReadText)
                        else -> Unit
                    }
                } }
            }
        }
    }
}

/** Error parts expose only typed code/display-key facts, never a display body.
 * Unknown keys therefore use the named local failure state at this exact anchor. */
@Composable
internal fun CanonicalErrorPart(part: ConversationProjectionV1.Part.Error, onReadText: (CanonicalPartTextSnapshot) -> Unit) {
    var details by remember(part.partId, part.code, part.displayKey) { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().testTag("chat.error.${part.partId}")) {
        Text(stringResource(R.string.canonical_part_error), color = MaterialTheme.colorScheme.error)
        TextButton(onClick = { details = !details }) {
            Text(stringResource(if (details) R.string.canonical_hide_details else R.string.canonical_details))
        }
        if (details) {
            Fact(part.partId, CanonicalPartReadField.ErrorIdentity, onReadText, stringResource(R.string.canonical_identity), part.partId)
            Fact(part.partId, CanonicalPartReadField.ErrorMetadata, onReadText, stringResource(R.string.canonical_preview), buildJsonObject {
                put("code", part.code); put("display_key", part.displayKey); put("recoverable", part.recoverable)
            }.toString(), code = true)
        }
    }
}

@Composable
private fun CanonicalActivityRow(part: ConversationProjectionV1.Part.Tool, details: Boolean, onDetailsChanged: (Boolean) -> Unit, onReadText: (CanonicalPartTextSnapshot) -> Unit) {
    val reduceMotion = rememberReduceMotionEnabled()
    val facts = part.cardFacts()
    Column(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(facts.title.ifEmpty { stringResource(R.string.chat_tool_activity) }, Modifier.weight(1f), style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
            ActivityStateBadge(facts.state)
        }
        facts.summary?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
        facts.artifacts.forEach { ArtifactCard(it) }
        TextButton(onClick = { onDetailsChanged(!details) }) {
            Text(stringResource(if (details) R.string.canonical_hide_details else R.string.canonical_details))
        }
        AnimatedVisibility(visible = details, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(listOfNotNull(facts.capabilityId, facts.origin.providerTitle, facts.origin.namespace).joinToString(" · "),
                    style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                facts.permission?.let { permission ->
                    Fact(part.partId, CanonicalPartReadField.ToolPermission, onReadText, stringResource(R.string.canonical_permission_record), listOfNotNull(permission.rationale, permission.permissionDomain, permission.decision).joinToString(" · "))
                }
                Fact(part.partId, CanonicalPartReadField.ToolInput, onReadText, stringResource(R.string.canonical_input), facts.input.toString(), code = true)
                facts.result?.let { Fact(part.partId, CanonicalPartReadField.ToolResult, onReadText, stringResource(R.string.canonical_result), it.toString(), code = true) }
                facts.preview?.let { Fact(part.partId, CanonicalPartReadField.ToolPreview, onReadText, stringResource(R.string.canonical_preview), it, code = true) }
                Fact(part.partId, CanonicalPartReadField.ToolOrigin, onReadText, stringResource(R.string.canonical_origin), listOfNotNull(facts.origin.kind, facts.origin.providerId, facts.origin.providerTitle, facts.origin.namespace).joinToString(" · "))
                facts.permission?.let { Fact(part.partId, CanonicalPartReadField.ToolPermissionIdentity, onReadText, stringResource(R.string.canonical_permission_record), it.requestId) }
                facts.startedAtMs?.let { Fact(part.partId, CanonicalPartReadField.ToolStarted, onReadText, stringResource(R.string.canonical_started), it.toString()) }
                facts.finishedAtMs?.let { Fact(part.partId, CanonicalPartReadField.ToolFinished, onReadText, stringResource(R.string.canonical_finished), it.toString()) }
                // Owner facts are selectable display only; there is intentionally no open/download action.
                facts.fullOutput?.let { owner -> Fact(part.partId, CanonicalPartReadField.ToolOutputOwner, onReadText, stringResource(R.string.canonical_retained_output),
                    listOf(owner.messageId, owner.partId, owner.toolCallId).joinToString(" · ")) }
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
                        facts.range?.let { Text("[${it.start}, ${it.end})") }
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
    val color = when (state) {
        ConversationProjectionV1.ActivityState.Failed -> MaterialTheme.colorScheme.error
        ConversationProjectionV1.ActivityState.WaitingForPermission,
        ConversationProjectionV1.ActivityState.WaitingForInput -> MaterialTheme.colorScheme.tertiary
        ConversationProjectionV1.ActivityState.Running,
        ConversationProjectionV1.ActivityState.Completed -> MaterialTheme.colorScheme.primary
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Text(label, Modifier.widthIn(max = 160.dp).background(color.copy(alpha = 0.08f), RoundedCornerShape(6.dp))
        .padding(horizontal = 6.dp, vertical = 3.dp),
        style = MaterialTheme.typography.labelSmall, color = color)
}

@Composable
private fun ArtifactCard(artifact: PartArtifact) {
    val context = LocalContext.current
    val canOpen = isExternalAnswerSource(artifact.uri)
    val titleModifier = if (canOpen) Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(role = androidx.compose.ui.semantics.Role.Button) {
        openAnswerSource(context, AnswerSource(id = artifact.uri, sourceId = artifact.uri, title = artifact.name, uri = artifact.uri, mimeType = artifact.mimeType))
    } else Modifier
    Text(artifact.name, titleModifier, style = MaterialTheme.typography.labelMedium,
        color = if (canOpen) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface)
    val metadata = listOfNotNull(artifact.mimeType, artifact.operation).filter { it.isNotBlank() }.joinToString(" · ")
    if (metadata.isNotEmpty()) Text(metadata, style = MaterialTheme.typography.labelSmall)
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
