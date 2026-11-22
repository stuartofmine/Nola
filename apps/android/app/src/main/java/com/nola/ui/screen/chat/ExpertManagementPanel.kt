// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Person
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import com.nola.ui.components.BuiltinToolNames
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.text.font.FontWeight
import com.nola.data.repository.ExpertRoleCatalog
import com.nola.data.repository.ExpertRoleTemplate
import com.nola.data.repository.expertRoleText
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nola.R
import com.nola.ui.components.NolaBadge
import com.nola.ui.components.NolaDisclosureBar
import com.nola.ui.components.NolaDropdownBar
import com.nola.ui.components.NolaEmptyState
import com.nola.ui.components.NolaTopAppBar
import com.nola.data.repository.AgentDirectory
import com.nola.data.repository.PersistentAgent
import com.nola.domain.model.ModelInfo
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*

internal interface ExpertManagementDataSource {
    suspend fun expertEditorScope(): ChatViewModel.ExpertEditorScope
    suspend fun directory(conversationId: String?, storeEpoch: String?): AgentDirectory
    suspend fun roleTemplates(conversationId: String?, storeEpoch: String?): ExpertRoleCatalog
    suspend fun readableExpertTemplates(): com.nola.data.local.ReadableExpertTemplateCatalog
    suspend fun expertDefaultReasoningEfforts(): List<String>
    suspend fun reasoningEfforts(model: ModelInfo): List<String>
    suspend fun createAgent(
        conversationId: String?,
        storeEpoch: String?,
        name: String,
        defaults: JsonObject,
        snapshot: AgentDirectory,
        description: String,
        agentId: String,
    ): AgentDirectory
    suspend fun updateAgent(
        conversationId: String?,
        storeEpoch: String?,
        original: PersistentAgent,
        name: String,
        description: String,
        snapshot: AgentDirectory,
    ): AgentDirectory
    fun refreshConversationExperts(restoreSelection: Boolean)
}

/** Directory management only: no room creation or independent chat admission. */
@Composable
internal fun ExpertManagementPanel(viewModel: ChatViewModel, localDirectory: Boolean = false, onBack: () -> Unit) {
    val state = if (localDirectory) {
        val modelCatalog = remember(viewModel) { viewModel.uiState.map { it.availableModels }.distinctUntilChanged() }
        val models by modelCatalog.collectAsStateWithLifecycle(initialValue = emptyList())
        remember(models) { ChatUiState(availableModels = models) }
    } else viewModel.uiState.collectAsStateWithLifecycle().value
    val dataSource = remember(viewModel, localDirectory) {
        object : ExpertManagementDataSource {
            private suspend fun repository(conversationId: String?, storeEpoch: String?) =
                if (localDirectory) viewModel.agentRooms else viewModel.expertRoomRepository(conversationId, storeEpoch)

            override suspend fun expertEditorScope(): ChatViewModel.ExpertEditorScope =
                if (localDirectory) ChatViewModel.ExpertEditorScope(false, emptyList(), null, emptyMap())
                else viewModel.expertEditorScope()

            override suspend fun directory(conversationId: String?, storeEpoch: String?): AgentDirectory =
                withContext(Dispatchers.IO) {
                    repository(conversationId, storeEpoch).directory()
                }

            override suspend fun roleTemplates(conversationId: String?, storeEpoch: String?): ExpertRoleCatalog =
                withContext(Dispatchers.IO) {
                    repository(conversationId, storeEpoch).roleTemplates()
                }

            override suspend fun readableExpertTemplates(): com.nola.data.local.ReadableExpertTemplateCatalog =
                viewModel.readableExpertTemplates(localDirectory)

            override suspend fun expertDefaultReasoningEfforts(): List<String> =
                viewModel.expertDefaultReasoningEfforts()

            override suspend fun reasoningEfforts(model: ModelInfo): List<String> =
                withContext(Dispatchers.IO) { viewModel.reasoningEfforts(model) }

            override suspend fun createAgent(
                conversationId: String?,
                storeEpoch: String?,
                name: String,
                defaults: JsonObject,
                snapshot: AgentDirectory,
                description: String,
                agentId: String,
            ): AgentDirectory = withContext(Dispatchers.IO) {
                repository(conversationId, storeEpoch)
                    .createAgent(name, defaults, snapshot, description, agentId)
            }

            override suspend fun updateAgent(
                conversationId: String?,
                storeEpoch: String?,
                original: PersistentAgent,
                name: String,
                description: String,
                snapshot: AgentDirectory,
            ): AgentDirectory = withContext(Dispatchers.IO) {
                repository(conversationId, storeEpoch)
                    .updateAgent(original, name, description, snapshot)
            }

            override fun refreshConversationExperts(restoreSelection: Boolean) {
                if (!localDirectory) viewModel.refreshConversationExperts(restoreSelection = restoreSelection)
            }
        }
    }
    ExpertManagementPanel(state = state, dataSource = dataSource, isFullScreen = localDirectory, onBack = onBack)
}

@Composable
internal fun ExpertManagementPanel(
    state: ChatUiState,
    dataSource: ExpertManagementDataSource,
    isFullScreen: Boolean = false,
    onBack: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    fun finishTextEntry() { focusManager.clearFocus(); keyboard?.hide() }
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
    var executorJSON by rememberSaveable { mutableStateOf<String?>(null) }
    val executor = remember(executorJSON) { executorJSON?.let { Json.parseToJsonElement(it).jsonObject } }
    var supportsExecutors by remember(state.conversationId, state.executionHostBinding) { mutableStateOf(false) }
    val executors = editorScope?.executors.orEmpty().filter { supportsExecutors && it.id != "nola" }
    val executorKind = executor?.get("backend")?.jsonPrimitive?.contentOrNull
    val backend = executors.firstOrNull { expertExecutorBinding(it.id)["backend"]?.jsonPrimitive?.content == executorKind }
    var executorMenu by remember { mutableStateOf(false) }
    var executorModelMenu by remember { mutableStateOf(false) }
    var inheritModel by rememberSaveable { mutableStateOf(true) }
    var modelID by rememberSaveable { mutableStateOf<String?>(null) }
    var modelMenu by remember { mutableStateOf(false) }
    var modelSelectionTouched by rememberSaveable { mutableStateOf(false) }
    var resetModelParameters by rememberSaveable { mutableStateOf<String?>(null) }
    var originalModelSelection by rememberSaveable { mutableStateOf<String?>(null) }
    var templateCatalog by remember { mutableStateOf<com.nola.data.local.ReadableExpertTemplateCatalog?>(null) }
    var templateCatalogFailed by remember { mutableStateOf(false) }
    var roles by remember { mutableStateOf<List<ExpertRoleTemplate>>(emptyList()) }
    var catalogReady by remember { mutableStateOf(false) }
    val catalogUnavailableMessage = stringResource(R.string.expert_role_catalog_unavailable)
    val saveFailedMessage = stringResource(R.string.chat_save_failed)
    val sourceChangedMessage = stringResource(R.string.expert_editor_source_changed)
    val selectedModelUnavailableMessage = stringResource(R.string.expert_editor_selected_model_unavailable)
    val selectedTemplateUnreadableMessage = stringResource(R.string.expert_editor_selected_template_unreadable)
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
            else if (inheritModel) dataSource.expertDefaultReasoningEfforts()
            else effectiveModel?.let { model -> dataSource.reasoningEfforts(model) }.orEmpty()
        declaredEfforts = readable.filter { it in listOf("none", "minimal", "low", "medium", "high", "xhigh", "max") }.distinct()
    }
    fun editAgent(agent: PersistentAgent) {
        editorOwnerKey = sourceKey
        resetModelParameters = null
        executorJSON = (agent.defaults["expertExecutor"] as? JsonObject)?.toString()
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
        val sourceModels = dataSource.expertEditorScope()
        directory = dataSource.directory(state.conversationId, sourceModels.storeEpoch)
        editorScope = sourceModels
        val roleCatalog = try { dataSource.roleTemplates(state.conversationId, sourceModels.storeEpoch) }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { throw IllegalStateException(catalogUnavailableMessage) }
        roles = roleCatalog.templates
        supportsExecutors = roleCatalog.features.externalExpertExecutors
        catalogReady = true
        try { templateCatalog = dataSource.readableExpertTemplates(); templateCatalogFailed = false }
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
    val startCreateExpert: () -> Unit = {
        editingID = null; executorJSON = null; newAgentID = java.util.UUID.randomUUID().toString()
        name = ""; description = ""; effort = "automatic"; inheritModel = true; modelID = null
        originalModelSelection = null; resetModelParameters = null; modelSelectionTouched = false
        modelMenu = false; templateID = null; originalTemplateSelection = null
        templateSelectionTouched = false; templateMenu = false; editorOwnerKey = sourceKey; editing = true
    }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .then(if (!isFullScreen) Modifier.navigationBarsPadding() else Modifier)
            .imePadding()
    ) {
        NolaTopAppBar(
            title = stringResource(R.string.expert_manage),
            subtitle = stringResource(if (editorScope?.remote == true) R.string.expert_manage_source_host else R.string.expert_manage_source_local),
            onBack = { if (!busy) back() },
        ) {
            if (!editing) {
                IconButton(
                    onClick = startCreateExpert,
                    enabled = !busy && catalogReady && directory != null,
                    modifier = Modifier.testTag("expert.create"),
                ) {
                    Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.room_create_agent))
                }
            }
        }
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .then(if (isFullScreen) Modifier.weight(1f) else Modifier)
                .padding(horizontal = 20.dp, vertical = 8.dp)
                .then(if (editing) Modifier.verticalScroll(editorScroll) else Modifier),
            verticalArrangement = if (editing) Arrangement.spacedBy(12.dp) else Arrangement.Top
        ) {
        error?.let {
            Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(vertical = 8.dp))
            TextButton(enabled = !busy, onClick = { perform { loadCatalog() } }) { Text(stringResource(R.string.common_refresh)) }
        }
        Box(Modifier.fillMaxWidth().height(4.dp)) {
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        }
        if (editing) {
            if (editorOwnerKey != null && editorOwnerKey != sourceKey) Text(stringResource(R.string.expert_editor_source_changed), color = MaterialTheme.colorScheme.error)
            OutlinedTextField(name, { name = it }, enabled = !busy,
                label = { Text(stringResource(R.string.room_agent_name)) },
                modifier = Modifier.fillMaxWidth().testTag("expert.editor.name"))
            OutlinedTextField(description, { description = it }, enabled = !busy,
                label = { Text(stringResource(R.string.room_agent_description)) },
                modifier = Modifier.fillMaxWidth().testTag("expert.editor.description"))
            if (executors.isNotEmpty() || executor != null) Box(Modifier.fillMaxWidth()) {
                NolaDropdownBar(value = backend?.displayName ?: executorKind ?: "Nola", expanded = executorMenu,
                    label = stringResource(R.string.chat_execution_unit), onClick = { finishTextEntry(); executorMenu = true },
                    enabled = !busy, modifier = Modifier.testTag("expert.editor.executor"))
                DropdownMenu(executorMenu, onDismissRequest = { executorMenu = false }) {
                    DropdownMenuItem(text = { Text("Nola") }, onClick = { executorJSON = null; executorMenu = false })
                    executors.forEach { option ->
                        DropdownMenuItem(text = { Text(option.displayName) }, enabled = option.availability == "available",
                            onClick = { if (backend?.id != option.id) executorJSON = expertExecutorBinding(option.id).toString(); executorMenu = false })
                    }
                }
            }
            if (executor != null) Box(Modifier.fillMaxWidth()) {
                val model = executor["model"]?.jsonPrimitive?.contentOrNull
                val provider = executor["model_provider"]?.jsonPrimitive?.contentOrNull
                val selection = backend?.models?.firstOrNull { it.id == model && (backend.id != "codex" || it.providerId == provider) }
                fun label(option: com.nola.data.remote.dto.ExecutionBackendModelDto) =
                    listOfNotNull(option.displayName, option.providerId?.takeIf { backend?.id == "codex" }).joinToString(" · ")
                val savedLabel = listOfNotNull(model, provider).joinToString(" · ")
                NolaDropdownBar(value = stringResource(R.string.room_agent_model, selection?.let(::label)
                    ?: savedLabel.ifEmpty { stringResource(R.string.chat_use_model_default) }),
                    expanded = executorModelMenu, onClick = { finishTextEntry(); executorModelMenu = true },
                    enabled = !busy && backend?.availability == "available", modifier = Modifier.testTag("expert.editor.executor.model"))
                DropdownMenu(executorModelMenu, onDismissRequest = { executorModelMenu = false }) {
                    if (backend != null) {
                        DropdownMenuItem(text = { Text(stringResource(R.string.chat_use_model_default)) },
                            onClick = { executorJSON = expertExecutorBinding(backend.id).toString(); executorModelMenu = false })
                        backend.models.forEach { option -> DropdownMenuItem(text = { Text(label(option)) },
                            onClick = { executorJSON = expertExecutorBinding(backend.id, option).toString(); executorModelMenu = false }) }
                    }
                }
            }
            if (executor == null) Box(Modifier.fillMaxWidth()) {
                val modelValue = if (inheritModel) {
                    stringResource(R.string.expert_follow_app_default_model)
                } else {
                    selectedModel?.name
                        ?: originalModelSelection?.let { stringResource(R.string.expert_model_unavailable, it) }
                        ?: stringResource(R.string.chat_no_model_selected)
                }
                NolaDropdownBar(
                    value = stringResource(R.string.room_agent_model, modelValue),
                    expanded = modelMenu,
                    onClick = { finishTextEntry(); modelMenu = true },
                    enabled = !busy,
                    modifier = Modifier.testTag("expert.editor.model")
                )
                DropdownMenu(modelMenu, onDismissRequest = { modelMenu = false }) {
                    DropdownMenuItem(text = { Text(stringResource(R.string.expert_follow_app_default_model)) }, onClick = { inheritModel = true; modelSelectionTouched = true; modelMenu = false })
                    models.forEach { model -> DropdownMenuItem(text = { Text(model.name) }, onClick = {
                        inheritModel = false; modelID = "${model.source}:${model.selectionId}"; modelSelectionTouched = true; modelMenu = false
                    }) }
                }
            }
            val parameterKey = effectiveModel?.let { com.nola.data.repository.conversationModelIdentity(it) }
            val savedOverrides = directory?.agents?.singleOrNull { it.id == editingID }?.defaults?.get("modelOverrides") as? JsonObject
            if (executor == null && parameterKey != null && (savedOverrides?.get(parameterKey) as? JsonObject)?.isNotEmpty() == true) {
                TextButton(
                    onClick = { resetModelParameters = parameterKey },
                    enabled = !busy && resetModelParameters != parameterKey,
                    modifier = Modifier.testTag("expert.editor.reset-model-parameters"),
                ) { Text(stringResource(R.string.chat_use_model_default)) }
            }
            Box(Modifier.fillMaxWidth()) {
                val selectedName = templateID?.let { id -> roles.firstOrNull { it.id == id }?.let { expertRoleText(it.name, language) }
                    ?: templateCatalog?.templates?.firstOrNull { it.id == id }?.name }
                val templateValue = if (templateID == null) {
                    stringResource(R.string.expert_template_default)
                } else {
                    selectedName ?: stringResource(R.string.expert_template_unavailable, checkNotNull(templateID))
                }
                NolaDropdownBar(
                    value = stringResource(R.string.expert_template_label, templateValue),
                    expanded = templateMenu,
                    onClick = { finishTextEntry(); templateMenu = true },
                    enabled = !busy,
                    modifier = Modifier.testTag("expert.editor.template")
                )
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
            if (executor == null) Box(Modifier.fillMaxWidth()) {
                NolaDropdownBar(
                    value = reasoningEffortLabel(effort),
                    expanded = effortMenu,
                    onClick = { finishTextEntry(); effortMenu = true },
                    enabled = !busy,
                    label = stringResource(R.string.models_reasoning_effort),
                    modifier = Modifier.testTag("expert.editor.effort")
                )
                DropdownMenu(effortMenu, onDismissRequest = { effortMenu = false }) {
                    (listOf("automatic") + declaredEfforts).distinct().forEach { value ->
                        DropdownMenuItem(text = { Text(reasoningEffortLabel(value)) }, onClick = { effort = value; effortMenu = false },
                            modifier = Modifier.testTag("expert.effort.$value"))
                    }
                }
            }
            if (executor == null && effort != "automatic" && effort !in declaredEfforts) Text(stringResource(R.string.expert_effort_unavailable),
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
            roles.firstOrNull { it.id == templateID }?.let { role ->
                Text(expertRoleText(role.description, language), style = MaterialTheme.typography.bodySmall)
                var preview by rememberSaveable(role.id) { mutableStateOf(false) }
                NolaDisclosureBar(
                    title = stringResource(R.string.expert_role_preview),
                    expanded = preview,
                    onToggle = { finishTextEntry(); preview = !preview },
                    modifier = Modifier.testTag("expert.template.preview")
                )
                if (preview) {
                    Text(stringResource(R.string.expert_role_prompt), style = MaterialTheme.typography.titleSmall)
                    Text(role.prompt, style = MaterialTheme.typography.bodySmall)
                    Text(stringResource(R.string.expert_role_tools), style = MaterialTheme.typography.titleSmall)
                    val toolLocale = LocalConfiguration.current.locales[0]
                    Text(role.toolPack.tools.joinToString(" · ") { BuiltinToolNames.label(it, null, toolLocale) ?: it },
                        style = MaterialTheme.typography.bodySmall)
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
                    (executor != null || effort == "automatic" || effort in declaredEfforts) &&
                    (!modelSelectionTouched || inheritModel || selectedModel != null) &&
                    (editingID != null || inheritModel || selectedModel != null),
                    modifier = Modifier.testTag("expert.editor.save"), onClick = { perform {
                        val snapshot = checkNotNull(directory)
                        check(editorOwnerKey == null || editorOwnerKey == sourceKey) { sourceChangedMessage }
                        val original = editingID?.let { id -> snapshot.agents.single { it.id == id } }
                        check(!modelSelectionTouched || inheritModel || selectedModel != null) {
                            selectedModelUnavailableMessage
                        }
                        val desiredModel = if (inheritModel) null else selectedModel?.let { com.nola.data.repository.conversationModelIdentity(it) }
                        val changeModel = if (original == null) !inheritModel else modelSelectionTouched && desiredModel != originalModelSelection
                        val changeTemplate = templateSelectionTouched && templateID != originalTemplateSelection
                        if (changeTemplate && templateID != null) {
                            val currentRoles = dataSource.roleTemplates(state.conversationId, editorScope?.storeEpoch).templates
                            if (currentRoles.none { it.id == templateID }) {
                                val currentCatalog = dataSource.readableExpertTemplates()
                                check(currentCatalog.templates.any { it.id == templateID }) { selectedTemplateUnreadableMessage }
                                templateCatalog = currentCatalog; templateCatalogFailed = false
                            }
                            roles = currentRoles
                        }
                        val modelDefaults = desiredModel?.let { buildJsonObject { put("modelSelectionID", it) } }
                        val defaults = mergeExpertEditorDefaults(original?.defaults ?: buildJsonObject {},
                            changeModel, modelDefaults, changeTemplate, templateID, reasoningEffort = effort.takeIf { executor == null },
                            changeExecutor = executor != (original?.defaults?.get("expertExecutor") as? JsonObject), executor = executor,
                            resetModelParameters = resetModelParameters?.takeIf { it == desiredModel })
                        val agentID = original?.id ?: newAgentID ?: java.util.UUID.randomUUID().toString().also { newAgentID = it }
                        val attempted = PersistentAgent(agentID, name.trim(), original?.revision ?: 0L,
                            original?.availability ?: "active", defaults, description.trim())
                        directory = try {
                            val next = if (original == null) dataSource.createAgent(
                                state.conversationId, editorScope?.storeEpoch, attempted.name, defaults, snapshot, attempted.description, agentID,
                            ) else dataSource.updateAgent(
                                state.conversationId, editorScope?.storeEpoch, original.copy(defaults = defaults), attempted.name, attempted.description, snapshot,
                            )
                            check(next.agents.any { confirmsExpertDefinition(it, attempted) }) { saveFailedMessage }
                            next
                        } catch (cancelled: CancellationException) { throw cancelled }
                        catch (failure: Exception) {
                            val recovered = try { dataSource.directory(state.conversationId, editorScope?.storeEpoch) }
                            catch (cancelled: CancellationException) { throw cancelled }
                            catch (_: Exception) { null }
                            if (recovered?.agents?.any { confirmsExpertDefinition(it, attempted) } != true) throw failure
                            recovered
                        }
                        dataSource.refreshConversationExperts(restoreSelection = false); editing = false; editingID = null
                    } }) { Text(stringResource(R.string.common_save)) }
            }
        } else {
            val agents = directory?.agents.orEmpty()
            val darkAppearance = MaterialTheme.colorScheme.background.luminance() < 0.5f
            if (directory != null && agents.isEmpty() && !busy) {
                NolaEmptyState(
                    icon = Icons.Filled.Person,
                    title = stringResource(R.string.expert_empty_title),
                    message = stringResource(R.string.expert_empty_message),
                    action = if (catalogReady) {
                        {
                            Button(onClick = startCreateExpert) {
                                Text(stringResource(R.string.room_create_agent))
                            }
                        }
                    } else null,
                )
            } else {
                LazyColumn(
                    modifier = (if (isFullScreen) Modifier.weight(1f) else Modifier.heightIn(max = 420.dp))
                        .fillMaxWidth()
                        .testTag("expert.directory.list")
                ) {
                    items(agents, key = PersistentAgent::id) { agent ->
                        val bubbleColor = rememberExpertBubbleColor(agent.id)
                        val initial = agent.name.trim().firstOrNull()?.uppercaseChar()?.toString() ?: "E"
                        val roleId = agent.defaults["selectedPromptTemplateID"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.contentOrNull
                        val roleLabel = roleId?.let { id ->
                            roles.firstOrNull { it.id == id }?.let { expertRoleText(it.name, language) }
                                ?: templateCatalog?.templates?.firstOrNull { it.id == id }?.name
                        }
                        ListItem(
                            colors = ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
                            leadingContent = {
                                Box(
                                    modifier = Modifier
                                        .size(38.dp)
                                        .clip(CircleShape)
                                        .background(bubbleColor.background(darkAppearance))
                                        .border(0.5.dp, MaterialTheme.colorScheme.outlineVariant, CircleShape),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Text(
                                        initial,
                                        style = MaterialTheme.typography.titleSmall,
                                        fontWeight = FontWeight.SemiBold,
                                        color = MaterialTheme.colorScheme.onSurface,
                                    )
                                }
                            },
                            headlineContent = { Text(agent.name, fontWeight = FontWeight.Medium) },
                            supportingContent = {
                                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    if (agent.availability == "archived" || !roleLabel.isNullOrBlank()) {
                                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                                            if (!roleLabel.isNullOrBlank()) {
                                                NolaBadge(text = roleLabel, tint = MaterialTheme.colorScheme.primary)
                                            }
                                            if (agent.availability == "archived") {
                                                NolaBadge(
                                                    text = stringResource(R.string.expert_archived),
                                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                                )
                                            }
                                        }
                                    }
                                    if (agent.description.isNotBlank()) Text(agent.description)
                                }
                            },
                            modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp).testTag("expert.row.${agent.id}")
                                .clickable(enabled = !busy && catalogReady) {
                                    editAgent(agent)
                                },
                            trailingContent = {
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
                                        val conflictMessage = stringResource(R.string.expert_conflict_refresh)
                                        DropdownMenuItem(text = { Text(stringResource(if (archived) R.string.expert_restore else R.string.expert_archive)) },
                                            modifier = Modifier.testTag("expert.availability.${agent.id}"),
                                            enabled = !busy && agent.availability in setOf("active", "archived"), onClick = {
                                                actionsID = null
                                                perform {
                                                    val snapshot = checkNotNull(directory)
                                                    val original = snapshot.agents.single { it.id == agent.id }
                                                    check(original.revision == agent.revision && original.availability == agent.availability) {
                                                        conflictMessage
                                                    }
                                                    directory = dataSource.updateAgent(
                                                        state.conversationId,
                                                        editorScope?.storeEpoch,
                                                        original.copy(availability = if (archived) "active" else "archived"),
                                                        original.name,
                                                        original.description,
                                                        snapshot,
                                                    )
                                                    dataSource.refreshConversationExperts(restoreSelection = false)
                                                }
                                            })
                                    }
                                }
                            }
                        )
                    }
                }
            }
        }
        }
    }
}
