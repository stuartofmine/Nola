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
    var directory by remember { mutableStateOf<AgentDirectory?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var editing by rememberSaveable { mutableStateOf(false) }
    var editingID by rememberSaveable { mutableStateOf<String?>(null) }
    var name by rememberSaveable { mutableStateOf("") }
    var description by rememberSaveable { mutableStateOf("") }
    var inheritModel by rememberSaveable { mutableStateOf(true) }
    var modelID by rememberSaveable { mutableStateOf<String?>(null) }
    var modelMenu by remember { mutableStateOf(false) }
    var modelSelectionTouched by rememberSaveable { mutableStateOf(false) }
    var originalModelSelection by rememberSaveable { mutableStateOf<String?>(null) }
    var templateCatalog by remember { mutableStateOf<com.nola.data.local.ReadableExpertTemplateCatalog?>(null) }
    var templateCatalogFailed by remember { mutableStateOf(false) }
    var templateID by rememberSaveable { mutableStateOf<String?>(null) }
    var originalTemplateSelection by rememberSaveable { mutableStateOf<String?>(null) }
    var templateSelectionTouched by rememberSaveable { mutableStateOf(false) }
    var templateMenu by remember { mutableStateOf(false) }
    var colorID by rememberSaveable { mutableStateOf<String?>(null) }
    var actionsID by remember { mutableStateOf<String?>(null) }
    val models = (state.availableModels + listOfNotNull(state.currentModel))
        .filter { it.source == ModelInfo.ModelSource.Local ||
            (it.source == ModelInfo.ModelSource.Cloud && !it.profileId.isNullOrBlank()) }
        .distinctBy { it.source to it.selectionId }
    val selectedModel = models.firstOrNull { "${it.source}:${it.selectionId}" == modelID }
        ?: if (editingID == null) state.currentModel?.takeIf { it in models } ?: models.firstOrNull()
        else if (!modelSelectionTouched) models.firstOrNull {
            com.nola.data.repository.conversationModelIdentity(it) == originalModelSelection
        } else null
    fun editAgent(agent: PersistentAgent) {
        editingID = agent.id; name = agent.name; description = agent.description
        originalModelSelection = agent.defaults["modelSelectionID"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
        inheritModel = originalModelSelection == null
        modelID = models.firstOrNull { com.nola.data.repository.conversationModelIdentity(it) == originalModelSelection }
            ?.let { "${it.source}:${it.selectionId}" }
        modelSelectionTouched = false; modelMenu = false
        originalTemplateSelection = agent.defaults["selectedPromptTemplateID"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
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
    LaunchedEffect(Unit) { perform {
        directory = withContext(Dispatchers.IO) { viewModel.agentRooms.directory() }
        try { templateCatalog = viewModel.readableExpertTemplates(); templateCatalogFailed = false }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { templateCatalogFailed = true }
    } }
    fun back() { if (editing) { editing = false; editingID = null } else onBack() }
    BackHandler { back() }
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
        Text(stringResource(R.string.expert_manage_source_local), style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        error?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(vertical = 8.dp)) }
        if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if (editing) {
            OutlinedTextField(name, { name = it }, enabled = !busy,
                label = { Text(stringResource(R.string.room_agent_name)) },
                modifier = Modifier.fillMaxWidth().testTag("expert.editor.name"))
            OutlinedTextField(description, { description = it }, enabled = !busy,
                label = { Text(stringResource(R.string.room_agent_description)) },
                modifier = Modifier.fillMaxWidth().testTag("expert.editor.description"))
            Box {
                TextButton(onClick = { modelMenu = true }, enabled = !busy, modifier = Modifier.testTag("expert.editor.model")) {
                    Text(stringResource(R.string.room_agent_model, if (inheritModel) stringResource(R.string.expert_use_conversation_model) else selectedModel?.name ?: originalModelSelection?.let { stringResource(R.string.expert_model_unavailable, it) } ?: stringResource(R.string.chat_no_model_selected)))
                }
                DropdownMenu(modelMenu, onDismissRequest = { modelMenu = false }) {
                    DropdownMenuItem(text = { Text(stringResource(R.string.expert_use_conversation_model)) }, onClick = { inheritModel = true; modelSelectionTouched = true; modelMenu = false })
                    models.forEach { model -> DropdownMenuItem(text = { Text(model.name) }, onClick = {
                        inheritModel = false; modelID = "${model.source}:${model.selectionId}"; modelSelectionTouched = true; modelMenu = false
                    }) }
                }
            }
            Box {
                val selectedName = templateID?.let { id -> templateCatalog?.templates?.firstOrNull { it.id == id }?.name }
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
                    templateCatalog?.templates?.forEach { template ->
                        DropdownMenuItem(text = { Text(template.name) }, modifier = Modifier.testTag("expert.template.${template.id}"), onClick = {
                            templateID = template.id; templateSelectionTouched = true; templateMenu = false
                        })
                    }
                    if (templateCatalogFailed) DropdownMenuItem(text = { Text(stringResource(R.string.expert_template_catalog_unavailable)) },
                        onClick = {}, enabled = false)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                TextButton(onClick = { editing = false; editingID = null }, enabled = !busy) { Text(stringResource(R.string.common_cancel)) }
                Button(enabled = !busy && name.isNotBlank() && directory != null &&
                    (!modelSelectionTouched || inheritModel || selectedModel != null) &&
                    (editingID != null || inheritModel || selectedModel != null),
                    modifier = Modifier.testTag("expert.editor.save"), onClick = { perform {
                        val snapshot = checkNotNull(directory)
                        val original = editingID?.let { id -> snapshot.agents.single { it.id == id } }
                        check(!modelSelectionTouched || inheritModel || selectedModel != null) {
                            "The selected model is unavailable"
                        }
                        val desiredModel = if (inheritModel) null else selectedModel?.let { com.nola.data.repository.conversationModelIdentity(it) }
                        val changeModel = if (original == null) !inheritModel else modelSelectionTouched && desiredModel != originalModelSelection
                        val changeTemplate = templateSelectionTouched && templateID != originalTemplateSelection
                        if (changeTemplate && templateID != null) {
                            val currentCatalog = viewModel.readableExpertTemplates()
                            check(currentCatalog.templates.any { it.id == templateID }) {
                                "The selected template is no longer readable"
                            }
                            templateCatalog = currentCatalog; templateCatalogFailed = false
                        }
                        val modelDefaults = if (changeModel && !inheritModel) {
                            if (original == null) viewModel.expertDefaultsForModel(checkNotNull(selectedModel))
                            else buildJsonObject { put("modelSelectionID", checkNotNull(desiredModel)) }
                        } else null
                        val defaults = mergeExpertEditorDefaults(original?.defaults ?: buildJsonObject {},
                            changeModel, modelDefaults, changeTemplate, templateID, initializeModelParameters = original == null)
                        directory = withContext(Dispatchers.IO) {
                            if (original == null) viewModel.agentRooms.createAgent(name.trim(), defaults, snapshot, description.trim())
                            else viewModel.agentRooms.updateAgent(original.copy(defaults = defaults), name.trim(), description.trim(), snapshot)
                        }
                        viewModel.refreshConversationExperts(); editing = false; editingID = null
                    } }) { Text(stringResource(R.string.common_save)) }
            }
        } else {
            TextButton(onClick = { editingID = null; name = ""; description = ""; inheritModel = true; modelID = null; originalModelSelection = null; modelSelectionTouched = false; modelMenu = false; templateID = null; originalTemplateSelection = null; templateSelectionTouched = false; templateMenu = false; editing = true },
                enabled = !busy && directory != null, modifier = Modifier.testTag("expert.create").heightIn(min = 48.dp)) {
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
                        .clickable(enabled = !busy) {
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
                                                    viewModel.agentRooms.updateAgent(original.copy(availability = if (archived) "active" else "archived"),
                                                        original.name, original.description, snapshot)
                                                }
                                                viewModel.refreshConversationExperts()
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
