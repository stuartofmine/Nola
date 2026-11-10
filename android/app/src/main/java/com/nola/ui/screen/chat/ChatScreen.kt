@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package com.nola.ui.screen.chat

import com.nola.ui.components.NolaIconButton
import androidx.activity.compose.BackHandler
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.SeekableTransitionState
import androidx.compose.animation.core.rememberTransition
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.semantics.Role

import android.content.Intent
import android.content.ClipData
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.togetherWith
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.content.ReceiveContentListener
import androidx.compose.foundation.content.TransferableContent
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.QuestionAnswer
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalConfiguration
import com.nola.ui.components.nolaPageGutter
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.annotation.StringRes
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import com.nola.R
import com.nola.core.AndroidAgentPermissionAction
import com.nola.ui.components.rememberNolaHaptics
import com.nola.core.IntelligenceEvent
import com.nola.core.IntelligenceIngestRequest
import com.nola.core.IntelligenceMemoryCandidate
import com.nola.core.IntelligenceTaskOutcome
import com.nola.core.AgentCoreTaskHandle
import com.nola.core.AgentCoreTaskGuidanceQueue
import com.nola.core.AgentCoreTaskKind
import com.nola.core.AgentCoreTaskState
import com.nola.core.AgentCoreSkillRun
import com.nola.core.AgentCoreSkillRunStatus
import com.nola.core.PersonalIntelligenceCore
import com.nola.data.health.AndroidHealthDataSource
import com.nola.domain.model.ChatRole
import com.nola.domain.model.ContextUsage
import com.nola.domain.model.ConversationMessage
import com.nola.domain.model.ConversationTitlePresentationPolicy
import com.nola.domain.model.ModelInfo
import com.nola.domain.model.HostAgentToolEvent
import com.nola.domain.model.AnswerSource
import com.nola.domain.model.decodeHostToolEventsFromMeta
import com.nola.domain.projection.CONVERSATION_SURFACE_CURRENT_TOOL_CALL_META_KEY
import com.nola.domain.projection.CONVERSATION_SURFACE_LIVE_META_KEY
import com.nola.domain.projection.CompactionSourceNavigator
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.model.ProcessingState
import com.nola.domain.model.decodeAnswerSourcesFromMeta
import com.nola.domain.model.decodeAgentThinkingFromMeta
import com.nola.ui.components.HostToolCard
import com.nola.ui.components.NolaColors
import com.nola.ui.theme.nolaSurface
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.NolaRadius
import com.nola.ui.components.NolaSpacing
import com.nola.ui.components.PermissionDialog
import com.nola.ui.components.hostToolDisplayName
import com.nola.ui.components.SmoothStreamingMarkdownRenderer
import com.nola.ui.components.ThinkingBlock
import com.nola.ui.components.NolaDisclosureChevron
import com.nola.ui.components.nolaDisclosureEnter
import com.nola.ui.components.nolaDisclosureExit
import com.nola.ui.components.rememberReduceMotionEnabled
import com.nola.ui.screen.intelligence.IntelligenceEntryKind
import com.nola.ui.screen.intelligence.IntelligenceViewModel
import kotlinx.coroutines.launch
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

const val CHAT_STREAMING_MARKDOWN_TEST_TAG = "chat.streaming.markdown"
const val CHAT_MESSAGE_TEST_TAG_PREFIX = "chat.message."
const val CHAT_TRANSCRIPT_TAIL_TEST_TAG = "chat.transcript.tail"
val ChatTranscriptMessageIds = androidx.compose.ui.semantics.SemanticsPropertyKey<List<String>>("ChatTranscriptMessageIds")
val ChatStreamingMarkdownSourceKey = SemanticsPropertyKey<String>("ChatStreamingMarkdownSource")
var SemanticsPropertyReceiver.chatStreamingMarkdownSource by ChatStreamingMarkdownSourceKey
val ChatComposerGenerationActiveKey = SemanticsPropertyKey<Boolean>("ChatComposerGenerationActive")
var SemanticsPropertyReceiver.chatComposerGenerationActive by ChatComposerGenerationActiveKey

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ChatScreen(
    viewModel: ChatViewModel = hiltViewModel(),
    intelligenceViewModel: IntelligenceViewModel = hiltViewModel(),
    attachmentImporter: AndroidAttachmentImporter = rememberAndroidAttachmentImporter(),
    onOpenModels: () -> Unit = {},
    onOpenPlans: () -> Unit = {},
    onDetailVisibilityChanged: (Boolean) -> Unit = {},
    requestedConversationId: String? = null,
    requestedNewConversation: Boolean = false,
    onRequestedConversationConsumed: () -> Unit = {},
) {
    val state by viewModel.uiState.collectAsStateWithLifecycle()
    val editDraft by viewModel.messageEditDraft.collectAsStateWithLifecycle()
    editDraft?.let { MessageEditDialog(it, viewModel::updateMessageEditText,
        viewModel::submitMessageEdit, viewModel::cancelMessageEdit) }

    val expertReplyProjections by viewModel.conversationExpertReplies.collectAsStateWithLifecycle()
    val expertReplies = remember(state.conversationId, expertReplyProjections) {
        expertReplyProjections.filterKeys { it.conversationId == state.conversationId }.mapKeys { it.key.messageId }
    }
    var uiSurfaces by remember(state.conversationId, state.executionHostBinding) {
        mutableStateOf<List<com.nola.data.local.UiSnapshot>>(emptyList())
    }
    var uiSurfaceFailure by remember(state.conversationId, state.executionHostBinding) { mutableStateOf<String?>(null) }
    var uiReadSequence by remember(state.conversationId, state.executionHostBinding) { mutableIntStateOf(0) }
    val hasForeignExpertMessage = expertReplies.values.flatten().any { reply ->
        reply.message?.conversationId?.let { it != reply.participant.contextConversationId } == true
    }
    val surfaceMessages = state.messages + expertReplies.values.flatten().mapNotNull { reply ->
        reply.message?.takeIf { it.conversationId == reply.participant.contextConversationId }
    }
    val uiBoundary = surfaceMessages.flatMap { message -> message.canonicalParts
        .filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.UiSurface>()
        .map { Triple(it.partId, it.revision, it.surface) } }
    LaunchedEffect(state.conversationId, state.executionHostBinding, uiBoundary) {
        val conversationId = state.conversationId ?: return@LaunchedEffect
        // The ViewModel resolves the persisted conversation Host for remote surfaces.
        try {
            val sequence = ++uiReadSequence
            val references = surfaceMessages.flatMap { it.canonicalParts }
                .filterIsInstance<com.nola.domain.projection.ConversationProjectionV1.Part.UiSurface>()
                .map { it.surface.identity }.distinct()
            val loaded = references.mapNotNull { identity ->
                viewModel.loadUiSurfaceChanges(identity, uiSurfaces.find { it.identity == identity })
            }
            if (sequence == uiReadSequence) {
                uiSurfaces = loaded
                uiSurfaceFailure = null
            }
        } catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (error: Exception) { uiSurfaceFailure = error.message }
    }
    val navigationFocusManager = androidx.compose.ui.platform.LocalFocusManager.current
    val navigationKeyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    fun dismissNavigationKeyboard() {
        navigationFocusManager.clearFocus()
        navigationKeyboard?.hide()
    }
    val composerDrafts = viewModel.composerDrafts
    val composerDraft = composerDrafts.forConversation(state.conversationId)
    // A positive revision is presentation readiness from the canonical load,
    // not an authorization grant. Existing active Runs keep editable drafts.
    val presentationReady = state.conversationId == null || state.configurationRevision > 0
    val draftWritable = presentationReady && !composerDraft.loading.value && !composerDrafts.initializing.value &&
        composerDraft.persistenceError.value == null && state.pendingExpertHostAdmission == null && state.expertHostConfirmationStatus == null
    fun hasCurrentDraftScope(): Boolean {
        val latest = viewModel.uiState.value
        return (latest.conversationId == null || latest.configurationRevision > 0) &&
            latest.executionHostBinding == state.executionHostBinding &&
            composerDrafts.forConversation(latest.conversationId) === composerDraft
    }
    fun canMutateDraft(): Boolean {
        val latest = viewModel.uiState.value
        return hasCurrentDraftScope() && !composerDraft.loading.value && !composerDrafts.initializing.value && composerDraft.persistenceError.value == null &&
            latest.pendingExpertHostAdmission == null && latest.expertHostConfirmationStatus == null
    }
    var inputText by composerDraft.text
    var showModelMenu by remember { mutableStateOf(false) }
    var showIMSearch by rememberSaveable { mutableStateOf(false) }
    var imSearch by rememberSaveable { mutableStateOf("") }
    var showActions by remember { mutableStateOf(false) }
    var showExecutionBackendMenu by remember { mutableStateOf(false) }
    var inspector by rememberSaveable(state.conversationId, state.executionHostBinding) { mutableStateOf<ConversationInspector?>(null) }
    val showMoreSettings = inspector == ConversationInspector.Settings
    var showContextSheet by rememberSaveable { mutableStateOf(ContextDisplayMode.Context) }
    var showContextSheetOpen by rememberSaveable(state.conversationId, state.executionHostBinding) { mutableStateOf(false) }
    var showConversations by rememberSaveable { mutableStateOf(true) }
    LaunchedEffect(requestedConversationId, requestedNewConversation) {
        if (requestedNewConversation) {
            dismissNavigationKeyboard()
            viewModel.newConversation()
            showConversations = false
            onRequestedConversationConsumed()
        } else requestedConversationId?.let { owner ->
            dismissNavigationKeyboard()
            viewModel.loadConversation(owner)
            showConversations = false
            onRequestedConversationConsumed()
        }
    }
    LaunchedEffect(showConversations) { onDetailVisibilityChanged(!showConversations) }
    var conversationSearch by rememberSaveable { mutableStateOf("") }
    val conversationListState = androidx.compose.foundation.lazy.rememberLazyListState()
    val conversationListError by viewModel.conversationListError.collectAsStateWithLifecycle()
    val conversationCatalog by viewModel.conversationCatalog.collectAsStateWithLifecycle()
    val conversationPreviews by viewModel.conversationPreviews.collectAsStateWithLifecycle()
    val conversationExperts by viewModel.conversationExperts.collectAsStateWithLifecycle()
    val conversationActivity by viewModel.conversationActivity.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val preparingExpertPicker by viewModel.preparingExpertPicker.collectAsStateWithLifecycle()
    var expertSourceLocal by remember(state.conversationId) { mutableStateOf(false) }
    var showExpertPicker by rememberSaveable { mutableStateOf(false) }
    var showExpertManagement by rememberSaveable { mutableStateOf(false) }
    var colorEditingExpertId by rememberSaveable { mutableStateOf<String?>(null) }
    val availableExperts by viewModel.conversationAgentDirectory.collectAsStateWithLifecycle()
    val selectedExpertIds by viewModel.selectedConversationAgentIds.collectAsStateWithLifecycle()
    val resumableExpertMessages by viewModel.resumableExpertMessages.collectAsState()
    val resumingExpertMessage by viewModel.resumingExpertMessage.collectAsState()
    var mentionFocused by remember(state.conversationId) { mutableStateOf(false) }
    var mentionDismissed by remember(state.conversationId) { mutableStateOf<String?>(null) }
    var mentionIndex by remember(state.conversationId) { mutableIntStateOf(0) }
    val mentionQuery = roomMentionQuery(inputText)
    val mentionCandidates = availableExperts?.agents.orEmpty().filter {
        expertSourceLocal &&
        it.availability == "active" && it.id !in selectedExpertIds &&
            it.name.contains(mentionQuery.orEmpty(), ignoreCase = true)
    }
    val mentionsVisible = mentionFocused && mentionQuery != null && mentionDismissed != inputText
    val context = LocalContext.current
    var commandError by remember(state.conversationId) { mutableStateOf<String?>(null) }
    var commandDismissed by remember(state.conversationId) { mutableStateOf<String?>(null) }
    var commandIndex by remember(state.conversationId) { mutableIntStateOf(0) }
    val commandQuery = ComposerCommand.query(inputText)
    val commandCandidates = ComposerCommand.entries.filter { commandQuery != null &&
        (it.name.startsWith(commandQuery) || context.getString(it.label).contains(commandQuery, ignoreCase = true)) }
    val commandsVisible = mentionFocused && commandQuery != null && commandDismissed != inputText
    fun openExpertPicker() {
        scope.launch {
            if (viewModel.prepareExpertPicker()) { expertSourceLocal = true; showExpertManagement = false; showExpertPicker = true }
        }
    }
    LaunchedEffect(state.conversationId, state.configurationRevision, state.processingState, state.executionHostBinding) {
        expertSourceLocal = false
        val id = state.conversationId ?: return@LaunchedEffect
        expertSourceLocal = try { viewModel.isLocalExpertSource(id) }
        catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (_: Exception) { false }
        if (!expertSourceLocal && !showExpertManagement) showExpertPicker = false
    }
    fun completeCommand(command: ComposerCommand) { if (!canMutateDraft()) return; inputText = "/${command.name} "; commandError = null; commandIndex = 0 }
    fun executeCommand(): Boolean {
        if (!canMutateDraft()) return false
        val parsed = ComposerCommand.parse(inputText) ?: return false
        val command = parsed.first
        if (command == null) { commandError = context.getString(R.string.composer_command_invalid); return true }
        if (command == ComposerCommand.stop && !state.processingState.isGenerating) {
            commandError = context.getString(R.string.composer_command_no_run); return true
        }
        if (command == ComposerCommand.permissions && !viewModel.beginParameterEditing()) return true
        inputText = ""; commandError = null
        when (command) {
            ComposerCommand.model -> showModelMenu = true
            ComposerCommand.experts -> openExpertPicker()
            ComposerCommand.resources -> inspector = ConversationInspector.Resources
            ComposerCommand.permissions -> inspector = ConversationInspector.Settings
            ComposerCommand.plan -> onOpenPlans()
            ComposerCommand.search -> { conversationSearch = parsed.second; showConversations = true; dismissNavigationKeyboard() }
            ComposerCommand.stop -> viewModel.cancelGeneration(viewModel.composerDrafts.ownerKey(state.conversationId), state.activeResponseId)
            ComposerCommand.help -> inputText = "/"
        }
        return true
    }
    val mentionListState = rememberLazyListState()
    LaunchedEffect(mentionQuery) { mentionIndex = 0 }
    LaunchedEffect(mentionIndex, mentionsVisible) {
        if (mentionsVisible && mentionCandidates.isNotEmpty()) mentionListState.animateScrollToItem(mentionIndex.coerceIn(mentionCandidates.indices))
    }
    LaunchedEffect(mentionQuery != null, state.conversationId) {
        if (mentionQuery != null) {
            if (state.conversationId == null) scope.launch { viewModel.prepareExpertPicker() }
            else if (expertSourceLocal) viewModel.refreshConversationExperts(restoreSelection = false)
        }
    }
    fun selectMention(id: String) {
        if (!canMutateDraft()) return
        if (mentionCandidates.none { it.id == id }) return
        viewModel.toggleConversationExpert(id)
        inputText = removeRoomMentionQuery(inputText)
        mentionIndex = 0
    }

    var attachments by composerDraft.attachments
    var attachmentMessage by composerDraft.notice
    var isImportingAttachments by composerDraft.importing
    var launchedAgentPermission by remember { mutableStateOf<AndroidAgentPermissionAction?>(null) }
    var learningMessage by remember { mutableStateOf<ConversationMessage?>(null) }
    var actionMessage by remember { mutableStateOf<ConversationMessage?>(null) }
    val transcriptStates = rememberSaveableStateHolder()
    val isWebHostModel = state.currentModel?.source == ModelInfo.ModelSource.WebHost
    fun importAttachmentUris(uris: List<Uri>, target: ConversationComposerDraft = composerDraft) {
        if (uris.isEmpty()) return
        var attachments by target.attachments
        var attachmentMessage by target.notice
        var isImportingAttachments by target.importing
        isImportingAttachments = true
        attachmentMessage = context.getString(R.string.chat_attachment_importing)
        scope.launch {
            try {
                val result = attachmentImporter.import(AndroidAttachmentImportRequest(uris))
                val merged = attachments + result.attachments
                attachments = merged.takeLast(MAX_CHAT_ATTACHMENTS)
                attachmentMessage = when {
                    result.attachments.isEmpty() -> context.getString(
                        R.string.chat_attachment_import_failed,
                    )
                    result.failedCount > 0 -> context.getString(
                        R.string.chat_attachment_partial_failure,
                        result.failedCount,
                    )
                    merged.size > MAX_CHAT_ATTACHMENTS -> context.getString(
                        R.string.chat_attachment_limit,
                    )
                    else -> null
                }
            } finally {
                isImportingAttachments = false
            }
        }
    }
    val attachmentPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenMultipleDocuments()
    ) { uris ->
        val target = composerDrafts.consumePicker()
        if (target != null) importAttachmentUris(uris, target)
    }
    val agentRuntimePermissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { grants ->
        val action = launchedAgentPermission as? AndroidAgentPermissionAction.RuntimePermissions
        if (action != null) viewModel.completeLocalRuntimePermission(action.requestId, grants)
        launchedAgentPermission = null
    }
    val agentDocumentLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        val action = launchedAgentPermission as? AndroidAgentPermissionAction.OpenDocument
        if (action != null) {
            viewModel.completeLocalScopedResource(action.requestId, uri, action.write)
        }
        launchedAgentPermission = null
    }
    val agentPhotoLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.PickVisualMedia()
    ) { uri ->
        val action = launchedAgentPermission as? AndroidAgentPermissionAction.PickVisualMedia
        if (action != null) {
            viewModel.completeLocalScopedResource(
                requestId = action.requestId,
                uri = uri,
                write = false,
                persistable = false
            )
        }
        launchedAgentPermission = null
    }
    val agentHealthLauncher = rememberLauncherForActivityResult(
        AndroidHealthDataSource.permissionContract()
    ) { granted ->
        val action = launchedAgentPermission as? AndroidAgentPermissionAction.HealthConnect
        if (action != null) {
            viewModel.completeLocalHealthPermission(
                action.requestId,
                granted.containsAll(AndroidHealthDataSource.readPermissions)
            )
        }
        launchedAgentPermission = null
    }

    val visiblePermissionAction = state.agentPermissionAction ?: expertReplies.values.flatten().firstNotNullOfOrNull { it.permissionAction }
    LaunchedEffect(visiblePermissionAction) {
        when (val action = visiblePermissionAction) {
            is AndroidAgentPermissionAction.RuntimePermissions -> {
                launchedAgentPermission = action
                agentRuntimePermissionLauncher.launch(action.permissions.toTypedArray())
            }
            is AndroidAgentPermissionAction.OpenDocument -> {
                launchedAgentPermission = action
                agentDocumentLauncher.launch(action.mimeTypes.toTypedArray())
            }
            is AndroidAgentPermissionAction.PickVisualMedia -> {
                launchedAgentPermission = action
                agentPhotoLauncher.launch(
                    PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)
                )
            }
            is AndroidAgentPermissionAction.HealthConnect -> {
                launchedAgentPermission = action
                agentHealthLauncher.launch(AndroidHealthDataSource.readPermissions)
            }
            else -> Unit
        }
    }

    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        viewModel.loadAvailableModels()
        viewModel.onConversationForeground()
    }

    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { viewModel.onConversationBackground() }

    BackHandler(enabled = inspector != null || showExpertPicker || showContextSheetOpen) {
        when {
            showContextSheetOpen -> showContextSheetOpen = false
            inspector != null -> inspector = null
            showExpertPicker -> if (showExpertManagement && expertSourceLocal) showExpertManagement = false else { showExpertManagement = false; showExpertPicker = false }
            else -> { dismissNavigationKeyboard(); viewModel.loadConversations(); showConversations = true }
        }
    }

    availableExperts?.agents?.firstOrNull { it.id == colorEditingExpertId }?.let { agent ->
        ExpertBubbleColorEditor(agentId = agent.id, name = agent.name, onDismiss = { colorEditingExpertId = null })
    }

    if (showExpertPicker) {
        ModalBottomSheet(onDismissRequest = { showExpertPicker = false; showExpertManagement = false },
            sheetState = androidx.compose.material3.rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
            if (showExpertManagement) {
                ExpertManagementPanel(viewModel, onBack = {
                    showExpertManagement = false
                    if (expertSourceLocal) viewModel.refreshConversationExperts() else showExpertPicker = false
                })
            } else Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp)) {
                TextButton(onClick = { showExpertManagement = true }, modifier = Modifier.testTag("chat.experts.manage").heightIn(min = 48.dp)) { Text(stringResource(R.string.expert_manage)) }
                Text(stringResource(R.string.chat_experts), style = MaterialTheme.typography.titleLarge)
                Text(stringResource(R.string.chat_experts_hint), style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 4.dp, bottom = 12.dp))
                val agents = availableExperts?.agents.orEmpty().filter { expertSourceLocal && it.availability == "active" }
                if (agents.isEmpty()) {
                    Text(stringResource(R.string.chat_experts_empty), modifier = Modifier.padding(vertical = 20.dp))
                } else {
                    LazyColumn(Modifier.heightIn(max = 420.dp).testTag("chat.experts.list")) {
                        items(agents, key = { it.id }) { agent ->
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                            FilterChip(
                                selected = agent.id in selectedExpertIds,
                                onClick = { viewModel.toggleConversationExpert(agent.id) },
                                label = { Column {
                                    Text(agent.name, style = MaterialTheme.typography.bodyLarge)
                                    if (agent.description.isNotBlank()) Text(agent.description, style = MaterialTheme.typography.bodySmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                } },
                                leadingIcon = { Text(agent.name.take(1).uppercase(), fontWeight = FontWeight.Bold) },
                                modifier = Modifier.weight(1f).padding(vertical = 4.dp).heightIn(min = 56.dp)
                            )
                                TextButton(onClick = { colorEditingExpertId = agent.id }, modifier = Modifier.heightIn(min = 48.dp)) {
                                    Text(stringResource(R.string.expert_color_action))
                                }
                            }
                        }
                    }
                }
                TextButton(onClick = { showExpertPicker = false }, modifier = Modifier.align(Alignment.End).heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.common_done))
                }
            }
        }
    }

    if (inspector != null || showContextSheetOpen) {
        ModalBottomSheet(onDismissRequest = { inspector = null; showContextSheetOpen = false }, sheetState = androidx.compose.material3.rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Box(Modifier.fillMaxWidth().fillMaxHeight(0.92f)) {
        if (inspector == ConversationInspector.Progress) {
            ConversationProgressPanel(state, viewModel::loadConversationProgress, viewModel::pendingGoalControl, viewModel::submitGoalControl, onClose = { inspector = null })
        }
        if (inspector == ConversationInspector.Resources) {
            ConversationResourcesPanel(state, onClose = { inspector = null })
        }
    if (showMoreSettings && !showContextSheetOpen) {
        ChatMoreSettingsScreen(
            selectedModel = state.currentModel,
            contextUsage = state.contextUsageDetail,
            agentModeEnabled = state.agentMode,
            permissionMode = state.permissionMode,
            hybridDeviceToolsEnabled = state.hybridDeviceToolsEnabled,
            hybridDeviceCapabilities = state.hybridDeviceCapabilities,
            pairedHostIdentity = state.pairedHostIdentity,
            autoCompactTriggerPercent = state.autoCompactTriggerPercent,
            responseStyleOverride = state.responseStyleOverride,
            sessionMemory = state.sessionMemory,
            longTermMemory = state.longTermMemory,
            userPreferences = state.userPreferences,
            uploadedSkills = state.uploadedSkills,
            hostUploadedSkills = state.hostUploadedSkills,
            hostUploadedDocuments = state.hostUploadedDocuments,
            hostMcpServers = state.hostMcpServers,
            hostComputerUseStatus = state.hostComputerUseStatus,
            hostNativeBrowserStatus = state.hostNativeBrowserStatus,
            draftParameters = state.currentParameters,
            reasoningEfforts = remember(state.currentModel) { state.currentModel?.let(viewModel::reasoningEfforts).orEmpty() },
            conversationId = state.conversationId,
            generationOverride = state.generationOverride,
            onAgentModeChange = { viewModel.setAgentMode(it) },
            onPermissionModeChange = { viewModel.savePermissionMode(it) },
            onHybridDeviceToolsChange = { viewModel.setHybridDeviceToolsEnabled(it) },
            onHybridDeviceCapabilityChange = { capability, enabled ->
                viewModel.setHybridDeviceCapability(capability, enabled)
            },
            onForgetHostPairing = { viewModel.forgetHostPairing() },
            onAutoCompactPercentChange = { viewModel.setAutoCompactPercent(it) },
            onResponseStyleChange = { viewModel.saveResponseStyle(it) },
            onSessionMemoryChange = { viewModel.updateSessionMemory(it) },
            onLongTermMemoryChange = { viewModel.updateLongTermMemory(it) },
            onUserPreferencesChange = { viewModel.updateUserPreferences(it) },
            onSkillToggle = { skill, enabled -> viewModel.toggleSkill(skill, enabled) },
            onSaveParameters = { viewModel.saveModelParameters(it) },
            onSaveGenerationOverride = { viewModel.saveGenerationOverride(it) },
            onReloadParameters = viewModel::beginParameterEditing,
            onSaveMemoryProfile = { viewModel.saveMemoryProfile() },
            onOpenContextSettings = {
                viewModel.refreshDataSourceCatalog()
                showContextSheet = ContextDisplayMode.Context
                showContextSheetOpen = true
            },
            onOpenSkillLibrary = {
                showContextSheet = ContextDisplayMode.Skills
                showContextSheetOpen = true
            },
            onBack = { inspector = null }
        )
    }

    if (showContextSheetOpen) {
        LaunchedEffect(showContextSheetOpen, showContextSheet) {
            viewModel.refreshDataSourceCatalog()
        }
        val conversation = state.conversations.find { it.id == state.conversationId }
        val convTitle = ConversationTitlePresentationPolicy.displayTitle(
            conversation?.title,
            conversation?.id ?: state.conversationId,
            stringResource(R.string.chat_new_chat),
        )
        ChatContextScreen(
            displayMode = showContextSheet,
            conversationId = state.conversationId.orEmpty(),
            conversationTitle = convTitle,
            contextUsage = state.contextUsageDetail,
            sessionMemory = state.sessionMemory,
            knowledgePoints = state.knowledgePoints,
            conversationWiki = state.conversationWiki,
            longTermMemory = state.longTermMemory,
            userPreferences = state.userPreferences,
            mcpServers = state.mcpServers,
            uploadedSkills = state.uploadedSkills,
            hostUploadedSkills = state.hostUploadedSkills,
            hostUploadedDocuments = state.hostUploadedDocuments,
            hostMcpServers = state.hostMcpServers,
            uploadedDocuments = state.uploadedDocuments,
            dataSources = state.dataSources,
            selectedDataSourceIds = state.selectedDataSourceIds,
            onDataSourceEnabledChange = viewModel::setConversationDataSourceEnabled,
            onDataSourceRestoreDefault = viewModel::restoreConversationDataSourceDefault,
            onSessionMemoryChange = { viewModel.updateSessionMemory(it) },
            onConversationKnowledgeChange = { knowledgePoints, wiki ->
                viewModel.updateConversationKnowledge(knowledgePoints, wiki)
            },
            onLongTermMemoryChange = { viewModel.updateLongTermMemory(it) },
            onUserPreferencesChange = { viewModel.updateUserPreferences(it) },
            onSkillToggle = { skill, enabled -> viewModel.toggleSkill(skill, enabled) },
            onSaveSkill = { original, name, content -> viewModel.saveSkill(original, name, content) },
            onSkillDelete = { viewModel.deleteSkill(it) },
            onDocumentAdd = { viewModel.addDocument(it) },
            onDocumentDelete = { viewModel.deleteDocument(it) },
            onSaveMemoryProfile = { viewModel.saveMemoryProfile() },
            onCompact = { viewModel.compactNow() },
            onReload = { viewModel.reloadContextState() },
            onSearchMemory = { query -> viewModel.searchMemoryForContext(query) },
            onBack = { showContextSheetOpen = false }
        )
    }
        }
        }
    }


    if (showIMSearch) {
        ModalBottomSheet(onDismissRequest = { showIMSearch = false },
            sheetState = androidx.compose.material3.rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
            Column(Modifier.fillMaxWidth().fillMaxHeight(.92f).imePadding().padding(16.dp)) {
                androidx.compose.material3.OutlinedTextField(value = imSearch, onValueChange = { imSearch = it },
                    label = { Text(stringResource(R.string.chat_search_loaded_title)) }, modifier = Modifier.fillMaxWidth())
                Text(stringResource(R.string.chat_search_loaded_scope), style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(vertical = 12.dp))
                LazyColumn(Modifier.weight(1f, fill = false).heightIn(max = 480.dp)) {
                    items(surfaceMessages.filter { imSearch.isNotEmpty() && it.text.contains(imSearch, ignoreCase = true) }, key = { it.id }) {
                        Text(it.text, modifier = Modifier.padding(vertical = 12.dp), style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        }
    }

    val navigationReduceMotion = rememberReduceMotionEnabled()
    val navigationDirection = if (androidx.compose.ui.platform.LocalLayoutDirection.current == androidx.compose.ui.unit.LayoutDirection.Rtl) -1 else 1
    val navigationState = remember { SeekableTransitionState(showConversations) }
    val navigationTransition = rememberTransition(navigationState, label = "conversationNavigation")
    LaunchedEffect(showConversations) {
        if (navigationReduceMotion) navigationState.snapTo(showConversations)
        else navigationState.animateTo(showConversations)
    }
    val keyboardVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    PredictiveBackHandler(enabled = !showConversations && !keyboardVisible && inspector == null &&
        !showExpertPicker && !showContextSheetOpen && !showActions && !showIMSearch) { events ->
        try {
            events.collect { event ->
                if (!navigationReduceMotion) navigationState.seekTo(event.progress, true)
            }
            dismissNavigationKeyboard()
            viewModel.loadConversations()
            showConversations = true
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            // Cancelling the system gesture keeps the same chat and draft.
            kotlinx.coroutines.withContext(kotlinx.coroutines.NonCancellable) {
                if (navigationReduceMotion) navigationState.snapTo(false) else navigationState.animateTo(false)
            }
            throw cancelled
        }
    }
    navigationTransition.AnimatedContent(transitionSpec = {
        val movement = tween<androidx.compose.ui.unit.IntOffset>(260, easing = FastOutSlowInEasing)
        val transition = if (navigationReduceMotion) fadeIn(tween(0)) togetherWith fadeOut(tween(0))
        else if (targetState) (slideInHorizontally(movement, initialOffsetX = { -it * navigationDirection / 8 }) + fadeIn(tween(160))) togetherWith
            slideOutHorizontally(movement, targetOffsetX = { it * navigationDirection })
        else slideInHorizontally(movement, initialOffsetX = { it * navigationDirection }) togetherWith
            (slideOutHorizontally(movement, targetOffsetX = { -it * navigationDirection / 8 }) + fadeOut(tween(160)))
        transition.using(SizeTransform(clip = false))
    }) { showingList ->
    if (showingList) {
        ConversationListScreen(
            conversations = conversationCatalog,
            currentConversationId = state.conversationId,
            onConversationSelected = {
                dismissNavigationKeyboard()
                viewModel.loadConversation(it)
                showConversations = false
            },
            onNewConversation = {
                dismissNavigationKeyboard()
                viewModel.newConversation()
                showConversations = false
            },
            onDeleteConversation = { viewModel.deleteConversation(it) },
            search = conversationSearch,
            onSearchChange = { conversationSearch = it },
            listState = conversationListState,
            previews = conversationPreviews,
            drafts = conversationCatalog.associate { it.id to composerDrafts.forConversation(it.id).text.value },
            experts = conversationExperts,
            activity = conversationActivity,
            onLoadPreview = viewModel::loadConversationPreview,
            error = conversationListError,
            onRetry = viewModel::loadConversations
        )
    } else {

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(NolaColors.chatBackground)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.background)
                .padding(horizontal = NolaSpacing.sm, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            NolaIconButton(label = stringResource(R.string.common_back), onClick = { dismissNavigationKeyboard(); viewModel.loadConversations(); showConversations = true }, modifier = Modifier.testTag("chat.conversation.back")) {
                Icon(painterResource(R.drawable.nola_back), contentDescription = stringResource(R.string.common_back))
            }
            Box(
                modifier = Modifier
                    .weight(1f)
                    .heightIn(min = 48.dp)
                    .testTag("chat.modelPicker")
                    .clickable { showActions = true },
                contentAlignment = Alignment.Center
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.widthIn(max = 220.dp)) {
                    Text(
                        state.conversations.find { it.id == state.conversationId }?.title
                            ?: stringResource(R.string.chat_new_chat),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        textAlign = TextAlign.Center
                    )
                    Text(when {
                        !presentationReady && state.processingState == ProcessingState.BuildingContext -> stringResource(R.string.chat_loading_conversation)
                        state.permissionRequest != null -> stringResource(R.string.conversations_waiting)
                        state.processingState == ProcessingState.AwaitingRecovery -> stringResource(R.string.chat_awaiting_recovery)
                        state.processingState.blocksInput -> stringResource(R.string.conversations_running)
                        else -> "Nola"
                    }, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis)

                }

            }

            Row(horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                Box {
                    NolaIconButton(label = stringResource(R.string.chat_search_loaded_title), onClick = { showIMSearch = true }, modifier = Modifier.testTag("chat.topbar.search")) {
                        Icon(painterResource(R.drawable.nola_search), contentDescription = stringResource(R.string.chat_search_loaded_title))
                    }
                    if (showActions) ModalBottomSheet(onDismissRequest = { showActions = false }) {
                        DropdownMenuItem(text = { Text(stringResource(R.string.chat_experts)) },
                            modifier = Modifier.testTag("chat.experts.open"),
                            enabled = draftWritable && !preparingExpertPicker,
                            onClick = { showActions = false; openExpertPicker() })
                        DropdownMenuItem(text = { Text(stringResource(R.string.expert_manage)) },
                            modifier = Modifier.testTag("chat.experts.manage.local"),
                            onClick = { showActions = false; showExpertManagement = true; showExpertPicker = true; viewModel.refreshConversationExperts() })
                        DropdownMenuItem(text = { Text(stringResource(R.string.chat_open_context_settings)) },
                            modifier = Modifier.testTag("chat.context.open"),
                            enabled = presentationReady,
                            onClick = {
                                showActions = false
                                inspector = null
                                showContextSheet = ContextDisplayMode.Context
                                showContextSheetOpen = true
                            })
                        ConversationInspector.entries.forEach { target ->
                            DropdownMenuItem(text = { Text(stringResource(target.label)) }, onClick = {
                                showActions = false
                                if (target != ConversationInspector.Settings || inspector == target || viewModel.beginParameterEditing()) {
                                    if (target == ConversationInspector.Resources) showContextSheet = ContextDisplayMode.Context
                                    showContextSheetOpen = false
                                    inspector = toggleInspector(inspector, target)
                                }
                            })
                        }
                    }
                }

                if (isWebHostModel && state.executionBackends.isNotEmpty()) {
                    ExecutionBackendSelector(
                        backends = state.executionBackends,
                        selectedBackendId = state.currentExecutionBackend,
                        expanded = showExecutionBackendMenu,
                        onExpandedChange = { showExecutionBackendMenu = it },
                        onSelect = { backendId ->
                            viewModel.selectExecutionBackend(backendId)
                            showExecutionBackendMenu = false
                        },
                    )
                }

            }
        }
        HorizontalDivider(
            thickness = 0.5.dp,
            color = MaterialTheme.colorScheme.outline.copy(alpha = 0.20f)
        )

        if (!presentationReady && state.processingState == ProcessingState.BuildingContext) {
            Box(Modifier.weight(1f).fillMaxWidth().testTag("chat.conversation.loading"), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    CircularProgressIndicator()
                    Text(stringResource(R.string.chat_loading_conversation), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        } else Box(Modifier.weight(1f).fillMaxWidth()) {
            transcriptStates.SaveableStateProvider(state.conversationId ?: "new-conversation") {
            ChatTranscript(
            state = state,
            onResendEditedMessage = viewModel::beginMessageEdit,
            expertReplies = expertReplies,
            resumableExpertMessages = resumableExpertMessages,
            resumingExpertMessage = resumingExpertMessage,
            onResumeExpertMessage = viewModel::resumeOriginalExpertMessage,
            uiSurfaces = uiSurfaces,
            uiSurfaceFailure = if (hasForeignExpertMessage) stringResource(R.string.canonical_message_owner_error) else uiSurfaceFailure,
            onCancelQueuedUiAction = viewModel::cancelRejectedQueuedUiAction,
            onUiSubmit = viewModel::submitUiInteraction,
            onUiPreview = viewModel::previewUiSurface,
            onUiReadHistory = viewModel::loadUiSurfaceRevision,
            onUiRestore = viewModel::restoreUiInteraction,
            onUiAccepted = { receipt -> ++uiReadSequence; uiSurfaces = uiSurfaces.map {
                if (it.identity == receipt.snapshot.identity && receipt.snapshot.revision >= it.revision) receipt.snapshot else it
            } },
            modifier = Modifier.fillMaxSize(),
            onSuggestion = { if (canMutateDraft()) inputText = it },
            onOpenModels = onOpenModels,
            onLongPress = { actionMessage = it },
            onResolveHostPermission = viewModel::resolvePermission,
            onResolveLocalPermission = viewModel::resolveLocalAgentPermission,
            onHostPermissionInstruction = viewModel::provideHostPermissionInstruction,
            onLocalPermissionInstruction = viewModel::provideLocalPermissionInstruction,
            onEditAndRetry = {
                if (canMutateDraft() && viewModel.dismissError(state)) {
                    state.messages.lastOrNull { it.role == ChatRole.user }?.text?.let { inputText = it }
                }
            },
            onSwitchModel = {
                if (viewModel.dismissError(state)) onOpenModels()
            },
            onDismissError = { viewModel.dismissError(state) },
            onInspectUnknownEffects = viewModel::inspectUnknownEffects,
            onUnknownEffectDecision = viewModel::resolveUnknownEffects,
            onCompactionSourceRequested = viewModel::locateCompactionSource,
            onCompactionSourceConsumed = viewModel::consumeCompactionSourceTarget,
            onCancelTask = { taskId -> viewModel.cancelAgentTask(viewModel.composerDrafts.ownerKey(state.conversationId), taskId) },
            loadTaskDetail = viewModel::loadTaskChildDetail,
            taskRecovery = viewModel::taskRecovery,
            onSendTaskGuidance = { taskId, message -> viewModel.sendAgentTaskGuidance(viewModel.composerDrafts.ownerKey(state.conversationId), taskId, message) }
        )
            }
        }

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.background)
                .imePadding()
        ) {
            HorizontalDivider(
                thickness = 0.5.dp,
                color = MaterialTheme.colorScheme.outline.copy(alpha = 0.18f)
            )
            commandError?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(horizontal = 16.dp)) }
            if (commandsVisible) {
                LazyColumn(Modifier.fillMaxWidth().heightIn(max = 224.dp).padding(horizontal = 16.dp)
                    .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(14.dp))) {
                    items(commandCandidates, key = { it.name }) { command ->
                        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp).testTag("chat.command.${command.name}")
                            .background(if (commandCandidates.getOrNull(commandIndex) == command) MaterialTheme.colorScheme.primaryContainer else androidx.compose.ui.graphics.Color.Transparent, RoundedCornerShape(10.dp))
                            .clickable { completeCommand(command) }.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Text("/${command.name}"); Text(stringResource(command.label), color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                    if (commandCandidates.isEmpty()) item { Text(stringResource(R.string.composer_command_invalid), Modifier.padding(12.dp)) }
                }
            }
            if (mentionsVisible) {
                LazyColumn(Modifier.fillMaxWidth().heightIn(max = 224.dp).padding(horizontal = 16.dp)
                    .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(14.dp)),
                    state = mentionListState, contentPadding = PaddingValues(6.dp)) {
                    if (mentionCandidates.isEmpty()) item { Text(stringResource(R.string.chat_no_matching_experts), Modifier.padding(12.dp)) }
                    items(mentionCandidates, key = { it.id }) { expert ->
                        Row(Modifier.fillMaxWidth().heightIn(min = 52.dp)
                            .background(if (mentionCandidates.getOrNull(mentionIndex)?.id == expert.id)
                                MaterialTheme.colorScheme.primaryContainer else androidx.compose.ui.graphics.Color.Transparent, RoundedCornerShape(10.dp))
                            .clickable { selectMention(expert.id) }.padding(horizontal = 12.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Text(expert.name.take(1), Modifier.background(MaterialTheme.colorScheme.surfaceContainer, CircleShape).padding(8.dp))
                            Column {
                                Text(expert.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                if (expert.description.isNotBlank()) Text(expert.description, style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            }
            if (selectedExpertIds.isNotEmpty()) Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
                availableExperts?.agents.orEmpty().filter { it.id in selectedExpertIds }.forEach { expert ->
                    TextButton(onClick = { viewModel.toggleConversationExpert(expert.id) }) { Text("${expert.name} ×") }
                }
            }
            state.expertHostConfirmationStatus?.let { status ->
                Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("chat.expert.host.pending")) {
                    Text(status, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(onClick = viewModel::checkOriginalExpertHostRequest,
                        enabled = !state.checkingExpertHostAdmission,
                        modifier = Modifier.heightIn(min = 48.dp).testTag("chat.expert.host.check-original")) {
                        Text(stringResource(R.string.expert_host_check_original))
                    }
                    if (state.checkingExpertHostAdmission) LinearProgressIndicator(Modifier.fillMaxWidth())
                }
            }
            if (state.taskChildOwner != null) {
                TextButton(onClick = { state.taskChildOwner?.let { viewModel.loadConversation(it.parentConversationId) } }) {
                    Text(stringResource(R.string.chat_task_return_parent))
                }
            } else key(composerDraft) {
                ChatComposerBar(
                modifier = Modifier
                    .widthIn(max = 760.dp)
                    .align(Alignment.CenterHorizontally),
                modelControl = {
                    val efforts = remember(state.currentModel) { state.currentModel?.let(viewModel::reasoningEfforts).orEmpty() }
                    val savingReasoning by viewModel.savingReasoningPreference.collectAsState()
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Box(Modifier.weight(1f)) {
                            TextButton(onClick = { showModelMenu = true }, modifier = Modifier.testTag("chat.composer.model")) {
                                Text(state.currentModel?.name ?: stringResource(R.string.chat_no_model_configured), maxLines = 1,
                                    modifier = Modifier.weight(1f, fill = false), style = MaterialTheme.typography.labelMedium, overflow = TextOverflow.Ellipsis)
                                Spacer(Modifier.width(4.dp))
                                Icon(Icons.Filled.ExpandMore, null, Modifier.size(14.dp))
                            }
                        DropdownMenu(
                            expanded = showModelMenu,
                            onDismissRequest = { showModelMenu = false }
                        ) {
                            Text(stringResource(if (isWebHostModel) R.string.execution_scope_host else R.string.execution_scope_local), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.bodySmall)
                            Text(stringResource(R.string.execution_scope_child), modifier = Modifier.padding(12.dp), style = MaterialTheme.typography.bodySmall)
                            val sourceOrder = listOf(
                                ModelInfo.ModelSource.Local,
                                ModelInfo.ModelSource.WebHost,
                                ModelInfo.ModelSource.Cloud
                            )
                            val modelsBySource = state.availableModels
                                .groupBy { it.source }
                                .toSortedMap(compareBy { sourceOrder.indexOf(it).takeIf { index -> index >= 0 } ?: Int.MAX_VALUE })
                            modelsBySource.forEach { (source, models) ->
                                val (icon, label) = when (source) {
                                    ModelInfo.ModelSource.Cloud -> Icons.Filled.Cloud to stringResource(R.string.models_official_models)
                                    ModelInfo.ModelSource.Local -> Icons.Filled.PhoneAndroid to stringResource(R.string.models_local_models)
                                    ModelInfo.ModelSource.WebHost -> Icons.Filled.Computer to stringResource(R.string.models_web_host)
                                }
                                DropdownMenuItem(
                                    text = {
                                        Row(verticalAlignment = Alignment.CenterVertically) {
                                            Icon(icon, null, Modifier.size(16.dp), tint = MaterialTheme.colorScheme.primary)
                                            Spacer(Modifier.width(8.dp))
                                            Text(label, style = MaterialTheme.typography.labelSmall)
                                        }
                                    },
                                    onClick = { /* section header, no-op */ },
                                    enabled = false
                                )
                                models.forEach { model ->
                                    DropdownMenuItem(
                                        text = {
                                            Row(verticalAlignment = Alignment.CenterVertically) {
                                                Column(Modifier.weight(1f)) {
                                                    Text(model.name, style = MaterialTheme.typography.bodyMedium)
                                                    Text(
                                                        when (model.source) {
                                                            ModelInfo.ModelSource.Local -> stringResource(R.string.chat_execution_device)
                                                            ModelInfo.ModelSource.Cloud -> stringResource(R.string.chat_execution_cloud)
                                                            ModelInfo.ModelSource.WebHost -> if (
                                                                state.hybridDeviceToolsEnabled && state.hybridDeviceCapabilities.isNotEmpty()
                                                            ) stringResource(R.string.chat_execution_hybrid)
                                                            else stringResource(R.string.chat_execution_host)
                                                        },
                                                        style = MaterialTheme.typography.labelSmall,
                                                        color = MaterialTheme.colorScheme.onSurfaceVariant
                                                    )
                                                }
                                                if (
                                                    model.selectionId == state.currentModel?.selectionId &&
                                                    model.source == state.currentModel?.source
                                                ) {
                                                    Icon(
                                                        Icons.Filled.Check, null, Modifier.size(16.dp),
                                                        tint = MaterialTheme.colorScheme.primary
                                                    )
                                                }
                                            }
                                        },
                                        onClick = {
                                            viewModel.setModel(model)
                                            showModelMenu = false
                                        }
                                    )
                                }
                                if (source != modelsBySource.keys.last()) {
                                    HorizontalDivider()
                                }
                            }
                            if (state.availableModels.isNotEmpty()) {
                                HorizontalDivider()
                            }
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.chat_add_model_cta), style = MaterialTheme.typography.bodyMedium) },
                                leadingIcon = {
                                    Icon(
                                        Icons.Filled.Add,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary
                                    )
                                },
                                onClick = {
                                    showModelMenu = false
                                    onOpenModels()
                                }
                            )
                        }
                        }
                        ReasoningEffortMenu(efforts, state.currentParameters.reasoningPreference ?: "automatic",
                            enabled = !savingReasoning && !state.processingState.blocksInput,
                            onSelect = viewModel::selectReasoningPreference)
                    }
                },
                inputText = inputText,
                draftWritable = draftWritable,
                onInputChange = { if (canMutateDraft()) { inputText = it; mentionDismissed = null; commandDismissed = null; commandIndex = 0; commandError = null } },
                onFocusChange = { mentionFocused = it },
                onMentionKey = { key ->
                    if (!canMutateDraft()) false else if (commandsVisible) when (key) {
                        Key.Escape -> { commandDismissed = inputText; true }
                        Key.Enter, Key.Tab -> { commandCandidates.getOrNull(commandIndex)?.let(::completeCommand); true }
                        Key.DirectionUp, Key.DirectionDown -> {
                            if (commandCandidates.isNotEmpty()) commandIndex = (commandIndex + (if (key == Key.DirectionUp) -1 else 1) + commandCandidates.size) % commandCandidates.size
                            true
                        }
                        else -> false
                    } else if (!mentionsVisible) false else when (key) {
                        Key.Escape -> { mentionDismissed = inputText; true }
                        Key.Enter, Key.Tab -> { mentionCandidates.getOrNull(mentionIndex)?.let { selectMention(it.id) }; true }
                        Key.DirectionUp, Key.DirectionDown -> {
                            if (mentionCandidates.isNotEmpty()) mentionIndex = (mentionIndex +
                                (if (key == Key.DirectionUp) -1 else 1) + mentionCandidates.size) % mentionCandidates.size
                            true
                        }
                        else -> false
                    }
                },
                attachments = attachments,
                attachmentMessage = composerDrafts.initializationError.value ?: composerDraft.persistenceError.value
                    ?: if (state.executionTeardownPending && !state.processingState.isGenerating)
                        context.getString(R.string.chat_previous_turn_settling) else attachmentMessage,
                isImportingAttachments = isImportingAttachments || composerDraft.loading.value || composerDrafts.initializing.value,
                enabled = !preparingExpertPicker && !state.processingState.blocksInput && !state.executionTeardownPending && !state.permissionSaving && !state.permissionSaveFailed && !composerDrafts.initializing.value,
                isGenerating = presentationReady && state.processingState.isGenerating && !inputText.startsWith("/"),
                onAttach = {
                    if (canMutateDraft() && !isImportingAttachments) {
                        composerDrafts.beginPicker(state.conversationId)
                        attachmentPicker.launch(
                            arrayOf(
                                "image/*",
                                "text/*",
                                "application/json",
                                "application/pdf",
                                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                            )
                        )
                    }
                },
                onPasteAttachments = pasteAttachments@ { clip ->
                    if (!canMutateDraft()) return@pasteAttachments false
                    val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE)
                        as android.content.ClipboardManager
                    val uris = attachmentUrisFromClip(clip ?: clipboard.primaryClip)
                    if (uris.isNotEmpty()) importAttachmentUris(uris)
                    uris.isNotEmpty()
                },
                onRemoveAttachment = { id -> if (canMutateDraft()) attachments = attachments.filterNot { it.id == id } },
                onSendOrStop = sendOrStop@ {
                    if (!hasCurrentDraftScope()) return@sendOrStop
                    // Stopping an exact active owner must remain available even
                    // when a separate draft cannot currently be persisted.
                    if ((!state.processingState.isGenerating || inputText.startsWith("/")) && !canMutateDraft()) return@sendOrStop
                    if (context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0)
                        android.util.Log.d("NolaAgentProtocol", "ui_send_tapped conversation=${state.conversationId} processing=${state.processingState} " +
                            "permission_saving=${state.permissionSaving} permission_failed=${state.permissionSaveFailed} importing=$isImportingAttachments")
                    if (executeCommand()) { /* Local command consumed before any admission. */ }
                    else if (state.processingState.isGenerating) {
                        viewModel.cancelGeneration(viewModel.composerDrafts.ownerKey(state.conversationId), state.activeResponseId)
                    } else if (state.currentModel == null) {
                        showModelMenu = true
                    } else if (!state.processingState.blocksInput && !state.permissionSaving && !state.permissionSaveFailed) {
                        val trimmed = inputText.trim()
                        if (!isImportingAttachments && (trimmed.isNotBlank() || attachments.isNotEmpty())) {
                            viewModel.sendMessage(
                                composeAttachmentPrompt(trimmed, attachments),
                                expertParticipantAgentIds = selectedExpertIds,
                                messageMeta = attachmentMessageMeta(trimmed, attachments),
                                attachments = attachments.mapNotNull { attachment ->
                                    attachment.base64Data?.let { data ->
                                        com.nola.domain.model.ModelAttachment(
                                            name = attachment.name,
                                            mimeType = attachment.mimeType,
                                            base64Data = data,
                                        )
                                    }
                                },
                            )
                            // The owner store clears input only after submission
                            // CAS succeeds. A conflict must preserve this draft.
                        }
                    }
                }
            ) }
            if (state.permissionApplicationStatus in setOf("saving", "pending", "failed")) {
            if (state.permissionApplicationStatus !in setOf("saved", "active")) {
            TextButton(
                onClick = { if (viewModel.beginParameterEditing()) inspector = ConversationInspector.Settings },
                modifier = Modifier
                    .padding(horizontal = NolaSpacing.lg)
                    .heightIn(min = 48.dp)
                    .testTag("chat.controls.permission")
            ) {
                Text(stringResource(R.string.chat_permission_mode) + ": " + stringResource(
                    when (state.permissionMode) {
                        com.nola.core.AndroidAgentPermissionMode.Cautious -> R.string.chat_permission_mode_cautious
                        com.nola.core.AndroidAgentPermissionMode.AllowAll -> R.string.chat_permission_mode_allow_all
                    }
                ))
            }
            Text(stringResource(when (state.permissionApplicationStatus) {
                "saving" -> R.string.chat_permission_state_saving
                "checking" -> R.string.chat_permission_state_checking
                "failed" -> R.string.chat_permission_state_failed
                "pending" -> R.string.chat_permission_state_pending
                "active" -> R.string.chat_permission_state_active
                else -> R.string.chat_permission_state_saved
            }), style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = NolaSpacing.lg).testTag("chat.controls.permission.status"))
            }
            }
        }
    }

    }
    }

    learningMessage?.let { message ->
        ChatLearningFeedbackSheet(
            message = message,
            onDismiss = { learningMessage = null },
            onSave = { statement, appliesToFuture ->
                recordLearningFeedback(message, statement, appliesToFuture)
                if (appliesToFuture) {
                    intelligenceViewModel.saveEntry(
                        kind = IntelligenceEntryKind.Preference,
                        index = null,
                        value = statement
                    )
                }
                learningMessage = null
            }
        )
    }

    actionMessage?.let { message ->
        ChatMessageActionSheet(
            message = message,
            onDismiss = { actionMessage = null },
            onCopy = {
                val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE)
                    as android.content.ClipboardManager
                clipboard.setPrimaryClip(
                    android.content.ClipData.newPlainText("Nola message", message.text)
                )
                actionMessage = null
            },
            onTeach = if (message.role == ChatRole.assistant && !state.processingState.isGenerating) {
                {
                    actionMessage = null
                    learningMessage = message
                }
            } else {
                null
            }
        )
    }
}


@Composable
internal fun ChatTranscript(
    state: ChatUiState,
    expertReplies: Map<String, List<ChatViewModel.ConversationExpertReply>> = emptyMap(),
    resumableExpertMessages: Set<String> = emptySet(),
    resumingExpertMessage: String? = null,
    onResumeExpertMessage: (String) -> Unit = {},
    uiSurfaces: List<com.nola.data.local.UiSnapshot> = emptyList(),
    uiSurfaceFailure: String? = null,
    onUiSubmit: suspend (com.nola.data.local.UiSubmission) -> com.nola.ui.generative.UiSubmissionOutcome = {
        com.nola.ui.generative.UiSubmissionOutcome.Rejected("交互入口不可用。")
    },
    onCancelQueuedUiAction: () -> Unit = {},
    onUiAccepted: (com.nola.data.local.UiInteractionReceipt) -> Unit = {},
    onUiRestore: suspend (com.nola.data.local.UiSurfaceIdentity) -> com.nola.data.local.UiSubmission? = { null },
    onUiPreview: (suspend (com.nola.data.local.UiPreviewInput) -> com.nola.data.local.UiPreview)? = null,
    onUiReadHistory: suspend (com.nola.data.local.UiSurfaceRevisionRef) -> com.nola.data.local.UiSnapshot? = { null },
    modifier: Modifier = Modifier,
    listState: androidx.compose.foundation.lazy.LazyListState = rememberLazyListState(),
    onSuggestion: (String) -> Unit = {},
    onOpenModels: () -> Unit = {},
    onLongPress: (ConversationMessage) -> Unit = {},
    onResolveHostPermission: (String, Boolean, Boolean) -> Unit = { _, _, _ -> },
    onResolveLocalPermission: (String, Boolean, Boolean) -> Unit = { _, _, _ -> },
    onHostPermissionInstruction: (String, String) -> Unit = { _, _ -> },
    onLocalPermissionInstruction: (String, String) -> Unit = { _, _ -> },
    onEditAndRetry: () -> Unit = {},
    onSwitchModel: () -> Unit = {},
    onDismissError: () -> Unit = {},
    onInspectUnknownEffects: () -> Unit = {},
    onUnknownEffectDecision: (UnknownEffectRecoveryPresentation, com.nola.data.local.UnknownEffectRecoveryDecision?) -> Unit = { _, _ -> },
    onCompactionSourceRequested: (CompactionSourceNavigator.Request) -> Unit = {},
    onCompactionSourceConsumed: (Long) -> Unit = {},
    onCancelTask: (String) -> Unit = {},
    loadTaskDetail: (suspend (AgentCoreTaskHandle, String, Long?) -> com.nola.domain.projection.ConversationProjectionWindowV1)? = null,
    taskRecovery: (suspend (AgentCoreTaskHandle, String, com.nola.data.local.UnknownEffectRecoveryDecision?, Boolean) -> Pair<UnknownEffectRecoveryPresentation?, Boolean>)? = null,
    onSendTaskGuidance: (String, String) -> Unit = { _, _ -> },
    onResendEditedMessage: ((ConversationMessage) -> Unit)? = null,
) {
    val timelineMessages = state.timelineMessages()
    val currentTimelineIds = remember(timelineMessages) { timelineMessages.map { it.id } }
    val expertFollowSignal = remember(expertReplies) {
        expertReplies.values.flatten().map { reply ->
            Triple(reply.submissionId, (reply.text.length + expertThinkingLength(reply)) / 24, reply.isStreaming)
        }
    }
    var knownTimelineIds by remember(state.conversationId) { mutableStateOf<List<String>?>(null) }
    val appendedMessageIds = ChatTimelineMotionPolicy.appendedMessageIds(knownTimelineIds, currentTimelineIds)
    SideEffect {
        if (knownTimelineIds != currentTimelineIds) knownTimelineIds = currentTimelineIds
    }
    // orderedRuns/orderedTasks materialize their full projection. Build the
    // owner index once per authoritative projection so each visible message
    // does an O(1) lookup while answer text is streaming.
    val skillRunsByOwnerTurn = remember(state.coreSkillProjection) {
        state.coreSkillProjection.orderedRuns.groupBy { it.ownerParentTurnId }
    }
    val taskHandlesById = remember(state.coreTaskProjection) {
        state.coreTaskProjection.orderedTasks.associateBy { it.id }
    }
    var followsLatest by rememberSaveable { mutableStateOf(true) }
    var reviewingPermission by remember(listState) { mutableStateOf(false) }
    var userScrollSession by remember(listState) { mutableStateOf(false) }
    val reduceMotion = rememberReduceMotionEnabled()
    val currentReduceMotion by rememberUpdatedState(reduceMotion)
    val permissionNavigation = state.pendingPermissionNavigation(timelineMessages.size)
    val currentPermissionNavigation by rememberUpdatedState(permissionNavigation)
    val permissionScope = rememberCoroutineScope()

    val followRequests = remember(listState, state.conversationId) {
        kotlinx.coroutines.channels.Channel<Unit>(kotlinx.coroutines.channels.Channel.CONFLATED)
    }
    suspend fun scrollToLatestContent(animated: Boolean) {
        val lastIndex = listState.layoutInfo.totalItemsCount - 1
        if (lastIndex < 0) return
        if (animated) {
            listState.animateScrollToItem(lastIndex)
        } else {
            listState.scrollToItem(lastIndex)
        }
    }

    // Drag events cannot be conflated with a layout snapshot. Stop following
    // as soon as the pointer takes ownership, before its first scroll delta.
    LaunchedEffect(listState.interactionSource) {
        listState.interactionSource.interactions.collect { interaction ->
            if (interaction is androidx.compose.foundation.interaction.DragInteraction.Start) {
                userScrollSession = true
                followsLatest = false
                reviewingPermission = false
            }
        }
    }

    val readerScroll = remember(listState, state.conversationId) {
        object : androidx.compose.ui.input.nestedscroll.NestedScrollConnection {
            private var towardLatest = false
            override fun onPreScroll(available: androidx.compose.ui.geometry.Offset,
                                     source: androidx.compose.ui.input.nestedscroll.NestedScrollSource): androidx.compose.ui.geometry.Offset {
                if (source == androidx.compose.ui.input.nestedscroll.NestedScrollSource.UserInput && available.y != 0f) {
                    userScrollSession = true
                    reviewingPermission = false
                    towardLatest = available.y < 0f
                    if (!towardLatest) followsLatest = false
                }
                return androidx.compose.ui.geometry.Offset.Zero
            }
            override fun onPostScroll(consumed: androidx.compose.ui.geometry.Offset,
                                      available: androidx.compose.ui.geometry.Offset,
                                      source: androidx.compose.ui.input.nestedscroll.NestedScrollSource): androidx.compose.ui.geometry.Offset {
                if (userScrollSession && consumed.y != 0f) {
                    followsLatest = followAfterViewportChange(followsLatest, true,
                        consumed.y > 0f, !listState.canScrollForward, reviewingPermission)
                }
                return androidx.compose.ui.geometry.Offset.Zero
            }
            override suspend fun onPostFling(consumed: androidx.compose.ui.unit.Velocity,
                                             available: androidx.compose.ui.unit.Velocity): androidx.compose.ui.unit.Velocity {
                // The final native viewport is authoritative, even when the
                // final pointer delta and stop are coalesced into one frame.
                if (userScrollSession && towardLatest && !listState.canScrollForward && !reviewingPermission) followsLatest = true
                userScrollSession = false
                if (followsLatest) followRequests.trySend(Unit)
                return androidx.compose.ui.unit.Velocity.Zero
            }
        }
    }

    LaunchedEffect(
        state.compactionSourceRequest,
        state.compactionSourceTargetId,
        timelineMessages.size,
    ) {
        val target = state.compactionSourceTargetId ?: return@LaunchedEffect
        val index = timelineMessages.indexOfFirst { it.id == target }
        if (index < 0) return@LaunchedEffect
        followsLatest = false
        if (reduceMotion) listState.scrollToItem(index) else listState.animateScrollToItem(index)
        onCompactionSourceConsumed(state.compactionSourceRequest)
    }

    LaunchedEffect(listState, timelineMessages.size, followRequests, state.compactionSourceStatus) {
        snapshotFlow {
            val layout = listState.layoutInfo
            Triple(layout.viewportSize.height, layout.totalItemsCount,
                layout.visibleItemsInfo.map { it.index to it.size })
        }.collect {
            if (followsLatest && !userScrollSession && state.compactionSourceStatus != CompactionSourceStatus.Loading) {
                followRequests.trySend(Unit)
            }
        }
    }

    // One frame consumer survives a burst of deltas. Cancelling a delay-based
    // effect on every 24 characters starved follow until the stream paused.
    LaunchedEffect(followRequests) {
        for (request in followRequests) {
            fun windowMillis() = if (currentReduceMotion) NolaMotion.reducedMotionFollowPasses * NolaMotion.reducedMotionFollowInterval
                else NolaMotion.followPasses * NolaMotion.followInterval
            var deadline = android.os.SystemClock.uptimeMillis() + windowMillis()
            do {
                androidx.compose.runtime.withFrameNanos { }
                if (!followsLatest || userScrollSession) break
                if (followRequests.tryReceive().isSuccess) deadline = android.os.SystemClock.uptimeMillis() + windowMillis()
                if (listState.canScrollForward && !listState.isScrollInProgress) {
                    // Input may cancel an individual scroll, not the persistent
                    // request consumer that must resume after the gesture.
                    kotlinx.coroutines.coroutineScope { launch { scrollToLatestContent(animated = false) } }
                }
            } while (android.os.SystemClock.uptimeMillis() < deadline)
        }
    }

    // Network/agent completion and visual presentation completion are separate
    // lifecycle boundaries. Follow late Markdown/tool layout for a bounded
    // window after visible content changes. An always-running timer kept waking
    // every 90 ms even for an idle conversation.
    LaunchedEffect(
        followRequests,
        followsLatest,
        userScrollSession,
        timelineMessages.size,
        timelineMessages.lastOrNull()?.id,
        (timelineMessages.lastOrNull()?.text?.length ?: 0) / 24,
        state.thinking.length / 24,
        expertFollowSignal,
        state.toolEvents.size,
        state.toolEvents.lastOrNull()?.id,
        state.toolEvents.lastOrNull()?.status,
        ((state.toolEvents.lastOrNull()?.resultSummary?.length ?: 0) +
            (state.toolEvents.lastOrNull()?.detailPreview?.length ?: 0)) / 24,
        state.processingState.isGenerating,
        state.permissionRequest?.id,
        state.agentPermissionRequest?.requestId,
    ) {
        followRequests.trySend(Unit)
    }

    Column(modifier = modifier.fillMaxSize()) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
        LazyColumn(
            modifier = Modifier.fillMaxSize().nestedScroll(readerScroll).testTag("chat-transcript-list").semantics {
                this[ChatTranscriptMessageIds] = timelineMessages.map { it.id }
            },
            state = listState,
            contentPadding = PaddingValues(top = 10.dp, bottom = 20.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            if (timelineMessages.isEmpty() && !state.processingState.isGenerating) {
                item {
                    ChatEmptyState(
                        hasModels = state.availableModels.isNotEmpty(),
                        currentModel = state.currentModel,
                        hybridEnabled = state.hybridDeviceToolsEnabled && state.hybridDeviceCapabilities.isNotEmpty(),
                        suggestions = listOf(
                            stringResource(R.string.chat_suggestion_brainstorm_title) to stringResource(R.string.chat_suggestion_brainstorm_prompt),
                            stringResource(R.string.chat_suggestion_summarize_title) to stringResource(R.string.chat_suggestion_summarize_prompt),
                            stringResource(R.string.chat_suggestion_schedule_title) to stringResource(R.string.chat_suggestion_schedule_prompt),
                            stringResource(R.string.chat_suggestion_translate_title) to stringResource(R.string.chat_suggestion_translate_prompt)
                        ),
                        onSuggestion = onSuggestion,
                        onPrimaryAction = onOpenModels
                    )
                }
            }

            if (state.compactionSourceStatus != CompactionSourceStatus.Idle) {
                item(key = "compaction-source-status") {
                    Text(
                        text = stringResource(
                            when (state.compactionSourceStatus) {
                                CompactionSourceStatus.Loading -> R.string.compaction_source_loading
                                CompactionSourceStatus.Unavailable -> R.string.compaction_source_unavailable
                                CompactionSourceStatus.Failed -> R.string.compaction_source_failed
                                CompactionSourceStatus.Idle -> R.string.compaction_source_loading
                            },
                        ),
                        modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            state.rejectedQueuedUiAction?.takeIf { it.identity.conversationId == state.conversationId }?.let { receipt ->
                item(key = "rejected-ui-action-${receipt.identity.submissionId}") {
                    Column(Modifier.padding(16.dp).testTag("chat.uiAction.rejected")) {
                        Text(stringResource(R.string.chat_ui_action_not_executed))
                        TextButton(onClick = onCancelQueuedUiAction, enabled = !state.cancellingQueuedUiAction,
                            modifier = Modifier.testTag("chat.uiAction.cancel")) {
                            Text(stringResource(R.string.chat_ui_action_remove))
                        }
                    }
                }
            }
            uiSurfaceFailure?.let { message -> item(key = "ui-surface-error") {
                Text(message, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(16.dp))
            } }
            items(timelineMessages, key = { ChatTimelineMotionPolicy.itemKey(state.conversationId, it.id) }) { message ->
                val isActiveResponse = message.id == state.activeResponseId
                val shouldAnimateEntrance = remember(message.id) {
                    state.processingState.isGenerating && message.id in appendedMessageIds
                }
                var entrancePlayed by remember(message.id) { mutableStateOf(false) }
                val entranceAlpha = remember(message.id) {
                    Animatable(if (shouldAnimateEntrance && !reduceMotion) 0f else 1f)
                }
                val entranceOffset = remember(message.id) {
                    Animatable(if (shouldAnimateEntrance && !reduceMotion) 8f else 0f)
                }
                LaunchedEffect(message.id, reduceMotion) {
                    if (reduceMotion) {
                        entranceAlpha.snapTo(1f)
                        entranceOffset.snapTo(0f)
                        entrancePlayed = true
                    } else if (shouldAnimateEntrance && !entrancePlayed) {
                        entrancePlayed = true
                        kotlinx.coroutines.coroutineScope {
                            launch {
                                entranceAlpha.animateTo(1f, tween(NolaMotion.messageEntrance, easing = NolaMotion.streamRevealEasing))
                            }
                            launch {
                                entranceOffset.animateTo(0f, tween(NolaMotion.messageEntrance, easing = NolaMotion.streamRevealEasing))
                            }
                        }
                    }
                }
                val ownerTurnId = if (isActiveResponse) {
                    state.activeTurnId
                } else {
                    message.meta?.get(AGENT_CORE_TURN_ID_META_KEY)
                }
                Box(
                    modifier = Modifier.fillMaxWidth().graphicsLayer {
                        alpha = entranceAlpha.value
                        translationY = entranceOffset.value.dp.toPx()
                    },
                    contentAlignment = Alignment.TopCenter
                ) {
                    Column(Modifier.widthIn(max = 860.dp)) {
                    MessageBubble(
                        message = message,
                        isConversationGenerating = state.processingState.blocksInput,
                        modifier = Modifier.widthIn(max = 860.dp),
                        isActiveResponse = isActiveResponse,
                        revealOnFirstComposition = isActiveResponse || message.id == state.revealResponseId,
                        isStreaming = isActiveResponse && state.processingState.isGenerating,
                        isRetryingModel = isActiveResponse && state.traceProjection.phase == "model_retry",
                        isCompactingContext = isActiveResponse && state.traceProjection.phase == "compacting",
                        thinking = if (isActiveResponse) {
                            state.thinking
                        } else {
                            decodeAgentThinkingFromMeta(message.meta)
                        },
                        thinkingFinished = if (isActiveResponse) {
                            state.thinkingFinished
                        } else {
                            decodeAgentThinkingFromMeta(message.meta).isNotEmpty()
                        },
                        thinkingEnabled = state.currentParameters.enableThinking,
                        liveToolEvents = if (isActiveResponse) {
                            state.toolEvents.lastOrNull { event ->
                                event.status in setOf(
                                    "queued",
                                    "running",
                                    "waiting",
                                    "permission_required",
                                    "suspended",
                                    "retry_waiting",
                                )
                            }?.let(::listOf).orEmpty()
                        } else {
                            emptyList()
                        },
                        liveAnswerSources = if (isActiveResponse) state.answerSources else emptyList(),
                        skillRuns = ownerTurnId?.let(skillRunsByOwnerTurn::get).orEmpty(),
                        taskHandles = canonicalTaskHandles(state.coreTaskProjection.sessionId, message.canonicalParts, taskHandlesById),
                        taskGuidance = state.coreTaskProjection.guidance,
                        conversationId = state.conversationId,
                        onCancelTask = onCancelTask,
                        loadTaskDetail = loadTaskDetail,
                        taskRecovery = taskRecovery,
                        onSendTaskGuidance = onSendTaskGuidance,
                        onCompactionSourceRequested = onCompactionSourceRequested,
                        onLongPress = { onLongPress(message) },
                        onResendEditedMessage = onResendEditedMessage,
                        uiPartContent = { part -> CanonicalUiReference(part, uiSurfaces, onUiReadHistory, onUiSubmit, onUiAccepted, onUiRestore, onUiPreview) },
                    )
                    if (message.role == com.nola.domain.model.ChatRole.user) {
                        expertReplies[message.id].orEmpty().forEach { reply ->
                            ConversationExpertReplyBubble(reply, Modifier.padding(top = 6.dp), onResolveHostPermission,
                                onResolveLocalPermission, onHostPermissionInstruction, onLocalPermissionInstruction,
                                uiPartContent = { part -> CanonicalUiReference(part, uiSurfaces, onUiReadHistory, onUiSubmit, onUiAccepted, onUiRestore, onUiPreview) })
                        }
                        if (message.id in resumableExpertMessages) TextButton(onClick = { onResumeExpertMessage(message.id) },
                            enabled = resumingExpertMessage == null) { Text(stringResource(R.string.chat_resume_expert_message)) }
                    }

                    }
                }
            }

            // The final message changes height throughout streaming and Markdown
            // parsing, so it is not a stable scroll target. This dedicated anchor
            // gives every presentation phase the same bottom-of-transcript target.
            item(key = "chat-transcript-tail") {
                Spacer(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(12.dp)
                        .testTag(CHAT_TRANSCRIPT_TAIL_TEST_TAG)
                )
            }

            state.permissionRequest?.let { request ->
                item(key = "host_permission_${request.id}") {
                    HostPermissionCard(request, onResolveHostPermission, onHostPermissionInstruction)
                }
            }
            state.agentPermissionRequest?.takeIf { state.agentPermissionAction == null }?.let { request ->
                item(key = "local_agent_permission_${request.requestId}") {
                    LocalPermissionCard(request, onResolveLocalPermission, onLocalPermissionInstruction)
                }
            }

            state.unknownEffectRecovery?.takeIf { it.conversationId == state.conversationId }?.let { snapshot ->
                item(key = "unknown-effects-${snapshot.snapshotId}") {
                    UnknownEffectRecoveryCard(snapshot, state.resolvingUnknownEffects, onUnknownEffectDecision)
                }
            }
            state.error?.let { error ->
                item(key = "error") {
                    if (state.currentModel?.source != ModelInfo.ModelSource.WebHost && state.unknownEffectRecovery == null) {
                        TextButton(onClick = onInspectUnknownEffects, modifier = Modifier.heightIn(min = 48.dp)) {
                            Text(stringResource(R.string.chat_unknown_check))
                        }
                    }
                    ChatErrorRecoveryCard(
                        message = error,
                        onEditAndRetry = onEditAndRetry,
                        onSwitchModel = onSwitchModel,
                        onDismiss = onDismissError
                    )
                }
            }
        }
        androidx.compose.animation.AnimatedVisibility(
            visible = !followsLatest && timelineMessages.isNotEmpty(),
            modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 12.dp),
            enter = androidx.compose.animation.fadeIn(tween(if (reduceMotion) 0 else NolaMotion.fast)),
            exit = androidx.compose.animation.fadeOut(tween(if (reduceMotion) 0 else NolaMotion.fast)),
        ) {
            IconButton(onClick = {
                reviewingPermission = false
                followsLatest = true
                userScrollSession = false
                permissionScope.launch {
                    try { scrollToLatestContent(animated = !currentReduceMotion) }
                    finally { followRequests.trySend(Unit) }
                }
            }, modifier = Modifier.size(48.dp).clip(CircleShape)
                .background(MaterialTheme.colorScheme.surface)
                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, CircleShape)
                .testTag("chat.transcript.followLatest")) {
                Icon(Icons.Filled.ArrowDownward, contentDescription = stringResource(R.string.room_latest))
            }
        }
        }
        permissionNavigation?.let { pending ->
            PendingPermissionNavigation(
                count = pending.count,
                toolName = pending.toolName,
                onReview = {
                    // Navigation only: all decisions still happen on the original
                    // permission card and go through the authoritative runtime.
                    followsLatest = false
                    reviewingPermission = true
                    permissionScope.launch {
                        // A request can resolve while the click is dispatched.
                        // Never navigate using a captured numeric index or target
                        // a replacement request without another explicit click.
                        val target = currentPermissionNavigation
                            ?.takeIf { it.itemKey == pending.itemKey } ?: return@launch
                        if (target.itemIndex < listState.layoutInfo.totalItemsCount) {
                            listState.scrollToItem(target.itemIndex)
                        }
                    }
                }
            )
        }
    }
}

@Composable
internal fun PendingPermissionNavigation(count: Int, toolName: String, onReview: () -> Unit) {
    TextButton(
        onClick = onReview,
        modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)
            .testTag("chat-pending-permission")
            .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        Text(
            pluralStringResource(R.plurals.chat_pending_permission_review, count, count, hostToolDisplayName(toolName)),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
internal fun ChatErrorRecoveryCard(
    message: String,
    onEditAndRetry: () -> Unit,
    onSwitchModel: () -> Unit,
    onDismiss: () -> Unit
) {
    var expanded by remember(message) { mutableStateOf(false) }
    val context = LocalContext.current
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.sm),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.error.copy(alpha = 0.22f)),
        shape = RoundedCornerShape(NolaRadius.lg)
    ) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Icon(
                    Icons.Filled.Error,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier.size(18.dp)
                )
                Text(
                    message,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = if (expanded) Int.MAX_VALUE else 3,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f)
                )
                IconButton(onClick = onDismiss, modifier = Modifier.size(48.dp)) {
                    Icon(
                        Icons.Filled.Close,
                        contentDescription = stringResource(R.string.chat_error_dismiss),
                        tint = MaterialTheme.colorScheme.onErrorContainer,
                        modifier = Modifier.size(15.dp)
                    )
                }
            }
            androidx.compose.foundation.layout.FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = onEditAndRetry, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.chat_error_edit_retry), color = MaterialTheme.colorScheme.onSurface)
                }
                TextButton(onClick = onSwitchModel, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(R.string.chat_error_switch_model), color = MaterialTheme.colorScheme.onSurface)
                }
                TextButton(onClick = { expanded = !expanded }, modifier = Modifier.heightIn(min = 48.dp)) {
                    Text(stringResource(if (expanded) R.string.canonical_hide_details else R.string.canonical_details), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (expanded) {
                    TextButton(modifier = Modifier.heightIn(min = 48.dp), onClick = {
                        val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
                        clipboard.setPrimaryClip(ClipData.newPlainText("Nola", message))
                    }) { Text(stringResource(R.string.chat_message_action_copy), color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            }
        }
    }
}

@Composable
private fun ChatExecutionBadge(
    model: ModelInfo,
    hybridEnabled: Boolean
) {
    val (icon, label, tint) = when (model.source) {
        ModelInfo.ModelSource.Local -> Triple(
            Icons.Filled.PhoneAndroid,
            stringResource(R.string.chat_execution_device),
            NolaColors.success
        )
        ModelInfo.ModelSource.Cloud -> Triple(
            Icons.Filled.Cloud,
            stringResource(R.string.chat_execution_cloud),
            MaterialTheme.colorScheme.primary
        )
        ModelInfo.ModelSource.WebHost -> if (hybridEnabled) {
            Triple(
                Icons.Filled.PhoneAndroid,
                stringResource(R.string.chat_execution_hybrid),
                NolaColors.accent
            )
        } else {
            Triple(
                Icons.Filled.Computer,
                stringResource(R.string.chat_execution_host),
                MaterialTheme.colorScheme.primary
            )
        }
    }
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(999.dp))
            .background(tint.copy(alpha = 0.10f))
            .padding(horizontal = 7.dp, vertical = 3.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(12.dp))
        Text(label, style = MaterialTheme.typography.labelSmall, color = tint, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun permissionAllowLabel(toolName: String): String = stringResource(
    when (toolName) {
        "device_read_file" -> R.string.permission_action_read_file
        "device_write_file" -> R.string.permission_action_write_file
        "device_clipboard_read" -> R.string.permission_action_read_clipboard
        "device_clipboard_write" -> R.string.permission_action_write_clipboard
        "device_get_location" -> R.string.permission_action_location
        "device_search_calendar_events" -> R.string.permission_action_read_calendar
        "device_create_calendar_event" -> R.string.permission_action_create_calendar
        else -> R.string.permission_action_allow_once
    }
)

private const val MAX_CHAT_ATTACHMENTS = 10

internal fun attachmentUrisFromClip(clip: ClipData?): List<Uri> = buildList {
    if (clip == null) return@buildList
    for (index in 0 until clip.itemCount) {
        clip.getItemAt(index).uri?.let(::add)
    }
}.distinct()

private fun composeAttachmentPrompt(text: String, attachments: List<AndroidChatAttachment>): String {
    if (attachments.isEmpty()) return text
    val documentAttachments = attachments.filterNot { it.mimeType.startsWith("image/") }
    if (documentAttachments.isEmpty()) return text
    val attachmentBlock = documentAttachments.joinToString("\n\n") { attachment ->
        """
        <attachment name="${escapeAttachmentAttribute(attachment.name)}" type="${escapeAttachmentAttribute(attachment.typeLabel)}" size="${attachment.sizeLabel}">
        ${attachment.content}
        </attachment>
        """.trimIndent()
    }
    return buildString {
        if (text.isNotBlank()) appendLine(text)
        appendLine()
        appendLine("[User Attachments]")
        append(attachmentBlock)
    }.trim()
}

private const val MESSAGE_ATTACHMENTS_META_KEY = "nola.attachments"
private const val MESSAGE_DISPLAY_TEXT_META_KEY = "nola.displayMessage"
private const val MESSAGE_ATTACHMENTS_PAYLOAD_PREFIX = "nola-attachments-v1\n"

internal fun attachmentMessageMeta(
    displayText: String,
    attachments: List<AndroidChatAttachment>
): Map<String, String>? {
    if (attachments.isEmpty()) return null
    return mapOf(
        MESSAGE_DISPLAY_TEXT_META_KEY to displayText,
        MESSAGE_ATTACHMENTS_META_KEY to encodeAttachmentPayload(attachments)
    )
}

private fun encodeAttachmentPayload(attachments: List<AndroidChatAttachment>): String = buildString {
    append(MESSAGE_ATTACHMENTS_PAYLOAD_PREFIX)
    attachments.forEach { attachment ->
        append(escapeAttachmentPayloadField(attachment.name))
        append('\t')
        append(escapeAttachmentPayloadField(attachment.typeLabel))
        append('\t')
        append(attachment.sizeBytes)
        append('\t')
        append(escapeAttachmentPayloadField(attachment.uri))
        append('\t')
        append(escapeAttachmentPayloadField(attachment.mimeType))
        append('\n')
    }
}

private fun escapeAttachmentPayloadField(value: String): String = buildString(value.length) {
    value.forEach { character ->
        when (character) {
            '\\' -> append("\\\\")
            '\t' -> append("\\t")
            '\n' -> append("\\n")
            '\r' -> append("\\r")
            else -> append(character)
        }
    }
}

private fun unescapeAttachmentPayloadField(value: String): String = buildString(value.length) {
    var index = 0
    while (index < value.length) {
        val character = value[index]
        if (character != '\\' || index == value.lastIndex) {
            append(character)
            index += 1
            continue
        }
        index += 1
        when (value[index]) {
            '\\' -> append('\\')
            't' -> append('\t')
            'n' -> append('\n')
            'r' -> append('\r')
            else -> {
                append('\\')
                append(value[index])
            }
        }
        index += 1
    }
}

internal data class MessageAttachment(
    val name: String,
    val type: String,
    val sizeBytes: Int,
    val uri: String,
    val mimeType: String
) {
    val sizeLabel: String
        get() = when {
            sizeBytes < 1024 -> "$sizeBytes B"
            sizeBytes < 1024 * 1024 -> "%.1f KB".format(sizeBytes / 1024.0)
            else -> "%.1f MB".format(sizeBytes / (1024.0 * 1024.0))
        }
}

internal fun decodeMessageAttachments(meta: Map<String, String>?): List<MessageAttachment> {
    val raw = meta?.get(MESSAGE_ATTACHMENTS_META_KEY).orEmpty()
    if (raw.isBlank()) return emptyList()
    if (raw.startsWith(MESSAGE_ATTACHMENTS_PAYLOAD_PREFIX)) {
        return raw
            .removePrefix(MESSAGE_ATTACHMENTS_PAYLOAD_PREFIX)
            .lineSequence()
            .filter { it.isNotEmpty() }
            .mapNotNull { line ->
                val fields = line.split('\t')
                if (fields.size != 5) return@mapNotNull null
                val name = unescapeAttachmentPayloadField(fields[0])
                val type = unescapeAttachmentPayloadField(fields[1])
                val size = fields[2].toIntOrNull() ?: return@mapNotNull null
                val uri = unescapeAttachmentPayloadField(fields[3])
                val mimeType = unescapeAttachmentPayloadField(fields[4])
                if (name.isBlank() || uri.isBlank()) return@mapNotNull null
                MessageAttachment(
                    name = name,
                    type = type,
                    sizeBytes = size,
                    uri = uri,
                    mimeType = mimeType.ifBlank { "*/*" }
                )
            }
            .toList()
    }
    return emptyList()
}

private fun escapeAttachmentAttribute(value: String): String =
    value
        .replace("&", "&amp;")
        .replace("\"", "&quot;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")


@Composable
internal fun ChatComposerBar(
    modifier: Modifier = Modifier,
    inputText: String,
    modelControl: @Composable () -> Unit,
    onInputChange: (String) -> Unit,
    attachments: List<AndroidChatAttachment>,
    attachmentMessage: String?,
    isImportingAttachments: Boolean,
    enabled: Boolean,
    isGenerating: Boolean,
    draftWritable: Boolean,
    onAttach: () -> Unit,
    onPasteAttachments: (ClipData?) -> Boolean,
    onRemoveAttachment: (String) -> Unit,
    onSendOrStop: () -> Unit,
    onFocusChange: (Boolean) -> Unit = {},
    onMentionKey: (Key) -> Boolean = { false }
) {
    val haptics = rememberNolaHaptics()
    var inputFocused by remember { mutableStateOf(false) }
    var editorValue by remember { mutableStateOf(androidx.compose.ui.text.input.TextFieldValue(inputText)) }
    // Keep IME composition local; programmatic mention selection updates only the text/caret.
    val currentEditor = if (editorValue.text == inputText) editorValue else
        androidx.compose.ui.text.input.TextFieldValue(inputText, androidx.compose.ui.text.TextRange(inputText.length))
    val attachmentContentReceiver = remember(onPasteAttachments) {
        object : ReceiveContentListener {
            override fun onReceive(transferableContent: TransferableContent): TransferableContent? {
                return if (onPasteAttachments(transferableContent.clipEntry.clipData)) {
                    null
                } else {
                    transferableContent
                }
            }
        }
    }
    Column(
        modifier = modifier
            .testTag("chat.composer")
            .semantics { chatComposerGenerationActive = isGenerating }
            .fillMaxWidth()
            .padding(horizontal = 8.dp)
            .padding(vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)
    ) {
        if (attachments.isNotEmpty()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                attachments.forEach { attachment ->
                    AttachmentChip(
                        attachment = attachment,
                        enabled = draftWritable,
                        onRemove = { onRemoveAttachment(attachment.id) }
                    )
                }
            }
        }
        if (!attachmentMessage.isNullOrBlank()) {
            Text(
                attachmentMessage,
                style = MaterialTheme.typography.bodySmall,
                color = if (isImportingAttachments) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 8.dp)
            )
        }

        Column(Modifier.fillMaxWidth().nolaSurface(radius = 24.dp, raised = false,
            border = if (inputFocused) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant).padding(4.dp)) {
            BasicTextField(
                value = currentEditor,
                onValueChange = { value -> if (draftWritable) { editorValue = value; onInputChange(value.text) } },
                modifier = Modifier
                    .testTag("chat.composer.input")
                    .onFocusChanged { inputFocused = it.isFocused; onFocusChange(it.isFocused) }
                    .fillMaxWidth()
                    .heightIn(min = 44.dp, max = 132.dp)
                    .padding(horizontal = 12.dp, vertical = 10.dp)
                    .contentReceiver(attachmentContentReceiver)
                    .onPreviewKeyEvent { event ->
                        if (event.type == KeyEventType.KeyDown && !event.isCtrlPressed && !event.isMetaPressed && !event.isShiftPressed && !event.isAltPressed && currentEditor.composition == null && onMentionKey(event.key)) true
                        else event.type == KeyEventType.KeyDown &&
                            event.key == Key.V &&
                            (event.isCtrlPressed || event.isMetaPressed) &&
                            !event.isAltPressed && !event.isShiftPressed &&
                            currentEditor.composition == null &&
                            event.nativeKeyEvent.repeatCount == 0 &&
                            onPasteAttachments(null)
                    },
                enabled = draftWritable,
                textStyle = MaterialTheme.typography.bodyLarge.copy(
                    color = MaterialTheme.colorScheme.onSurface
                ),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                maxLines = 6,
                decorationBox = { innerTextField ->
                    Box(
                        modifier = Modifier.fillMaxWidth(),
                        contentAlignment = Alignment.CenterStart
                    ) {
                        if (inputText.isEmpty()) {
                            Text(
                                stringResource(R.string.chat_ask_placeholder),
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis
                            )
                        }
                        innerTextField()
                    }
                }
            )

            Row(
                modifier = Modifier
                    .fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(3.dp)
            ) {
                ComposerCircleButton(
                    enabled = enabled && !isImportingAttachments,
                    background = Color.Transparent,
                    contentColor = MaterialTheme.colorScheme.primary,
                    onClick = onAttach
                ) {
                    if (isImportingAttachments) {
                        CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    } else {
                        Icon(painterResource(R.drawable.nola_plus), contentDescription = stringResource(R.string.chat_attach), modifier = Modifier.size(20.dp))
                    }
                }

                Box(Modifier.weight(1f)) { modelControl() }

                val canSend = inputText.trim().isNotEmpty() || attachments.isNotEmpty()
                val sendEnabled = isGenerating || (draftWritable && (inputText.startsWith("/") || (enabled && !isImportingAttachments && canSend)))
                val sendBackground = when {
                    isGenerating -> MaterialTheme.colorScheme.primary
                    sendEnabled -> MaterialTheme.colorScheme.primary
                    else -> MaterialTheme.colorScheme.onSurface.copy(alpha = 0.09f)
                }
                val sendContent = if (sendEnabled) {
                    MaterialTheme.colorScheme.onPrimary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                }
                ComposerCircleButton(
                    enabled = sendEnabled,
                    background = sendBackground,
                    contentColor = sendContent,
                    onClick = {
                        if (isGenerating) haptics.warning() else haptics.primaryAction()
                        onSendOrStop()
                    }
                ) {
                    Box(modifier = Modifier.testTag("chat.composer.send")) {
                        Icon(
                            painterResource(if (isGenerating) R.drawable.nola_stop else R.drawable.nola_arrow),
                            contentDescription = if (isGenerating) stringResource(R.string.chat_stop) else stringResource(R.string.chat_send),
                            modifier = Modifier.size(19.dp)
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun AttachmentChip(
    attachment: AndroidChatAttachment,
    enabled: Boolean,
    onRemove: () -> Unit
) {
    val shape = RoundedCornerShape(NolaRadius.lg)
    Row(
        modifier = Modifier
            .widthIn(max = 260.dp)
            .clip(shape)
            .background(MaterialTheme.colorScheme.surface)
            .border(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = 0.28f), shape)
            .padding(start = 10.dp, top = 4.dp, end = 2.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Icon(
            Icons.Filled.Description,
            contentDescription = null,
            modifier = Modifier.size(18.dp),
            tint = MaterialTheme.colorScheme.primary
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                attachment.name,
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Text(
                "${attachment.typeLabel} · ${attachment.sizeLabel}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
        Box(
            modifier = Modifier
                .size(48.dp)
                .clip(CircleShape)
                .clickable(enabled = enabled, onClick = onRemove),
            contentAlignment = Alignment.Center
        ) {
            Icon(
                Icons.Filled.Close,
                contentDescription = stringResource(R.string.chat_attachment_remove),
                modifier = Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun ComposerCircleButton(
    enabled: Boolean,
    background: Color,
    contentColor: Color,
    onClick: () -> Unit,
    content: @Composable () -> Unit
) {
    val reduceMotion = rememberReduceMotionEnabled()
    val animatedBackground by animateColorAsState(
        targetValue = background,
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.controlState,
            easing = NolaMotion.disclosureEasing,
        ),
        label = "chat-composer-control-background",
    )
    val animatedContentColor by animateColorAsState(
        targetValue = contentColor,
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.controlState,
            easing = NolaMotion.disclosureEasing,
        ),
        label = "chat-composer-control-content",
    )
    Box(
        modifier = Modifier
            .size(48.dp)
            .clip(CircleShape)
            .background(animatedBackground)
            .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        androidx.compose.runtime.CompositionLocalProvider(
            androidx.compose.material3.LocalContentColor provides animatedContentColor
        ) {
            content()
        }
    }
}

@Composable
fun ChatEmptyState(
    hasModels: Boolean,
    currentModel: ModelInfo?,
    hybridEnabled: Boolean,
    suggestions: List<Pair<String, String>>,
    onSuggestion: (String) -> Unit,
    onPrimaryAction: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 18.dp, vertical = 28.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Column(
            modifier = Modifier
                .widthIn(max = 600.dp)
                .padding(horizontal = NolaSpacing.lg, vertical = NolaSpacing.md),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Text(
                stringResource(R.string.chat_empty_title),
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
                color = MaterialTheme.colorScheme.onSurface,
                textAlign = TextAlign.Center
            )
            Spacer(Modifier.height(8.dp))
            Text(
                currentModel?.let { model ->
                    stringResource(
                        when (model.source) {
                            ModelInfo.ModelSource.Local -> R.string.chat_empty_mode_device
                            ModelInfo.ModelSource.Cloud -> R.string.chat_empty_mode_cloud
                            ModelInfo.ModelSource.WebHost -> if (hybridEnabled) {
                                R.string.chat_empty_mode_hybrid
                            } else {
                                R.string.chat_empty_mode_host
                            }
                        }
                    )
                } ?: if (hasModels) stringResource(R.string.chat_empty_has_models)
                else stringResource(R.string.chat_empty_no_models),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center
            )
            if (hasModels) {
                Spacer(Modifier.height(18.dp))
                suggestions.forEach { (title, prompt) ->
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = 62.dp)
                            .clip(RoundedCornerShape(NolaRadius.md))
                            .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.035f))
                            .clickable { onSuggestion(prompt) }
                            .padding(horizontal = 15.dp, vertical = 11.dp),
                        verticalArrangement = Arrangement.Center
                    ) {
                        Text(
                            title,
                            style = MaterialTheme.typography.labelMedium,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.primary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        Spacer(Modifier.height(2.dp))
                        Text(
                            prompt,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                    Spacer(Modifier.height(8.dp))
                }
            } else {
                Spacer(Modifier.height(16.dp))
                Row(
                    modifier = Modifier
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(MaterialTheme.colorScheme.onSurface)
                        .clickable(role = Role.Button, onClick = onPrimaryAction)
                        .heightIn(min = 48.dp)
                        .padding(horizontal = 22.dp, vertical = 11.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        stringResource(R.string.chat_add_model_cta),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.surface
                    )
                }
            }
        }
    }
}

private fun expertThinkingLength(reply: ChatViewModel.ConversationExpertReply): Int = maxOf(reply.thinking.length,
    reply.message?.canonicalParts?.filterIsInstance<ConversationProjectionV1.Part.Reasoning>()?.sumOf { it.summary?.length ?: 0 } ?: 0)

@Composable
internal fun ConversationExpertReplyBubble(reply: ChatViewModel.ConversationExpertReply, modifier: Modifier = Modifier,
    onResolveHostPermission: (String, Boolean, Boolean) -> Unit = { _, _, _ -> },
    onResolveLocalPermission: (String, Boolean, Boolean) -> Unit = { _, _, _ -> },
    onHostPermissionInstruction: (String, String) -> Unit = { _, _ -> },
    onLocalPermissionInstruction: (String, String) -> Unit = { _, _ -> },
    uiPartContent: (@Composable (com.nola.data.local.UiReferencePresentation) -> Unit)? = null,
) {
    if (reply.message?.conversationId?.let { it != reply.participant.contextConversationId } == true) {
        Text(stringResource(R.string.canonical_message_owner_error), color = MaterialTheme.colorScheme.error,
            modifier = modifier.testTag("expert.reply.owner-error.${reply.submissionId}"))
        return
    }
    val color = rememberExpertBubbleColor(reply.participant.agentId)
    val dark = MaterialTheme.colorScheme.background.luminance() < 0.5f
    val tint = color.background(dark)
    val narrow = LocalConfiguration.current.screenWidthDp < 360
    val bubblePadding = if (narrow) 12.dp else 14.dp
    val compactWidth = expertReplyBubbleWidth(reply.text, bubblePadding)
    Row(modifier.fillMaxWidth().padding(horizontal = nolaPageGutter())
        .testTag("expert.reply.${reply.submissionId}")
        .semantics { expertReplyStreaming = reply.isStreaming; expertReplyThinkingLength = expertThinkingLength(reply) },
        verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(if (narrow) 8.dp else 10.dp)) {
        ExpertReplyIdentity(reply.participant.displayName)
        Column(Modifier.weight(1f), horizontalAlignment = Alignment.Start) {
            val label = stringResource(when (reply.state) {
                "accepted", "queued" -> R.string.room_submission_accepted
                "executing", "running" -> R.string.room_submission_executing
                "completed", "completed_with_warnings" -> R.string.room_submission_completed
                "blocked" -> R.string.room_submission_blocked
                "incomplete" -> R.string.room_submission_incomplete
                "cancelled" -> R.string.room_submission_cancelled
                "failed" -> R.string.chat_response_failed
                else -> R.string.room_submission_unconfirmed
            })
            val bubble = Modifier.widthIn(max = compactWidth?.coerceAtMost(720.dp) ?: 720.dp)
                .clip(RoundedCornerShape(topStart = 5.dp, topEnd = 14.dp, bottomEnd = 14.dp, bottomStart = 14.dp))
                .background(tint).padding(horizontal = bubblePadding, vertical = 10.dp)
            if (reply.message != null) {
                MessageBubble(message = reply.message, conversationId = reply.participant.contextConversationId,
                    contentPadding = PaddingValues(0.dp), assistantContainerColor = tint,
                    isActiveResponse = reply.isStreaming, isStreaming = reply.isStreaming,
                    thinking = reply.thinking, thinkingFinished = reply.thinkingFinished, liveToolEvents = reply.toolEvents, uiPartContent = uiPartContent)
                if (!reply.isStreaming && reply.state !in setOf("completed", "completed_with_warnings")) Text(label,
                    style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else if (reply.text.isNotBlank()) {
                StreamingText(reply.text, isStreaming = reply.isStreaming, modifier = bubble)
                MessageCopyAction(reply.text)
            } else Text(label, modifier = bubble, style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurface)
            reply.hostPermission?.let { HostPermissionCard(it, onResolveHostPermission, onHostPermissionInstruction) }
            reply.localPermission?.takeIf { reply.permissionAction == null }?.let {
                LocalPermissionCard(it, onResolveLocalPermission, onLocalPermissionInstruction)
            }
        }
    }
}

@Composable
private fun HostPermissionCard(request: com.nola.domain.model.HostAgentPermissionRequest,
    onDecision: (String, Boolean, Boolean) -> Unit, onInstruction: (String, String) -> Unit) {
    PermissionDialog(toolName = request.toolName,
        message = listOf(request.message, if (request.permissionInstructions.isEmpty()) "" else
            stringResource(R.string.permission_guidance_recorded) + "\n" + request.permissionInstructions.joinToString("\n") { it.instruction }
        ).filter { it.isNotBlank() }.joinToString("\n\n"),
        executionLocation = if (request.deviceBridge) stringResource(R.string.permission_location_device)
            else stringResource(R.string.permission_location_host, request.hostId?.take(18) ?: stringResource(R.string.permission_connected_host)),
        dataBoundary = stringResource(if (request.deviceBridge) R.string.permission_data_device else R.string.permission_data_host),
        validity = stringResource(if (request.deviceBridge) R.string.permission_valid_once else R.string.permission_valid_choice),
        allowLabel = permissionAllowLabel(request.toolName),
        conversationAllowLabel = if (!request.deviceBridge && request.allowConversation) stringResource(R.string.permission_action_allow_conversation) else null,
        onAllowConversation = if (!request.deviceBridge && request.allowConversation) ({ onDecision(request.id, true, true) }) else null,
        onAllow = { onDecision(request.id, true, false) }, onDeny = { onDecision(request.id, false, false) },
        permissionIdentity = request.id, permissionInstructions = request.permissionInstructions,
        onProvideInstruction = if (!request.deviceBridge) ({ instruction -> onInstruction(request.id, instruction) }) else null)
}

@Composable
private fun LocalPermissionCard(request: com.nola.core.AndroidAgentPermissionRequest,
    onDecision: (String, Boolean, Boolean) -> Unit, onInstruction: (String, String) -> Unit) {
    PermissionDialog(toolName = request.toolName,
        message = listOf(request.rationale.ifBlank { request.scope.orEmpty() }, if (request.permissionInstructions.isEmpty()) "" else
            stringResource(R.string.permission_guidance_recorded) + "\n" + request.permissionInstructions.joinToString("\n") { it.instruction }
        ).filter { it.isNotBlank() }.joinToString("\n\n"),
        executionLocation = stringResource(if(request.originKind=="mcp") R.string.mcp_permission_http_server else R.string.permission_location_device),
        dataBoundary = stringResource(if(request.originKind=="mcp") R.string.mcp_permission_server_and_model else R.string.permission_data_device),
        validity = stringResource(R.string.permission_valid_choice), allowLabel = permissionAllowLabel(request.toolName),
        conversationAllowLabel = if (request.allowConversation) stringResource(R.string.permission_action_allow_conversation) else null,
        onAllowConversation = if (request.allowConversation) ({ onDecision(request.requestId, true, true) }) else null,
        onAllow = { onDecision(request.requestId, true, false) }, onDeny = { onDecision(request.requestId, false, false) },
        permissionIdentity = request.requestId, permissionInstructions = request.permissionInstructions,
        onProvideInstruction = if (request.allowsInstruction) ({ instruction -> onInstruction(request.requestId, instruction) }) else null)
}

@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
fun MessageBubble(
    message: ConversationMessage,
    modifier: Modifier = Modifier,
    isActiveResponse: Boolean = false,
    revealOnFirstComposition: Boolean = false,
    isStreaming: Boolean = false,
    isRetryingModel: Boolean = false,
    isCompactingContext: Boolean = false,
    thinking: String = "",
    thinkingFinished: Boolean = false,
    thinkingEnabled: Boolean = true,
    liveToolEvents: List<HostAgentToolEvent> = emptyList(),
    liveAnswerSources: List<AnswerSource> = emptyList(),
    skillRuns: List<AgentCoreSkillRun> = emptyList(),
    taskHandles: List<AgentCoreTaskHandle> = emptyList(),
    taskGuidance: Map<String, AgentCoreTaskGuidanceQueue> = emptyMap(),
    conversationId: String? = null,
    onCancelTask: (String) -> Unit = {},
    loadTaskDetail: (suspend (AgentCoreTaskHandle, String, Long?) -> com.nola.domain.projection.ConversationProjectionWindowV1)? = null,
    taskRecovery: (suspend (AgentCoreTaskHandle, String, com.nola.data.local.UnknownEffectRecoveryDecision?, Boolean) -> Pair<UnknownEffectRecoveryPresentation?, Boolean>)? = null,
    onSendTaskGuidance: (String, String) -> Unit = { _, _ -> },
    onCompactionSourceRequested: (CompactionSourceNavigator.Request) -> Unit = {},
    onLongPress: () -> Unit = {},
    contentPadding: PaddingValues = PaddingValues(horizontal = nolaPageGutter(), vertical = 6.dp),
    isConversationGenerating: Boolean = false,
    assistantContainerColor: Color? = null,
    onResendEditedMessage: ((ConversationMessage) -> Unit)? = null,
    uiPartContent: (@Composable (com.nola.data.local.UiReferencePresentation) -> Unit)? = null,
) {
    var activityChoices by remember(message.conversationId, message.id) { mutableStateOf(CanonicalActivityDisclosureChoices()) }
    val isUser = message.role == ChatRole.user
    val alignment = if (isUser) Alignment.End else Alignment.Start
    val surfaceIsLive = message.meta?.get(CONVERSATION_SURFACE_LIVE_META_KEY) == "true"
    val surfaceCurrentToolCallId = message.meta
        ?.get(CONVERSATION_SURFACE_CURRENT_TOOL_CALL_META_KEY)
        .orEmpty()
    val archivedHostToolEvents = remember(message.role, message.meta) {
        if (isUser) emptyList() else decodeHostToolEventsFromMeta(message.meta)
    }
    val hostToolEvents = when {
        isUser -> emptyList()
        isActiveResponse -> liveToolEvents
        surfaceIsLive && surfaceCurrentToolCallId.isNotEmpty() ->
            archivedHostToolEvents
                .filter { it.id == surfaceCurrentToolCallId }
        else -> archivedHostToolEvents
    }
    val skillToolEventIds = skillRuns.mapTo(mutableSetOf()) { "core-skill:${it.runId}" }
    val visibleHostToolEvents = hostToolEvents.filterNot { it.id in skillToolEventIds }
    val hasCanonicalActivities = message.canonicalParts.any {
        it is ConversationProjectionV1.Part.Tool || it is ConversationProjectionV1.Part.Reasoning
    }
    val hasCanonicalSources = message.canonicalParts.any {
        it is ConversationProjectionV1.Part.Source || it is ConversationProjectionV1.Part.Citation || it is ConversationProjectionV1.Part.Attachment
    }
    val attachments = remember(message.role, message.meta) {
        if (isUser) decodeMessageAttachments(message.meta) else emptyList()
    }
    val archivedAnswerSources = remember(message.role, message.meta) {
        if (isUser) emptyList() else decodeAnswerSourcesFromMeta(message.meta)
    }
    val answerSources = when {
        isUser -> emptyList()
        isActiveResponse -> liveAnswerSources
        else -> archivedAnswerSources
    }
    val visibleText = message.meta?.get(MESSAGE_DISPLAY_TEXT_META_KEY) ?: message.text
    val expertPadding = if (LocalConfiguration.current.screenWidthDp < 360) 12.dp else 14.dp
    val compactExpertWidth = if (assistantContainerColor != null && thinking.isBlank() &&
        !isRetryingModel && !isCompactingContext && visibleHostToolEvents.isEmpty() &&
        skillRuns.isEmpty() && taskHandles.isEmpty() && answerSources.isEmpty() &&
        message.canonicalParts.all { it is ConversationProjectionV1.Part.Text }) {
        expertReplyBubbleWidth(visibleText, expertPadding)
    } else null
    val hapticFeedback = LocalHapticFeedback.current
    val latestLongPress by rememberUpdatedState(onLongPress)
    val messageLongPress = remember(hapticFeedback) {
        {
            hapticFeedback.performHapticFeedback(HapticFeedbackType.LongPress)
            latestLongPress()
        }
    }
    val context = LocalContext.current

    CanonicalPartReaderBoundary(message.conversationId, message.id, message.canonicalParts) { onReadText ->
    Row(Modifier.fillMaxWidth().padding(contentPadding), horizontalArrangement = Arrangement.spacedBy(7.dp), verticalAlignment = Alignment.Top) {
    if (!isUser && assistantContainerColor == null) {
        Column(Modifier.width(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Box(Modifier.size(30.dp).background(MaterialTheme.colorScheme.surface, CircleShape), contentAlignment = Alignment.Center) {
                Text("n", fontSize = 14.sp, color = MaterialTheme.colorScheme.primary)
            }
            Text("Nola", fontSize = 10.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    Column(
        modifier = modifier
            .weight(1f)
            .testTag(CHAT_MESSAGE_TEST_TAG_PREFIX + message.id)
            .combinedClickable(
                onClick = {},
                onLongClick = messageLongPress
            ),
        horizontalAlignment = alignment
    ) {
        if (isUser) {
            HistoricalPartActions(message.historicalParts)
            CompactionSourceActions(conversationId, message.canonicalParts, onCompactionSourceRequested)
            ModelAttemptDisclosures(message.canonicalParts)
        }
        if (isUser) {
            if (visibleText.isNotBlank()) {
                val userBubbleShape = RoundedCornerShape(
                    topStart = NolaRadius.lg,
                    topEnd = 5.dp,
                    bottomStart = NolaRadius.lg,
                    bottomEnd = NolaRadius.lg
                )
                Card(
                    modifier = Modifier
                        .widthIn(max = 560.dp)
                        .border(
                            width = 0.5.dp,
                            color = MaterialTheme.colorScheme.primary.copy(alpha = 0.18f),
                            shape = userBubbleShape
                        ),
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer
                    ),
                    shape = userBubbleShape
                ) {
                    Text(
                        text = visibleText,
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 11.dp),
                        style = MaterialTheme.typography.bodyLarge
                    )
                }
            }
            attachments.forEach { attachment ->
                Row(
                    modifier = Modifier
                        .widthIn(max = 420.dp)
                        .padding(top = 6.dp)
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.58f))
                        .clickable {
                            runCatching {
                                val intent = Intent(Intent.ACTION_VIEW).apply {
                                    setDataAndType(
                                        Uri.parse(attachment.uri),
                                        attachment.mimeType
                                    )
                                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                }
                                context.startActivity(Intent.createChooser(intent, attachment.name))
                            }
                        }
                        .heightIn(min = 48.dp)
                        .padding(horizontal = 12.dp, vertical = 9.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(9.dp)
                ) {
                    Icon(
                        if (attachment.mimeType.startsWith("image/")) Icons.Filled.Image else Icons.Filled.Description,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(22.dp)
                    )
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            attachment.name,
                            style = MaterialTheme.typography.labelLarge,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        Text(
                            listOf(attachment.type, attachment.sizeLabel)
                                .filter { it.isNotBlank() }
                                .joinToString(" · "),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        } else {
            Column(
                modifier = Modifier
                    .widthIn(max = compactExpertWidth?.coerceAtMost(720.dp) ?: 720.dp)
                    .clip(RoundedCornerShape(topStart = 6.dp, topEnd = 18.dp, bottomEnd = 18.dp, bottomStart = 18.dp))
                    .background(assistantContainerColor ?: MaterialTheme.colorScheme.surface)
                    .padding(horizontal = expertPadding, vertical = 10.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                if (isRetryingModel || isCompactingContext) {
                    Row(
                        modifier = Modifier.padding(horizontal = 2.dp, vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(14.dp),
                            strokeWidth = 1.5.dp,
                            color = MaterialTheme.colorScheme.primary,
                        )
                        Text(
                            text = stringResource(if (isCompactingContext) R.string.chat_context_compacting else R.string.chat_model_retrying),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                if (
                    !hasCanonicalActivities && (
                        (thinkingEnabled && thinking.isNotEmpty()) ||
                        (
                            isStreaming &&
                                message.text.isEmpty() &&
                                visibleHostToolEvents.isEmpty() &&
                                skillRuns.isEmpty() &&
                                taskHandles.isEmpty()
                        )
                    )
                ) {
                    ThinkingBlock(
                        thinking = if (thinkingEnabled) thinking else "",
                        finished = !isStreaming || thinkingFinished
                    )
                }
                if (message.canonicalParts.isNotEmpty()) {
                    canonicalMessageBlocks(message.canonicalParts).forEach { block ->
                        if (block is CanonicalMessageBlock.Part && block.part is ConversationProjectionV1.Part.Text &&
                            block.part.disclosedMarkdown.isEmpty()) return@forEach
                        if (block is CanonicalMessageBlock.Activity && block.parts.none { part ->
                            part is ConversationProjectionV1.Part.Tool || (thinkingEnabled && part is ConversationProjectionV1.Part.Reasoning &&
                                (!part.summary.isNullOrBlank() || part.partId == visibleReasoningPlaceholderId(block.parts, isStreaming)))
                        }) return@forEach
                        key(block.key) {
                        Column(Modifier.testTag("chat.part.${block.key}")) { when (block) {
                            is CanonicalMessageBlock.Activity -> CanonicalActivityCards(block.parts, isStreaming, activityChoices, { activityChoices = it }, onReadText, thinkingEnabled)
                            is CanonicalMessageBlock.Part -> when (val part = block.part) {
                                is ConversationProjectionV1.Part.Text -> if (part.disclosedMarkdown.isNotEmpty()) StreamingText(
                                    text = part.disclosedMarkdown, isStreaming = isStreaming, revealOnFirstComposition = revealOnFirstComposition,
                                    modifier = Modifier.padding(horizontal = 2.dp, vertical = 2.dp))
                                is ConversationProjectionV1.Part.UiSurface, is ConversationProjectionV1.Part.Historical -> {
                                    if (part is ConversationProjectionV1.Part.Historical) {
                                        HistoricalPartActions(listOf(part))
                                        if (part.projection.canonicalKind == "reasoning" && part.projection.decodedText != null) {
                                            if (thinkingEnabled) ThinkingBlock(part.projection.decodedText.orEmpty(), finished = true)
                                        } else part.projection.timelineText?.takeIf { it.isNotBlank() }?.let { text -> StreamingText(text,
                                            isStreaming = false, modifier = Modifier.padding(horizontal = 2.dp, vertical = 2.dp)) }
                                    }
                                    com.nola.data.local.uiReferencePresentation(part, conversationId, message.id)?.let { reference ->
                                        if (uiPartContent != null) uiPartContent(reference) else CanonicalUiReferenceUnavailable(reference)
                                    }
                                }
                                is ConversationProjectionV1.Part.Error -> CanonicalErrorPart(part, onReadText)
                                is ConversationProjectionV1.Part.UiAction -> if (part.summary.isNotBlank()) Text(part.summary)
                                is ConversationProjectionV1.Part.Compaction -> CompactionSourceActions(conversationId, listOf(part), onCompactionSourceRequested)
                                is ConversationProjectionV1.Part.ModelAttempt -> ModelAttemptDisclosures(listOf(part))
                                is ConversationProjectionV1.Part.Source, is ConversationProjectionV1.Part.Citation,
                                is ConversationProjectionV1.Part.Attachment -> CanonicalSourceCards(listOf(part), onReadText)
                                is ConversationProjectionV1.Part.Skill -> AgentCoreSkillRunList(skillRuns.filter { it.runId == part.runId }, Modifier.fillMaxWidth())
                                is ConversationProjectionV1.Part.Task -> AgentCoreTaskList(
                                    tasks = taskHandles.filter { it.id == part.taskId },
                                    loadDetail = loadTaskDetail?.let { loader -> { task, before -> loader(task, message.meta?.get("nola.canonicalMessageID") ?: message.id, before) } },
                                    recovery = taskRecovery?.let { action -> { task, decision, submit -> action(task, message.meta?.get("nola.canonicalMessageID") ?: message.id, decision, submit) } },
                                    controls = mapOf(part.taskId to part), guidance = taskGuidance, onCancel = onCancelTask,
                                    onSendGuidance = onSendTaskGuidance, modifier = Modifier.fillMaxWidth())
                                else -> Unit
                            }
                        } }
                    } }
                } else {
                    if (visibleHostToolEvents.isNotEmpty()) HostToolTimeline(visibleHostToolEvents, Modifier.fillMaxWidth())
                    if (skillRuns.isNotEmpty()) AgentCoreSkillRunList(skillRuns, Modifier.fillMaxWidth())
                    if (taskHandles.isNotEmpty()) AgentCoreTaskList(tasks = taskHandles, controls = emptyMap(),
                        loadDetail = loadTaskDetail?.let { loader -> { task, before -> loader(task, message.id, before) } },
                        recovery = taskRecovery?.let { action -> { task, decision, submit -> action(task, message.id, decision, submit) } },
                        guidance = taskGuidance, onCancel = onCancelTask, onSendGuidance = onSendTaskGuidance, modifier = Modifier.fillMaxWidth())
                    if (message.text.isNotEmpty()) StreamingText(message.text, isStreaming = isStreaming,
                        revealOnFirstComposition = revealOnFirstComposition, modifier = Modifier.padding(horizontal = 2.dp, vertical = 2.dp))
                }
                if (message.meta?.get(com.nola.domain.projection.CONVERSATION_SURFACE_MESSAGE_STATE_META_KEY) == "failed") {
                    Text(
                        text = stringResource(R.string.chat_response_failed),
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
                    )
                }
                if (!hasCanonicalSources && answerSources.isNotEmpty()) AnswerSourcesList(answerSources)
            }
        }
        if (!isUser && !isActiveResponse) {
            AnswerContextFooter(message = message)
        }
        val copyText = if (isUser) visibleText else executionAnswer(message.canonicalParts, message.text)
        if (copyText.isNotBlank()) androidx.compose.foundation.layout.FlowRow(
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp)
        ) {
            MessageCopyAction(copyText, compact = isUser)
            if (isUser) MessageEditAction(message, generating = isConversationGenerating, onResend = onResendEditedMessage)
        }
    }
    }
    }
}

@Composable
private fun CompactionSourceActions(
    conversationId: String?,
    parts: List<ConversationProjectionV1.Part>,
    onRequested: (CompactionSourceNavigator.Request) -> Unit,
) {
    parts.filterIsInstance<ConversationProjectionV1.Part.Compaction>().forEach { compaction ->
        if (conversationId == null || compaction.sourceItemIds.isEmpty() || compaction.sourceFingerprint.isBlank()) {
            return@forEach
        }
        Card(
            modifier = Modifier
                .fillMaxWidth()
                .padding(bottom = 6.dp)
                .clickable {
                    onRequested(
                        CompactionSourceNavigator.Request(
                            conversationId = conversationId,
                            compactionPartId = compaction.partId,
                            sourceItemIds = compaction.sourceItemIds,
                            sourceFingerprint = compaction.sourceFingerprint,
                        )
                    )
                },
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.32f),
            ),
        ) {
            Column(Modifier.padding(horizontal = 12.dp, vertical = 10.dp)) {
                Text(
                    stringResource(R.string.compaction_source_action, compaction.sourceItemIds.size),
                    style = MaterialTheme.typography.labelLarge,
                )
                if (compaction.summary.isNotBlank()) {
                    Text(
                        compaction.summary,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(
                    "SHA-256 · ${compaction.sourceFingerprint.take(12)}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun ModelAttemptDisclosures(parts: List<ConversationProjectionV1.Part>) {
    parts.filterIsInstance<ConversationProjectionV1.Part.ModelAttempt>().forEach { attempt ->
        val presentation = modelAttemptPresentation(attempt)
        var details by remember(attempt.partId) { mutableStateOf(false) }
        val title = when (presentation.kind) {
            ModelAttemptPresentation.Kind.Pending -> R.string.model_attempt_pending
            ModelAttemptPresentation.Kind.Running -> R.string.model_attempt_running
            ModelAttemptPresentation.Kind.Waiting -> R.string.model_attempt_waiting
            ModelAttemptPresentation.Kind.Completed -> R.string.model_attempt_completed
            ModelAttemptPresentation.Kind.Cancelled -> R.string.model_attempt_cancelled
            ModelAttemptPresentation.Kind.Failed -> R.string.model_attempt_failed
            ModelAttemptPresentation.Kind.Restarted -> R.string.model_attempt_restarted
            ModelAttemptPresentation.Kind.RouteChanged -> R.string.model_attempt_route_changed
        }
        val foreground = if (presentation.isFailure) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.onSurfaceVariant
        Card(modifier = Modifier.fillMaxWidth().padding(bottom = 6.dp),
            colors = CardDefaults.cardColors(containerColor = if (presentation.isFailure)
                MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.38f) else MaterialTheme.colorScheme.surfaceContainerLow)) {
            Row(Modifier.padding(horizontal = 12.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                Icon(if (presentation.isFailure) Icons.Default.Error else if (presentation.kind == ModelAttemptPresentation.Kind.Cancelled)
                    Icons.Default.Stop else Icons.Default.History, contentDescription = null,
                    tint = if (presentation.isFailure) MaterialTheme.colorScheme.error else foreground)
                Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Text(stringResource(title), style = MaterialTheme.typography.labelLarge)
                    Text(presentation.modelName, style = MaterialTheme.typography.bodySmall, color = foreground)
                    presentation.successorModelName?.let {
                        Text(stringResource(R.string.model_attempt_next_model, it), style = MaterialTheme.typography.labelSmall, color = foreground)
                    }
                    TextButton(onClick = { details = !details }, modifier = Modifier.heightIn(min = 48.dp)) {
                        Text(stringResource(R.string.model_attempt_details))
                    }
                    if (details) {
                        attempt.provider?.takeIf(String::isNotBlank)?.let { Text(stringResource(R.string.model_attempt_provider_field, it), style = MaterialTheme.typography.bodySmall, color = foreground) }
                        attempt.fallbackProvider?.takeIf { attempt.fallbackAttemptId?.isNotBlank() == true && it.isNotBlank() && it != attempt.provider }?.let { Text(stringResource(R.string.model_attempt_next_provider_field, it), style = MaterialTheme.typography.bodySmall, color = foreground) }
                        if (attempt.failure.message.isNotBlank()) Text(attempt.failure.message, style = MaterialTheme.typography.bodySmall, color = foreground)
                    }
                }
            }
        }
    }
}

@Composable
private fun AgentCoreSkillRunList(
    runs: List<AgentCoreSkillRun>,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(7.dp)) {
        runs.forEach { run ->
            AgentCoreSkillRunCard(run = run)
        }
    }
}

@Composable
private fun AgentCoreSkillRunCard(run: AgentCoreSkillRun) {
    val reduceMotion = rememberReduceMotionEnabled()
    var expanded by remember(run.runId) { mutableStateOf(false) }
    val active = !run.status.isTerminal
    val statusLabel = stringResource(
        when (run.status) {
            AgentCoreSkillRunStatus.Running -> R.string.chat_skill_run_running
            AgentCoreSkillRunStatus.WaitingForCatalog -> R.string.chat_skill_run_waiting
            AgentCoreSkillRunStatus.Completed -> R.string.chat_skill_run_completed
            AgentCoreSkillRunStatus.Failed -> R.string.chat_skill_run_failed
            AgentCoreSkillRunStatus.Cancelled -> R.string.chat_skill_run_cancelled
        },
    )
    val statusColor = when (run.status) {
        AgentCoreSkillRunStatus.Completed -> MaterialTheme.colorScheme.primary
        AgentCoreSkillRunStatus.Failed,
        AgentCoreSkillRunStatus.Cancelled -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NolaRadius.md))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f))
            .combinedClickable(onClick = { expanded = !expanded }, onLongClick = {}),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 11.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (active) {
                CircularProgressIndicator(modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
            } else {
                Icon(
                    imageVector = when (run.status) {
                        AgentCoreSkillRunStatus.Completed -> Icons.Filled.Check
                        AgentCoreSkillRunStatus.Failed -> Icons.Filled.Error
                        AgentCoreSkillRunStatus.Cancelled -> Icons.Filled.Close
                        else -> Icons.Filled.AutoAwesome
                    },
                    contentDescription = null,
                    modifier = Modifier.size(18.dp),
                    tint = statusColor,
                )
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = run.displayName.ifBlank { run.skillId },
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = statusLabel,
                    style = MaterialTheme.typography.bodySmall,
                    color = statusColor,
                )
            }
            NolaDisclosureChevron(expanded, reduceMotion, Modifier.size(18.dp))
        }
        AnimatedVisibility(visible = expanded, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
            Column(
                modifier = Modifier.padding(start = 39.dp, end = 11.dp, bottom = 11.dp),
                verticalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                Text(
                    text = stringResource(R.string.chat_skill_run_identity, run.mode, run.version),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    text = stringResource(
                        R.string.chat_skill_run_usage,
                        run.usage.modelTurns,
                        run.usage.toolCalls,
                        run.usage.elapsedMillis / 1000,
                    ),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                run.resultRef?.takeIf(String::isNotBlank)?.let { result ->
                    Text(
                        text = stringResource(R.string.chat_skill_run_result, result),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                run.failure?.takeIf(String::isNotBlank)?.let { failure ->
                    Text(
                        text = stringResource(R.string.chat_skill_run_error, failure),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                        maxLines = 4,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun AgentCoreTaskList(
    tasks: List<AgentCoreTaskHandle>,
    controls: Map<String, com.nola.domain.projection.ConversationProjectionV1.Part.Task>,
    loadDetail: (suspend (AgentCoreTaskHandle, Long?) -> com.nola.domain.projection.ConversationProjectionWindowV1)?,
    recovery: (suspend (AgentCoreTaskHandle, com.nola.data.local.UnknownEffectRecoveryDecision?, Boolean) -> Pair<UnknownEffectRecoveryPresentation?, Boolean>)?,
    guidance: Map<String, AgentCoreTaskGuidanceQueue>,
    onCancel: (String) -> Unit,
    onSendGuidance: (String, String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(7.dp)) {
        tasks.forEach { task ->
            AgentCoreTaskCard(
                task = task,
                control = controls[task.id],
                loadDetail = loadDetail?.let { loader -> { before -> loader(task, before) } },
                recovery = recovery?.let { action -> { decision, submit -> action(task, decision, submit) } },
                guidance = guidance[task.id],
                onCancel = { onCancel(task.id) },
                onSendGuidance = { onSendGuidance(task.id, it) },
            )
        }
    }
}

@Composable
private fun AgentCoreTaskCard(
    task: AgentCoreTaskHandle,
    control: com.nola.domain.projection.ConversationProjectionV1.Part.Task?,
    loadDetail: (suspend (Long?) -> com.nola.domain.projection.ConversationProjectionWindowV1)?,
    recovery: TaskRecoveryCommand?,
    guidance: AgentCoreTaskGuidanceQueue?,
    onCancel: () -> Unit,
    onSendGuidance: (String) -> Unit,
) {
    var showDetail by remember(task.id) { mutableStateOf(false) }
    if (showDetail && loadDetail != null) key(task.ownerSessionId, task.ownerTurnId, task.id) {
        TaskChildDetailSheet(loadDetail, recovery) { showDetail = false }
    }
    val reduceMotion = rememberReduceMotionEnabled()
    var expanded by remember(task.id) { mutableStateOf(false) }
    var draft by remember(task.id) { mutableStateOf("") }
    val active = task.state.isActive
    val waitingForInput = control?.state == ConversationProjectionV1.ActivityState.WaitingForInput
    val terminalIcon = when (task.state) {
        AgentCoreTaskState.Sealed -> Icons.Filled.Info
        AgentCoreTaskState.Completed -> Icons.Filled.Check
        AgentCoreTaskState.Failed -> Icons.Filled.Error
        AgentCoreTaskState.Cancelled -> Icons.Filled.Close
        else -> null
    }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(NolaRadius.md))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f))
            .testTag("chat.task.card.${task.id}")
            .combinedClickable(onClick = { expanded = !expanded }, onLongClick = {}),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 11.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (waitingForInput) {
                Icon(Icons.Filled.Info, contentDescription = null, modifier = Modifier.size(18.dp),
                    tint = MaterialTheme.colorScheme.tertiary)
            } else if (active) {
                CircularProgressIndicator(modifier = Modifier.size(18.dp).testTag("chat.task.running.${task.id}"), strokeWidth = 2.dp)
            } else if (terminalIcon != null) {
                Icon(
                    imageVector = terminalIcon,
                    contentDescription = null,
                    modifier = Modifier.size(18.dp),
                    tint = when (task.state) {
                        AgentCoreTaskState.Sealed -> MaterialTheme.colorScheme.onSurfaceVariant
                        AgentCoreTaskState.Completed -> MaterialTheme.colorScheme.primary
                        else -> MaterialTheme.colorScheme.error
                    },
                )
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = stringResource(
                        when (task.kind) {
                            AgentCoreTaskKind.Agent -> R.string.chat_task_agent
                            AgentCoreTaskKind.Tool -> R.string.chat_task_tool
                            AgentCoreTaskKind.Skill -> R.string.chat_task_skill
                            AgentCoreTaskKind.DataRefresh -> R.string.chat_task_data_refresh
                        },
                    ),
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.SemiBold,
                )
                task.currentStep?.let { step ->
                    Text(step, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
                if (waitingForInput) ActivityStateBadge(ConversationProjectionV1.ActivityState.WaitingForInput)
                if (task.state == AgentCoreTaskState.Sealed) {
                    Text(stringResource(R.string.chat_task_sealed), style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            task.progress?.takeIf { active }?.let { progress ->
                Text("${(progress * 100).toInt()}%", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        AnimatedVisibility(visible = expanded, enter = nolaDisclosureEnter(reduceMotion), exit = nolaDisclosureExit(reduceMotion)) {
            Column(
                modifier = Modifier.padding(start = 11.dp, end = 11.dp, bottom = 11.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (task.lineage.childSessionId != null && loadDetail != null) TextButton(
                    onClick = { showDetail = true }, modifier = Modifier.testTag("chat.task.detail.open.${task.id}")) { Text(stringResource(R.string.chat_task_details_open)) }
                guidance?.pending?.lastOrNull()?.let { pending ->
                    Text(stringResource(R.string.chat_task_guidance_pending), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.primary)
                    Text(pending.content, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 3, overflow = TextOverflow.Ellipsis)
                }
                if (active && control?.supportsGuidance == true && !control.parentRunId.isNullOrBlank() && !task.cancelRequested) {
                    androidx.compose.material3.OutlinedTextField(
                        value = draft,
                        onValueChange = { draft = it },
                        modifier = Modifier.fillMaxWidth(),
                        singleLine = true,
                        textStyle = MaterialTheme.typography.bodySmall,
                        placeholder = { Text(stringResource(R.string.chat_task_guidance_placeholder)) },
                        trailingIcon = {
                            IconButton(
                                enabled = draft.isNotBlank(),
                                onClick = {
                                    val content = draft.trim()
                                    if (content.isNotEmpty()) {
                                        draft = ""
                                        onSendGuidance(content)
                                    }
                                },
                            ) {
                                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = stringResource(R.string.chat_task_guidance_send))
                            }
                        },
                    )
                }
                if (active && control?.supportsCancellation == true && !control.parentRunId.isNullOrBlank() && !task.cancelRequested) {
                    androidx.compose.material3.TextButton(onClick = onCancel) {
                        Text(stringResource(R.string.chat_task_cancel))
                    }
                }
            }
        }
    }
}

@Composable
internal fun AnswerSourcesList(sources: List<AnswerSource>) {
    val context = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        sources.forEach { source ->
            val parsed = remember(source.uri) { Uri.parse(source.uri) }
            val domain = parsed.host ?: parsed.lastPathSegment.orEmpty()
            val location = listOfNotNull(
                source.section?.takeIf(String::isNotBlank),
                source.page?.let { "p. $it" },
                source.lineStart?.let { start ->
                    source.lineEnd?.takeIf { it != start }?.let { "L$start–$it" } ?: "L$start"
                },
            ).joinToString(" · ")
            val memoryMetadata = listOfNotNull(
                source.memoryScope?.replace('_', ' '),
                source.memoryNamespace?.replace('_', ' '),
                source.memorySource?.takeIf(String::isNotBlank),
                source.memoryObservedAt?.takeIf(String::isNotBlank),
                source.memoryUsageCount.takeIf { it > 0 }?.let { "×$it" },
                source.memorySupersedesCount.takeIf { it > 0 }?.let { "↺$it" },
                source.memoryRequiresVerification.takeIf { it }?.let { "⚠︎" },
                source.memoryTruncated.takeIf { it }?.let { "…" },
            ).joinToString(" · ")
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(NolaRadius.md))
                    .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.34f))
                    .clickable(enabled = isExternalAnswerSource(source.uri), role = Role.Button) { openAnswerSource(context, source) }
                    .heightIn(min = 48.dp)
                    .padding(horizontal = 11.dp, vertical = 9.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(9.dp),
            ) {
                Icon(Icons.Filled.Description, null, Modifier.size(18.dp), MaterialTheme.colorScheme.primary)
                Column(Modifier.weight(1f)) {
                    Text(source.title, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(listOf(domain, location).filter(String::isNotBlank).joinToString(" · "), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    if (memoryMetadata.isNotEmpty()) {
                        Text(memoryMetadata, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}

internal fun openAnswerSource(context: android.content.Context, source: AnswerSource) {
    if (!isExternalAnswerSource(source.uri)) return
    runCatching {
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(source.uri))
        context.startActivity(Intent.createChooser(intent, source.title))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChatMessageActionSheet(
    message: ConversationMessage,
    onDismiss: () -> Unit,
    onCopy: () -> Unit,
    onTeach: (() -> Unit)?
) {
    ModalBottomSheet(onDismissRequest = onDismiss,
        sheetState = androidx.compose.material3.rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 18.dp)
                .padding(bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            Text(
                stringResource(R.string.chat_message_actions_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(horizontal = 4.dp)
            )
            Text(stringResource(R.string.canonical_reader_snapshot),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            SelectionContainer {
                Text(
                    message.text,
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 320.dp)
                        .verticalScroll(rememberScrollState())
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.46f))
                        .padding(horizontal = 13.dp, vertical = 11.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            ChatMessageActionRow(
                icon = Icons.Filled.ContentCopy,
                title = stringResource(R.string.chat_message_action_copy),
                onClick = onCopy
            )
            if (onTeach != null) {
                ChatMessageActionRow(
                    icon = Icons.Filled.AutoAwesome,
                    title = stringResource(R.string.chat_message_action_teach),
                    tint = MaterialTheme.colorScheme.primary,
                    onClick = onTeach
                )
            }
        }
    }
}

@Composable
private fun ChatMessageActionRow(
    icon: ImageVector,
    title: String,
    tint: Color = MaterialTheme.colorScheme.onSurface,
    onClick: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 52.dp)
            .clip(RoundedCornerShape(NolaRadius.md))
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Box(
            modifier = Modifier
                .size(44.dp)
                .clip(CircleShape)
                .background(tint.copy(alpha = 0.09f)),
            contentAlignment = Alignment.Center
        ) {
            Icon(icon, contentDescription = null, modifier = Modifier.size(17.dp), tint = tint)
        }
        Text(
            title,
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.Medium,
            color = tint
        )
    }
}

private enum class ChatLearningFocus(
    @StringRes val titleRes: Int,
    @StringRes val statementRes: Int
) {
    Concise(R.string.chat_learning_focus_concise, R.string.chat_learning_statement_concise),
    Detailed(R.string.chat_learning_focus_detailed, R.string.chat_learning_statement_detailed),
    Tone(R.string.chat_learning_focus_tone, R.string.chat_learning_statement_tone),
    Context(R.string.chat_learning_focus_context, R.string.chat_learning_statement_context),
    Correction(R.string.chat_learning_focus_correction, R.string.chat_learning_statement_correction)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChatLearningFeedbackSheet(
    message: ConversationMessage,
    onDismiss: () -> Unit,
    onSave: (String, Boolean) -> Unit
) {
    var focus by remember { mutableStateOf(ChatLearningFocus.Concise) }
    var note by rememberSaveable(message.id) { mutableStateOf("") }
    var appliesToFuture by rememberSaveable(message.id) { mutableStateOf(true) }
    var discard by remember { mutableStateOf(false) }
    val dirty = note.isNotEmpty() || !appliesToFuture || focus != ChatLearningFocus.Concise
    val requestDismiss = { if (dirty) discard = true else onDismiss() }
    if (discard) com.nola.ui.components.DiscardChangesDialog(onDiscard = onDismiss, onKeepEditing = { discard = false })
    val defaultStatement = stringResource(focus.statementRes)

    ModalBottomSheet(onDismissRequest = requestDismiss, sheetState = com.nola.ui.components.rememberGuardedEditorSheetState(false, dirty) { discard = true }) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .imePadding()
                .padding(horizontal = 20.dp)
                .padding(bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp)
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(5.dp)) {
                Text(
                    stringResource(R.string.chat_learning_title),
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold
                )
                Text(
                    stringResource(R.string.chat_learning_subtitle),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            Column(verticalArrangement = Arrangement.spacedBy(7.dp)) {
                Text(
                    stringResource(R.string.chat_learning_preview),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Text(
                    message.text,
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.52f))
                        .padding(12.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis
                )
            }

            Row(
                modifier = Modifier.horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                ChatLearningFocus.entries.forEach { item ->
                    FilterChip(
                        selected = focus == item,
                        onClick = { focus = item },
                        label = { Text(stringResource(item.titleRes)) }
                    )
                }
            }

            Column(verticalArrangement = Arrangement.spacedBy(7.dp)) {
                Text(
                    stringResource(R.string.chat_learning_note),
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.SemiBold
                )
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = 92.dp)
                        .clip(RoundedCornerShape(NolaRadius.md))
                        .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.48f))
                        .padding(12.dp)
                ) {
                    BasicTextField(
                        value = note,
                        onValueChange = { note = it },
                        modifier = Modifier.fillMaxWidth(),
                        textStyle = MaterialTheme.typography.bodyMedium.copy(color = MaterialTheme.colorScheme.onSurface),
                        cursorBrush = SolidColor(MaterialTheme.colorScheme.primary)
                    )
                    if (note.isEmpty()) {
                        Text(
                            stringResource(R.string.chat_learning_note_placeholder),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.72f)
                        )
                    }
                }
            }

            Row(
                modifier = Modifier.fillMaxWidth().toggleable(value = appliesToFuture, role = Role.Switch, onValueChange = { appliesToFuture = it }),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        stringResource(R.string.chat_learning_future),
                        style = MaterialTheme.typography.bodyMedium,
                        fontWeight = FontWeight.SemiBold
                    )
                    Text(
                        stringResource(R.string.chat_learning_future_detail),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Switch(checked = appliesToFuture, onCheckedChange = null)
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.End,
                verticalAlignment = Alignment.CenterVertically
            ) {
                TextButton(onClick = requestDismiss) {
                    Text(stringResource(R.string.chat_learning_cancel))
                }
                TextButton(
                    onClick = {
                        val detail = note.trim()
                        onSave(if (detail.isEmpty()) defaultStatement else "$defaultStatement $detail", appliesToFuture)
                    }
                ) {
                    Text(stringResource(R.string.chat_learning_save), fontWeight = FontWeight.SemiBold)
                }
            }
        }
    }
}

private fun recordLearningFeedback(
    message: ConversationMessage,
    statement: String,
    appliesToFuture: Boolean
) {
    val eventID = UUID.randomUUID().toString()
    val semanticKey = "user-preference-${statement.lowercase().hashCode()}"
    PersonalIntelligenceCore.ingest(
        IntelligenceIngestRequest(
            events = listOf(
                IntelligenceEvent(
                    id = eventID,
                    source = "user_feedback",
                    kind = "response_correction",
                    subject = message.id.toString(),
                    summary = statement,
                    importance = 0.9,
                    confidence = 1.0,
                    tags = listOf("user_confirmed", if (appliesToFuture) "future_preference" else "answer_feedback"),
                    dedupe_key = semanticKey.takeIf { appliesToFuture }
                )
            ),
            memory_candidates = if (appliesToFuture) {
                listOf(
                    IntelligenceMemoryCandidate(
                        semantic_key = semanticKey,
                        statement = statement,
                        confidence = 1.0,
                        source_event_ids = listOf(eventID),
                        tags = listOf("user_confirmed", "response_preference")
                    )
                )
            } else {
                emptyList()
            },
            outcomes = listOf(
                IntelligenceTaskOutcome(
                    task_type = "conversation_response",
                    route = "chat",
                    model = message.meta?.get("model") ?: "unknown",
                    success = true,
                    user_corrected = true
                )
            )
        )
    )
}

@Composable
private fun AnswerContextFooter(message: ConversationMessage) {
    val reduceMotion = rememberReduceMotionEnabled()
    val basis = remember(message.meta) {
        message.meta?.get("nola.answerBasis").orEmpty()
            .split(',').map(String::trim).filter(String::isNotEmpty).distinct()
    }
    val learnedCount = message.meta?.get("nola.learnedCount")?.toIntOrNull() ?: 0
    if (basis.isEmpty() && learnedCount == 0) return

    var expanded by remember(message.id) { mutableStateOf(false) }
    var showPersonalIntelligence by remember(message.id) { mutableStateOf(false) }
    val sourceLabels = listOfNotNull(
        stringResource(R.string.chat_answer_source_file).takeIf { "file" in basis },
        stringResource(R.string.chat_answer_source_memory).takeIf { "memory" in basis },
        stringResource(R.string.chat_answer_source_knowledge).takeIf { "knowledge" in basis },
        stringResource(R.string.chat_answer_source_skills).takeIf { "skills" in basis },
        stringResource(R.string.chat_answer_source_tools).takeIf { "tools" in basis }
    )

    Column(modifier = Modifier.fillMaxWidth().padding(horizontal = 4.dp, vertical = 4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (sourceLabels.isNotEmpty()) {
                Row(
                    modifier = Modifier
                        .clip(RoundedCornerShape(NolaRadius.pill))
                        .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.045f))
                        .clickable(role = Role.Button) { expanded = !expanded }
                        .heightIn(min = 48.dp)
                        .padding(horizontal = 9.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(5.dp)
                ) {
                    Icon(Icons.Filled.Description, null, Modifier.size(13.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text(
                        sourceLabels.joinToString(" · "),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    Icon(
                        if (expanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                        null,
                        Modifier.size(13.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
            if (learnedCount > 0) {
                Row(
                    modifier = Modifier
                        .clip(RoundedCornerShape(NolaRadius.pill))
                        .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.08f))
                        .clickable(role = Role.Button) { showPersonalIntelligence = true }
                        .heightIn(min = 48.dp)
                        .padding(horizontal = 9.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(5.dp)
                ) {
                    Icon(Icons.Filled.Check, null, Modifier.size(13.dp), tint = MaterialTheme.colorScheme.primary)
                    Text(
                        stringResource(R.string.chat_answer_memory_updates, learnedCount),
                        style = MaterialTheme.typography.labelSmall,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.primary
                    )
                }
            }
        }
        AnimatedVisibility(
            visible = expanded,
            enter = androidx.compose.animation.fadeIn(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)
            ) + androidx.compose.animation.expandVertically(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
                expandFrom = Alignment.Top,
            ),
            exit = androidx.compose.animation.fadeOut(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)
            ) + androidx.compose.animation.shrinkVertically(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
                shrinkTowards = Alignment.Top,
            ),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth().padding(top = 7.dp)
                    .clip(RoundedCornerShape(NolaRadius.md))
                    .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.035f))
                    .padding(11.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                Text(
                    stringResource(R.string.chat_answer_basis_detail),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                sourceLabels.forEach { label -> Text("• $label", style = MaterialTheme.typography.labelMedium) }
                message.meta?.get("nola.learnedPreview")?.takeIf(String::isNotBlank)?.let { preview ->
                    Text(
                        "“$preview”",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis
                    )
                }
            }
        }
    }
    if (showPersonalIntelligence) {
        PersonalIntelligenceDialog(onDismiss = { showPersonalIntelligence = false })
    }
}

@Composable
private fun HostToolTimeline(
    events: List<HostAgentToolEvent>,
    modifier: Modifier = Modifier
) {
    val reduceMotion = rememberReduceMotionEnabled()
    val presentation = hostToolActivityPresentation(events)
    // Appending streamed events must not reset the user's disclosure choice.
    var userExpanded by remember(events.firstOrNull()?.id) { mutableStateOf(false) }
    val needsAttention = presentation.state == HostToolActivityState.PermissionRequired ||
        presentation.state == HostToolActivityState.Failed
    val expanded = userExpanded || needsAttention

    Column(
        modifier = modifier
            .clip(RoundedCornerShape(NolaRadius.lg))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.22f))
            .border(
                width = 0.5.dp,
                color = MaterialTheme.colorScheme.outline.copy(alpha = 0.16f),
                shape = RoundedCornerShape(NolaRadius.lg)
            )
            .padding(horizontal = 10.dp, vertical = 6.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable(role = Role.Button) { userExpanded = !userExpanded }
                .heightIn(min = 48.dp)
                .padding(horizontal = 2.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Icon(
                Icons.Filled.QuestionAnswer,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.primary
            )
            Text(
                stringResource(R.string.chat_tool_activity),
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.weight(1f)
            )
            Text(
                when (presentation.state) {
                    HostToolActivityState.PermissionRequired ->
                        stringResource(R.string.chat_host_tool_status_permission_required)
                    HostToolActivityState.Running ->
                        stringResource(R.string.chat_tool_activity_running_format, presentation.count)
                    HostToolActivityState.Failed ->
                        stringResource(R.string.chat_tool_activity_failed_format, presentation.count)
                    HostToolActivityState.Completed ->
                        stringResource(R.string.chat_tool_activity_completed_format, presentation.count)
                },
                style = MaterialTheme.typography.labelSmall,
                color = when (presentation.state) {
                    HostToolActivityState.PermissionRequired -> MaterialTheme.colorScheme.tertiary
                    HostToolActivityState.Running -> NolaColors.accent
                    HostToolActivityState.Failed -> MaterialTheme.colorScheme.error
                    HostToolActivityState.Completed -> NolaColors.success
                }
            )
            Icon(
                imageVector = if (expanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        AnimatedVisibility(
            visible = expanded,
            enter = androidx.compose.animation.fadeIn(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)
            ) + androidx.compose.animation.expandVertically(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
                expandFrom = Alignment.Top,
            ),
            exit = androidx.compose.animation.fadeOut(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing)
            ) + androidx.compose.animation.shrinkVertically(
                tween(if (reduceMotion) 0 else NolaMotion.disclosure, easing = NolaMotion.disclosureEasing),
                shrinkTowards = Alignment.Top,
            ),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                events.forEach { tool ->
                    androidx.compose.runtime.key(tool.id) {
                        HostToolEventRow(tool)
                    }
                }
            }
        }
    }
}

@Composable
private fun HostToolEventRow(tool: HostAgentToolEvent) {
    val argsPreview = tool.argsPreview.ifBlank {
        tool.args.entries.joinToString(separator = "\n") { (key, value) -> "$key: $value" }
    }
    HostToolCard(
        toolName = tool.toolName,
        status = tool.status,
        argsPreview = argsPreview,
        resultSummary = tool.resultSummary,
        detailPreview = tool.detailPreview,
        fullOutputRef = tool.fullOutputRef,
        recovery = tool.recovery,
        nextAction = tool.nextAction,
        screenshotUrl = tool.screenshotUrl,
        finalUrl = tool.finalUrl,
        pageTitle = tool.pageTitle,
        downloadUrl = tool.downloadUrl,
        downloadFilename = tool.downloadFilename,
        screenshotWarning = tool.screenshotWarning,
        sensitive = tool.sensitive,
        error = tool.error,
        modifier = Modifier.fillMaxWidth()
    )
}

@Composable
fun StreamingText(
    text: String,
    isStreaming: Boolean,
    revealOnFirstComposition: Boolean = false,
    modifier: Modifier = Modifier
) {
    SmoothStreamingMarkdownRenderer(
        text = text,
        modifier = modifier
            .testTag(CHAT_STREAMING_MARKDOWN_TEST_TAG)
            .semantics { chatStreamingMarkdownSource = text }
            .padding(vertical = 2.dp),
        isStreaming = isStreaming,
        revealOnFirstComposition = revealOnFirstComposition,
    )
}

@Composable
private fun ProcessingBreathingDot() {
    val transition = rememberInfiniteTransition(label = "processing-breath")
    val pulse by transition.animateFloat(
        initialValue = 0.78f,
        targetValue = 1.16f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1_350),
            repeatMode = RepeatMode.Reverse
        ),
        label = "processing-scale"
    )
    val haloAlpha by transition.animateFloat(
        initialValue = 0.20f,
        targetValue = 0.06f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1_350),
            repeatMode = RepeatMode.Reverse
        ),
        label = "processing-alpha"
    )

    Box(modifier = Modifier.size(18.dp), contentAlignment = Alignment.Center) {
        Box(
            modifier = Modifier
                .size(17.dp)
                .graphicsLayer {
                    scaleX = pulse
                    scaleY = pulse
                    alpha = haloAlpha
                }
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary)
        )
        Box(
            modifier = Modifier
                .size(7.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary)
        )
    }
}

@Composable
private fun StreamingActivityIndicator() {
    var activeDot by remember { mutableStateOf(0) }
    LaunchedEffect(Unit) {
        while (true) {
            kotlinx.coroutines.delay(240)
            activeDot = (activeDot + 1) % 3
        }
    }
    Row(
        modifier = Modifier.padding(top = 5.dp, bottom = 2.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        repeat(3) { index ->
            Box(
                modifier = Modifier
                    .size(4.dp)
                    .clip(CircleShape)
                    .background(
                        MaterialTheme.colorScheme.onSurfaceVariant.copy(
                            alpha = if (index == activeDot) 0.72f else 0.20f
                        )
                    )
            )
        }
    }
}
