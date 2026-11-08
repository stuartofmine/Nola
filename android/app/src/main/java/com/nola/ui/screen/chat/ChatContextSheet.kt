package com.nola.ui.screen.chat

import androidx.activity.compose.BackHandler
import com.nola.ui.components.DiscardChangesDialog

import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.nola.R
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.DataSourceAccessPolicy
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.MemorySearchHit
import com.nola.domain.model.UploadedContent
import com.nola.domain.model.UnifiedDataSource
import com.nola.domain.model.UnifiedDataSourceCapability
import com.nola.domain.model.UnifiedDataSourceConnectionState
import com.nola.domain.model.UnifiedDataSourcePrivacyScope
import com.nola.ui.components.NolaCard
import com.nola.ui.components.NolaColors
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.NolaSectionTitle
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.NolaTopAppBar
import com.nola.ui.components.rememberReduceMotionEnabled
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

enum class ContextDisplayMode { Context, Skills }
private enum class ContextPane { Overview, Knowledge, Tools }

internal const val CHAT_CONTEXT_KNOWLEDGE_TEST_TAG = "chat-context-knowledge"
internal const val CHAT_CONTEXT_WIKI_TEST_TAG = "chat-context-wiki"
internal const val CHAT_CONTEXT_KNOWLEDGE_PANE_TEST_TAG = "chat-context-pane-knowledge"

@Composable
internal fun ConversationScopeSummary(
    title: String,
    detail: String,
    scopeLabel: String
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Box(
            modifier = Modifier
                .width(3.dp)
                .height(36.dp)
                .clip(RoundedCornerShape(2.dp))
                .background(NolaColors.primary)
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1
            )
            Text(
                text = detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2
            )
        }
        Text(
            text = scopeLabel,
            style = MaterialTheme.typography.labelSmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier
                .clip(RoundedCornerShape(50))
                .background(MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.55f))
                .padding(horizontal = 9.dp, vertical = 5.dp)
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatContextScreen(
    displayMode: ContextDisplayMode = ContextDisplayMode.Context,
    conversationId: String,
    conversationTitle: String,
    contextUsage: ContextUsage?,
    sessionMemory: String,
    knowledgePoints: String,
    conversationWiki: String,
    longTermMemory: String,
    userPreferences: String,
    mcpServers: List<McpServerConfig>,
    uploadedSkills: List<UploadedContent>,
    hostUploadedSkills: List<UploadedContent> = emptyList(),
    hostUploadedDocuments: List<UploadedContent> = emptyList(),
    hostMcpServers: List<McpServerConfig> = emptyList(),
    uploadedDocuments: List<UploadedContent> = emptyList(),
    dataSources: List<UnifiedDataSource> = emptyList(),
    selectedDataSourceIds: List<String> = emptyList(),
    onDataSourceEnabledChange: (String, Boolean) -> Unit = { _, _ -> },
    onDataSourceRestoreDefault: (String) -> Unit = {},
    onSessionMemoryChange: (String) -> Unit,
    onConversationKnowledgeChange: (String, String) -> Unit,
    onLongTermMemoryChange: (String) -> Unit,
    onUserPreferencesChange: (String) -> Unit,
    onSkillToggle: (UploadedContent, Boolean) -> Unit,
    onSaveSkill: suspend (UploadedContent?, String, String) -> Unit,
    onSkillDelete: (UploadedContent) -> Unit,
    onDocumentAdd: (UploadedContent) -> Unit,
    onDocumentDelete: (UploadedContent) -> Unit,
    onSaveMemoryProfile: suspend () -> Unit,
    onCompact: suspend () -> Unit,
    onReload: () -> Unit,
    onSearchMemory: suspend (String) -> List<MemorySearchHit>,
    onBack: () -> Unit
) {
    val reduceMotion = rememberReduceMotionEnabled()
    var localSessionMemory by remember(conversationId, sessionMemory) { mutableStateOf(sessionMemory) }
    var localKnowledgePoints by remember(conversationId, knowledgePoints) { mutableStateOf(knowledgePoints) }
    var localConversationWiki by remember(conversationId, conversationWiki) { mutableStateOf(conversationWiki) }
    var localLongTermMemory by remember(longTermMemory) { mutableStateOf(longTermMemory) }
    var localUserPreferences by remember(userPreferences) { mutableStateOf(userPreferences) }
    var searchQuery by remember(conversationId) { mutableStateOf("") }
    var searchResults by remember(conversationId) { mutableStateOf<List<MemorySearchHit>>(emptyList()) }
    var isSearching by remember { mutableStateOf(false) }
    var isSaving by remember { mutableStateOf(false) }
    var isCompacting by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var noticeMessage by remember { mutableStateOf<String?>(null) }
    var selectedContextPane by remember(displayMode) { mutableStateOf(ContextPane.Overview) }
    var expandedSkillIndex by remember { mutableStateOf<Int?>(null) }
    var expandedMcpServerIds by remember { mutableStateOf<Set<String>>(emptySet()) }
    var pendingSkillDeletion by remember { mutableStateOf<UploadedContent?>(null) }
    var showSkillEditor by remember { mutableStateOf(false) }
    var skillEditorOriginal by remember { mutableStateOf<UploadedContent?>(null) }
    var discard by remember(conversationId) { mutableStateOf(false) }
    val memoryDirty = localSessionMemory != sessionMemory || localLongTermMemory != longTermMemory || localUserPreferences != userPreferences
    val leave = {
        // Flush the debounced knowledge edit before disposing this screen.
        if (localKnowledgePoints != knowledgePoints || localConversationWiki != conversationWiki) {
            onConversationKnowledgeChange(localKnowledgePoints, localConversationWiki)
        }
        onBack()
    }
    val requestBack = { if (!isSaving && !isCompacting) { if (memoryDirty) discard = true else leave() } }
    BackHandler { requestBack() }
    if (discard) DiscardChangesDialog(onDiscard = leave, onKeepEditing = { discard = false })
    if (showSkillEditor) {
        ChatSkillEditorDialog(
            original = skillEditorOriginal,
            onSave = onSaveSkill,
            onDismiss = { showSkillEditor = false },
        )
    }

    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    pendingSkillDeletion?.let { skill ->
        val displayName = if (skill.name.isBlank()) stringResource(R.string.chat_untitled_skill) else skill.name
        AlertDialog(
            onDismissRequest = { pendingSkillDeletion = null },
            title = { Text(stringResource(R.string.chat_skill_delete_title)) },
            text = {
                Text(
                    stringResource(
                        R.string.chat_skill_delete_message,
                        displayName
                    )
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        onSkillDelete(skill)
                        expandedSkillIndex = null
                        pendingSkillDeletion = null
                    }
                ) {
                    Text(stringResource(R.string.common_delete), color = NolaColors.danger)
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingSkillDeletion = null }) {
                    Text(stringResource(R.string.common_cancel))
                }
            }
        )
    }
    LaunchedEffect(localKnowledgePoints, localConversationWiki, conversationId) {
        if (localKnowledgePoints == knowledgePoints && localConversationWiki == conversationWiki) {
            return@LaunchedEffect
        }
        delay(700)
        onConversationKnowledgeChange(localKnowledgePoints, localConversationWiki)
        noticeMessage = context.getString(R.string.chat_conversation_knowledge_saved)
    }
    LaunchedEffect(conversationId) {
        errorMessage = null
        noticeMessage = null
    }
    LaunchedEffect(noticeMessage) {
        if (noticeMessage != null) {
            delay(2_600)
            noticeMessage = null
        }
    }
    val documentPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            errorMessage = null
            try {
                val bytes = context.contentResolver.openInputStream(uri)?.use { input ->
                    input.readBytes()
                } ?: ByteArray(0)
                val content = bytes.toString(Charsets.UTF_8).take(MAX_DOCUMENT_CHARS)
                val name = context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
                    val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
                } ?: uri.lastPathSegment ?: context.getString(R.string.chat_imported_document)
                onDocumentAdd(
                    UploadedContent(
                        name = name,
                        content = content,
                        kind = "document"
                    )
                )
                noticeMessage = context.getString(R.string.chat_document_imported)
            } catch (e: Exception) {
                errorMessage = e.message ?: context.getString(R.string.chat_document_import_failed)
            }
        }
    }

    Scaffold(
        modifier = Modifier.testTag("chat.context.sheet"),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            NolaTopAppBar(
                title = if (displayMode == ContextDisplayMode.Context) stringResource(R.string.context) else stringResource(R.string.chat_skills_title),
                subtitle = conversationTitle,
                onBack = requestBack
            ) { }
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(NolaSpacing.lg)
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 4.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Text(
                    text = when (displayMode) {
                        ContextDisplayMode.Context -> stringResource(R.string.chat_context_desc)
                        ContextDisplayMode.Skills -> stringResource(R.string.chat_skills_desc)
                    },
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Text(
                    text = stringResource(
                        if (displayMode == ContextDisplayMode.Context) {
                            R.string.chat_scope_current_conversation
                        } else {
                            R.string.chat_scope_across_conversations
                        }
                    ),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier
                        .clip(RoundedCornerShape(50))
                        .background(MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.48f))
                        .padding(horizontal = 9.dp, vertical = 5.dp)
                )
            }

            val statusMessage = errorMessage ?: noticeMessage
            AnimatedVisibility(visible = statusMessage != null) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = NolaSpacing.sm)
                        .clip(RoundedCornerShape(12.dp))
                        .background(
                            if (errorMessage != null) {
                                MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.72f)
                            } else {
                                MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.62f)
                            }
                        )
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(9.dp)
                ) {
                    Icon(
                        imageVector = if (errorMessage != null) Icons.Filled.ErrorOutline else Icons.Filled.Check,
                        contentDescription = null,
                        tint = if (errorMessage != null) MaterialTheme.colorScheme.error else NolaColors.success,
                        modifier = Modifier.size(17.dp)
                    )
                    Text(
                        text = statusMessage.orEmpty(),
                        modifier = Modifier.weight(1f),
                        style = MaterialTheme.typography.bodySmall,
                        color = if (errorMessage != null) {
                            MaterialTheme.colorScheme.onErrorContainer
                        } else {
                            MaterialTheme.colorScheme.onSecondaryContainer
                        }
                    )
                }
            }

            if (displayMode == ContextDisplayMode.Context) {
                Spacer(Modifier.height(NolaSpacing.md))
                SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                    ContextPane.entries.forEachIndexed { index, pane ->
                        val label = when (pane) {
                            ContextPane.Overview -> stringResource(R.string.chat_context_tab_overview)
                            ContextPane.Knowledge -> stringResource(R.string.chat_context_tab_knowledge)
                            ContextPane.Tools -> stringResource(R.string.chat_context_tab_tools)
                        }
                        SegmentedButton(
                            selected = selectedContextPane == pane,
                            onClick = { selectedContextPane = pane },
                            shape = SegmentedButtonDefaults.itemShape(index, ContextPane.entries.size),
                            modifier = if (pane == ContextPane.Knowledge) {
                                Modifier.testTag(CHAT_CONTEXT_KNOWLEDGE_PANE_TEST_TAG)
                            } else {
                                Modifier
                            },
                            label = { Text(label, maxLines = 2) }
                        )
                    }
                }
            }

            if (displayMode == ContextDisplayMode.Context) {
                if (selectedContextPane == ContextPane.Overview) {
                // ---- Context Usage ----
                if (contextUsage != null) {
                    NolaSectionTitle(title = stringResource(R.string.chat_context_usage_section))
                    NolaCard {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween
                        ) {
                            Text(
                                "${contextUsage.usagePercent.toInt().coerceIn(0, 100)}%",
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = FontWeight.SemiBold,
                                color = if (contextUsage.usagePercent >= 80) NolaColors.warning else NolaColors.primary
                            )
                            Text(
                                "${contextUsage.totalTokens} / ${contextUsage.maxTokens}",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                        Spacer(Modifier.height(NolaSpacing.sm))

                        // Progress bar
                        Box(
                            modifier = Modifier.fillMaxWidth().height(10.dp)
                                .clip(RoundedCornerShape(5.dp))
                                .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f))
                        ) {
                            Box(
                                modifier = Modifier
                                    .fillMaxWidth((contextUsage.usagePercent / 100f).coerceIn(0f, 1f))
                                    .height(10.dp)
                                    .clip(RoundedCornerShape(5.dp))
                                    .background(
                                        if (contextUsage.usagePercent >= 80) NolaColors.warning
                                        else NolaColors.primary
                                    )
                            )
                        }

                        Spacer(Modifier.height(NolaSpacing.md))

                        // Two columns stay readable on compact phones and with large text.
                        Text(stringResource(if (contextUsage.contextIsEstimated) R.string.chat_context_estimated_request
                            else R.string.chat_context_measured_request), style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
                        contextUsage.cacheDiagnosticRequestId?.let { request ->
                            Text(stringResource(R.string.chat_cache_request_identity, request),
                                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        Text(stringResource(R.string.chat_cache_conversation_rate) + ": " +
                            (contextUsage.cacheHitRateBps?.let { java.text.NumberFormat.getPercentInstance().apply {
                                minimumFractionDigits = 1; maximumFractionDigits = 1
                            }.format(it / 10_000.0) } ?: stringResource(R.string.chat_cache_unknown)),
                            style = MaterialTheme.typography.bodySmall)
                        contextUsage.cacheReportedRequests?.let { reported ->
                            Text(stringResource(R.string.chat_cache_coverage, reported, contextUsage.cacheUnreportedRequests ?: 0L),
                                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                        val cacheStatus = when (contextUsage.promptCacheStatus) {
                            "disabled" -> stringResource(R.string.chat_cache_disabled)
                            "invalid_identity" -> stringResource(R.string.chat_cache_invalid)
                            "first_request" -> stringResource(R.string.chat_cache_first_request)
                            "hit" -> stringResource(R.string.chat_cache_hit)
                            "write" -> stringResource(R.string.chat_cache_stored)
                            "miss" -> stringResource(R.string.chat_cache_miss)
                            "stable_candidate" -> stringResource(R.string.chat_cache_candidate)
                            null -> null
                            else -> stringResource(R.string.chat_cache_unknown)
                        }
                        val metrics = buildList {
                            add(stringResource(R.string.chat_metric_active) to contextUsage.breakdown.messages.toString())
                            add(stringResource(R.string.chat_metric_session) to contextUsage.breakdown.sessionMemory.toString())
                            add(stringResource(R.string.chat_metric_long_term) to contextUsage.breakdown.longTermMemory.toString())
                            add(stringResource(R.string.chat_metric_preferences) to contextUsage.breakdown.userPreferences.toString())
                            add(stringResource(R.string.chat_metric_mcp) to contextUsage.breakdown.mcpTools.toString())
                            add(stringResource(R.string.chat_metric_skills) to contextUsage.breakdown.uploadedSkills.toString())
                            add(stringResource(R.string.chat_metric_documents) to contextUsage.breakdown.uploadedDocuments.toString())
                            add(stringResource(R.string.chat_metric_data_sources) to contextUsage.breakdown.dataSources.toString())
                            add(stringResource(R.string.chat_metric_scaffold) to contextUsage.breakdown.scaffold.toString())
                            cacheStatus?.let { add(stringResource(R.string.chat_metric_prompt_cache) to it) }
                            contextUsage.cachedInputTokens?.let {
                                add(stringResource(R.string.chat_metric_cache_read) to it.toString())
                            }
                            contextUsage.cacheWriteInputTokens?.let {
                                add(stringResource(R.string.chat_metric_cache_write) to it.toString())
                            }
                        }
                        val rows = metrics.chunked(2)
                        rows.forEach { row ->
                            Row(
                                modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                                horizontalArrangement = Arrangement.spacedBy(NolaSpacing.sm)
                            ) {
                                row.forEach { (label, value) ->
                                    Column(modifier = Modifier.weight(1f)) {
                                        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                                    }
                                }
                                repeat(2 - row.size) {
                                    Spacer(Modifier.weight(1f))
                                }
                            }
                        }

                        // Suggestions
                        if (contextUsage.suggestions.isNotEmpty()) {
                            Spacer(Modifier.height(NolaSpacing.md))
                            contextUsage.suggestions.forEach { suggestion ->
                                val tint = when (suggestion.severity) {
                                    "high" -> NolaColors.danger
                                    "medium" -> NolaColors.warning
                                    "low" -> NolaColors.primary
                                    else -> NolaColors.primary
                                }
                                Card(
                                    modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
                                    colors = CardDefaults.cardColors(containerColor = tint.copy(alpha = 0.08f)),
                                    shape = RoundedCornerShape(10.dp)
                                ) {
                                    Column(modifier = Modifier.padding(12.dp)) {
                                        Row(
                                            modifier = Modifier.fillMaxWidth(),
                                            horizontalArrangement = Arrangement.SpaceBetween
                                        ) {
                                            Text(suggestion.title, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold)
                                            if (suggestion.estimatedSavingsTokens > 0) {
                                                Text(
                                                    stringResource(R.string.chat_tokens_estimate_format, suggestion.estimatedSavingsTokens),
                                                    style = MaterialTheme.typography.bodySmall,
                                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                                )
                                            }
                                        }
                                        if (suggestion.detail.isNotEmpty()) {
                                            Spacer(Modifier.height(4.dp))
                                            Text(suggestion.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Spacer(Modifier.height(NolaSpacing.md))

                DataSourceCatalogSection(
                    dataSources = dataSources,
                    selectedDataSourceIds = selectedDataSourceIds,
                    onEnabledChange = onDataSourceEnabledChange,
                    onRestoreDefault = onDataSourceRestoreDefault
                )

                Spacer(Modifier.height(NolaSpacing.md))
                }

                if (selectedContextPane == ContextPane.Knowledge) {
                // ---- Memory Search ----
                NolaSectionTitle(title = stringResource(R.string.chat_raw_memory_search))
                NolaCard {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(
                            value = searchQuery,
                            onValueChange = { searchQuery = it },
                            modifier = Modifier.weight(1f),
                            placeholder = { Text(stringResource(R.string.chat_memory_search_placeholder), style = MaterialTheme.typography.bodySmall) },
                            singleLine = true
                        )
                        Spacer(Modifier.width(NolaSpacing.sm))
                        Button(
                            onClick = {
                                scope.launch {
                                    isSearching = true
                                    errorMessage = null
                                    try {
                                        searchResults = onSearchMemory(searchQuery)
                                    } catch (e: Exception) {
                                        errorMessage = e.message
                                    }
                                    isSearching = false
                                }
                            },
                            enabled = searchQuery.isNotBlank() && !isSearching,
                            modifier = Modifier.width(88.dp)
                        ) {
                            Box(
                                modifier = Modifier.size(width = 64.dp, height = 20.dp),
                                contentAlignment = Alignment.Center
                            ) {
                                if (isSearching) {
                                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
                                } else {
                                    Text(stringResource(R.string.common_search))
                                }
                            }
                        }
                    }

                    if (searchResults.isNotEmpty()) {
                        Spacer(Modifier.height(NolaSpacing.sm))
                        searchResults.forEach { hit ->
                            Card(
                                modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
                                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f))
                            ) {
                                Column(modifier = Modifier.padding(NolaSpacing.sm)) {
                                    Row(
                                        modifier = Modifier.fillMaxWidth(),
                                        horizontalArrangement = Arrangement.SpaceBetween
                                    ) {
                                        Text(hit.source, style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold)
                                        if (hit.score > 0) {
                                            Text(stringResource(R.string.chat_score_format, hit.score), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                        }
                                    }
                                    Spacer(Modifier.height(4.dp))
                                    Text(hit.preview, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 3)
                                }
                            }
                        }
                    } else if (searchQuery.isNotEmpty() && !isSearching) {
                        Spacer(Modifier.height(NolaSpacing.sm))
                        Text(stringResource(R.string.chat_no_search_results), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }

                Spacer(Modifier.height(NolaSpacing.md))
                }

                if (selectedContextPane == ContextPane.Overview) {
                // ---- Session Memory ----
                NolaSectionTitle(title = stringResource(R.string.chat_session_memory))
                NolaCard {
                    OutlinedTextField(
                        value = localSessionMemory,
                        onValueChange = { localSessionMemory = it },
                        modifier = Modifier.fillMaxWidth().height(140.dp),
                        placeholder = { Text(stringResource(R.string.chat_session_memory_placeholder), style = MaterialTheme.typography.bodySmall) },
                        maxLines = 8
                    )
                }

                Spacer(Modifier.height(NolaSpacing.md))
                }

                if (selectedContextPane == ContextPane.Knowledge) {
                // ---- Conversation Knowledge ----
                NolaSectionTitle(title = stringResource(R.string.chat_conversation_knowledge))
                NolaCard {
                    OutlinedTextField(
                        value = localKnowledgePoints,
                        onValueChange = { localKnowledgePoints = it },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(150.dp)
                            .testTag(CHAT_CONTEXT_KNOWLEDGE_TEST_TAG),
                        placeholder = {
                            Text(
                                stringResource(R.string.chat_conversation_knowledge_placeholder),
                                style = MaterialTheme.typography.bodySmall
                            )
                        },
                        maxLines = 9
                    )
                }

                Spacer(Modifier.height(NolaSpacing.md))

                // ---- Conversation Wiki ----
                NolaSectionTitle(title = stringResource(R.string.chat_conversation_wiki))
                NolaCard {
                    OutlinedTextField(
                        value = localConversationWiki,
                        onValueChange = { localConversationWiki = it },
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(190.dp)
                            .testTag(CHAT_CONTEXT_WIKI_TEST_TAG),
                        placeholder = {
                            Text(
                                stringResource(R.string.chat_conversation_wiki_placeholder),
                                style = MaterialTheme.typography.bodySmall
                            )
                        },
                        maxLines = 12
                    )
                }

                Spacer(Modifier.height(NolaSpacing.md))

                // ---- Long-term Memory ----
                NolaSectionTitle(title = stringResource(R.string.chat_long_term_memory_global))
                NolaCard {
                    OutlinedTextField(
                        value = localLongTermMemory,
                        onValueChange = { localLongTermMemory = it },
                        modifier = Modifier.fillMaxWidth().height(130.dp),
                        placeholder = { Text(stringResource(R.string.chat_long_term_memory_placeholder), style = MaterialTheme.typography.bodySmall) },
                        maxLines = 7
                    )
                }

                Spacer(Modifier.height(NolaSpacing.md))

                // ---- User Preferences ----
                NolaSectionTitle(title = stringResource(R.string.chat_user_preferences_global))
                NolaCard {
                    OutlinedTextField(
                        value = localUserPreferences,
                        onValueChange = { localUserPreferences = it },
                        modifier = Modifier.fillMaxWidth().height(130.dp),
                        placeholder = { Text(stringResource(R.string.chat_user_preferences_placeholder), style = MaterialTheme.typography.bodySmall) },
                        maxLines = 7
                    )
                }

                Spacer(Modifier.height(NolaSpacing.md))
                }

                // ---- Save & Compact Buttons ----
                if (selectedContextPane != ContextPane.Tools) {
                    NolaCard {
                        Button(
                            onClick = {
                                scope.launch {
                                    isSaving = true
                                    errorMessage = null
                                    onSessionMemoryChange(localSessionMemory)
                                    onConversationKnowledgeChange(localKnowledgePoints, localConversationWiki)
                                    onLongTermMemoryChange(localLongTermMemory)
                                    onUserPreferencesChange(localUserPreferences)
                                    try {
                                        onSaveMemoryProfile()
                                        noticeMessage = context.getString(R.string.chat_memory_saved)
                                    } catch (e: Exception) {
                                        errorMessage = e.message
                                    }
                                    isSaving = false
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                            enabled = !isSaving && !isCompacting
                        ) {
                            Box(modifier = Modifier.size(20.dp), contentAlignment = Alignment.Center) {
                                if (isSaving) {
                                    CircularProgressIndicator(
                                        Modifier.size(16.dp),
                                        strokeWidth = 2.dp,
                                        color = MaterialTheme.colorScheme.onPrimary
                                    )
                                }
                            }
                            Spacer(Modifier.width(8.dp))
                            Text(stringResource(R.string.chat_save_memory_preferences))
                        }

                        if (selectedContextPane == ContextPane.Overview) {
                            Spacer(Modifier.height(NolaSpacing.sm))
                            OutlinedButton(
                                onClick = {
                                    scope.launch {
                                        isCompacting = true
                                        errorMessage = null
                                        try {
                                            onCompact()
                                        } catch (e: Exception) {
                                            errorMessage = e.message
                                        }
                                        isCompacting = false
                                    }
                                },
                                modifier = Modifier.fillMaxWidth(),
                                enabled = !isSaving && !isCompacting
                            ) {
                                Box(modifier = Modifier.size(20.dp), contentAlignment = Alignment.Center) {
                                    if (isCompacting) {
                                        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                                    }
                                }
                                Spacer(Modifier.width(8.dp))
                                Text(stringResource(R.string.chat_run_compact))
                            }
                        }
                    }

                    Spacer(Modifier.height(NolaSpacing.md))
                }

                if (selectedContextPane == ContextPane.Tools) {
                // ---- Agent Host MCP capability catalog ----
                NolaSectionTitle(title = stringResource(R.string.chat_mcp_capabilities))
                NolaCard {
                    if (hostMcpServers.isEmpty()) {
                        Text(
                            stringResource(R.string.chat_host_mcp_empty),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    hostMcpServers.forEachIndexed { index, server ->
                        val identity = server.serverId.ifBlank { server.name }
                        val expanded = identity in expandedMcpServerIds
                        val catalogCount = server.tools.size + server.resources.size +
                            server.resourceTemplates.size + server.prompts.size
                        if (index > 0) HorizontalDivider(Modifier.padding(vertical = NolaSpacing.sm))
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable(role = Role.Button) {
                                    expandedMcpServerIds = if (expanded) {
                                        expandedMcpServerIds - identity
                                    } else {
                                        expandedMcpServerIds + identity
                                    }
                                }
                                .heightIn(min = 48.dp)
                                .padding(vertical = NolaSpacing.xs)
                        ) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Icon(
                                    Icons.Filled.CheckCircle,
                                    null,
                                    Modifier.size(18.dp),
                                    tint = if (server.availability == "available") NolaColors.success
                                    else MaterialTheme.colorScheme.onSurfaceVariant
                                )
                                Spacer(Modifier.width(NolaSpacing.sm))
                                Column(Modifier.weight(1f)) {
                                    Text(server.name, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                                    Text(
                                        stringResource(R.string.chat_host_mcp_catalog_count, catalogCount),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                                Icon(
                                    if (expanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                                    contentDescription = null,
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                            if (expanded) {
                                server.protocolVersion?.takeIf { it.isNotBlank() }?.let { version ->
                                    Text(
                                        stringResource(R.string.chat_host_mcp_protocol, version),
                                        modifier = Modifier.padding(start = 26.dp, top = NolaSpacing.sm),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                                listOf(
                                    stringResource(R.string.chat_host_mcp_tools) to server.tools,
                                    stringResource(R.string.chat_host_mcp_resources) to server.resources,
                                    stringResource(R.string.chat_host_mcp_templates) to server.resourceTemplates,
                                    stringResource(R.string.chat_host_mcp_prompts) to server.prompts
                                ).filter { it.second.isNotEmpty() }.forEach { (label, names) ->
                                    Text(
                                        "$label · ${names.joinToString(", ")}",
                                        modifier = Modifier.padding(start = 26.dp, top = NolaSpacing.xs),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                            }
                        }
                    }
                }

                Spacer(Modifier.height(NolaSpacing.md))

                // ---- Documents ----
                NolaSectionTitle(title = stringResource(R.string.chat_user_documents))
                NolaCard {
                    if (uploadedDocuments.isEmpty() && hostUploadedDocuments.isEmpty()) {
                        Text(stringResource(R.string.chat_no_documents), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    hostUploadedDocuments.forEach { doc ->
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Icon(Icons.Filled.Storage, null, Modifier.size(18.dp), tint = if (doc.enabled) NolaColors.success else MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.width(NolaSpacing.sm))
                            Column(Modifier.weight(1f)) {
                                Text(doc.name, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                                Text(stringResource(R.string.chat_characters_format, doc.content.length), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                Text(stringResource(R.string.chat_host_document_desc), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                    uploadedDocuments.forEach { doc ->
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Icon(Icons.Filled.Storage, null, Modifier.size(18.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                            Spacer(Modifier.width(NolaSpacing.sm))
                            Column(Modifier.weight(1f)) {
                                Text(doc.name, style = MaterialTheme.typography.bodyMedium)
                                Text(stringResource(R.string.chat_characters_format, doc.content.length), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            IconButton(onClick = { onDocumentDelete(doc) }) {
                                Icon(Icons.Filled.Delete, stringResource(R.string.common_delete), Modifier.size(18.dp), tint = NolaColors.danger)
                            }
                        }
                    }
                    Spacer(Modifier.height(NolaSpacing.sm))
                    TextButton(onClick = { documentPicker.launch(arrayOf("text/*", "application/json")) }) {
                        Icon(Icons.Filled.Add, null, Modifier.size(16.dp))
                        Spacer(Modifier.width(4.dp))
                        Text(stringResource(R.string.chat_import_text_document))
                    }
                }
                }
            }

            // ---- Skills Section (both modes) ----
            if (displayMode == ContextDisplayMode.Skills) {
                NolaSectionTitle(title = stringResource(R.string.chat_skill_library))
                NolaCard {
                    Column {
                        Text(stringResource(R.string.chat_skill_library_desc), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }

                // Host skills
                if (hostUploadedSkills.isNotEmpty()) {
                    Spacer(Modifier.height(NolaSpacing.md))
                    NolaSectionTitle(title = stringResource(R.string.chat_web_host_skills))
                    hostUploadedSkills.forEach { skill ->
                        NolaCard {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Icon(Icons.Filled.CheckCircle, null, Modifier.size(18.dp), tint = NolaColors.success)
                                Spacer(Modifier.width(NolaSpacing.sm))
                                Column(Modifier.weight(1f)) {
                                    Text(skill.name, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.SemiBold)
                                    if (skill.content.isNotEmpty()) {
                                        Text(skill.content.take(100), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2)
                                    }
                                }
                            }
                        }
                        Spacer(Modifier.height(NolaSpacing.sm))
                    }
                }

                // Uploaded skills
                Spacer(Modifier.height(NolaSpacing.md))
                NolaSectionTitle(title = stringResource(R.string.chat_your_skills))
                if (uploadedSkills.isEmpty()) {
                    NolaCard {
                        Text(stringResource(R.string.chat_no_skill_help), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }

                uploadedSkills.forEachIndexed { index, skill ->
                    val isExpanded = expandedSkillIndex == index

                    NolaCard {
                        Column {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Column(
                                    Modifier
                                        .weight(1f)
                                        .clip(RoundedCornerShape(10.dp))
                                        .clickable {
                                            expandedSkillIndex = if (isExpanded) null else index
                                        }
                                        .heightIn(min = 48.dp)
                                        .padding(vertical = 4.dp)
                                ) {
                                    Text(
                                        if (skill.name.isBlank()) stringResource(R.string.chat_untitled_skill) else skill.name,
                                        style = MaterialTheme.typography.bodyMedium,
                                        fontWeight = FontWeight.SemiBold
                                    )
                                    Text(
                                        if (skill.enabled) stringResource(R.string.chat_skill_enabled_desc) else stringResource(R.string.chat_skill_disabled_desc),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                    )
                                }
                                Switch(checked = skill.enabled, onCheckedChange = { onSkillToggle(skill, it) })
                                IconButton(onClick = { expandedSkillIndex = if (isExpanded) null else index }) {
                                    Icon(
                                        if (isExpanded) Icons.Filled.KeyboardArrowUp else Icons.Filled.KeyboardArrowDown,
                                        contentDescription = stringResource(R.string.chat_skill_content)
                                    )
                                }
                            }
                            if (!isExpanded && skill.content.isNotBlank()) {
                                Text(
                                    skill.content,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 2
                                )
                            }
                            AnimatedVisibility(
                                visible = isExpanded,
                                enter = androidx.compose.animation.fadeIn(
                                    androidx.compose.animation.core.tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)
                                ) + androidx.compose.animation.expandVertically(
                                    androidx.compose.animation.core.tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
                                    expandFrom = Alignment.Top,
                                ),
                                exit = androidx.compose.animation.fadeOut(
                                    androidx.compose.animation.core.tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)
                                ) + androidx.compose.animation.shrinkVertically(
                                    androidx.compose.animation.core.tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
                                    shrinkTowards = Alignment.Top,
                                ),
                            ) {
                                Column {
                                    Spacer(Modifier.height(NolaSpacing.sm))
                                    TextButton(onClick = { skillEditorOriginal = skill; showSkillEditor = true }) {
                                        Text(stringResource(R.string.intelligence_edit))
                                    }
                                    Spacer(Modifier.height(NolaSpacing.xs))
                                    Text(stringResource(R.string.chat_characters_format, skill.content.length), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    Spacer(Modifier.height(NolaSpacing.xs))
                                    Text(skill.content, modifier = Modifier.fillMaxWidth(), maxLines = 12)
                                    Spacer(Modifier.height(NolaSpacing.sm))
                                    TextButton(onClick = { pendingSkillDeletion = skill }) {
                                        Icon(Icons.Filled.Delete, null, Modifier.size(16.dp), tint = NolaColors.danger)
                                        Spacer(Modifier.width(4.dp))
                                        Text(stringResource(R.string.common_delete), color = NolaColors.danger)
                                    }
                                }
                            }
                        }
                    }
                    Spacer(Modifier.height(NolaSpacing.sm))
                }

                // Create template skill
                NolaCard {
                    Button(
                        onClick = {
                            skillEditorOriginal = null
                            showSkillEditor = true
                        },
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.surfaceVariant, contentColor = MaterialTheme.colorScheme.onSurfaceVariant)
                    ) {
                        Icon(Icons.Filled.Add, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(NolaSpacing.sm))
                        Text(stringResource(R.string.chat_create_template_skill))
                    }
                }
            }

            Spacer(Modifier.height(NolaSpacing.xxl))
        }
    }
}

@Composable
private fun DataSourceCatalogSection(
    dataSources: List<UnifiedDataSource>,
    selectedDataSourceIds: List<String>,
    onEnabledChange: (String, Boolean) -> Unit,
    onRestoreDefault: (String) -> Unit
) {
    NolaSectionTitle(title = stringResource(R.string.chat_metric_data_sources))
    NolaCard {
        if (dataSources.isEmpty()) {
            Text(
                stringResource(R.string.data_sources_empty_title),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.SemiBold
            )
            Spacer(Modifier.height(4.dp))
            Text(
                stringResource(R.string.data_sources_empty_detail),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            return@NolaCard
        }

        dataSources.forEachIndexed { index, source ->
            if (index > 0) {
                Spacer(Modifier.height(NolaSpacing.sm))
            }
            DataSourceCatalogRow(
                source = source,
                selectedDataSourceIds = selectedDataSourceIds,
                onEnabledChange = onEnabledChange,
                onRestoreDefault = onRestoreDefault
            )
        }
    }
}

@Composable
internal fun DataSourceCatalogRow(
    source: UnifiedDataSource,
    selectedDataSourceIds: List<String>,
    onEnabledChange: (String, Boolean) -> Unit,
    onRestoreDefault: (String) -> Unit
) {
    val statusTint = when (source.connectionState) {
        UnifiedDataSourceConnectionState.Ready -> NolaColors.success
        UnifiedDataSourceConnectionState.Disabled -> MaterialTheme.colorScheme.onSurfaceVariant
        UnifiedDataSourceConnectionState.NeedsPermission -> NolaColors.warning
        UnifiedDataSourceConnectionState.Unavailable,
        UnifiedDataSourceConnectionState.Error -> NolaColors.danger
    }
    val customId = source.metadata["customSourceId"]
    val accessPolicy = source.metadata["accessPolicy"]?.let { raw ->
        runCatching { DataSourceAccessPolicy.valueOf(raw) }.getOrNull()
    }
    val selectionId = source.id
    val selected = selectionId in selectedDataSourceIds
    val toggleChecked = if (customId != null) selected else when (accessPolicy) {
        DataSourceAccessPolicy.DefaultEnabled -> true
        DataSourceAccessPolicy.ConversationOptIn,
        DataSourceAccessPolicy.AskWhenRelevant -> selected
        DataSourceAccessPolicy.Blocked -> false
        null -> source.connectionState == UnifiedDataSourceConnectionState.Ready
    }
    val canToggle = customId != null || (accessPolicy !in setOf(DataSourceAccessPolicy.DefaultEnabled, DataSourceAccessPolicy.Blocked) &&
        source.connectionState == UnifiedDataSourceConnectionState.Ready)
    val selectionModifier = if (accessPolicy != null) Modifier
        .testTag("chat.data-source.$selectionId")
        .toggleable(value = toggleChecked, enabled = canToggle, role = Role.Switch) {
            onEnabledChange(selectionId, it)
        }
        .heightIn(min = 48.dp)
    else Modifier
    val detailText = listOfNotNull(
        dataSourceKindLabel(source),
        source.metadata["characters"]?.let { stringResource(R.string.data_sources_characters_count, it) }
    ).joinToString(" · ")

    Row(
        modifier = Modifier.fillMaxWidth().then(selectionModifier),
        verticalAlignment = Alignment.Top
    ) {
        Icon(Icons.Filled.Storage, null, Modifier.padding(top = 4.dp).size(18.dp), tint = statusTint)
        Spacer(Modifier.width(NolaSpacing.sm))
        Column(Modifier.weight(1f)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    source.name,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.SemiBold
                )
                Spacer(Modifier.width(NolaSpacing.sm))
                if (accessPolicy != null) {
                    Switch(
                        checked = toggleChecked,
                        onCheckedChange = null,
                        enabled = canToggle
                    )
                } else {
                    Text(
                        dataSourceStatusLabel(source.connectionState),
                        style = MaterialTheme.typography.labelSmall,
                        color = statusTint,
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }
            if (detailText.isNotBlank()) {
                Text(
                    detailText,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1
                )
            }
            source.metadata["reader"]?.takeIf(String::isNotBlank)?.let { reader ->
                Text(reader, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            source.location?.takeIf(String::isNotBlank)?.let { location ->
                Text(
                    location,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
                )
            }
            accessPolicy?.let { policy ->
                Text(
                    dataSourcePolicyLabel(policy),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.primary
                )
            }
            if (customId != null) {
                androidx.compose.material3.TextButton(
                    onClick = { onRestoreDefault(selectionId) },
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.onSurfaceVariant),
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 8.dp),
                    modifier = Modifier.testTag("chat.data-source.restore.$selectionId")
                ) {
                    Text(stringResource(R.string.chat_data_source_restore_default), style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}

@Composable
private fun dataSourceStatusLabel(state: UnifiedDataSourceConnectionState): String = when (state) {
    UnifiedDataSourceConnectionState.Ready -> stringResource(R.string.data_sources_status_ready)
    UnifiedDataSourceConnectionState.Disabled -> stringResource(R.string.data_sources_status_disabled)
    UnifiedDataSourceConnectionState.NeedsPermission -> stringResource(R.string.data_sources_status_access_needed)
    UnifiedDataSourceConnectionState.Unavailable -> stringResource(R.string.data_sources_status_unavailable)
    UnifiedDataSourceConnectionState.Error -> stringResource(R.string.data_sources_status_error)
}

@Composable
private fun dataSourceKindLabel(source: UnifiedDataSource): String = when (source.kind) {
    com.nola.domain.model.UnifiedDataSourceKind.Health -> stringResource(R.string.data_sources_kind_health)
    com.nola.domain.model.UnifiedDataSourceKind.UploadedDocument -> stringResource(R.string.data_sources_kind_document)
    com.nola.domain.model.UnifiedDataSourceKind.Mcp -> stringResource(R.string.data_sources_kind_mcp)
    com.nola.domain.model.UnifiedDataSourceKind.File,
    com.nola.domain.model.UnifiedDataSourceKind.Folder -> stringResource(R.string.data_sources_kind_file)
    com.nola.domain.model.UnifiedDataSourceKind.WebHost -> stringResource(R.string.data_sources_kind_host)
    else -> stringResource(R.string.data_sources_kind_external)
}

@Composable
private fun dataSourcePolicyLabel(policy: DataSourceAccessPolicy): String = when (policy) {
    DataSourceAccessPolicy.DefaultEnabled -> stringResource(R.string.data_sources_policy_default)
    DataSourceAccessPolicy.ConversationOptIn -> stringResource(R.string.data_sources_policy_conversation)
    DataSourceAccessPolicy.AskWhenRelevant -> stringResource(R.string.data_sources_policy_ask)
    DataSourceAccessPolicy.Blocked -> stringResource(R.string.data_sources_policy_blocked)
}

private const val MAX_DOCUMENT_CHARS = 200_000
