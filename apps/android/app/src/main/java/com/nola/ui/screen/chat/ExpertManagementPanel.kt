// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalConfiguration
import com.nola.data.repository.ExpertRoleTemplate
import com.nola.data.repository.expertRoleText
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.data.repository.AgentDirectory
import com.nola.data.repository.PersistentAgent
import com.nola.domain.model.ModelInfo
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

/** Directory management only: no room creation or independent chat admission. */
@Composable
internal fun ExpertManagementPanel(viewModel: ChatViewModel, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    var repository by remember(state.conversationId, state.executionHostBinding) { mutableStateOf<com.nola.data.repository.AgentRoomRepository?>(null) }
    var editorScope by remember(state.conversationId, state.executionHostBinding) { mutableStateOf<ChatViewModel.ExpertEditorScope?>(null) }
    val sourceKey = "${state.conversationId}:${state.executionHostBinding}:${editorScope?.sourceIdentity}"
    var directory by remember(state.conversationId, state.executionHostBinding) { mutableStateOf<AgentDirectory?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var editorOwnerKey by rememberSaveable { mutableStateOf<String?>(null) }
    var editing by rememberSaveable { mutableStateOf(false) }
    var editingID by rememberSaveable { mutableStateOf<String?>(null) }
    var newAgentID by rememberSaveable { mutableStateOf<String?>(null) }
    var name by rememberSaveable { mutableStateOf("") }
    var description by rememberSaveable { mutableStateOf("") }
    var inheritModel by rememberSaveable { mutableStateOf(true) }
    var modelID by rememberSaveable { mutableStateOf<String?>(null) }
    var modelMenu by remember { mutableStateOf(false) }
    var modelSelectionTouched by rememberSaveable { mutableStateOf(false) }
    var originalModelSelection by rememberSaveable { mutableStateOf<String?>(null) }
    var templateCatalog by remember { mutableStateOf<com.nola.data.local.ReadableExpertTemplateCatalog?>(null) }
    var templateCatalogFailed by remember { mutableStateOf(false) }
    var roles by remember { mutableStateOf<List<ExpertRoleTemplate>>(emptyList()) }
    var catalogReady by remember { mutableStateOf(false) }
    val catalogUnavailableMessage = stringResource(R.string.expert_role_catalog_unavailable)
    val language = LocalConfiguration.current.locales[0].toLanguageTag()
    var effort by rememberSaveable { mutableStateOf("automatic") }
    var effortMenu by remember { mutableStateOf(false) }
    var declaredEfforts by remember { mutableStateOf<List<String>>(emptyList()) }
    var templateID by rememberSaveable { mutableStateOf<String?>(null) }
    var originalTemplateSelection by rememberSaveable { mutableStateOf<String?>(null) }
    var templateSelectionTouched by rememberSaveable { mutableStateOf(false) }
    var templateMenu by remember { mutableStateOf(false) }
    var colorID by rememberSaveable { mutableStateOf<String?>(null) }
    var actionsID by remember { mutableStateOf<String?>(null) }
    val models = (if (editorScope?.remote == true) editorScope!!.models else state.availableModels + listOfNotNull(state.currentModel))
        .filter { it.source == ModelInfo.ModelSource.Local ||
            (it.source == ModelInfo.ModelSource.Cloud && !it.profileId.isNullOrBlank()) }
        .distinctBy { it.source to it.selectionId }
    val selectedModel = models.firstOrNull { "${it.source}:${it.selectionId}" == modelID }
        ?: if (editingID == null) state.currentModel?.takeIf { it in models } ?: models.firstOrNull()
        else if (!modelSelectionTouched) models.firstOrNull {
            com.nola.data.repository.conversationModelIdentity(it) == originalModelSelection
        } else null
    val effectiveModel = selectedModel.takeUnless { inheritModel }
    LaunchedEffect(editorScope, inheritModel, effectiveModel?.source, effectiveModel?.selectionId, effectiveModel?.profileRevision) {
        declaredEfforts = emptyList()
        val source = editorScope
        val readable = if (source?.remote == true) (if (inheritModel) source.defaultProfileId else effectiveModel?.profileId)?.let { source.efforts[it] }.orEmpty()
            else if (inheritModel) viewModel.expertDefaultReasoningEfforts()
            else effectiveModel?.let { model -> withContext(Dispatchers.IO) { viewModel.reasoningEfforts(model) } }.orEmpty()
        declaredEfforts = readable.filter { it in listOf("none", "minimal", "low", "medium", "high", "xhigh", "max") }.distinct()
    }
    fun editAgent(agent: PersistentAgent) {
        editorOwnerKey = sourceKey
        editingID = agent.id; name = agent.name; description = agent.description
        originalModelSelection = agent.defaults["modelSelectionID"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
        inheritModel = originalModelSelection == null
        modelID = models.firstOrNull { com.nola.data.repository.conversationModelIdentity(it) == originalModelSelection }
            ?.let { "${it.source}:${it.selectionId}" }
        modelSelectionTouched = false; modelMenu = false
        originalTemplateSelection = agent.defaults["selectedPromptTemplateID"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
        effort = agent.defaults["expertReasoningEffort"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content ?: "automatic"
        templateID = originalTemplateSelection; templateSelectionTouched = false; templateMenu = false; editing = true
    }
    fun perform(action: suspend () -> Unit) {
        if (busy) return
        busy = true; error = null
        scope.launch {
            try { action() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (failure: Exception) { error = failure.localizedMessage ?: failure.toString() }
            finally { busy = false }
        }
    }
    suspend fun loadCatalog() {
        catalogReady = false
        val sourceModels = viewModel.expertEditorScope()
        val sourceRepository = viewModel.expertRoomRepository(state.conversationId, sourceModels.storeEpoch)
        directory = withContext(Dispatchers.IO) { sourceRepository.directory() }
        repository = sourceRepository; editorScope = sourceModels
        roles = try { withContext(Dispatchers.IO) { checkNotNull(repository).roleTemplates().templates } }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { throw IllegalStateException(catalogUnavailableMessage) }
        catalogReady = true
        try { templateCatalog = viewModel.readableExpertTemplates(); templateCatalogFailed = false }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { templateCatalogFailed = true }
    }
    LaunchedEffect(state.conversationId, state.executionHostBinding) { perform { loadCatalog() } }
    fun back() { if (editing) { editing = false; editingID = null } else onBack() }
    BackHandler { if (!busy) back() }
    directory?.agents?.firstOrNull { it.id == colorID }?.let {
        ExpertBubbleColorEditor(it.id, it.name, onDismiss = { colorID = null })
    }
    val editorScroll = rememberScrollState()
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp).imePadding()
        .then(if (editing) Modifier.verticalScroll(editorScroll) else Modifier)) {
        TextButton(onClick = ::back, enabled = !busy, modifier = Modifier.heightIn(min = 48.dp)) {
            Text(stringResource(R.string.common_back))
        }
        Text(stringResource(R.string.expert_manage), style = MaterialTheme.typography.titleLarge)
        Text(stringResource(if (editorScope?.remote == true) R.string.expert_manage_source_host else R.string.expert_manage_source_local), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        error?.let {
            Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(vertical = 8.dp))
            TextButton(enabled = !busy, onClick = { perform { loadCatalog() } }) { Text(stringResource(R.string.common_refresh)) }
        }
        if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (editing) {
            if (editorOwnerKey != null && editorOwnerKey != sourceKey) Text(stringResource(R.string.expert_editor_source_changed), color = MaterialTheme.colorScheme.error)
            OutlinedTextField(name, { name = it }, enabled = !busy,
                label = { Text(stringResource(R.string.room_agent_name)) },
                modifier = Modifier.fillMaxWidth().testTag("expert.editor.name"))
            OutlinedTextField(description, { description = it }, enabled = !busy,
                label = { Text(stringResource(R.string.room_agent_description)) },
                modifier = Modifier.fillMaxWidth().testTag("expert.editor.description"))
            Box {
                TextButton(onClick = { modelMenu = true }, enabled = !busy, modifier = Modifier.testTag("expert.editor.model")) {
                    Text(stringResource(R.string.room_agent_model, if (inheritModel) stringResource(R.string.expert_follow_app_default_model) else selectedModel?.name ?: originalModelSelection?.let { stringResource(R.string.expert_model_unavailable, it) } ?: stringResource(R.string.chat_no_model_selected)))
                }
                DropdownMenu(modelMenu, onDismissRequest = { modelMenu = false }) {
                    DropdownMenuItem(text = { Text(stringResource(R.string.expert_follow_app_default_model)) }, onClick = { inheritModel = true; modelSelectionTouched = true; modelMenu = false })
                    models.forEach { model -> DropdownMenuItem(text = { Text(model.name) }, onClick = {
                        inheritModel = false; modelID = "${model.source}:${model.selectionId}"; modelSelectionTouched = true; modelMenu = false
                    }) }
                }
            }
            Box {
                val selectedName = templateID?.let { id -> roles.firstOrNull { it.id == id }?.let { expertRoleText(it.name, language) }
                    ?: templateCatalog?.templates?.firstOrNull { it.id == id }?.name }
                TextButton(onClick = { templateMenu = true }, enabled = !busy,
                    modifier = Modifier.testTag("expert.editor.template")) {
                    Text(stringResource(R.string.expert_template_label, if (templateID == null)
                        stringResource(R.string.expert_template_default) else selectedName
                            ?: stringResource(R.string.expert_template_unavailable, checkNotNull(templateID))))
                }
                DropdownMenu(templateMenu, onDismissRequest = { templateMenu = false }) {
                    DropdownMenuItem(text = { Text(stringResource(R.string.expert_template_default)) }, onClick = {
                        templateID = null; templateSelectionTouched = true; templateMenu = false
                    })
                    roles.forEach { role ->
                        DropdownMenuItem(text = { Text(expertRoleText(role.name, language)) },
                            modifier = Modifier.testTag("expert.template.${role.id}"), onClick = {
                                templateID = role.id; templateSelectionTouched = true; templateMenu = false
                            })
                    }
                    templateCatalog?.templates?.filter { item -> roles.none { it.id == item.id } }?.forEach { template ->
                        DropdownMenuItem(text = { Text(template.name) }, modifier = Modifier.testTag("expert.template.${template.id}"), onClick = {
                            templateID = template.id; templateSelectionTouched = true; templateMenu = false
                        })
                    }
                    if (templateCatalogFailed) DropdownMenuItem(text = { Text(stringResource(R.string.expert_template_catalog_unavailable)) },
                        onClick = {}, enabled = false)
                }
            }
            Box {
                TextButton(onClick = { effortMenu = true }, enabled = !busy,
                    modifier = Modifier.testTag("expert.editor.effort")) {
                    Text(stringResource(R.string.models_reasoning_effort) + ": " + reasoningEffortLabel(effort))
                }
                DropdownMenu(effortMenu, onDismissRequest = { effortMenu = false }) {
                    (listOf("automatic") + declaredEfforts).distinct().forEach { value ->
                        DropdownMenuItem(text = { Text(reasoningEffortLabel(value)) }, onClick = { effort = value; effortMenu = false },
                            modifier = Modifier.testTag("expert.effort.$value"))
                    }
                }
            }
            if (effort != "automatic" && effort !in declaredEfforts) Text(stringResource(R.string.expert_effort_unavailable),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            roles.firstOrNull { it.id == templateID }?.let { role ->
                Text(expertRoleText(role.description, language), style = MaterialTheme.typography.bodySmall)
                var preview by rememberSaveable(role.id) { mutableStateOf(false) }
                TextButton(onClick = { preview = !preview }, modifier = Modifier.testTag("expert.template.preview")) {
                    Text(stringResource(R.string.expert_role_preview))
                }
                if (preview) {
                    Text(stringResource(R.string.expert_role_prompt), style = MaterialTheme.typography.titleSmall)
                    Text(role.prompt, style = MaterialTheme.typography.bodySmall)
                    Text(stringResource(R.string.expert_role_tools), style = MaterialTheme.typography.titleSmall)
                    Text(role.toolPack.id, style = MaterialTheme.typography.labelSmall)
                    Text(role.toolPack.tools.joinToString(" · "), style = MaterialTheme.typography.bodySmall)
                    Text(stringResource(R.string.expert_role_skills), style = MaterialTheme.typography.titleSmall)
                    role.skills.forEach { skill ->
                        Text(expertRoleText(skill.name, language), style = MaterialTheme.typography.labelLarge)
                        Text(skill.instructions, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            Text(stringResource(R.string.expert_role_next_introduction), style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                TextButton(onClick = { editing = false; editingID = null }, enabled = !busy) { Text(stringResource(R.string.common_cancel)) }
                Button(enabled = !busy && catalogReady && name.isNotBlank() && directory != null &&
                    (editorOwnerKey == null || editorOwnerKey == sourceKey) &&
                    (effort == "automatic" || effort in declaredEfforts) &&
                    (!modelSelectionTouched || inheritModel || selectedModel != null) &&
                    (editingID != null || inheritModel || selectedModel != null),
                    modifier = Modifier.testTag("expert.editor.save"), onClick = { perform {
                        val snapshot = checkNotNull(directory)
                        check(editorOwnerKey == null || editorOwnerKey == sourceKey) { "The expert editor source changed; the draft is retained" }
                        val original = editingID?.let { id -> snapshot.agents.single { it.id == id } }
                        check(!modelSelectionTouched || inheritModel || selectedModel != null) {
                            "The selected model is unavailable"
                        }
                        val desiredModel = if (inheritModel) null else selectedModel?.let { com.nola.data.repository.conversationModelIdentity(it) }
                        val changeModel = if (original == null) !inheritModel else modelSelectionTouched && desiredModel != originalModelSelection
                        val changeTemplate = templateSelectionTouched && templateID != originalTemplateSelection
                        if (changeTemplate && templateID != null) {
                            val currentRoles = withContext(Dispatchers.IO) { checkNotNull(repository).roleTemplates().templates }
                            if (currentRoles.none { it.id == templateID }) {
                                val currentCatalog = viewModel.readableExpertTemplates()
                                check(currentCatalog.templates.any { it.id == templateID }) { "The selected template is no longer readable" }
                                templateCatalog = currentCatalog; templateCatalogFailed = false
                            }
                            roles = currentRoles
                        }
                        val modelDefaults = if (changeModel && !inheritModel) {
                            if (original == null && editorScope?.remote != true) viewModel.expertDefaultsForModel(checkNotNull(selectedModel))
                            else buildJsonObject { put("modelSelectionID", checkNotNull(desiredModel)) }
                        } else null
                        val defaults = mergeExpertEditorDefaults(original?.defaults ?: buildJsonObject {},
                            changeModel, modelDefaults, changeTemplate, templateID, initializeModelParameters = original == null, reasoningEffort = effort)
                        val agentID = original?.id ?: newAgentID ?: java.util.UUID.randomUUID().toString().also { newAgentID = it }
                        val attempted = PersistentAgent(agentID, name.trim(), original?.revision ?: 0L,
                            original?.availability ?: "active", defaults, description.trim())
                        directory = withContext(Dispatchers.IO) {
                            try {
                                val next = if (original == null) checkNotNull(repository).createAgent(attempted.name, defaults, snapshot, attempted.description, agentID)
                                    else checkNotNull(repository).updateAgent(original.copy(defaults = defaults), attempted.name, attempted.description, snapshot)
                                check(next.agents.any { confirmsExpertDefinition(it, attempted) }) { "Expert save was not confirmed" }
                                next
                            } catch (cancelled: CancellationException) { throw cancelled }
                            catch (failure: Exception) {
                                val recovered = try { checkNotNull(repository).directory() }
                                catch (cancelled: CancellationException) { throw cancelled }
                                catch (_: Exception) { null }
                                if (recovered?.agents?.any { confirmsExpertDefinition(it, attempted) } != true) throw failure
                                recovered
                            }
                        }
                        viewModel.refreshConversationExperts(restoreSelection = false); editing = false; editingID = null
                    } }) { Text(stringResource(R.string.common_save)) }
            }
        } else {
            TextButton(onClick = { editingID = null; newAgentID = java.util.UUID.randomUUID().toString(); name = ""; description = ""; effort = "automatic"; inheritModel = true; modelID = null; originalModelSelection = null; modelSelectionTouched = false; modelMenu = false; templateID = null; originalTemplateSelection = null; templateSelectionTouched = false; templateMenu = false; editorOwnerKey = sourceKey; editing = true },
                enabled = !busy && catalogReady && directory != null, modifier = Modifier.testTag("expert.create").heightIn(min = 48.dp)) {
                Text(stringResource(R.string.room_create_agent))
            }
            LazyColumn(Modifier.heightIn(max = 420.dp).testTag("expert.directory.list")) {
                items(directory?.agents.orEmpty(), key = PersistentAgent::id) { agent ->
                    ListItem(headlineContent = { Text(agent.name) }, supportingContent = {
                        Column {
                            if (agent.availability == "archived") Text(stringResource(R.string.expert_archived),
                                style = MaterialTheme.typography.labelSmall)
                            if (agent.description.isNotBlank()) Text(agent.description)
                        }
                    }, modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag("expert.row.${agent.id}")
                        .clickable(enabled = !busy && catalogReady) {
                            editAgent(agent)
                        }, trailingContent = {
                            Box {
                                IconButton(onClick = { actionsID = agent.id }, enabled = !busy,
                                    modifier = Modifier.sizeIn(minWidth = 48.dp, minHeight = 48.dp).testTag("expert.actions.${agent.id}")) {
                                    Icon(Icons.Filled.MoreVert, contentDescription = stringResource(R.string.expert_actions, agent.name))
                                }
                                DropdownMenu(expanded = actionsID == agent.id, onDismissRequest = { actionsID = null }) {
                                    DropdownMenuItem(text = { Text(stringResource(R.string.expert_color_action)) }, onClick = {
                                        actionsID = null; colorID = agent.id
                                    }, enabled = !busy)
                                    val archived = agent.availability == "archived"
                                    DropdownMenuItem(text = { Text(stringResource(if (archived) R.string.expert_restore else R.string.expert_archive)) },
                                        modifier = Modifier.testTag("expert.availability.${agent.id}"),
                                        enabled = !busy && agent.availability in setOf("active", "archived"), onClick = {
                                            actionsID = null
                                            perform {
                                                val snapshot = checkNotNull(directory)
                                                val original = snapshot.agents.single { it.id == agent.id }
                                                check(original.revision == agent.revision && original.availability == agent.availability) {
                                                    "The expert changed. Refresh before trying again."
                                                }
                                                directory = withContext(Dispatchers.IO) {
                                                    checkNotNull(repository).updateAgent(original.copy(availability = if (archived) "active" else "archived"),
                                                        original.name, original.description, snapshot)
                                                }
                                                viewModel.refreshConversationExperts(restoreSelection = false)
                                            }
                                        })
                                }
                            }
                        })
                }
            }
        }
    }
}
