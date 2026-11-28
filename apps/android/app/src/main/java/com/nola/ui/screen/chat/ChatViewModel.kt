package com.nola.ui.screen.chat

import com.nola.ui.components.BuiltinToolNames
import com.nola.data.local.readableExpertTemplates
import com.nola.data.local.*
import com.nola.data.local.requireIdentity
import com.nola.data.local.loadPreparedEditedResend
import com.nola.data.local.loadRunIdentity
import com.nola.data.local.prepareInPlaceResend
import com.nola.data.local.inPlace
import com.nola.data.local.prepareEditedResend
import com.nola.data.local.AndroidPreparedEditedResend
import com.nola.data.local.AndroidPrepareEditedResend
import com.nola.data.local.listUiSurfaces
import com.nola.data.local.remoteUiEpoch
import com.nola.data.local.previewUiSurface
import com.nola.data.local.readUiSurfaceRevision
import com.nola.data.local.readUiSurfaceChanges
import com.nola.data.local.submitUiInteraction

import com.nola.agent.context.ChatContextPreparationRequest
import com.nola.agent.context.ChatContextPreparer
import android.content.Context
import android.util.Log
import android.net.Uri
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.nola.R
import com.nola.llm.local.DeviceModelStore
import com.nola.llm.provider.LocalLlamaProvider
import com.nola.llm.provider.LocalModelRuntimeState
import com.nola.agent.trace.AgentTraceEvent
import com.nola.agent.trace.AgentTraceProjection
import com.nola.agent.trace.AgentTraceSession
import com.nola.core.AgentCoreMessage
import com.nola.core.AgentCoreRuntimeConfig
import com.nola.core.AndroidAgentExecutionMode
import com.nola.core.DeviceAgentCoordinatorEvent
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.decodeAgentCoreTaskProjection
import com.nola.core.AgentCoreSnapshot
import com.nola.core.AgentCoreContextComponent
import com.nola.core.AgentCoreAndroidToolPlan
import com.nola.core.AndroidContextRoute
import com.nola.core.AndroidPreparedContext
import com.nola.core.AndroidAgentPermissionAction
import com.nola.core.AndroidAgentPermissionDecision
import com.nola.core.AndroidAgentPermissionRequest
import com.nola.core.AndroidAgentTurnRequest
import com.nola.core.AndroidAgentTurnService
import com.nola.core.modelFacingTools
import com.nola.core.AndroidCanonicalTurnDescriptor
import com.nola.core.IntelligenceEvent
import com.nola.core.IntelligenceIngestRequest
import com.nola.core.IntelligenceTaskOutcome
import com.nola.core.PersonalIntelligenceCore
import com.nola.data.health.AndroidHealthAvailability
import com.nola.data.health.AndroidHealthDataSource
import com.nola.data.local.datastore.ModelParametersStore
import com.nola.data.local.datastore.PreferenceDataStore
import com.nola.data.repository.ChatRepository
import com.nola.data.repository.AgentTraceRepository
import com.nola.data.repository.ContextRepository
import com.nola.data.repository.ConversationRepository
import com.nola.data.repository.ModelRepository
import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.dto.HOST_PERMISSION_PROTOCOL_VERSION
import com.nola.domain.model.ChatRole
import com.nola.domain.model.Conversation
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceCatalogBuilder
import com.nola.domain.model.DataSourceUsageRecord
import com.nola.domain.model.HostAgentPermissionRequest
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.decodeAgentThinkingFromMeta
import com.nola.domain.model.decodeHostToolEventsFromMeta
import com.nola.domain.projection.CONVERSATION_SURFACE_LIVE_META_KEY
import com.nola.domain.projection.CanonicalConversationProjector
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.CompactionSourceNavigator
import com.nola.domain.model.HostComputerUseHost
import com.nola.domain.model.HostComputerUseStatus
import com.nola.domain.model.HostNativeBrowserHost
import com.nola.domain.model.HostNativeBrowserStatus
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.McpServerConfig
import com.nola.domain.model.MemorySearchHit
import com.nola.domain.model.ModelInput
import com.nola.domain.model.ModelParameters
import com.nola.domain.model.ProcessingState
import com.nola.domain.model.UploadedContent
import com.nola.domain.model.AnswerSource
import com.nola.domain.model.UnifiedDataSource
import com.nola.domain.model.UnifiedDataSourceConnectionState
import com.nola.llm.ModelService
import com.nola.llm.provider.WebAgentProvider
import dagger.hilt.android.lifecycle.HiltViewModel
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.collect
import com.nola.core.AndroidChildPermissions
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import com.nola.data.local.conversationSourceOwner
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.long
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.util.TimeZone
import java.util.UUID
import javax.inject.Inject

internal fun conversationPermissionMode(
    stored: String?,
    default: com.nola.core.AndroidAgentPermissionMode,
): com.nola.core.AndroidAgentPermissionMode = when (stored) {
    null -> default
    "cautious" -> com.nola.core.AndroidAgentPermissionMode.Cautious
    "allow_all" -> com.nola.core.AndroidAgentPermissionMode.AllowAll
    else -> throw IllegalArgumentException("Permission profile must be cautious or allow_all")
}

private val CHAT_ATTACHMENT_BLOCK = Regex(
    pattern = """<attachment\b[^>]*>.*?</attachment>""",
    options = setOf(RegexOption.IGNORE_CASE, RegexOption.DOT_MATCHES_ALL),
)

private fun List<JsonObject>.dropLastUserMessage(): List<JsonObject> {
    val lastUserIndex = indexOfLast { message ->
        message["role"]?.jsonPrimitive?.contentOrNull == "user"
    }
    if (lastUserIndex < 0) return this
    return filterIndexed { index, _ -> index != lastUserIndex }
}

private fun agentCoreMessage(message: JsonObject): AgentCoreMessage? {
    val role = message["role"]?.jsonPrimitive?.contentOrNull ?: return null
    val content = message["content"]?.jsonPrimitive?.contentOrNull.orEmpty()
    val toolCalls = message["tool_calls"]?.jsonArray?.mapNotNull { it as? JsonObject }.orEmpty()
    if (content.isBlank() && toolCalls.isEmpty()) return null
    return AgentCoreMessage(
        role = role,
        content = content,
        name = message["name"]?.jsonPrimitive?.contentOrNull,
        toolCallId = message["tool_call_id"]?.jsonPrimitive?.contentOrNull,
        toolCalls = toolCalls,
    )
}

@HiltViewModel
class ChatViewModel @Inject constructor(
    private val conversationRepo: ConversationRepository,
    private val contextRepo: ContextRepository,
    private val chatRepo: ChatRepository,
    private val memoryProfileDataStore: com.nola.data.local.datastore.MemoryProfileDataStore,
    private val modelRepo: ModelRepository,
    private val modelService: ModelService,
    private val localLlamaProvider: LocalLlamaProvider,
    private val deviceModelStore: DeviceModelStore,
    private val webAgentProvider: WebAgentProvider,
    private val assistantJournal: com.nola.data.local.AndroidAssistantJournal,
    private val prefs: PreferenceDataStore,
    private val modelParamsStore: ModelParametersStore,
    private val agentTraceRepository: AgentTraceRepository,
    private val healthDataSource: AndroidHealthDataSource,
    private val agentTurnService: AndroidAgentTurnService,
    private val contextPreparer: ChatContextPreparer,
    private val roomCoordinator: com.nola.data.repository.RoomCoordinator,
    private val applicationRuntime: com.nola.data.local.AndroidApplicationRuntime,
    @ApplicationContext private val context: Context
) : ViewModel() {

    private val uiIntents = com.nola.data.local.UiInteractionIntentStore(java.io.File(context.noBackupFilesDir, "ui-submission-intents.json"))
    internal suspend fun restoreUiInteraction(identity: com.nola.data.local.UiSurfaceIdentity): com.nola.data.local.UiSubmission? =
        withContext(Dispatchers.IO) { uiIntents.pending(identity) }

    internal suspend fun loadUiSurfaces(conversationId: String): List<com.nola.data.local.UiSnapshot> =
        withContext(Dispatchers.IO) {
            val conversation = checkNotNull(conversationRepo.getConversation(conversationId)) {
                "Conversation is unavailable"
            }
            val binding = conversation.executionHostBinding
            if (binding != null) {
                val configuration = resolveBoundHost(binding)
                val epoch = applicationRuntime.remoteUiEpoch(conversationId, configuration.expectedHostId)
                    ?: return@withContext emptyList()
                webAgentProvider.listUiSurfaces(conversationId, configuration, epoch)
            } else {
                check(conversation.selectedModelInfo?.source != ModelInfo.ModelSource.WebHost) {
                    "Remote conversation has no bound Host"
                }
                applicationRuntime.listUiSurfaces(conversationId)
            }
        }

    internal suspend fun loadUiSurfaceChanges(identity: com.nola.data.local.UiSurfaceIdentity, baseline: com.nola.data.local.UiSnapshot?): com.nola.data.local.UiSnapshot? =
        withContext(Dispatchers.IO) {
            val conversation = checkNotNull(conversationRepo.getConversation(identity.owner.conversation_id))
            val binding = conversation.executionHostBinding
            val update = if (binding != null) webAgentProvider.readUiSurfaceChanges(identity, baseline?.revision, resolveBoundHost(binding))
            else {
                check(conversation.selectedModelInfo?.source != ModelInfo.ModelSource.WebHost) { "Remote conversation has no bound Host" }
                applicationRuntime.readUiSurfaceChanges(identity, baseline?.revision)
            }
            com.nola.data.local.applyUiSurfaceUpdate(identity, baseline, update)
        }

    internal suspend fun loadUiSurfaceRevision(reference: com.nola.data.local.UiSurfaceRevisionRef): com.nola.data.local.UiSnapshot? =
        withContext(Dispatchers.IO) {
            val conversation = checkNotNull(conversationRepo.getConversation(reference.identity.owner.conversation_id))
            val binding = conversation.executionHostBinding
            if (binding != null) webAgentProvider.readUiSurfaceRevision(reference, resolveBoundHost(binding))
            else {
                check(conversation.selectedModelInfo?.source != ModelInfo.ModelSource.WebHost) { "Remote conversation has no bound Host" }
                applicationRuntime.readUiSurfaceRevision(reference.identity, reference.revision)
            }
        }

    internal suspend fun previewUiSurface(input: com.nola.data.local.UiPreviewInput): com.nola.data.local.UiPreview =
        withContext(Dispatchers.IO) {
            val conversation = checkNotNull(conversationRepo.getConversation(input.identity.owner.conversation_id))
            val binding = conversation.executionHostBinding
            if (binding != null) webAgentProvider.previewUiSurface(input, resolveBoundHost(binding))
            else {
                check(conversation.selectedModelInfo?.source != ModelInfo.ModelSource.WebHost) { "Remote conversation has no bound Host" }
                applicationRuntime.previewUiSurface(input)
            }
        }

    internal suspend fun submitUiInteraction(input: com.nola.data.local.UiSubmission): com.nola.ui.generative.UiSubmissionOutcome =
        withContext(Dispatchers.IO) {
            try {
                val conversationId = input.interaction.identity.owner.conversation_id
                val conversation = checkNotNull(conversationRepo.getConversation(conversationId)) {
                    "Conversation is unavailable"
                }
                val binding = conversation.executionHostBinding
                if (binding != null) {
                    val configuration = resolveBoundHost(binding)
                    val intent = uiIntents.stage(input, buildJsonObject {
                        put("host_id", configuration.expectedHostId); put("base_url", configuration.hostBaseUrl)
                        put("certificate", configuration.pinnedCertificateSha256)
                    })
                    val result = webAgentProvider.submitUiInteraction(input, configuration)
                    if (result.status != "unknown") uiIntents.retire(intent)
                    if (result.status == "committed" && result.receipt?.queued_submission_id != null) {
                        viewModelScope.launch(Dispatchers.Main.immediate) {
                            remoteUiWakeSuppressed.remove(conversationId)
                            wakeRemoteUiQueue(conversationId)
                        }
                    }
                    return@withContext when (result.status) {
                        "committed" -> com.nola.ui.generative.UiSubmissionOutcome.Committed(checkNotNull(result.receipt))
                        "rejected", "conflict" -> com.nola.ui.generative.UiSubmissionOutcome.Rejected(
                            result.message ?: context.getString(R.string.chat_ui_host_submission_rejected))
                        else -> com.nola.ui.generative.UiSubmissionOutcome.Unknown(
                            result.message ?: context.getString(R.string.chat_ui_host_submission_unknown))
                    }
                }
                check(conversation.selectedModelInfo?.source != ModelInfo.ModelSource.WebHost) {
                    "Remote conversation has no bound Host"
                }
                val intent = uiIntents.stage(input, buildJsonObject { put("local", true) })
                val receipt = try { applicationRuntime.submitUiInteraction(input) }
                catch (rejected: com.nola.data.local.AndroidApplicationRuntime.RejectedMutation) {
                    uiIntents.retire(intent); throw rejected
                }
                uiIntents.retire(intent)
                receipt.queued_submission_id?.let { commandId ->
                    viewModelScope.launch(Dispatchers.Main.immediate) {
                        dispatchQueuedUiInput(receipt.snapshot.identity.owner.conversation_id, commandId)
                    }
                }
                com.nola.ui.generative.UiSubmissionOutcome.Committed(receipt)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (rejected: com.nola.data.local.AndroidApplicationRuntime.RejectedMutation) {
                com.nola.ui.generative.UiSubmissionOutcome.Rejected(
                    rejected.message ?: context.getString(R.string.chat_ui_submission_rejected))
            } catch (error: Exception) {
                com.nola.ui.generative.UiSubmissionOutcome.Unknown(
                    error.message ?: context.getString(R.string.chat_ui_submission_unknown))
            }
        }

    private val displayedState = MutableStateFlow(ChatUiState())
    internal val agentRoomPresentation = AgentRoomPresentation()
    internal val conversationAgentDirectory = MutableStateFlow<com.nola.data.repository.AgentDirectory?>(null)
    internal val selectedConversationAgentIds = MutableStateFlow<Set<String>>(emptySet())
    private val selectedConversationAgentRevisions = mutableMapOf<String, Long>()
    private val selectedConversationAgentParticipants = androidx.compose.runtime.mutableStateMapOf<String, com.nola.data.repository.RoomParticipant>()
    private data class ExpertPickerSource(val host: String, val epoch: String)
    private var expertPickerSource: ExpertPickerSource? = null
    private suspend fun expertSelectionRepository(id: String?, refreshEmpty: Boolean = false): com.nola.data.repository.AgentRoomRepository {
        val pin = expertPickerSource
        val frozen = pin?.takeIf { selectedConversationAgentIds.value.isNotEmpty() || !refreshEmpty }
        val repository = expertRoomRepository(id, frozen?.epoch)
        require(frozen == null || repository.remoteHostId == frozen.host) { "Expert source changed; reopen the expert picker" }
        return repository
    }
    private fun mainExpertDirectory(repository: com.nola.data.repository.AgentRoomRepository, inline: Boolean = false): com.nola.data.repository.AgentDirectory {
        if (repository.remoteHostId != null) {
            val supported = try { repository.roleTemplates().features.let { it.mainAgentDelegation && (!inline || it.inlineExpertMentions) } }
            catch (failure: Exception) {
                if (failure is kotlinx.coroutines.CancellationException || failure is com.nola.data.remote.api.WebHostStatusException && failure.errorCode == "room_source_epoch_changed") throw failure
                throw IllegalStateException(context.getString(R.string.expert_role_catalog_unavailable), failure)
            }
            check(supported) { context.getString(if (inline) R.string.expert_mention_upgrade_host else R.string.expert_role_catalog_unavailable) }
        }
        return repository.directory()
    }
    private fun captureExpertPickerSource(repository: com.nola.data.repository.AgentRoomRepository) {
        val source = repository.remoteHostId?.let { ExpertPickerSource(it, checkNotNull(repository.observedSourceEpoch)) }
        require(selectedConversationAgentIds.value.isEmpty() || expertPickerSource == null || expertPickerSource == source) { "The selected expert source changed" }
        expertPickerSource = source
    }
    private val expertSelectionMutex = kotlinx.coroutines.sync.Mutex()
    internal data class ExpertRejoinRequest(val conversationId: String, val room: com.nola.data.repository.AgentRoomSnapshot,
        val agent: com.nola.data.repository.PersistentAgent)
    internal val expertRejoinRequest = MutableStateFlow<ExpertRejoinRequest?>(null)
    internal val expertRejoining = MutableStateFlow(false)
    internal val expertRejoinError = MutableStateFlow<String?>(null)
    internal val resumableExpertMessages = MutableStateFlow<Set<String>>(emptySet())
    internal val resumingExpertMessage = MutableStateFlow<String?>(null)
    internal data class ExpertParentMessageKey(val conversationId: String, val messageId: String)
    internal val conversationExpertReplies = MutableStateFlow<Map<ExpertParentMessageKey, List<ConversationExpertReply>>>(emptyMap())

    internal data class ConversationExpertReply(
        val participant: com.nola.data.repository.RoomParticipant,
        val submissionId: String,
        val state: String,
        val text: String = "",
        val hostPermission: HostAgentPermissionRequest? = null,
        val localPermission: AndroidAgentPermissionRequest? = null,
        val message: ConversationMessage? = null,
        val thinking: String = "",
        val thinkingFinished: Boolean = true,
        val isStreaming: Boolean = false,
        val toolEvents: List<com.nola.domain.model.HostAgentToolEvent> = emptyList(),
        val permissionAction: AndroidAgentPermissionAction? = null,
    )

    private var expertDirectoryRefreshGeneration = 0L
    internal fun refreshConversationExperts(restoreSelection: Boolean = false) {
        val conversationId = _uiState.value.conversationId
        val generation = ++expertDirectoryRefreshGeneration
        viewModelScope.launch {
            val repository = runCatching { expertSelectionRepository(conversationId, true) }.getOrNull()
            val directory = runCatching { withContext(Dispatchers.IO) { repository?.let { mainExpertDirectory(it) } } }.getOrNull()
            if (generation != expertDirectoryRefreshGeneration) return@launch
            if (repository != null && directory != null) captureExpertPickerSource(repository)
            conversationAgentDirectory.value = directory
            if (restoreSelection && conversationId != null) {
                val association = runCatching { withContext(Dispatchers.IO) { expertRoomRepository(conversationId).forConversation(conversationId) } }.getOrNull()
                if (generation == expertDirectoryRefreshGeneration && association != null && _uiState.value.conversationId == conversationId) {
                    val active = association.room.room.participants.filter { it.active }.map { it.agentId }.toSet()
                    association.room.room.participants.filter { it.active }.forEach { member ->
                        selectedConversationAgentRevisions.putIfAbsent(member.agentId, member.agentRevision)
                        selectedConversationAgentParticipants.putIfAbsent(member.agentId, member)
                    }
                    selectedConversationAgentIds.value = selectedConversationAgentIds.value + active
                }
            }
        }
    }

    internal fun inlineMentionCandidate(agentId: String): InlineExpertMention? {
        val agent = conversationAgentDirectory.value?.agents?.singleOrNull { it.id == agentId && it.availability == "active" } ?: return null
        val member = pickerMembers.value[agentId]
        return InlineExpertMention(0, "@" + (member?.displayName ?: agent.name), agentId, member?.agentRevision ?: agent.revision,
            member, expertPickerSource?.host, expertPickerSource?.epoch)
    }

    internal val pickerMembers = MutableStateFlow<Map<String, com.nola.data.repository.RoomParticipant>>(emptyMap())

    internal fun requestLatestExpert(agentId: String) {
        val agent = conversationAgentDirectory.value?.agents?.singleOrNull { it.id == agentId && it.availability == "active" } ?: return
        val owner = displayedState.value.conversationId ?: return
        val generation = conversationLoads.currentGeneration()
        viewModelScope.launch {
            try {
                val room = withContext(Dispatchers.IO) { expertSelectionRepository(owner).forConversation(owner)?.room } ?: return@launch
                if (generation != conversationLoads.currentGeneration() || displayedState.value.conversationId != owner) return@launch
                val member = room.room.participants.singleOrNull { it.agentId == agentId && it.active } ?: return@launch
                if (member.agentRevision == agent.revision) return@launch
                expertRejoinRequest.value = ExpertRejoinRequest(owner, room, agent)
                expertRejoinError.value = null
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (failure: Exception) { if (generation == conversationLoads.currentGeneration())
                composerDrafts.forConversation(owner).notice.value = failure.localizedMessage }
        }
    }

    internal fun cancelExpertRejoin() {
        if (expertRejoining.value) return
        expertRejoinRequest.value = null; expertRejoinError.value = null
    }

    internal fun refreshExpertRejoin() {
        val request = expertRejoinRequest.value ?: return
        if (expertRejoining.value || displayedState.value.conversationId != request.conversationId) return
        expertRejoining.value = true; expertRejoinError.value = null
        val generation = conversationLoads.currentGeneration()
        viewModelScope.launch {
            try {
                val (directory, room) = withContext(Dispatchers.IO) { val repository = expertSelectionRepository(request.conversationId); mainExpertDirectory(repository) to repository.load(request.room.room.id) }
                if (displayedState.value.conversationId != request.conversationId || conversationLoads.currentGeneration() != generation || expertRejoinRequest.value != request) return@launch
                conversationAgentDirectory.value = directory
                val agent = directory.agents.singleOrNull { it.id == request.agent.id && it.availability == "active" }
                val member = room?.room?.participants?.singleOrNull { it.agentId == request.agent.id && it.active }
                if (agent == null || room == null || member?.agentRevision == agent.revision) {
                    expertRejoinRequest.value = null
                    composerDrafts.forConversation(request.conversationId).notice.value = context.getString(R.string.expert_profile_changed)
                } else expertRejoinRequest.value = ExpertRejoinRequest(request.conversationId, room, agent)
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (failure: Exception) {
                if (expertRejoinRequest.value == request) expertRejoinError.value = failure.localizedMessage
            } finally { expertRejoining.value = false }
        }
    }

    internal fun confirmExpertRejoin() {
        val request = expertRejoinRequest.value ?: return
        if (expertRejoining.value || displayedState.value.conversationId != request.conversationId) return
        val generation = conversationLoads.currentGeneration()
        fun current() = displayedState.value.conversationId == request.conversationId &&
            conversationLoads.currentGeneration() == generation && expertRejoinRequest.value == request
        expertRejoining.value = true; expertRejoinError.value = null
        viewModelScope.launch {
            expertSelectionMutex.withLock {
                try {
                    if (!current()) return@withLock
                    val joined = withContext(Dispatchers.IO) {
                        val agentRooms = expertSelectionRepository(request.conversationId)
                        try { agentRooms.roleTemplates() }
                        catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                        catch (_: Exception) { throw IllegalStateException(context.getString(R.string.expert_role_catalog_unavailable)) }
                        try { agentRooms.rejoin(request.agent, request.room) }
                        catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                        catch (failure: Exception) {
                            val observed = try { agentRooms.load(request.room.room.id) }
                            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                            catch (_: Exception) { null }
                            if (observed == null) throw failure
                            try { rejoinedExpertParticipant(request.room, observed, request.agent) }
                            catch (_: Exception) { throw failure }
                            observed
                        }
                    }
                    if (!current()) return@withLock
                    val joinedMember = rejoinedExpertParticipant(request.room, joined, request.agent)
                    pickerMembers.update { it + (request.agent.id to joinedMember) }
                    // Explicitly swap all occurrences of this one expert in the owning unsent draft.
                    val draft = composerDrafts.forConversation(request.conversationId)
                    val current = draft.text.value
                    var edit = MentionEdit(androidx.compose.ui.text.input.TextFieldValue(current), draft.mentions)
                    for (previous in draft.mentions.asReversed().filter { it.agentId == request.agent.id }) {
                        val replacement = previous.copy(label = "@${joinedMember.displayName}", revision = joinedMember.agentRevision, member = joinedMember)
                        edit = insertExpert(edit.value, edit.mentions, androidx.compose.ui.text.TextRange(previous.start, previous.end), replacement)
                    }
                    draft.setContent(edit.value.text, edit.mentions)
                    expertRejoinRequest.value = null
                } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (current()) expertRejoinError.value = failure.localizedMessage ?: context.getString(R.string.expert_profile_changed)
                } finally { expertRejoining.value = false }
            }
        }
    }

    private val pendingExpertAdmissionByDraft = CanonicalUserMessageAdmissionRetryState()
    private val activeExpertAdmissionDrafts = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()

    private suspend fun dispatchExpertManifest(manifest: AndroidExpertMessageManifest) {
        val room = withContext(Dispatchers.IO) { checkNotNull(agentRooms.load(manifest.input.roomId)) }
        check(manifest.entries.size == manifest.input.selectedParticipants.size &&
            manifest.entries.map { it.participant } == manifest.input.selectedParticipants)
        val selected = manifest.entries.map { entry ->
            val receipt = withContext(Dispatchers.IO) { checkNotNull(agentRooms.submission(manifest.input.roomId, entry.childSubmissionId)) }
            check(receipt.contextConversationId == entry.participant.contextConversationId &&
                receipt.selectedParticipantId == entry.participant.participantId &&
                receipt.input.parentMessageId == manifest.input.userMessageId && receipt.input.text == manifest.input.text)
            receipt
        }
        roomSubmissions[room.room.id] = mergeRoomSubmissions(room.room.id, roomSubmissions[room.room.id].orEmpty(), selected)
        for (receipt in selected) {
            val owner = room.room.participants.single { it.id == receipt.selectedParticipantId }
            publishConversationExpertReply(manifest.input.userMessageId, owner, receipt, "")
            if (receipt.state in setOf("selected", "executing")) {
                viewModelScope.launch { dispatchRoomSubmission(room, receipt) }
            }
        }
    }

    /** A private native intent has its own family. Never send its request ID
     * through ordinary Application receipt recovery or clear a newer draft. */
    fun checkOriginalExpertHostRequest() {
        val state = _uiState.value
        val owner = state.conversationId ?: return
        if (state.checkingExpertHostAdmission) return
        val pending = state.pendingExpertHostAdmission ?: run { loadConversation(owner); return }
        val generation = conversationLoads.currentGeneration()
        fun current() = _uiState.value.conversationId == owner && conversationLoads.isCurrent(generation)
        _uiState.update { it.copy(checkingExpertHostAdmission = true, error = null) }
        viewModelScope.launch {
            var confirmed = false
            try {
                val manifest = withContext(Dispatchers.IO) { applicationRuntime.expertHostFactory.checkAdmission(pending) }
                check(manifest.input == pending.input && manifest.input.conversationId == owner)
                confirmed = true
                dispatchExpertManifest(manifest)
                val draftOwner = composerDrafts.ownerKey(owner)
                pendingExpertAdmissionByDraft.find(draftOwner, owner)?.let { pendingExpertAdmissionByDraft.confirm(draftOwner, it) }
                // No old submission ticket is acknowledged here: the composer
                // may contain input written after the interrupted invocation.
                if (current()) {
                    _uiState.update { it.copy(pendingExpertHostAdmission = null, expertHostConfirmationStatus = null,
                        checkingExpertHostAdmission = false) }
                    loadConversation(owner, background = false, reconcileNativeIntent = true)
                }
            } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
            catch (pendingOutcome: AndroidApplicationRuntime.ExpertHostPending) {
                if (current()) _uiState.update { it.copy(checkingExpertHostAdmission = false,
                    pendingExpertHostAdmission = pending.copy(requestId = pendingOutcome.requestId),
                    expertHostConfirmationStatus = context.getString(if (pendingOutcome.reopenRequired)
                        R.string.expert_host_reopen_confirmation else R.string.expert_host_awaiting_confirmation), error = null) }
            } catch (failure: Exception) {
                if (current()) _uiState.update {
                    if (!confirmed && failure is AndroidApplicationRuntime.ExpertHostRejected)
                        it.copy(pendingExpertHostAdmission = null, expertHostConfirmationStatus = null,
                            checkingExpertHostAdmission = false, processingState = ProcessingState.Error, error = failure.message)
                    else it.copy(checkingExpertHostAdmission = false, expertHostConfirmationStatus =
                        context.getString(R.string.expert_host_awaiting_confirmation), error = null)
                }
            }
        }
    }

    private suspend fun admitAndFanOutExpertOnly(
        text: String,
        messageMeta: Map<String, String>?,
        submittedState: ChatUiState,
        selectedAgentIds: Set<String>,
        draftOwner: String,
    ) {
        require(text.isNotBlank() && selectedAgentIds.isNotEmpty())
        require(messageMeta.isNullOrEmpty()) { "Expert-only messages currently support plain text only" }
        val selectedRevisions = selectedConversationAgentRevisions.toMap()
        val selectedMembers = selectedConversationAgentParticipants.toMap()
        val capture = composerDrafts.captureSubmission(draftOwner)
        val draftTicket = composerDrafts.prepareSubmission(capture)
        val previousAdmission = pendingExpertAdmissionByDraft.find(draftOwner, submittedState.conversationId)
        val conversationId = previousAdmission?.conversationId ?: ensureConversation(submittedState, draftOwner)
        val conversation = requireNotNull(conversationRepo.getConversation(conversationId)) {
            context.getString(R.string.chat_conversation_unavailable)
        }
        check(submittedState.conversationId == null || submittedState.configurationRevision == conversation.configurationRevision) {
            "Conversation configuration changed. Reload before submitting; your draft is retained."
        }
        if (conversation.executionHostBinding != null) {
            val recovered = withContext(Dispatchers.IO) { readRemoteExpertRetry(resolveBoundHost(conversation.executionHostBinding), conversationId) }
            recovered?.let { pendingExpertAdmissionByDraft.restore(draftOwner, it.input) }
        }
        val pending = pendingExpertAdmissionByDraft.beginOrResume(draftOwner, conversationId, text, selectedAgentIds)
        selectedConversationAgentIds.value = pending.selectedAgentIds
        val messageId = pending.messageId
        val manifest = if (conversation.executionHostBinding != null) {
            admitRemoteExpertOnly(conversation, pending, selectedRevisions, selectedMembers)
        } else withContext(Dispatchers.IO) {
            val prior = applicationRuntime.expertMessageSource(conversationId, messageId)
            if (prior != null) {
                check(prior.input.text == text && prior.input.selectedParticipants.map { it.agentId }.toSet() == pending.selectedAgentIds)
                applicationRuntime.admitExpertMessage(prior.input)
            } else {
                var room = agentRooms.ensureForConversation(conversationId).room
                var directory: com.nola.data.repository.AgentDirectory? = null
                for (id in pending.selectedAgentIds.sorted()) {
                    if (selectedExpertParticipant(room, id, selectedRevisions[id], selectedMembers[id]) == null) {
                        val currentDirectory = directory ?: agentRooms.directory().also { directory = it }
                        val candidate = currentDirectory.agents.singleOrNull { it.id == id && it.availability == "active" }
                            ?: throw ExpertSelectionChanged()
                        val agent = checkedExpertCandidate(currentDirectory, id, selectedRevisions[id] ?: candidate.revision)
                        room = agentRooms.join(agent, room)
                    }
                }
                val participants = pending.selectedAgentIds.sorted().map { id ->
                    val member = room.room.participants.singleOrNull { it.agentId == id && it.active }
                        ?: throw ExpertSelectionChanged()
                    AndroidExpertParticipantReference(member.id, member.agentId, member.agentRevision, member.contextConversationId)
                }
                applicationRuntime.admitExpertMessage(AndroidAdmitExpertMessage(conversationId,
                    checkNotNull(applicationRuntime.loadConversation(conversationId)).revision, room.room.id, room.revision,
                    messageId, messageId, text, participants, pending.createdAtMs))
            }
        }
        check(manifest.input.conversationId == conversationId && manifest.input.userMessageId == messageId)
        val persisted = conversationRepo.getActiveMessages(conversationId)
        val canonical = persisted.singleOrNull { it.id == messageId && it.role == ChatRole.user && it.text == text }
            ?: error("Canonical user message was not visible after admission")
        conversationLoads.invalidate()
        _uiState.update { current ->
            if (current.conversationId != null && current.conversationId != conversationId) current
            else current.copy(
                conversationId = conversationId,
                configurationRevision = conversation.configurationRevision,
                messages = if (current.messages.any { it.id == messageId }) current.messages else current.messages + canonical,
                processingState = ProcessingState.BuildingContext,
                error = null,
            )
        }
        if (conversation.executionHostBinding == null) dispatchExpertManifest(manifest)
        else startRemoteExpertObservation(conversationId)
        composerDrafts.submitted(draftTicket)
        pendingExpertAdmissionByDraft.confirm(draftOwner, pending)
        conversation.executionHostBinding?.let { binding -> withContext(Dispatchers.IO) {
            val configuration = resolveBoundHost(binding)
            if (readRemoteExpertRetry(configuration, conversationId)?.input?.userMessageId == pending.messageId) {
                check(remoteExpertRetryStorage.edit().remove(remoteExpertRetryKey(configuration.expectedHostId, configuration.clientId, conversationId)).commit()) { "Could not finalize the confirmed expert submission" }
            }
        } }
        _uiState.update { if (it.conversationId == conversationId) it.copy(processingState = ProcessingState.Idle, error = null) else it }
    }

    private val remoteExpertRetryStorage by lazy { context.getSharedPreferences("remote-expert-retry-intents", android.content.Context.MODE_PRIVATE) }
    private fun readRemoteExpertRetry(configuration: WebAgentProvider.Config, parent: String): RemoteExpertRetryIntent? =
        remoteExpertRetryStorage.getString(remoteExpertRetryKey(configuration.expectedHostId, configuration.clientId, parent), null)?.let {
            Json.decodeFromString<RemoteExpertRetryIntent>(it).validated(configuration.expectedHostId, configuration.clientId, parent)
        }
    private fun saveRemoteExpertRetry(configuration: WebAgentProvider.Config, intent: RemoteExpertRetryIntent) {
        intent.validated(configuration.expectedHostId, configuration.clientId, intent.input.conversationId)
        val key = remoteExpertRetryKey(configuration.expectedHostId, configuration.clientId, intent.input.conversationId)
        val previous = readRemoteExpertRetry(configuration, intent.input.conversationId)
        require(previous == null || previous == intent) { "An earlier remote expert submission still requires confirmation" }
        check(remoteExpertRetryStorage.edit().putString(key, Json.encodeToString(RemoteExpertRetryIntent.serializer(), intent)).commit()) { "Could not preserve the remote expert submission" }
    }

    private suspend fun admitRemoteExpertOnly(conversation: Conversation,
        pending: PendingCanonicalUserMessageAdmission, revisions: Map<String, Long>, members: Map<String, com.nola.data.repository.RoomParticipant>): AndroidExpertMessageManifest = withContext(Dispatchers.IO) {
        val binding = checkNotNull(conversation.executionHostBinding)
        val configuration = resolveBoundHost(binding)
        val repository = expertRoomRepository(conversation.id)
        val prepared = webAgentProvider.expertMessageRequest("parent", buildJsonObject {
            put("conversation_id", conversation.id); put("title", conversation.title.orEmpty())
        }, configuration)
        val epoch = prepared.getValue("source_store_epoch").jsonPrimitive.content.also { require(it.isNotBlank()) }
        val source = applicationRuntime.conversationSourceOwner(conversation.id)
        if (source is com.nola.data.local.ConversationSourceOwner.Remote) require(source.hostId == configuration.expectedHostId && source.storeEpoch == epoch)
        var remote = com.nola.data.remote.api.decodeRemoteExpertAdmission(webAgentProvider.expertMessageRequest("manifest", buildJsonObject {
            put("conversation_id", conversation.id); put("message_id", pending.messageId)
        }, configuration), configuration.expectedHostId, configuration.clientId, conversation.id, pending.messageId)
        if (remote == null) {
            val saved = readRemoteExpertRetry(configuration, conversation.id)
            require(saved == null || saved.storeEpoch == epoch && saved.input.userMessageId == pending.messageId && saved.input.text == pending.text)
            val input = saved?.input ?: run {
            var room = Json.decodeFromJsonElement<com.nola.data.repository.ConversationRoomAssociation>(prepared.getValue("room")).room
            for (id in pending.selectedAgentIds.sorted()) {
                if (selectedExpertParticipant(room, id, revisions[id], members[id]) == null) {
                    val directory = repository.directory()
                    val candidate = directory.agents.singleOrNull { it.id == id && it.availability == "active" }
                        ?: throw ExpertSelectionChanged()
                    room = repository.join(checkedExpertCandidate(directory, id, revisions[id] ?: candidate.revision), room)
                }
            }
            val participants = pending.selectedAgentIds.sorted().map { id ->
                val member = room.room.participants.singleOrNull { it.agentId == id && it.active }
                    ?: throw ExpertSelectionChanged()
                AndroidExpertParticipantReference(member.id, member.agentId, member.agentRevision, member.contextConversationId)
            }
            AndroidAdmitExpertMessage(conversation.id, prepared.getValue("parent").jsonObject.getValue("revision").jsonPrimitive.long,
                room.room.id, room.revision, pending.messageId, pending.messageId, pending.text, participants, pending.createdAtMs)
            }
            val frozen = RemoteExpertRetryIntent(hostId = configuration.expectedHostId, clientId = configuration.clientId, storeEpoch = epoch, input = input)
            saveRemoteExpertRetry(configuration, frozen)
            remote = try { checkNotNull(com.nola.data.remote.api.decodeRemoteExpertAdmission(webAgentProvider.expertMessageRequest("admit", buildJsonObject {
                put("input", Json.encodeToJsonElement(AndroidAdmitExpertMessage.serializer(), input))
            }, configuration), configuration.expectedHostId, configuration.clientId, conversation.id, pending.messageId)) }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (failure: Exception) {
                if (resolveBoundHost(binding) == configuration && com.nola.data.remote.api.definitiveRemoteExpertRejection(failure, configuration.expectedHostId, configuration.clientId, input) && readRemoteExpertRetry(configuration, conversation.id) == frozen) {
                    check(remoteExpertRetryStorage.edit().remove(remoteExpertRetryKey(configuration.expectedHostId, configuration.clientId, conversation.id)).commit()) { "Could not finalize the rejected original submission" }
                    pendingExpertAdmissionByDraft.confirm(composerDrafts.ownerKey(conversation.id), pending)
                }
                throw failure
            }
        }
        require(remote.manifest.input.text == pending.text && remote.manifest.input.selectedParticipants.map { it.agentId }.toSet() == pending.selectedAgentIds)
        require(remote.parentAdmission.getValue("source_store_epoch").jsonPrimitive.content == epoch && resolveBoundHost(binding) == configuration)
        applicationRuntime.execute(buildJsonObject {
            put("type", "receive_remote_expert_parent_admission"); put("expected_source_host_id", configuration.expectedHostId)
            put("expected_source_store_epoch", epoch); put("admission", remote.parentAdmission)
        })
        remote.manifest
    }

    private var remoteExpertObservation: Job? = null
    private var remoteExpertObservationOwner: String? = null
    private fun startRemoteExpertObservation(parent: String) {
        if (remoteExpertObservationOwner == parent && remoteExpertObservation?.isActive == true) return
        remoteExpertObservation?.cancel(); remoteExpertObservationOwner = parent
        val generation = conversationLoads.currentGeneration()
        remoteExpertObservation = viewModelScope.launch {
            var failures = 0
            while (displayedState.value.conversationId == parent && conversationLoads.currentGeneration() == generation) {
                try {
                    val ongoing = loadRemoteExpertReplies(parent, generation)
                    failures = 0
                    if (!ongoing) break
                    kotlinx.coroutines.delay(750)
                } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (displayedState.value.conversationId != parent || conversationLoads.currentGeneration() != generation) break
                    composerDrafts.forConversation(parent).notice.value = failure.localizedMessage ?: context.getString(R.string.expert_source_unavailable)
                    failures = (failures + 1).coerceAtMost(5); kotlinx.coroutines.delay(1000L shl failures)
                }
            }
        }
    }

    private fun ensureRemoteExpertChildObservation(parent: String, participant: com.nola.data.repository.RoomParticipant,
        receipt: com.nola.data.repository.RoomSubmission, assistantId: String, configuration: WebAgentProvider.Config): ChatRunSession {
        val key = composerDrafts.ownerKey(participant.contextConversationId)
        val session = runRegistry.session(key, ChatUiState(conversationId = participant.contextConversationId,
            currentModel = ModelInfo("web-host", "Nola Web Host", ModelInfo.ModelSource.WebHost), processingState = ProcessingState.Streaming,
            expertContextOwner = com.nola.domain.model.AgentRoomContextOwner(receipt.input.roomId, participant.id, participant.agentId),
            expertContextParentId = receipt.input.conversationId))
        if (session.initialKey == com.nola.data.remote.api.RemoteRunKey(participant.contextConversationId, checkNotNull(receipt.executionId), assistantId) && session.configuration == configuration) return session
        session.observer.reset(); session.presentationJob?.cancel()
        session.hasCommittedTerminalSeal = false
        session.state.value = session.state.value.copy(currentResponse = "", journalResponse = null, activeResponseId = assistantId,
            thinking = "", thinkingFinished = false, permissionRequest = null, pendingPermissionRequests = emptyList(),
            agentPermissionRequest = null, agentPermissionAction = null,
            processingState = ProcessingState.Streaming)
        runRegistry.enter(session) {
            session.configuration = configuration
            session.initialKey = com.nola.data.remote.api.RemoteRunKey(participant.contextConversationId, checkNotNull(receipt.executionId), assistantId)
            session.permissionSourceUnconfirmed = true
            startRemoteConversationObservation(participant.contextConversationId)
        }
        session.presentationJob?.cancel()
        session.presentationJob = runRegistry.scope(session).launch {
            session.state.collect { childState ->
                publishConversationExpertReply(parent, participant, receipt, childState.currentResponse,
                    hostPermission = childState.permissionRequest, localPermission = childState.agentPermissionRequest,
                    ownerState = childState, terminalMessage = childState.journalResponse)
            }
        }
        return session
    }

    private suspend fun loadRemoteExpertReplies(parent: String, generation: Long): Boolean = withContext(Dispatchers.IO) {
        val conversation = checkNotNull(conversationRepo.getConversation(parent))
        val binding = checkNotNull(conversation.executionHostBinding); val configuration = resolveBoundHost(binding)
        val repository = expertRoomRepository(parent)
        fun current() = displayedState.value.conversationId == parent && conversationLoads.currentGeneration() == generation
        val visible = displayedState.value.timelineMessages().filter { it.role == ChatRole.user }.map { it.id }
        val rows = mutableMapOf<ExpertParentMessageKey, List<ConversationExpertReply>>()
        var ongoing = false
        for (message in visible) {
            val source = com.nola.data.remote.api.decodeRemoteExpertAdmission(webAgentProvider.expertMessageRequest("manifest", buildJsonObject {
                put("conversation_id", parent); put("message_id", message)
            }, configuration), configuration.expectedHostId, configuration.clientId, parent, message) ?: continue
            val replies = source.manifest.entries.map { entry ->
                val receipt = checkNotNull(repository.submission(source.manifest.input.roomId, entry.childSubmissionId))
                val participant = com.nola.data.repository.RoomParticipant(entry.participant.participantId, entry.participant.agentId,
                    entry.participant.agentRevision, entry.displayName, entry.participant.contextConversationId, true, entry.description)
                require(receipt.contextConversationId == participant.contextConversationId && receipt.selectedParticipantId == participant.id &&
                    receipt.input.conversationId == parent && receipt.input.parentMessageId == message)
                val roomConfiguration = configuration.copy(clientId = "room:${source.manifest.input.roomId}")
                val session = receipt.executionId?.let {
                    withContext(Dispatchers.Main.immediate) {
                        ensureRemoteExpertChildObservation(message, participant, receipt, entry.childAssistantMessageId, roomConfiguration)
                    }
                }
                val canonical = session?.state?.value?.journalResponse ?: conversationRepo.getActiveMessages(participant.contextConversationId)
                    .singleOrNull { it.id == entry.childAssistantMessageId }
                val running = receipt.state in setOf("accepted", "selected", "executing") || (session != null && !session.hasCommittedTerminalSeal)
                ongoing = ongoing || running
                val live = session?.state?.value
                ConversationExpertReply(participant, entry.childSubmissionId, receipt.state, canonical?.text.orEmpty(), message = canonical,
                    hostPermission = live?.permissionRequest, localPermission = live?.agentPermissionRequest,
                    thinking = live?.thinking ?: canonical?.let { com.nola.domain.model.decodeAgentThinkingFromMeta(it.meta) }.orEmpty(),
                    thinkingFinished = !running, isStreaming = running, permissionAction = live?.agentPermissionAction)
            }
            rows[ExpertParentMessageKey(parent, message)] = replies
        }
        require(resolveBoundHost(binding) == configuration)
        if (current()) conversationExpertReplies.update { old -> old.filterKeys { it.conversationId != parent } + rows }
        ongoing
    }

    private fun publishConversationExpertReply(parentId: String, participant: com.nola.data.repository.RoomParticipant,
        submission: com.nola.data.repository.RoomSubmission, text: String,
        hostPermission: HostAgentPermissionRequest? = null, localPermission: AndroidAgentPermissionRequest? = null,
        ownerState: ChatUiState? = null, terminalMessage: ConversationMessage? = null) {
        require(submission.contextConversationId == participant.contextConversationId)
        require(ownerState == null || ownerState.conversationId == participant.contextConversationId)
        require(submission.input.parentMessageId == parentId)
        val parentKey = ExpertParentMessageKey(requireNotNull(submission.input.conversationId), parentId)
        val exactAssistantId = submission.executionId?.let { "$it:assistant" }
        require(terminalMessage == null || (submission.state in setOf("completed", "completed_with_warnings", "blocked", "incomplete", "cancelled", "failed") &&
            terminalMessage.id == exactAssistantId && terminalMessage.conversationId == participant.contextConversationId &&
            terminalMessage.role == ChatRole.assistant))
        val ownerMessage = terminalMessage ?: submission.executionId?.let { ownerState?.exactExpertAssistant(participant.contextConversationId, it) }
        val active = exactAssistantId != null && ownerState?.activeResponseId == exactAssistantId && ownerState?.activeTurnId == submission.executionId
        conversationExpertReplies.update { current ->
            val previous = current[parentKey].orEmpty().singleOrNull {
                it.submissionId == submission.input.submissionId && it.participant.contextConversationId == participant.contextConversationId
            }
            val message = ownerMessage ?: previous?.message?.takeIf { it.id == exactAssistantId && it.conversationId == participant.contextConversationId && it.role == ChatRole.assistant }
            val running = submission.state in setOf("executing", "running")
            val reply = ConversationExpertReply(participant, submission.input.submissionId, submission.state,
                message?.text ?: text.takeIf(String::isNotEmpty) ?: previous?.text.orEmpty(),
                if (ownerState != null) hostPermission.takeIf { active } else previous?.hostPermission.takeIf { running },
                if (ownerState != null) localPermission.takeIf { active } else previous?.localPermission.takeIf { running },
                message = message,
                thinking = if (active) ownerState?.thinking.orEmpty() else message?.let { com.nola.domain.model.decodeAgentThinkingFromMeta(it.meta) }.orEmpty(),
                thinkingFinished = !active || ownerState?.thinkingFinished != false,
                isStreaming = running && active && ownerState?.processingState?.isGenerating == true,
                toolEvents = if (active) ownerState?.toolEvents.orEmpty() else emptyList(),
                permissionAction = if (active) ownerState?.agentPermissionAction else null)
            current + (parentKey to (current[parentKey].orEmpty().filterNot { it.submissionId == reply.submissionId } + reply))
        }
    }
    internal val agentRooms = com.nola.data.repository.AgentRoomRepository(applicationRuntime)
    private fun taskParentRun(task: com.nola.core.AgentCoreTaskHandle, messageId: String): String {
        val parent = checkNotNull(_uiState.value.conversationId)
        var cursor: Long? = null
        var identity: JsonObject? = null
        do {
            val window = applicationRuntime.loadConversationProjectionWindowV1(parent, cursor, 64, 256 * 1024)
            identity = window.assistantJournalHeads.map { it.getValue("identity").jsonObject }
                .singleOrNull { it.getValue("assistant_message_id").jsonPrimitive.content == messageId }
            if (identity != null || !window.hasMore) break
            val next = checkNotNull(window.beforeOrdinal); require(next != cursor); cursor = next
        } while (true)
        val owner = checkNotNull(identity) { "The task's parent journal owner is unavailable" }
        require(owner.getValue("conversation_id").jsonPrimitive.content == parent)
        // Journal identity carries Conversation/Run, not a Core session ID.
        // The handle comes from this conversation's validated Core projection;
        // the Core child-detail/recovery command checks the exact task binding.
        require(_uiState.value.coreTaskProjection.sessionId == task.ownerSessionId)
        return owner.getValue("run_id").jsonPrimitive.content
    }

    internal suspend fun loadTaskChildDetail(task: com.nola.core.AgentCoreTaskHandle, messageId: String, before: Long?): com.nola.domain.projection.ConversationProjectionWindowV1 = withContext(Dispatchers.IO) {
        val parent = checkNotNull(_uiState.value.conversationId)
        val child = checkNotNull(task.lineage.childSessionId)
        val run = taskParentRun(task, messageId)
        val binding = checkNotNull(conversationRepo.getConversation(parent)).executionHostBinding
        val transport = binding?.let { webAgentProvider.journalTransport(resolveBoundHost(it)) }
        val callId = task.lineage.parentCallId
        val message = _uiState.value.messages.singleOrNull { (it.meta?.get("nola.canonicalMessageID") ?: it.id) == messageId }
        val origin = message?.canonicalParts?.filterIsInstance<ConversationProjectionV1.Part.Tool>()
            ?.singleOrNull { it.toolCallId == callId }
        val expertCallId = callId?.takeIf { origin?.capabilityId in setOf("delegate_expert", "expert_exchange") }
        com.nola.data.repository.readTaskChildDetail(parent, run, task.id, child, before, transport != null,
            expertCallId = expertCallId, expertQuery = { command -> expertHistoryCommand(parent, command) }) { command ->
            if (transport != null) transport(command).getValue("result") else applicationRuntime.execute(command)
        }
    }

    internal suspend fun taskRecovery(task: com.nola.core.AgentCoreTaskHandle, messageId: String,
        decision: com.nola.data.local.UnknownEffectRecoveryDecision?, submit: Boolean): Pair<UnknownEffectRecoveryPresentation?, Boolean> = withContext(Dispatchers.IO) {
        val parent = checkNotNull(_uiState.value.conversationId)
        require(conversationRepo.getConversation(parent)?.executionHostBinding == null) {
            context.getString(com.nola.R.string.task_recovery_source_host)
        }
        val owner = buildJsonObject {
            put("parent_conversation_id", parent); put("parent_run_id", taskParentRun(task, messageId)); put("task_id", task.id)
        }
        val child = checkNotNull(task.lineage.childSessionId)
        val raw = applicationRuntime.executeQuery(buildJsonObject { put("type", "inspect_owned_task_recovery"); put("owner", owner) })
        val inspection = raw.takeUnless { it is JsonNull }?.let {
            Json.decodeFromJsonElement<com.nola.data.local.UnknownEffectRecoveryInspection>(it).also { value ->
                require(value.conversationId == child)
                value.validate(child, value.runId, value.assistantMessageId)
            }
        }
        val pending = applicationRuntime.pendingOwnedTaskDecision(owner)
        if (submit) {
            val reply = if (pending != null) {
                require(decision == null) { "Recover the original decision first" }
                applicationRuntime.recoverPendingUnknownEffectDecision(pending)
            } else {
                requireNotNull(decision); requireNotNull(inspection)
                require(decision.choice.kind == com.nola.data.local.UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked &&
                    decision.conversationId == child && decision.runId == inspection.runId &&
                    decision.invocationConfigDigest == inspection.invocationConfigDigest && decision.targets == inspection.targets)
                decision.validate()
                val result = applicationRuntime.submitMutation(buildJsonObject {
                    put("type", "resolve_owned_task_recovery"); put("owner", owner); put("input", Json.encodeToJsonElement(com.nola.data.local.UnknownEffectRecoveryDecision.serializer(), decision))
                }, decision.resolutionId)
                Json.decodeFromJsonElement<com.nola.data.local.UnknownEffectRecoveryReply>(result).also { it.validate(decision) }
            }
            require(reply.outcome == com.nola.data.local.UnknownEffectRecoveryReply.Outcome.UnknownPreservedBlocked)
            null to true
        } else {
            if (inspection == null && pending == null) null to false else
                UnknownEffectRecoveryPresentation(child, inspection?.runId ?: checkNotNull(pending).decision.runId,
                    0, null, inspection, pending) to false
        }
    }

    private val remoteGoalMutex = kotlinx.coroutines.sync.Mutex()
    internal suspend fun pendingGoalControl(owner: String): Boolean = withContext(Dispatchers.IO) {
        val host = resolveConversationProgressHost(owner) ?: return@withContext false
        context.getSharedPreferences("goal-control-intents", android.content.Context.MODE_PRIVATE)
            .contains("${host.expectedHostId}:$owner")
    }
    internal suspend fun discardPendingGoalControl(owner: String) = withContext(Dispatchers.IO) {
        val host = resolveConversationProgressHost(owner) ?: return@withContext
        val preferences = context.getSharedPreferences("goal-control-intents", android.content.Context.MODE_PRIVATE)
        check(preferences.edit().remove("${host.expectedHostId}:$owner").commit()) {
            context.getString(R.string.goal_operation_failed)
        }
    }

    internal suspend fun submitGoalControl(owner: String, control: JsonObject?): com.nola.data.repository.ConversationProgress =
        withContext(Dispatchers.IO) { remoteGoalMutex.withLock {
            val host = resolveConversationProgressHost(owner)
            if (host == null) {
                val result = stepLocalGoal(owner, control)
                if (result["interrupted_run_id"]?.takeUnless { it is JsonNull } != null) {
                    withContext(Dispatchers.Main) {
                        runRegistry.find(composerDrafts.ownerKey(owner))?.runtime?.deviceCoordinator()?.requestCancellation("user_requested")
                    }
                }
                withContext(Dispatchers.Main) { wakeLocalGoal(owner, result) }
                return@withLock conversationProgress.load(owner)
            }
            val preferences = context.getSharedPreferences("goal-control-intents", android.content.Context.MODE_PRIVATE)
            val key = "${host.expectedHostId}:$owner"
            val request = control?.let { buildJsonObject {
                put("expected_host_id", host.expectedHostId); put("conversation_id", owner); put("control", it)
            } }
            com.nola.data.local.submitRecoverableGoalControl(request, host.expectedHostId, owner,
                read = { preferences.getString(key, null) },
                write = { value -> check(preferences.edit().apply {
                    if (value == null) remove(key) else putString(key, value)
                }.commit()) { "Could not persist Goal operation" } },
                observe = { webAgentProvider.conversationProgress(owner, host) },
                transport = { intent ->
                    try {
                        webAgentProvider.goalControl(intent, host)
                    } catch (failure: com.nola.data.remote.api.WebHostStatusException) {
                        if (failure.statusCode == 409 && failure.errorCode == "goal_host_default_unavailable")
                            throw IllegalStateException(context.getString(R.string.goal_host_default_unavailable), failure)
                        throw failure
                    }
                })
        } }

    private val localGoalWakes = mutableSetOf<String>()
    private val activeGoalReplies = mutableSetOf<String>()

    private suspend fun stepLocalGoal(
        owner: String,
        control: JsonObject? = null,
        hostAvailable: Boolean = true,
        userPrompt: String = "",
    ): JsonObject = withContext(Dispatchers.IO) {
        check(resolveConversationProgressHost(owner) == null) { "Goal belongs to a paired Host" }
        val plan = withContext(Dispatchers.Main) { localToolPlan(owner,
            runRegistry.find(composerDrafts.ownerKey(owner))?.state?.value ?: _uiState.value) }
        val action = control?.get("action")?.jsonPrimitive?.content
        val state = withContext(Dispatchers.Main) { runRegistry.find(composerDrafts.ownerKey(owner))?.state?.value ?: _uiState.value }
        val needsModel = action in setOf("start", "resume") || userPrompt.isNotBlank()
        val admission = if (needsModel) {
            val resolvedModel = state.currentModel
            if (resolvedModel != null) {
                agentTurnService.goalModelAdmission(owner, resolvedModel, state.effectiveGenerationParameters,
                    state.currentParameters.enableThinking, state.responseStyle, action == "start",
                    state.agentMode, plan.permissionMode)
            } else {
                check(action != "start") { context.getString(R.string.chat_no_model_selected) }
                null
            }
        } else null
        val operation = control?.getValue("operationId")?.jsonPrimitive?.content
        val intent = control?.let { value -> buildJsonObject {
                put("conversation_id", owner); put("operation_id", value.getValue("operationId")); put("action", value.getValue("action"))
                put("expected_goal_id", value["expectedGoalId"] ?: JsonNull)
                put("expected_control_token", value["expectedControlToken"] ?: JsonNull)
                put("objective", value["objective"] ?: JsonPrimitive("")); put("constraints", value["constraints"] ?: JsonArray(emptyList()))
                put("wake_at_epoch_ms", value["wakeAtEpochMs"] ?: JsonNull)
            } }
        val envelope = intent?.let { applicationRuntime.submitMutation(buildJsonObject {
            put("type", "prepare_goal_control_intent"); put("input", it)
        }, operation?.let { "goal-intent:$owner:$it" }) }
        val command = buildJsonObject {
            put("type", "step_goal"); put("conversation_id", owner); put("source_host_id", plan.identity.hostId); put("host_available",hostAvailable)
            put("control", envelope ?: JsonNull); put("selected_model", admission?.selection ?: JsonNull); put("user_prompt", userPrompt)
            admission?.startConfiguration?.let { put("start_configuration", it) }
        }
        val result = applicationRuntime.submitMutation(command, operation?.let { "embedded-goal:$owner:$it" }).jsonObject
        result["error"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content?.let { error(it) }
        result
    }

    private fun wakeLocalGoal(owner: String, initial: JsonObject? = null) {
        if (!remoteUiForeground || owner in localGoalWakes) return
        val session = runRegistry.find(composerDrafts.ownerKey(owner))
        if (session?.runtime?.hasOutstandingGeneration() == true) return
        localGoalWakes.add(owner)
        viewModelScope.launch {
            try {
                var result = initial ?: stepLocalGoal(owner)
                while (remoteUiForeground && result["instruction"]?.jsonObject?.get("type") == JsonPrimitive("wait")) {
                    val instruction = result.getValue("instruction").jsonObject
                    val wake = instruction["wake_at_epoch_ms"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.longOrNull ?: return@launch
                    kotlinx.coroutines.delay((wake - System.currentTimeMillis()).coerceAtLeast(250L))
                    if (!remoteUiForeground) return@launch
                    result = stepLocalGoal(owner)
                }
                val instruction = result.getValue("instruction").jsonObject
                if (instruction["type"] !in listOf(JsonPrimitive("execute_run"),JsonPrimitive("needs_user"))) return@launch
                val run = instruction["run_id"]?.takeUnless {it is JsonNull}?.jsonPrimitive?.content ?: return@launch
                if (!remoteUiForeground) return@launch
                val conversation = withContext(Dispatchers.IO) { conversationRepo.getConversation(owner) } ?: return@launch
                // Existing registry/recovery owns the physical model, tools,
                // permission wait, journal projection and terminal seal.
                val targetSession = runRegistry.find(composerDrafts.ownerKey(owner)) ?: runRegistry.session(
                    composerDrafts.ownerKey(owner),
                    ChatUiState(
                        conversationId = conversation.id,
                        configurationRevision = conversation.configurationRevision,
                        currentModel = conversation.selectedModelInfo,
                        executionHostBinding = conversation.executionHostBinding,
                        permissionMode = conversationPermissionMode(conversation.permissionMode, defaultPermissionMode),
                        responseStyleOverride = conversation.promptResponseStyle ?: _uiState.value.responseStyleOverride,
                    ),
                )
                runRegistry.enter(targetSession) { resumeExistingDeviceRun(conversation, run) }
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (failure: Exception) {
                runRegistry.find(composerDrafts.ownerKey(owner))?.state?.update { it.copy(error = failure.message) }
                if (displayedState.value.conversationId == owner) displayedState.update { it.copy(error = failure.message) }
            } finally { localGoalWakes.remove(owner) }
        }
    }

    private fun pauseLocalGoalsForLifecycle() {
        // Each retained owner is independent of the displayed conversation.
        for (owner in conversationActivity.value.keys) {
            val session = runRegistry.find(composerDrafts.ownerKey(owner)) ?: continue
            if (!session.runtime.hasOutstandingGeneration()) continue
            viewModelScope.launch {
                try {
                    val result = stepLocalGoal(owner, hostAvailable = false)
                    if (result["interrupted_run_id"]?.takeUnless { it is JsonNull } != null)
                        session.runtime.deviceCoordinator()?.requestCancellation("user_requested")
                } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) { Log.w("ChatViewModel", "Goal lifecycle pause remains unconfirmed") }
            }
        }
    }

    internal val conversationProgress = com.nola.data.repository.ConversationProgressRepository(applicationRuntime)
    internal suspend fun resolveConversationProgressHost(owner: String): WebAgentProvider.Config? = withContext(Dispatchers.IO) {
        require(owner.isNotBlank())
        val conversation = checkNotNull(conversationRepo.getConversation(owner)) { "Conversation is unavailable" }
        val binding = conversation.executionHostBinding
        if (binding != null) resolveBoundHost(binding)
        else {
            check(conversation.selectedModelInfo?.source != ModelInfo.ModelSource.WebHost) { "Remote conversation has no bound Host" }
            null
        }
    }

    internal suspend fun loadConversationProgress(owner: String): com.nola.data.repository.ConversationProgress = withContext(Dispatchers.IO) {
        val host = resolveConversationProgressHost(owner)
        if (host == null) return@withContext conversationProgress.load(owner)
        val progress = webAgentProvider.conversationProgress(owner, host)
        remoteGoalMutex.withLock {
            val preferences = context.getSharedPreferences("goal-control-intents", android.content.Context.MODE_PRIVATE)
            val key = "${host.expectedHostId}:$owner"
            preferences.getString(key, null)?.let { raw ->
                com.nola.data.local.settleRemoteGoalReceipt(Json.parseToJsonElement(raw).jsonObject, host.expectedHostId, progress) {
                    check(preferences.edit().remove(key).commit()) { context.getString(R.string.goal_operation_failed) }
                }
            }
        }
        progress
    }

    internal val roomDrafts = androidx.compose.runtime.mutableStateMapOf<String, RoomComposerState>()
    internal val roomSubmissions = androidx.compose.runtime.mutableStateMapOf<String, List<com.nola.data.repository.RoomSubmission>>()
    private val activeRoomDispatches = mutableSetOf<String>()

    internal fun submitRoomDraft(room: com.nola.data.repository.AgentRoomSnapshot) {
        val id = room.room.id
        val draft = roomDrafts[id] ?: return
        if (draft.text.isBlank() || draft.pending != null) return
        val input = com.nola.data.repository.RoomSubmissionInput(id, java.util.UUID.randomUUID().toString(),
            room.revision, draft.text, draft.participantId, System.currentTimeMillis())
        roomDrafts[id] = draft.copy(pending = input, failed = false)
        viewModelScope.launch {
            try {
                val accepted = withContext(Dispatchers.IO) { agentRooms.submit(input) }
                val current = roomDrafts[id] ?: return@launch
                if (current.pending?.submissionId != input.submissionId) return@launch
                roomDrafts[id] = current.copy(text = if (current.text == input.text) "" else current.text, pending = null)
                roomSubmissions[id] = mergeRoomSubmissions(id, roomSubmissions[id].orEmpty(), listOf(accepted))
                roomSubmissions[id]?.firstOrNull { it.input.submissionId == accepted.input.submissionId }?.let {
                    dispatchRoomSubmission(room, it)
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (_: Exception) {
                roomDrafts[id]?.takeIf { it.pending?.submissionId == input.submissionId }?.let {
                    roomDrafts[id] = it.copy(failed = true)
                }
            }
        }
    }

    private suspend fun awaitExpertRunSession(contextId: String): ChatRunSession = kotlinx.coroutines.withTimeout(30_000) {
        while (true) {
            val loaded = runRegistry.find(composerDrafts.ownerKey(contextId))
            if (loaded != null && !loaded.loadingPresentation && loaded.state.value.conversationId == contextId) return@withTimeout loaded
            kotlinx.coroutines.delay(25)
        }
        @Suppress("UNREACHABLE_CODE") error("context unavailable")
    }

    private suspend fun dispatchRoomSubmission(room: com.nola.data.repository.AgentRoomSnapshot,
        accepted: com.nola.data.repository.RoomSubmission) {
        if (!activeRoomDispatches.add(accepted.input.submissionId)) return
        var current = accepted
        var permissionMirror: Job? = null
        try {
            if (current.state == "accepted") {
                val ownerHostId = current.input.conversationId?.let { id ->
                    withContext(Dispatchers.IO) {
                        conversationRepo.getConversation(id)?.executionHostBinding?.let { binding ->
                            runCatching { resolveBoundHost(binding).expectedHostId }.getOrNull()
                        }
                    }
                }
                current = roomCoordinator.select(room, current, remoteHostId = ownerHostId)
            }
            if (current.state !in setOf("selected", "executing")) return
            val contextId = requireNotNull(current.contextConversationId)
            val existingRunId = current.executionId ?: current.input.submissionId
            if (exactExpertRunExists(contextId, existingRunId)) {
                current = withContext(Dispatchers.IO) { checkNotNull(agentRooms.submission(room.room.id, current.input.submissionId)) }
                check(current.contextConversationId == contextId && current.executionId == existingRunId)
                loadConversation(contextId, background = true)
            } else {
            val preparedResend = withContext(Dispatchers.IO) { applicationRuntime.loadPreparedEditedResend(contextId) }
            if (preparedResend != null && preparedResend.admittedRunId == null) {
                preparedResend.requireIdentity()
                check(preparedResend.runId == current.input.submissionId)
                val frozenModel = agentTurnService.captureEditedResendModel(preparedResend.source.model)
                withContext(Dispatchers.IO) {
                    if (agentRooms.executionIntent(current) == null) {
                        val target = checkNotNull(conversationRepo.getConversation(contextId))
                        conversationRepo.selectModel(contextId, frozenModel.model, frozenModel.parameters, target.configurationRevision)
                    }
                }
            }
            loadConversation(contextId, background = true)
            val session = awaitExpertRunSession(contextId)
            check(!session.state.value.processingState.blocksInput)
            check(!session.runtime.hasOutstandingGeneration())
            val snapshot = session.state.value
            val parent = withContext(Dispatchers.IO) { conversationRepo.getActiveMessages(contextId).lastOrNull()?.id }
            val manifestEntry = current.input.parentMessageId?.let { parentId -> withContext(Dispatchers.IO) {
                val manifest = checkNotNull(applicationRuntime.expertMessageSource(checkNotNull(current.input.conversationId), parentId)) {
                    "The complete expert source manifest is unavailable"
                }
                manifest.entries.single { it.childSubmissionId == current.input.submissionId }.also {
                    check(it.participant.contextConversationId == contextId &&
                        (preparedResend != null || it.contextConfigurationRevision == snapshot.configurationRevision))
                }
            } }
            val capturedExpertModel = if (manifestEntry != null && preparedResend == null) {
                val profileId = manifestEntry.modelDeclaration.getValue("profile_id").jsonPrimitive.content
                val revision = manifestEntry.modelDeclaration.getValue("profile_revision").jsonPrimitive.longOrNull
                val candidate = modelRepo.listConfiguredModels().singleOrNull { it.profileId == profileId && it.profileRevision == revision }
                    ?: error("The original expert model declaration is no longer available")
                val parameters = com.nola.data.repository.readExpertConversationParameters(
                    manifestEntry.defaults,
                    candidate,
                    manifestEntry.modelDeclaration.getValue("parameters").jsonObject,
                )
                agentTurnService.captureModel(candidate, parameters)
            } else null
            // Inherited expert contexts intentionally have no selected model.
            // The complete manifest's resolved declaration is the authority.
            val executionModel = capturedExpertModel?.model ?: checkNotNull(snapshot.currentModel) {
                "The original expert model declaration is unavailable"
            }
            val (executionProfileId, executionProfileRevision) = if (executionModel.source == ModelInfo.ModelSource.Local) {
                val localProfile = withContext(Dispatchers.IO) { applicationRuntime.modelProfile(executionModel.id) }
                (executionModel.profileId ?: localProfile?.id ?: executionModel.id) to
                    (executionModel.profileRevision ?: localProfile?.revision ?: 0L)
            } else {
                requireNotNull(executionModel.profileId) to requireNotNull(executionModel.profileRevision)
            }
            val persisted = withContext(Dispatchers.IO) { agentRooms.executionIntent(current) }
            val intent = persisted ?: buildJsonObject {
                put("execution_id", manifestEntry?.childSubmissionId ?: UUID.randomUUID().toString())
                put("envelope", buildJsonObject {
                    put("kind", "native"); put("platform", "android")
                    put("conversation_configuration_revision", requireNotNull(snapshot.configurationRevision))
                    put("model_profile_id", executionProfileId); put("model_profile_revision", executionProfileRevision)
                    put("expected_parent_message_id", parent?.let(::JsonPrimitive) ?: JsonNull); put("text", current.input.text)
                })
            }
            val envelope = intent.getValue("envelope").jsonObject
            check(envelope["platform"]?.jsonPrimitive?.content == "android"
                && envelope["conversation_configuration_revision"]?.jsonPrimitive?.longOrNull == snapshot.configurationRevision
                && envelope["model_profile_id"]?.jsonPrimitive?.content == executionProfileId
                && envelope["model_profile_revision"]?.jsonPrimitive?.longOrNull == executionProfileRevision
                && envelope["expected_parent_message_id"] == (parent?.let(::JsonPrimitive) ?: JsonNull))
            if (persisted == null) current = withContext(Dispatchers.IO) { agentRooms.prepareExecution(current, intent) }
            val claim = withContext(Dispatchers.IO) { agentRooms.claimExecution(current) }
            current = claim.submission
            if (current.state != "executing") return
            if (claim.dispatchPermitted) {
                runRegistry.enter(session) { sendMessage(current.input.text, submissionIdentity = requireNotNull(current.executionId),
                    canonicalUserMessageId = manifestEntry?.childUserMessageId, editedResend = preparedResend,
                    capturedExpertModel = capturedExpertModel) }
            } else {
                // An existing reservation is observation authority, never a new
                // dispatch grant. Its original owner recovers an admitted Run;
                // without one, reconcile below keeps the outcome unconfirmed.
                restoreExistingRoomExecution(current.executionId, contextId,
                    executionExists = { withContext(Dispatchers.IO) { agentRooms.hasExecution(it) } },
                    loadConversationOwner = { loadConversation(it, background = true) })
            }
            }
            val dispatchedSession = awaitExpertRunSession(contextId)
            val parentId = accepted.input.parentMessageId
            permissionMirror = parentId?.let { anchor -> viewModelScope.launch {
                dispatchedSession.state.collect { ownerState ->
                    val currentSubmission = roomSubmissions[room.room.id].orEmpty()
                        .firstOrNull { it.input.submissionId == accepted.input.submissionId }?.takeIf { it.revision >= current.revision } ?: current
                    publishConversationExpertReply(anchor, room.room.participants.single { it.id == currentSubmission.selectedParticipantId },
                        currentSubmission, "", ownerState.permissionRequest, ownerState.agentPermissionRequest, ownerState)
                }
            } }
            while (true) {
                val observed = withContext(Dispatchers.IO) { agentRooms.observeExecution(current) }
                current = observed
                roomSubmissions[room.room.id] = mergeRoomSubmissions(room.room.id, roomSubmissions[room.room.id].orEmpty(), listOf(current))
                val participant = room.room.participants.singleOrNull { it.id == current.selectedParticipantId }
                val parentId = current.input.parentMessageId
                if (participant != null && parentId != null) {
                    val ownerState = dispatchedSession.state.value
                    publishConversationExpertReply(parentId, participant, current, "", ownerState.permissionRequest, ownerState.agentPermissionRequest, ownerState)
                }
                if (current.state != "executing") {
                    // The receipt can seal before the journal presentation consumes its
                    // last batch. Stop the live writer, then reconcile the exact durable
                    // assistant through the existing canonical projector. Never freeze
                    // the last sampled preview as the completed answer.
                    permissionMirror?.cancel()
                    permissionMirror?.join()
                    if (participant != null && parentId != null && current.executionId != null) {
                        val exact = withContext(Dispatchers.IO) {
                            conversationRepo.getActiveMessages(contextId).singleOrNull {
                                it.id == "${current.executionId}:assistant" && it.conversationId == contextId && it.role == ChatRole.assistant
                            }
                        }
                        if (current.state == "completed") {
                            checkNotNull(exact) { "The exact completed expert assistant is unavailable" }
                        }
                        publishConversationExpertReply(parentId, participant, current, "", terminalMessage = exact)
                    }
                    break
                }
                kotlinx.coroutines.delay(1000)
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (error: Exception) {
            if (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0) {
                val detail = com.nola.util.redactHostToolText(error.message.orEmpty())
                    .replace(Regex("https?://[^\\s]+"), "[endpoint]").take(600)
                val origin = error.stackTrace.firstOrNull { it.className.startsWith("com.nola.") }
                android.util.Log.w("NolaExpertDispatch", "${error.javaClass.simpleName} at $origin: $detail")
            }
            val freshPrepared = current.contextConversationId?.let { withContext(Dispatchers.IO) { applicationRuntime.loadPreparedEditedResend(it) } }
            if (current.state != "executing" && freshPrepared == null) runCatching {
                current = withContext(Dispatchers.IO) { agentRooms.fail(current, if (current.state == "accepted") "routing_failed" else "execution_unavailable") }
            }
        } finally {
            withContext(kotlinx.coroutines.NonCancellable) {
                runCatching { refreshRoomSubmissions(room.room.id, accepted.input.conversationId) }
            }
            permissionMirror?.cancel()
            activeRoomDispatches.remove(accepted.input.submissionId)
        }
    }

    internal suspend fun refreshRoomSubmissions(roomId: String, parentConversationId: String?) {
        val (values, exchanges) = withContext(Dispatchers.IO) {
            val submissions = agentRooms.submissions(roomId).map { value ->
                if (value.state == "executing") runCatching { agentRooms.observeExecution(value) }.getOrDefault(value) else value
            }
            val parent = parentConversationId ?: submissions.mapNotNull { it.input.conversationId }.distinct().singleOrNull()
            val peerList = if (parent != null && agentRooms.forConversation(parent)?.roomId == roomId) {
                agentRooms.peerExchanges(parent).getValue("exchanges").jsonArray.map { it.jsonObject }
            } else emptyList()
            submissions to peerList
        }
        roomSubmissions[roomId] = mergeRoomSubmissions(roomId, roomSubmissions[roomId].orEmpty(), values)
        roomSubmissions[roomId].orEmpty().filter { it.state in setOf("accepted", "selected", "executing") && it.input.submissionId !in activeRoomDispatches }.forEach { value ->
            viewModelScope.launch {
                try {
                    withContext(Dispatchers.IO) { agentRooms.load(roomId) }?.let { dispatchRoomSubmission(it, value) }
                } catch (cancelled: kotlinx.coroutines.CancellationException) {
                    throw cancelled
                } catch (error: Exception) {
                    Log.e("ChatViewModel", "Room submission dispatch failed", error)
                }
            }
        }
        exchanges.mapNotNull { exchange ->
            exchange["target"]?.jsonObject?.get("context_conversation_id")?.jsonPrimitive?.contentOrNull
                ?.takeIf(String::isNotBlank)
        }.distinct().forEach { childContextId ->
            if (runRegistry.find(composerDrafts.ownerKey(childContextId)) == null) {
                loadConversation(childContextId, background = true)
            }
        }
        val current = roomDrafts[roomId] ?: return
        val pending = current.pending ?: return
        if (values.any { it.input.submissionId == pending.submissionId }) {
            roomDrafts[roomId] = current.copy(text = if (current.text == pending.text) "" else current.text, pending = null, failed = false)
        }
    }

    internal fun cancelPendingRoomSubmission(value: com.nola.data.repository.RoomSubmission) {
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) { agentRooms.cancelPending(value) }
                val ownerHostId = value.input.conversationId?.let { id ->
                    withContext(Dispatchers.IO) {
                        conversationRepo.getConversation(id)?.executionHostBinding?.let { binding ->
                            runCatching { resolveBoundHost(binding).expectedHostId }.getOrNull()
                        }
                    }
                }
                runCatching { roomCoordinator.cancel(value, remoteHostId = ownerHostId) }
                refreshRoomSubmissions(value.input.roomId, value.input.conversationId)
            }
        }
    }

    /** This editor's directory is the local Store, never a paired Host mirror. */
    internal data class ExpertEditorScope(val remote: Boolean, val models: List<ModelInfo>, val defaultProfileId: String?, val efforts: Map<String, List<String>>, val sourceIdentity: String = "local", val storeEpoch: String? = null,
        val executors: List<com.nola.data.remote.dto.ExecutionBackendDto> = emptyList())
    internal suspend fun expertEditorScope(): ExpertEditorScope = withContext(Dispatchers.IO) {
        val state = displayedState.value
        val binding = state.conversationId?.let { conversationRepo.getConversation(it)?.executionHostBinding } ?: state.executionHostBinding
        if (binding == null) return@withContext ExpertEditorScope(false, state.availableModels, null, emptyMap())
        val configuration = resolveBoundHost(binding)
        val settings = checkNotNull(webAgentProvider.fetchHostAgentSettings(configuration)) { "The expert Host model catalog is unavailable" }
        require(resolveBoundHost(binding) == configuration)
        val epoch = settings.sourceStoreEpoch?.takeIf { it.isNotBlank() } ?: error("The Host source catalog is unavailable; update or reconnect it")
        val source = state.conversationId?.let { applicationRuntime.conversationSourceOwner(it) }
        if (source is com.nola.data.local.ConversationSourceOwner.Remote) require(source.hostId == configuration.expectedHostId && source.storeEpoch == epoch)
        val models = settings.modelProfiles.map { ModelInfo(it.modelName, it.displayName, ModelInfo.ModelSource.Cloud,
            profileId = it.profileId, profileRevision = it.revision, executionCapabilitiesDeclared = false) }
        val efforts = settings.modelProfiles.associate { it.profileId to it.reasoning?.takeIf { it.control == "effort" }?.efforts.orEmpty()
            .filter { value -> value in listOf("none", "minimal", "low", "medium", "high", "xhigh", "max") }.distinct() }
        ExpertEditorScope(true, models, settings.defaultChatModelResolution?.takeIf { it.kind == "resolved" }?.profileId, efforts,
            JsonArray(listOf(configuration.expectedHostId, configuration.clientId, epoch).map(::JsonPrimitive)).toString(), epoch, settings.executionBackends)
    }

    internal suspend fun readableExpertTemplates(localDirectory: Boolean = false): com.nola.data.local.ReadableExpertTemplateCatalog = withContext(Dispatchers.IO) {
        if (localDirectory) return@withContext applicationRuntime.readableExpertTemplates()
        val state = displayedState.value
        val binding = state.conversationId?.let { conversationRepo.getConversation(it)?.executionHostBinding } ?: state.executionHostBinding
        require(binding == null) { "The connected Host custom-template catalog is unavailable; built-in roles remain available" }
        applicationRuntime.readableExpertTemplates()
    }

    internal suspend fun expertDefaultReasoningEfforts(): List<String> = withContext(Dispatchers.IO) {
        try {
            val profiles = applicationRuntime.listModelProfiles()
            val preference = applicationRuntime.productSetting("chat.default_model")?.valueJson
            val resolved = applicationRuntime.executeQuery(expertDefaultModelResolutionCommand(profiles, preference))
            val profileId = resolvedExpertDefaultProfileId(resolved) ?: return@withContext emptyList()
            modelParamsStore.parameterSnapshot(profileId).reasoningEfforts
        } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (_: Exception) { emptyList() }
    }

    internal suspend fun openAgentRoom(roomId: String) {
        val generation = agentRoomPresentation.beginLoad()
        val room = withContext(Dispatchers.IO) { requireNotNull(agentRooms.load(roomId)) }
        if (!agentRoomPresentation.acceptsLoad(generation)) return
        agentRoomPresentation.open(room)
        for (participant in room.room.participants) {
            val live = runRegistry.find(composerDrafts.ownerKey(participant.contextConversationId))
            if (live != null && live.retainsLoadedPresentation()) {
                agentRoomPresentation.publish(live.state.value)
            } else {
                try {
                    val window = withContext(Dispatchers.IO) {
                        conversationRepo.getProjectionWindow(participant.contextConversationId, beforeOrdinal = null)
                    }
                    if (!agentRoomPresentation.acceptsLoad(generation)) return
                    // Do not replace a run that started while the disk read was suspended.
                    val latest = runRegistry.find(composerDrafts.ownerKey(participant.contextConversationId))
                    agentRoomPresentation.publish(latest?.takeIf { it.retainsLoadedPresentation() }?.state?.value
                        ?: ChatUiState(conversationId = participant.contextConversationId,
                            messages = CanonicalConversationProjector.project(window.projection),
                            historyBeforeOrdinal = window.beforeOrdinal, hasMoreHistory = window.hasMore))
                } catch (cancelled: kotlinx.coroutines.CancellationException) {
                    throw cancelled
                } catch (failure: Exception) {
                    if (!agentRoomPresentation.acceptsLoad(generation)) return
                    agentRoomPresentation.publish(ChatUiState(conversationId = participant.contextConversationId,
                        pendingCommandId = (failure as? com.nola.data.local.AndroidApplicationRuntime.PendingMutation)?.requestId,
                        error = failure.message ?: "Unable to load this member"))
                }
            }
            loadConversation(participant.contextConversationId, background = true)
        }
    }
    val conversationListError = MutableStateFlow<String?>(null)
    val conversationCatalog = MutableStateFlow<List<com.nola.domain.model.Conversation>>(emptyList())
    val conversationPreviews = MutableStateFlow<Map<String, String>>(emptyMap())
    val conversationExperts = MutableStateFlow<Map<String, List<String>>>(emptyMap())
    val conversationActivity = MutableStateFlow<Map<String, ChatUiState>>(emptyMap())
    private suspend fun loadConversationExpertReplies(conversationId: String) {
        if (withContext(Dispatchers.IO) { conversationRepo.getConversation(conversationId)?.executionHostBinding != null }) {
            startRemoteExpertObservation(conversationId); return
        }
        val generation = conversationLoads.currentGeneration()
        fun current() = _uiState.value.conversationId == conversationId && conversationLoads.isCurrent(generation)
        val repliesAtReadStart = conversationExpertReplies.value
        val visibleParents = _uiState.value.takeIf { it.conversationId == conversationId }
            ?.timelineMessages()?.filter { it.role == ChatRole.user }?.map { it.id }?.toSet().orEmpty()
        val visibleKeys = visibleParents.mapTo(mutableSetOf()) { ExpertParentMessageKey(conversationId, it) }
        val association = withContext(Dispatchers.IO) { agentRooms.forConversation(conversationId) } ?: run {
            if (current()) {
                conversationExpertReplies.update { latest -> latest.filter { (key, value) ->
                    key !in visibleKeys || value !== repliesAtReadStart[key]
                } }
                resumableExpertMessages.value = emptySet()
            }
            return
        }
        if (!current()) return
        val participants = association.room.room.participants.associateBy { it.id }
        val submissions = withContext(Dispatchers.IO) {
            visibleParents.flatMap { parent ->
                val manifest = applicationRuntime.expertMessageSource(conversationId, parent) ?: return@flatMap emptyList()
                check(manifest.input.conversationId == conversationId && manifest.input.userMessageId == parent &&
                    manifest.input.roomId == association.roomId && manifest.entries.map { it.participant } == manifest.input.selectedParticipants)
                manifest.entries.map { entry ->
                    checkNotNull(agentRooms.submission(association.roomId, entry.childSubmissionId)).also {
                        check(it.input.parentMessageId == parent && it.input.conversationId == conversationId &&
                            it.contextConversationId == entry.participant.contextConversationId && it.selectedParticipantId == entry.participant.participantId)
                    }
                }
            }
        }
        if (!current()) return
        val resume = mutableSetOf<String>()
        val restored = mutableMapOf<ExpertParentMessageKey, MutableList<ConversationExpertReply>>()
        for (original in submissions) {
            val parent = original.input.parentMessageId ?: continue
            if (parent !in visibleParents || original.input.conversationId?.let { it != conversationId } == true) continue
            val participantId = original.selectedParticipantId ?: original.input.participantId ?: continue
            val participant = participants[participantId] ?: continue
            if (original.selectedParticipantId != null && participant.id != original.selectedParticipantId) continue
            if (original.contextConversationId != participant.contextConversationId) continue
            val current = if (original.state == "executing") runCatching {
                withContext(Dispatchers.IO) { agentRooms.observeExecution(original) }
            }.getOrDefault(original) else original
            if (original.state in setOf("selected", "executing") && !exactExpertRunExists(participant.contextConversationId, original.input.submissionId)) resume += parent
            val response = current.executionId?.let { executionId -> runCatching { withContext(Dispatchers.IO) {
                conversationRepo.getActiveMessages(participant.contextConversationId)
                    .singleOrNull { it.id == "$executionId:assistant" }
            } }.getOrNull() }
            restored.getOrPut(ExpertParentMessageKey(conversationId, parent)) { mutableListOf() } += ConversationExpertReply(participant,
                current.input.submissionId, current.state, response?.text.orEmpty(), message = response,
                thinking = response?.let { com.nola.domain.model.decodeAgentThinkingFromMeta(it.meta) }.orEmpty())
            if (current.state == "executing" && current.executionId != null && exactExpertRunExists(participant.contextConversationId, current.executionId)) {
                viewModelScope.launch { dispatchRoomSubmission(association.room, current) }
            }
        }
        if (current()) {
            conversationExpertReplies.update { latest ->
                if (!current()) latest else latest.filterKeys { it !in visibleKeys } + restored.mapValues { (key, rows) ->
                    rows.map { restoredReply ->
                        val live = latest[key].orEmpty().singleOrNull { it.submissionId == restoredReply.submissionId }
                        val before = repliesAtReadStart[key].orEmpty().singleOrNull { it.submissionId == restoredReply.submissionId }
                        // A suspended disk read cannot overwrite newer exact-child journal presentation.
                        if (live != null && live !== before) live else restoredReply
                    }
                }
            }
            resumableExpertMessages.value = resume
        }
    }

    private suspend fun exactExpertRunExists(owner: String, run: String): Boolean = withContext(Dispatchers.IO) {
        val value = applicationRuntime.executeQuery(buildJsonObject {
            put("type", "load_exact_run_invocation"); put("conversation_id", owner); put("expected_run_id", run); put("assistant_message_id", "$run:assistant")
        })
        if (value == JsonNull) false else {
            val fields = value.jsonObject
            check(fields.getValue("conversation_id").jsonPrimitive.content == owner && fields.getValue("run_id").jsonPrimitive.content == run &&
                fields.getValue("assistant_message_id").jsonPrimitive.content == "$run:assistant")
            true
        }
    }

    fun resumeOriginalExpertMessage(messageId: String) {
        val owner = _uiState.value.conversationId ?: return
        if (messageId !in resumableExpertMessages.value || resumingExpertMessage.value != null) return
        val generation = conversationLoads.currentGeneration()
        fun current() = _uiState.value.conversationId == owner && conversationLoads.isCurrent(generation)
        resumingExpertMessage.value = messageId
        viewModelScope.launch {
            try {
                val original = withContext(Dispatchers.IO) { checkNotNull(applicationRuntime.expertMessageSource(owner, messageId)) }
                check(original.input.conversationId == owner && original.input.userMessageId == messageId)
                if (!current()) return@launch
                val prepared = withContext(Dispatchers.IO) { applicationRuntime.loadPreparedExpertResend(owner) }
                if (prepared != null) {
                    check(prepared.input.targetConversationId == owner && prepared.parentMessageId == messageId &&
                        prepared.entries.map { it.prepared.runId } == original.entries.map { it.childSubmissionId })
                    // The prepared child factory retains the original model,
                    // instructions, IDs and fresh target policy through dispatch.
                    if (current()) dispatchExpertManifest(original)
                } else {
                    if (!current()) return@launch
                    val plan = withContext(Dispatchers.IO) { applicationRuntime.admitExpertMessage(original.input) }
                    check(plan.input == original.input && plan.entries.map { it.childSubmissionId } == original.entries.map { it.childSubmissionId })
                    val room = withContext(Dispatchers.IO) { checkNotNull(agentRooms.load(plan.input.roomId)) }
                    val receipts = withContext(Dispatchers.IO) { plan.entries.map { entry ->
                        checkNotNull(agentRooms.submission(plan.input.roomId, entry.childSubmissionId)).also {
                            check(it.input.parentMessageId == messageId && it.contextConversationId == entry.participant.contextConversationId &&
                                it.selectedParticipantId == entry.participant.participantId)
                        }
                    } }
                    for (receipt in receipts) {
                        if (!current()) return@launch
                        val context = checkNotNull(receipt.contextConversationId)
                        if (receipt.state in setOf("selected", "executing")) viewModelScope.launch { dispatchRoomSubmission(room, receipt) }
                    }
                }
                if (current()) loadConversationExpertReplies(owner)
            } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
            catch (failure: Exception) { if (current()) _uiState.update { it.copy(error = failure.message ?: "Original expert recovery was rejected") } }
            finally { if (resumingExpertMessage.value == messageId) resumingExpertMessage.value = null }
        }
    }

    fun loadConversationPreview(id: String) {
        viewModelScope.launch {
            runCatching { withContext(Dispatchers.IO) {
                CanonicalConversationProjector.project(applicationRuntime.loadConversationProjectionWindowV1(id, null, 2, 4096).projection)
                    .lastOrNull { it.text.isNotBlank() }?.text.orEmpty().replace("\n", " ")
                    .ifBlank { hostSessionReference(id)?.get("preview")?.jsonPrimitive?.contentOrNull.orEmpty() }
            } }.onSuccess { preview -> conversationPreviews.update { it + (id to preview) } }
            // Read the existing association only; opening a list must never create a room.
            try {
                val names = withContext(Dispatchers.IO) {
                    agentRooms.forConversation(id)?.room?.room?.participants.orEmpty()
                        .filter { it.active }.map { it.displayName }.distinct()
                }
                if (conversationCatalog.value.any { it.id == id }) {
                    conversationExperts.update { it + (id to names) }
                }
            } catch (cancel: kotlinx.coroutines.CancellationException) { throw cancel }
            catch (_: Exception) { /* Unavailable identities are omitted, never inferred. */ }
        }
    }
    private val runRegistry by lazy { ChatRunRegistry(viewModelScope, agentTraceRepository, generationCompleted = { session ->
        val conversationId = session.runtime.conversationOwner
        if (conversationId != null) viewModelScope.launch(Dispatchers.Main) {
            // Leave the completed Run thread-local before inspecting the selected conversation.
            try { wakeIdleQueuedUiInput(conversationId); wakeLocalGoal(conversationId) }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (displayedState.value.conversationId == conversationId) {
                    displayedState.update { it.copy(error = error.message) }
                }
            }
        }
    }, sealedRunObserved = { session ->
        session.runtime.conversationOwner?.let { owner -> viewModelScope.launch(Dispatchers.Main) { wakeLocalGoal(owner) } }
    }) { session, snapshot ->
        agentRoomPresentation.publish(snapshot)
        snapshot.conversationId?.let { id -> conversationActivity.update { it + (id to snapshot) } }
        if (composerDrafts.ownerKey(displayedState.value.conversationId) == session.ownerKey) displayedState.value = snapshot
    } }
    private val executionSession: ChatRunSession get() = runRegistry.current
        ?: runRegistry.session(composerDrafts.ownerKey(displayedState.value.conversationId), displayedState.value)
    private val executionScope get() = runRegistry.current?.let(runRegistry::scope) ?: viewModelScope
    private val _uiState get() = runRegistry.current?.state ?: displayedState
    private fun isExecutionCurrent(generation: Long): Boolean = runRegistry.current?.let { !it.retired }
        ?: conversationLoads.isCurrent(generation)
    val uiState: StateFlow<ChatUiState> = displayedState.asStateFlow()

    internal fun selectRoomRecipient(participantId: String) {
        loadConversation(agentRoomPresentation.select(participantId))
    }

    internal fun inspectRoomParticipant(participantId: String) {
        loadConversation(agentRoomPresentation.inspect(participantId))
    }

    internal suspend fun loadEarlierRoomMessages() {
        val initial = agentRoomPresentation.state.value
        val roomId = initial.room?.room?.id ?: return
        for ((contextId, state) in initial.contexts) {
            if (!state.hasMoreHistory) continue
            val window = try {
                withContext(Dispatchers.IO) {
                    conversationRepo.getProjectionWindow(contextId, beforeOrdinal = state.historyBeforeOrdinal)
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                if (agentRoomPresentation.state.value.room?.room?.id != roomId) return
                agentRoomPresentation.state.value.contexts[contextId]?.let { current ->
                    val failed = current.copy(error = failure.message ?: "Unable to load earlier messages",
                        pendingCommandId = (failure as? com.nola.data.local.AndroidApplicationRuntime.PendingMutation)?.requestId)
                    runRegistry.find(composerDrafts.ownerKey(contextId))?.state?.value = failed
                    if (displayedState.value.conversationId == contextId) displayedState.value = failed
                    agentRoomPresentation.publish(failed)
                }
                continue
            }
            if (agentRoomPresentation.state.value.room?.room?.id != roomId) return
            val current = agentRoomPresentation.state.value.contexts[contextId] ?: continue
            val currentIds = current.messages.map { it.id }.toSet()
            val updated = current.copy(
                messages = CanonicalConversationProjector.project(window.projection).filterNot { it.id in currentIds } + current.messages,
                historyBeforeOrdinal = window.beforeOrdinal, hasMoreHistory = window.hasMore,
            )
            val session = runRegistry.find(composerDrafts.ownerKey(contextId))
            if (session != null) session.state.value = updated
            if (displayedState.value.conversationId == contextId) displayedState.value = updated
            agentRoomPresentation.publish(updated)
        }
    }

    internal fun sendRoomMessage(text: String) {
        agentRoomPresentation.requireRecipient(displayedState.value.conversationId)
        sendMessage(text)
    }
    private var defaultPermissionMode = com.nola.core.AndroidAgentPermissionMode.Cautious

    private val conversationLoads = ConversationLoadCoordinator()
    private val turnRuntime get() = executionSession.runtime
    private val permissionSession get() = executionSession.permission
    private val remoteTaskObservation get() = executionSession.taskObservation
    private val remoteConversationObserver get() = executionSession.observer
    private var remoteObservationConfiguration: WebAgentProvider.Config?
        get() = executionSession.configuration
        set(value) { executionSession.configuration = value }
    private var remoteObservationInitialKey: com.nola.data.remote.api.RemoteRunKey?
        get() = executionSession.initialKey
        set(value) { executionSession.initialKey = value }
    private var remoteObservationVisibleHeads: List<JsonObject>
        get() = executionSession.visibleHeads
        set(value) { executionSession.visibleHeads = value }
    private var remoteSubmissionIntent: com.nola.data.remote.api.RemoteSubmissionIntent?
        get() = executionSession.submission
        set(value) { executionSession.submission = value }
    private var remoteSubmissionAdmitted: Boolean
        get() = executionSession.admitted
        set(value) { executionSession.admitted = value }
    private var remoteAdmissionReceipt: com.nola.data.remote.api.RemoteSubmissionReceipt?
        get() = executionSession.receipt
        set(value) { executionSession.receipt = value }
    private val traceCoordinator get() = executionSession.trace
    private val agentPermissionBroker = agentTurnService.permissionBroker
    private val agentHostExecutor = agentTurnService.hostExecutor
    private val capabilityActivationBuffer get() = executionSession.capabilities
    private var remoteJournalTurn: com.nola.data.local.AndroidJournalTurn?
        get() = executionSession.journalTurn
        set(value) { executionSession.journalTurn = value }
    private var remoteJournalTransport: (suspend (JsonObject) -> JsonObject)?
        get() = executionSession.journalTransport
        set(value) { executionSession.journalTransport = value }
    private var remoteJournalProjection: com.nola.domain.projection.ConversationProjectionV1?
        get() = executionSession.journalProjection
        set(value) { executionSession.journalProjection = value }
    private var healthConnectionState: UnifiedDataSourceConnectionState =
        UnifiedDataSourceConnectionState.NeedsPermission

    internal val composerDrafts = ConversationComposerDrafts(ConversationDraftRepository(
        java.io.File(context.noBackupFilesDir, "conversation-drafts"), "${context.packageName}.fileprovider"))
    private var restoredPendingConfiguration: ConversationDraftRepository.Configuration? = null
    private var modelCatalogGeneration = 0L

    private val displayedChildPermissionIds = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()

    init {
        viewModelScope.launch {
            memoryProfileDataStore.skillCatalog
                .catch { e -> Log.w("ChatViewModel", "skillCatalog collector failed", e) }
                .collect { skills ->
                    _uiState.update { it.copy(uploadedSkills = skills) }
                }
        }
        viewModelScope.launch {
            memoryProfileDataStore.profile
                .catch { e -> Log.w("ChatViewModel", "profile collector failed", e) }
                .collect { profile ->
                    val previousSnapshot = memoryEditorSnapshot
                    val previousLongTerm = previousSnapshot?.longTermMemory?.joinToString("\n")
                    val previousUserPrefs = previousSnapshot?.userPreferences?.joinToString("\n")
                    memoryEditorSnapshot = profile
                    _uiState.update { state ->
                        val longTermDirty = previousLongTerm != null && state.longTermMemory != previousLongTerm
                        val userPrefsDirty = previousUserPrefs != null && state.userPreferences != previousUserPrefs
                        state.copy(
                            longTermMemory = if (longTermDirty) state.longTermMemory else profile.longTermMemory.joinToString("\n"),
                            userPreferences = if (userPrefsDirty) state.userPreferences else profile.userPreferences.joinToString("\n"),
                            uploadedSkills = profile.uploadedSkills,
                            uploadedDocuments = profile.uploadedDocuments,
                            mcpServers = profile.mcpServers,
                            customDataSources = profile.customDataSources,
                            dataSources = buildDataSources(
                                uploadedDocuments = profile.uploadedDocuments,
                                hostUploadedDocuments = state.hostUploadedDocuments,
                                mcpServers = profile.mcpServers,
                                hostMcpServers = state.hostMcpServers,
                                customDataSources = profile.customDataSources,
                            ),
                        )
                    }
                }
        }
        viewModelScope.launch {
            var displayedChildRequest: String? = null
            combine(AndroidChildPermissions.pending, _uiState) { pending, state ->
                pending.firstOrNull { state.taskChildOwner == null &&
                    it.parentConversationId == state.conversationId }?.request
            }.catch { e -> Log.w("ChatViewModel", "child permissions collector failed", e) }
                .collect { request ->
                request?.let { displayedChildPermissionIds.add(it.requestId) }
                _uiState.update { state ->
                    if (request != null && (state.agentPermissionRequest == null ||
                            state.agentPermissionRequest?.requestId == displayedChildRequest)) {
                        state.copy(agentPermissionRequest = request,
                            agentPermissionAction = state.agentPermissionAction.takeIf {
                                state.agentPermissionRequest?.requestId == request.requestId
                            })
                    } else if (request == null && state.agentPermissionRequest?.requestId == displayedChildRequest) {
                        state.copy(agentPermissionRequest = null, agentPermissionAction = null)
                    } else state
                }
                displayedChildRequest = request?.requestId
            }
        }

        viewModelScope.launch {
            runCatching { composerDrafts.initialize() }
                .onFailure {
                    if (it is kotlinx.coroutines.CancellationException) throw it
                    Log.w("ChatViewModel", "composerDrafts initialize failed", it)
                }
            val restored = runCatching { composerDrafts.pendingConfiguration() }.getOrNull()
            restoredPendingConfiguration = restored
            if (_uiState.value.conversationId == null && restored != null) _uiState.update { it.copy(currentModel = restored.model,
                currentParameters = restored.parameters, contextMaxTokens = restored.parameters.maxContextTokens,
                executionHostBinding = restored.hostBinding) }
            runCatching { loadConversations() }.onFailure {
                if (it is kotlinx.coroutines.CancellationException) throw it
                Log.w("ChatViewModel", "loadConversations failed", it)
            }
            runCatching { loadAvailableModels() }.onFailure {
                if (it is kotlinx.coroutines.CancellationException) throw it
                Log.w("ChatViewModel", "loadAvailableModels failed", it)
            }
            runCatching { loadPreferences() }.onFailure {
                if (it is kotlinx.coroutines.CancellationException) throw it
                Log.w("ChatViewModel", "loadPreferences failed", it)
            }
            runCatching { loadLastConversation() }.onFailure {
                if (it is kotlinx.coroutines.CancellationException) throw it
                Log.w("ChatViewModel", "loadLastConversation failed", it)
            }
        }
    }

    private var memoryEditorSnapshot: com.nola.domain.model.ChatMemoryProfile? = null
    private var documentEditorSnapshot: com.nola.data.local.datastore.MemoryProfileDataStore.DocumentEditorSnapshot? = null

    private fun loadPreferences() {
        viewModelScope.launch {
            prefs.hostDeviceGrants
                .catch { e -> Log.w("ChatViewModel", "hostDeviceGrants collector failed", e) }
                .collect { grants ->
                    _uiState.update { it.copy(pairedHostIdentity = grants.identity,
                        hybridDeviceToolsEnabled = grants.enabled, hybridDeviceCapabilities = grants.capabilities) }
                }
        }
        viewModelScope.launch {
            runCatching {
                val agentMode = prefs.isAgentModeEnabled.first()
                val permissionMode = prefs.agentPermissionMode.first()
                defaultPermissionMode = permissionMode
                val autoCompact = memoryProfileDataStore.autoCompactTriggerPercent.first()
                val documents = memoryProfileDataStore.documentEditor()
                val profile = chatRepo.getMemoryProfile().copy(uploadedDocuments = documents.documents)
                documentEditorSnapshot = documents
                memoryEditorSnapshot = profile
                healthConnectionState = readHealthConnectionState()
                val dataSources = buildDataSources(
                    uploadedDocuments = profile.uploadedDocuments,
                    mcpServers = profile.mcpServers,
                    customDataSources = profile.customDataSources
                )
                _uiState.update { it.copy(
                    agentMode = agentMode,
                    permissionMode = if (it.conversationId == null) permissionMode else it.permissionMode,
                    autoCompactTriggerPercent = autoCompact,
                    longTermMemory = profile.longTermMemory.joinToString("\n"),
                    userPreferences = profile.userPreferences.joinToString("\n"),
                    uploadedSkills = profile.uploadedSkills,
                    uploadedDocuments = profile.uploadedDocuments,
                    mcpServers = profile.mcpServers,
                    dataSources = dataSources,
                    customDataSources = profile.customDataSources
                ) }
            }.onFailure {
                if (it is kotlinx.coroutines.CancellationException) throw it
                Log.w("ChatViewModel", "loadPreferences failed", it)
            }
        }
    }

    fun loadAvailableModels() {
        val request = ++modelCatalogGeneration
        val owner = composerDrafts.captureRead(_uiState.value.conversationId, modelSelectionGeneration)
        viewModelScope.launch {
            val configured = modelRepo.listConfiguredModels()
            val models = mutableListOf<ModelInfo>()
            models.addAll(configured)

            // Configured options come from canonical model profiles above; the
            // endpoint's comma-separated display value is not a model identity.
            models.addAll(deviceModelStore.installedModels().map { record ->
                ModelInfo(id = record.id, name = record.displayName, source = ModelInfo.ModelSource.Local)
            })
            val host = prefs.webHostConfiguration()
            if (host != null) {
                models.add(ModelInfo("web-host", "Nola Web Host", ModelInfo.ModelSource.WebHost))
            }

            val activeModel = models.firstOrNull { model ->
                model.id == modelService.currentModelId &&
                    model.profileId == modelService.currentProfileId &&
                    model.source == modelService.currentModelSource.toModelSource()
            } ?: models.firstOrNull {
                it.source == ModelInfo.ModelSource.Local && modelService.currentProfileId == null &&
                    it.profileId == null && it.id == modelService.currentModelId
            } ?: prefs.defaultModelSelection(models)
            val activeParameters = activeModel?.let { modelParamsStore.loadParameters(it.parameterProfileId) }
            val defaultHostBinding = if (activeModel?.source == ModelInfo.ModelSource.WebHost && host != null)
                com.nola.domain.model.ExecutionHostBinding(host.address, host.pin.takeIf { it.isNotBlank() }) else null
            if (request != modelCatalogGeneration) return@launch
            // The directory is application-scoped; only selection/defaults belong
            // to the captured conversation or pending draft.
            _uiState.update { it.copy(availableModels = models) }
            if (!composerDrafts.ownsRead(owner, _uiState.value.conversationId, modelSelectionGeneration)) return@launch

            _uiState.update {
                it.copy(
                    currentModel = if (it.conversationId == null) it.currentModel ?: activeModel else it.currentModel,
                    currentParameters = if (it.conversationId == null && it.currentModel == null) activeParameters ?: it.currentParameters else it.currentParameters,
                    executionHostBinding = if (it.conversationId == null && it.currentModel == null) defaultHostBinding else it.executionHostBinding,
                )
            }
            val pendingState = _uiState.value
            if (pendingState.conversationId == null && composerDrafts.pendingConfiguration() == null) {
                composerDrafts.savePendingConfiguration(owner.key,
                    ConversationDraftRepository.Configuration(pendingState.currentModel, pendingState.currentParameters,
                        userConfigured = false, hostBinding = pendingState.executionHostBinding))
            }
        }
    }

    fun loadConversations() {
        viewModelScope.launch {
            runCatching { withContext(Dispatchers.IO) { conversationRepo.getAllConversations().filter { it.agentRoomOwner == null } } }
                .onSuccess { convs ->
                    conversationCatalog.value = convs
                    conversationListError.value = null
                    _uiState.update { it.copy(conversations = convs) }
                }.onFailure { error ->
                    if (error is kotlinx.coroutines.CancellationException) throw error
                    conversationListError.value = error.message ?: error.toString()
                }
        }
    }

    fun renameConversation(conversationId: String, rawTitle: String) {
        val trimmed = rawTitle.trim()
        if (conversationId.isBlank() || trimmed.isEmpty()) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    conversationRepo.updateTitle(conversationId, trimmed)
                }
                conversationCatalog.update { list ->
                    list.map { conv -> if (conv.id == conversationId) conv.copy(title = trimmed) else conv }
                }
                _uiState.update { state ->
                    state.copy(conversations = state.conversations.map { conv ->
                        if (conv.id == conversationId) conv.copy(title = trimmed) else conv
                    })
                }
                displayedState.update { state ->
                    state.copy(conversations = state.conversations.map { conv ->
                        if (conv.id == conversationId) conv.copy(title = trimmed) else conv
                    })
                }
                loadConversations()
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                conversationListError.value = error.message ?: error.toString()
            }
        }
    }

    fun loadConversation(convId: String) = loadConversation(convId, background = false)

    private fun loadConversation(convId: String, background: Boolean, reconcileNativeIntent: Boolean = false) {
        if (!background) {
            modelSelectionGeneration += 1
            if (_uiState.value.conversationId != convId) {
                selectedConversationAgentIds.value = emptySet(); selectedConversationAgentRevisions.clear(); selectedConversationAgentParticipants.clear(); expertPickerSource = null; expertRejoinRequest.value = null; expertRejoinError.value = null
            }
        }
        val loadGeneration = if (background) conversationLoads.currentGeneration() else conversationLoads.beginRequest()
        runRegistry.find(composerDrafts.ownerKey(convId))?.takeIf { !reconcileNativeIntent && it.retainsLoadedPresentation() }?.let {
            if (!background) {
                val restored = it.state.value
                it.state.value = restored.copy(unknownEffectRecovery = restored.unknownEffectRecovery?.copy(loadGeneration = loadGeneration))
                displayedState.value = it.state.value
            }
            val selected = it
            runRegistry.scope(selected).launch { runCatching { refreshPermissionOwnerProjection(selected) } }
            return
        }
        if (!background && displayedState.value.conversationId != convId) {
            // Route and draft ownership change immediately. The old source's
            // composer must never accept input while the new Store snapshot is
            // still loading. Existing execution sessions keep their own state.
            displayedState.update { it.newConversationPresentation(defaultPermissionMode)
                .copy(conversationId = convId, processingState = ProcessingState.BuildingContext) }
        }
        val loadingSession = if (background) runRegistry.session(composerDrafts.ownerKey(convId),
            (agentRoomPresentation.state.value.contexts[convId] ?: ChatUiState(conversationId = convId))
                .copy(processingState = ProcessingState.BuildingContext)
        ).also { it.loadingPresentation = true } else null
        val loadingScope = loadingSession?.let(runRegistry::scope) ?: viewModelScope
        loadingScope.launch {
            try {
                fun canCommitLoad(): Boolean = isExecutionCurrent(loadGeneration)
                val conv = conversationRepo.getConversation(convId)
                if (!canCommitLoad()) return@launch
                if (conv == null) {
                    markConversationUnavailable()
                    return@launch
                }
                val remoteBacked = conv.selectedModelInfo?.source == ModelInfo.ModelSource.WebHost ||
                    (conv.executionHostBinding != null && withContext(Dispatchers.IO) {
                        applicationRuntime.conversationSourceOwner(convId) is com.nola.data.local.ConversationSourceOwner.Remote
                    })
                val localTasks = withContext(Dispatchers.IO) {
                    if (applicationRuntime.conversationSourceOwner(convId) != com.nola.data.local.ConversationSourceOwner.Local) null
                    else {
                        val consumer = "android-task-view:${java.util.UUID.randomUUID()}"
                        try {
                            while (applicationRuntime.execute(buildJsonObject {
                                put("type", "apply_pending_task_notice"); put("conversation_id", convId)
                                put("consumer_id", consumer); put("now_ms", System.currentTimeMillis())
                            }) != JsonNull) { }
                        } catch (failure: Exception) {
                            if (failure is kotlinx.coroutines.CancellationException) throw failure
                            android.util.Log.w("ChatViewModel", "Task notice delivery remains pending; inspection is still read-only")
                        }
                        // Delivery failure cannot hide already committed Core facts.
                        // New conversations do not yet have a Run checkpoint.
                        runCatching {
                            val read = applicationRuntime.executeQuery(buildJsonObject {
                                put("type", "load_conversation_task_projection_v1"); put("conversation_id", convId)
                            }).jsonObject
                            decodeAgentCoreTaskProjection(read["task_projection"], read.getValue("core_session_id").jsonPrimitive.content)
                        }.getOrNull()
                    }
                }
                val window = withContext(Dispatchers.IO) {
                    conversationRepo.getProjectionWindow(convId, beforeOrdinal = null)
                }
                val privatePending = withContext(Dispatchers.IO) { runCatching {
                    if (applicationRuntime.conversationSourceOwner(convId) == com.nola.data.local.ConversationSourceOwner.Local)
                        applicationRuntime.expertHostFactory.pendingAdmission(convId) else null
                } }
                val remotePending = conv.executionHostBinding?.let { binding -> withContext(Dispatchers.IO) { readRemoteExpertRetry(resolveBoundHost(binding), convId) } }
                if (!isExecutionCurrent(loadGeneration)) return@launch
                remotePending?.let { saved ->
                    pendingExpertAdmissionByDraft.restore(composerDrafts.ownerKey(convId), saved.input)
                    selectedConversationAgentIds.value = saved.input.selectedParticipants.map { it.agentId }.toSet()
                    saved.input.selectedParticipants.forEach { ref ->
                        selectedConversationAgentRevisions[ref.agentId] = ref.agentRevision
                        selectedConversationAgentParticipants[ref.agentId] = com.nola.data.repository.RoomParticipant(ref.participantId, ref.agentId, ref.agentRevision,
                            context.getString(R.string.chat_experts), ref.contextConversationId, true, "")
                    }
                }
                val nativePending = privatePending.getOrNull()
                nativePending?.let { pendingExpertAdmissionByDraft.restore(composerDrafts.ownerKey(convId), it.input) }
                val messages = CanonicalConversationProjector.project(window.projection)
                val expertParent = conv.agentRoomOwner?.let { owner -> withContext(Dispatchers.IO) {
                    val remoteKey = "remote_expert_context_source/v1/$convId"
                    val key = if (conv.executionHostBinding != null) remoteKey else "agent_chat_room_conversation/${owner.roomId}"
                    val record = applicationRuntime.executeQuery(buildJsonObject { put("type", "get_product_setting"); put("key", key) })
                    if (record == JsonNull) null else Json.parseToJsonElement(record.jsonObject.getValue("value_json").jsonPrimitive.content).jsonObject.let { link ->
                        require(link["room_id"]?.jsonPrimitive?.content == owner.roomId)
                        if (conv.executionHostBinding != null) { require(link["conversation_id"]?.jsonPrimitive?.content == convId); link["parent_conversation_id"]?.jsonPrimitive?.content }
                        else link["conversation_id"]?.jsonPrimitive?.content
                    }
                } }
                val childOwner = window.projection.taskChildOwner
                val restoredLiveMessage = messages.lastOrNull { message ->
                    message.meta?.get(CONVERSATION_SURFACE_LIVE_META_KEY) == "true"
                }
                val restoredThinking = restoredLiveMessage
                    ?.let { decodeAgentThinkingFromMeta(it.meta) }
                    .orEmpty()
                val restoredToolEvents = restoredLiveMessage
                    ?.let { decodeHostToolEventsFromMeta(it.meta) }
                    .orEmpty()
                val traceProjection = agentTraceRepository.latestProjection(convId) ?: AgentTraceProjection()
                val selectedModel = conv.selectedModelInfo
                val savedParameters = selectedModel?.let { conversationRepo.modelParameters(convId, it) }
                val params = savedParameters ?: ModelParameters()
                val generationOverride = selectedModel?.let {
                    com.nola.domain.model.ConversationGenerationOverride.modelKeyOrNull(it)
                        ?.let(conv.generationOverrides::get)
                } ?: com.nola.domain.model.ConversationGenerationOverride()
                val restoredContext = RestoredConversationContextUsage(
                    usedTokens = window.projection.contextUsage.usedTokens.coerceIn(0, Int.MAX_VALUE.toLong()).toInt(),
                    limitTokens = window.projection.contextUsage.windowTokens.coerceIn(0, Int.MAX_VALUE.toLong()).toInt(),
                )
                val restoredPercent = restoredContext.fraction * 100f
                if (!canCommitLoad()) return@launch
                _uiState.update {
                    val stagedToolEvents = capabilityActivationBuffer.events(convId)
                    val liveProcessingState = when {
                        restoredLiveMessage == null -> ProcessingState.Idle
                        restoredLiveMessage.text.isNotEmpty() -> ProcessingState.Streaming
                        restoredThinking.isNotEmpty() -> ProcessingState.ParsingThinking
                        else -> ProcessingState.BuildingContext
                    }
                    val base = if (it.conversationId == convId) it else it.newConversationPresentation(defaultPermissionMode)
                    base.copy(
                        conversationId = convId,
                        coreTaskProjection = localTasks ?: AgentCoreTaskProjection(),
                        taskChildOwner = childOwner,
                        expertContextOwner = conv.agentRoomOwner,
                        expertContextParentId = expertParent,
                        agentPermissionRequest = null,
                        agentPermissionAction = null,
                        unknownEffectRecovery = null,
                        resolvingUnknownEffects = false,
                        messages = messages,
                        historyBeforeOrdinal = window.beforeOrdinal,
                        hasMoreHistory = window.hasMore,
                        compactionSourceTargetId = null,
                        compactionSourceStatus = CompactionSourceStatus.Idle,
                        // Cold journal content remains authoritative even if
                        // the Host cannot resume. An empty live draft must not
                        // erase its task controls, tools, or published text.
                        journalResponse = restoredLiveMessage,
                        activeResponseId = restoredLiveMessage?.id,
                        activeResponseStartedAt = restoredLiveMessage?.createdAt,
                        currentResponse = restoredLiveMessage?.text.orEmpty(),
                        thinking = restoredThinking,
                        thinkingFinished = false,
                        toolEvents = (restoredToolEvents + stagedToolEvents).distinctBy { event -> event.id },
                        permissionRequest = null,
                        pendingPermissionRequests = emptyList(),
                        processingState = if (restoredLiveMessage == null && (nativePending != null || privatePending.isFailure))
                            ProcessingState.AwaitingRecovery else liveProcessingState,
                        pendingExpertHostAdmission = nativePending,
                        expertHostConfirmationStatus = if (nativePending != null || privatePending.isFailure)
                            context.getString(R.string.expert_host_awaiting_confirmation) else null,
                        checkingExpertHostAdmission = false,
                        currentExecutionBackend = conv.executionBackend,
                        currentModel = selectedModel,
                        configurationRevision = conv.configurationRevision,
                        pendingCommandId = null,
                        executionHostBinding = conv.executionHostBinding,
                        error = if (nativePending == null && privatePending.isSuccess && childOwner == null && (selectedModel == null || savedParameters == null)) "Conversation model configuration is missing; select and configure a model for this conversation." else null,
                        sessionMemory = conv.sessionMemory,
                        knowledgePoints = conv.knowledgePoints,
                        conversationWiki = conv.wiki,
                        selectedDataSourceIds = conv.dataSourceIds,
                        contextUsage = restoredContext.fraction,
                        contextTokens = restoredContext.usedTokens,
                        contextUsageDetail = com.nola.domain.model.ContextUsage(
                            totalTokens = restoredContext.usedTokens,
                            maxTokens = restoredContext.limitTokens,
                            usagePercent = restoredPercent,
                            breakdown = com.nola.domain.model.ContextBreakdown(),
                            cachedInputTokens = restoredContext.cachedInputTokens,
                            cacheWriteInputTokens = restoredContext.cacheWriteInputTokens,
                            promptCacheStatus = restoredContext.promptCacheStatus,
                        ),
                        currentParameters = params,
                        generationOverride = generationOverride,
                        responseStyleOverride = conv.promptResponseStyle,
                        permissionMode = conversationPermissionMode(conv.permissionMode, defaultPermissionMode),
                        permissionSaving = false,
                        permissionSaveFailed = false,
                        permissionCommitRevision = 0,
                        permissionPolicyRunId = null,
                        permissionPolicyRevision = 0,
                        remotePermissionPolicy = null,
    remotePermissionApplicationState = null,
                        contextMaxTokens = restoredContext.limitTokens,
                        traceProjection = traceProjection,
                    )
                }
                _uiState.update { state ->
                    if (state.conversationId == conv.id) state.applyCanonicalContextUsage(window.projection.contextUsage) else state
                }
                // Task workers own their execution. Opening history never starts a second pump.
                if (childOwner != null) return@launch
                val pendingRun = window.projection.activeRun
                val pendingCommand = withContext(Dispatchers.IO) { applicationRuntime.pendingConversationCommand(convId) }
                if (!canCommitLoad() || _uiState.value.conversationId != convId) return@launch
                if (pendingCommand != null) _uiState.update {
                    it.copy(processingState = ProcessingState.AwaitingRecovery, pendingCommandId = pendingCommand,
                        error = com.nola.data.local.AndroidApplicationRuntime.PendingMutation(pendingCommand).message)
                }
                val permissionOwnerSession = executionSession
                if (runRegistry.current == null) permissionOwnerSession.state.value = displayedState.value
                permissionOwnerSession.permissionSourceUnconfirmed = remoteBacked
                if (!permissionOwnerSession.permissionSourceUnconfirmed) runCatching { refreshPermissionOwnerProjection(permissionOwnerSession) }
                val recoveryInspected = if (pendingRun != null && !remoteBacked) {
                    refreshUnknownEffectRecovery(conv, pendingRun?.runId, loadGeneration)
                } else false
                if (!isExecutionCurrent(loadGeneration) || _uiState.value.conversationId != convId) return@launch
                if (
                    !remoteBacked &&
                    shouldResumeLoadedDeviceRun(pendingRun?.state, recoveryInspected,
                        pendingCommand != null, _uiState.value.unknownEffectRecovery != null)
                ) {
                    resumeExistingDeviceRun(conversation = conv, runId = checkNotNull(pendingRun).runId)
                }
                if (pendingRun == null && !remoteBacked) {
                    val lastAssistant=window.projection.messages.lastOrNull { it.role==com.nola.domain.projection.ConversationProjectionV1.Role.Assistant }
                    val identity=window.assistantJournalHeads.mapNotNull {it["identity"] as? JsonObject}.singleOrNull {
                        it["conversation_id"]==JsonPrimitive(convId)&&it["assistant_message_id"]==lastAssistant?.messageId?.let(::JsonPrimitive)
                    }
                    val sealed=identity?.let { captured -> withContext(Dispatchers.IO) {
                        com.nola.core.loadAndroidHistoryTerminal(captured,applicationRuntime::executeQuery)
                    } }
                    if(isExecutionCurrent(loadGeneration)&&_uiState.value.conversationId==convId)presentSealedRunResult(convId,sealed?.result)
                    if(sealed==null||sealed.result["value"]?.jsonObject?.get("kind") in setOf(JsonPrimitive("answered"),JsonPrimitive("action_completed"))) {
                        wakeLocalGoal(convId);wakeIdleQueuedUiInput(convId)
                    }
                }
                if (remoteBacked) {
                    // Paint the local window first. Catch up only its Runs; no model resubmission
                    // or full-history scan is needed to recover post-terminal task updates.
                    fun currentWindow() = _uiState.value.conversationId == convId &&
                        isExecutionCurrent(loadGeneration)
                    _uiState.update { if (currentWindow()) it.copy(processingState = ProcessingState.BuildingContext) else it }
                    try {
                        val configuration = resolveBoundHost(requireNotNull(conv.executionHostBinding)).let { captured ->
                            conv.agentRoomOwner?.let { captured.copy(clientId = "room:${it.roomId}") } ?: captured
                        }
                        if (currentWindow()) {
                            remoteObservationConfiguration = configuration
                            webAgentProvider.config = configuration
                            // Discovery supplies the canonical root even with no
                            // heads; a display message ID alone is not a root.
                            remoteObservationInitialKey = null
                            remoteObservationVisibleHeads = window.assistantJournalHeads
                            _uiState.update { if (currentWindow()) it.copy(processingState = ProcessingState.BuildingContext) else it }
                            val session = executionSession
                            session.directHostHistory = conv.agentRoomOwner == null && childOwner == null &&
                                window.assistantJournalHeads.isEmpty() &&
                                withContext(Dispatchers.IO) { conversationRepo.readRemoteSubmissionIntent(convId) } == null
                            if (!currentWindow()) return@launch
                            startRemoteConversationObservation(convId)
                            if (!session.directHostHistory) observeRemoteAgentTasks(convId)
                            val hostSettingsGeneration = modelSelectionGeneration
                            viewModelScope.launch {
                                refreshHostAgentSettings(convId, hostSettingsGeneration)
                            }
                        }
                    } catch (cancelled: kotlinx.coroutines.CancellationException) {
                        throw cancelled
                    } catch (failure: Exception) {
                        // Offline history stays usable; a later window refresh retries exact cursors.
                        Log.w("ChatViewModel", "Visible journal refresh unavailable")
                        if (currentWindow()) _uiState.update { it.copy(processingState = ProcessingState.AwaitingRecovery,
                            error = failure.message ?: context.getString(R.string.chat_agent_error)) }
                    }
                }
                // Direct Web Host transcripts already include their expert replies. Only canonical
                // conversation owners can resolve separate per-message admission manifests.
                if (!executionSession.directHostHistory) loadConversationExpertReplies(convId)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                if (isExecutionCurrent(loadGeneration)) _uiState.update {
                    it.copy(processingState = if (failure is com.nola.data.local.AndroidApplicationRuntime.PendingMutation)
                        ProcessingState.AwaitingRecovery else ProcessingState.Error,
                        error = failure.message ?: context.getString(R.string.chat_agent_error),
                        pendingCommandId = (failure as? com.nola.data.local.AndroidApplicationRuntime.PendingMutation)?.requestId)
                }
            } finally {
                loadingSession?.loadingPresentation = false
            }
        }
    }

    fun onConversationForeground() {
        remoteUiForeground = true
        _uiState.value.conversationId?.let { wakeLocalGoal(it) }
        if (!turnRuntime.hasActiveGeneration()) _uiState.value.conversationId?.let { conversationId ->
            if (_uiState.value.currentModel?.source == ModelInfo.ModelSource.WebHost) {
                if (executionSession.configuration == null &&
                    _uiState.value.processingState == ProcessingState.AwaitingRecovery) {
                    loadConversation(conversationId)
                } else {
                    startRemoteConversationObservation(conversationId)
                    wakeRemoteUiQueue(conversationId)
                    val hostSettingsGeneration = modelSelectionGeneration
                    viewModelScope.launch {
                        refreshHostAgentSettings(conversationId, hostSettingsGeneration)
                    }
                }
            } else viewModelScope.launch {
                try { wakeIdleQueuedUiInput(conversationId) }
                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (_uiState.value.conversationId == conversationId) _uiState.update { it.copy(error = failure.message) }
                }
            }
        }
    }

    fun onConversationBackground() {
        remoteUiForeground = false
        pauseLocalGoalsForLifecycle()
        remoteUiWakeJob?.cancel()
        // Backgrounding changes presentation only. Original dispatch/receipt
        // reconciliation stays with its owner, including unknown ACK windows.
    }

    private fun hostSessionKey(localId: String) = "web_host_chat_session/$localId"

    private fun hostSessionReference(localId: String): JsonObject? =
        applicationRuntime.productSetting(hostSessionKey(localId))?.let { Json.parseToJsonElement(it.valueJson).jsonObject }

    private suspend fun readHostSession(hostId: String, localId: String): String? = withContext(Dispatchers.IO) {
        hostSessionReference(localId)?.let {
            require(it["host"] == JsonPrimitive(hostId)) { context.getString(R.string.chat_error_host_identity_changed) }
            it.getValue("session").jsonPrimitive.content
        }
    }

    private suspend fun rememberHostSession(hostId: String, localId: String, remoteId: String,
        messages: List<ConversationMessage> = emptyList()) = withContext(Dispatchers.IO) {
        require(remoteId.isNotBlank())
        val previous = hostSessionReference(localId)
        require(previous == null || previous["host"] == JsonPrimitive(hostId) && previous["session"] == JsonPrimitive(remoteId))
        val preview = messages.lastOrNull { it.text.isNotBlank() }?.text?.replace("\n", " ")?.take(240)
        val reference = buildJsonObject {
            put("host", hostId)
            put("session", remoteId)
            (preview ?: previous?.get("preview")?.jsonPrimitive?.contentOrNull)?.let { put("preview", it) }
        }
        if (previous != reference) {
            applicationRuntime.putProductSetting(hostSessionKey(localId), reference.toString(), System.currentTimeMillis())
            if (preview != null) conversationPreviews.update { it + (localId to preview) }
        }
        val first = messages.firstOrNull { it.role == ChatRole.user }
        if (first != null && applicationRuntime.updateFirstMessageTitle(localId, first.id, first.text.trim().take(80), verifiedHostHistory = true)) {
            loadConversations()
        }
    }

    private fun observeDirectHostHistory(localId: String, configuration: WebAgentProvider.Config) {
        val session = executionSession
        if (session.directHistoryJob?.isActive == true || session.runtime.hasOutstandingGeneration()) return
        fun current() = !session.retired && _uiState.value.conversationId == localId &&
            remoteObservationConfiguration === configuration && !session.runtime.hasOutstandingGeneration()
        session.directHistoryJob = executionScope.launch {
            try {
                val saved = readHostSession(configuration.expectedHostId, localId)
                val knownId = saved ?: _uiState.value.v2SessionId
                session.directHistoryMissing = knownId == null
                val remoteId = knownId ?: localId
                var checkExists = true
                while (current()) {
                    val snapshot = webAgentProvider.v2ChatHistory(localId, remoteId, configuration, checkExists)
                    if (!current()) return@launch
                    if (snapshot == null) {
                        // Earlier builds kept the direct Host session ID in memory only.
                        // A transcript from another session must never be attached by title.
                        session.directHistoryMissing = true
                        _uiState.update { it.copy(processingState = ProcessingState.Error,
                            error = context.getString(R.string.chat_missing_host_history)) }
                        return@launch
                    }
                    if (!snapshot.active || saved == null && checkExists) {
                        rememberHostSession(configuration.expectedHostId, localId, remoteId, snapshot.messages)
                    }
                    if (!current()) return@launch
                    checkExists = false
                    session.directHistoryMissing = false
                    session.permissionSourceUnconfirmed = false
                    permissionSession.setHostRequest(null)
                    _uiState.update { it.copy(v2SessionId = remoteId, messages = snapshot.messages,
                        v2UiSurfaces = snapshot.surfaces, processingState = if (snapshot.active) ProcessingState.Streaming else ProcessingState.Idle,
                        activeResponseId = null, activeResponseStartedAt = null, currentResponse = "", thinking = "",
                        thinkingFinished = !snapshot.active, toolEvents = emptyList(), pendingUserQuestion = null,
                        permissionRequest = null, pendingPermissionRequests = emptyList(), error = null) }
                    if (!snapshot.active) {
                        refreshPermissionOwnerProjection(session)
                        return@launch
                    }
                    snapshot.pending.forEach { handleAgentEvent(it, localId) }
                    delay(1_000)
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                if (current()) _uiState.update { it.copy(processingState = ProcessingState.Error, error = failure.message) }
            }
        }
    }

    private fun startRemoteConversationObservation(convId: String) {
        if (runRegistry.current == null) {
            if (displayedState.value.conversationId != convId) return
            val session = executionSession
            if (!session.runtime.hasActiveGeneration()) session.state.value = displayedState.value
            runRegistry.enter(session) { startRemoteConversationObservation(convId) }
            return
        }
        val configuration = remoteObservationConfiguration ?: return
        if (executionSession.retired || _uiState.value.conversationId != convId) return
        if (executionSession.directHostHistory) {
            observeDirectHostHistory(convId, configuration)
            return
        }
        // One owner refresh per explicit foreground/reconnect subscription.
        // Failed hydration can retry here; unchanged periodic observations do not query authority.
        executionSession.permissionObservedIdentity = null
        executionSession.permissionObservedRecovery = null
        executionSession.permissionHydration.invalidate(forgetSuccess = true)
        val transport = webAgentProvider.journalTransport(configuration)
        val resume = webAgentProvider.resumeTransport(configuration)
        fun current() = !executionSession.retired && _uiState.value.conversationId == convId &&
            remoteObservationConfiguration === configuration
        remoteConversationObserver.start(executionScope,
            contextId = "$convId:${configuration.expectedHostId}:${configuration.clientId}",
            hostId = configuration.expectedHostId, clientId = configuration.clientId, conversationId = convId,
            initialKey = remoteObservationInitialKey, current = ::current,
            query = webAgentProvider.observationTransport(configuration),
            catchUp = { observation ->
                val receipt = com.nola.data.remote.api.decodeObservedSubmissionReceipt(
                    checkNotNull(observation.submissionReceipt) { "Observed Run has no source submission admission" },
                    configuration.expectedHostId, configuration.clientId, observation.key)
                val originating = withContext(Dispatchers.IO) { conversationRepo.readRemoteSubmissionIntent(convId) }
                if (!current()) return@start false
                withContext(Dispatchers.IO) {
                    if (originating == null) conversationRepo.adoptObservedRemoteRun(receipt)
                    else {
                        require(originating.identity == receipt.identity && originating.requestDigest == receipt.requestDigest)
                        conversationRepo.acceptRemoteSubmissionReceipt(receipt)
                    }
                }
                if (!current()) return@start false
                remoteSubmissionAdmitted = true
                remoteAdmissionReceipt = receipt
                val owner = remoteJournalTurn?.takeIf { it.conversationId == convId && it.runId == observation.key.runId &&
                    it.assistantMessageId == observation.key.assistantMessageId }
                    ?: com.nola.data.local.AndroidJournalTurn(configuration.expectedHostId, convId,
                        observation.key.runId, observation.key.assistantMessageId, observation.key.runId)
                remoteJournalTurn = owner; remoteJournalTransport = transport
                remoteObservationInitialKey = observation.key
                val received = withContext(Dispatchers.IO) {
                    val result = assistantJournal.catchUpRun(owner, transport, ::current, observation.admission,
                        observation.terminalSeal, observation.contextUsage)
                    val history = remoteObservationVisibleHeads
                    if (history.isNotEmpty()) assistantJournal.recoverVisible(configuration.expectedHostId, convId, history, transport, ::current)
                    result
                }
                if (current()) {
                    remoteObservationVisibleHeads = emptyList()
                    val window = withContext(Dispatchers.IO) { conversationRepo.getProjectionWindow(convId, null) }
                    if (current()) {
                        remoteJournalProjection = window.projection
                        val messages = CanonicalConversationProjector.project(window.projection)
                        val root = messages.find { it.id == owner.assistantMessageId }
                        _uiState.update { state -> if (!current()) state else state.copy(
                            messages = messages, historyBeforeOrdinal = window.beforeOrdinal, hasMoreHistory = window.hasMore,
                            activeResponseId = owner.assistantMessageId, activeResponseStartedAt = root?.createdAt,
                            journalResponse = root, currentResponse = root?.text.orEmpty(),
                            thinking = root?.let { decodeAgentThinkingFromMeta(it.meta) }.orEmpty(),
                            toolEvents = root?.let { decodeHostToolEventsFromMeta(it.meta) }.orEmpty(),
                        ).applyCanonicalContextUsage(window.projection.contextUsage) }
                    }
                }
                if (current()) {
                    executionSession.permissionSourceUnconfirmed = !(received.second && observation.canSubmit &&
                        observation.recovery == com.nola.data.remote.api.RemoteRunRecovery.Terminal)
                    if (executionSession.permissionObservedIdentity != receipt.identity ||
                        executionSession.permissionObservedRecovery != observation.recovery ||
                        received.second && observation.canSubmit) {
                        executionSession.permissionObservedIdentity = receipt.identity
                        executionSession.permissionObservedRecovery = observation.recovery
                        runCatching { refreshPermissionOwnerProjection() }
                    }
                }
                received.second
            },
            onObservation = { observation, terminalCommitted ->
                if (current()) {
                    val terminal = terminalCommitted && (observation.canSubmit || _uiState.value.expertContextOwner != null) &&
                        observation.recovery == com.nola.data.remote.api.RemoteRunRecovery.Terminal
                    var state = _uiState.value.copy(hostGoalInProgress = false, processingState = if (terminal) ProcessingState.Idle else ProcessingState.Streaming,
                        error = if (observation.recovery == com.nola.data.remote.api.RemoteRunRecovery.Inconsistent)
                            context.getString(R.string.chat_agent_error) else null)
                    // These are the original live nonce challenges, not trace
                    // text or a fabricated permission inferred from a Part.
                    state = state.copy(permissionRequest = null, pendingPermissionRequests = emptyList())
                    for (wire in observation.permissions) {
                        require(wire.getValue("canonical_run_id").jsonPrimitive.content == observation.key.runId)
                        val event = com.nola.data.remote.sse.decodeSsePayload(wire.toString(), null) as? SseEvent.PermissionRequest
                            ?: error("Observation permission is not a typed live challenge")
                        val permissionClient = if (observation.goalExecution != null && event.clientId == "web-goal")
                            "web-goal" else configuration.clientId
                        require(observation.submissionReceipt != null || permissionClient == "web-goal")
                        val next = acceptHostPermissionEvent(state, event, convId, configuration.expectedHostId,
                            permissionClient, System.currentTimeMillis() / 1_000L)
                            ?: error("Observation permission identity mismatch")
                        state = next
                    }
                    permissionSession.setHostRequests(if (terminal) emptyList() else state.hostPermissionQueue)
                    if (terminal) executionSession.markTerminalCommitted()
                    if (terminal) state = state.copy(activeResponseId = null, activeResponseStartedAt = null,
                        permissionRequest = null, pendingPermissionRequests = emptyList(), thinkingFinished = true)
                    _uiState.update { if (current()) state else it }
                    val resultKind = (observation.terminalSeal?.get("result") as? JsonObject)
                        ?.get("value")?.let { it as? JsonObject }?.get("kind")?.jsonPrimitive?.content
                    if(terminal)presentSealedRunResult(convId,observation.terminalSeal?.get("result") as? JsonObject)
                    if (terminal && resultKind in setOf("answered", "action_completed")) wakeRemoteUiQueue(convId)
                }
            },
            onAbsent = {
                if (current()) {
                    permissionSession.setHostRequest(null)
                    val terminalSubmission = remoteSubmissionIntent?.phase in setOf(
                        com.nola.data.remote.api.RemoteSubmissionPhase.Rejected,
                        com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled)
                    _uiState.update { it.withRemoteOwnerAbsent(preserveSubmissionOutcome = terminalSubmission) }
                    executionSession.permissionSourceUnconfirmed = false
                    if (!terminalSubmission) wakeRemoteUiQueue(convId)
                }
            },
            onUnavailable = { failure ->
                if (current()) _uiState.update { it.copy(processingState = ProcessingState.BuildingContext,
                    error = failure.message ?: context.getString(R.string.chat_agent_error)) }
            },
            resume = { key ->
                // Only an authenticated resume_required snapshot grants this
                // exact operation. The normal submission endpoint is never used.
                resume(key).collect { event -> if (current()) handleAgentEvent(event, convId) }
            }, reconcileSubmission = { reconcileRemoteSubmissionBeforeRun(convId, configuration, ::current) },
            admittedKey = { remoteObservationInitialKey.takeIf { _uiState.value.activeResponseId != null } },
            stopAtTerminal = _uiState.value.expertContextOwner != null,
            presentHostExecution = { observation ->
                val pending = withContext(Dispatchers.IO) { conversationRepo.readRemoteSubmissionIntent(convId) }
                require(pending == null) { "A Host execution cannot acknowledge a mobile submission" }
                if (current()) {
                    remoteAdmissionReceipt = null
                    remoteSubmissionAdmitted = false
                    executionSession.permissionSourceUnconfirmed = !observation.canSubmit
                    permissionSession.setHostRequest(null)
                    _uiState.update { state -> state.copy(
                        hostGoalInProgress = observation.goalExecution != null && !observation.canSubmit,
                        processingState = if (observation.canSubmit) ProcessingState.Idle else ProcessingState.AwaitingRecovery,
                        activeResponseId = null, activeResponseStartedAt = null,
                        permissionRequest = null, pendingPermissionRequests = emptyList(), error = null,
                    ) }
                }
            })
    }

    private suspend fun reconcileRemoteSubmissionBeforeRun(convId: String, configuration: WebAgentProvider.Config,
        current: () -> Boolean): Boolean {
        val intent = withContext(Dispatchers.IO) { conversationRepo.readRemoteSubmissionIntent(convId) } ?: return true
        require(intent.identity.sourceHostId == configuration.expectedHostId && intent.identity.clientId == configuration.clientId)
        if (!current()) return false
        remoteSubmissionIntent = intent
        if (intent.phase == com.nola.data.remote.api.RemoteSubmissionPhase.Admitted) {
            remoteSubmissionAdmitted = true
            remoteObservationInitialKey = com.nola.data.remote.api.RemoteRunKey(convId, intent.identity.submissionId, intent.identity.assistantMessageId)
            return true
        }
        _uiState.update { it.copy(processingState = ProcessingState.BuildingContext,
            activeResponseId = intent.identity.assistantMessageId) }
        val digest = intent.requestDigest ?: return false // Never posted; explicit local abandon remains available.
        val query = com.nola.data.remote.api.remoteSubmissionReconciliationRequest(intent.identity, digest)
        val raw = webAgentProvider.submissionTransport(configuration)(query).getValue("result")
        if (!current() || raw !is JsonObject) return false // null is unknown, never rejection/idle.
        acceptRemoteSubmissionReceipt(raw, configuration, intent)
        return remoteSubmissionIntent?.phase in setOf(com.nola.data.remote.api.RemoteSubmissionPhase.Admitted,
            com.nola.data.remote.api.RemoteSubmissionPhase.Rejected, com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled)
    }

    private suspend fun acceptRemoteSubmissionReceipt(wire: JsonObject, configuration: WebAgentProvider.Config,
        intent: com.nola.data.remote.api.RemoteSubmissionIntent = checkNotNull(remoteSubmissionIntent)) {
        val loadGeneration = conversationLoads.currentGeneration()
        val receipt = com.nola.data.remote.api.decodeRemoteSubmissionReceipt(wire, intent.identity, checkNotNull(intent.requestDigest))
        require(receipt.identity.sourceHostId == configuration.expectedHostId && receipt.identity.clientId == configuration.clientId)
        val accepted = withContext(Dispatchers.IO) { conversationRepo.acceptRemoteSubmissionReceipt(receipt) }
        require(accepted.identity == intent.identity && accepted.requestDigest == intent.requestDigest)
        if (!isExecutionCurrent(loadGeneration) || _uiState.value.conversationId != intent.identity.conversationId ||
            remoteSubmissionIntent?.identity?.let { it != intent.identity } == true) return
        remoteSubmissionIntent = accepted
        when (accepted.phase) {
            com.nola.data.remote.api.RemoteSubmissionPhase.Admitted -> {
                retireSubmittedDraft()
                remoteSubmissionAdmitted = true
                remoteAdmissionReceipt = receipt
                val run = checkNotNull(receipt.admission).getValue("identity").jsonObject.getValue("run_id").jsonPrimitive.content
                val owner = remoteJournalTurn?.takeIf { it.hostId == configuration.expectedHostId && it.conversationId == intent.identity.conversationId
                    && it.submissionId == intent.identity.submissionId && it.assistantMessageId == intent.identity.assistantMessageId }
                    ?: com.nola.data.local.AndroidJournalTurn(configuration.expectedHostId, intent.identity.conversationId,
                        intent.identity.submissionId, intent.identity.assistantMessageId)
                owner.runId = run
                remoteJournalTurn = owner
                remoteJournalTransport = webAgentProvider.journalTransport(configuration)
                remoteObservationInitialKey = com.nola.data.remote.api.RemoteRunKey(intent.identity.conversationId, run, intent.identity.assistantMessageId)
                runCatching { refreshPermissionOwnerProjection() }
            }
            com.nola.data.remote.api.RemoteSubmissionPhase.Rejected, com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled -> {
                remoteSubmissionAdmitted = false; permissionSession.clear()
                _uiState.update { it.copy(processingState = ProcessingState.Idle, activeResponseId = null, currentResponse = "",
                    permissionRequest = null, pendingPermissionRequests = emptyList(), messages = it.messages.filterNot { message -> message.id in setOf(intent.identity.userMessageId, intent.identity.assistantMessageId) },
                    error = if (mainExpertRequiresTools(receipt.rejectionCode)) context.getString(R.string.expert_main_requires_tools) else receipt.rejectionCode) }
            }
            else -> _uiState.update { it.copy(processingState = ProcessingState.BuildingContext) }
        }
    }

    private suspend fun refreshUnknownEffectRecovery(conversation: Conversation, runId: String?, generation: Long): Boolean {
        fun current() = isExecutionCurrent(generation) && _uiState.value.conversationId == conversation.id
        try {
            val snapshot = withContext(Dispatchers.IO) {
                val pending = agentTurnService.pendingUnknownRecovery(conversation.id)
                if (pending != null) {
                    UnknownEffectRecoveryPresentation(conversation.id, pending.decision.runId, generation,
                        descriptor = null, inspection = null, pending = pending)
                } else if (runId != null) {
                    val descriptor = agentTurnService.recoveryDescriptor(conversation.id, conversation.title.orEmpty(), runId)
                    agentTurnService.inspectUnknownRecovery(descriptor)?.takeIf { it.targets.isNotEmpty() }?.let {
                        UnknownEffectRecoveryPresentation(conversation.id, runId, generation, descriptor, it, null)
                    }
                } else null
            }
            if (current()) _uiState.update { it.copy(unknownEffectRecovery = snapshot,
                processingState = if (snapshot != null) ProcessingState.AwaitingRecovery else it.processingState) }
            return current()
        } catch (failure: Exception) {
            if (failure is kotlinx.coroutines.CancellationException) throw failure
            if (current()) _uiState.update { it.copy(error = context.getString(R.string.chat_unknown_check_failed)) }
            return false
        }
    }

    fun inspectUnknownEffects() {
        val state = _uiState.value
        val conversationId = state.conversationId ?: return
        if (state.currentModel?.source == ModelInfo.ModelSource.WebHost || turnRuntime.hasActiveGeneration()) return
        val generation = conversationLoads.currentGeneration()
        viewModelScope.launch {
            try {
                val conversation = conversationRepo.getConversation(conversationId) ?: return@launch
                val projection = withContext(Dispatchers.IO) { conversationRepo.getProjectionWindow(conversationId, null).projection }
                refreshUnknownEffectRecovery(conversation, projection.activeRun?.runId, generation)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                if (_uiState.value.conversationId == conversationId && conversationLoads.currentGeneration() == generation) {
                    _uiState.update { it.copy(error = context.getString(R.string.chat_unknown_check_failed)) }
                }
            }
        }
    }

    fun resolveUnknownEffects(snapshot: UnknownEffectRecoveryPresentation,
                              decision: com.nola.data.local.UnknownEffectRecoveryDecision?) {
        fun current() = snapshot.owns(_uiState.value.conversationId, conversationLoads.currentGeneration()) &&
            _uiState.value.unknownEffectRecovery == snapshot
        if (!current() || _uiState.value.resolvingUnknownEffects || turnRuntime.hasActiveGeneration()) return
        if (snapshot.pending == null) {
            val inspection = snapshot.inspection ?: return
            if (decision == null || decision.conversationId != snapshot.conversationId || decision.runId != snapshot.runId ||
                decision.invocationConfigDigest != inspection.invocationConfigDigest) return
            when (decision.choice.kind) {
                com.nola.data.local.UnknownEffectRecoveryChoice.Kind.AuthorizeRiskRetry ->
                    if (!inspection.retryAllowed || decision.targets.size != 1 || !inspection.targets.contains(decision.targets.single())) return
                com.nola.data.local.UnknownEffectRecoveryChoice.Kind.KeepUnknownBlocked ->
                    if (inspection.independentPendingEffects != 0L || decision.targets != inspection.targets) return
            }
        } else if (decision != null) return
        // Once generated, the request identity and time are never replaced on a transport retry.
        val requestId = UUID.randomUUID().toString()
        _uiState.update { it.copy(resolvingUnknownEffects = true) }
        viewModelScope.launch {
            try {
                val reply = withContext(Dispatchers.IO) {
                    snapshot.pending?.let(agentTurnService::recoverUnknownRecovery)
                        ?: agentTurnService.submitUnknownRecovery(checkNotNull(decision), requestId)
                }
                if (!current()) return@launch
                when (reply.outcome) {
                    com.nola.data.local.UnknownEffectRecoveryReply.Outcome.RetryAuthorized -> {
                        val conversation = conversationRepo.getConversation(snapshot.conversationId) ?: return@launch
                        refreshUnknownEffectRecovery(conversation, snapshot.runId, snapshot.loadGeneration)
                        if (snapshot.owns(_uiState.value.conversationId, conversationLoads.currentGeneration()) &&
                            _uiState.value.unknownEffectRecovery == null) {
                            resumeExistingDeviceRun(conversation, snapshot.runId)
                        }
                    }
                    com.nola.data.local.UnknownEffectRecoveryReply.Outcome.UnknownPreservedBlocked -> {
                        val seal = withContext(Dispatchers.IO) {
                            agentTurnService.unknownRecoveryTerminal(snapshot.conversationId, snapshot.runId,
                                snapshot.descriptor?.assistantMessageId)
                        }
                        check(seal?.result?.get("value")?.jsonObject?.get("kind")?.jsonPrimitive?.content == "blocked")
                        if (current()) {
                            _uiState.update { it.copy(processingState = ProcessingState.Idle, unknownEffectRecovery = null) }
                            loadConversation(snapshot.conversationId)
                        }
                    }
                    else -> {
                        if (current()) _uiState.update { it.copy(unknownEffectRecovery = null,
                            error = context.getString(R.string.chat_unknown_changed)) }
                    }
                }
            } catch (failure: Exception) {
                if (failure is kotlinx.coroutines.CancellationException) throw failure
                if (current()) {
                    val conversation = conversationRepo.getConversation(snapshot.conversationId)
                    if (conversation != null) refreshUnknownEffectRecovery(conversation, snapshot.runId, snapshot.loadGeneration)
                    if (snapshot.owns(_uiState.value.conversationId, conversationLoads.currentGeneration()))
                        _uiState.update { it.copy(error = context.getString(R.string.chat_unknown_pending)) }
                }
            } finally {
                if (snapshot.owns(_uiState.value.conversationId, conversationLoads.currentGeneration()))
                    _uiState.update { it.copy(resolvingUnknownEffects = false) }
            }
        }
    }

    private fun resumeExistingDeviceRun(
        conversation: Conversation,
        runId: String,
    ) {
        val key = composerDrafts.ownerKey(conversation.id)
        if (runRegistry.find(key)?.runtime?.hasOutstandingGeneration() == true || runId.isBlank()) return
        if (conversation.selectedModelInfo?.source == ModelInfo.ModelSource.Local && !runRegistry.localAvailable(key)) {
            _uiState.update { it.copy(error = "Waiting for the device execution resource.") }
            return
        }
        val session = runRegistry.beginExecution(key, _uiState.value)
        try {
            if (conversation.selectedModelInfo?.source == ModelInfo.ModelSource.Local) check(runRegistry.acquireLocal(session))
            session.runtime.bindOwner(conversation.id)
            runRegistry.start(session) {
            try {
                val state = _uiState.value
                val toolPlan = localToolPlan(conversation.id, state)
                val descriptor = withContext(Dispatchers.IO) {
                    agentTurnService.recoveryDescriptor(conversation.id, conversation.title.orEmpty(), runId)
                }
                _uiState.update { it.bindRecoveredAssistant(descriptor) }
                runDeviceAgentResponse(
                    input = "",
                    convId = conversation.id,
                    prepared = null,
                    state = state,
                    toolPlan = toolPlan,
                    attachments = emptyList(),
                    canonicalTurn = descriptor,
                    recoverExistingRun = true,
                )
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                Log.e("ChatViewModel", "Existing Run recovery failed", error)
                _uiState.update {
                    it.copy(
                        // Failure to reacquire an existing owner is not a terminal Run receipt.
                        // Keep new submissions blocked until that exact Run is reconciled.
                        processingState = ProcessingState.AwaitingRecovery,
                        error = error.message ?: context.getString(R.string.common_unknown_error),
                    )
                }
            } finally {
                clearGenerationJobIfCurrent(coroutineContext[Job])
            }
            }
        } catch (error: Throwable) {
            runRegistry.releaseLocal(session)
            runRegistry.remove(key)
            throw error
        }
    }

    fun locateCompactionSource(request: CompactionSourceNavigator.Request) {
        val initial = _uiState.value
        if (!CompactionSourceNavigator.accepts(request, initial.conversationId, initial.messages)) return
        val conversationId = request.conversationId
        val sourceItemIds = request.sourceItemIds
        conversationLoads.cancelCompactionLookup()
        CompactionSourceNavigator.targetMessageId(sourceItemIds, initial.messages)?.let { target ->
            _uiState.update {
                it.copy(
                    compactionSourceTargetId = target,
                    compactionSourceRequest = it.compactionSourceRequest + 1,
                    compactionSourceStatus = CompactionSourceStatus.Idle,
                )
            }
            return
        }
        val loadGeneration = conversationLoads.currentGeneration()
        conversationLoads.startCompactionLookup(viewModelScope) {
            fun isCurrentLookup(): Boolean =
                CompactionSourceNavigator.isCurrentLookup(
                    requestedConversationId = conversationId,
                    currentConversationId = _uiState.value.conversationId,
                    requestedLoadGeneration = loadGeneration,
                    currentLoadGeneration = conversationLoads.currentGeneration(),
                )

            val stagedOlderPages = mutableListOf<List<ConversationMessage>>()
            var stagedBeforeOrdinal: Long? = null
            var stagedHasMore = false
            fun finishWithoutPages(status: CompactionSourceStatus) {
                if (!isCurrentLookup()) return
                _uiState.update { state ->
                    if (CompactionSourceNavigator.isCurrentLookup(
                            requestedConversationId = conversationId,
                            currentConversationId = state.conversationId,
                            requestedLoadGeneration = loadGeneration,
                            currentLoadGeneration = conversationLoads.currentGeneration(),
                        )
                    ) state.copy(compactionSourceStatus = status) else state
                }
            }
            fun commit(status: CompactionSourceStatus, target: String? = null) {
                if (!isCurrentLookup()) return
                _uiState.update { state ->
                    val current = CompactionSourceNavigator.isCurrentLookup(
                        requestedConversationId = conversationId,
                        currentConversationId = state.conversationId,
                        requestedLoadGeneration = loadGeneration,
                        currentLoadGeneration = conversationLoads.currentGeneration(),
                    )
                    when {
                        !current -> state
                        !CompactionSourceNavigator.accepts(
                            request,
                            state.conversationId,
                            state.messages,
                        ) -> state.copy(compactionSourceStatus = CompactionSourceStatus.Failed)
                        else -> state.copy(
                            messages = CompactionSourceNavigator.mergeFetchedOlderPages(
                                stagedOlderPages,
                                state.messages,
                            ),
                            historyBeforeOrdinal = stagedBeforeOrdinal,
                            hasMoreHistory = stagedHasMore,
                            compactionSourceTargetId = target,
                            compactionSourceRequest = if (target != null) {
                                state.compactionSourceRequest + 1
                            } else state.compactionSourceRequest,
                            compactionSourceStatus = status,
                        )
                    }
                }
            }

            _uiState.update {
                if (isCurrentLookup()) it.copy(compactionSourceStatus = CompactionSourceStatus.Loading)
                else it
            }
            val initialWindow = try {
                withContext(Dispatchers.IO) {
                    conversationRepo.getProjectionWindow(conversationId, beforeOrdinal = null)
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                finishWithoutPages(CompactionSourceStatus.Failed)
                return@startCompactionLookup
            }
            if (!isCurrentLookup() || !CompactionSourceNavigator.accepts(
                    request,
                    _uiState.value.conversationId,
                    _uiState.value.messages,
                ) || initialWindow.projection.conversationId != conversationId
            ) {
                finishWithoutPages(CompactionSourceStatus.Failed)
                return@startCompactionLookup
            }
            val expectedRevision = initialWindow.projection.revision
            val expectedEventSequence = initialWindow.eventSequence
            var requestFoundInSnapshot = CompactionSourceNavigator.accepts(
                request,
                conversationId,
                initialWindow.projection,
            )
            var cursor = initialWindow.beforeOrdinal
            var hasMore = initialWindow.hasMore
            stagedBeforeOrdinal = initialWindow.beforeOrdinal
            stagedHasMore = initialWindow.hasMore
            while (hasMore && cursor != null) {
                if (!isCurrentLookup() || !CompactionSourceNavigator.accepts(
                        request,
                        _uiState.value.conversationId,
                        _uiState.value.messages,
                    )
                ) {
                    finishWithoutPages(CompactionSourceStatus.Failed)
                    return@startCompactionLookup
                }
                val previousCursor = cursor ?: break
                val window = try {
                    withContext(Dispatchers.IO) {
                        conversationRepo.getProjectionWindow(conversationId, previousCursor)
                    }
                } catch (cancelled: kotlinx.coroutines.CancellationException) {
                    throw cancelled
                } catch (_: Exception) {
                    finishWithoutPages(CompactionSourceStatus.Failed)
                    return@startCompactionLookup
                }
                if (!isCurrentLookup() || !CompactionSourceNavigator.belongsToSnapshot(
                        window,
                        conversationId,
                        expectedRevision,
                        expectedEventSequence,
                    ) || !CompactionSourceNavigator.accepts(
                        request,
                        _uiState.value.conversationId,
                        _uiState.value.messages,
                    )
                ) {
                    finishWithoutPages(CompactionSourceStatus.Failed)
                    return@startCompactionLookup
                }
                val older = CanonicalConversationProjector.project(window.projection)
                stagedOlderPages.add(older)
                requestFoundInSnapshot = requestFoundInSnapshot || CompactionSourceNavigator.accepts(
                    request,
                    conversationId,
                    older,
                )
                stagedBeforeOrdinal = window.beforeOrdinal
                stagedHasMore = window.hasMore
                val target = CompactionSourceNavigator.targetMessageId(sourceItemIds, older)
                if (target != null) {
                    if (!requestFoundInSnapshot) {
                        finishWithoutPages(CompactionSourceStatus.Failed)
                        return@startCompactionLookup
                    }
                    commit(CompactionSourceStatus.Idle, target)
                    return@startCompactionLookup
                }
                if (!CompactionSourceNavigator.canContinue(
                        previousCursor,
                        window.beforeOrdinal,
                        window.hasMore,
                    )
                ) {
                    if (requestFoundInSnapshot) commit(CompactionSourceStatus.Unavailable)
                    else finishWithoutPages(CompactionSourceStatus.Failed)
                    return@startCompactionLookup
                }
                cursor = window.beforeOrdinal
                hasMore = window.hasMore
                kotlinx.coroutines.yield()
            }
            if (requestFoundInSnapshot) commit(CompactionSourceStatus.Unavailable)
            else finishWithoutPages(CompactionSourceStatus.Failed)
        }
    }

    fun consumeCompactionSourceTarget(request: Long) {
        _uiState.update { it.consumeCompactionSourceTarget(request) }
    }

    fun newConversation() {
        selectedConversationAgentIds.value = emptySet(); selectedConversationAgentRevisions.clear(); selectedConversationAgentParticipants.clear(); expertPickerSource = null; expertRejoinRequest.value = null; expertRejoinError.value = null
        modelSelectionGeneration += 1
        conversationLoads.beginRequest()
        _uiState.update { it.newConversationPresentation(defaultPermissionMode) }
        _uiState.update { it.copy(currentModel = null, currentParameters = ModelParameters()) }
        composerDrafts.beginNewDraft { loadAvailableModels() }
    }

    fun deleteConversation(convId: String) {
        if (runRegistry.find(composerDrafts.ownerKey(convId))?.state?.value?.processingState?.blocksInput == true) return
        viewModelScope.launch {
            runCatching { conversationRepo.deleteConversation(convId) }.onSuccess {
                runRegistry.remove(composerDrafts.ownerKey(convId))
                composerDrafts.remove(convId)
                conversationPreviews.update { it - convId }
                conversationExperts.update { it - convId }
                conversationActivity.update { it - convId }
                if (displayedState.value.conversationId == convId) newConversation()
                loadConversations()
            }.onFailure { error ->
                if (error is kotlinx.coroutines.CancellationException) throw error
                conversationListError.value = error.message ?: error.toString()
            }
        }
    }

    fun toggleAgentMode() {
        setAgentMode(!_uiState.value.agentMode)
    }

    fun setAgentMode(enabled: Boolean) {
        _uiState.update { it.copy(agentMode = enabled) }
        viewModelScope.launch { prefs.setAgentModeEnabled(enabled) }
    }

    suspend fun savePermissionMode(mode: com.nola.core.AndroidAgentPermissionMode) {
        val session = executionSession
        if (runRegistry.current == null && !session.runtime.hasActiveGeneration()) session.state.value = displayedState.value
        withContext(runRegistry.scope(session).coroutineContext) { savePermissionModeForOwner(mode) }
    }

    private suspend fun refreshPermissionOwnerProjection(session: ChatRunSession = executionSession): com.nola.data.local.PermissionConfigurationOwner {
        check(!session.retired)
        val state = session.state.value
        val conversationId = state.conversationId ?: return com.nola.data.local.PermissionConfigurationOwner.Inactive
        val configuration = session.configuration
        val knownRun = session.receipt?.takeIf { it.phase == com.nola.data.remote.api.RemoteSubmissionPhase.Admitted }?.identity?.submissionId
            ?: session.journalTurn?.runId ?: state.agentCoreSnapshot?.turnId
        try {
            val facts = withContext(Dispatchers.IO) {
                assistantJournal.permissionOwnerFacts(conversationId, configuration?.expectedHostId, configuration?.clientId,
                    knownRun, session.permissionSourceUnconfirmed || session.submission?.phase in setOf(
                        com.nola.data.remote.api.RemoteSubmissionPhase.Prepared, com.nola.data.remote.api.RemoteSubmissionPhase.Bound))
            }
            check(!session.retired && session.state.value.conversationId == conversationId && session.configuration === configuration)
            if (facts is com.nola.data.local.PermissionConfigurationOwner.Remote) {
                session.receipt?.let { require(it.phase == com.nola.data.remote.api.RemoteSubmissionPhase.Admitted && it.identity == facts.identity) }
            }
            session.state.update { it.copy(permissionConfigurationOwner = facts) }
            if (facts is com.nola.data.local.PermissionConfigurationOwner.Remote && configuration != null &&
                !session.state.value.permissionSaving) {
                val generation = session.permissionHydration.begin(facts.identity) ?: return facts
                runRegistry.scope(session).launch {
                    try {
                        val saved = webAgentProvider.permissionConfiguration(
                            com.nola.data.remote.api.RemoteRunKey(facts.identity.conversationId, facts.identity.submissionId,
                                facts.identity.assistantMessageId), configuration)
                        if (session.retired || session.configuration !== configuration ||
                            session.state.value.permissionConfigurationOwner != facts ||
                            !session.permissionHydration.finish(facts.identity, generation, succeeded = true)) return@launch
                        observeRemotePermissionPolicy(facts.identity, configuration, saved, session.state.value.permissionMode)
                    } catch (cancelled: kotlinx.coroutines.CancellationException) {
                        throw cancelled
                    } catch (_: Exception) {
                        // Retry only at the next selection/reconnect owner boundary, never per token.
                    } finally {
                        session.permissionHydration.finish(facts.identity, generation, succeeded = false)
                    }
                }
            }
            return facts
        } catch (failure: Throwable) {
            session.state.update { it.copy(permissionConfigurationOwner = null) }
            throw failure
        }
    }

    fun clearPermissionSaveFailed() {
        _uiState.update { if (it.permissionSaveFailed) it.copy(permissionSaveFailed = false) else it }
    }

    private suspend fun savePermissionModeForOwner(mode: com.nola.core.AndroidAgentPermissionMode) {
        val session = executionSession
        val conversationId = _uiState.value.conversationId
        val remoteConfiguration = remoteObservationConfiguration
        if (_uiState.value.permissionSaving) return
        val previousMode = _uiState.value.permissionMode
        session.permissionHydration.invalidate()
        _uiState.update { it.copy(permissionMode = mode, permissionSaving = true, permissionSaveFailed = false) }
        try {
            var committedRevision = 0L
            val policyOwner = refreshPermissionOwnerProjection()
            val baselineRevision = conversationId?.let {
                checkNotNull(conversationRepo.getConversation(it)).configurationRevision
            }
            if (policyOwner is com.nola.data.local.PermissionConfigurationOwner.Remote) {
                checkNotNull(remoteConfiguration) { "The active Host permission owner must be reconnected before saving" }
                val identity = policyOwner.identity
                val key = com.nola.data.remote.api.RemoteRunKey(identity.conversationId, identity.submissionId, identity.assistantMessageId)
                val baseline = webAgentProvider.permissionConfiguration(key, remoteConfiguration)
                check(refreshPermissionOwnerProjection() == policyOwner && !session.retired) { "The permission Run owner changed before saving" }
                val remoteSaved = if (baseline["permission_profile"]?.jsonPrimitive?.content == mode.wireValue) baseline else
                    webAgentProvider.permissionConfiguration(key, remoteConfiguration,
                        checkNotNull(baseline["configuration_revision"]?.jsonPrimitive?.content?.toLongOrNull()), mode)
                observeRemotePermissionPolicy(identity, remoteConfiguration, remoteSaved, mode)
                session.permissionHydration.confirm(identity)
            }
            check(!session.retired && refreshPermissionOwnerProjection() == policyOwner) { "The permission Run owner changed before the local mirror save" }
            if (conversationId != null) {
                conversationRepo.updatePermissionMode(conversationId, mode.wireValue,
                    checkNotNull(baselineRevision), checkNotNull(policyOwner))
                val committed = checkNotNull(conversationRepo.getConversation(conversationId))
                check(committed.permissionMode == mode.wireValue) { "Permission save was not confirmed" }
                committedRevision = committed.configurationRevision
            } else {
                prefs.setAgentPermissionMode(mode)
                defaultPermissionMode = mode
            }
            if (_uiState.value.conversationId == conversationId) turnRuntime.deviceCoordinator()?.refreshPermissionPolicy()
            _uiState.update {
                if (it.conversationId == conversationId) it.copy(permissionSaving = false,
                    permissionCommitRevision = committedRevision,
                    configurationRevision = maxOf(it.configurationRevision, committedRevision)) else it
            }
        } catch (error: Throwable) {
            _uiState.update {
                if (it.conversationId == conversationId) {
                    it.copy(
                        permissionMode = previousMode,
                        permissionSaving = false,
                        permissionSaveFailed = true,
                    )
                } else it
            }
            throw error
        }
    }

    private fun observeRemotePermissionPolicy(owner: com.nola.data.remote.api.RemoteSubmissionIdentity,
                                             configuration: WebAgentProvider.Config, saved: JsonObject,
                                             mode: com.nola.core.AndroidAgentPermissionMode) {
        val savedRevision = checkNotNull(saved["configuration_revision"]?.jsonPrimitive?.content?.toLongOrNull())
        fun current() = !executionSession.retired && _uiState.value.conversationId == owner.conversationId &&
            _uiState.value.permissionConfigurationOwner == com.nola.data.local.PermissionConfigurationOwner.Remote(owner)
        _uiState.update { state -> if (!current()) state else state.copy(
            permissionPolicyRunId = saved["active_run_id"]?.jsonPrimitive?.content?.takeUnless { it == "null" },
            permissionPolicyRevision = savedRevision,
            remotePermissionApplicationState = saved["state"]?.jsonPrimitive?.content,
            remotePermissionPolicy = (saved["active_policy"] as? JsonObject)?.let {
                Json.decodeFromJsonElement<com.nola.core.AgentActivePermissionPolicy>(it)
            }) }
        // A later mirror CAS failure must not hide a source-side policy ACK.
        executionScope.launch {
            var applicationState = saved["state"]?.jsonPrimitive?.content
            val key = com.nola.data.remote.api.RemoteRunKey(owner.conversationId,
                owner.submissionId, owner.assistantMessageId)
            while (current() && remoteObservationConfiguration === configuration &&
                _uiState.value.permissionPolicyRevision == savedRevision &&
                _uiState.value.permissionPolicyRunId == owner.submissionId) {
                if (applicationState != "pending") break
                kotlinx.coroutines.delay(750)
                if (!current()) break
                val next = runCatching { webAgentProvider.permissionConfiguration(key, configuration, minimumPolicyRevision = savedRevision) }.getOrNull() ?: break
                if (!current()) break
                applicationState = next["state"]?.jsonPrimitive?.content
                val nextPolicy = (next["active_policy"] as? JsonObject)?.let {
                    Json.decodeFromJsonElement<com.nola.core.AgentActivePermissionPolicy>(it)
                }
                _uiState.update { state -> if (!current() || state.permissionPolicyRevision != savedRevision) state else
                    state.copy(remotePermissionPolicy = nextPolicy,
                        remotePermissionApplicationState = applicationState,
                        permissionPolicyRunId = next["active_run_id"]?.jsonPrimitive?.content?.takeUnless { it == "null" }) }
            }
        }
    }

    fun setHybridDeviceToolsEnabled(enabled: Boolean) {
        _uiState.update { it.copy(hybridDeviceToolsEnabled = enabled) }
        viewModelScope.launch { prefs.setHybridDeviceToolsEnabled(enabled) }
    }

    fun setHybridDeviceCapability(capability: String, enabled: Boolean) {
        _uiState.update { state ->
            val capabilities = state.hybridDeviceCapabilities.toMutableSet()
            if (enabled) capabilities += capability else capabilities -= capability
            state.copy(hybridDeviceCapabilities = capabilities)
        }
        viewModelScope.launch { prefs.setHybridDeviceCapability(capability, enabled) }
    }

    fun forgetHostPairing() {
        val conversationId = _uiState.value.conversationId
        val generation = ++modelSelectionGeneration
        viewModelScope.launch {
            prefs.clearWebHostConfig()
            webAgentProvider.config = null
            if (modelService.currentModelSource == "web-host") modelService.usePlaceholder()
            if (generation != modelSelectionGeneration || _uiState.value.conversationId != conversationId) return@launch
            _uiState.update {
                it.copy(
                    pairedHostIdentity = "",
                    hybridDeviceToolsEnabled = false,
                    hybridDeviceCapabilities = emptySet()
                )
            }
            loadAvailableModels()
        }
    }

    private var modelSelectionGeneration = 0L
    private val hostSettingsOwnership = HostSettingsOwnership()

    private suspend fun resolveBoundHost(binding: com.nola.domain.model.ExecutionHostBinding): WebAgentProvider.Config {
        com.nola.data.repository.validateExecutionHostBinding(binding)
        val host = prefs.webHostConfiguration()
        require(host != null && binding.baseURL == host.address && binding.tlsCertificateSHA256.orEmpty() == host.pin) {
            "This conversation's Host is not the currently paired connection; reconnect its Host to continue"
        }
        return WebAgentProvider.Config(hostBaseUrl = host.address, token = host.token,
            expectedHostId = host.identity, clientId = prefs.getOrCreateClientUuid(),
            pinnedCertificateSha256 = host.pin)
    }

    fun setModel(model: ModelInfo) {
        if (preparingExpertPicker.value) return
        val requestedConversationId = _uiState.value.conversationId
        val expectedRevision = _uiState.value.configurationRevision
        val requestedDraftOwner = composerDrafts.ownerKey(requestedConversationId)
        val generation = ++modelSelectionGeneration
        viewModelScope.launch {
            fun current() = generation == modelSelectionGeneration && _uiState.value.conversationId == requestedConversationId &&
                composerDrafts.ownerKey(requestedConversationId) == requestedDraftOwner
            try {
            if (model.source != ModelInfo.ModelSource.WebHost) com.nola.data.repository.conversationModelIdentity(model)
            var hostConfiguration: WebAgentProvider.Config? = null
            val params = requestedConversationId?.let { conversationRepo.modelParameters(it, model) }
                ?: modelParamsStore.loadParameters(model.parameterProfileId)
            if (!current()) return@launch

            when (model.source) {
                ModelInfo.ModelSource.Local -> {
                    val ready = localLlamaProvider.state.value as? LocalModelRuntimeState.Ready
                    if (ready?.modelId != model.id) {
                        _uiState.update { it.copy(error = context.getString(R.string.models_error_model_unavailable)) }
                        return@launch
                    }
                }
                ModelInfo.ModelSource.Cloud -> {
                    val config = modelRepo.getApiModelConfigFor(model)
                    if (!current()) return@launch
                    if (config == null) {
                        _uiState.update { it.copy(error = context.getString(R.string.models_error_configure_endpoint_before_cloud)) }
                        return@launch
                    }
                }
                ModelInfo.ModelSource.WebHost -> {
                    val host = requireNotNull(prefs.webHostConfiguration()) { "Host connection is not configured" }
                    hostConfiguration = WebAgentProvider.Config(hostBaseUrl = host.address, token = host.token,
                        timezone = TimeZone.getDefault().id, maxIterations = 0,
                        expectedHostId = host.identity, clientId = prefs.getOrCreateClientUuid(),
                        pinnedCertificateSha256 = host.pin)
                }
            }

            if (generation != modelSelectionGeneration || _uiState.value.conversationId != requestedConversationId) return@launch
            val binding = hostConfiguration?.let { com.nola.domain.model.ExecutionHostBinding(it.hostBaseUrl, it.pinnedCertificateSha256.takeIf { value -> value.isNotBlank() }) }
            binding?.let { com.nola.data.repository.validateExecutionHostBinding(it) }
            val committed = requestedConversationId?.let {
                if (binding != null) conversationRepo.selectHost(it, binding, params, expectedRevision)
                else conversationRepo.selectModel(it, model, params, expectedRevision)
            }
            if (generation != modelSelectionGeneration || _uiState.value.conversationId != requestedConversationId) return@launch
            val generationOverride = requestedConversationId?.let { id ->
                com.nola.domain.model.ConversationGenerationOverride.modelKeyOrNull(model)?.let { key ->
                    conversationRepo.getConversation(id)?.generationOverrides?.get(key)
                }
            } ?: com.nola.domain.model.ConversationGenerationOverride()
            if (!current()) return@launch
            _uiState.update {
                it.copy(
                    currentModel = if (committed != null) committed.selectedModelInfo else model,
                    configurationRevision = committed?.configurationRevision ?: 0,
                    executionHostBinding = binding,
                    currentParameters = params,
                    generationOverride = generationOverride,
                    contextMaxTokens = generationOverride.applyingTo(params).maxContextTokens,
                    error = if (committed != null && committed.selectedModelInfo == null) "Selected model is unavailable" else null
                )
            }
            if (requestedConversationId == null) composerDrafts.savePendingConfiguration(requestedDraftOwner,
                ConversationDraftRepository.Configuration(model, params, hostBinding = binding))
            if (hostConfiguration != null && current()) {
                webAgentProvider.config = hostConfiguration
                refreshHostAgentSettings(requestedConversationId, generation)
            }
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                _uiState.update {
                    if (generation == modelSelectionGeneration && it.conversationId == requestedConversationId) {
                        it.copy(error = error.localizedMessage ?: context.getString(R.string.models_error_model_unavailable))
                    } else it
                }
            }
        }
    }

    fun setResponseStyle(style: String?) {
        val requestedConversationId = _uiState.value.conversationId
        viewModelScope.launch {
            if (_uiState.value.conversationId != requestedConversationId) return@launch
            try {
                saveResponseStyle(style)
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                _uiState.update {
                    if (it.conversationId == requestedConversationId) it.copy(error = error.localizedMessage) else it
                }
            }
        }
    }

    suspend fun saveResponseStyle(style: String?) {
        com.nola.domain.model.requireValidPromptResponseStyle(style)
        val conversationId = _uiState.value.conversationId
        val committedRevision = if (conversationId != null) conversationRepo.updateResponseStyle(conversationId, style) else null
        _uiState.update { current ->
            if (current.conversationId == conversationId) current.copy(
                responseStyleOverride = style,
                configurationRevision = committedRevision?.let { maxOf(current.configurationRevision, it) } ?: current.configurationRevision,
            ) else current
        }
    }

    fun setAutoCompactPercent(percent: Int) {
        val clamped = percent.coerceIn(50, 95)
        _uiState.update { it.copy(autoCompactTriggerPercent = clamped) }
        viewModelScope.launch {
            runCatching { memoryProfileDataStore.saveAutoCompactTriggerPercent(clamped) }
        }
    }

    private var parameterEditorOwner: Pair<String?, ModelInfo>? = null
    private var parameterEditorBaseline: ModelParameters? = null
    private var parameterEditorDraftOwner: String? = null
    private var parameterEditorRevision: Long = 0

    private val _savingReasoningPreference = MutableStateFlow(false)
    val savingReasoningPreference = _savingReasoningPreference.asStateFlow()

    fun reasoningEfforts(model: ModelInfo): List<String> {
        if (model.source != ModelInfo.ModelSource.Cloud) return emptyList()
        return runCatching { modelParamsStore.parameterSnapshot(model.parameterProfileId).reasoningEfforts }.getOrDefault(emptyList())
    }

    fun selectReasoningPreference(preference: String) {
        val state = _uiState.value
        val model = state.currentModel ?: return
        if (_savingReasoningPreference.value || state.processingState.blocksInput ||
            preference !in listOf("automatic") + reasoningEfforts(model) || !beginParameterEditing()) return
        _savingReasoningPreference.value = true
        viewModelScope.launch {
            try {
                val updated = if (preference in setOf("concise", "balanced", "detailed")) {
                    state.currentParameters.copy(reasoningPreference = null, reasoningTier = preference)
                } else {
                    state.currentParameters.copy(reasoningPreference = preference, reasoningTier = null)
                }
                saveModelParameters(updated)
            }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (_uiState.value.conversationId == state.conversationId && _uiState.value.currentModel == model)
                    _uiState.update { it.copy(error = context.getString(R.string.chat_save_failed)) }
            } finally { _savingReasoningPreference.value = false }
        }
    }

    fun beginParameterEditing(): Boolean {
        if (preparingExpertPicker.value) return false
        val state = _uiState.value
        val model = state.currentModel ?: return false
        parameterEditorOwner = state.conversationId to model
        parameterEditorBaseline = state.currentParameters
        parameterEditorDraftOwner = composerDrafts.ownerKey(state.conversationId)
        parameterEditorRevision = state.configurationRevision
        return true
    }

    suspend fun saveModelParameters(params: ModelParameters) {
        val owner = checkNotNull(parameterEditorOwner)
        val draftOwner = parameterEditorDraftOwner
        if (owner.first == null && draftOwner != composerDrafts.ownerKey(null)) return
        val committed = owner.first?.let { conversationRepo.saveModelParameters(it, owner.second, params,
            parameterEditorBaseline, parameterEditorRevision) }
        if (parameterEditorOwner != owner || _uiState.value.conversationId != owner.first || _uiState.value.currentModel != owner.second) return
        _uiState.update {
            it.copy(
                currentParameters = params,
                configurationRevision = committed?.configurationRevision ?: 0,
                generationOverride = com.nola.domain.model.ConversationGenerationOverride(),
                contextMaxTokens = params.maxContextTokens,
                error = null,
            )
        }
        if (owner.first == null && draftOwner != null) composerDrafts.savePendingConfiguration(draftOwner,
            ConversationDraftRepository.Configuration(owner.second, params, hostBinding = _uiState.value.executionHostBinding))
    }

    suspend fun saveGenerationOverride(value: com.nola.domain.model.ConversationGenerationOverride) {
        value.validate()
        val state = _uiState.value
        val conversationId = checkNotNull(state.conversationId) { "Select a conversation first" }
        val model = checkNotNull(state.currentModel) { "Select a model first" }
        require(model.source != ModelInfo.ModelSource.WebHost) { "Host model settings are managed by the host" }
        val committedRevision = conversationRepo.updateGenerationOverride(conversationId, model, value)
        _uiState.update { current ->
            if (current.conversationId == conversationId && current.currentModel == model) {
                current.copy(
                    generationOverride = value,
                    configurationRevision = maxOf(current.configurationRevision, committedRevision),
                    contextMaxTokens = value.applyingTo(current.currentParameters).maxContextTokens,
                    contextUsage = (current.contextTokens.toFloat() /
                        value.applyingTo(current.currentParameters).maxContextTokens).coerceIn(0f, 1f),
                    contextUsageDetail = current.contextUsageDetail?.let { usage ->
                        val limit = value.applyingTo(current.currentParameters).maxContextTokens
                        usage.copy(
                            maxTokens = limit,
                            usagePercent = usage.totalTokens.toFloat() / limit * 100f,
                        )
                    },
                    error = null,
                )
            } else current
        }
    }

    suspend fun saveMemoryProfile() {
        val state = _uiState.value
        val profile = memoryEditorSnapshot ?: memoryProfileDataStore.getSnapshot()
        val updated = profile.copy(
            longTermMemory = state.longTermMemory.split("\n").filter { it.isNotBlank() },
            userPreferences = state.userPreferences.split("\n").filter { it.isNotBlank() }
        )
        val committed = memoryProfileDataStore.saveMemoryText(updated)
        memoryProfileDataStore.saveAutoCompactTriggerPercent(state.autoCompactTriggerPercent.coerceIn(50, 95))
        memoryEditorSnapshot = committed
        _uiState.update { current ->
            current.copy(
                longTermMemory = if (current.longTermMemory == state.longTermMemory) {
                    committed.longTermMemory.joinToString("\n")
                } else current.longTermMemory,
                userPreferences = if (current.userPreferences == state.userPreferences) {
                    committed.userPreferences.joinToString("\n")
                } else current.userPreferences
            )
        }
    }

    private suspend fun refreshHostAgentSettings(conversationId: String?, selectionGeneration: Long) {
        val configuredHost = webAgentProvider.config ?: return
        if (selectionGeneration != modelSelectionGeneration || _uiState.value.conversationId != conversationId) return
        val request = hostSettingsOwnership.begin(conversationId, selectionGeneration, configuredHost)
        fun current() = hostSettingsOwnership.accepts(request, _uiState.value.conversationId,
            modelSelectionGeneration, webAgentProvider.config)
        try {
            val settings = webAgentProvider.fetchHostAgentSettings(request.configuration) ?: return
            if (!current()) return
            if (settings.hostIdentity?.permissionProtocolVersion != HOST_PERMISSION_PROTOCOL_VERSION) {
                _uiState.update { it.copy(error = context.getString(R.string.chat_error_host_permission_protocol)) }
                return
            }
            val identity = settings.hostIdentity?.id?.trim()?.takeIf { it.isNotEmpty() }
            if (identity != null) {
                val trusted = prefs.trustWebHostIdentity(configuredHost.hostBaseUrl, identity)
                if (!current()) return
                if (!trusted) {
                    _uiState.update { it.copy(error = context.getString(R.string.chat_error_host_identity_changed)) }
                    return
                }
            }
            val hostSkills = settings.uploadedSkills.orEmpty()
                .filter { it.resolvedContent.isNotBlank() }
                .map {
                    UploadedContent(
                        name = it.name.ifBlank { context.getString(R.string.chat_host_skill_default_name) },
                        content = it.resolvedContent,
                        kind = it.kind,
                        enabled = it.enabled,
                        description = it.description,
                        version = it.version,
                        requiredTools = it.requiredTools,
                        validationSteps = it.validationSteps,
                    )
                }
            val hostDocuments = settings.uploadedDocuments.orEmpty()
                .filter { it.resolvedContent.isNotBlank() }
                .map {
                    UploadedContent(
                        name = it.name.ifBlank { context.getString(R.string.chat_host_document_default_name) },
                        content = it.resolvedContent,
                        kind = it.kind,
                        enabled = it.enabled
                    )
                }
            val hostMcpServers = settings.mcpServers.orEmpty()
                .filter { it.resolvedName.isNotBlank() }
                .map {
                    McpServerConfig(
                        name = it.resolvedName.ifBlank { context.getString(R.string.chat_host_mcp_default_name) },
                        url = it.resolvedUrl,
                        enabled = it.enabled,
                        serverId = it.serverId ?: it.resolvedName,
                        availability = it.availability,
                        protocolVersion = it.protocolVersion,
                        tools = it.tools.keys.sorted(),
                        resources = it.resources.keys.sorted(),
                        resourceTemplates = it.resourceTemplates.keys.sorted(),
                        prompts = it.prompts.keys.sorted()
                    )
                }
            val computerUse = settings.computerUse?.let { status ->
                HostComputerUseStatus(
                    featureEnabled = status.featureEnabled,
                    tokenConfigured = status.tokenConfigured,
                    connected = status.connected,
                    hosts = status.hosts.orEmpty().map { host ->
                        HostComputerUseHost(
                            hostId = host.hostId,
                            capabilities = host.capabilities.orEmpty(),
                            lastSeenAgeSec = host.lastSeenAgeSec
                        )
                    },
                    tools = status.tools.orEmpty()
                )
            }
            val nativeBrowser = settings.nativeBrowser?.let { status ->
                HostNativeBrowserStatus(
                    tokenConfigured = status.tokenConfigured,
                    connected = status.connected,
                    structuredSearch = status.structuredSearch,
                    hosts = status.hosts.orEmpty().map { host ->
                        HostNativeBrowserHost(
                            hostId = host.hostId,
                            capabilities = host.capabilities.orEmpty(),
                            lastSeenAgeSec = host.lastSeenAgeSec
                        )
                    },
                    pendingCommands = status.pendingCommands,
                    inFlightCommands = status.inFlightCommands
                )
            }
            if (!current()) return
            if (identity != null) webAgentProvider.config = configuredHost.copy(expectedHostId = identity)
            _uiState.update {
                it.copy(
                    pairedHostIdentity = identity ?: configuredHost.expectedHostId,
                    hostUploadedSkills = hostSkills,
                    hostUploadedDocuments = hostDocuments,
                    hostMcpServers = hostMcpServers,
                    hostComputerUseStatus = computerUse,
                    hostNativeBrowserStatus = nativeBrowser,
                    executionBackends = settings.executionBackends,
                    dataSources = buildDataSources(
                        uploadedDocuments = it.uploadedDocuments,
                        hostUploadedDocuments = hostDocuments,
                        mcpServers = it.mcpServers,
                        hostMcpServers = hostMcpServers
                    )
                )
            }
        } catch (error: Exception) {
            if (error is kotlinx.coroutines.CancellationException) throw error
            if (!current()) return
            _uiState.update {
                it.copy(
                    hostUploadedSkills = emptyList(),
                    hostUploadedDocuments = emptyList(),
                    hostMcpServers = emptyList(),
                    hostComputerUseStatus = null,
                    hostNativeBrowserStatus = null,
                    executionBackends = emptyList(),
                    dataSources = buildDataSources(
                        uploadedDocuments = it.uploadedDocuments,
                        mcpServers = it.mcpServers
                    )
                )
            }
        }
    }

    fun updateSessionMemory(memory: String) {
        val previous = _uiState.value.sessionMemory
        _uiState.update { it.copy(sessionMemory = memory) }
        val convId = _uiState.value.conversationId ?: return
        viewModelScope.launch {
            try {
                conversationRepo.updateSessionMemory(convId, memory)
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update { current ->
                    if (current.conversationId == convId) {
                        current.copy(
                            sessionMemory = if (current.sessionMemory == memory) previous else current.sessionMemory,
                            error = error.localizedMessage ?: context.getString(R.string.chat_save_failed),
                        )
                    } else current
                }
            }
        }
    }

    fun selectExecutionBackend(backendId: String) {
        val state = _uiState.value
        val backend = state.executionBackends.firstOrNull { it.id == backendId } ?: return
        if (backend.availability != "available") return
        val conversationId = state.conversationId ?: return
        val previousBackend = state.currentExecutionBackend
        _uiState.update { it.copy(currentExecutionBackend = backendId) }
        viewModelScope.launch {
            try {
                val conversation = conversationRepo.getConversation(conversationId) ?: return@launch
                val committedRevision = conversationRepo.updateExecutionBackend(
                    conversationId,
                    backendId,
                    conversation.externalExecutionSessions,
                )
                _uiState.update { current ->
                    if (current.conversationId == conversationId) {
                        current.copy(configurationRevision = maxOf(current.configurationRevision, committedRevision))
                    } else current
                }
                loadConversations()
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update { current ->
                    if (current.conversationId == conversationId) {
                        current.copy(
                            currentExecutionBackend = if (current.currentExecutionBackend == backendId) previousBackend else current.currentExecutionBackend,
                            error = error.localizedMessage ?: context.getString(R.string.chat_save_failed),
                        )
                    } else current
                }
            }
        }
    }

    fun updateConversationKnowledge(knowledgePoints: String, wiki: String) {
        val previousKnowledge = _uiState.value.knowledgePoints
        val previousWiki = _uiState.value.conversationWiki
        _uiState.update {
            it.copy(knowledgePoints = knowledgePoints, conversationWiki = wiki)
        }
        val convId = _uiState.value.conversationId ?: return
        viewModelScope.launch {
            try {
                val committedRevision = conversationRepo.updateKnowledge(convId, knowledgePoints, wiki)
                _uiState.update { current ->
                    if (current.conversationId == convId) {
                        current.copy(configurationRevision = maxOf(current.configurationRevision, committedRevision))
                    } else current
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update { current ->
                    if (current.conversationId == convId) {
                        current.copy(
                            knowledgePoints = if (current.knowledgePoints == knowledgePoints) previousKnowledge else current.knowledgePoints,
                            conversationWiki = if (current.conversationWiki == wiki) previousWiki else current.conversationWiki,
                            error = error.localizedMessage ?: context.getString(R.string.chat_save_failed),
                        )
                    } else current
                }
            }
        }
    }

    fun updateLongTermMemory(memory: String) {
        _uiState.update { it.copy(longTermMemory = memory) }
    }

    fun updateUserPreferences(prefs: String) {
        _uiState.update { it.copy(userPreferences = prefs) }
    }

    fun toggleSkill(skill: UploadedContent, enabled: Boolean) {
        mutateSkillActivation(skill, deleting = false) { snapshot ->
            memoryProfileDataStore.setSkillEnabled(snapshot, enabled)
        }
    }

    fun deleteSkill(skill: UploadedContent) {
        mutateSkillActivation(skill, deleting = true) { snapshot ->
            memoryProfileDataStore.deleteSkill(snapshot)
        }
    }

    private val skillMutationMutex = kotlinx.coroutines.sync.Mutex()

    private fun mutateSkillActivation(
        skill: UploadedContent,
        deleting: Boolean,
        operation: suspend (com.nola.data.local.AndroidApplicationRuntime.SkillEditorSnapshot) -> Unit,
    ) {
        viewModelScope.launch {
            skillMutationMutex.withLock {
                try {
                    val principal = memoryProfileDataStore.getOrCreateResourcePrincipalId()
                    val access = com.nola.core.AndroidManagedResourceAccess.project(skill, principal)
                    check(if (deleting) access.canDelete else access.canEditContent) { "Skill operation is not permitted" }
                    val snapshot = memoryProfileDataStore.skillEditor(skill)
                    operation(snapshot)
                    val committed = memoryProfileDataStore.skills()
                    _uiState.update { it.copy(uploadedSkills = committed, error = null) }
                } catch (error: Exception) {
                    if (error is kotlinx.coroutines.CancellationException) throw error
                    _uiState.update { it.copy(error = error.localizedMessage ?: error.javaClass.simpleName) }
                }
            }
        }
    }

    suspend fun saveSkill(original: UploadedContent?, name: String, description: String, content: String) {
        skillMutationMutex.withLock {
            require(name.isNotBlank() && content.isNotBlank())
            val principal = memoryProfileDataStore.getOrCreateResourcePrincipalId()
            if (original != null) {
                check(com.nola.core.AndroidManagedResourceAccess.project(original, principal).canEditContent)
            }
            val edited = (original ?: UploadedContent(
                name = name.trim(),
                description = description.trim(),
                content = content,
                resourceId = java.util.UUID.randomUUID().toString(),
                accessOwnerId = principal,
            )).copy(name = name.trim(), description = description.trim(), content = content)
            memoryProfileDataStore.saveSkill(original?.let(memoryProfileDataStore::skillEditor), edited)
            val committed = memoryProfileDataStore.skills()
            _uiState.update { it.copy(uploadedSkills = committed, error = null) }
        }
    }

    fun refreshDataSourceCatalog() {
        viewModelScope.launch {
            healthConnectionState = readHealthConnectionState()
            val profile = chatRepo.getMemoryProfile()
            _uiState.update { state ->
                state.copy(
                    mcpServers = profile.mcpServers,
                    dataSources = buildDataSources(
                        uploadedDocuments = state.uploadedDocuments,
                        hostUploadedDocuments = state.hostUploadedDocuments,
                        mcpServers = profile.mcpServers,
                        hostMcpServers = state.hostMcpServers,
                        customDataSources = profile.customDataSources
                    ),
                    customDataSources = profile.customDataSources
                )
            }
        }
    }

    suspend fun compactNow() {
        val state = _uiState.value
        if (state.processingState.blocksInput) return
        val convId = state.conversationId ?: return
        val isRemote = state.currentModel?.source == ModelInfo.ModelSource.WebHost || state.executionHostBinding != null
        try {
            val result = withContext(Dispatchers.IO) {
                if (isRemote) {
                    val cfg = state.executionHostBinding?.let { resolveBoundHost(it) }
                        ?: remoteObservationConfiguration
                        ?: webAgentProvider.config
                        ?: error("Web Agent Host is not configured")
                    webAgentProvider.v2CompactConversation(state.v2SessionId ?: convId, cfg)
                } else {
                    applicationRuntime.v2CompactConversation(convId)
                }
            }
            val explicitSummary = result["summary"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
            val tokensBefore = result["tokens_before"]?.jsonPrimitive?.contentOrNull
            val tokensAfter = result["tokens_after"]?.jsonPrimitive?.contentOrNull
            val summary = explicitSummary ?: when {
                tokensBefore != null && tokensAfter != null -> context.getString(R.string.chat_v2_context_compacted_range, tokensBefore, tokensAfter)
                tokensAfter != null -> context.getString(R.string.chat_v2_context_compacted_after, tokensAfter)
                else -> context.getString(R.string.chat_v2_context_compacted)
            }
            _uiState.update { it.copy(v2CompactionSummary = summary, error = null) }
            reloadContextState()
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            _uiState.update { it.copy(error = error.localizedMessage ?: error.message ?: context.getString(R.string.chat_v2_context_compaction_failed)) }
        }
    }

    fun dismissV2CompactionSummary() {
        _uiState.update { it.copy(v2CompactionSummary = null) }
    }

    fun branchConversation() {
        val state = _uiState.value
        if (state.processingState.blocksInput) return
        val convId = state.conversationId ?: return
        val isRemote = state.currentModel?.source == ModelInfo.ModelSource.WebHost || state.executionHostBinding != null
        viewModelScope.launch {
            try {
                if (isRemote) {
                    val remoteConvId = state.v2SessionId?.takeIf { it.isNotBlank() } ?: convId
                    val cfg = withContext(Dispatchers.IO) {
                        state.executionHostBinding?.let { resolveBoundHost(it) }
                            ?: remoteObservationConfiguration
                            ?: webAgentProvider.config
                            ?: error("Web Agent Host is not configured")
                    }
                    val result = withContext(Dispatchers.IO) {
                        webAgentProvider.v2BranchConversation(remoteConvId, cfg)
                    }
                    val branchedObj = result["conversation"] as? JsonObject
                    val branchedRemoteId = branchedObj?.get("id")?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                        ?: result["conversationId"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                        ?: result["id"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                        ?: error("WebHost returned no branched conversation ID")
                    val sourceTitle = state.conversations.firstOrNull { it.id == convId }?.title?.takeIf { it.isNotBlank() } ?: "Chat"
                    val branchedTitle = branchedObj?.get("title")?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                        ?: "$sourceTitle (copy)"
                    val localCopy = withContext(Dispatchers.IO) {
                        conversationRepo.createConversation(
                            title = branchedTitle,
                            modelInfo = state.currentModel,
                            hostBinding = state.executionHostBinding,
                        )
                    }
                    rememberHostSession(cfg.expectedHostId, localCopy.id, branchedRemoteId)
                    loadConversations()
                    loadConversation(localCopy.id)
                } else {
                    val result = withContext(Dispatchers.IO) {
                        applicationRuntime.v2BranchConversation(convId)
                    }
                    val newId = result["id"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                        ?: (result["conversation"] as? JsonObject)?.get("id")?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                        ?: result["conversation_id"]?.jsonPrimitive?.contentOrNull?.takeIf { it.isNotBlank() }
                    loadConversations()
                    if (newId != null) {
                        loadConversation(newId)
                    }
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update {
                    it.copy(error = error.localizedMessage ?: error.message ?: context.getString(R.string.chat_v2_branch_failed))
                }
            }
        }
    }

    fun openWorkspaceReview() {
        val state = _uiState.value
        val convId = state.conversationId ?: return
        _uiState.update { it.copy(v2WorkspaceReview = V2WorkspaceReviewState(loading = true)) }
        viewModelScope.launch {
            try {
                val cfg = withContext(Dispatchers.IO) {
                    state.executionHostBinding?.let { resolveBoundHost(it) }
                        ?: remoteObservationConfiguration
                        ?: webAgentProvider.config
                }
                if (cfg == null) {
                    _uiState.update {
                        it.copy(v2WorkspaceReview = V2WorkspaceReviewState(error = context.getString(R.string.chat_v2_workspace_review_host_required)))
                    }
                    return@launch
                }
                val remoteConvId = state.v2SessionId?.takeIf { it.isNotBlank() } ?: convId
                val raw = withContext(Dispatchers.IO) {
                    webAgentProvider.v2WorkspaceReview(remoteConvId, cfg)
                }
                _uiState.update { it.copy(v2WorkspaceReview = parseV2WorkspaceReview(raw)) }
            } catch (cancelled: kotlinx.coroutines.CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                _uiState.update {
                    it.copy(
                        v2WorkspaceReview = V2WorkspaceReviewState(
                            error = error.localizedMessage ?: error.message ?: context.getString(R.string.chat_v2_workspace_review_empty)
                        )
                    )
                }
            }
        }
    }

    fun dismissWorkspaceReview() {
        _uiState.update { it.copy(v2WorkspaceReview = null) }
    }

    suspend fun truncateAndResendV2Message(message: ConversationMessage, editedText: String): Boolean {
        val state = _uiState.value
        val convId = state.conversationId ?: return false
        val trimmed = editedText.trim()
        val boundaryIndex = state.messages.indexOfFirst { it.id == message.id }
        if (trimmed.isEmpty() || state.processingState.blocksInput || state.executionTeardownPending ||
            message.conversationId != convId || message.role != ChatRole.user || boundaryIndex < 0) return false
        val sourceChanged = context.getString(R.string.chat_edit_source_changed)
        return try {
            val cfg = state.executionHostBinding?.let { resolveBoundHost(it) }
                ?: checkNotNull(remoteObservationConfiguration)
            val remoteId = readHostSession(cfg.expectedHostId, convId) ?: checkNotNull(state.v2SessionId)
            val history = checkNotNull(webAgentProvider.v2ChatHistory(convId, remoteId, cfg, checkExists = true)) { sourceChanged }
            check(!history.active && history.messages.size == state.messages.size &&
                history.messages.zip(state.messages).all { (remote, shown) -> remote.role == shown.role && remote.text == shown.text }) { sourceChanged }
            val target = history.messages[boundaryIndex]
            val seq = checkNotNull(target.meta?.get("nola.v2Seq")?.toLongOrNull()) { sourceChanged }
            check(message.meta?.get("nola.v2Seq")?.let { it == seq.toString() && message.id == target.id } != false) { sourceChanged }
            check(_uiState.value.conversationId == convId && !_uiState.value.processingState.blocksInput) { sourceChanged }
            webAgentProvider.v2TruncateConversation(remoteId, seq, cfg)
            if (_uiState.value.conversationId != convId) {
                composerDrafts.forConversation(convId).restoreRetryTextIfEmpty(trimmed)
                return false
            }
            _uiState.update { it.copy(messages = history.messages.take(boundaryIndex), error = null) }
            sendMessage(trimmed).also { accepted ->
                if (!accepted) composerDrafts.forConversation(convId).restoreRetryTextIfEmpty(trimmed)
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            if (_uiState.value.conversationId == convId) {
                _uiState.update { it.copy(error = error.localizedMessage ?: context.getString(R.string.chat_v2_edit_resend_failed)) }
            }
            false
        }
    }

    fun reloadContextState() {
        val convId = _uiState.value.conversationId
        if (convId != null) {
            loadConversation(convId)
        }
    }

    suspend fun searchMemoryForContext(query: String): List<MemorySearchHit> {
        val state = _uiState.value
        val convId = state.conversationId ?: return emptyList()
        val conv = conversationRepo.getConversation(convId) ?: return emptyList()
        val archived = conversationRepo.getArchivedMessages(convId)
        val normalized = query.trim()
        val conversationKnowledge = buildList {
            if (normalized.isNotEmpty() && conv.knowledgePoints.contains(normalized, ignoreCase = true)) {
                add(MemorySearchHit("conversation_knowledge", conv.knowledgePoints.take(600), 1f))
            }
            if (normalized.isNotEmpty() && conv.wiki.contains(normalized, ignoreCase = true)) {
                add(MemorySearchHit("conversation_wiki", conv.wiki.take(600), 1f))
            }
        }
        return conversationKnowledge + contextRepo.searchMemory(
            query,
            conv.sessionMemory,
            conv.compactedMemoryEntries,
            archived
        )
    }

    private val documentMutationMutex = kotlinx.coroutines.sync.Mutex()

    fun deleteDocument(document: UploadedContent) {
        viewModelScope.launch {
            try {
                mutateDocuments { documents ->
                    val id = requireNotNull(document.resourceId) { "Document identity is missing" }
                    require(documents.singleOrNull { it.resourceId == id } == document) { "Reload the document before deleting" }
                    documents.filterNot { it.resourceId == id }
                }
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                _uiState.update { it.copy(error = error.localizedMessage ?: error.javaClass.simpleName) }
            }
        }
    }

    suspend fun addDocument(document: UploadedContent) {
        mutateDocuments { documents ->
            documents + document.copy(resourceId = document.resourceId ?: java.util.UUID.randomUUID().toString())
        }
    }

    private suspend fun mutateDocuments(transform: (List<UploadedContent>) -> List<UploadedContent>) {
        documentMutationMutex.withLock {
            val baseline = documentEditorSnapshot
                ?: throw IllegalStateException(context.getString(R.string.chat_save_failed))
            val committed = withContext(Dispatchers.IO) {
                memoryProfileDataStore.saveDocuments(baseline, transform(baseline.documents))
            }
            documentEditorSnapshot = committed
            _uiState.update {
                it.copy(
                    uploadedDocuments = committed.documents,
                    error = null,
                    dataSources = buildDataSources(
                        uploadedDocuments = committed.documents,
                        hostUploadedDocuments = it.hostUploadedDocuments,
                        mcpServers = it.mcpServers,
                        hostMcpServers = it.hostMcpServers,
                    ),
                )
            }
        }
    }

    private fun buildDataSources(
        uploadedDocuments: List<UploadedContent> = emptyList(),
        hostUploadedDocuments: List<UploadedContent> = emptyList(),
        mcpServers: List<McpServerConfig> = emptyList(),
        hostMcpServers: List<McpServerConfig> = emptyList(),
        customDataSources: List<CustomDataSourceConfig> = _uiState.value.customDataSources
    ): List<UnifiedDataSource> = DataSourceCatalogBuilder.forChatContext(
        uploadedDocuments = uploadedDocuments,
        hostUploadedDocuments = hostUploadedDocuments,
        mcpServers = mcpServers,
        hostMcpServers = hostMcpServers,
        healthConnectionState = healthConnectionState,
        customDataSources = customDataSources
    )

    private val sourceSelectionMutex = kotlinx.coroutines.sync.Mutex()

    fun setConversationDataSourceEnabled(sourceId: String, enabled: Boolean) {
        applyConversationDataSourceOverride(sourceId, enabled)
    }

    fun restoreConversationDataSourceDefault(sourceId: String) {
        applyConversationDataSourceOverride(sourceId, null)
    }

    private fun applyConversationDataSourceOverride(sourceId: String, enabled: Boolean?) {
        val conversationId = _uiState.value.conversationId ?: return
        viewModelScope.launch {
            sourceSelectionMutex.withLock {
                try {
                    val saved = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                        conversationRepo.setDataSourceEnabled(conversationId, sourceId, enabled)
                        requireNotNull(conversationRepo.getConversation(conversationId))
                    }
                    _uiState.update { state ->
                        if (state.conversationId == conversationId) state.copy(
                            selectedDataSourceIds = saved.dataSourceIds,
                            configurationRevision = maxOf(state.configurationRevision, saved.configurationRevision),
                        )
                        else state
                    }
                } catch (error: kotlinx.coroutines.CancellationException) {
                    throw error
                } catch (error: Exception) {
                    _uiState.update { state ->
                        if (state.conversationId == conversationId) state.copy(error = error.localizedMessage ?: error.javaClass.simpleName)
                        else state
                    }
                }
            }
        }
    }

    private suspend fun readHealthConnectionState(): UnifiedDataSourceConnectionState {
        return try {
            when (healthDataSource.availability()) {
                AndroidHealthAvailability.Available -> {
                    if (healthDataSource.hasReadPermission()) {
                        UnifiedDataSourceConnectionState.Ready
                    } else {
                        UnifiedDataSourceConnectionState.NeedsPermission
                    }
                }
                AndroidHealthAvailability.ProviderUpdateRequired -> UnifiedDataSourceConnectionState.Error
                AndroidHealthAvailability.Unavailable -> UnifiedDataSourceConnectionState.Unavailable
            }
        } catch (_: Exception) {
            UnifiedDataSourceConnectionState.Error
        }
    }

    private var remoteUiForeground = true
    private var remoteUiWakeJob: kotlinx.coroutines.Job? = null
    private var remoteUiWakeConversation: String? = null
    private val remoteUiWakeSuppressed = mutableSetOf<String>()
    private val rejectedRemoteUiActions = mutableMapOf<String, com.nola.data.remote.api.RemoteSubmissionReceipt>()

    private fun wakeRemoteUiQueue(conversationId: String) {
        if (conversationId in remoteUiWakeSuppressed) {
            rejectedRemoteUiActions[conversationId]?.let { receipt ->
                if (_uiState.value.conversationId == conversationId) _uiState.update { it.copy(rejectedQueuedUiAction = receipt) }
            }
            return
        }
        if (remoteUiWakeJob?.isActive == true && remoteUiWakeConversation == conversationId) return
        remoteUiWakeJob?.cancel()
        remoteUiWakeConversation = conversationId
        remoteUiWakeJob = viewModelScope.launch {
            try {
                fun eligible() = remoteUiForeground && conversationId !in remoteUiWakeSuppressed &&
                    _uiState.value.conversationId == conversationId &&
                    _uiState.value.currentModel?.source == ModelInfo.ModelSource.WebHost &&
                    !_uiState.value.processingState.blocksInput &&
                    runRegistry.find(composerDrafts.ownerKey(conversationId))?.runtime?.hasOutstandingGeneration() != true
                var dispatchWitness: Pair<com.nola.data.remote.api.RemoteSubmissionIdentity, String>? = null
                while (eligible()) {
                    if (!eligible()) return@launch
                    val intent = conversationRepo.readRemoteSubmissionIntent(conversationId)
                    val conversation = conversationRepo.getConversation(conversationId) ?: return@launch
                    if (conversation.executionBackend != "nola") return@launch
                    val configuration = resolveBoundHost(checkNotNull(conversation.executionHostBinding))
                    val epoch = applicationRuntime.remoteUiEpoch(conversationId, configuration.expectedHostId) ?: return@launch
                    val queue = webAgentProvider.idleUiQueueHead(conversationId, configuration, epoch)
                    if (!eligible()) return@launch
                    val head = queue.head
                    if (head == null) {
                        _uiState.update { it.copy(rejectedQueuedUiAction = null, cancellingQueuedUiAction = false) }
                        return@launch
                    }
                    val dispatch = queue.dispatch
                    val witness = dispatch?.let { it.identity to it.requestDigest }
                    require(dispatchWitness == null || dispatchWitness == witness) { "Queued dispatch identity changed" }
                    if (witness != null) dispatchWitness = witness
                    if (dispatch != null) {
                        when (dispatch.phase) {
                            com.nola.data.remote.api.RemoteSubmissionPhase.Prepared -> {
                                // Read the exact frozen receipt again; never construct another tool plan/request.
                                kotlinx.coroutines.delay(1000); continue
                            }
                            com.nola.data.remote.api.RemoteSubmissionPhase.Admitted -> {
                                remoteObservationConfiguration = configuration
                                remoteObservationInitialKey = com.nola.data.remote.api.RemoteRunKey(conversationId,
                                    dispatch.identity.submissionId, dispatch.identity.assistantMessageId)
                                startRemoteConversationObservation(conversationId)
                                return@launch
                            }
                            com.nola.data.remote.api.RemoteSubmissionPhase.Rejected,
                            com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled -> {
                                remoteUiWakeSuppressed.add(conversationId)
                                rejectedRemoteUiActions[conversationId] = dispatch
                                if (eligible() || _uiState.value.conversationId == conversationId)
                                    _uiState.update { it.copy(rejectedQueuedUiAction = dispatch, cancellingQueuedUiAction = false) }
                                return@launch
                            }
                            else -> error("Invalid queued dispatch phase")
                        }
                    }
                    if (intent?.phase in setOf(com.nola.data.remote.api.RemoteSubmissionPhase.Prepared,
                            com.nola.data.remote.api.RemoteSubmissionPhase.Bound)) return@launch
                    if (!eligible() || head.kind != "user_input" || head.user_message_id != "ui:${head.command_id}:user") return@launch
                    val leaseUntil = head.lease_until_ms
                    val now = System.currentTimeMillis()
                    if (leaseUntil != null && leaseUntil > now) {
                        // Re-read authority after waiting; never claim from this stale hint.
                        kotlinx.coroutines.delay(if (leaseUntil - now > 30_000L) 30_000L else (leaseUntil - now).coerceAtLeast(250L))
                        continue
                    }
                    val current = conversationRepo.getConversation(conversationId) ?: return@launch
                    if (!eligible() || current.configurationRevision != conversation.configurationRevision ||
                        current.executionHostBinding != conversation.executionHostBinding ||
                        resolveBoundHost(checkNotNull(current.executionHostBinding)) != configuration) return@launch
                    sendMessage("表单操作", submissionIdentity = head.command_id, remoteQueuedCommandId = head.command_id,
                        remoteQueuedConfiguration = configuration)
                    return@launch
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (_uiState.value.conversationId == conversationId) _uiState.update { it.copy(error = error.message) }
            }
        }
    }

    fun cancelRejectedQueuedUiAction() {
        val receipt = _uiState.value.rejectedQueuedUiAction ?: return
        if (_uiState.value.cancellingQueuedUiAction) return
        val conversationId = receipt.identity.conversationId
        viewModelScope.launch {
            _uiState.update { it.copy(cancellingQueuedUiAction = true) }
            try {
                val conversation = conversationRepo.getConversation(conversationId) ?: error("Conversation missing")
                val configuration = resolveBoundHost(checkNotNull(conversation.executionHostBinding))
                val epoch = checkNotNull(applicationRuntime.remoteUiEpoch(conversationId, configuration.expectedHostId))
                webAgentProvider.cancelQueuedUiInput(receipt, configuration, epoch)
                com.nola.data.remote.api.clearCancelledUiQueueOwner(receipt, rejectedRemoteUiActions, remoteUiWakeSuppressed)
                if (_uiState.value.conversationId == conversationId && _uiState.value.rejectedQueuedUiAction == receipt) {
                    _uiState.update { it.copy(rejectedQueuedUiAction = null, cancellingQueuedUiAction = false) }
                    wakeRemoteUiQueue(conversationId)
                }
            } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) {
                if (_uiState.value.conversationId == conversationId)
                    _uiState.update { it.copy(cancellingQueuedUiAction = false, error = error.message) }
            }
        }
    }

    private suspend fun wakeIdleQueuedUiInput(conversationId: String) {
        val state = _uiState.value
        val owner = runRegistry.find(composerDrafts.ownerKey(conversationId))
        if (state.conversationId != conversationId ||
            state.currentModel?.source == ModelInfo.ModelSource.WebHost ||
            state.processingState.blocksInput || state.permissionSaving || state.permissionSaveFailed ||
            state.unknownEffectRecovery != null || owner?.successorCancellationRequested == true ||
            owner?.runtime?.hasOutstandingGeneration() == true) return
        val next = withContext(Dispatchers.IO) {
            if (applicationRuntime.execute(buildJsonObject {
                put("type", "active_run"); put("conversation_id", conversationId)
            }) != JsonNull) return@withContext null
            val promoted = applicationRuntime.execute(buildJsonObject {
                put("type", "promote_queued_assistant_turn")
                put("input", buildJsonObject {
                    put("conversation_id", conversationId); put("created_at_ms", System.currentTimeMillis())
                })
            })
            if (promoted != JsonNull) return@withContext promoted.jsonObject.getValue("begin").jsonObject to true
            val head = applicationRuntime.execute(buildJsonObject {
                put("type", "peek_agent_command"); put("session_id", conversationId)
            }) as? JsonObject ?: return@withContext null
            head to false
        } ?: return
        if (_uiState.value.conversationId != conversationId) return
        if (next.second) {
            require(next.first.getValue("conversation_id").jsonPrimitive.content == conversationId)
            val runId = next.first.getValue("run_id").jsonPrimitive.content
            val conversation = conversationRepo.getConversation(conversationId) ?: return
            if (_uiState.value.conversationId == conversationId) {
                resumeExistingDeviceRun(conversation, runId)
            }
            return
        }
        val head = next.first
        val commandId = head["command_id"]?.jsonPrimitive?.content ?: return
        if (head["kind"]?.jsonPrimitive?.content != "user_input" ||
            head["user_message_id"]?.jsonPrimitive?.content != "ui:$commandId:user") return
        dispatchQueuedUiInput(conversationId, commandId)
    }

    private suspend fun dispatchQueuedUiInput(conversationId: String, commandId: String) {
        val state = _uiState.value
        if (state.conversationId != conversationId || state.currentModel?.source == ModelInfo.ModelSource.WebHost ||
            state.processingState.blocksInput || state.permissionSaving || state.permissionSaveFailed ||
            runRegistry.find(composerDrafts.ownerKey(conversationId))?.runtime?.hasOutstandingGeneration() == true) return
        var claimedOwner: String? = null
        try {
            val pending = withContext(Dispatchers.IO) {
                val active = applicationRuntime.execute(buildJsonObject {
                    put("type", "active_run"); put("conversation_id", conversationId)
                })
                if (active != JsonNull) return@withContext null
                val head = applicationRuntime.execute(buildJsonObject {
                    put("type", "peek_agent_command"); put("session_id", conversationId)
                }) as? JsonObject ?: return@withContext null
                if (head["command_id"]?.jsonPrimitive?.content != commandId) return@withContext null
                val owner = "android-ui:${UUID.randomUUID()}"
                val now = System.currentTimeMillis()
                val lease = applicationRuntime.execute(buildJsonObject {
                    put("type", "claim_next_agent_command"); put("session_id", conversationId)
                    put("lease_owner", owner); put("now_ms", now); put("lease_until_ms", now + 300_000)
                }) as? JsonObject ?: return@withContext null
                require(lease.getValue("command_id").jsonPrimitive.content == commandId &&
                    lease.getValue("session_id").jsonPrimitive.content == conversationId &&
                    lease.getValue("lease_owner").jsonPrimitive.content == owner)
                claimedOwner = owner
                val envelope = Json.parseToJsonElement(lease.getValue("payload_json").jsonPrimitive.content).jsonObject
                val submission = envelope.getValue("submission").jsonObject
                require(envelope.getValue("type").jsonPrimitive.content == "submission" &&
                    submission.getValue("type").jsonPrimitive.content == "user_input" &&
                    submission.getValue("submission_id").jsonPrimitive.content == commandId)
                val (messageId, summary) = com.nola.core.queuedUiInputPresentation(submission, conversationId, commandId)
                com.nola.core.AndroidQueuedUserAdmission(commandId, owner,
                    lease.getValue("delivery_attempt").jsonPrimitive.content.toLong(), messageId) to summary
            } ?: return
            if (_uiState.value.conversationId != conversationId) {
                withContext(Dispatchers.IO) { applicationRuntime.execute(buildJsonObject {
                    put("type", "release_agent_command_lease"); put("session_id", conversationId)
                    put("command_id", commandId); put("lease_owner", pending.first.leaseOwner)
                }) }
                return
            }
            sendMessage(pending.second, submissionIdentity = commandId, queuedAdmission = pending.first)
            if (runRegistry.find(composerDrafts.ownerKey(conversationId))?.runtime?.hasOutstandingGeneration() != true) {
                releaseQueuedUiLease(conversationId, pending.first)
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            claimedOwner?.let { owner ->
                withContext(kotlinx.coroutines.NonCancellable + Dispatchers.IO) {
                    runCatching { applicationRuntime.execute(buildJsonObject {
                        put("type", "release_agent_command_lease"); put("session_id", conversationId)
                        put("command_id", commandId); put("lease_owner", owner)
                    }) }
                }
            }
            throw cancelled
        } catch (error: Exception) {
            claimedOwner?.let { owner ->
                withContext(Dispatchers.IO) { runCatching { applicationRuntime.execute(buildJsonObject {
                    put("type", "release_agent_command_lease"); put("session_id", conversationId)
                    put("command_id", commandId); put("lease_owner", owner)
                }) } }
            }
            if (_uiState.value.conversationId == conversationId) _uiState.update { it.copy(error = error.message) }
        }
    }

    private suspend fun releaseQueuedUiLease(conversationId: String, admission: com.nola.core.AndroidQueuedUserAdmission) {
        withContext(kotlinx.coroutines.NonCancellable + Dispatchers.IO) {
            // Owner-fenced release is a no-op once BeginQueuedUserTurn consumed
            // the command. Failed preparation/cancellation leaves it recoverable.
            runCatching { applicationRuntime.execute(buildJsonObject {
                put("type", "release_agent_command_lease"); put("session_id", conversationId)
                put("command_id", admission.commandId); put("lease_owner", admission.leaseOwner)
            }) }
        }
    }

    private val _messageEditDraft = MutableStateFlow<AndroidMessageEditDraft?>(null)
    val messageEditDraft: StateFlow<AndroidMessageEditDraft?> = _messageEditDraft.asStateFlow()

    fun beginMessageEdit(message: ConversationMessage) {
        if (_messageEditDraft.value?.busy == true || _uiState.value.processingState.blocksInput) return
        if (_uiState.value.currentModel?.source == ModelInfo.ModelSource.WebHost) return
        viewModelScope.launch {
            try {
                val source = message.conversationId
                val canonicalId = checkNotNull(message.meta?.get("nola.canonicalMessageID")) { "The original message identity is unavailable" }
                val expectedRevision = checkNotNull(message.meta?.get("nola.canonicalMessageRevision")?.toLongOrNull())
                val facts = withContext(Dispatchers.IO) {
                    val raw = checkNotNull(applicationRuntime.loadConversation(source))
                    val projection = applicationRuntime.loadConversationProjectionV1(source)
                    check(projection.activeRun == null && projection.taskChildOwner == null) { "Wait for the current Run to finish before editing" }
                    val original = projection.messages.single { it.messageId == canonicalId }
                    check(original.role == ConversationProjectionV1.Role.User && original.state == ConversationProjectionV1.MessageState.Completed &&
                        original.revision == expectedRevision && original.parts.size == 1)
                    val part = original.parts.single()
                    val text = when (part) {
                        is ConversationProjectionV1.Part.Text -> part.markdown.takeIf { part.isFinal }
                        is ConversationProjectionV1.Part.Historical -> part.projection.let { p ->
                            p.canonicalText?.takeIf { p.canonicalKind == "text" && p.schema == null &&
                                p.owner["conversation_id"] == JsonPrimitive(source) && p.owner["message_id"] == JsonPrimitive(canonicalId) &&
                                p.owner["part_id"] == JsonPrimitive(part.partId) && p.canonicalTextUTF8Bytes == it.toByteArray(Charsets.UTF_8).size.toLong() }
                        }
                        else -> null
                    }
                    check(text != null && text == message.text) { "The original message changed; reload it before editing" }
                    raw.revision
                }
                val remote = withContext(Dispatchers.IO) { applicationRuntime.conversationSourceOwner(source) } as? com.nola.data.local.ConversationSourceOwner.Remote
                val pending = withContext(Dispatchers.IO) { applicationRuntime.expertHostFactory.pendingEdit(source, canonicalId) }
                _messageEditDraft.value = AndroidMessageEditDraft(pending?.input ?: AndroidPrepareEditedResend(
                    UUID.randomUUID().toString(), source, canonicalId, facts, expectedRevision, message.text,
                    UUID.randomUUID().toString(), context.getString(R.string.chat_edit_resend), System.currentTimeMillis()), message.text,
                    mentions = historyMentions(message.text, message.expertMentionHistory()).map { it.copy(host = remote?.hostId, epoch = remote?.storeEpoch) },
                    outcomePending = pending != null,
                    progress = if (pending != null) context.getString(R.string.expert_host_awaiting_confirmation) else null)
            } catch (failure: Exception) {
                _uiState.update { it.copy(error = failure.message ?: "The original message is unavailable") }
            }
        }
    }

    fun updateMessageEditText(text: String, mentions: List<InlineExpertMention>) {
        _messageEditDraft.update { it?.takeUnless { it.busy || it.outcomePending }?.let { draft ->
            if (draft.input.editedText == text && draft.mentions == mentions) draft else draft.copy(mentions = mentions, input = draft.input.copy(editedText = text,
                operationId = UUID.randomUUID().toString(), targetConversationId = UUID.randomUUID().toString(),
                createdAtMs = System.currentTimeMillis()), error = null, progress = null) } ?: it }
    }

    fun cancelMessageEdit() {
        val draft = _messageEditDraft.value ?: return
        if (draft.busy || draft.outcomePending) return
        _messageEditDraft.value = null
        if (_uiState.value.conversationId != draft.input.sourceConversationId) loadConversation(draft.input.sourceConversationId)
    }

    fun submitMessageEdit() {
        val draft = _messageEditDraft.value ?: return
        if (draft.busy || draft.input.editedText.isBlank()) return
        if (mentionOnly(draft.input.editedText, draft.mentions)) {
            _messageEditDraft.value = draft.copy(error = context.getString(R.string.expert_mention_needs_text))
            return
        }
        if (draft.mentions.isNotEmpty()) {
            val model = _uiState.value.currentModel
            if (model == null || model.executionCapabilitiesDeclared && !model.executionCapabilities.toolCalls) {
                _messageEditDraft.value = draft.copy(error = context.getString(R.string.expert_main_requires_tools))
                return
            }
        }
        _messageEditDraft.value = draft.copy(busy = true, error = null, progress = null)
        viewModelScope.launch {
            var expertEdit = draft.outcomePending
            try {
                if (draft.outcomePending) {
                    performExpertMessageEdit(draft)
                    _messageEditDraft.value = null
                    return@launch
                }
                val selected = inlineSelection(draft.input.editedText, draft.mentions)
                val input = draft.input.inPlace().copy(selectedExperts = selected?.get("selected_experts")?.jsonArray,
                    expertSourceEpoch = selected?.get("expected_source_store_epoch")?.jsonPrimitive?.content,
                    expertMentions = selected?.get("expert_mentions")?.jsonObject)
                val existing = withContext(Dispatchers.IO) { applicationRuntime.loadRunIdentity(input.operationId) }
                if (existing != null) {
                    check(existing["conversation_id"] == JsonPrimitive(input.conversationId))
                    val prior = withContext(Dispatchers.IO) { applicationRuntime.loadPreparedInPlaceResend(input.operationId) }
                    check(prior?.input == input) { "This resend intent changed" }
                    _messageEditDraft.value = null; return@launch
                }
                val prepared = withContext(Dispatchers.IO) { applicationRuntime.prepareInPlaceResend(input) }
                check(prepared.input == input && prepared.runId == input.operationId)
                val owner = input.conversationId
                conversationLoads.invalidate()
                val priorMessages = withContext(Dispatchers.IO) { conversationRepo.getActiveMessages(owner) }
                val prefix = if (prepared.deferredRewind) {
                    val boundary = priorMessages.indexOfFirst { it.id == input.messageId }
                    check(boundary >= 0) { "The original message changed; reopen it before editing" }
                    priorMessages.take(boundary)
                } else priorMessages
                if (!prepared.deferredRewind) {
                    _uiState.update { current -> check(current.conversationId == owner); current.copy(messages = prefix) }
                }
                if (prepared.experts.isNotEmpty()) {
                    expertEdit = true
                    val manifest = withContext(Dispatchers.IO) {
                        val association = checkNotNull(agentRooms.forConversation(owner))
                        applicationRuntime.admitExpertMessage(AndroidAdmitExpertMessage(owner,
                            checkNotNull(applicationRuntime.loadConversation(owner)).revision,
                            association.room.room.id, association.room.revision, input.operationId, prepared.userMessageId,
                            input.editedText, prepared.experts, input.createdAtMs))
                    }
                    dispatchExpertManifest(manifest)
                    loadConversation(owner)
                } else {
                    val started = sendMessage(input.editedText, submissionIdentity = prepared.runId, canonicalUserMessageId = prepared.userMessageId, inlineMentions = draft.mentions)
                    if (!started) {
                        error(_uiState.value.error ?: "Resend was not admitted")
                    }
                    kotlinx.coroutines.withTimeout(30_000) {
                        while (withContext(Dispatchers.IO) { applicationRuntime.loadRunIdentity(prepared.runId) } == null) {
                            val currentState = _uiState.value
                            if (currentState.processingState == ProcessingState.Error ||
                                (currentState.processingState == ProcessingState.Idle && currentState.error != null)) {
                                error(currentState.error ?: "Resend was not admitted")
                            }
                            delay(50)
                        }
                    }
                    if (prepared.deferredRewind) {
                        // Begin committed the rewind together with the replacement. Drop only retired
                        // rows so a newer streaming update cannot be overwritten by this snapshot.
                        val retired = priorMessages.drop(prefix.size).mapTo(mutableSetOf()) { it.id }
                        _uiState.update { current -> if (current.conversationId != owner) current
                            else current.copy(messages = current.messages.filterNot { it.id in retired }) }
                    }
                }
                if (prefix.none { it.role == ChatRole.user }) {
                    updateFirstMessageTitle(owner, prepared.userMessageId, input.editedText)
                }
                _messageEditDraft.value = null
            } catch (cancel: kotlinx.coroutines.CancellationException) {
                if (cancel is kotlinx.coroutines.TimeoutCancellationException) {
                    if (expertEdit) _messageEditDraft.update { it?.copy(busy = false, error = null, outcomePending = true,
                        progress = context.getString(R.string.expert_host_awaiting_confirmation)) }
                    else _messageEditDraft.update { it?.copy(busy = false, error = listOfNotNull(it.progress, it.error,
                        "Delivery has not been confirmed. Retry this same edit to inspect its durable receipt.").joinToString("\n")) }
                } else throw cancel
            } catch (pending: AndroidApplicationRuntime.ExpertHostPending) {
                _messageEditDraft.update { it?.copy(busy = false, error = null, outcomePending = true,
                    progress = context.getString(if (pending.reopenRequired) R.string.expert_host_reopen_confirmation
                        else R.string.expert_host_awaiting_confirmation)) }
            } catch (failure: Exception) {
                _messageEditDraft.update { current -> current?.let {
                    if (it.outcomePending && failure !is AndroidApplicationRuntime.ExpertHostRejected)
                        it.copy(busy = false, error = null, progress = context.getString(R.string.expert_host_awaiting_confirmation))
                    else it.copy(busy = false, outcomePending = false, error = failure.message ?: "Edited resend was rejected")
                } }
            }
        }
    }

    private suspend fun performExpertMessageEdit(draft: AndroidMessageEditDraft) {
        val plan = withContext(Dispatchers.IO) {
            if (draft.outcomePending) applicationRuntime.prepareExpertResend(draft.input) else
            applicationRuntime.loadPreparedExpertResend(draft.input.targetConversationId)?.also {
                require(it.input == draft.input) { "This expert edit intent has already been prepared with different text" }
            } ?: applicationRuntime.prepareExpertResend(draft.input)
        }
        check(plan.input == draft.input && plan.entries.isNotEmpty())
        val manifest = withContext(Dispatchers.IO) {
            checkNotNull(applicationRuntime.expertMessageSource(plan.input.targetConversationId, plan.parentMessageId))
        }
        check(manifest.entries.size == plan.entries.size && manifest.entries.map { it.childSubmissionId }.toSet() ==
            plan.entries.map { it.prepared.runId }.toSet())
        dispatchExpertManifest(manifest)
        kotlinx.coroutines.withTimeout(30_000) {
            while (true) {
                val observed = withContext(Dispatchers.IO) { checkNotNull(applicationRuntime.loadPreparedExpertResend(plan.input.targetConversationId)) }
                check(observed.input == plan.input)
                val admitted = observed.entries.count { it.prepared.admittedRunId == it.prepared.runId }
                if (admitted == observed.entries.size) {
                    loadConversation(plan.input.targetConversationId)
                    selectedConversationAgentIds.value = plan.entries.map { it.participant.agentId }.toSet()
                    break
                }
                _messageEditDraft.update { it?.copy(progress = context.getString(R.string.chat_edit_expert_delivery_progress, admitted, observed.entries.size)) }
                delay(100)
            }
        }
    }

    fun sendMessage(
        text: String,
        messageMeta: Map<String, String>? = null,
        attachments: List<com.nola.domain.model.ModelAttachment> = emptyList(),
        submissionIdentity: String? = null,
        queuedAdmission: com.nola.core.AndroidQueuedUserAdmission? = null,
        remoteQueuedCommandId: String? = null,
        remoteQueuedConfiguration: WebAgentProvider.Config? = null,
        expertParticipantAgentIds: Set<String> = emptySet(),
        editedResend: AndroidPreparedEditedResend? = null,
        canonicalUserMessageId: String? = null,
        capturedExpertModel: com.nola.core.AndroidTurnModelSnapshot? = null,
        inlineMentions: List<InlineExpertMention> = emptyList(),
    ): Boolean {
        if (text.isBlank() && attachments.isEmpty()) return false
        val state = capturedExpertModel?.let { _uiState.value.copy(currentModel = it.model, currentParameters = it.parameters) } ?: _uiState.value
        if (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0) {
            val existing = runRegistry.find(composerDrafts.ownerKey(state.conversationId))
            Log.d("NolaAgentProtocol", "ui_send_received conversation=${state.conversationId} processing=${state.processingState} " +
                "preparing_picker=${preparingExpertPicker.value} permission_saving=${state.permissionSaving} permission_failed=${state.permissionSaveFailed} " +
                "child_owner=${state.taskChildOwner != null} model_present=${state.currentModel != null} " +
                "job_outstanding=${existing?.runtime?.hasOutstandingGeneration()} sealed=${existing?.hasCommittedTerminalSeal} " +
                "cancel_requested=${existing?.successorCancellationRequested}")
        }
        require(editedResend == null || (submissionIdentity == editedResend.runId &&
            state.conversationId == editedResend.input.targetConversationId && text == editedResend.input.editedText &&
            attachments.isEmpty() && messageMeta == null && queuedAdmission == null && remoteQueuedCommandId == null &&
            expertParticipantAgentIds.isEmpty()))
        if (state.taskChildOwner != null) return false
        require((remoteQueuedCommandId == null) == (remoteQueuedConfiguration == null))
        require(remoteQueuedCommandId == null || (remoteQueuedCommandId == submissionIdentity &&
            state.currentModel?.source == ModelInfo.ModelSource.WebHost && queuedAdmission == null && attachments.isEmpty()))
        if (preparingExpertPicker.value || state.processingState.blocksInput || state.permissionSaving || state.permissionSaveFailed) return false
        val draftOwner = if (submissionIdentity == null) composerDrafts.ownerKey(state.conversationId) else null
        val pendingExpertAdmission = draftOwner?.let { pendingExpertAdmissionByDraft.find(it, state.conversationId) }
        val selectedExpertIds = if (submissionIdentity == null) pendingExpertAdmission?.selectedAgentIds ?: expertParticipantAgentIds else emptySet()
        // Only reconcile already-persisted historical expert-only intents. New inputs always enter Main.
        if (selectedExpertIds.isNotEmpty() && pendingExpertAdmission != null) {
            if (attachments.isNotEmpty() || !messageMeta.isNullOrEmpty()) {
                _uiState.update { it.copy(error = "Expert-only messages currently support plain text only. Remove attachments and try again.") }
                return false
            }
            val owner = requireNotNull(draftOwner)
            if (!activeExpertAdmissionDrafts.add(owner)) return false
            val selectionChanged = pendingExpertAdmission != null && expertParticipantAgentIds != pendingExpertAdmission.selectedAgentIds
            if (pendingExpertAdmission != null) selectedConversationAgentIds.value = pendingExpertAdmission.selectedAgentIds
            _uiState.update { it.copy(processingState = ProcessingState.BuildingContext,
                error = if (selectionChanged) "Retrying this message with its original expert selection." else null) }
            viewModelScope.launch {
                try { admitAndFanOutExpertOnly(text, messageMeta, state, selectedExpertIds, owner) }
                catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (pendingOutcome: com.nola.data.local.AndroidExpertAdmissionOutcomePending) {
                    val pending = pendingOutcome.pending
                    pendingExpertAdmissionByDraft.restore(owner, pending.input)
                    _uiState.update { current ->
                        if (current.conversationId == state.conversationId || current.conversationId == pending.input.conversationId || current.conversationId == null)
                            current.copy(conversationId = pending.input.conversationId, processingState = ProcessingState.AwaitingRecovery,
                                pendingExpertHostAdmission = pending, checkingExpertHostAdmission = false,
                                expertHostConfirmationStatus = context.getString(if (pendingOutcome.reopenRequired)
                                    R.string.expert_host_reopen_confirmation else R.string.expert_host_awaiting_confirmation), error = null)
                        else current
                    }
                }
                catch (failure: Exception) {
                    _uiState.update { current -> if (current.conversationId == state.conversationId || current.conversationId == null)
                        current.copy(processingState = ProcessingState.Error, error = if (failure is ExpertSelectionChanged)
                            context.getString(R.string.expert_profile_changed) else failure.message ?: "Expert messages could not be confirmed") else current }
                } finally { activeExpertAdmissionDrafts.remove(owner) }
            }
            return true
        }
        val selectedExpertSource = if (inlineMentions.isEmpty()) expertPickerSource else inlineMentions.first().let { ref -> ref.host?.let { ExpertPickerSource(it, checkNotNull(ref.epoch)) } }
        val selectedExpertContext = runCatching {
            inlineSelection(text, inlineMentions) ?: mainExpertSelection(selectedExpertIds, selectedConversationAgentRevisions.toMap(), selectedConversationAgentParticipants.toMap())?.let { selection ->
                selectedExpertSource?.let { JsonObject(selection + ("expected_source_store_epoch" to JsonPrimitive(it.epoch))) } ?: selection
            }
        }.getOrElse { _uiState.update { it.copy(error = context.getString(R.string.expert_profile_changed)) }; return false }
        if (state.currentModel == null) {
            _uiState.update { it.copy(error = context.getString(R.string.chat_no_model_selected)) }
            return false
        }
        if (state.currentModel.source == ModelInfo.ModelSource.WebHost && attachments.isNotEmpty()) {
            _uiState.update { it.copy(error = context.getString(R.string.chat_web_host_images_unsupported)) }
            return false
        }
        if (selectedExpertContext != null && state.currentModel.source != ModelInfo.ModelSource.WebHost &&
            state.currentModel.executionCapabilitiesDeclared && !state.currentModel.executionCapabilities.toolCalls) {
            _uiState.update { it.copy(error = context.getString(R.string.expert_main_requires_tools)) }
            return false
        }
        val existingExpertAdmission = pendingExpertAdmissionByDraft.find(
            composerDrafts.ownerKey(state.conversationId), state.conversationId)
        if (existingExpertAdmission != null) {
            _uiState.update { it.copy(error = "An expert message is still awaiting confirmation. Restore its original text and select its experts to retry.") }
            return false
        }
        val targetGoalConversationId = state.conversationId
        if (submissionIdentity == null && editedResend == null && selectedExpertIds.isEmpty() && attachments.isEmpty() &&
            targetGoalConversationId != null && state.currentModel?.source != ModelInfo.ModelSource.WebHost) {
            val capturedDraft = composerDrafts.captureSubmission(composerDrafts.ownerKey(targetGoalConversationId))
            if (activeGoalReplies.add(targetGoalConversationId)) {
                viewModelScope.launch {
                    try {
                        val goalStatus = withContext(Dispatchers.IO) {
                            runCatching { conversationProgress.load(targetGoalConversationId).status }
                                .onFailure { if (it is kotlinx.coroutines.CancellationException) throw it }
                                .getOrNull()
                        }
                        if (goalStatus in setOf("needs_user", "paused")) {
                            val ticket = composerDrafts.prepareSubmission(capturedDraft)
                            val result = stepLocalGoal(targetGoalConversationId, userPrompt = text)
                            composerDrafts.submitted(ticket)
                            val updatedMessages = withContext(Dispatchers.IO) {
                                conversationRepo.getMessages(targetGoalConversationId)
                            }
                            runRegistry.find(composerDrafts.ownerKey(targetGoalConversationId))?.state?.update {
                                it.copy(messages = updatedMessages, error = null)
                            }
                            if (displayedState.value.conversationId == targetGoalConversationId) {
                                displayedState.update { it.copy(messages = updatedMessages, error = null) }
                            }
                            wakeLocalGoal(targetGoalConversationId, result)
                            return@launch
                        }
                    } catch (cancelled: kotlinx.coroutines.CancellationException) {
                        throw cancelled
                    } catch (failure: Exception) {
                        runRegistry.find(composerDrafts.ownerKey(targetGoalConversationId))?.state?.update {
                            it.copy(error = failure.message)
                        }
                        if (displayedState.value.conversationId == targetGoalConversationId) {
                            displayedState.update { it.copy(error = failure.message) }
                        }
                        return@launch
                    } finally {
                        activeGoalReplies.remove(targetGoalConversationId)
                    }
                    if (displayedState.value.conversationId != targetGoalConversationId) {
                        return@launch
                    }
                    dispatchSendMessage(
                        text, messageMeta, attachments, submissionIdentity, queuedAdmission,
                        remoteQueuedCommandId, remoteQueuedConfiguration, expertParticipantAgentIds,
                        editedResend, canonicalUserMessageId, capturedExpertModel, inlineMentions,
                        expectedConversationId = targetGoalConversationId,
                    )
                }
                return true
            }
            return false
        }
        return dispatchSendMessage(
            text, messageMeta, attachments, submissionIdentity, queuedAdmission,
            remoteQueuedCommandId, remoteQueuedConfiguration, expertParticipantAgentIds,
            editedResend, canonicalUserMessageId, capturedExpertModel, inlineMentions,
        )
    }

    private fun dispatchSendMessage(
        text: String,
        messageMeta: Map<String, String>? = null,
        attachments: List<com.nola.domain.model.ModelAttachment> = emptyList(),
        submissionIdentity: String? = null,
        queuedAdmission: com.nola.core.AndroidQueuedUserAdmission? = null,
        remoteQueuedCommandId: String? = null,
        remoteQueuedConfiguration: WebAgentProvider.Config? = null,
        expertParticipantAgentIds: Set<String> = emptySet(),
        editedResend: AndroidPreparedEditedResend? = null,
        canonicalUserMessageId: String? = null,
        capturedExpertModel: com.nola.core.AndroidTurnModelSnapshot? = null,
        inlineMentions: List<InlineExpertMention> = emptyList(),
        expectedConversationId: String? = null,
    ): Boolean {
        val state = capturedExpertModel?.let { _uiState.value.copy(currentModel = it.model, currentParameters = it.parameters) } ?: _uiState.value
        if (expectedConversationId != null && state.conversationId != expectedConversationId) {
            return false
        }
        val draftOwner = if (submissionIdentity == null) composerDrafts.ownerKey(state.conversationId) else null
        val pendingExpertAdmission = draftOwner?.let { pendingExpertAdmissionByDraft.find(it, state.conversationId) }
        val selectedExpertIds = if (submissionIdentity == null) pendingExpertAdmission?.selectedAgentIds ?: expertParticipantAgentIds else emptySet()
        val selectedExpertSource = if (inlineMentions.isEmpty()) expertPickerSource else inlineMentions.first().let { ref -> ref.host?.let { ExpertPickerSource(it, checkNotNull(ref.epoch)) } }
        val selectedExpertContext = runCatching {
            inlineSelection(text, inlineMentions) ?: mainExpertSelection(selectedExpertIds, selectedConversationAgentRevisions.toMap(), selectedConversationAgentParticipants.toMap())?.let { selection ->
                selectedExpertSource?.let { JsonObject(selection + ("expected_source_store_epoch" to JsonPrimitive(it.epoch))) } ?: selection
            }
        }.getOrElse { _uiState.update { it.copy(error = context.getString(R.string.expert_profile_changed)) }; return false }
        if (state.currentModel == null) {
            _uiState.update { it.copy(error = context.getString(R.string.chat_no_model_selected)) }
            return false
        }
        if (state.currentModel.source == ModelInfo.ModelSource.WebHost && attachments.isNotEmpty()) {
            _uiState.update { it.copy(error = context.getString(R.string.chat_web_host_images_unsupported)) }
            return false
        }
        if (selectedExpertContext != null && state.currentModel.source != ModelInfo.ModelSource.WebHost &&
            state.currentModel.executionCapabilitiesDeclared && !state.currentModel.executionCapabilities.toolCalls) {
            _uiState.update { it.copy(error = context.getString(R.string.expert_main_requires_tools)) }
            return false
        }
        val executionOwner = composerDrafts.ownerKey(state.conversationId)
        if (runRegistry.find(executionOwner)?.runtime?.hasOutstandingGeneration() == true) {
            _uiState.update { it.copy(error = context.getString(R.string.chat_previous_turn_settling)) }
            return false
        }
        if (state.currentModel?.source == ModelInfo.ModelSource.Local && !runRegistry.localAvailable(executionOwner)) {
            _uiState.update { it.copy(error = "Waiting for the device execution resource. Your draft is retained.") }
            return false
        }
        if (runRegistry.find(executionOwner)?.directHistoryMissing == true && state.currentModel?.source == ModelInfo.ModelSource.WebHost) {
            _uiState.update { it.copy(error = context.getString(R.string.chat_missing_host_history)) }
            return false
        }
        val session = runRegistry.beginExecution(executionOwner, state)
        try {
            if (state.currentModel?.source == ModelInfo.ModelSource.Local) check(runRegistry.acquireLocal(session))

            // Invalidate any database snapshot that started before this turn. A
            // late load must never replace the optimistic user/assistant messages.
            conversationLoads.invalidate()
            session.permission.clear()
            session.trace.reset()
            session.observer.reset()
            session.taskObservation.reset()
            val submissionId = submissionIdentity ?: UUID.randomUUID().toString().lowercase()
            val responseId = "$submissionId:assistant"
            val responseStartedAt = System.currentTimeMillis()
            _uiState.update { it.beginAssistantTurn(responseId, responseStartedAt) }
            session.state.value = _uiState.value
            val submittedState = state
            val submittedDraftOwner = composerDrafts.ownerKey(state.conversationId)
            val submittedDraftCapture = composerDrafts.captureSubmission(submittedDraftOwner)

            runRegistry.start(session) {
            try {
                // A frozen Room/queued submission already owns its input. It must not
                // require or retire a UI draft in the expert's private context.
                if (submissionIdentity == null) {session.submittedDraft = composerDrafts.prepareSubmission(submittedDraftCapture);session.submittedDraftRunId=submissionId}
                val convId = ensureConversation(submittedState, submittedDraftOwner)
                val publishConversation = composerDrafts.ownerKey(displayedState.value.conversationId) == submittedDraftOwner
                runRegistry.bind(session, convId)
                if (publishConversation) displayedState.update { it.copy(conversationId = convId) }
                val conv = conversationRepo.getConversation(convId) ?: error(
                    context.getString(R.string.chat_conversation_unavailable)
                )
                check(submittedState.conversationId == null || submittedState.configurationRevision == conv.configurationRevision) {
                    "Conversation configuration changed. Reload before submitting; your draft is retained."
                }
                val model = capturedExpertModel?.model ?: checkNotNull(conv.selectedModelInfo) { "Conversation model is missing" }
                check(model.source != ModelInfo.ModelSource.Local || runRegistry.acquireLocal(session)) {
                    "Waiting for the device execution resource. Your draft is retained."
                }
                val savedParameters = capturedExpertModel?.parameters ?: checkNotNull(conversationRepo.modelParameters(convId, model)) { "Conversation model parameters are missing" }
                val frozenHost = if (model.source == ModelInfo.ModelSource.WebHost)
                    resolveBoundHost(checkNotNull(conv.executionHostBinding)) else null
                if (selectedExpertContext != null) {
                    require(if (frozenHost == null) selectedExpertSource == null else selectedExpertSource?.host == frozenHost.expectedHostId) { "The selected expert Host changed; reopen the expert picker" }
                    if (frozenHost != null) withContext(Dispatchers.IO) { mainExpertDirectory(expertRoomRepository(convId, selectedExpertSource?.epoch), inline = inlineMentions.isNotEmpty()) }
                }
                check(remoteQueuedConfiguration == null || frozenHost == remoteQueuedConfiguration) {
                    "The queued UI command's Host connection changed; reload the original conversation"
                }
                _uiState.update { it.copy(conversationId = convId,
                    configurationRevision = conv.configurationRevision) }
                val state = submittedState.copy(currentModel = model, currentParameters = savedParameters,
                    generationOverride = com.nola.domain.model.ConversationGenerationOverride())
                val turnModel = if (model.source != ModelInfo.ModelSource.WebHost) {
                    if (editedResend != null) agentTurnService.captureEditedResendModel(editedResend.source.model)
                    else capturedExpertModel ?: agentTurnService.captureModel(model, savedParameters)
                } else null
                ensureTraceSession(convId)
                val activeMessages = conversationRepo.getActiveMessages(convId)
                val userMsg = ConversationMessage(
                    id = editedResend?.userMessageId ?: canonicalUserMessageId ?: queuedAdmission?.userMessageId ?: remoteQueuedCommandId?.let { "ui:$it:user" } ?: submissionId,
                    conversationId = convId,
                    role = ChatRole.user,
                    text = text,
                    meta = messageMeta
                )
                if (state.currentModel?.source != ModelInfo.ModelSource.WebHost) {
                    check(com.nola.data.local.AndroidApplicationRuntime.isAvailable) {
                        "A compatible Nola Application runtime is required for local and API model execution"
                    }
                }
                val usesCanonicalDeviceCore = state.currentModel?.source != ModelInfo.ModelSource.WebHost
                if (!usesCanonicalDeviceCore) {
                    remoteConversationObserver.reset()
                    remoteSubmissionIntent = null
                    remoteAdmissionReceipt = null
                    remoteObservationInitialKey = null
                    remoteJournalTurn = null
                    val configuration = checkNotNull(frozenHost)
                    remoteObservationConfiguration = configuration
                    remoteSubmissionAdmitted = false
                    executionSession.v2TurnCompleted = false
                }
                _uiState.update { it.copy(
                    messages = it.messages + userMsg,
                    processingState = ProcessingState.Streaming,
                    conversationId = convId
                )}

                if (usesCanonicalDeviceCore) {
                    val toolPlan = localToolPlan(convId, state)
                    val modelTools = turnModel?.modelFacingTools(toolPlan.tools) ?: toolPlan.tools
                    val contextRoute = if (state.currentModel?.source == ModelInfo.ModelSource.Local) {
                        AndroidContextRoute.Local
                    } else {
                        AndroidContextRoute.Cloud
                    }
                    check(editedResend == null || activeMessages.isEmpty()) { "Edited resend cannot inherit history" }
                    val prepared = if (editedResend != null) null else prepareFinalContext(
                        turnState = state,
                        conv = conv,
                        messages = activeMessages + userMsg,
                        currentUserText = text,
                        route = contextRoute,
                        tools = modelTools,
                    )
                    prepared?.let { recordPreparedDataSourceUsage(it, convId, contextRoute) }
                    runDeviceAgentResponse(
                        modelSnapshot = turnModel,
                        input = text,
                        convId = convId,
                        prepared = prepared,
                        state = state,
                        toolPlan = toolPlan,
                        attachments = attachments,
                        canonicalTurn = AndroidCanonicalTurnDescriptor(
                            conversationId = convId,
                            conversationTitle = conv.title.orEmpty(),
                            submissionId = submissionId,
                            userMessageId = queuedAdmission?.userMessageId ?: userMsg.id,
                            queuedUserAdmission = queuedAdmission,
                            editedResendOperationId = editedResend?.input?.operationId,
                            userText = text,
                            userContentJson = messageMeta?.let { values ->
                                JsonObject(values.mapValues { JsonPrimitive(it.value) }).toString()
                            },
                            createdAtMs = responseStartedAt,
                            mainExpertSelection = selectedExpertContext,
                            expectedConfigurationRevision = conv.configurationRevision,
                            expectedParentMessageId = activeMessages.lastOrNull()?.let { it.meta?.get("nola.canonicalMessageID") ?: it.id },
                        ),
                    )
                } else if (state.currentModel?.source == ModelInfo.ModelSource.WebHost) {
                    val v2ModelInput = ModelInput(messages = listOf(mapOf("role" to "user", "content" to text)))
                    streamAgentResponse(
                        input = v2ModelInput,
                        convId = convId,
                        submissionId = submissionId,
                        originalText = text,
                        expectedConfigurationRevision = conv.configurationRevision,
                        expectedParentMessageId = activeMessages.lastOrNull()?.id,
                        queuedUserCommandId = remoteQueuedCommandId,
                        selectedExperts = selectedExpertContext?.get("selected_experts")?.jsonArray,
                        expertSourceEpoch = selectedExpertContext?.get("expected_source_store_epoch")?.jsonPrimitive?.content,
                        expertMentions = selectedExpertContext?.get("expert_mentions")?.jsonObject,
                    )
                }
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                Log.e("ChatViewModel", "Chat turn failed", e)
                if (session.retired) return@start
                val failureMessage = if (e is com.nola.data.local.AndroidApplicationRuntime.RejectedMutation &&
                    (mainExpertRequiresTools(e.code) || mainExpertRequiresTools(e.message)))
                    context.getString(R.string.expert_main_requires_tools) else e.message
                if (e is com.nola.core.HostEffectCancellationUncertain) {
                    _uiState.update { it.copy(processingState = ProcessingState.AwaitingRecovery, error = failureMessage) }
                    return@start
                }
                if (!session.dispatchStarted && remoteSubmissionIntent == null) {
                    val visible = displayedState.value.activeResponseId == responseId
                    val restored = session.preparationBase.copy(error = failureMessage)
                    session.state.value = restored
                    if (visible) displayedState.value = restored
                    return@start
                }
                if (state.currentModel?.source == ModelInfo.ModelSource.WebHost && remoteSubmissionIntent != null) {
                    _uiState.update { it.copy(processingState = ProcessingState.BuildingContext, error = failureMessage) }
                    _uiState.value.conversationId?.let(::startRemoteConversationObservation)
                    return@start
                }
                _uiState.update { it.copy(
                    processingState = if (e is com.nola.data.local.AndroidApplicationRuntime.PendingMutation)
                        ProcessingState.AwaitingRecovery else ProcessingState.Error,
                    error = failureMessage ?: context.getString(R.string.common_unknown_error),
                    pendingCommandId = (e as? com.nola.data.local.AndroidApplicationRuntime.PendingMutation)?.requestId,
                )}
            } finally {
                if (queuedAdmission != null && submittedState.conversationId != null) {
                    releaseQueuedUiLease(submittedState.conversationId, queuedAdmission)
                }
                clearGenerationJobIfCurrent(coroutineContext[Job])
            }
        }
        } catch (error: Throwable) {
            runRegistry.releaseLocal(session)
            runRegistry.remove(executionOwner)
            throw error
        }
        return true
    }

    private suspend fun runDeviceAgentResponse(
        input: String,
        convId: String,
        prepared: AndroidPreparedContext?,
        state: ChatUiState,
        toolPlan: AgentCoreAndroidToolPlan,
        attachments: List<com.nola.domain.model.ModelAttachment>,
        canonicalTurn: AndroidCanonicalTurnDescriptor,
        recoverExistingRun: Boolean = false,
        modelSnapshot: com.nola.core.AndroidTurnModelSnapshot? = null,
    ) {
        var mayContinue = runSingleDeviceAgentResponse(input, convId, prepared, state, toolPlan,
            attachments, canonicalTurn, recoverExistingRun, modelSnapshot)
        while (mayContinue && !executionSession.retired && !executionSession.successorCancellationRequested) {
            kotlinx.coroutines.currentCoroutineContext().let { context ->
                if (context[Job]?.isActive == false) throw kotlinx.coroutines.CancellationException()
            }
            val accepted = withContext(Dispatchers.IO) {
                applicationRuntime.execute(buildJsonObject {
                    put("type", "promote_queued_assistant_turn")
                    put("input", buildJsonObject {
                        put("conversation_id", convId); put("created_at_ms", System.currentTimeMillis())
                    })
                })
            }
            if (accepted == JsonNull) return
            val begin = accepted.jsonObject.getValue("begin").jsonObject
            require(begin.getValue("conversation_id").jsonPrimitive.content == convId)
            val runId = begin.getValue("run_id").jsonPrimitive.content
            val next = withContext(Dispatchers.IO) {
                agentTurnService.recoveryDescriptor(convId, canonicalTurn.conversationTitle, runId)
            }
            // Exact recovery deliberately has no new user-message input.
            require(next.assistantMessageId == begin.getValue("assistant_message_id").jsonPrimitive.content)
            // Cancellation after admission preserves the independent Run for
            // exact recovery; never dispatch it with the predecessor's stop.
            if (executionSession.successorCancellationRequested || executionSession.retired) return
            _uiState.update { it.bindRecoveredAssistant(next).copy(agentPermissionRequest = null, agentPermissionAction = null) }
            mayContinue = runSingleDeviceAgentResponse("", convId, null, _uiState.value,
                localToolPlan(convId, _uiState.value), emptyList(), next, true, null)
        }
    }

    private suspend fun runSingleDeviceAgentResponse(
        input: String,
        convId: String,
        prepared: AndroidPreparedContext?,
        state: ChatUiState,
        toolPlan: AgentCoreAndroidToolPlan,
        attachments: List<com.nola.domain.model.ModelAttachment>,
        canonicalTurn: AndroidCanonicalTurnDescriptor,
        recoverExistingRun: Boolean = false,
        modelSnapshot: com.nola.core.AndroidTurnModelSnapshot? = null,
    ): Boolean {
        executionSession.hasCommittedTerminalSeal = false
        val executionMode = AndroidAgentExecutionMode.DeviceLocal
        val hasPreparedAttachments = prepared?.turnContextComponents?.any { it.kind == "attachment" } == true
        val modelInput = if (hasPreparedAttachments) {
            input.replace(CHAT_ATTACHMENT_BLOCK, "").trim()
        } else {
            input
        }
        val turn = agentTurnService.start(
            AndroidAgentTurnRequest(
                model = if (recoverExistingRun) null else modelSnapshot ?: agentTurnService.captureModel(
                    checkNotNull(state.currentModel) { "Select a model before starting" }, state.effectiveGenerationParameters),
                sessionId = convId,
                input = modelInput,
                history = prepared?.modelInput?.canonicalMessages?.dropLastUserMessage()
                    ?.mapNotNull(::agentCoreMessage).orEmpty(),
                tools = modelSnapshot?.modelFacingTools(prepared?.modelInput?.tools ?: toolPlan.tools)
                    ?: prepared?.modelInput?.tools ?: toolPlan.tools,
                toolPlan = toolPlan,
                config = AgentCoreRuntimeConfig(
                    executionMode = executionMode.wireValue,
                    maxIterations = 0,
                    contextWindowTokens = state.effectiveGenerationParameters.maxContextTokens,
                    grantedCapabilities = toolPlan.grantedCapabilities,
                ),
                turnContextComponents = prepared?.turnContextComponents.orEmpty(),
                responseStyle = state.responseStyle,
                customSystemPrompt = state.currentParameters.customSystemPrompt,
                enableThinking = state.currentParameters.enableThinking,
                attachments = attachments,
                canonicalTurn = canonicalTurn,
                recoverExistingRun = recoverExistingRun,
                resultEgressAuthorized = state.agentMode,
                delegationEnabled = state.agentMode,
            ),
        )
        val coordinator = turn.coordinator
        turnRuntime.bindDeviceCoordinator(coordinator)
        var localJournalProjection: ConversationProjectionV1? = null
        var localJournalAdmissions: List<ConversationMessage> = emptyList()
        var responseFinalized = false
        var mayContinue = false
        try {
            executionSession.dispatchStarted = true
            turn.events.collect { event ->
                if (executionSession.retired || _uiState.value.conversationId != convId || turnRuntime.deviceCoordinator() !== coordinator) return@collect
                val traceEvent = (event as? DeviceAgentCoordinatorEvent.Trace)?.event
                if (traceEvent != null) {
                    applyTraceEvent(traceEvent)
                }
                when (event) {
                    is DeviceAgentCoordinatorEvent.CancellationRequested -> _uiState.update { it.copy(
                        processingState = if (event.externalOutcomeUnknown) ProcessingState.AwaitingRecovery else ProcessingState.BuildingContext,
                        agentPermissionRequest = null, agentPermissionAction = null,
                        error = if (event.externalOutcomeUnknown) "The interrupted tool outcome is unknown. Inspect exact Run recovery before continuing." else null,
                    ) }
                    is DeviceAgentCoordinatorEvent.Admitted -> {
                        require(event.conversationId == convId && event.submissionId == canonicalTurn.submissionId)
                        retireSubmittedDraft()

                    }
                    is DeviceAgentCoordinatorEvent.Snapshot -> {
                        require(event.snapshot.sessionId == turn.coreSessionId) { "Control belongs to another Core session" }
                        applyAgentCoreSnapshot(event.snapshot)
                    }
                    is DeviceAgentCoordinatorEvent.JournalPublications -> {
                        for (cursor in event.cursors) {
                            val identity = cursor.getValue("identity").jsonObject
                            require(identity.getValue("conversation_id").jsonPrimitive.content == convId &&
                                identity.getValue("run_id").jsonPrimitive.content == canonicalTurn.submissionId) {
                                "Local journal belongs to another conversation or Run"
                            }
                            val messageId = identity.getValue("assistant_message_id").jsonPrimitive.content
                            if (localJournalProjection?.messages?.none { it.messageId == messageId } != false) {
                                localJournalProjection = withContext(Dispatchers.IO) {
                                    conversationRepo.getProjectionWindow(convId, null).projection
                                }
                                localJournalAdmissions = CanonicalConversationProjector.project(checkNotNull(localJournalProjection))
                            }
                            val projection = checkNotNull(localJournalProjection)
                            val source = checkNotNull(projection.messages.find { it.messageId == messageId }) {
                                "Local journal message has no visible canonical admission"
                            }
                            val parts = withContext(Dispatchers.IO) { assistantJournal.displayParts(cursor, source.revision) }
                            val updated = CanonicalConversationProjector.project(
                                projection.copy(messages = listOf(source.copy(parts = parts)))).single()
                            _uiState.update { state ->
                                if (state.conversationId != convId || turnRuntime.deviceCoordinator() !== coordinator) state
                                else {
                                    val messages = CanonicalConversationProjector.mergeJournalMessage(state.messages, localJournalAdmissions, updated)
                                    if (messageId != canonicalTurn.assistantMessageId) state.copy(messages = messages)
                                    else state.copy(messages = messages, journalResponse = updated, currentResponse = updated.text,
                                        thinking = decodeAgentThinkingFromMeta(updated.meta), toolEvents = decodeHostToolEventsFromMeta(updated.meta))
                                }
                            }
                        }
                    }
                    is DeviceAgentCoordinatorEvent.TaskProjectionRestored -> {
                        _uiState.update { state ->
                            if (state.conversationId == convId && event.projection.sessionId == turn.coreSessionId) {
                                state.copy(coreTaskProjection = event.projection)
                            } else {
                                state
                            }
                        }
                    }
                    is DeviceAgentCoordinatorEvent.TaskLifecycle -> applyCoreTaskUpdate(event.update)
                    is DeviceAgentCoordinatorEvent.SkillProjectionUpdated -> {
                        _uiState.update { state ->
                            if (state.conversationId == convId && event.projection.sessionId == turn.coreSessionId) {
                                state.copy(
                                    coreSkillProjection = event.projection,
                                    toolEvents = event.projection.mergeIntoToolEvents(state.toolEvents),
                                )
                            } else {
                                state
                            }
                        }
                    }
                    is DeviceAgentCoordinatorEvent.CapabilitiesActivated -> {
                        val stagedToolEvents = capabilityActivationBuffer.stage(event.activation)
                        _uiState.update { state ->
                            if (state.conversationId == convId && event.activation.sessionId == turn.coreSessionId) {
                                state.copy(
                                    toolEvents = stagedToolEvents.fold(state.toolEvents, ::upsertHostToolEvent),
                                )
                            } else {
                                state
                            }
                        }
                    }
                    is DeviceAgentCoordinatorEvent.Trace -> Unit
                    is DeviceAgentCoordinatorEvent.PermissionRequested -> {
                        _uiState.update {
                            it.copy(agentPermissionRequest = event.request, agentPermissionAction = null)
                        }
                    }
                    is DeviceAgentCoordinatorEvent.Completed -> {
                        mayContinue = canonicalTurn.automationLease == null && canonicalTurn.automationSpec == null &&
                            event.seal.result.getValue("value").jsonObject.getValue("kind").jsonPrimitive.content in
                                setOf("answered", "action_completed")
                        require(event.seal.conversationId == convId && event.seal.runId == canonicalTurn.submissionId &&
                            event.seal.assistantMessageId == canonicalTurn.assistantMessageId &&
                            event.seal.coreSessionId == turn.coreSessionId) {
                            "Local completion seal belongs to another execution owner"
                        }
                        executionSession.markTerminalCommitted()
                        refreshPermissionOwnerProjection(executionSession)
                        _uiState.update { it.copy(agentPermissionRequest = null, agentPermissionAction = null) }
                        if (!responseFinalized) {
                            finalizeResponse(convId) {
                                _uiState.value.conversationId == convId &&
                                    _uiState.value.activeResponseId == canonicalTurn.assistantMessageId &&
                                    turnRuntime.deviceCoordinator() === coordinator
                            }
                            presentSealedRunResult(convId,event.seal.result)
                            responseFinalized = true
                        }
                    }
                }
            }
            if (!responseFinalized && _uiState.value.conversationId == convId && turnRuntime.deviceCoordinator() === coordinator) {
                error("Local Agent stream ended without an exact committed terminal seal")
            }
        } finally {
            turnRuntime.clearDeviceCoordinator(coordinator)
        }
        return mayContinue && responseFinalized
    }

    private suspend fun streamAgentResponse(input: ModelInput, convId: String, submissionId: String, originalText: String,
        expectedConfigurationRevision: Long?, expectedParentMessageId: String?, queuedUserCommandId: String? = null, selectedExperts: JsonArray? = null, expertSourceEpoch: String? = null, expertMentions: JsonObject? = null) {
        remoteConversationObserver.reset()
        val configuration = checkNotNull(remoteObservationConfiguration)
        val hostId = configuration.expectedHostId
        require(hostId.isNotBlank())
        val owner = com.nola.data.local.AndroidJournalTurn(hostId, convId, submissionId, "$submissionId:assistant")
        remoteJournalTurn = owner
        executionSession.v2TurnCompleted = false
        executionSession.dispatchStarted = true
        check(!executionSession.directHistoryMissing) { context.getString(R.string.chat_missing_host_history) }
        executionSession.directHistoryJob?.cancel()
        val remoteConvId = _uiState.value.v2SessionId ?: readHostSession(hostId, convId)
            ?: webAgentProvider.v2CreateConversation(conversationRepo.getConversation(convId)?.title ?: "Chat", configuration).also {
                // Persist the server-issued identity before the first message can reach the Host.
                rememberHostSession(hostId, convId, it)
            }
        executionSession.directHostHistory = true
        _uiState.update { it.copy(v2SessionId = remoteConvId) }
        // The Host row selects an execution provider. Let that Host resolve its own model.
        val selectedModelId = _uiState.value.currentModel?.takeUnless { it.source == ModelInfo.ModelSource.WebHost }
            ?.id?.takeIf { it.isNotBlank() }
        val permissionProfile = when (_uiState.value.permissionMode) {
            com.nola.core.AndroidAgentPermissionMode.AllowAll -> "allow_all"
            com.nola.core.AndroidAgentPermissionMode.Cautious -> "cautious"
        }
        val hybridState = _uiState.value
        val deviceBridge = if (hybridState.hybridDeviceToolsEnabled) {
            val tools = webAgentProvider.contextToolSchemas(
                conversationId = remoteConvId,
                hybridDeviceToolsEnabled = true,
                hybridDeviceCapabilities = hybridState.hybridDeviceCapabilities,
                permissionMode = hybridState.permissionMode,
            )
            webAgentProvider.deviceBridgeAdvertisement(
                config = configuration,
                sessionId = remoteConvId,
                allowedCapabilities = hybridState.hybridDeviceCapabilities,
                preparedTools = tools,
            )
        } else null
        try {
            webAgentProvider.streamV2Message(
                conversationId = remoteConvId,
                content = originalText,
                configuration = configuration,
                modelId = selectedModelId,
                permissionProfile = permissionProfile,
                selectedExperts = selectedExperts,
                deviceBridge = deviceBridge,
            ).collect { event ->
                if (!currentRemoteJournal(owner) && !executionSession.v2TurnCompleted) return@collect
                handleAgentEvent(event, convId)
            }
            if (!executionSession.v2TurnCompleted && currentRemoteJournal(owner)) {
                val state = _uiState.value
                if (state.currentResponse.isNotEmpty()) {
                    val responseId = state.activeResponseId ?: owner.assistantMessageId
                    val startedAt = state.activeResponseStartedAt ?: System.currentTimeMillis()
                    val assistantMsg = ConversationMessage(
                        id = responseId,
                        conversationId = convId,
                        role = ChatRole.assistant,
                        text = state.currentResponse,
                        createdAt = startedAt,
                    )
                    executionSession.v2TurnCompleted = true
                    _uiState.update { it.completeAssistantTurn(assistantMsg).copy(pendingUserQuestion = null) }
                } else {
                    _uiState.update {
                        it.copy(
                            processingState = ProcessingState.Idle,
                            activeResponseId = null,
                            activeTurnId = null,
                            activeResponseStartedAt = null,
                            pendingUserQuestion = null,
                        )
                    }
                }
            }
        } catch (failure: Exception) {
            if (failure is kotlinx.coroutines.CancellationException) throw failure
            if (executionSession.v2TurnCompleted) return
            if (!currentRemoteJournal(owner)) return
            _uiState.update { it.copy(processingState = ProcessingState.Error, error = failure.message) }
        }
    }

    private fun currentRemoteJournal(owner: com.nola.data.local.AndroidJournalTurn): Boolean =
        !executionSession.retired && remoteJournalTurn === owner && _uiState.value.conversationId == owner.conversationId && _uiState.value.activeResponseId == owner.assistantMessageId

    private fun presentSealedRunResult(conversationId:String,result:JsonObject?) {
        val value=result?.get("value") as? JsonObject ?:return
        if(value["kind"]!=JsonPrimitive("blocked"))return
        val overBudget=(value["reason"] as? JsonObject)?.get("code")==JsonPrimitive("context.input_budget_exceeded")
        if(overBudget&&result["run_id"]==executionSession.submittedDraftRunId?.let(::JsonPrimitive))executionSession.submittedDraft?.let { composerDrafts.restoreBlockedSubmission(it,conversationId) }
        _uiState.update { it.copy(error=context.getString(if(overBudget)R.string.chat_context_budget_exceeded else R.string.chat_run_blocked)) }
    }

    private suspend fun verifyRemoteTerminal(owner: com.nola.data.local.AndroidJournalTurn, digest: String? = null, candidateRunId: String? = null): Boolean {
        if (!remoteSubmissionAdmitted) return false
        if (!currentRemoteJournal(owner)) return false
        val transport = checkNotNull(remoteJournalTransport)
        val heads = withContext(Dispatchers.IO) {
            assistantJournal.receiveTerminal(owner, transport, { currentRemoteJournal(owner) }, digest, candidateRunId)
        } ?: return false
        if (!currentRemoteJournal(owner)) return false
        require(heads.any { it.getValue("identity").jsonObject["assistant_message_id"]?.jsonPrimitive?.content == owner.assistantMessageId })
        executionSession.permissionSourceUnconfirmed = false
        val projection = withContext(Dispatchers.IO) { conversationRepo.getProjection(owner.conversationId) }
        if (!currentRemoteJournal(owner)) return false
        _uiState.update { state -> if (!currentRemoteJournal(owner)) state else state.applyCanonicalContextUsage(projection.contextUsage) }
        finalizeResponse(owner.conversationId)
        presentSealedRunResult(owner.conversationId,owner.terminalResult)
        return true
    }

    private suspend fun applyRemoteJournal(through: JsonObject, owner: com.nola.data.local.AndroidJournalTurn) {
        require(remoteSubmissionAdmitted) { "Journal preceded source submission admission" }
        val transport = checkNotNull(remoteJournalTransport)
        val parts = withContext(Dispatchers.IO) {
            assistantJournal.receive(through, owner, transport) { currentRemoteJournal(owner) }
            assistantJournal.displayParts(through)
        }
        if (!currentRemoteJournal(owner)) return
        remoteObservationInitialKey = owner.runId?.let {
            com.nola.data.remote.api.RemoteRunKey(owner.conversationId, it, owner.assistantMessageId)
        }
        val messageId = through.getValue("identity").jsonObject.getValue("assistant_message_id").jsonPrimitive.content
        var projection = checkNotNull(remoteJournalProjection)
        var admittedMessages: List<ConversationMessage> = emptyList()
        if (projection.messages.none { it.messageId == messageId }) {
            projection = withContext(Dispatchers.IO) { conversationRepo.getProjection(owner.conversationId) }
            if (!currentRemoteJournal(owner)) return
            remoteJournalProjection = projection
            admittedMessages = CanonicalConversationProjector.project(projection)
        }
        val source = checkNotNull(projection.messages.find { it.messageId == messageId })
        val presented = CanonicalConversationProjector.project(projection.copy(messages = listOf(source.copy(parts = parts)))).single()
        _uiState.update { state ->
            if (!currentRemoteJournal(owner)) state else {
                val admissions = if (admittedMessages.isEmpty()) listOf(presented) else admittedMessages
                val messages = CanonicalConversationProjector.mergeJournalMessage(state.messages, admissions, presented)
                if (messageId != owner.assistantMessageId) state.copy(messages = messages)
                else state.copy(messages = messages, journalResponse = presented, currentResponse = presented.text,
                    thinking = decodeAgentThinkingFromMeta(presented.meta), toolEvents = decodeHostToolEventsFromMeta(presented.meta))
            }
        }
    }

    private val v2PermissionRequestIds = java.util.Collections.synchronizedSet(mutableSetOf<String>())

    private fun isV2SseEvent(event: SseEvent): Boolean = when (event) {
        is SseEvent.V2SessionStarted,
        is SseEvent.V2TextDelta,
        is SseEvent.V2ReasoningDelta,
        is SseEvent.V2ToolStarted,
        is SseEvent.V2ToolFinished,
        is SseEvent.V2Compacted,
        is SseEvent.V2UiSurface,
        is SseEvent.V2PermissionRequested,
        is SseEvent.V2UserQuestion,
        is SseEvent.V2ExpertReply,
        is SseEvent.V2TaskSnapshot,
        is SseEvent.V2TaskEvent,
        is SseEvent.V2TurnCompleted -> true
        else -> false
    }

    private fun upsertV2UiSurface(surfaces: List<JsonObject>, snapshot: JsonObject): List<JsonObject> {
        val surfaceId = ((snapshot["identity"] as? JsonObject)?.get("surface_id") as? JsonPrimitive)?.contentOrNull
            ?.takeIf { it.isNotBlank() }
            ?: return surfaces + snapshot
        val index = surfaces.indexOfFirst { existing ->
            ((existing["identity"] as? JsonObject)?.get("surface_id") as? JsonPrimitive)?.contentOrNull == surfaceId
        }
        return if (index < 0) surfaces + snapshot else surfaces.toMutableList().also { it[index] = snapshot }
    }

    private suspend fun handleAgentEvent(event: SseEvent, convId: String) {
        if (event is SseEvent.SubmissionReceipt) {
            val configuration = checkNotNull(remoteObservationConfiguration)
            if (remoteSubmissionIntent != null) acceptRemoteSubmissionReceipt(event.receipt, configuration)
            else {
                val owner = checkNotNull(remoteJournalTurn)
                val loadGeneration = conversationLoads.currentGeneration()
                val receipt = com.nola.data.remote.api.decodeObservedSubmissionReceipt(event.receipt, configuration.expectedHostId,
                    configuration.clientId, com.nola.data.remote.api.RemoteRunKey(convId, checkNotNull(owner.runId), owner.assistantMessageId))
                withContext(Dispatchers.IO) { conversationRepo.adoptObservedRemoteRun(receipt) }
                if (!isExecutionCurrent(loadGeneration) || !currentRemoteJournal(owner)) return
                remoteSubmissionAdmitted = true
                remoteAdmissionReceipt = receipt
            }
            return
        }
        if (!remoteSubmissionAdmitted && !isV2SseEvent(event)) {
            if (event is SseEvent.Error) _uiState.update { it.copy(error = if (mainExpertRequiresTools(event.code)) context.getString(R.string.expert_main_requires_tools) else event.message, processingState = ProcessingState.BuildingContext) }
            else if (event is SseEvent.AssistantJournal || event is SseEvent.AssistantTerminal || event is SseEvent.PermissionRequest)
                error("Remote event preceded source submission admission")
            return
        }
        val traceEvent = ensureTraceSession(convId).fromSse(event)
        if (traceEvent != null) {
            applyTraceEvent(traceEvent)
        }
        when (event) {
            is SseEvent.SubmissionReceipt -> Unit
            is SseEvent.CoreProjectionHint -> {
                require(event.conversationId == convId)
                // The dedicated task subscription owns task projections; host challenges
                // own approvals. Never promote a control hint to content, a grant or a seal.
            }
            is SseEvent.ContextUsageChanged -> {
                val owner = checkNotNull(remoteJournalTurn)
                val loadGeneration = conversationLoads.currentGeneration()
                require(event.envelope["session_id"]?.jsonPrimitive?.content == convId)
                withContext(Dispatchers.IO) {
                    assistantJournal.receiveContextUsage(owner, checkNotNull(remoteJournalTransport), current = {
                        currentRemoteJournal(owner) && isExecutionCurrent(loadGeneration)
                    })
                }
                if (!currentRemoteJournal(owner) || !isExecutionCurrent(loadGeneration)) return
                val projection = withContext(Dispatchers.IO) { conversationRepo.getProjection(convId) }
                if (!currentRemoteJournal(owner) || !isExecutionCurrent(loadGeneration)) return
                remoteJournalProjection = projection
                _uiState.update { state -> if (!currentRemoteJournal(owner)) state else
                    state.applyCanonicalContextUsage(projection.contextUsage) }
            }
            is SseEvent.AssistantJournal -> {
                val owner = checkNotNull(remoteJournalTurn)
                require(event.envelope["source_host_id"]?.jsonPrimitive?.content == owner.hostId && event.envelope["session_id"]?.jsonPrimitive?.content == convId)
                applyRemoteJournal(event.envelope.getValue("through").jsonObject, owner)
            }
            is SseEvent.AssistantTerminal -> {
                val owner = checkNotNull(remoteJournalTurn)
                require(event.envelope["source_host_id"]?.jsonPrimitive?.content == owner.hostId && event.envelope["session_id"]?.jsonPrimitive?.content == convId)
                val candidate = event.envelope.getValue("run_id").jsonPrimitive.content
                require(owner.runId == null || candidate == owner.runId)
                verifyRemoteTerminal(owner, event.envelope.getValue("seal_digest").jsonPrimitive.content, candidate)
            }
            is SseEvent.Trace -> Unit
            is SseEvent.V2SessionStarted -> {
                retireSubmittedDraft()
                val resolvedSessionId = event.sessionId.takeIf { it.isNotBlank() } ?: convId
                _uiState.update {
                    it.copy(
                        v2SessionId = resolvedSessionId,
                        processingState = ProcessingState.Streaming,
                        error = null,
                    )
                }
            }
            is SseEvent.V2TextDelta -> {
                if (event.delta.isNotEmpty()) {
                    _uiState.update {
                        it.copy(
                            currentResponse = it.currentResponse + event.delta,
                            processingState = ProcessingState.Streaming,
                            thinkingFinished = true,
                        )
                    }
                }
            }
            is SseEvent.V2ReasoningDelta -> {
                if (event.delta.isNotEmpty()) {
                    _uiState.update {
                        it.copy(
                            thinking = it.thinking + event.delta,
                            processingState = ProcessingState.Streaming,
                            thinkingFinished = false,
                        )
                    }
                }
            }
            is SseEvent.V2ToolStarted -> {
                val toolEvent = HostAgentToolEvent(
                    id = event.callId.ifBlank { UUID.randomUUID().toString() },
                    toolName = event.name.ifBlank { "tool" },
                    status = "running",
                )
                _uiState.update {
                    it.copy(
                        toolEvents = upsertHostToolEvent(it.toolEvents, toolEvent),
                        processingState = ProcessingState.Streaming,
                    )
                }
            }
            is SseEvent.V2ToolFinished -> {
                val toolEvent = HostAgentToolEvent(
                    id = event.callId.ifBlank { UUID.randomUUID().toString() },
                    toolName = event.name.ifBlank { "tool" },
                    status = if (event.isError) "error" else "done",
                    resultSummary = event.summary.takeIf { it.isNotBlank() },
                    error = event.isError,
                )
                _uiState.update {
                    it.copy(
                        toolEvents = upsertHostToolEvent(it.toolEvents, toolEvent),
                        processingState = ProcessingState.Streaming,
                    )
                }
            }
            is SseEvent.V2Compacted -> {
                _uiState.update {
                    it.copy(
                        v2CompactionSummary = event.summary.takeIf { s -> s.isNotBlank() } ?: context.getString(R.string.chat_v2_context_compacted),
                        processingState = ProcessingState.Streaming,
                    )
                }
            }
            is SseEvent.V2UiSurface -> {
                _uiState.update {
                    it.copy(
                        v2UiSurfaces = upsertV2UiSurface(it.v2UiSurfaces, event.snapshot),
                        processingState = ProcessingState.Streaming,
                    )
                }
            }
            is SseEvent.V2PermissionRequested -> {
                val requestId = event.requestId.ifBlank { UUID.randomUUID().toString() }
                val argsMap: Map<String, Any> = (event.arguments as? JsonObject)
                    ?.mapValues { (_, v) -> (v as? JsonPrimitive)?.contentOrNull ?: v.toString() }
                    .orEmpty()
                val toolLabel = event.tool.ifBlank { "tool" }
                val reviewDetail = event.reason.takeIf { it.isNotBlank() }
                    ?: event.arguments?.toString()?.takeIf { it.isNotBlank() && it != "null" && it != "{}" }
                val safeReviewText = if (reviewDetail != null) "$toolLabel: $reviewDetail" else toolLabel
                val request = HostAgentPermissionRequest(
                    allowConversation = true,
                    id = requestId,
                    toolName = toolLabel,
                    args = argsMap,
                    reason = event.reason,
                    message = event.reason.ifBlank { toolLabel },
                    sessionId = _uiState.value.v2SessionId?.takeIf { it.isNotBlank() } ?: convId,
                    safeReviewText = safeReviewText,
                )
                v2PermissionRequestIds.add(requestId)
                permissionSession.enqueueHostRequest(request)
                val queued = permissionSession.hostRequests()
                _uiState.update {
                    it.copy(
                        toolEvents = transitionHostToolPermission(it.toolEvents, request, "permission_required"),
                        pendingPermissionRequests = queued,
                        permissionRequest = queued.firstOrNull(),
                        processingState = ProcessingState.Streaming,
                        thinkingFinished = true,
                    )
                }
            }
            is SseEvent.V2UserQuestion -> {
                val prompt = V2UserQuestionPrompt(
                    requestId = event.requestId,
                    conversationId = _uiState.value.v2SessionId?.takeIf { it.isNotBlank() } ?: convId,
                    question = event.question,
                    options = event.options,
                )
                _uiState.update {
                    it.copy(
                        pendingUserQuestion = prompt,
                        processingState = ProcessingState.Streaming,
                        thinkingFinished = true,
                    )
                }
            }
            is SseEvent.V2ExpertReply -> Unit
            is SseEvent.V2TaskSnapshot -> {
                if (event.conversationId == convId && event.cursor > 0L) {
                    remoteTaskObservation.advance(event.cursor)
                }
            }
            is SseEvent.V2TaskEvent -> {
                if (event.conversationId == convId && event.seq > 0L) {
                    remoteTaskObservation.advance(event.seq)
                }
            }
            is SseEvent.V2TurnCompleted -> {
                executionSession.v2TurnCompleted = true
                val resolvedSessionId = event.sessionId.takeIf { it.isNotBlank() }
                val currentState = _uiState.value
                val finalText = event.finalText.takeIf { text -> text.isNotBlank() }
                    ?: currentState.currentResponse
                val responseId = currentState.activeResponseId ?: "${UUID.randomUUID()}:assistant"
                val startedAt = currentState.activeResponseStartedAt ?: System.currentTimeMillis()
                val assistantMsg = ConversationMessage(
                    id = responseId,
                    conversationId = convId,
                    role = ChatRole.assistant,
                    text = finalText,
                    createdAt = startedAt,
                )
                _uiState.update { state ->
                    val completed = if (finalText.isNotEmpty()) {
                        state.completeAssistantTurn(assistantMsg)
                    } else {
                        state.copy(
                            processingState = ProcessingState.Idle,
                            currentResponse = "",
                            activeResponseId = null,
                            activeTurnId = null,
                            activeResponseStartedAt = null,
                            thinking = "",
                            thinkingFinished = true,
                            toolEvents = emptyList(),
                            permissionRequest = null,
                            pendingPermissionRequests = emptyList(),
                        )
                    }
                    completed.copy(
                        v2SessionId = resolvedSessionId ?: completed.v2SessionId,
                        pendingUserQuestion = null,
                    )
                }
                resolvedSessionId?.let { remoteId ->
                    remoteObservationConfiguration?.let { cfg -> rememberHostSession(cfg.expectedHostId, convId, remoteId, _uiState.value.messages) }
                }
                executionSession.permissionSourceUnconfirmed = false
                refreshPermissionOwnerProjection(executionSession)
            }
            is SseEvent.TurnLifecycle -> {
                // Core lifecycle is only a hint; exact Run seal proves completion.
                val owner = remoteJournalTurn
                if (owner != null && event.envelope["session_id"]?.jsonPrimitive?.content == convId &&
                    event.envelope["type"]?.jsonPrimitive?.content != "turn_started") verifyRemoteTerminal(owner)
            }
            is SseEvent.TaskProjection -> applyRemoteTaskProjection(event)
            is SseEvent.TaskStreamCheckpoint -> {
                if (event.conversationId == convId) {
                    remoteTaskObservation.advance(event.cursor)
                }
            }
            is SseEvent.ExternalSessionStarted -> {
                if (event.backendId.isBlank() || event.sessionId.isBlank()) return
                val conversation = conversationRepo.getConversation(convId) ?: return
                if (conversation.executionBackend != event.backendId) return
                val committedRevision = conversationRepo.updateExecutionBackend(
                    convId,
                    conversation.executionBackend,
                    conversation.externalExecutionSessions + (event.backendId to event.sessionId),
                )
                _uiState.update { current ->
                    if (current.conversationId == convId) {
                        current.copy(configurationRevision = maxOf(current.configurationRevision, committedRevision))
                    } else current
                }
            }
            is SseEvent.PermissionRequest -> {
                val hostConfig = remoteObservationConfiguration
                val next = acceptHostPermissionEvent(
                    _uiState.value, event, convId,
                    hostConfig?.expectedHostId, hostConfig?.clientId,
                    System.currentTimeMillis() / 1_000L,
                )
                if (next == null) {
                    _uiState.update { it.copy(error = context.getString(R.string.chat_error_host_permission_identity_mismatch)) }
                    return
                }
                permissionSession.setHostRequests(next.hostPermissionQueue)
                _uiState.value = next
            }

            is SseEvent.DeviceToolRequest -> {
                val hostConfig = remoteObservationConfiguration
                val activeRemoteSessionId = _uiState.value.v2SessionId?.takeIf { it.isNotBlank() }
                val matchesSession = event.sessionId == convId ||
                    (activeRemoteSessionId != null && event.sessionId == activeRemoteSessionId)
                if (hostConfig == null ||
                    !hostConfig.expectedHostId.equals(event.hostId) ||
                    !hostConfig.clientId.equals(event.clientId) ||
                    !matchesSession ||
                    event.protocolVersion != 1 ||
                    event.nonce.isBlank() ||
                    event.expiresAt < System.currentTimeMillis() / 1_000L
                ) {
                    _uiState.update { it.copy(error = context.getString(R.string.chat_error_host_permission_identity_mismatch)) }
                    return
                }
                permissionSession.setDeviceToolRequest(event)
                val request = HostAgentPermissionRequest(
                    allowConversation = false,
                    id = event.id,
                    toolName = event.toolName,
                    args = event.args,
                    match = event.capability,
                    reason = context.getString(R.string.chat_permission_hybrid_request_reason),
                    message = context.getString(
                        R.string.chat_permission_hybrid_run_format,
                        BuiltinToolNames.label(event.toolName, null, context.resources.configuration.locales[0]) ?: event.toolName,
                    ),
                    nonce = event.nonce,
                    hostId = event.hostId,
                    clientId = event.clientId,
                    sessionId = event.sessionId,
                    expiresAt = event.expiresAt,
                    protocolVersion = event.protocolVersion,
                    deviceBridge = true
                )
                permissionSession.enqueueHostRequest(request)
                val queued = permissionSession.hostRequests()
                _uiState.update {
                    it.copy(
                        toolEvents = transitionHostToolPermission(
                            events = it.toolEvents,
                            request = request,
                            status = "permission_required",
                        ),
                        pendingPermissionRequests = queued,
                        permissionRequest = queued.firstOrNull(),
                        processingState = ProcessingState.Streaming,
                        thinkingFinished = true
                    )
                }
            }
            is SseEvent.Error -> {
                _uiState.update { it.copy(error = if (mainExpertRequiresTools(event.code)) context.getString(R.string.expert_main_requires_tools) else event.message) }
            }
        }
    }

    fun provideHostPermissionInstruction(requestId: String, instruction: String) {
        if (runRegistry.current == null) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { provideHostPermissionInstruction(requestId, instruction) } }
            return
        }
        val request = permissionSession.findHostRequest(requestId) ?: return
        if (request.deviceBridge || !permissionSession.beginInstruction(request.id)) return
        val payload = runCatching { hostPermissionInstructionPayload(request, instruction) }.getOrElse {
            permissionSession.finishInstruction(request.id)
            _uiState.update { state -> state.copy(error = it.message) }; return
        }
        val configuration = checkNotNull(remoteObservationConfiguration) { "Permission owner has no captured Host configuration" }
        executionScope.launch {
            try {
                webAgentProvider.submitHostPermissionDecision(payload, configuration)
                if (permissionSession.findHostRequest(request.id) != null) {
                    permissionSession.removeHostRequest(request.id)
                    val remaining = permissionSession.hostRequests()
                    _uiState.update { it.copy(pendingPermissionRequests = remaining, permissionRequest = remaining.firstOrNull()) }
                }
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                if (permissionSession.findHostRequest(request.id) != null) {
                    _uiState.update { it.copy(error = error.message) }
                }
            } finally {
                permissionSession.finishInstruction(request.id)
            }
        }
    }

    fun provideLocalPermissionInstruction(requestId: String, instruction: String) {
        if (runRegistry.current == null && !AndroidChildPermissions.owns(requestId)) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { provideLocalPermissionInstruction(requestId, instruction) } }
            return
        }
        val request = _uiState.value.agentPermissionRequest ?: return
        if (request.requestId != requestId || permissionSession.deviceToolRequest()?.id == request.requestId || !permissionSession.beginInstruction(request.requestId)) return
        val text = runCatching { com.nola.core.normalizedPermissionInstruction(request.requestId, request.permissionInstructions, instruction) }.getOrElse {
            permissionSession.finishInstruction(request.requestId)
            _uiState.update { state -> state.copy(error = it.message) }; return
        }
        completeLocalAgentPermission(AndroidAgentPermissionDecision(request.requestId,
            com.nola.core.AndroidAgentPermissionUserDecision.ProvideInstruction, instruction = text))
    }

    fun resolvePermission(requestId: String, allow: Boolean, rememberForConversation: Boolean = false) {
        if (runRegistry.current == null) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { resolvePermission(requestId, allow, rememberForConversation) } }
            return
        }
        val request = permissionSession.takeHostRequest(requestId) ?: return
        val configuration = checkNotNull(remoteObservationConfiguration) { "Permission owner has no captured Host configuration" }
        val permissionTrace = traceCoordinator.permissionResolved(request.id, request.toolName, allow)
        if (permissionTrace != null) {
            executionScope.launch { applyTraceEvent(permissionTrace) }
        }
        val remaining = permissionSession.hostRequests()
        _uiState.update {
            it.copy(
                pendingPermissionRequests = remaining,
                permissionRequest = remaining.firstOrNull(),
            )
        }

        val deviceRequest = permissionSession.deviceToolRequest()
        if (deviceRequest != null && request.deviceBridge) {
            if (!allow) {
                completeBridgeDevicePermission(
                    AndroidAgentPermissionDecision(
                        deviceRequest.id,
                        com.nola.core.AndroidAgentPermissionUserDecision.Deny
                    )
                )
                return
            }
            val platformRequest = AndroidAgentPermissionRequest(
                allowConversation = false,
                requestId = deviceRequest.id,
                callId = deviceRequest.id,
                toolName = deviceRequest.toolName,
                capability = deviceRequest.capability,
                authorization = "agent_approval",
                allowsInstruction = false,
                rationale = request.reason,
                scope = deviceRequest.args.entries.joinToString(limit = 6) { "${it.key}=${it.value}" }
            )
            val conversationId = _uiState.value.conversationId
            val bridgePlan = conversationId?.let { localToolPlan(it, _uiState.value) }
            if (bridgePlan == null || !agentPermissionBroker.bindPermissionRequest(
                    platformRequest,
                    bridgePlan,
                )
            ) {
                completeBridgeDevicePermission(
                    AndroidAgentPermissionDecision(
                        platformRequest.requestId,
                        com.nola.core.AndroidAgentPermissionUserDecision.Deny
                    )
                )
                return
            }
            when (val action = agentPermissionBroker.actionFor(platformRequest, userApproved = true)) {
                is AndroidAgentPermissionAction.Immediate -> completeBridgeDevicePermission(
                    AndroidAgentPermissionDecision(
                        requestId = action.requestId,
                        decision = if (action.allow) {
                            com.nola.core.AndroidAgentPermissionUserDecision.AllowOnce
                        } else {
                            com.nola.core.AndroidAgentPermissionUserDecision.Deny
                        }
                    )
                )
                is AndroidAgentPermissionAction.Unavailable -> completeBridgeDevicePermission(
                    AndroidAgentPermissionDecision(
                        action.requestId,
                        com.nola.core.AndroidAgentPermissionUserDecision.Deny
                    )
                )
                else -> _uiState.update {
                    it.copy(agentPermissionRequest = platformRequest, agentPermissionAction = action)
                }
            }
            return
        }

        val isV2Permission = v2PermissionRequestIds.remove(request.id) || (request.nonce == null && !request.deviceBridge)
        executionScope.launch {
            try {
                _uiState.update {
                    it.copy(
                        processingState = ProcessingState.Streaming,
                        error = null
                    )
                }
                if (isV2Permission) {
                    val targetConversationId = _uiState.value.v2SessionId?.takeIf { it.isNotBlank() }
                        ?: request.sessionId?.takeIf { it.isNotBlank() }
                        ?: _uiState.value.conversationId.orEmpty()
                    webAgentProvider.v2RespondPermission(
                        conversationId = targetConversationId,
                        requestId = request.id,
                        allow = allow,
                        scope = if (rememberForConversation) "session" else "once",
                        configuration = configuration,
                    )
                } else {
                    webAgentProvider.submitHostPermissionDecision(
                        hostPermissionDecisionPayload(request, allow, rememberForConversation), configuration,
                    )
                }
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                if (isV2Permission) v2PermissionRequestIds.add(request.id)
                if (_uiState.value.conversationId == request.sessionId || _uiState.value.v2SessionId == request.sessionId) {
                    permissionSession.prependHostRequest(request)
                    val restored = permissionSession.hostRequests()
                    _uiState.update { it.copy(processingState = ProcessingState.Streaming,
                        pendingPermissionRequests = restored,
                        permissionRequest = restored.firstOrNull(),
                        error = e.message ?: context.getString(R.string.chat_agent_error)) }
                }
            }
        }
    }

    fun respondToV2UserQuestion(requestId: String, answer: String?) {
        val currentPrompt = _uiState.value.pendingUserQuestion
        val conversationId = _uiState.value.v2SessionId?.takeIf { it.isNotBlank() }
            ?: currentPrompt?.takeIf { it.requestId == requestId }?.conversationId
            ?: _uiState.value.conversationId
            ?: return
        val configuration = remoteObservationConfiguration ?: webAgentProvider.config ?: return
        _uiState.update { state ->
            if (state.pendingUserQuestion?.requestId == requestId) {
                state.copy(pendingUserQuestion = null, processingState = ProcessingState.Streaming, error = null)
            } else {
                state
            }
        }
        executionScope.launch {
            try {
                webAgentProvider.v2RespondUserInput(
                    conversationId = conversationId,
                    requestId = requestId,
                    answer = answer,
                    configuration = configuration,
                )
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                if ((_uiState.value.conversationId == conversationId || _uiState.value.v2SessionId == conversationId) && currentPrompt != null) {
                    _uiState.update {
                        it.copy(
                            pendingUserQuestion = currentPrompt,
                            error = e.message ?: context.getString(R.string.chat_agent_error),
                        )
                    }
                }
            }
        }
    }

    suspend fun submitV2UiInteraction(conversationId: String, interaction: JsonObject): JsonObject =
        withContext(Dispatchers.IO) {
            val configuration = remoteObservationConfiguration ?: webAgentProvider.config
                ?: error("Web Agent Host is not configured")
            val targetConversationId = _uiState.value.v2SessionId?.takeIf { it.isNotBlank() } ?: conversationId
            val result = webAgentProvider.v2SubmitUiInteraction(
                conversationId = targetConversationId,
                interaction = interaction,
                configuration = configuration,
            )
            val updatedSnapshot = result["snapshot"] as? JsonObject
            if (updatedSnapshot != null && (_uiState.value.conversationId == conversationId || _uiState.value.v2SessionId == targetConversationId)) {
                _uiState.update { state ->
                    state.copy(v2UiSurfaces = upsertV2UiSurface(state.v2UiSurfaces, updatedSnapshot))
                }
            }
            result
        }

    private fun clearGenerationJobIfCurrent(job: Job?) {
        turnRuntime.clearGenerationIfCurrent(job)
    }

    private suspend fun retireSubmittedDraft() {
        val session = executionSession
        if (session.draftRetired) return
        session.submittedDraft?.let { composerDrafts.submitted(it) }
        session.draftRetired = true
    }

    fun cancelGeneration(ownerKey: String, responseId: String?) {
        if (runRegistry.current == null) {
            val session = runRegistry.find(ownerKey) ?: return
            if (session.state.value.activeResponseId != responseId) return
            runRegistry.enter(session) { cancelGeneration(ownerKey, responseId) }
            return
        }
        if (executionSession.ownerKey != ownerKey || _uiState.value.activeResponseId != responseId) return
        executionSession.successorCancellationRequested = true
        _uiState.value.conversationId?.let { conversationId ->
            remoteUiWakeSuppressed.add(conversationId)
            if (remoteUiWakeConversation == conversationId) remoteUiWakeJob?.cancel()
        }
        if (!executionSession.dispatchStarted && remoteSubmissionIntent == null &&
            _uiState.value.currentModel?.source != ModelInfo.ModelSource.WebHost) {
            val session = executionSession
            val visible = displayedState.value.activeResponseId == responseId
            session.runtime.cancelGeneration()
            session.state.value = session.preparationBase
            if (visible) displayedState.value = session.preparationBase
            return
        }
        val pending = remoteSubmissionIntent
        val capturedConfiguration = remoteObservationConfiguration?.copy()
        if (pending != null && !remoteSubmissionAdmitted && pending.phase !in setOf(
                com.nola.data.remote.api.RemoteSubmissionPhase.Rejected, com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled)) {
            turnRuntime.cancelGeneration()
            executionScope.launch {
                try {
                    // Bind/admission may have committed while the generation
                    // was being stopped. Choose the operation from the exact
                    // durable intent, never from a stale prepared UI cache.
                    val durable = withContext(Dispatchers.IO) {
                        conversationRepo.readRemoteSubmissionIntent(pending.identity.conversationId)
                    } ?: error("Submission cancellation requires its durable intent")
                    require(durable.identity == pending.identity) { "Submission authority changed" }
                    fun current() = _uiState.value.conversationId == pending.identity.conversationId &&
                        remoteSubmissionIntent?.identity == pending.identity
                    if (durable.requestDigest == null) {
                        val abandoned = withContext(Dispatchers.IO) { conversationRepo.abandonPreparedRemoteSubmission(pending.identity) }
                        require(abandoned.phase == com.nola.data.remote.api.RemoteSubmissionPhase.Cancelled && abandoned.requestDigest == null)
                        if (current()) {
                            remoteSubmissionIntent = abandoned
                            remoteConversationObserver.reset()
                            val base = executionSession.preparationBase
                            val visible = displayedState.value.activeResponseId == responseId
                            _uiState.value = base
                            if (visible) displayedState.value = base
                        }
                    } else {
                        val configuration = checkNotNull(capturedConfiguration)
                        val digest = checkNotNull(durable.requestDigest)
                        val query = com.nola.data.remote.api.remoteSubmissionReconciliationRequest(durable.identity, digest, cancel = true)
                        val result = webAgentProvider.submissionTransport(configuration)(query).getValue("result")
                        val receipt = (result as? JsonObject)?.let {
                            com.nola.data.remote.api.decodeRemoteSubmissionReceipt(it, durable.identity, digest)
                        }
                        if (result is JsonObject) acceptRemoteSubmissionReceipt(result, configuration, durable)
                        // If admission won the cancel race, cancel that exact
                        // live Run normally; a pending receipt never means idle.
                        if (receipt?.phase == com.nola.data.remote.api.RemoteSubmissionPhase.Admitted)
                            webAgentProvider.cancelAdmittedSubmission(durable.identity, configuration)
                        else if (result !is JsonObject && current())
                            _uiState.update { it.copy(processingState = ProcessingState.BuildingContext) }
                    }
                } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (_uiState.value.conversationId == pending.identity.conversationId && remoteSubmissionIntent?.identity == pending.identity)
                        _uiState.update { it.copy(processingState = ProcessingState.BuildingContext, error = failure.message) }
                }
            }
            return
        }
        val state = _uiState.value
        val convId = state.conversationId
        val usedWebHost = state.currentModel?.source == ModelInfo.ModelSource.WebHost
        val cancelIdentity = remoteAdmissionReceipt?.identity ?: pending?.takeIf {
            it.phase == com.nola.data.remote.api.RemoteSubmissionPhase.Admitted }?.identity
        val localCoordinator = turnRuntime.deviceCoordinator()
        if (localCoordinator == null && (!usedWebHost || cancelIdentity == null)) turnRuntime.cancelGeneration()
        permissionSession.clear()
        _uiState.update { it.copy(permissionRequest = null, pendingPermissionRequests = emptyList()) }
        executionScope.launch {
            if (localCoordinator != null) {
                localCoordinator.requestCancellation("user_requested")
                return@launch
            }
            if (usedWebHost && convId != null) {
                try {
                    if (cancelIdentity != null && cancelIdentity.conversationId == convId) {
                        webAgentProvider.cancelAdmittedSubmission(cancelIdentity, capturedConfiguration)
                    } else {
                        val cfg = capturedConfiguration ?: webAgentProvider.config
                        if (cfg != null) {
                            webAgentProvider.v2CancelConversation(readHostSession(cfg.expectedHostId, convId) ?: convId, cfg)
                        }
                        _uiState.update {
                            if (it.conversationId == convId) it.copy(processingState = ProcessingState.Idle) else it
                        }
                    }
                } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                catch (failure: Exception) {
                    if (_uiState.value.conversationId == convId &&
                        (cancelIdentity == null || remoteAdmissionReceipt?.identity == cancelIdentity))
                        _uiState.update { it.copy(error = failure.message) }
                }
                return@launch
            }
        }
    }

    fun cancelAgentTask(ownerKey: String, taskId: String) {
        if (runRegistry.current == null) {
            runRegistry.find(ownerKey)?.let { runRegistry.enter(it) { cancelAgentTask(ownerKey, taskId) } }
            return
        }
        if (executionSession.ownerKey != ownerKey) return
        val state = _uiState.value
        val control = state.messages.flatMap { it.canonicalParts }.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Task>()
            .lastOrNull { it.taskId == taskId } ?: return
        if (!control.supportsCancellation) return
        val parentRun = control.parentRunId?.takeIf { it.isNotBlank() } ?: return
        if (state.currentModel?.source == ModelInfo.ModelSource.WebHost && state.conversationId != null) {
            val configuration = remoteObservationConfiguration ?: return
            executionScope.launch {
                runCatching { webAgentProvider.cancelAgentTask(state.conversationId, parentRun, taskId, configuration) }
                    .onSuccess {
                        // Receipt confirms the request, not task termination.
                        if (_uiState.value.conversationId != state.conversationId || executionSession.ownerKey != ownerKey
                            || remoteObservationConfiguration != configuration) return@onSuccess
                        observeRemoteAgentTasks(state.conversationId)
                    }
                    .onFailure { error ->
                        if (error is kotlinx.coroutines.CancellationException) throw error
                        if (_uiState.value.conversationId != state.conversationId || executionSession.ownerKey != ownerKey
                            || remoteObservationConfiguration != configuration) return@onFailure
                        _uiState.update { it.copy(error = error.message ?: context.getString(R.string.chat_agent_error)) }
                    }
            }
            return
        }
        if (turnRuntime.deviceCoordinator()?.requestTaskCancellation(taskId) == true) return
        _uiState.update { it.copy(error = context.getString(R.string.chat_task_control_unavailable)) }
    }

    private val remoteGuidanceMutex = kotlinx.coroutines.sync.Mutex()

    fun sendAgentTaskGuidance(ownerKey: String, taskId: String, message: String): kotlinx.coroutines.Deferred<Boolean> {
        fun unavailable() = kotlinx.coroutines.CompletableDeferred(false)
        if (runRegistry.current == null) {
            val owner = runRegistry.find(ownerKey) ?: return unavailable()
            return runRegistry.enter(owner) { sendAgentTaskGuidance(ownerKey, taskId, message) }
        }
        if (executionSession.ownerKey != ownerKey) return unavailable()
        val content = message.trim()
        if (content.isEmpty()) return unavailable()
        val state = _uiState.value
        val control = state.messages.flatMap { it.canonicalParts }.filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.Task>()
            .lastOrNull { it.taskId == taskId } ?: return unavailable()
        if (!control.supportsGuidance) return unavailable()
        val parentRun = control.parentRunId?.takeIf { it.isNotBlank() } ?: return unavailable()
        if (state.currentModel?.source == ModelInfo.ModelSource.WebHost && state.conversationId != null) {
            return executionScope.async {
                try {
                    remoteGuidanceMutex.withLock {
                        val cfg = checkNotNull(remoteObservationConfiguration)
                        val intent = buildJsonObject {
                            put("hostId", cfg.expectedHostId); put("conversationId", state.conversationId)
                            put("parentRunId", parentRun); put("taskId", taskId)
                            put("messageId", UUID.randomUUID().toString()); put("submittedAtMs", System.currentTimeMillis())
                            put("message", content)
                        }
                        val key = kotlinx.serialization.json.JsonArray(listOf(cfg.expectedHostId, state.conversationId, parentRun, taskId).map(::JsonPrimitive)).toString()
                        val preferences = context.getSharedPreferences("task-guidance-intents", android.content.Context.MODE_PRIVATE)
                        val status = com.nola.data.local.submitRecoverableTaskGuidance(intent,
                            read = { preferences.getString(key, null) },
                            write = { value -> check((if (value == null) preferences.edit().remove(key)
                                else preferences.edit().putString(key, value)).commit()) { "Cannot persist guidance intent" } },
                            transport = { frozen, receipt -> webAgentProvider.taskGuidanceCommand(frozen, receipt, cfg) })
                        check(status != "rejected") { "The original task instruction was rejected" }
                    }
                    observeRemoteAgentTasks(state.conversationId)
                    true
                } catch (error: Exception) {
                    if (error is kotlinx.coroutines.CancellationException) throw error
                    _uiState.update { it.copy(error = error.message ?: context.getString(R.string.chat_agent_error)) }
                    false
                }
            }
        }
        val coordinator = turnRuntime.deviceCoordinator()
        return executionScope.async {
            val accepted = coordinator?.sendTaskGuidance(taskId, content) == true
            if (!accepted) _uiState.update { it.copy(error = context.getString(R.string.chat_task_control_unavailable)) }
            accepted
        }
    }

    private fun applyRemoteTaskProjection(event: SseEvent.TaskProjection) {
        val projection = event.projection ?: return
        val state = _uiState.value
        if (event.conversationId != state.conversationId || projection.revision < state.coreTaskProjection.revision) return
        remoteTaskObservation.advance(event.cursor)
        _uiState.update { it.copy(coreTaskProjection = projection) }
    }

    private fun observeRemoteAgentTasks(conversationId: String) {
        remoteTaskObservation.observe(
            scope = executionScope,
            provider = webAgentProvider,
            configuration = checkNotNull(remoteObservationConfiguration),
            conversationId = conversationId,
            isOwnerActive = { !executionSession.retired && _uiState.value.conversationId == conversationId },
            shouldReconnect = {
                val state = _uiState.value
                state.coreTaskProjection.orderedTasks.any { it.state.isActive } ||
                    state.processingState !in setOf(ProcessingState.Idle, ProcessingState.Error)
            },
            onProjection = ::applyRemoteTaskProjection,
        )
    }

    fun dismissError(expected: ChatUiState): Boolean {
        val current = _uiState.value
        if (current !== expected) return false
        // Closing a banner is not cancellation, completion or release of the
        // active response. Only the runtime owns those fields.
        return _uiState.compareAndSet(current, current.dismissingDisplayedError(expected))
    }

    private fun markHostPermissionResumeUnavailable() {
        _uiState.update {
            it.copy(
                processingState = ProcessingState.Error,
                currentResponse = "",
                activeResponseId = null,
                activeResponseStartedAt = null,
                thinking = "",
                thinkingFinished = true,
                permissionRequest = null,
                pendingPermissionRequests = emptyList(),
                error = context.getString(R.string.chat_agent_permission_resume_failed)
            )
        }
    }

    private fun markConversationUnavailable() {
        _uiState.update {
            it.copy(
                processingState = ProcessingState.Error,
                currentResponse = "",
                activeResponseId = null,
                activeResponseStartedAt = null,
                thinking = "",
                thinkingFinished = true,
                permissionRequest = null,
                pendingPermissionRequests = emptyList(),
                error = context.getString(R.string.chat_conversation_unavailable)
            )
        }
    }

    internal val preparingExpertPicker = MutableStateFlow(false)
    private val conversationCreationMutex = kotlinx.coroutines.sync.Mutex()
    private val preparedDraftConversations = mutableMapOf<String, String>()

    /** Source provenance comes from Core, never the current model or endpoint. */
    internal suspend fun prepareExpertPicker(): Boolean {
        if (preparingExpertPicker.value || composerDrafts.initializing.value || displayedState.value.processingState.blocksInput) return false
        preparingExpertPicker.value = true
        val state = displayedState.value
        val generation = conversationLoads.currentGeneration()
        fun current() = conversationLoads.currentGeneration() == generation && displayedState.value.conversationId == state.conversationId
        try {
            val id = state.conversationId

            val repository = expertSelectionRepository(id, true)
            val (directory, members) = withContext(Dispatchers.IO) {
                mainExpertDirectory(repository, inline = true) to id?.let { repository.forConversation(it)?.room?.room?.participants }.orEmpty()
            }
            if (!current()) return false
            captureExpertPickerSource(repository)
            pickerMembers.value = members.filter { it.active }.associateBy { it.agentId }
            conversationAgentDirectory.value = directory
            return true
        } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (error: Exception) {
            if (current()) composerDrafts.forConversation(state.conversationId).notice.value =
                error.localizedMessage ?: context.getString(R.string.expert_source_unavailable)
            return false
        } finally { preparingExpertPicker.value = false }
    }

    /** Resolve experts against their owning Host; names never choose a transport. */
    internal suspend fun expertRoomRepository(parentId: String? = displayedState.value.conversationId, expectedStoreEpoch: String? = null): com.nola.data.repository.AgentRoomRepository = withContext(Dispatchers.IO) {
        val conversation = parentId?.let { conversationRepo.getConversation(it) }
        val binding = conversation?.executionHostBinding ?: if (parentId == null) displayedState.value.executionHostBinding else null
        if (binding == null) return@withContext agentRooms
        val configuration = resolveBoundHost(binding)
        val source = parentId?.let { applicationRuntime.conversationSourceOwner(it) }
        if (source is com.nola.data.local.ConversationSourceOwner.Remote) require(source.hostId == configuration.expectedHostId)
        if (source == com.nola.data.local.ConversationSourceOwner.Local && parentId != null) {
            val local = applicationRuntime.loadConversationProjectionWindowV1(parentId, null, limit = 1, contentByteLimit = 4096)
            require(local.totalMessageCount == 0L) { "Local history cannot be used as a remote expert parent" }
        }
        check(source != com.nola.data.local.ConversationSourceOwner.Unavailable)
        var capturedEpoch = (source as? com.nola.data.local.ConversationSourceOwner.Remote)?.storeEpoch ?: expectedStoreEpoch
        require(expectedStoreEpoch == null || capturedEpoch == expectedStoreEpoch)
        lateinit var repository: com.nola.data.repository.AgentRoomRepository
        repository = com.nola.data.repository.AgentRoomRepository { envelope ->
            // Repository clients run inside their existing IO dispatcher. The
            // captured connection is revalidated before and after each request.
            kotlinx.coroutines.runBlocking {
                require(resolveBoundHost(binding) == configuration)
                if (parentId != null) require(applicationRuntime.conversationSourceOwner(parentId) == source)
                val response = webAgentProvider.expertRoomReply(envelope.getValue("command").jsonObject, configuration, capturedEpoch)
                val epoch = response.getValue("source_store_epoch").jsonPrimitive.content
                require(capturedEpoch == null || capturedEpoch == epoch); capturedEpoch = epoch
                require(resolveBoundHost(binding) == configuration)
                if (parentId != null) require(applicationRuntime.conversationSourceOwner(parentId) == source)
                repository.confirmRemoteSource(configuration.expectedHostId, epoch)
                response.getValue("result")
            }
        }
        repository.remoteHostId = configuration.expectedHostId
        repository
    }

    /** All history observations retain their persisted source instead of using today's selected model. */
    private suspend fun expertHistoryCommand(parentId: String, command: JsonObject, expectedSource: String? = null): JsonObject = withContext(Dispatchers.IO) {
        val source = applicationRuntime.conversationSourceOwner(parentId)
        require(expectedSource == null || source.toString() == expectedSource) { "Expert history source changed" }
        val result = when (source) {
            com.nola.data.local.ConversationSourceOwner.Local -> applicationRuntime.execute(buildJsonObject {
                put("type", "agent_room"); put("command", command)
            }).jsonObject
            is com.nola.data.local.ConversationSourceOwner.Remote -> {
                val parent = checkNotNull(conversationRepo.getConversation(parentId))
                val configuration = resolveBoundHost(checkNotNull(parent.executionHostBinding))
                require(configuration.expectedHostId == source.hostId)
                val response = webAgentProvider.readExpertHistoryCommand(command, configuration, source.storeEpoch)
                require(resolveBoundHost(checkNotNull(parent.executionHostBinding)) == configuration) { "Expert history connection changed" }
                response
            }
            else -> error(context.getString(R.string.expert_source_unavailable))
        }
        require(applicationRuntime.conversationSourceOwner(parentId) == source) { "Expert history source changed" }
        result
    }

    internal suspend fun loadExpertChildren(parentId: String, offset: Int): com.nola.data.repository.ExpertChildConversationPage {
        val source = withContext(Dispatchers.IO) { applicationRuntime.conversationSourceOwner(parentId) }
        return Json.decodeFromJsonElement<com.nola.data.repository.ExpertChildConversationPage>(expertHistoryCommand(parentId, buildJsonObject {
            put("type", "child_conversations"); put("conversation_id", parentId); put("offset", offset); put("limit", 40)
        }, source.toString())).validated(parentId, offset).copy(sourceIdentity = source.toString())
    }

    internal suspend fun namedExpertHistory(parentId: String, ref: InlineExpertMention): Pair<ExpertHistoryTarget, String>? {
        val pinned = ref.member ?: return null
        val first = loadExpertChildren(parentId, 0)
        val roomId = first.roomId ?: return null
        var page = first
        while (true) {
            if (page.roomId != roomId || page.roomRevision != first.roomRevision || page.sourceIdentity != first.sourceIdentity) return null
            val child = page.children.singleOrNull(ref::matchesHistory)
            if (child != null) {
                val description = if (!child.isCurrent) "" else try {
                    val repository = expertRoomRepository(parentId)
                    val room = withContext(Dispatchers.IO) { repository.forConversation(parentId)?.room }
                    room?.takeIf { it.room.id == roomId && it.revision == page.roomRevision }?.room?.participants
                        ?.singleOrNull { it.id == pinned.id && it.agentId == ref.agentId && it.agentRevision == ref.revision &&
                            it.contextConversationId == pinned.contextConversationId }?.description.orEmpty()
                } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                  catch (_: Exception) { "" }
                return ExpertHistoryTarget(parentId, roomId, page.sourceIdentity, child) to description
            }
            page = loadExpertChildren(parentId, page.nextOffset ?: return null)
        }
    }

    internal suspend fun loadExpertChildHistory(target: ExpertHistoryTarget, before: Long?): ExpertHistoryWindow {
        val response = expertHistoryCommand(target.parentId, buildJsonObject {
            put("type", "child_history"); put("conversation_id", target.parentId); put("child_conversation_id", target.child.conversationId)
            put("before_ordinal", before?.let(::JsonPrimitive) ?: JsonNull); put("limit", 50); put("content_byte_limit", 262144)
        }, target.sourceIdentity)
        require(response["version"]?.jsonPrimitive?.content == "1" && response["conversation_id"]?.jsonPrimitive?.content == target.parentId &&
            response["child_conversation_id"]?.jsonPrimitive?.content == target.child.conversationId && response["room_id"]?.jsonPrimitive?.content == target.roomId)
        val returnedSource = com.nola.data.local.decodeConversationSourceOwner(response.getValue("source_owner").jsonObject)
        require(returnedSource == com.nola.data.local.ConversationSourceOwner.Local || returnedSource.toString() == target.sourceIdentity)
        val window = com.nola.domain.projection.ConversationProjectionV1Decoder.decodeWindow(response.getValue("window"))
        require(window.projection.conversationId == target.child.conversationId)
        return ExpertHistoryWindow(window, com.nola.data.repository.expertChildExchangeSenders(response["expert_exchanges"], target.parentId, target.child))
    }

    internal suspend fun continueExpertChildHistory(parentId: String, child: com.nola.data.repository.ExpertChildConversation) {
        val repository = expertSelectionRepository(parentId)
        val association = withContext(Dispatchers.IO) { repository.forConversation(parentId) }
        val member = checkNotNull(association?.let { com.nola.data.repository.currentExpertChildMember(it.room, child) }) {
            context.getString(R.string.expert_profile_changed)
        }
        loadConversation(parentId)
        val generation = conversationLoads.currentGeneration()
        kotlinx.coroutines.withTimeout(10_000) { displayedState.first {
            it.conversationId == parentId && it.processingState != ProcessingState.BuildingContext
        } }
        check(conversationLoads.currentGeneration() == generation && displayedState.value.conversationId == parentId)
        captureExpertPickerSource(repository)
        val draft = composerDrafts.awaitReady(parentId)
        check(conversationLoads.currentGeneration() == generation && displayedState.value.conversationId == parentId)
        val text = draft.text.value
        val caret = androidx.compose.ui.text.TextRange((draft.caret ?: text.length).coerceIn(0, text.length))
        val inserted = insertExpert(androidx.compose.ui.text.input.TextFieldValue(text, caret), draft.mentions, caret,
            InlineExpertMention(0, "@${member.displayName}", member.agentId, member.agentRevision, member, expertPickerSource?.host, expertPickerSource?.epoch))
        draft.caret = inserted.value.selection.end
        draft.setContent(inserted.value.text, inserted.mentions)
    }

    internal suspend fun canReadExpertSource(id: String): Boolean = runCatching { expertRoomRepository(id); true }.getOrElse {
        if (it is kotlinx.coroutines.CancellationException) throw it
        false
    }

    internal suspend fun isLocalExpertSource(id: String): Boolean = withContext(Dispatchers.IO) {
        applicationRuntime.conversationSourceOwner(id) == com.nola.data.local.ConversationSourceOwner.Local
    }

    private suspend fun ensureConversation(state: ChatUiState, draftOwner: String): String = conversationCreationMutex.withLock {
        state.conversationId?.let { return@withLock it }
        preparedDraftConversations[draftOwner]?.let {
            composerDrafts.bindConversation(it, draftOwner)
            return@withLock it
        }
        val conv = conversationRepo.createConversation(
            modelInfo = state.currentModel,
            promptResponseStyle = state.responseStyleOverride,
            permissionMode = state.permissionMode.wireValue,
            parameters = state.effectiveGenerationParameters,
            hostBinding = state.executionHostBinding,
        )
        preparedDraftConversations[draftOwner] = conv.id
        composerDrafts.bindConversation(conv.id, draftOwner)
        loadConversations()
        conv.id
    }

    private suspend fun updateFirstMessageTitle(owner: String, firstMessageId: String, text: String) {
        val title = text.replace(CHAT_ATTACHMENT_BLOCK, "").trim().take(80)
        try {
            if (withContext(Dispatchers.IO) { applicationRuntime.updateFirstMessageTitle(owner, firstMessageId, title) }) {
                loadConversations()
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            Log.w("NolaAgentProtocol", "Unable to update first-message title", failure)
        }
    }

    private suspend fun finalizeResponse(convId: String, currentOwner: () -> Boolean = { _uiState.value.conversationId == convId }) {
        if (!currentOwner()) return
        Log.d("NolaAgentProtocol", "ui_finalize_begin conversation=$convId")
        val toolEvents = _uiState.value.toolEvents
        val responseId = checkNotNull(_uiState.value.activeResponseId) {
            "Canonical assistant completion has no active message identity"
        }
        val canonicalMessages = conversationRepo.getMessages(convId)
        if (!currentOwner() || _uiState.value.activeResponseId != responseId) return
        val assistantMsg = checkNotNull(canonicalMessages.lastOrNull {
            it.id == responseId && it.role == ChatRole.assistant
        }) {
            "Canonical conversation projection is missing assistant message $responseId"
        }
        val response = assistantMsg.text
        if (response.isNotEmpty()) {
            _uiState.update {
                it.copy(messages = canonicalMessages).completeAssistantTurn(assistantMsg)
            }
            Log.d(
                "NolaAgentProtocol",
                "ui_finalize_idle conversation=$convId response=${assistantMsg.id} characters=${response.length}",
            )
            executionScope.launch {
                delay(2_500)
                _uiState.update { state ->
                    if (state.revealResponseId == assistantMsg.id) {
                        state.copy(revealResponseId = null)
                    } else {
                        state
                    }
                }
            }
            recordPersonalIntelligenceTurn(convId, response, toolEvents.isNotEmpty())
        } else {
            _uiState.update {
                it.copy(
                    processingState = ProcessingState.Idle,
                    currentResponse = "",
                    activeResponseId = null,
                    activeTurnId = null,
                    activeResponseStartedAt = null,
                    thinking = "",
                    thinkingFinished = false,
                    toolEvents = emptyList(),
                    answerSources = emptyList(),
                )
            }
        }
        capabilityActivationBuffer.clear(convId)
        if (currentOwner()) runCatching { refreshPermissionOwnerProjection() }

        val firstUser = canonicalMessages.firstOrNull { it.role == ChatRole.user }
        val answeredUser = canonicalMessages.takeWhile { it.id != responseId }.lastOrNull { it.role == ChatRole.user }
        if (firstUser != null && answeredUser?.id == firstUser.id) {
            val query = firstUser.meta?.get("nola.displayMessage")?.takeIf(String::isNotBlank) ?: firstUser.text
            updateFirstMessageTitle(convId, firstUser.id, query)
        }

        checkAutoCompact(convId)
        traceCoordinator.reset()
    }

    private fun recordPersonalIntelligenceTurn(
        conversationId: String,
        response: String,
        usedTools: Boolean
    ) {
        val state = _uiState.value
        val model = state.currentModel
        val eventId = UUID.randomUUID().toString()
        val route = model?.source?.name?.lowercase() ?: "unknown"
        executionScope.launch(Dispatchers.IO) {
            runCatching {
                PersonalIntelligenceCore(applicationRuntime).ingest(
                    IntelligenceIngestRequest(
                        events = listOf(
                            IntelligenceEvent(
                                id = eventId,
                                source = "conversation",
                                kind = "turn.completed",
                                subject = conversationId,
                                summary = response.trim().take(280),
                                importance = if (usedTools) 0.55 else 0.35,
                                confidence = 1.0,
                                tags = listOf("conversation", route),
                                dedupe_key = "conversation:$conversationId:$eventId"
                            )
                        ),
                        outcomes = listOf(
                            IntelligenceTaskOutcome(
                                task_type = if (usedTools) "agent_conversation" else "conversation",
                                route = route,
                                model = model?.name.orEmpty(),
                                success = response.isNotBlank()
                            )
                        )
                    )
                )
            }
        }
    }

    fun resolveLocalAgentPermission(
        requestId: String,
        userApproved: Boolean,
        rememberForConversation: Boolean = false,
    ) {
        if (runRegistry.current == null && !AndroidChildPermissions.owns(requestId)) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { resolveLocalAgentPermission(requestId, userApproved, rememberForConversation) } }
            return
        }
        val request = _uiState.value.agentPermissionRequest ?: return
        if (request.requestId != requestId) return
        permissionSession.rememberLocalGrant(userApproved && rememberForConversation)
        when (val action = agentPermissionBroker.actionFor(request, userApproved)) {
            is AndroidAgentPermissionAction.Immediate -> completeLocalAgentPermission(
                AndroidAgentPermissionDecision(
                    requestId = action.requestId,
                    decision = if (action.allow) {
                        com.nola.core.AndroidAgentPermissionUserDecision.AllowOnce
                    } else {
                        com.nola.core.AndroidAgentPermissionUserDecision.Deny
                    }
                ).withConversationScope(
                    action.allow && permissionSession.shouldRememberLocalGrant()
                )
            )
            is AndroidAgentPermissionAction.Unavailable -> {
                _uiState.update { it.copy(error = action.reason) }
                completeLocalAgentPermission(
                    AndroidAgentPermissionDecision(
                        action.requestId,
                        com.nola.core.AndroidAgentPermissionUserDecision.Deny
                    )
                )
            }
            else -> _uiState.update { it.copy(agentPermissionAction = action) }
        }
    }

    fun completeLocalRuntimePermission(requestId: String, grants: Map<String, Boolean>) {
        if (runRegistry.current == null && !AndroidChildPermissions.owns(requestId)) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { completeLocalRuntimePermission(requestId, grants) } }
            return
        }
        val decision = agentPermissionBroker.runtimeResult(requestId, grants)
        completeLocalAgentPermission(
            decision.withConversationScope(
                decision.isAllowed && permissionSession.shouldRememberLocalGrant()
            )
        )
    }

    fun completeLocalScopedResource(
        requestId: String,
        uri: Uri?,
        write: Boolean,
        persistable: Boolean = false
    ) {
        if (runRegistry.current == null && !AndroidChildPermissions.owns(requestId)) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { completeLocalScopedResource(requestId, uri, write, persistable) } }
            return
        }
        val decision = agentPermissionBroker.scopedResourceResult(
                requestId = requestId,
                uri = uri,
                write = write,
                persistable = persistable || permissionSession.shouldRememberLocalGrant(),
            )
        completeLocalAgentPermission(
            decision.withConversationScope(
                decision.isAllowed && permissionSession.shouldRememberLocalGrant()
            )
        )
    }

    fun completeLocalHealthPermission(requestId: String, granted: Boolean) {
        if (runRegistry.current == null && !AndroidChildPermissions.owns(requestId)) {
            runRegistry.permissionOwner(requestId)?.let { runRegistry.enter(it) { completeLocalHealthPermission(requestId, granted) } }
            return
        }
        completeLocalAgentPermission(
            AndroidAgentPermissionDecision(
                requestId = requestId,
                decision = if (granted) {
                    com.nola.core.AndroidAgentPermissionUserDecision.AllowOnce
                } else {
                    com.nola.core.AndroidAgentPermissionUserDecision.Deny
                }
            ).withConversationScope(granted && permissionSession.shouldRememberLocalGrant())
        )
    }

    private fun completeLocalAgentPermission(decision: AndroidAgentPermissionDecision) {
        if (decision.requestId in displayedChildPermissionIds || AndroidChildPermissions.owns(decision.requestId)) {
            // A system picker can return after its child effect was cancelled.
            // Never route that late result to the foreground parent's channel.
            AndroidChildPermissions.resolve(decision)
            permissionSession.clearLocalGrantPreference()
            permissionSession.finishInstruction(decision.requestId)
            _uiState.update { state ->
                if (state.agentPermissionRequest?.requestId == decision.requestId)
                    state.copy(agentPermissionRequest = null, agentPermissionAction = null) else state
            }
            return
        }

        if (permissionSession.deviceToolRequest()?.id == decision.requestId) {
            completeBridgeDevicePermission(decision)
            return
        }
        _uiState.update { it.copy(agentPermissionAction = null) }
        permissionSession.clearLocalGrantPreference()
        executionScope.launch {
            val coordinator = turnRuntime.deviceCoordinator()
            if (coordinator != null) {
                runCatching { coordinator.resolvePermission(decision) }
                    .onSuccess {
                        permissionSession.finishInstruction(decision.requestId)
                        _uiState.update {
                            if (permissionResponseStillCurrent(decision.requestId, it.agentPermissionRequest?.requestId))
                                it.copy(agentPermissionRequest = null, agentPermissionAction = null) else it
                        }
                    }
                    .onFailure { error ->
                        if (error is kotlinx.coroutines.CancellationException) throw error
                        permissionSession.finishInstruction(decision.requestId)
                        _uiState.update {
                            if (permissionResponseStillCurrent(decision.requestId, it.agentPermissionRequest?.requestId))
                                it.copy(agentPermissionAction = null, error = error.message) else it
                        }
                }
                return@launch
            }
            _uiState.update {
                it.copy(
                    agentPermissionRequest = null,
                    agentPermissionAction = null,
                    error = context.getString(R.string.chat_agent_error),
                )
            }
        }
    }

    private fun completeBridgeDevicePermission(decision: AndroidAgentPermissionDecision) {
        val request = permissionSession.takeDeviceToolRequest() ?: return
        val configuration = remoteObservationConfiguration?.copy()
        val stateSnapshot = _uiState.value
        val remaining = permissionSession.hostRequests()
        _uiState.update {
            it.copy(
                agentPermissionRequest = null,
                agentPermissionAction = null,
                pendingPermissionRequests = remaining,
                permissionRequest = remaining.firstOrNull(),
            )
        }
        executionScope.launch {
            val result = executeBridgeDeviceCall(
                request = request,
                decision = decision,
                executor = { hostCall ->
                    val localConvId = stateSnapshot.conversationId ?: request.sessionId
                    val bridgePlan = localToolPlan(localConvId, stateSnapshot)
                    withContext(Dispatchers.IO) {
                        agentHostExecutor.execute(hostCall, bridgePlan)
                    }
                },
            )
            val payload = mapOf(
                "protocol_version" to request.protocolVersion,
                "nonce" to request.nonce,
                "request_id" to request.id,
                "host_id" to request.hostId,
                "client_id" to request.clientId,
                "session_id" to request.sessionId,
                "tool_name" to request.toolName,
                "output" to result.output,
                "error" to result.isError
            )
            try {
                webAgentProvider.submitDeviceToolResult(payload, configuration)
            } catch (error: Exception) {
                if (error is kotlinx.coroutines.CancellationException) throw error
                _uiState.update {
                    if (it.conversationId != request.sessionId && it.v2SessionId != request.sessionId) it else it.copy(
                        processingState = ProcessingState.BuildingContext,
                        error = error.message ?: context.getString(R.string.chat_agent_error)
                    )
                }
            }
        }
    }

    private fun ensureTraceSession(convId: String): AgentTraceSession {
        val binding = traceCoordinator.ensure(
            conversationId = convId,
            activeTurnId = _uiState.value.activeTurnId,
            messageId = _uiState.value.activeResponseId,
        )
        if (_uiState.value.activeTurnId != binding.turnId) {
            _uiState.update { it.copy(activeTurnId = binding.turnId) }
        }
        return binding.session
    }

    private suspend fun applyTraceEvent(
        event: AgentTraceEvent,
    ) {
        val projection = traceCoordinator.accept(
            event = event,
            activeTurnId = _uiState.value.activeTurnId,
            messageId = _uiState.value.activeResponseId,
        ) ?: return
        _uiState.update { state ->
            state.applyTraceProjection(projection)
        }
    }

    private fun applyCoreTaskUpdate(event: com.nola.core.AgentCoreTaskUpdate) {
        _uiState.update { state -> turnRuntime.reduceTask(state, event) ?: state }
    }

    private suspend fun applyAgentCoreSnapshot(snapshot: AgentCoreSnapshot) {
        val owner = _uiState.value.conversationId ?: return
        val usage = runCatching { withContext(Dispatchers.IO) { conversationRepo.getContextUsage(owner) } }.getOrNull()
        _uiState.update { state ->
            if (state.conversationId != owner) state else turnRuntime.applySnapshot(
                usage?.let { state.applyCanonicalContextUsage(it) } ?: state, snapshot)
        }
        if (executionSession.permissionOwnerProjectionRunId != snapshot.turnId) {
            executionSession.permissionOwnerProjectionRunId = snapshot.turnId
            runCatching { refreshPermissionOwnerProjection() }
        }
    }

    private fun finalizeResponseAnyway() {
        _uiState.update { it.copy(
            processingState = ProcessingState.Idle,
            currentResponse = (it.currentResponse + "...").trim()
        )}
    }

    private suspend fun checkAutoCompact(convId: String, mode: String = "normal") {
        val conv = conversationRepo.getConversation(convId) ?: return
        val messages = conversationRepo.getActiveMessages(convId)
        val state = _uiState.value
        val route = when (state.currentModel?.source) {
            ModelInfo.ModelSource.Local -> AndroidContextRoute.Local
            ModelInfo.ModelSource.WebHost -> AndroidContextRoute.Web
            else -> AndroidContextRoute.Cloud
        }
        val tools = when (route) {
            AndroidContextRoute.Local -> localToolPlan(convId, state).tools
            AndroidContextRoute.Web -> webAgentProvider.contextToolSchemas(
                convId,
                state.hybridDeviceToolsEnabled,
                state.hybridDeviceCapabilities,
                state.permissionMode,
            )
            else -> emptyList()
        }
        prepareFinalContext(
            conv = conv,
            messages = messages,
            currentUserText = messages.lastOrNull { it.role == ChatRole.user }?.text.orEmpty(),
            route = route,
            tools = tools,
            mode = mode,
        )
        val refreshedConv = withContext(Dispatchers.IO) { conversationRepo.getConversation(convId) }
        val refreshedMessages = withContext(Dispatchers.IO) { conversationRepo.getActiveMessages(convId) }
        if (mode == "force_summary" ||
            refreshedConv?.sessionMemory != conv.sessionMemory ||
            refreshedMessages.size != messages.size
        ) {
            _uiState.update { current ->
                if (current.conversationId != convId) current else current.copy(
                    sessionMemory = refreshedConv?.sessionMemory ?: current.sessionMemory,
                    messages = refreshedMessages,
                )
            }
        }
    }

    private suspend fun prepareFinalContext(
        conv: Conversation,
        messages: List<ConversationMessage>,
        currentUserText: String,
        route: AndroidContextRoute,
        tools: List<JsonElement> = emptyList(),
        mode: String = "normal",
        turnState: ChatUiState = _uiState.value,
    ): AndroidPreparedContext {
        val state = turnState
        val useWebHost = route == AndroidContextRoute.Web
        val result = contextPreparer.prepare(
            ChatContextPreparationRequest(
                conversation = conv,
                messages = messages,
                currentUserText = currentUserText,
                route = route,
                tools = tools,
                mode = mode,
                responseStyle = state.responseStyle,
                parameters = state.effectiveGenerationParameters,
                uploadedSkills = if (useWebHost) state.hostUploadedSkills else state.uploadedSkills,
                uploadedDocuments = if (useWebHost) state.hostUploadedDocuments else state.uploadedDocuments,
                mcpServers = if (useWebHost) state.hostMcpServers else state.mcpServers,
            )
        )
        _uiState.update {
            it.copy(
                contextUsage = (result.usage.usagePercent / 100f).coerceIn(0f, 1f),
                contextTokens = result.usage.totalTokens,
                contextMaxTokens = result.usage.maxTokens,
                contextUsageDetail = result.usage,
            )
        }
        return result.prepared
    }

    private suspend fun recordPreparedDataSourceUsage(
        prepared: AndroidPreparedContext,
        conversationId: String,
        route: AndroidContextRoute
    ) {
        contextPreparer.recordDataSourceUsage(prepared, conversationId, route)
    }

    private fun localToolPlan(convId: String, state: ChatUiState): AgentCoreAndroidToolPlan =
        agentTurnService.toolPlan(
            sessionId = convId,
            resultEgressAuthorized = state.agentMode,
            permissionMode = state.permissionMode,
        )

    private suspend fun prepareWebResumeInput(
        conv: Conversation,
        messages: List<ConversationMessage>,
        route: AndroidContextRoute,
    ): ModelInput {
        val state = _uiState.value
        val tools = webAgentProvider.contextToolSchemas(
            conv.id,
            state.hybridDeviceToolsEnabled,
            state.hybridDeviceCapabilities,
            state.permissionMode,
        )
        return prepareFinalContext(
            conv = conv,
            messages = messages,
            currentUserText = messages.lastOrNull { it.role == ChatRole.user }?.text.orEmpty(),
            route = route,
            tools = tools,
            mode = "context_limit_recovery",
        ).modelInput
    }

    private suspend fun loadLastConversation() {
        if (composerDrafts.shouldRestorePending()) return
        val owner = composerDrafts.captureRead(_uiState.value.conversationId, modelSelectionGeneration)
        val convs = try {
            conversationRepo.getAllConversations()
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            Log.w("ChatViewModel", "loadLastConversation failed", error)
            return
        }
        if (!composerDrafts.ownsRead(owner, _uiState.value.conversationId, modelSelectionGeneration) ||
            composerDrafts.shouldRestorePending()) return
        val last = convs.firstOrNull { it.agentRoomOwner == null } ?: return
        runCatching { loadConversation(last.id) }
            .onFailure {
                if (it is kotlinx.coroutines.CancellationException) throw it
                Log.w("ChatViewModel", "loadLastConversation failed to load ${last.id}", it)
            }
    }

    private fun String.toModelSource(): ModelInfo.ModelSource? {
        return when (this) {
            "local" -> ModelInfo.ModelSource.Local
            "cloud" -> ModelInfo.ModelSource.Cloud
            "web-host" -> ModelInfo.ModelSource.WebHost
            else -> null
        }
    }

    override fun onCleared() {
        composerDrafts.close()
        runRegistry.clear()
        super.onCleared()
    }
}

internal object ConversationLoadCommitGate {
    fun shouldCommit(
        requestedGeneration: Long,
        currentGeneration: Long,
    ): Boolean = requestedGeneration == currentGeneration
}

internal suspend fun executeBridgeDeviceCall(
    request: SseEvent.DeviceToolRequest,
    decision: AndroidAgentPermissionDecision,
    executor: suspend (com.nola.core.AndroidAgentHostCallRequest) -> com.nola.core.AndroidAgentHostCallResult,
): com.nola.core.AndroidAgentHostCallResult {
    if (!decision.isAllowed) {
        return com.nola.core.AndroidAgentHostCallResult(
            callId = request.id,
            output = JsonObject(mapOf("error" to JsonPrimitive("Permission denied by user or system"))),
            isError = true,
        )
    }
    val hostCall = com.nola.core.AndroidAgentHostCallRequest(
        callId = request.id,
        toolName = request.toolName,
        capability = request.capability,
        arguments = request.arguments,
        grant = decision.scopedUri?.let { uri -> buildJsonObject { put("uri", uri) } },
    )
    return try {
        executor(hostCall)
    } catch (cancelled: kotlinx.coroutines.CancellationException) {
        throw cancelled
    } catch (error: Exception) {
        com.nola.core.AndroidAgentHostCallResult(
            callId = request.id,
            output = JsonObject(
                mapOf(
                    "error" to JsonPrimitive(error.message ?: "Device tool execution failed"),
                    "capability" to JsonPrimitive(request.capability),
                    "tool_name" to JsonPrimitive(request.toolName),
                )
            ),
            isError = true,
        )
    }
}
