package com.nola.ui.components

import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.ExpandLess

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.PlainTooltip
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.Composable


import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.util.hostToolContainsSensitiveData
import com.nola.util.httpToolUriOrNull
import com.nola.util.redactHostToolText
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL

/** Native hover/long-press help; the caller keeps its labelled SVG content. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NolaIconButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable () -> Unit,
) {
    TooltipBox(
        positionProvider = TooltipDefaults.rememberPlainTooltipPositionProvider(),
        tooltip = { PlainTooltip { Text(label) } },
        state = rememberTooltipState(),
    ) { IconButton(onClick = onClick, modifier = modifier, enabled = enabled, content = content) }
}

internal fun openableToolUri(raw: String?): String? {
    return httpToolUriOrNull(raw)
}

private const val HOST_TOOL_SCREENSHOT_MAX_BYTES = 5 * 1024 * 1024

private data class HostToolScreenshotLoad(
    val loading: Boolean = true,
    val bitmap: Bitmap? = null
)

internal fun hostToolStatusStringRes(status: String, error: Boolean): Int? {
    val normalized = status.trim().lowercase()
    return when {
        normalized == "permission_required" || normalized == "permissionrequired" -> R.string.chat_host_tool_status_permission_required
        error || normalized == "error" || normalized == "failed" -> R.string.chat_host_tool_status_failed
        normalized == "running" -> R.string.chat_host_tool_status_running
        normalized == "done" || normalized == "completed" || normalized == "complete" ||
            normalized == "finished" || normalized == "succeeded" || normalized == "success" ->
            R.string.chat_host_tool_status_succeeded
        else -> null
    }
}

internal fun hostToolDisplayNameStringRes(toolName: String): Int? {
    val normalized = toolName.trim().lowercase().replace(Regex("[\\s-]+"), "_")
    return when (normalized) {
        "bash", "shell", "run_command", "execute_command", "bash_command_review",
        "bash_background_status", "bash_background_wait", "bash_background_stop", "terminal_input" -> R.string.chat_host_tool_name_command
        "read_file", "read_text_file", "read_many_files", "notebook_read", "file_outline",
        "read_symbol", "device_read_file" -> R.string.chat_host_tool_name_read_file
        "write_file", "create_file", "device_write_file" -> R.string.chat_host_tool_name_write_file
        "edit_file", "edit_file_lines", "multi_edit_file", "notebook_edit", "apply_patch",
        "replace_text", "replace_symbol_body", "replace_in_files", "rename_symbol", "format_files",
        "move_file" -> R.string.chat_host_tool_name_edit_file
        "delete_file", "remove_file" -> R.string.chat_host_tool_name_delete_file
        "list_dir", "list_directory", "file_tree", "find_files", "find_up" -> R.string.chat_host_tool_name_list_folder
        "project_overview", "repo_map" -> R.string.chat_host_tool_name_project_overview
        "grep", "glob", "search_files", "ripgrep", "code_search", "workspace_symbols",
        "symbol_search", "implementation_search", "code_navigation", "language_service_query", "language_service_list", "call_graph", "failure_context",
        "failure_triage", "file_dependencies", "related_files", "document_search" -> R.string.chat_host_tool_name_search_files
        "workspace_status", "code_diagnostics", "coding_conventions", "instruction_context",
        "agent_context", "command_context", "agent_config_context", "ci_context",
        "context_pack", "focused_context", "impact_analysis", "validation_plan",
        "test_context", "format_plan", "verification_review", "handoff_context", "workspace_checkpoint" -> R.string.chat_host_tool_name_workspace
        "git_diff", "change_review", "diff_review", "changed_symbols", "git_history", "git_blame",
        "git_show" -> R.string.chat_host_tool_name_changes
        "web_search" -> R.string.chat_host_tool_name_web_search
        "web_fetch", "fetch_url" -> R.string.chat_host_tool_name_web_read
        "tool_search" -> R.string.chat_host_tool_name_search_tools
        "tool_output_list", "tool_output_read", "tool_output_search", "tool_output_find" -> R.string.chat_host_tool_name_read_output
        "memory_save", "memory_search" -> R.string.chat_host_tool_name_memory
        "spawn_agent", "nola_orchestrate" -> R.string.chat_host_tool_name_subtask
        "list_tasks", "read_task", "wait_task", "stop_task" -> R.string.chat_host_tool_name_background
        "propose_agent_plan", "update_agent_todos", "agent_plan_file" -> R.string.chat_host_tool_name_plan
        "system_time" -> R.string.chat_host_tool_name_time
        "device_clipboard_read", "device_clipboard_write" -> R.string.chat_host_tool_name_clipboard
        "device_search_calendar_events", "search_calendar_events" -> R.string.chat_host_tool_name_search_calendar
        "device_create_calendar_event", "create_calendar_event" -> R.string.chat_host_tool_name_create_calendar_event
        "search_reminders" -> R.string.chat_host_tool_name_search_reminders
        "create_reminder" -> R.string.chat_host_tool_name_create_reminder
        "update_reminder" -> R.string.chat_host_tool_name_update_reminder
        "device_get_location" -> R.string.chat_host_tool_name_location
        "skill", "skill_context" -> R.string.chat_host_tool_name_skill
        else -> when {
            normalized.startsWith("chrome_") || normalized.startsWith("browser_") -> R.string.chat_host_tool_name_browser
            normalized.startsWith("computer_") -> R.string.chat_host_tool_name_computer
            normalized.startsWith("skill_") -> R.string.chat_host_tool_name_skill
            normalized.startsWith("mcp_") -> R.string.chat_host_tool_name_mcp
            normalized.startsWith("background_") -> R.string.chat_host_tool_name_background
            else -> null
        }
    }
}

@Composable
internal fun hostToolDisplayName(toolName: String): String {
    val normalized = toolName.trim()
    val resource = hostToolDisplayNameStringRes(normalized)
    return when {
        resource != null -> stringResource(resource)
        normalized.isBlank() -> stringResource(R.string.chat_host_tool_name_generic)
        else -> stringResource(R.string.chat_host_tool_name_unknown_format, normalized)
    }
}

private fun hostToolIsRunning(status: String): Boolean {
    return status.trim().equals("running", ignoreCase = true)
}

private fun hostToolNeedsPermission(status: String): Boolean {
    val normalized = status.trim().lowercase()
    return normalized == "permission_required" || normalized == "permissionrequired"
}

internal fun hostToolIsBrowserTool(toolName: String): Boolean {
    val normalized = toolName.trim()
    return normalized.startsWith("chrome_native_")
}

internal fun hostToolIsComputerTool(toolName: String): Boolean {
    return toolName.trim().startsWith("computer_")
}

internal fun hostToolShouldShowComputerScreenshotHint(
    toolName: String,
    containsSensitiveData: Boolean,
    screenshotUri: String?
): Boolean {
    return !containsSensitiveData && screenshotUri != null && hostToolIsComputerTool(toolName)
}

@Composable
private fun hostToolStatusText(status: String, error: Boolean): String {
    val resId = hostToolStatusStringRes(status, error)
    if (resId != null) return stringResource(resId)

    val fallback = status.trim()
    return if (fallback.isBlank()) stringResource(R.string.chat_host_tool_status_succeeded) else fallback
}

@Composable
private fun hostToolTint(status: String, error: Boolean): Color {
    val normalized = status.trim().lowercase()
    return when {
        normalized == "running" -> MaterialTheme.colorScheme.primary
        normalized == "permission_required" || normalized == "permissionrequired" -> NolaColors.warning
        error || normalized == "error" || normalized == "failed" -> MaterialTheme.colorScheme.error
        else -> NolaColors.success
    }
}

internal fun thinkingExpandedAfterStatusChange(current: Boolean, finished: Boolean, userChangedExpansion: Boolean): Boolean =
    if (finished || userChangedExpansion) current else true

internal val ThinkingBodyVisibleCharactersKey = SemanticsPropertyKey<Int>("ThinkingBodyVisibleCharacters")

@Composable
fun ThinkingBlock(
    thinking: String,
    finished: Boolean,
    modifier: Modifier = Modifier,
    initiallyExpanded: Boolean = !finished,
) {
    val haptics = rememberNolaHaptics()
    val reduceMotion = rememberReduceMotionEnabled()
    val isWaiting = thinking.isBlank() && !finished
    var expanded by remember { mutableStateOf(initiallyExpanded) }
    var userChangedExpansion by remember { mutableStateOf(false) }
    LaunchedEffect(finished) {
        // Completion must not remove content height while the user is reading.
        // Restored history still initializes collapsed above.
        expanded = thinkingExpandedAfterStatusChange(expanded, finished, userChangedExpansion)
    }
    val pulseAlpha = if (
        continuousStatusAnimationEnabled(active = !finished, reduceMotion = reduceMotion)
    ) {
        val pulseTransition = rememberInfiniteTransition(label = "assistant-state-pulse")
        val animatedAlpha by pulseTransition.animateFloat(
            initialValue = 0.12f,
            targetValue = 0.28f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 1_450),
                repeatMode = RepeatMode.Reverse
            ),
            label = "assistant-state-alpha"
        )
        animatedAlpha
    } else {
        0.20f
    }
    val disclosureRotation by animateFloatAsState(
        targetValue = if (expanded) 180f else 0f,
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.disclosure,
            easing = NolaMotion.disclosureEasing,
        ),
        label = "thinking-disclosure-rotation"
    )
    AnimatedVisibility(
        visible = thinking.isNotEmpty() || isWaiting,
        modifier = modifier.testTag("assistant.reasoning.block").semantics {
            this[ThinkingBodyVisibleCharactersKey] = if (expanded) thinking.length else 0
        },
        enter = nolaDisclosureEnter(reduceMotion),
        exit = nolaDisclosureExit(reduceMotion)
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(NolaRadius.lg))
                .background(if (expanded) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent)
                .border(
                    BorderStroke(
                        0.5.dp,
                        if (expanded) MaterialTheme.colorScheme.outlineVariant else Color.Transparent
                    ),
                    RoundedCornerShape(NolaRadius.lg)
                )
                .padding(horizontal = 10.dp, vertical = 5.dp)
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(enabled = thinking.isNotBlank()) {
                        haptics.selection()
                        userChangedExpansion = true
                        expanded = !expanded
                    }
                    .heightIn(min = 48.dp)
                    .padding(horizontal = 4.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                if (finished) {
                    Box(
                        modifier = Modifier
                            .size(12.dp),
                        contentAlignment = Alignment.Center
                    ) {
                        Box(
                            modifier = Modifier
                                .size(7.dp)
                                .clip(CircleShape)
                                .background(MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.46f))
                        )
                    }
                } else {
                    Box(
                        modifier = Modifier
                            .size(12.dp)
                            .clip(CircleShape)
                            .background(MaterialTheme.colorScheme.primary.copy(alpha = pulseAlpha)),
                        contentAlignment = Alignment.Center
                    ) {
                        Box(
                            modifier = Modifier
                                .size(6.dp)
                                .clip(CircleShape)
                                .background(MaterialTheme.colorScheme.primary)
                        )
                    }
                }
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = when {
                        finished -> stringResource(R.string.thinking_process)
                        isWaiting -> stringResource(R.string.chat_processing)
                        else -> stringResource(R.string.chat_thinking)
                    },
                    style = MaterialTheme.typography.labelMedium,
                    color = if (finished) {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    } else {
                        MaterialTheme.colorScheme.onSurface
                    },
                    modifier = Modifier.weight(1f)
                )
                if (thinking.isNotBlank()) {
                    Icon(
                        imageVector = Icons.Filled.ExpandMore,
                        contentDescription = null,
                        modifier = Modifier
                            .size(18.dp)
                            .graphicsLayer { rotationZ = disclosureRotation },
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            AnimatedVisibility(
                visible = expanded && thinking.isNotBlank(),
                enter = nolaDisclosureEnter(reduceMotion),
                exit = nolaDisclosureExit(reduceMotion)
            ) {
                ThinkingBody(thinking, Modifier.padding(start = 20.dp, end = 6.dp, bottom = 10.dp))
            }
        }
    }
}

/** Shared public reasoning body. Disclosure belongs to its owning presentation. */
@Composable
internal fun ThinkingBody(thinking: String, modifier: Modifier = Modifier) {
    Text(text = thinking, modifier = modifier, style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.88f))
}

@Composable
fun ContextUsageIndicator(
    usagePercent: Float,
    totalTokens: Int,
    maxTokens: Int,
    modifier: Modifier = Modifier
) {
    Column(modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text(
                text = stringResource(R.string.context),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Text(
                text = "${totalTokens} / $maxTokens (${usagePercent.toInt()}%)",
                style = MaterialTheme.typography.labelSmall,
                color = if (usagePercent > 80f)
                    MaterialTheme.colorScheme.error
                else
                    MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Spacer(modifier = Modifier.height(2.dp))
        LinearProgressIndicator(
            progress = { (usagePercent / 100f).coerceIn(0f, 1f) },
            modifier = Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)),
            color = when {
                usagePercent > 90f -> MaterialTheme.colorScheme.error
                usagePercent > 80f -> MaterialTheme.colorScheme.tertiary
                else -> MaterialTheme.colorScheme.primary
            }
        )
    }
}

@Composable
fun HostToolCard(
    toolName: String,
    status: String,
    modifier: Modifier = Modifier,
    argsPreview: String = "",
    resultSummary: String? = null,
    detailPreview: String? = null,
    fullOutputRef: String? = null,
    recovery: String? = null,
    nextAction: String? = null,
    screenshotUrl: String? = null,
    finalUrl: String? = null,
    pageTitle: String? = null,
    downloadUrl: String? = null,
    downloadFilename: String? = null,
    screenshotWarning: String? = null,
    sensitive: Boolean = false,
    error: Boolean = false
) {
    var expanded by remember { mutableStateOf(false) }
    val uriHandler = LocalUriHandler.current
    val reduceMotion = rememberReduceMotionEnabled()
    val screenshotUri = openableToolUri(screenshotUrl)
    val finalUri = openableToolUri(finalUrl)
    val downloadUri = openableToolUri(downloadUrl)
    val containsSensitiveData = sensitive || hostToolContainsSensitiveData(
        argsPreview,
        resultSummary,
        detailPreview,
        fullOutputRef,
        nextAction,
        screenshotUrl,
        finalUrl,
        pageTitle,
        downloadUrl,
        downloadFilename,
        screenshotWarning
    )
    val redactedSummary = redactHostToolText(resultSummary)
    val redactedDetail = redactHostToolText(detailPreview).trim().take(8_000)
    val visibleFullOutputRef = redactHostToolText(fullOutputRef).trim()
    val recoveryGuidance = when (recovery) {
        "retry_idempotent" -> stringResource(R.string.chat_host_tool_recovery_retry)
        "inspect_then_retry" -> stringResource(R.string.chat_host_tool_recovery_inspect)
        "recovery_decision_required" -> stringResource(R.string.chat_host_tool_recovery_review)
        "correct_or_alternate" -> stringResource(R.string.chat_host_tool_recovery_correct)
        else -> redactHostToolText(nextAction).trim()
    }
    val redactedArgs = redactHostToolText(argsPreview).trim()
    val fileSummary = hostToolFileSummary(redactedArgs)
    val isFilePayloadTool = hostToolCarriesFilePayload(toolName, redactedArgs)
    val visibleSummary = if (isFilePayloadTool) fileSummary else redactedSummary.take(1200)
    val visibleArgs = if (isFilePayloadTool) "" else redactedArgs.take(1200)
    val redactedScreenshotWarning = redactHostToolText(screenshotWarning).takeIf { it.isNotBlank() }
    val tint = hostToolTint(status, error)
    val displayStatus = hostToolStatusText(status, error)
    val displayName = hostToolDisplayName(toolName)
    val icon = when {
        error || hostToolNeedsPermission(status) -> Icons.Filled.Warning
        else -> hostToolIcon(toolName)
    }
    Card(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 2.dp, vertical = 4.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.22f),
            contentColor = MaterialTheme.colorScheme.onSurface
        ),
        border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outline.copy(alpha = 0.14f)),
        shape = RoundedCornerShape(NolaRadius.lg)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(role = androidx.compose.ui.semantics.Role.Button) { expanded = !expanded }
                .heightIn(min = 48.dp)
                .padding(horizontal = 10.dp, vertical = 9.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            HostToolStatusGlyph(
                icon = icon,
                tint = tint,
                running = hostToolIsRunning(status),
                reduceMotion = reduceMotion
            )
            Spacer(modifier = Modifier.width(9.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = displayName,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
                Text(
                    text = displayStatus,
                    style = MaterialTheme.typography.labelSmall,
                    color = tint,
                    fontWeight = FontWeight.SemiBold
                )
                if (!containsSensitiveData && visibleSummary.isNotBlank()) {
                    Spacer(modifier = Modifier.height(2.dp))
                    Text(
                        text = visibleSummary,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.8f),
                        maxLines = if (expanded) 6 else 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.animateContentSize(tween(if (reduceMotion) 0 else NolaMotion.disclosure,
                            easing = NolaMotion.disclosureEasing))
                    )
                }
                AnimatedVisibility(visible = expanded, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
                    Column {
                        if (!containsSensitiveData && fileSummary.isNotBlank()) {
                            HostToolDetailBlock(
                                title = stringResource(R.string.chat_host_tool_files),
                                text = fileSummary
                            )
                        }
                        if (!containsSensitiveData && visibleArgs.isNotBlank()) {
                            HostToolDetailBlock(
                                title = stringResource(R.string.chat_host_tool_parameters),
                                text = visibleArgs
                            )
                        }
                        if (!containsSensitiveData && redactedDetail.isNotBlank()) {
                            HostToolDetailBlock(
                                title = stringResource(R.string.chat_host_tool_result),
                                text = redactedDetail
                            )
                        }
                        if (!containsSensitiveData && visibleFullOutputRef.isNotBlank()) {
                            HostToolDetailBlock(
                                title = stringResource(R.string.chat_host_tool_full_output),
                                text = visibleFullOutputRef
                            )
                        }
                        if (!containsSensitiveData && recoveryGuidance.isNotBlank()) {
                            HostToolDetailBlock(
                                title = stringResource(R.string.chat_host_tool_recovery_title),
                                text = recoveryGuidance
                            )
                        }
                        if (redactedScreenshotWarning != null) {
                            Spacer(modifier = Modifier.height(4.dp))
                            Text(
                                text = stringResource(R.string.chat_host_tool_screenshot_warning, redactedScreenshotWarning),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.tertiary
                            )
                        }
                        if (containsSensitiveData) {
                            Spacer(modifier = Modifier.height(6.dp))
                            Text(
                                text = stringResource(R.string.chat_host_tool_sensitive_result_hidden),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.tertiary
                            )
                        }
                        if (hostToolShouldShowComputerScreenshotHint(toolName, containsSensitiveData, screenshotUri)) {
                            Spacer(modifier = Modifier.height(6.dp))
                            Text(
                                text = stringResource(R.string.chat_host_tool_computer_screenshot_hint),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        if (!containsSensitiveData && screenshotUri != null) {
                            Spacer(modifier = Modifier.height(8.dp))
                            HostToolScreenshotPreview(
                                screenshotUri = screenshotUri,
                                onOpen = { runCatching { uriHandler.openUri(screenshotUri) } }
                            )
                        }
                        if (!containsSensitiveData && (screenshotUri != null || finalUri != null || downloadUri != null)) {
                            Spacer(modifier = Modifier.height(6.dp))
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(8.dp)
                            ) {
                                if (screenshotUri != null) {
                                    TextButton(
                                        modifier = Modifier.weight(1f),
                                        onClick = { runCatching { uriHandler.openUri(screenshotUri) } }
                                    ) {
                                        Text(
                                            text = stringResource(R.string.chat_host_tool_open_screenshot),
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis
                                        )
                                    }
                                }
                                if (finalUri != null) {
                                    TextButton(
                                        modifier = Modifier.weight(1f),
                                        onClick = { runCatching { uriHandler.openUri(finalUri) } }
                                    ) {
                                        Text(
                                            text = pageTitle?.takeIf { it.isNotBlank() } ?: stringResource(R.string.chat_host_tool_open_url),
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis
                                        )
                                    }
                                }
                                if (downloadUri != null) {
                                    TextButton(
                                        modifier = Modifier.weight(1f),
                                        onClick = { runCatching { uriHandler.openUri(downloadUri) } }
                                    ) {
                                        Text(
                                            text = downloadFilename?.takeIf { it.isNotBlank() }
                                                ?: stringResource(R.string.chat_host_tool_open_download),
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
            NolaDisclosureChevron(expanded, reduceMotion, Modifier.size(18.dp),
                stringResource(if (expanded) R.string.common_collapse else R.string.common_expand))
        }
    }
}

private fun hostToolIcon(toolName: String): ImageVector {
    val normalized = toolName.trim().lowercase()
    return when {
        hostToolIsBrowserTool(toolName) || normalized.contains("search") || normalized.contains("web") -> Icons.Filled.Public
        normalized.contains("file") || normalized.contains("read") || normalized.contains("write") -> Icons.Filled.Description
        else -> Icons.Filled.Computer
    }
}

@Composable
private fun HostToolStatusGlyph(
    icon: ImageVector,
    tint: Color,
    running: Boolean,
    reduceMotion: Boolean
) {
    val pulse = if (continuousStatusAnimationEnabled(running, reduceMotion)) {
        val transition = rememberInfiniteTransition(label = "tool-status")
        val animatedPulse by transition.animateFloat(
            initialValue = 0.88f,
            targetValue = 1.12f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 1_300),
                repeatMode = RepeatMode.Reverse
            ),
            label = "tool-status-pulse"
        )
        animatedPulse
    } else {
        1f
    }
    Box(modifier = Modifier.size(28.dp), contentAlignment = Alignment.Center) {
        Box(
            modifier = Modifier
                .size(26.dp)
                .graphicsLayer {
                    scaleX = if (running) pulse else 1f
                    scaleY = if (running) pulse else 1f
                    alpha = if (running) 0.13f else 0.09f
                }
                .clip(CircleShape)
                .background(tint)
        )
        Icon(
            imageVector = icon,
            contentDescription = null,
            modifier = Modifier.size(15.dp),
            tint = tint
        )
    }
}

internal fun continuousStatusAnimationEnabled(active: Boolean, reduceMotion: Boolean): Boolean =
    active && !reduceMotion

@Composable
private fun HostToolDetailBlock(title: String, text: String) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 8.dp)
            .clip(RoundedCornerShape(NolaRadius.sm))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.42f))
            .padding(horizontal = 10.dp, vertical = 8.dp)
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(
            text = text,
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

private fun hostToolFileSummary(argsPreview: String): String {
    if (argsPreview.isBlank()) return ""
    val patterns = listOf(
        Regex("(?im)(?:^|[,{\\s])path\\s*[:=]\\s*[\\\"']?([^,}\\n\\\"']+)"),
        Regex("(?im)\\\"path\\\"\\s*:\\s*\\\"([^\\\"]+)\\\"")
    )
    return patterns
        .flatMap { regex -> regex.findAll(argsPreview).map { it.groupValues[1].trim() }.toList() }
        .filter { it.isNotBlank() }
        .distinct()
        .take(8)
        .joinToString(separator = "\n")
}

private fun hostToolCarriesFilePayload(toolName: String, argsPreview: String): Boolean {
    if (hostToolFileSummary(argsPreview).isNotBlank()) return true
    val normalized = toolName.lowercase().replace('-', '_')
    return listOf(
        "read_file",
        "write_file",
        "edit_file",
        "apply_patch",
        "create_file",
        "delete_file",
        "move_file",
        "copy_file",
        "notebook",
        "document",
        "spreadsheet",
        "presentation",
    ).any(normalized::contains)
}

@Composable
private fun HostToolScreenshotPreview(
    screenshotUri: String,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier
) {
    var load by remember(screenshotUri) { mutableStateOf(HostToolScreenshotLoad()) }
    LaunchedEffect(screenshotUri) {
        load = HostToolScreenshotLoad()
        load = HostToolScreenshotLoad(
            loading = false,
            bitmap = loadHostToolScreenshot(screenshotUri)
        )
    }
    val bitmap = load.bitmap
    Box(
        modifier = modifier
            .fillMaxWidth()
            .heightIn(max = 220.dp)
            .clip(RoundedCornerShape(NolaRadius.md))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.45f))
            .clickable(role = androidx.compose.ui.semantics.Role.Button, onClick = onOpen)
            .heightIn(min = 48.dp),
        contentAlignment = Alignment.Center
    ) {
        when {
            bitmap != null -> {
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = stringResource(R.string.chat_host_tool_open_screenshot),
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 220.dp),
                    contentScale = ContentScale.Fit
                )
            }
            load.loading -> {
                Row(
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(14.dp),
                        strokeWidth = 2.dp
                    )
                    Text(
                        text = stringResource(R.string.chat_host_tool_loading_screenshot),
                        style = MaterialTheme.typography.labelSmall
                    )
                }
            }
            else -> {
                Text(
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                    text = stringResource(R.string.chat_host_tool_open_screenshot),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary
                )
            }
        }
    }
}

private suspend fun loadHostToolScreenshot(uri: String): Bitmap? = withContext(Dispatchers.IO) {
    runCatching {
        val connection = (URL(uri).openConnection() as? HttpURLConnection) ?: return@runCatching null
        connection.connectTimeout = 5_000
        connection.readTimeout = 10_000
        connection.instanceFollowRedirects = false
        try {
            if (connection.responseCode !in 200..299) return@runCatching null
            val contentType = connection.contentType?.lowercase().orEmpty()
            if (contentType.isNotBlank() && !contentType.startsWith("image/")) return@runCatching null
            val output = ByteArrayOutputStream()
            connection.inputStream.use { input ->
                val buffer = ByteArray(8 * 1024)
                var total = 0
                while (true) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    total += read
                    if (total > HOST_TOOL_SCREENSHOT_MAX_BYTES) return@runCatching null
                    output.write(buffer, 0, read)
                }
            }
            BitmapFactory.decodeByteArray(output.toByteArray(), 0, output.size())
        } finally {
            connection.disconnect()
        }
    }.getOrNull()
}

@Composable
fun PermissionDialog(
    toolName: String,
    message: String,
    executionLocation: String,
    dataBoundary: String,
    validity: String,
    allowLabel: String,
    conversationAllowLabel: String? = null,
    onAllowConversation: (() -> Unit)? = null,
    onAllow: () -> Unit,
    onDeny: () -> Unit,
    onProvideInstruction: ((String) -> Unit)? = null,
    permissionIdentity: String = toolName,
    permissionInstructions: List<com.nola.core.AgentPermissionInstruction> = emptyList(),
    modifier: Modifier = Modifier
) {
    var instruction by remember(permissionIdentity) { mutableStateOf("") }
    var editingInstruction by remember(permissionIdentity) { mutableStateOf(false) }
    var showingDetails by remember(permissionIdentity) { mutableStateOf(false) }
    val validInstruction = runCatching { com.nola.core.normalizedPermissionInstruction(permissionIdentity, permissionInstructions, instruction) }.isSuccess
    val historyFull = permissionInstructions.size >= 16 || permissionInstructions.sumOf { it.permissionRequestId.toByteArray(Charsets.UTF_8).size.toLong() + it.instruction.toByteArray(Charsets.UTF_8).size } + permissionIdentity.toByteArray(Charsets.UTF_8).size >= 16384
    val haptics = rememberNolaHaptics()
    val displayName = hostToolDisplayName(toolName)
    if (showingDetails) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { showingDetails = false },
            title = { Text(stringResource(R.string.permission_required_title, displayName)) },
            text = {
                Column(Modifier.heightIn(max = 360.dp).verticalScroll(rememberScrollState())) {
                    androidx.compose.foundation.text.selection.SelectionContainer { Text(message) }
                }
            },
            confirmButton = {
                TextButton(onClick = { showingDetails = false }) { Text(stringResource(R.string.common_close)) }
            },
        )
    }
    Card(
        modifier = modifier
            .fillMaxWidth()
            .padding(16.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer.copy(alpha = 0.3f)
        ),
        shape = RoundedCornerShape(NolaRadius.lg)
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Filled.Warning,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.tertiary
                )
                Spacer(modifier = Modifier.width(8.dp))
                Text(
                    text = stringResource(R.string.permission_required_title, displayName),
                    style = MaterialTheme.typography.titleSmall
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = message,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 4,
                overflow = TextOverflow.Ellipsis,
            )
            TextButton(onClick = { showingDetails = true }) {
                Text(stringResource(R.string.canonical_details))
            }
            Spacer(modifier = Modifier.height(12.dp))
            PermissionDetailRow(stringResource(R.string.permission_detail_location), executionLocation)
            PermissionDetailRow(stringResource(R.string.permission_detail_data), dataBoundary)
            PermissionDetailRow(stringResource(R.string.permission_detail_validity), validity)
            if (onProvideInstruction != null) {
                if (historyFull) Text(stringResource(R.string.permission_instruction_history_full))
                TextButton(onClick = { editingInstruction = !editingInstruction }, enabled = !historyFull) {
                    Text(stringResource(R.string.permission_add_instruction))
                }
                if (editingInstruction) {
                    androidx.compose.material3.OutlinedTextField(
                        value = instruction,
                        onValueChange = { instruction = it },
                        label = { Text(stringResource(R.string.permission_instruction_label)) },
                        supportingText = { Text(stringResource(if (historyFull) R.string.permission_instruction_history_full else R.string.permission_instruction_limit)) },
                        isError = instruction.trim().toByteArray(Charsets.UTF_8).size > 4096,
                        modifier = Modifier.fillMaxWidth(),
                        maxLines = 6,
                    )
                    TextButton(onClick = { onProvideInstruction(instruction.trim()) }, enabled = validInstruction) {
                        Text(stringResource(R.string.permission_send_instruction))
                    }
                }
            }
            Spacer(modifier = Modifier.height(12.dp))
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Button(onClick = {
                    haptics.primaryAction()
                    onAllow()
                }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).testTag("permission.allow-once.$permissionIdentity")) {
                    Text(allowLabel)
                }
                OutlinedButton(onClick = {
                    haptics.warning()
                    onDeny()
                }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.common_deny))
                }
                if (conversationAllowLabel != null && onAllowConversation != null) {
                    OutlinedButton(onClick = {
                        haptics.success()
                        onAllowConversation()
                    }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text(conversationAllowLabel)
                    }
                }
            }
        }
    }
}

@Composable
private fun PermissionDetailRow(label: String, value: String) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp)
    ) {
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(
            value,
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface
        )
    }
}
