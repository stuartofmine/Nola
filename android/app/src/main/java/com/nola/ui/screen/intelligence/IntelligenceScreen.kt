package com.nola.ui.screen.intelligence

import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import com.nola.ui.components.DiscardChangesDialog

import androidx.compose.animation.animateColorAsState
import com.nola.ui.components.rememberReduceMotionEnabled
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Extension
import androidx.compose.material.icons.filled.Lightbulb
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Psychology
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.UploadedContent
import com.nola.ui.components.NolaCompactEmptyState
import com.nola.ui.components.NolaMotion
import java.net.URI

private data class IntelligenceEditor(
    val kind: IntelligenceEntryKind,
    val index: Int?,
    val initialValue: String
)

internal data class SkillEditor(val index: Int?, val skill: UploadedContent, val isEditable: Boolean = true,
    val snapshot: com.nola.data.local.AndroidApplicationRuntime.SkillEditorSnapshot? = null)
internal data class McpEditor(val index: Int?, val server: McpServerConfig, val isEditable: Boolean = true)
private sealed interface CapabilityDeletion {
    data class Skill(val skill: UploadedContent, val snapshot: com.nola.data.local.AndroidApplicationRuntime.SkillEditorSnapshot) : CapabilityDeletion
    data class Mcp(val index: Int, val name: String, val server: McpServerConfig) : CapabilityDeletion
}

enum class IntelligenceSection {
    MEMORY,
    EXTENSIONS
}

private enum class CapabilitySection {
    SKILLS,
    MCP
}

internal const val INTELLIGENCE_CAPABILITY_SKILLS_TAB_TAG = "intelligence.capabilities.skills"
internal const val INTELLIGENCE_CAPABILITY_MCP_TAB_TAG = "intelligence.capabilities.mcp"

@Composable
fun IntelligenceScreen(viewModel: IntelligenceViewModel = hiltViewModel(), section: IntelligenceSection? = null) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val profile = state.profile
    val visibleSkillIndices = profile.uploadedSkills.indices.filter {
        state.skillAccess.getOrNull(it)?.canReadMetadata == true
    }
    val visibleMcpIndices = profile.mcpServers.indices.filter {
        state.mcpAccess.getOrNull(it)?.canReadMetadata == true
    }
    val readableEnabledSkillCount = visibleSkillIndices.count { index ->
        profile.uploadedSkills[index].enabled && state.skillAccess.getOrNull(index)?.canReadContent == true
    }
    val readableEnabledMcpCount = visibleMcpIndices.count { index ->
        profile.mcpServers[index].enabled && state.mcpAccess.getOrNull(index)?.canReadContent == true
    }
    val newSkillTemplate = stringResource(R.string.chat_new_skill_template)
    var editor by remember { mutableStateOf<IntelligenceEditor?>(null) }
    var skillEditor by remember { mutableStateOf<SkillEditor?>(null) }
    var mcpEditor by remember { mutableStateOf<McpEditor?>(null) }
    var pendingDeletion by remember { mutableStateOf<CapabilityDeletion?>(null) }
    var pendingEntryDeletion by remember { mutableStateOf<IntelligenceEditor?>(null) }
    var activeSection by remember(section) { mutableStateOf(section ?: IntelligenceSection.MEMORY) }
    var activeCapabilitySection by remember { mutableStateOf(CapabilitySection.SKILLS) }
    var dismissedSaveRevision by remember { mutableStateOf(0L) }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .statusBarsPadding(),
        contentAlignment = Alignment.TopCenter
    ) {
        LazyColumn(
            modifier = Modifier
                .fillMaxWidth()
                .widthIn(max = 1_020.dp),
            contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 20.dp, bottom = 30.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = stringResource(when (section) {
                            IntelligenceSection.MEMORY -> R.string.intelligence_memories_title
                            IntelligenceSection.EXTENSIONS -> R.string.settings_capabilities
                            null -> R.string.intelligence_title
                        }),
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold
                    )
                    Text(
                        text = stringResource(if (section == IntelligenceSection.EXTENSIONS) R.string.settings_capabilities_desc else R.string.intelligence_subtitle),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Box(
                    modifier = Modifier.size(28.dp),
                    contentAlignment = Alignment.Center
                ) {
                    if (state.isSaving || state.isLoadingProfile) {
                        CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    }
                }
            }
        }

        if (section == null) item {
            IntelligenceSectionSwitcher(
                selected = activeSection,
                onSelected = { activeSection = it }
            )
        }

        if (state.profileLoadError != null) {
            item {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.intelligence_profile_load_failed), color = MaterialTheme.colorScheme.error)
                    OutlinedButton(onClick = viewModel::reloadProfile) { Text(stringResource(R.string.common_retry)) }
                }
            }
        }

        if (state.saveRevision > dismissedSaveRevision) {
            item {
                Surface(
                    onClick = { dismissedSaveRevision = state.saveRevision },
                    shape = RoundedCornerShape(15.dp),
                    color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.72f)
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 11.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(9.dp)
                    ) {
                        Icon(
                            Icons.Filled.Check,
                            contentDescription = null,
                            modifier = Modifier.size(17.dp),
                            tint = MaterialTheme.colorScheme.primary
                        )
                        Text(
                            stringResource(R.string.intelligence_changes_saved),
                            modifier = Modifier.weight(1f),
                            style = MaterialTheme.typography.bodySmall,
                            fontWeight = FontWeight.SemiBold
                        )
                        Text(
                            stringResource(R.string.data_sources_action_dismiss),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        }

        if (activeSection == IntelligenceSection.MEMORY) {
            item {
                IntelligenceHero(
                    preferences = profile.userPreferences.size,
                    memories = profile.longTermMemory.size,
                    knowledge = profile.uploadedDocuments.count { it.enabled }
                        + readableEnabledSkillCount
                        + readableEnabledMcpCount
                )
            }

            item {
                LearningStatusCard()
            }

            item {
                MemorySectionCard(
                    title = stringResource(R.string.intelligence_preferences_title),
                    subtitle = stringResource(R.string.intelligence_preferences_subtitle),
                    icon = Icons.Filled.Lightbulb,
                    tint = MaterialTheme.colorScheme.tertiary,
                    entries = profile.userPreferences,
                    emptyText = stringResource(R.string.intelligence_preferences_empty),
                    onAdd = { editor = IntelligenceEditor(IntelligenceEntryKind.Preference, null, "") },
                    onEdit = { index, value -> editor = IntelligenceEditor(IntelligenceEntryKind.Preference, index, value) },
                    onDelete = { pendingEntryDeletion = IntelligenceEditor(IntelligenceEntryKind.Preference, it, profile.userPreferences[it]) }
                )
            }

            item {
                MemorySectionCard(
                    title = stringResource(R.string.intelligence_memories_title),
                    subtitle = stringResource(R.string.intelligence_memories_subtitle),
                    icon = Icons.Filled.Psychology,
                    tint = MaterialTheme.colorScheme.primary,
                    entries = profile.longTermMemory,
                    emptyText = stringResource(R.string.intelligence_memories_empty),
                    onAdd = { editor = IntelligenceEditor(IntelligenceEntryKind.Memory, null, "") },
                    onEdit = { index, value -> editor = IntelligenceEditor(IntelligenceEntryKind.Memory, index, value) },
                    onDelete = { pendingEntryDeletion = IntelligenceEditor(IntelligenceEntryKind.Memory, it, profile.longTermMemory[it]) }
                )
            }

            item {
                KnowledgeAssetsCard(
                    documentCount = profile.uploadedDocuments.count { it.enabled }
                )
            }

            item {
                HowLearningWorksCard()
            }
        } else {
            item {
                CapabilityTypeSwitcher(
                    showingSkills = activeCapabilitySection == CapabilitySection.SKILLS,
                    enabledSkills = readableEnabledSkillCount,
                    totalSkills = visibleSkillIndices.size,
                    enabledMcp = readableEnabledMcpCount,
                    totalMcp = visibleMcpIndices.size,
                    onShowSkills = { activeCapabilitySection = CapabilitySection.SKILLS },
                    onShowMcp = { activeCapabilitySection = CapabilitySection.MCP }
                )
            }

            when (activeCapabilitySection) {
                CapabilitySection.SKILLS -> item {
                    SkillLibraryCard(
                        skills = profile.uploadedSkills,
                        access = state.skillAccess,
                        onAdd = {
                            skillEditor = SkillEditor(
                                null,
                                UploadedContent(
                                    name = "",
                                    content = newSkillTemplate,
                                    kind = "markdown",
                                    resourceId = java.util.UUID.randomUUID().toString()
                                )
                            )
                        },
                        onEdit = { index, skill ->
                            val permission = state.skillAccess.getOrNull(index)
                            viewModel.openSkill(skill) { loaded, snapshot ->
                                skillEditor = SkillEditor(index, loaded, permission?.canEditContent == true, snapshot)
                            }
                        },
                        onToggle = viewModel::setSkillEnabled,
                        onDelete = { _, skill ->
                            viewModel.openSkill(skill) { loaded, snapshot ->
                                pendingDeletion = CapabilityDeletion.Skill(loaded, snapshot)
                            }
                        }
                    )
                }
                CapabilitySection.MCP -> item {
                    McpLibraryCard(
                        servers = profile.mcpServers,
                        access = state.mcpAccess,
                        onAdd = { mcpEditor = McpEditor(null, McpServerConfig(name = "", url = "")) },
                        onEdit = { index, server ->
                            val permission = state.mcpAccess.getOrNull(index)
                            mcpEditor = McpEditor(index, server, permission?.canEditContent == true)
                        },
                        onToggle = viewModel::setMcpEnabled,
                        onDelete = { index, server -> pendingDeletion = CapabilityDeletion.Mcp(index, server.name, server) }
                    )
                }
            }
        }
        }
    }

    pendingEntryDeletion?.let { target ->
        val entries = if (target.kind == IntelligenceEntryKind.Preference) profile.userPreferences else profile.longTermMemory
        val index = target.index
        val unchanged = index != null && entries.getOrNull(index) == target.initialValue
        AlertDialog(
            onDismissRequest = { pendingEntryDeletion = null },
            title = { Text(stringResource(R.string.common_delete)) },
            text = { Column {
                Text(target.initialValue)
                Text(stringResource(if (unchanged) R.string.ui_delete_entry_message else R.string.ui_entry_changed))
            } },
            confirmButton = { TextButton(enabled = unchanged, onClick = {
                if (index != null) viewModel.deleteEntry(target.kind, index)
                pendingEntryDeletion = null
            }) { Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error) } },
            dismissButton = { TextButton(onClick = { pendingEntryDeletion = null }) { Text(stringResource(R.string.common_cancel)) } },
        )
    }

    editor?.let { activeEditor ->
        IntelligenceEditorDialog(
            editor = activeEditor,
            onDismiss = { editor = null },
            onSave = { value ->
                viewModel.saveEntry(activeEditor.kind, activeEditor.index, value)
                editor = null
            }
        )
    }

    skillEditor?.let { activeEditor ->
        SkillEditorDialog(
            editor = activeEditor,
            isSaving = state.isSaving,
            errorMessage = state.errorMessage,
            onDismiss = { skillEditor = null; viewModel.clearError() },
            onSave = { name, content ->
                viewModel.saveSkill(activeEditor.skill, activeEditor.snapshot, name, content) {
                    if (skillEditor == activeEditor) skillEditor = null
                }
            }
        )
    }

    mcpEditor?.let { activeEditor ->
        McpEditorDialog(
            editor = activeEditor,
            isSaving = state.isSaving,
            errorMessage = state.errorMessage,
            onDismiss = { mcpEditor = null; viewModel.clearError() },
            onSave = { name, url ->
                viewModel.saveMcpServer(activeEditor.server.takeIf { activeEditor.index != null }, name, url) {
                    if (mcpEditor == activeEditor) mcpEditor = null
                }
            }
        )
    }

    pendingDeletion?.let { target ->
        AlertDialog(
            onDismissRequest = { pendingDeletion = null },
            title = { Text(stringResource(R.string.common_delete)) },
            text = {
                Text(
                    when (target) {
                        is CapabilityDeletion.Skill -> target.skill.name
                        is CapabilityDeletion.Mcp -> target.name
                    }
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    when (target) {
                        is CapabilityDeletion.Skill -> viewModel.deleteSkill(target.skill, target.snapshot)
                        is CapabilityDeletion.Mcp -> viewModel.deleteMcpServer(target.server)
                    }
                    pendingDeletion = null
                }) { Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { pendingDeletion = null }) { Text(stringResource(R.string.common_cancel)) }
            }
        )
    }

    state.errorMessage?.takeIf { skillEditor == null && mcpEditor == null }?.let { message ->
        AlertDialog(
            onDismissRequest = viewModel::clearError,
            title = { Text(stringResource(R.string.intelligence_save_error_title)) },
            text = { Text(message) },
            confirmButton = {
                TextButton(onClick = viewModel::clearError) { Text(stringResource(R.string.common_ok)) }
            }
        )
    }
}

@Composable
internal fun CapabilityTypeSwitcher(
    showingSkills: Boolean,
    enabledSkills: Int,
    totalSkills: Int,
    enabledMcp: Int,
    totalMcp: Int,
    onShowSkills: () -> Unit,
    onShowMcp: () -> Unit
) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = Color.Transparent
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            CapabilityTypeSegment(
                label = stringResource(R.string.chat_skill_library),
                count = "$enabledSkills/$totalSkills",
                selected = showingSkills,
                modifier = Modifier
                    .weight(1f)
                    .testTag(INTELLIGENCE_CAPABILITY_SKILLS_TAB_TAG),
                onClick = onShowSkills
            )
            CapabilityTypeSegment(
                label = stringResource(R.string.chat_mcp_endpoints),
                count = "$enabledMcp/$totalMcp",
                selected = !showingSkills,
                modifier = Modifier
                    .weight(1f)
                    .testTag(INTELLIGENCE_CAPABILITY_MCP_TAB_TAG),
                onClick = onShowMcp
            )
        }
    }
}

@Composable
private fun CapabilityTypeSegment(
    label: String,
    count: String,
    selected: Boolean,
    modifier: Modifier,
    onClick: () -> Unit
) {
    Surface(
        modifier = modifier
            .heightIn(min = 48.dp)
            .clip(RoundedCornerShape(12.dp))
            .selectable(selected = selected, role = Role.Tab, onClick = onClick),
        shape = RoundedCornerShape(12.dp),
        color = selectionBackground(selected),
        contentColor = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
        tonalElevation = 0.dp
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 9.dp),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(label, style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.width(7.dp))
            Text(count, style = MaterialTheme.typography.labelSmall)
        }
    }
}

@Composable
private fun IntelligenceSectionSwitcher(
    selected: IntelligenceSection,
    onSelected: (IntelligenceSection) -> Unit
) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        color = Color.Transparent
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(4.dp),
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            IntelligenceSectionChoice(
                title = stringResource(R.string.intelligence_memories_title),
                icon = Icons.Filled.Psychology,
                selected = selected == IntelligenceSection.MEMORY,
                modifier = Modifier.weight(1f),
                onClick = { onSelected(IntelligenceSection.MEMORY) }
            )
            IntelligenceSectionChoice(
                title = stringResource(R.string.chat_skills_extended_context),
                icon = Icons.Filled.Extension,
                selected = selected == IntelligenceSection.EXTENSIONS,
                modifier = Modifier.weight(1f),
                onClick = { onSelected(IntelligenceSection.EXTENSIONS) }
            )
        }
    }
}

@Composable
private fun IntelligenceSectionChoice(
    title: String,
    icon: ImageVector,
    selected: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit
) {
    Surface(
        onClick = onClick,
        modifier = modifier.heightIn(min = 48.dp),
        shape = RoundedCornerShape(12.dp),
        color = selectionBackground(selected),
        tonalElevation = 0.dp
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                icon,
                contentDescription = null,
                modifier = Modifier.size(17.dp),
                tint = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.width(7.dp))
            Text(
                title,
                style = MaterialTheme.typography.labelLarge,
                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
                color = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                softWrap = true
            )
        }
    }
}

@Composable
private fun selectionBackground(selected: Boolean): Color {
    val reduceMotion = rememberReduceMotionEnabled()
    val color by animateColorAsState(
        targetValue = if (selected) MaterialTheme.colorScheme.primaryContainer else Color.Transparent,
        animationSpec = tween(if (reduceMotion) 0 else NolaMotion.fast),
        label = "intelligence-selection-background"
    )
    return color
}

@Composable
private fun IntelligenceHero(preferences: Int, memories: Int, knowledge: Int) {
    Surface(shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
        Column(
            modifier = Modifier
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Surface(shape = RoundedCornerShape(15.dp), color = MaterialTheme.colorScheme.primary.copy(alpha = 0.11f)) {
                    Icon(
                        Icons.Filled.Psychology,
                        contentDescription = null,
                        modifier = Modifier.padding(12.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
                Column {
                    Text(stringResource(R.string.intelligence_hero_title), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.Bold)
                    Text(stringResource(R.string.intelligence_hero_body), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                IntelligenceMetric(stringResource(R.string.intelligence_metric_preferences), preferences, Modifier.weight(1f))
                IntelligenceMetric(stringResource(R.string.intelligence_metric_memories), memories, Modifier.weight(1f))
                IntelligenceMetric(stringResource(R.string.intelligence_metric_knowledge), knowledge, Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun IntelligenceMetric(label: String, value: Int, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .padding(horizontal = 4.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp)
    ) {
        Text(value.toString(), style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun LearningStatusCard() {
    Surface(shape = RoundedCornerShape(12.dp), color = Color.Transparent) {
        Row(modifier = Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
            Icon(Icons.Filled.AutoAwesome, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(stringResource(R.string.intelligence_learning_active), style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Bold)
                Text(stringResource(R.string.intelligence_learning_body), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun MemorySectionCard(
    title: String,
    subtitle: String,
    icon: ImageVector,
    tint: Color,
    entries: List<String>,
    emptyText: String,
    onAdd: () -> Unit,
    onEdit: (Int, String) -> Unit,
    onDelete: (Int) -> Unit
) {
    Surface(
        shape = RoundedCornerShape(20.dp),
        color = MaterialTheme.colorScheme.surface,
        modifier = Modifier.animateContentSize(
            animationSpec = tween(
                durationMillis = NolaMotion.base,
                easing = NolaMotion.emphasizedEasing
            )
        )
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(11.dp)) {
                Surface(shape = RoundedCornerShape(12.dp), color = tint.copy(alpha = 0.10f)) {
                    Icon(icon, contentDescription = null, modifier = Modifier.padding(9.dp), tint = tint)
                }
                Column(modifier = Modifier.weight(1f)) {
                    Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                    Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                IconButton(onClick = onAdd) {
                    Icon(Icons.Filled.Add, stringResource(R.string.intelligence_add))
                }
            }

            if (entries.isEmpty()) {
                NolaCompactEmptyState(
                    icon = icon,
                    title = stringResource(R.string.intelligence_empty_title),
                    message = emptyText,
                    modifier = Modifier.padding(top = 4.dp)
                )
            } else {
                Column(modifier = Modifier.padding(top = 8.dp)) {
                    entries.forEachIndexed { index, value ->
                        if (index > 0) HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.12f))
                        IntelligenceEntryRow(
                            text = value,
                            onEdit = { onEdit(index, value) },
                            onDelete = { onDelete(index) }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun IntelligenceEntryRow(text: String, onEdit: () -> Unit, onDelete: () -> Unit) {
    var menuExpanded by remember { mutableStateOf(false) }
    Row(modifier = Modifier.padding(vertical = 10.dp), verticalAlignment = Alignment.Top) {
        Box(
            modifier = Modifier
                .padding(top = 7.dp)
                .size(5.dp)
                .background(MaterialTheme.colorScheme.primary, RoundedCornerShape(50))
        )
        Spacer(Modifier.width(10.dp))
        Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Box {
            IconButton(onClick = { menuExpanded = true }, modifier = Modifier.size(48.dp)) {
                Icon(Icons.Filled.MoreVert, stringResource(R.string.intelligence_more))
            }
            DropdownMenu(expanded = menuExpanded, onDismissRequest = { menuExpanded = false }) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.intelligence_edit)) },
                    leadingIcon = { Icon(Icons.Filled.Edit, contentDescription = null) },
                    onClick = { menuExpanded = false; onEdit() }
                )
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.intelligence_delete), color = MaterialTheme.colorScheme.error) },
                    leadingIcon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                    onClick = { menuExpanded = false; onDelete() }
                )
            }
        }
    }
}

@Composable
private fun KnowledgeAssetsCard(documentCount: Int) {
    Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surface) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(13.dp)) {
            Text(stringResource(R.string.intelligence_knowledge_title), style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            KnowledgeAssetRow(Icons.Filled.Description, stringResource(R.string.intelligence_documents_title), stringResource(R.string.intelligence_documents_count, documentCount))
        }
    }
}

@Composable
private fun SkillLibraryCard(
    skills: List<UploadedContent>,
    access: List<com.nola.core.AndroidManagedResourceAccessProjection>,
    onAdd: () -> Unit,
    onEdit: (Int, UploadedContent) -> Unit,
    onToggle: (Int, Boolean) -> Unit,
    onDelete: (Int, UploadedContent) -> Unit
) {
    CapabilityLibraryCard(
        title = stringResource(R.string.chat_skill_library),
        detail = stringResource(R.string.chat_skill_library_desc),
        addLabel = stringResource(R.string.chat_create_template_skill),
        onAdd = onAdd,
        isEmpty = skills.indices.none { access.getOrNull(it)?.canReadMetadata == true },
        emptyTitle = stringResource(R.string.intelligence_capability_empty_title),
        emptyText = stringResource(R.string.chat_no_skill_help)
    ) {
        CapabilityGrid(skills.withIndex().filter { access.getOrNull(it.index)?.canReadMetadata == true }) { (index, skill), modifier ->
            val permission = access.getOrNull(index)
            CapabilityTile(
                title = skill.name.ifBlank { stringResource(R.string.chat_untitled_skill) },
                detail = if (permission?.canReadContent == true) {
                    skillGalleryPreview(skill.content)
                } else {
                    stringResource(R.string.intelligence_resource_content_locked)
                },
                icon = R.drawable.nola_file,
                enabled = skill.enabled,
                canReadContent = permission?.canReadContent == true,
                canEdit = permission?.canEditContent == true,
                canDelete = permission?.canDelete == true,
                modifier = modifier,
                onToggle = { onToggle(index, it) },
                onEdit = { onEdit(index, skill) },
                onDelete = { onDelete(index, skill) }
            )
        }
    }
}

@Composable
private fun McpLibraryCard(
    servers: List<McpServerConfig>,
    access: List<com.nola.core.AndroidManagedResourceAccessProjection>,
    onAdd: () -> Unit,
    onEdit: (Int, McpServerConfig) -> Unit,
    onToggle: (Int, Boolean) -> Unit,
    onDelete: (Int, McpServerConfig) -> Unit
) {
    CapabilityLibraryCard(
        title = stringResource(R.string.chat_mcp_endpoints),
        detail = stringResource(R.string.mcp_mobile_http_execution),
        addLabel = stringResource(R.string.chat_add_mcp_endpoint),
        onAdd = onAdd,
        isEmpty = servers.indices.none { access.getOrNull(it)?.canReadMetadata == true },
        emptyTitle = stringResource(R.string.intelligence_capability_empty_title),
        emptyText = stringResource(R.string.chat_no_mcp_endpoints)
    ) {
        CapabilityGrid(servers.withIndex().filter { access.getOrNull(it.index)?.canReadMetadata == true }) { (index, server), modifier ->
            val permission = access.getOrNull(index)
            CapabilityTile(
                title = server.name.ifBlank { stringResource(R.string.chat_new_mcp_server) },
                detail = if (permission?.canReadContent == true) {
                    server.url.ifBlank { stringResource(R.string.chat_mcp_endpoint_placeholder) }
                } else {
                    stringResource(R.string.intelligence_resource_content_locked)
                },
                icon = R.drawable.nola_link,
                enabled = server.enabled,
                canReadContent = permission?.canReadContent == true,
                canEdit = permission?.canEditContent == true,
                canDelete = permission?.canDelete == true,
                modifier = modifier,
                onToggle = { onToggle(index, it) },
                onEdit = { onEdit(index, server) },
                onDelete = { onDelete(index, server) }
            )
        }
    }
}

@Composable
private fun CapabilityLibraryCard(
    title: String,
    detail: String,
    addLabel: String,
    onAdd: () -> Unit,
    isEmpty: Boolean,
    emptyTitle: String,
    emptyText: String,
    content: @Composable () -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Column {
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        if (isEmpty) {
            Column(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(emptyTitle, style = MaterialTheme.typography.titleMedium)
                Text(emptyText, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                TextButton(onClick = onAdd) { Text(addLabel) }
            }
        } else {
            content()
            TextButton(onClick = onAdd) { Text(addLabel) }
        }
    }
}

@Composable
private fun <T> CapabilityGrid(
    entries: List<IndexedValue<T>>,
    content: @Composable (IndexedValue<T>, Modifier) -> Unit
) {
    BoxWithConstraints(modifier = Modifier.fillMaxWidth()) {
        val columnCount = when {
            maxWidth >= 760.dp -> 3
            maxWidth >= 460.dp -> 2
            else -> 1
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            entries.chunked(columnCount).forEach { rowItems ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.Top
                ) {
                    rowItems.forEach { entry ->
                        content(entry, Modifier.weight(1f))
                    }
                    repeat(columnCount - rowItems.size) {
                        Spacer(Modifier.weight(1f))
                    }
                }
            }
        }
    }
}

@Composable
private fun CapabilityTile(
    title: String,
    detail: String,
    icon: Int,
    enabled: Boolean,
    canReadContent: Boolean,
    canEdit: Boolean,
    canDelete: Boolean,
    modifier: Modifier = Modifier,
    onToggle: (Boolean) -> Unit,
    onEdit: () -> Unit,
    onDelete: () -> Unit
) {
    var menuExpanded by remember { mutableStateOf(false) }
    Surface(
        onClick = { if (canReadContent) onEdit() },
        enabled = canReadContent,
        modifier = modifier.heightIn(min = 132.dp),
        shape = RoundedCornerShape(18.dp),
        color = MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp
    ) {
        Column(
            modifier = Modifier.padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(9.dp)
        ) {
            Row(verticalAlignment = Alignment.Top) {
                Surface(
                    shape = RoundedCornerShape(11.dp),
                    color = MaterialTheme.colorScheme.primary.copy(alpha = 0.09f)
                ) {
                    Icon(
                        painterResource(icon),
                        contentDescription = null,
                        modifier = Modifier.padding(8.dp).size(18.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                }
                Spacer(Modifier.weight(1f))
                Switch(
                    checked = enabled,
                    onCheckedChange = onToggle,
                    enabled = canEdit,
                )
            }

            Text(
                title,
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis
            )

            if (detail.isNotBlank()) {
                Text(
                    detail.replace("\n", " ").trim(),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    stringResource(
                        when {
                            !canReadContent -> R.string.intelligence_locked
                            canEdit -> R.string.intelligence_edit
                            else -> R.string.intelligence_view
                        }
                    ),
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.primary
                )
                if (canDelete) Box {
                    com.nola.ui.components.NolaIconButton(label = stringResource(R.string.intelligence_more), onClick = { menuExpanded = true }, modifier = Modifier.size(48.dp)) {
                        Icon(painterResource(R.drawable.nola_more), contentDescription = stringResource(R.string.intelligence_more))
                    }
                    DropdownMenu(expanded = menuExpanded, onDismissRequest = { menuExpanded = false }) {
                        DropdownMenuItem(text = { Text(stringResource(R.string.common_delete), color = MaterialTheme.colorScheme.error) }, leadingIcon = { Icon(Icons.Filled.Delete, null, tint = MaterialTheme.colorScheme.error) }, onClick = { menuExpanded = false; onDelete() })
                    }
                }
            }
        }
    }
}

private fun skillGalleryPreview(content: String): String = content
    .lineSequence()
    .map(String::trim)
    .filter { line ->
        line.isNotBlank()
            && line != "---"
            && !line.startsWith("#")
            && !line.startsWith("name:", ignoreCase = true)
            && !line.startsWith("description:", ignoreCase = true)
    }
    .take(3)
    .joinToString(" ")
    .replace(Regex("\\s+"), " ")
    .trim()

@Composable
internal fun SkillEditorDialog(editor: SkillEditor, isSaving: Boolean, errorMessage: String?, onDismiss: () -> Unit, onSave: (String, String) -> Unit) {
    var name by remember(editor) { mutableStateOf(editor.skill.name) }
    var content by remember(editor) { mutableStateOf(editor.skill.content) }
    var discard by remember(editor) { mutableStateOf(false) }
    val requestDismiss = {
        if (!isSaving) {
            if (editor.isEditable && (name != editor.skill.name || content != editor.skill.content)) discard = true
            else onDismiss()
        }
    }
    if (discard) DiscardChangesDialog(onDiscard = onDismiss, onKeepEditing = { discard = false })
    AlertDialog(
        modifier = Modifier
            .fillMaxWidth()
            .widthIn(max = 720.dp)
            .padding(horizontal = 16.dp),
        properties = DialogProperties(usePlatformDefaultWidth = false),
        onDismissRequest = requestDismiss,
        title = {
            Text(
                when {
                    !editor.isEditable -> stringResource(R.string.intelligence_view)
                    editor.index == null -> stringResource(R.string.chat_new_skill)
                    else -> stringResource(R.string.intelligence_edit)
                }
            )
        },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(9.dp)) {
                if (isSaving) androidx.compose.material3.LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                if (errorMessage != null) Text(
                    stringResource(R.string.intelligence_editor_save_failed),
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
                OutlinedTextField(name, { name = it }, modifier = Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.chat_skill_name)) }, singleLine = true, enabled = editor.isEditable && !isSaving)
                OutlinedTextField(
                    value = content,
                    onValueChange = { content = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text(stringResource(R.string.chat_skill_content)) },
                    supportingText = { Text(stringResource(R.string.intelligence_skill_characters, content.length)) },
                    minLines = 10,
                    maxLines = 18,
                    enabled = editor.isEditable && !isSaving,
                )
            }
        },
        confirmButton = {
            if (editor.isEditable) {
                Button(onClick = { onSave(name, content) }, enabled = !isSaving && name.isNotBlank() && content.isNotBlank()) { Text(stringResource(R.string.common_save)) }
            }
        },
        dismissButton = {
            TextButton(onClick = requestDismiss, enabled = !isSaving) {
                Text(stringResource(if (editor.isEditable) R.string.common_cancel else R.string.common_ok))
            }
        }
    )
}

@Composable
internal fun McpEditorDialog(
    editor: McpEditor,
    isSaving: Boolean,
    errorMessage: String?,
    onDismiss: () -> Unit,
    onSave: (String, String) -> Unit
) {
    var name by remember(editor) { mutableStateOf(editor.server.name) }
    var url by remember(editor) { mutableStateOf(editor.server.url) }
    var discard by remember(editor) { mutableStateOf(false) }
    val requestDismiss = {
        if (!isSaving) {
            if (editor.isEditable && (name != editor.server.name || url != editor.server.url)) discard = true
            else onDismiss()
        }
    }
    if (discard) DiscardChangesDialog(onDiscard = onDismiss, onKeepEditing = { discard = false })
    val normalizedUrl = url.trim()
    val isValidUrl = remember(normalizedUrl) {
        val parsed = runCatching { URI(normalizedUrl) }.getOrNull()
        parsed != null && parsed.scheme?.lowercase() in setOf("http", "https") && !parsed.host.isNullOrBlank()
    }
    AlertDialog(
        modifier = Modifier
            .fillMaxWidth()
            .widthIn(max = 620.dp)
            .padding(horizontal = 16.dp),
        properties = DialogProperties(usePlatformDefaultWidth = false),
        onDismissRequest = requestDismiss,
        title = {
            Text(
                when {
                    !editor.isEditable -> stringResource(R.string.intelligence_view)
                    editor.index == null -> stringResource(R.string.chat_new_mcp_server)
                    else -> stringResource(R.string.intelligence_edit)
                }
            )
        },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(9.dp)) {
                OutlinedTextField(name, { name = it }, modifier = Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.scheduled_name)) }, singleLine = true, enabled = editor.isEditable && !isSaving)
                OutlinedTextField(
                    value = url,
                    onValueChange = { url = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text(stringResource(R.string.chat_endpoint_url)) },
                    placeholder = { Text(stringResource(R.string.chat_mcp_endpoint_placeholder)) },
                    supportingText = {
                        if (url.isNotBlank() && !isValidUrl) {
                            Text(stringResource(R.string.chat_mcp_endpoint_invalid))
                        }
                    },
                    isError = url.isNotBlank() && !isValidUrl,
                    singleLine = true,
                    enabled = editor.isEditable && !isSaving,
                )
                if (isSaving) androidx.compose.material3.LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                errorMessage?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = {
            if (editor.isEditable) {
                Button(onClick = { onSave(name, normalizedUrl) }, enabled = !isSaving && name.isNotBlank() && isValidUrl) { Text(stringResource(R.string.common_save)) }
            }
        },
        dismissButton = {
            TextButton(onClick = requestDismiss, enabled = !isSaving) {
                Text(stringResource(if (editor.isEditable) R.string.common_cancel else R.string.common_ok))
            }
        }
    )
}

@Composable
private fun KnowledgeAssetRow(icon: ImageVector, title: String, detail: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(11.dp)) {
        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
        Column {
            Text(title, style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.Medium)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun HowLearningWorksCard() {
    Surface(shape = RoundedCornerShape(18.dp), color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.42f)) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Icon(Icons.Filled.Check, contentDescription = null, modifier = Modifier.size(17.dp), tint = MaterialTheme.colorScheme.primary)
                Text(stringResource(R.string.intelligence_how_title), style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold)
            }
            Text(stringResource(R.string.intelligence_how_body), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun IntelligenceEditorDialog(
    editor: IntelligenceEditor,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit
) {
    var value by remember(editor) { mutableStateOf(editor.initialValue) }
    var discard by remember(editor) { mutableStateOf(false) }
    val requestDismiss = { if (value != editor.initialValue) discard = true else onDismiss() }
    if (discard) DiscardChangesDialog(onDiscard = onDismiss, onKeepEditing = { discard = false })
    val title = when (editor.kind) {
        IntelligenceEntryKind.Preference -> if (editor.index == null) stringResource(R.string.intelligence_add_preference) else stringResource(R.string.intelligence_edit_preference)
        IntelligenceEntryKind.Memory -> if (editor.index == null) stringResource(R.string.intelligence_add_memory) else stringResource(R.string.intelligence_edit_memory)
    }
    AlertDialog(
        onDismissRequest = requestDismiss,
        title = { Text(title) },
        text = {
            TextField(
                value = value,
                onValueChange = { value = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text(stringResource(R.string.intelligence_editor_placeholder)) },
                minLines = 2,
                maxLines = 5
            )
        },
        dismissButton = {
            TextButton(onClick = requestDismiss) { Text(stringResource(R.string.common_cancel)) }
        },
        confirmButton = {
            Button(onClick = { onSave(value) }, enabled = value.isNotBlank()) {
                Text(stringResource(R.string.intelligence_save))
            }
        }
    )
}
