<script lang="ts">
    import ExpertChildHistory from '../components/ExpertChildHistory.svelte';
    import InlineExpertInput from '../components/InlineExpertInput.svelte';
    import { candidateIdentity, mentionSegments, mentionTargets, replaceMentionRange, partMentions, type InlineExpertMention } from '../lib/inlineExpertMentions';
    import {supportsMainExpertDelegation, ExpertRoleCatalogUnavailable} from '../lib/expertRoles';
    import {roomCommandAtSource} from '../lib/agentRooms';
    import { currentExpertChildMember, type ExpertChildConversation } from '../lib/expertChildConversations';
    import { activityReturnPath } from '../lib/activity';
    let returnToActivity = $state(typeof window === 'undefined' ? null : activityReturnPath(new URL(window.location.href)));
    import PrototypeIcon from '../components/PrototypeIcon.svelte';
    import { darkAppearance, toggleAppearance } from '../lib/appearance';
    import { workspaceTabCloseEvent } from '../lib/workspacePageLocation';
    import { composerCommands, commandQuery, parseComposerCommand } from '../lib/composerCommands';
    import { navigateWorkspace, workspaceContentNavigationEvent } from '../lib/workspaceNavigation';
    import { getContext, onMount, tick, untrack } from 'svelte';
    import { fade, fly } from 'svelte/transition';
    import { completesPanelSwipe, panelOutwardSign, type PanelSide, type SwipePoint } from '../lib/panelSwipe';
    import { cubicOut } from 'svelte/easing';
    import { createConversationSubmissionController, type ConversationSubmissionController } from '../lib/conversationSubmissionController';
    import { createWorkspaceDrafts, workspaceDraftsContext } from '../lib/workspaceDrafts';
    import ChatReplyError from '../components/ChatReplyError.svelte';
    import type { WorkspaceDrafts } from '../lib/workspaceDrafts';
    import { api, ApiError } from '../lib/api';
    import { parseRunObservation } from '../lib/runObservation';
    import { editedResendLabel } from '../lib/editedResendLocale';
    import { ensureBrowserClientId } from '../lib/browserClientIdentity';
    import { chatInitializationError, parseInitializationSession } from '../lib/chatInitialization';
    import { locale, t } from '../lib/i18n';
    import { randomId } from '../lib/randomId';
    import PanelResizeHandle from '../components/PanelResizeHandle.svelte';
    import { workspaceWidths, setWorkspaceWidth, visiblePanelWidths, layoutDefaults } from '../lib/workspaceLayout';
    let viewportWidth = $state(1280);
    import { timelineMessageText } from '../lib/chatTimeline';
    import { isExecutionPart } from '../lib/chatMessagePresentation';
    import type { CommittedRunContextUsage } from '../lib/runContextUsage';
    import { ConversationCacheReader } from '../lib/conversationCacheReader';
    import type { ConversationCacheUsage } from '../lib/conversationCacheUsage';
    import ChatMessageContent from '../components/ChatMessageContent.svelte';
    import ChatSurfaceReference from '../components/ChatSurfaceReference.svelte';
    import { uiIdentityKey, uiSurfaceReference, readUiSurfaceChanges } from '../lib/generativeUi';
    import type { UiSnapshot, UiInteraction, UiSubmissionOutcome } from '../lib/generativeUi';

    import ChatUnknownEffectRecovery from '../components/ChatUnknownEffectRecovery.svelte';
    import type { RecoveryOwner } from '../lib/unknownEffectRecovery';
    import type { ChatTimeline, TimelineOwner } from '../lib/chatTimeline';
    import { canReleaseChatTurn } from '../lib/chatTurn';
    import type { ChatTurn, ChatTurnOwner } from '../lib/chatTurn';
    import { composerEnterSubmits } from '../lib/composerInteraction';
    import { ChatScrollOwnership } from '../lib/chatScroll';
    import type { ChatReadingPosition, ChatScrollToken } from '../lib/chatScroll';
    import { createConversationDraftStore, parseChatConversation, parseChatConversations } from '../lib/chatConversations';
    import type { ChatConversation } from '../lib/chatConversations';
    import {parseExpertRoles} from '../lib/expertRoles';
    import {rejoinedExpertParticipant, selectedExpertMembers, ExpertSelectionChanged} from '../lib/expertMentionSelection';
    import { roomCommand, type AgentDirectory, type PersistentAgent, type RoomSnapshot, type RoomSubmission } from '../lib/agentRooms';
    import { projectExpertSubmissions } from '../lib/expertConversation';
    import ExpertConversationReply from '../components/ExpertConversationReply.svelte';
    import ExpertReplyFrame from '../components/ExpertReplyFrame.svelte';
    import ExpertManager from '../components/ExpertManager.svelte';
    import ChatConversationManager from '../components/ChatConversationManager.svelte';
    import ChatResponsivePanel from '../components/ChatResponsivePanel.svelte';
    let navigationPanel: ChatResponsivePanel;
    let inspectorPanel = $state<ChatResponsivePanel>();
    import ChatModelMenu from '../components/ChatModelMenu.svelte';
    import ChatReasoningMenu from '../components/ChatReasoningMenu.svelte';
    import ChatSessionSettings from '../components/ChatSessionSettings.svelte';
    import ChatPdfResources from '../components/ChatPdfResources.svelte';
    import ContentTabStrip from '../components/ConversationContentTabs.svelte';
    import ConversationPdfTab from '../components/ConversationPdfTab.svelte';
    import WorkspacePageTab from '../components/WorkspacePageTab.svelte';
    import WorkspaceListNavigation from '../components/WorkspaceListNavigation.svelte';
    import DesktopPluginNavigation from '../components/DesktopPluginNavigation.svelte';
    import { desktopPluginPath, type DesktopPluginPage } from '../lib/desktopPluginPages';
    let pluginTitles = $state<Record<string,string>>({});
    function reconcilePluginPages(pages: DesktopPluginPage[], removed: Set<string>) {
        pluginTitles = Object.fromEntries(pages.map(page => [desktopPluginPath(page.id), page.title]));
        if (!removed.size) return;
        function keepInstalled(tabs: ConversationContentTabs) {
            return tabs.items.reduce((state, item) => item.kind === 'page' && removed.has(item.path) ? closeContentTab(state, item.key) : state, tabs);
        }
        for (const [owner, tabs] of workspaceHost?.contentTabs ?? []) {
            workspaceHost?.contentTabs.set(owner, keepInstalled(tabs));
        }
        const current = keepInstalled(contentTabs);
        if (current !== contentTabs) updateContentTabs(current);
    }
    import WorkspaceResourceNavigation from '../components/WorkspaceResourceNavigation.svelte';
    let navigationExpanded = $state(true);
    let listCategory = $state<'conversations' | 'resources'>('conversations');
    function setListCategory(category: typeof listCategory) {
        if (viewportWidth > 760) navigationExpanded = category === listCategory ? !navigationExpanded : true;
        listCategory = category;
    }

    import { emptyContentTabs, openPdfContentTab, selectContentTab, closeContentTab, conversationTabKey, pdfTabKey, managementContentPath, openPageContentTab, updatePageContentLocation, contentTabDomId } from '../lib/conversationContentTabs';
    import type { ConversationContentTabs } from '../lib/conversationContentTabs';
    import type { ConversationPdfResource } from '../lib/chatResources';
    let contentTabs = $state<ConversationContentTabs>(emptyContentTabs());
    const selectedContentTab = $derived(contentTabs.items.find(item => item.key === contentTabs.selected));
    const managementContentSelected = $derived(selectedContentTab?.kind === 'page');
    const retainedContentTabs = $derived.by(() => {
        const stored = new Map(workspaceHost?.contentTabs.entries() ?? []);
        stored.set(active?.conversationId ?? null, contentTabs);
        return [...stored].flatMap(([conversationId, state]) => state.items.map(tab => ({conversationId, tab,
            hostId, workspaceKey: JSON.stringify([hostId, conversationId]), ownerKey: JSON.stringify([hostId, conversationId, tab.key])})));
    });
    function captureConversationReading() {
        if (!active || !workspaceHost || !transcript || contentTabs.selected !== conversationTabKey || transcript.clientHeight <= 0) return;
        const first = [...transcript.querySelectorAll<HTMLElement>('[data-message-id]')]
            .find(element => element.getBoundingClientRect().bottom > transcript.getBoundingClientRect().top);
        workspaceHost.readingPositions.set(active.conversationId, {followsLatest: scroll.followsLatest,
            scrollTop: transcript.scrollTop, anchorId: first?.dataset.messageId ?? null,
            viewportOffset: first ? contentTop(first) - transcript.scrollTop : 0});
    }
    async function restoreConversationReading(position: ChatReadingPosition, token: ChatScrollToken) {
        await tick();
        if (!transcript || !scroll.accepts(token) || contentTabs.selected !== conversationTabKey) return;
        const element = position.anchorId ? anchorElement(position.anchorId) : null;
        const target = scroll.restoredPosition(token, position, metrics(), element && position.anchorId ? {id: position.anchorId, contentTop: contentTop(element)} : null);
        if (target !== null) transcript.scrollTop = target;
        showLatest = !scroll.followsLatest;
    }
    function updateStoredPageLocation(conversationId: string | null, sourceHost: string, key: string, path: string) {
        if (sourceHost !== hostId) return;
        const currentOwner = (active?.conversationId ?? null) === conversationId;
        const stored = workspaceHost?.contentTabs.get(conversationId) ?? (currentOwner ? contentTabs : undefined);
        if (!stored) return;
        const next = updatePageContentLocation(stored, key, path);
        workspaceHost?.contentTabs.set(conversationId, next);
        if ((active?.conversationId ?? null) === conversationId) updateContentTabs(next);
    }
    function updateContentTabs(next: ConversationContentTabs) {
        const returning = contentTabs.selected !== conversationTabKey && next.selected === conversationTabKey;
        if (contentTabs.selected === conversationTabKey && next.selected !== conversationTabKey) captureConversationReading();
        contentTabs = next;
        const location = new URL(window.location.href);
        const page = next.items.find(item => item.key === next.selected);
        if (page?.kind === 'page') { inspectorTab = null; location.searchParams.set('content', page.path); }
        else location.searchParams.delete('content');
        const entry = {...window.history.state};
        if (page?.kind === 'page' && page.path.startsWith('/plugin-page?')) entry.nolaPluginDestination = true;
        else delete entry.nolaPluginDestination;
        window.history.replaceState(entry, '', location);
        if (returning) {
            const position = active && workspaceHost?.readingPositions.get(active.conversationId);
            if (position) { const token = scroll.restoreIntent(scroll.token, position); if (token) void restoreConversationReading(position, token); }
            else followLatest(scroll.token);
        }
        if (workspaceHost) workspaceHost.contentTabs.set(active?.conversationId ?? null, next);
    }
    function openManagementTab(href: string): boolean {
        let requested: URL;
        try { requested = new URL(href, window.location.origin); } catch { return false; }
        if (requested.origin === window.location.origin && !requested.username && !requested.password && requested.pathname === '/chat' && !requested.searchParams.has('conversation')) {
            updateContentTabs(selectContentTab(contentTabs, conversationTabKey));
            return true;
        }
        const target = managementContentPath(href, window.location.origin);
        if (!target) return false;
        if (viewportWidth <= 760) { navigationPanel?.closePanel(); navigateWorkspace(target.path); return true; }
        if (requested.pathname === '/plugin-page' && new URL(window.location.href).searchParams.get('content') !== target.path) {
            history.pushState(history.state, '', window.location.href);
        }
        updateContentTabs(openPageContentTab(contentTabs, target.path, target.titleKey));
        if (viewportWidth <= 760) navigationPanel?.closePanel();
        return true;
    }
    async function closeWorkspaceContentTab(key: string) {
        const item = contentTabs.items.find(item => item.key === key);
        const ownerHost = hostId, ownerConversation = active?.conversationId, ownerGeneration = generation;
        if (item?.kind === 'page') {
            const pending = workspaceTabCloseEvent(JSON.stringify([hostId, active?.conversationId ?? null, key]));
            window.dispatchEvent(pending);
            if (pending.defaultPrevented && !await messageConfirmation.ask({title: t($locale, 'chatInspector.closeTab'),
                description: t($locale, 'shell.navigationDiscard'), confirmLabel: t($locale, 'chatInspector.closeTab'), destructive: true})) return;
        }
        if (ownerHost !== hostId || ownerConversation !== active?.conversationId || ownerGeneration !== generation) return;
        updateContentTabs(closeContentTab(contentTabs, key));
    }
    function openPdfInTab(resource: ConversationPdfResource) {
        if (!active || !workspaceHost || resource.conversationId !== active.conversationId) return;
        updateContentTabs(openPdfContentTab(contentTabs, active.conversationId, resource));
        if (viewportWidth <= 1250) inspectorPanel?.closePanel();
    }

    import ChatDataSources from '../components/ChatDataSources.svelte';
    import ChatDecisionOverview from '../components/ChatDecisionOverview.svelte';
    import ChatOverview from '../components/ChatOverview.svelte';
    import ChatTextAttachments from '../components/ChatTextAttachments.svelte';
    import ChatKnowledge from '../components/ChatKnowledge.svelte';
    import { knowledgePending } from '../lib/chatKnowledge';
    import ChatActivityGoal from '../components/ChatActivityGoal.svelte';
    import ChatUserQuestions from '../components/ChatUserQuestions.svelte';
    import ChatBrowser from '../components/ChatBrowser.svelte';
    import ChatPermissionMode from '../components/ChatPermissionMode.svelte';
    import type { PermissionChoice, SessionPermissionModeState, SessionReasoningQuickState, SessionModelChoiceState } from '../lib/chatSessionSettings';
    import { displayTextAttachmentMessage } from '../lib/chatTextAttachments';
    import type { ChatTextAttachment } from '../lib/chatTextAttachments';
    import ChatPermissionPanel from '../components/ChatPermissionPanel.svelte';
    import Markdown from '../components/Markdown.svelte';
    import WorkspaceReview from './WorkspaceReview.svelte';
    import ToolbarPopover from '../components/ToolbarPopover.svelte';
    import ChatArtifacts from '../components/ChatArtifacts.svelte';
    import { prepareInPlaceResend, type InPlaceResendCompletion } from '../lib/inPlaceResend';
    import { completeEditedResend, editedResendSourceForMessage } from '../lib/editedResend';
    import { admitExpertMessage, loadExpertManifest, completeEditedExpertResend, loadExpertIntent, saveExpertIntent, clearExpertIntent, resumePreparedExpertMessage } from '../lib/expertMessage';
    import type { AdmitExpertMessageInput, ExpertMessageManifest } from '../lib/expertMessage';
    import type { EditedResendDraft } from '../lib/editedResend';
    import ChatMessageActions from '../components/ChatMessageActions.svelte';
    import { messageMutationId } from '../lib/chatMessageActions';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    let messageConfirmation: ConfirmationDialog;
    let conversationManager: ChatConversationManager;
    import { conversationExecutionOwnership } from '../lib/chatTaskDetails';
    let conversationWriting = $state(false);
    let selectingConversation = $state<number | null>(null);
    import { matchingChatMessageIds } from '../lib/chatSearch';
    import { appendedMessageIds } from '../lib/chatMessageMotion';

    const workspaceDrafts = getContext<WorkspaceDrafts | undefined>(workspaceDraftsContext) ?? createWorkspaceDrafts();
    let workspaceHost = $state.raw<ReturnType<WorkspaceDrafts['forHost']>>();
    let drafts = createConversationDraftStore();
    const scroll = new ChatScrollOwnership();
    let active = $state<ChatConversation | null>(null);
    let inspectorTab = $state<'overview' | 'review' | 'artifacts' | 'goal' | 'progress' | 'execution' | 'participants' | 'resources' | 'settings' | null>(null);
    const inspectorGroups: {label: string; icon: 'progress' | 'artifacts' | 'file'; tabs: Exclude<typeof inspectorTab, null>[]}[] = [
        {label: 'task', icon: 'progress', tabs: ['overview', 'progress', 'goal']},
        {label: 'artifacts', icon: 'artifacts', tabs: ['review', 'artifacts']},
        {label: 'context', icon: 'file', tabs: ['resources', 'execution', 'participants', 'settings']},
    ];
    const inspectorActivitySections = ['progress', 'goal'] as const;
    const activeInspectorGroup = $derived(inspectorGroups.find(group => inspectorTab !== null && group.tabs.includes(inspectorTab)));
    const inspectorLabel = $derived(t($locale, `chatInspector.${activeInspectorGroup?.label ?? 'panel'}`));
    let expandedInspectorSections = $state<string[]>(['overview']);
    let inspectorRoot = $state<HTMLElement>();
    const widths = $derived(visiblePanelWidths(viewportWidth, $workspaceWidths, !!active && !managementContentSelected && inspectorTab !== null));
    const knowledgeSavePending = $derived(!!active && knowledgePending(workspaceHost?.knowledge.get(active.conversationId)));
    let timeline = $state<ChatTimeline | null>(null);
    let observedRecovery = $state('observe');
    const executionOwnership = $derived(conversationExecutionOwnership(timeline?.projection?.execution_ownership, active?.conversationId ?? ''));
    const canBranchDuringMaintenance = $derived(!!active && !active.expertContextOwner && executionOwnership.kind === 'user' && timeline?.owner.conversationId === active.conversationId && timeline.window?.canBranchDuringMaintenance === true);
    const canInteract = $derived(!canBranchDuringMaintenance && !active?.expertContextOwner && executionOwnership.kind === 'user' && observedRecovery !== 'control_unconfirmed');
    let enteringMessageIds = $state<Set<string>>(new Set());
    let renderedMessageIds: string[] = [];
    let timelineHydrated = false;
    let historyPrepending = false;
    let reducedMotion = $state(false);
    let turn = $state<ChatTurn | null>(null);
    let challenge = $state<unknown>(null);
    let reviewText = $state('');
    let draft = $state('');
    let selectedMessageId = $state<string | null>(null);
    let searchVisible = $state(false);
    let searchQuery = $state('');
    let searchInput = $state<HTMLInputElement>();
    let generation = $state(0);
    let observedCanSubmit = $state(false);
    let queuedDispatch = $state<import('../lib/sourceSubmission.ts').SourceSubmissionReceipt | null>(null);
    let queuedCancelling = $state(false);
    let contextUsage = $state<CommittedRunContextUsage | null>(null);
    let conversationCacheUsage = $state<ConversationCacheUsage>(null);
    let conversationCacheFailed = $state(false);
    const conversationCacheReader = new ConversationCacheReader(
        (id, signal) => api(`/api/chat/conversations/${encodeURIComponent(id)}/context-usage`, { signal }),
        (value, failed) => { conversationCacheUsage = value; conversationCacheFailed = failed; });
    const cacheBoundary = $derived(contextUsage
        ? `${contextUsage.identity.run_id}:${contextUsage.attempt_sequence}:${contextUsage.event_sequence}` : null);
    $effect(() => { if (cacheBoundary !== null) conversationCacheReader.refresh(); });
    let hostId = $state('');
    const contentWorkspaceKey = $derived(JSON.stringify([hostId, active?.conversationId ?? null]));
    let historicalRecoveryOwners = $state<RecoveryOwner[]>([]);
    let clientId = $state('');
    let initializing = $state(true);
    let initializationFailed = $state(false);
    let initializationInFlight = false;
    let historyLoading = $state(false);
    let controllerError = $state('');
    let localError = $state('');
    const error = $derived(localError || controllerError);
    let notice = $state('');
    let pendingText = $state('');
    let pendingAttachmentNames = $state<string[]>([]);
    let attachments = $state<ChatTextAttachment[]>([]);
    let attachmentsReading = $state(false);
    let showLatest = $state(false);
    let inspectorVisited = $state({progress: false, resources: false, review: false});
    let mountedInspectorSections = $state<string[]>([]);
    function setInspectorExpanded(section: string, expanded: boolean) {
        if (expanded) {
            if (!mountedInspectorSections.includes(section)) mountedInspectorSections = [...mountedInspectorSections, section];
            if (!expandedInspectorSections.includes(section)) expandedInspectorSections = [...expandedInspectorSections, section];
            if (['progress', 'goal', 'execution'].includes(section)) inspectorVisited.progress = true;
            if (section === 'resources') inspectorVisited.resources = true;
            if (section === 'review') inspectorVisited.review = true;
        } else expandedInspectorSections = expandedInspectorSections.filter(value => value !== section);
    }
    $effect(() => {
        const selected = inspectorTab;
        if (!selected) return;
        untrack(() => setInspectorExpanded(selected, true));
        void tick().then(() => {
            if (inspectorTab !== selected) return;
            const section = inspectorRoot?.querySelector<HTMLElement>(`[data-inspector-section="${selected}"]`);
            if (section && inspectorRoot?.getClientRects().length) {
                const motion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
                const scroller = viewportWidth <= 1250 ? inspectorRoot.closest('dialog') : inspectorRoot;
                if (scroller) scroller.scrollTo({top: scroller.scrollTop + section.getBoundingClientRect().top - scroller.getBoundingClientRect().top - 52, behavior: motion ? 'instant' : 'smooth'});
            }
        });
    });
    let edgeSwipe: {start: SwipePoint; side: PanelSide; owner: string | undefined} | undefined;
    function beginEdgeSwipe(event: TouchEvent) {
        edgeSwipe = undefined;
        if (viewportWidth > 760 || event.touches.length !== 1 || document.querySelector('dialog:modal') || window.getSelection()?.toString()) return;
        if ((event.target as HTMLElement).closest('input, textarea, select, button, a, pre, code, [contenteditable], [role="slider"]')) return;
        const touch = event.touches[0];
        const rtl = getComputedStyle(event.currentTarget as HTMLElement).direction === 'rtl';
        const physical = touch.clientX <= 28 ? 'start' : touch.clientX >= window.innerWidth - 28 ? 'end' : null;
        if (!physical) return;
        edgeSwipe = {start:{x:touch.clientX,y:touch.clientY,time:event.timeStamp}, side:rtl ? physical === 'start' ? 'end' : 'start' : physical, owner:active?.conversationId};
    }
    function finishEdgeSwipe(event: TouchEvent) {
        const swipe = edgeSwipe; edgeSwipe = undefined;
        if (!swipe || event.changedTouches.length !== 1 || swipe.owner !== active?.conversationId) return;
        const touch = event.changedTouches[0];
        const rtl = getComputedStyle(event.currentTarget as HTMLElement).direction === 'rtl';
        if (!completesPanelSwipe(swipe.start, {x:touch.clientX,y:touch.clientY,time:event.timeStamp}, window.innerWidth, -panelOutwardSign(swipe.side, rtl))) return;
        if (swipe.side === 'start') navigationPanel?.openPanel();
        else if (active && !managementContentSelected) toggleInspector('resources');
    }
    function toggleInspector(tab: Exclude<typeof inspectorTab, null>) {
        if (managementContentSelected) updateContentTabs(selectContentTab(contentTabs, conversationTabKey));
        inspectorTab = inspectorTab === tab ? null : tab;
        if (tab === 'progress' || tab === 'goal' || tab === 'execution') inspectorVisited.progress = true;
        if (tab === 'resources') inspectorVisited.resources = true;
        if (tab === 'review') inspectorVisited.review = true;
        if (viewportWidth <= 760) navigationPanel?.closePanel();
    }
    let transcript: HTMLDivElement;
    let composer = $state<HTMLTextAreaElement>();
    let textAttachments = $state<ChatTextAttachments | null>(null);
    let sessionSettings = $state<ChatSessionSettings | null>(null);
    let permissionState = $state<SessionPermissionModeState | null>(null);
    let modelState = $state<SessionModelChoiceState | null>(null);
    let selectedModelName = $state('');
    let reasoningState = $state<SessionReasoningQuickState | null>(null);
    let disposed = false;
    let userScrollTimer: ReturnType<typeof setTimeout> | null = null;
    let pointerActive = false;
    let composing = false;
    const busy = $derived(!observedCanSubmit || !!turn && !canReleaseChatTurn(turn));
    const runStatusKey = $derived(turn?.phase === 'awaiting_permission' ? 'chatPage.awaiting'
        : turn?.phase === 'cancel_requested' ? 'chatPage.cancelPending'
        : turn?.phase === 'recovering' ? 'chatPage.uncertain' : 'chatPage.working');
    const messages = $derived(timeline?.messages ?? []);
    let uiSurfaces = $state<UiSnapshot[]>([]);
    let uiSurfaceFailure = $state('');
    let uiReadSequence = 0;
    $effect(() => {
        active?.conversationId; generation;
        uiSurfaces = [];
        uiSurfaceFailure = '';
    });
    // Tool publication boundaries, not token deltas, invalidate the projection.
    const uiPublicationBoundary = $derived(JSON.stringify(messages.flatMap(message => message.parts
        .filter(part => part.value.kind === 'ui_surface')
        .map(part => [message.id, part.id, part.value.revision, part.value.surface]))));
    $effect(() => {
        const conversationId = active?.conversationId;
        const selectionGeneration = generation;
        uiPublicationBoundary;
        const controller = new AbortController();
        if (!conversationId) { uiSurfaces = []; uiSurfaceFailure = ''; }
        else void untrack(() => refreshUiSurfaces({ conversationId, generation: selectionGeneration }, controller.signal));
        return () => controller.abort();
    });
    async function refreshUiSurfaces(selected: TimelineOwner, signal?: AbortSignal) {
        const sequence = ++uiReadSequence;
        try {
            const references = messages.flatMap(message => message.parts.flatMap(part => {
                const ref = part.value.kind === 'ui_surface' ? uiSurfaceReference(part.value, selected.conversationId, message.id) : null;
                return ref ? [ref] : [];
            }));
            const snapshots: (UiSnapshot | null)[] = [];
            for (let index = 0; index < references.length; index += 4) {
                if (signal?.aborted || !sameActive(selected) || sequence !== uiReadSequence) return;
                snapshots.push(...await Promise.all(references.slice(index, index + 4).map(reference => readUiSurfaceChanges(reference.identity,
                    uiSurfaces.find(snapshot => uiIdentityKey(snapshot.identity) === uiIdentityKey(reference.identity)), signal))));
            }
            if (sequence !== uiReadSequence || !sameActive(selected) || signal?.aborted) return;
            uiSurfaces = snapshots.filter((snapshot): snapshot is UiSnapshot => snapshot !== null);
            uiSurfaceFailure = '';
        } catch (failure) {
            if (sequence === uiReadSequence && sameActive(selected) && !signal?.aborted)
                uiSurfaceFailure = failure instanceof Error ? failure.message : String(failure);
        }
    }
    async function submitUiInteraction(interaction: UiInteraction, requestId: string, submittedAtMs: number): Promise<UiSubmissionOutcome> {
        const selected = owner();
        if (!canInteract) return { status: 'rejected', message: t($locale, 'chatPage.taskReadOnly') };
        if (!selected || interaction.identity.owner.conversation_id !== selected.conversationId)
            return { status: 'rejected', message: t($locale, 'chatPage.conversationSwitched') };
        const controller = submissionController;
        const result = await api<UiSubmissionOutcome & { receipt?: { snapshot: UiSnapshot; queued_submission_id?: string | null } }>(
            `/api/chat/conversations/${encodeURIComponent(selected.conversationId)}/ui-interactions`, {
                method: 'POST', body: JSON.stringify({ request_id: requestId,
                    input: { interaction, submission_id: requestId, submitted_at_ms: submittedAtMs } })
            });
        if (!['committed', 'rejected', 'conflict', 'unknown'].includes(result.status)) throw new Error('Invalid UI submission result');
        if (result.status === 'committed' && (!result.receipt
            || uiIdentityKey(result.receipt.snapshot.identity) !== uiIdentityKey(interaction.identity)
            || result.receipt.snapshot.revision !== interaction.expected_surface_revision + 1
            || result.receipt.queued_submission_id != null && result.receipt.queued_submission_id !== requestId)) {
            throw new Error('UI submission receipt identity mismatch');
        }
        if (result.status === 'committed' && result.receipt?.queued_submission_id &&
            controller?.owner.conversationId === selected.conversationId) {
            void controller.submitQueued(result.receipt.queued_submission_id);
        }
        if (result.status === 'committed' && result.receipt && sameActive(selected)) {
            // Invalidate any earlier read so it cannot replace this receipt.
            ++uiReadSequence;
            const snapshot = result.receipt.snapshot;
            uiSurfaces = uiSurfaces.map(surface => uiIdentityKey(surface.identity) === uiIdentityKey(snapshot.identity) ? snapshot : surface);
        }
        return result;
    }

    const replyNotices = $derived(active ? workspaceHost?.replyNotices.get(active.conversationId) ?? [] : []);
    async function continueConversation() {
        // Recovery links lead to the single creation entry, never create a second one.
        inspectorTab = null;
        navigationExpanded = true;
        navigationPanel?.openPanel();
        await tick();
        conversationManager?.focusCreateEntry();
    }
    const matchingMessageIds = $derived(matchingChatMessageIds(messages, searchVisible ? searchQuery : '', $locale));

    let searchReturnFocus: HTMLElement | null = null;
    async function openSearch() {
        if (!searchVisible) searchReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        const owner = generation;
        searchVisible = true;
        await tick();
        if (!disposed && generation === owner && searchVisible && contentTabs.selected === conversationTabKey) searchInput?.focus({preventScroll: true});
    }
    function closeSearch() {
        const opener = searchReturnFocus, owner = generation;
        searchVisible = false; searchQuery = ''; searchReturnFocus = null;
        void tick().then(() => {
            if (!disposed && generation === owner && !searchVisible && contentTabs.selected === conversationTabKey
                && !document.querySelector('dialog:modal') && opener?.isConnected && opener.getClientRects().length) opener.focus({preventScroll: true});
        });
    }
    function windowKeydown(event: KeyboardEvent) {
        // A modal panel owns keyboard interaction. Do not move focus into the
        // inert transcript when its search shortcut bubbles to the window.
        if (event.defaultPrevented || event.isComposing || document.querySelector('dialog:modal')) return;
        const mod = (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey;
        if (mod) {
            const key = event.key.toLowerCase();
            if (key === 'b') {
                event.preventDefault();
                if (viewportWidth > 760) navigationExpanded = !navigationExpanded;
                else navigationPanel?.openPanel();
                return;
            }
            if (key === ',') {
                event.preventDefault();
                openManagementTab('/settings');
                return;
            }
            if (key === 'n') {
                event.preventDefault();
                navigationExpanded = true;
                listCategory = 'conversations';
                if (viewportWidth <= 760) navigationPanel?.openPanel();
                void tick().then(() => conversationManager?.focusCreateEntry());
                return;
            }
            if (key === 'k') {
                event.preventDefault();
                navigationExpanded = true;
                listCategory = 'conversations';
                if (viewportWidth <= 760) navigationPanel?.openPanel();
                void tick().then(() => document.querySelector<HTMLInputElement>('.conversation-search input')?.focus());
                return;
            }
        }
        if (contentTabs.selected !== conversationTabKey) return;
        if (event.key === 'f' && (event.metaKey || event.ctrlKey) && !event.altKey) {
            event.preventDefault(); void openSearch();
        } else if (event.key === 'Escape' && searchVisible) {
            event.preventDefault(); closeSearch();
        } else if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(event.key)
            && !event.ctrlKey && !event.metaKey && !event.altKey
            && event.target instanceof HTMLElement
            && (event.target === document.body || transcript?.contains(event.target))
            && !event.target.closest('input, textarea, select, button, summary, a, [contenteditable]')) {
            userScrollStarted();
        }
    }

    function owner(): TimelineOwner | null { return active ? { conversationId: active.conversationId, generation } : null; }
    function turnOwner(): ChatTurnOwner | null { const value = owner(); return value && hostId && clientId ? { ...value, hostId, clientId } : null; }
    function sameActive(value: TimelineOwner): boolean { return active?.conversationId === value.conversationId && generation === value.generation && !disposed; }
    function metrics() { return { scrollTop: transcript.scrollTop, scrollHeight: transcript.scrollHeight, clientHeight: transcript.clientHeight }; }
    function anchorElement(id: string): HTMLElement | null { return [...transcript.querySelectorAll<HTMLElement>('[data-message-id]')].find(item => item.dataset.messageId === id) ?? null; }
    function contentTop(element: HTMLElement): number { return transcript.scrollTop + element.getBoundingClientRect().top - transcript.getBoundingClientRect().top; }
    let followFrame: number | null = null;
    function scheduleFollowLatest(token: ChatScrollToken) {
        if (disposed) return;
        if (followFrame !== null) cancelAnimationFrame(followFrame);
        followFrame = requestAnimationFrame(() => {
            followFrame = null;
            if (!disposed && scroll.accepts(token)) followLatest(token);
        });
    }
    function followLatest(token: ChatScrollToken) {
        if (contentTabs.selected !== conversationTabKey) return;
        if (!transcript) return;
        const target = scroll.contentChanged(token, metrics());
        if (target !== null && scroll.accepts(token) && Math.abs(transcript.scrollTop - target) > .5) transcript.scrollTop = target;
    }
    function userScrollStarted() {
        if (!transcript) return;
        scroll.beginUserScroll(scroll.token);
        // A pointer press also bubbles from form controls. Showing navigation
        // here changes transcript height between pointerdown and pointerup and
        // steals the click. Only observed scroll/end metrics change visibility.
        scheduleScrollEnd();
    }
    function transcriptPointerStarted(event: PointerEvent | TouchEvent) {
        // Control activation is not a scroll gesture, including long presses.
        // Touch movement and wheel events independently take scroll ownership.
        if (event.target instanceof Element && event.target.closest('button,input,textarea,select,a,[contenteditable="true"],[role="button"]')) return;
        if ('pointerId' in event) pointerActive = true;
        userScrollStarted();
    }
    function transcriptPointerEnded() {
        if (!pointerActive) return;
        pointerActive = false;
        scheduleScrollEnd();
    }
    function scheduleScrollEnd() {
        if (userScrollTimer) clearTimeout(userScrollTimer);
        const token = scroll.token;
        userScrollTimer = setTimeout(() => {
            userScrollTimer = null;
            if (pointerActive) return;
            if (!disposed && transcript && contentTabs.selected === conversationTabKey && scroll.accepts(token)) {
                scroll.endUserScroll(token, metrics());
                showLatest = !scroll.followsLatest;
            }
        }, 150);
    }
    function observedScroll() {
        if (contentTabs.selected !== conversationTabKey) return;
        if ((userScrollTimer || pointerActive) && transcript) {
            scroll.observe(scroll.token, metrics(), 'user');
            showLatest = !scroll.followsLatest;
            // Momentum and held keyboard scrolling extend the gesture.
            scheduleScrollEnd();
        }
    }
    function jumpToLatest() {
        if (!transcript) return;
        if (userScrollTimer) clearTimeout(userScrollTimer);
        userScrollTimer = null;
        pointerActive = false;
        const next = scroll.returnToLatest(scroll.token);
        if (next && scroll.accepts(next)) {
            const top = Math.max(0, transcript.scrollHeight - transcript.clientHeight);
            if (typeof transcript.scrollTo === 'function') transcript.scrollTo({ top, behavior: reducedMotion ? 'instant' : 'smooth' });
            else transcript.scrollTop = top;
            showLatest = false;
        }
    }
    $effect(() => {
        timeline?.version;
        replyNotices; viewportWidth;
        const token = scroll.token;
        if (transcript) void tick().then(() => { if (!disposed && scroll.accepts(token)) scheduleFollowLatest(token); });
    });
    $effect(() => {
        if (!transcript) return;
        const observer = new ResizeObserver(() => scheduleFollowLatest(scroll.token));
        observer.observe(transcript);
        return () => observer.disconnect();
    });
    let draftSelection = 0;
    let submissionController: ConversationSubmissionController | null = null;
    let expertRoom = $state<RoomSnapshot | null>(null);
    let expertSourceEpoch=$state<string|null>(null);
    function selectedExpertEpoch(){return active?workspaceHost?.mentions.get(active.conversationId)?.epoch??null:null;}
    async function expertSourceCommand<T>(command:unknown,expectedEpoch=selectedExpertEpoch()??expertSourceEpoch):Promise<T>{
        const current=expertScope();
        let pinned=expectedEpoch;
        if((command as {type?:string})?.type==='directory'){
            let capability:Awaited<ReturnType<typeof roomCommandAtSource<unknown>>>;
            try{capability=await roomCommandAtSource<unknown>({type:'role_templates'},pinned);}
            catch(cause){if(cause instanceof ApiError&&cause.code==='room_source_epoch_changed')throw cause;throw new ExpertRoleCatalogUnavailable();}
            if(!current())throw Error('Expert conversation changed');
            if(!supportsMainExpertDelegation(capability.result) || (capability.result as {features?:{inline_expert_mentions?:boolean}})?.features?.inline_expert_mentions!==true)throw new ExpertRoleCatalogUnavailable();
            pinned=capability.sourceEpoch;
        }
        const read=await roomCommandAtSource<T>(command,pinned);
        if(!current())throw Error('Expert conversation changed');
        if(selectedExperts.length&&selectedExpertEpoch()&&selectedExpertEpoch()!==read.sourceEpoch)throw Error('Expert source changed');
        expertSourceEpoch=read.sourceEpoch;return read.result;
    }
    let expertDirectory = $state<AgentDirectory | null>(null);
    let expertRejoin = $state<{owner:string; agent:PersistentAgent; room:RoomSnapshot} | null>(null);
    let expertSubmissions = $state<RoomSubmission[]>([]);
    const expertExecutionPending = $derived(expertSubmissions.some(item => ['accepted','selected','executing'].includes(item.state)));
    let expertRetryIntent = $state<AdmitExpertMessageInput | null>(null);
    let expertIntentStorageError = $state('');
    let expertResumeMessage = $state<string | null>(null);
    let expertHistoryRequest = 0;
    let occurrences = $state<InlineExpertMention[]>([]);
    const selectedExperts=$derived([...new Set(occurrences.map(m=>m.selection.agent_id))]);
    const composerUnavailable = $derived(!active || conversationWriting || selectingConversation !== null || !canInteract);
    function editComposer(text:string, next:InlineExpertMention[]) {
        if (!active || !workspaceHost || composerUnavailable) return;
        const epoch=selectedExpertEpoch()??expertSourceEpoch;
        if(next.length&&!epoch) {expertError=t($locale,'rooms.expertCatalogUnavailable');return;}
        draft=text; occurrences=next; commandDismissed=null;commandIndex=0;commandError='';expertError='';
        workspaceHost.writeComposer(active.conversationId,text,next.length?{epoch:epoch!,occurrences:$state.snapshot(next)}:undefined);
    }
    function insertExpert(agent:PersistentAgent) {
        const member=expertRoom?.room.participants.find(m=>m.active&&m.agent_id===agent.id);
        const pos=composer?.selectionStart??draft.length;
        const result=replaceMentionRange(draft,occurrences,pos,composer?.selectionEnd??pos,'@'+(member?.display_name??agent.name),candidateIdentity(agent,member));
        editComposer(result.text,result.mentions);
        void tick().then(()=>{composer?.focus();composer?.setSelectionRange(result.cursor,result.cursor);});
    }
    function convertToPlainText() { editComposer(draft,[]); }
    let mentionFocused = $state(false);
    let commandIndex = $state(0);
    let commandDismissed = $state<string | null>(null);
    let commandError = $state('');
    const commandsVisible = $derived(mentionFocused && commandQuery(draft) !== null && commandDismissed !== draft);
    const commandCandidates = $derived(composerCommands.filter(command => {
        const query = commandQuery(draft);
        return query !== null && (command.name.startsWith(query) || t($locale, command.label).toLocaleLowerCase().includes(query));
    }));
    function completeCommand(name: string) {
        draft = `/${name} `; commandError = ''; commandIndex = 0;
        if (active) drafts.write(active.conversationId, draft);
        composer?.focus();
    }
    let goalDraftRevision = $state(0);
    function executeCommand(): boolean {
        const parsed = parseComposerCommand(draft);
        if (!parsed) return false;
        if (!parsed.valid || !parsed.command) { commandError = t($locale, 'commands.invalid'); return true; }
        if (parsed.command.name === 'stop' && !busy) { commandError = t($locale, 'commands.noRun'); return true; }
        commandError = ''; draft = ''; if (active) drafts.write(active.conversationId, '');
        switch (parsed.command.name) {
            case 'model': inspectorTab = 'settings'; break;
            case 'experts': void openExperts(); break;
            case 'resources': inspectorTab = 'resources'; inspectorVisited.resources = true; break;
            case 'permissions': inspectorTab = 'settings'; break;
            case 'goal':
                inspectorTab = 'goal';
                inspectorVisited.progress = true;
                if (parsed.argument && active) {
                    workspaceHost?.objectives.write(active.conversationId, parsed.argument);
                    goalDraftRevision += 1;
                }
                break;
            case 'search': searchQuery = parsed.argument; void openSearch(); break;
            case 'plan': navigateWorkspace('/data-sources?tab=plans' + (parsed.argument ? `&draft=${encodeURIComponent(parsed.argument)}` : '')); break;
            case 'stop': void stop(); break;
            case 'help': draft = '/'; if (active) drafts.write(active.conversationId, draft); composer?.focus(); break;
        }
        return true;
    }
    const expertManifestCache = new Map<string,ExpertMessageManifest | null>();
    const expertTerminalSubmissionCache = new Map<string, RoomSubmission>();
    const isTerminalExpertSubmission = (state: RoomSubmission['state']) => !['accepted', 'selected', 'executing'].includes(state);
    function expertScope(conversationId = active?.conversationId) {
        const sourceHost=hostId,sourceClient=clientId,workspace=workspaceHost,selection=draftSelection,sourceGeneration=generation;
        return () => !disposed && !!conversationId && hostId===sourceHost && clientId===sourceClient && workspaceHost===workspace
            && draftSelection===selection && generation===sourceGeneration && active?.conversationId===conversationId;
    }
    async function loadMentionDirectory() {
        if(!active || !canInteract)throw Error('Expert conversation unavailable');
        if(expertDirectory && expertSourceEpoch)return;
        const current=expertScope();
        const directory=await expertSourceCommand<AgentDirectory>({type:'directory'});
        if(current())expertDirectory=directory;
    }
    let expertsLoading = $state(false);
    function showMentionProfile(dialog:HTMLDialogElement){dialog.showModal();return {destroy:()=>dialog.close()};}
    let mentionProfile=$state<{name:string;parent:string;host:string;epoch:string|null;ref:import('../lib/mainExpertSelection').MainExpertSelection;member?:import('../lib/agentRooms').RoomParticipant}|null>(null);
    let managingExperts = $state(false);
    let expertAdmissionPending = $state(false);
    let expertError = $state('');
    let attributedExperts = $derived.by(() => expertRoom && timeline
        ? projectExpertSubmissions(expertRoom.room.id, active?.conversationId ?? '', expertRoom.room.participants, expertSubmissions,
            new Set(timeline.messages.filter(message => message.canonical?.role === 'user').map(message => message.id))) : []);
    let expertRequest = 0;
    let expertPoll: ReturnType<typeof setInterval> | null = null;
    $effect(() => {
        const key=JSON.stringify([hostId,clientId,active?.conversationId,generation,draftSelection,expertRoom?.revision,
            timeline?.messages.filter(message=>message.role==='user').map(message=>[message.id,message.canonical?.revision,message.canonicalSnapshot?.revision])]);
        void key;
        if(expertRoom&&timeline)void refreshExpertSubmissions();
    });
    async function openExperts(force = false) {
        if (!canInteract || !active || expertsLoading) return;
        inspectorTab = 'participants';
        if (!force && expertDirectory && expertRoom) return;
        if(force&&!selectedExperts.length)expertSourceEpoch=null;
        expertsLoading = true; expertError = '';
        const conversationId = active.conversationId, scope = expertScope(conversationId), request = ++expertRequest;
        const current = () => scope() && expertRequest === request;
        try {
            const association = await expertSourceCommand<{conversation_id: string; room_id: string; room: RoomSnapshot} | null>({type:'for_conversation', conversation_id:conversationId});
            const directory = await expertSourceCommand<AgentDirectory>({type:'directory'});
            if (!current()) return;
            expertRoom = association?.room ?? null; expertDirectory = directory;
            if (force && expertRejoin?.owner === conversationId) {
                const agent = directory.agents.find(agent => agent.id === expertRejoin!.agent.id && agent.availability === 'active');
                const member = association?.room.room.participants.find(member => member.agent_id === agent?.id && member.active);
                expertRejoin = agent && association && member?.agent_revision !== agent.revision
                    ? {...expertRejoin,agent,room:association.room} : null;
            }
            if (association) await refreshExpertSubmissions(conversationId, association.room);
        } catch(cause) { if (current()) expertError=t($locale,cause instanceof ExpertRoleCatalogUnavailable?'rooms.expertCatalogUnavailable':'rooms.expertAddFailed'); }
        finally { if (current()) expertsLoading = false; }
    }
    async function refreshExpertSubmissions(conversationId = active?.conversationId, snapshot = expertRoom) {
        if (!conversationId || !snapshot || active?.conversationId !== conversationId || !timeline) return;
        const scope=expertScope(conversationId),sourceHost=hostId,sourceClient=clientId,request=++expertHistoryRequest;
        const current=()=>scope()&&request===expertHistoryRequest&&expertRoom?.room.id===snapshot.room.id&&expertRoom.revision===snapshot.revision;
        const parents=timeline.messages.flatMap(message=>{
            const source=editedResendSourceForMessage(message,{hostId:sourceHost,clientId:sourceClient,conversationId});
            return source?[source]:[];
        });
        const incoming:RoomSubmission[]=[];
        try {
            // Limit transport concurrency, not history correctness: each disclosed
            // parent has its exact Core manifest; each child is an exact read.
            for(let offset=0;offset<parents.length;offset+=8){
                if(!current())return;
                const batch=await Promise.all(parents.slice(offset,offset+8).map(async source=>{
                    const key=JSON.stringify([sourceHost,sourceClient,conversationId,source.conversationRevision,source.messageId,source.messageRevision]);
                    let manifest=expertManifestCache.get(key);
                    if(manifest===undefined){manifest=await loadExpertManifest(source);if(current())expertManifestCache.set(key,manifest);}
                    if(!manifest)return [];
                    if(manifest.input.room_id!==snapshot.room.id)throw new Error('Expert history Room changed');
                    const receipts:RoomSubmission[]=[];
                    for(const entry of manifest.entries){
                        if(!current())return [];
                        const subKey = JSON.stringify([sourceHost, sourceClient, conversationId, snapshot.room.id, entry.child_submission_id]);
                        const cached = expertTerminalSubmissionCache.get(subKey);
                        if (cached) { receipts.push(cached); continue; }
                        const receipt=await roomCommand<RoomSubmission|null>({type:'submission',room_id:manifest.input.room_id,submission_id:entry.child_submission_id});
                        if(!receipt||receipt.input.submission_id!==entry.child_submission_id||receipt.input.conversation_id!==conversationId
                            ||receipt.input.parent_message_id!==manifest.input.user_message_id||receipt.input.room_id!==snapshot.room.id
                            ||receipt.context_conversation_id!==entry.participant.context_conversation_id
                            ||receipt.selected_participant_id!==entry.participant.participant_id)throw new Error('Exact expert reply is unavailable');
                        if (isTerminalExpertSubmission(receipt.state)) expertTerminalSubmissionCache.set(subKey, receipt);
                        receipts.push(receipt);
                    }
                    return receipts;
                }));
                incoming.push(...batch.flat());
            }
            if(!current())return;
            expertSubmissions=incoming.sort((a,b)=>a.input.at_ms-b.input.at_ms||a.input.submission_id.localeCompare(b.input.submission_id));
        } catch {if(current())expertError=t($locale, 'chatPage.expertHistoryRefreshFailed');}
    }
    async function toggleExpert(agentId: string) {
        if (!canInteract || !active || !expertDirectory || expertsLoading) return;
        const agent=expertDirectory.agents.find(a=>a.id===agentId&&a.availability==='active');
        if(!agent)return;
        if(selectedExperts.includes(agentId)){
            let text=draft, next=occurrences;
            for(const item of occurrences.filter(m=>m.selection.agent_id===agentId).reverse()){
                const result=replaceMentionRange(text,next,item.start,item.start+item.text.length,'');
                text=result.text;next=result.mentions;
            }
            editComposer(text,next);
        }else insertExpert(agent);
    }
    async function rejoinExpert() {
        const request = expertRejoin;
        if (!request || expertsLoading || !canInteract || active?.conversationId !== request.owner) return;
        try { parseExpertRoles(await expertSourceCommand<unknown>({type:'role_templates'})); }
        catch { expertError = t($locale,'rooms.expertCatalogUnavailable'); return; }
        if (expertRejoin !== request || !canInteract || active?.conversationId !== request.owner) return;
        if (!await messageConfirmation.ask({title:t($locale,'rooms.rejoinExpert'),description:t($locale,'rooms.rejoinHint'),
            confirmLabel:t($locale,'rooms.rejoinExpert')})) return;
        if (expertRejoin !== request || !canInteract || active?.conversationId !== request.owner) return;
        const scope = expertScope(request.owner);
        expertsLoading = true; expertError = '';
        try {
            let joined: RoomSnapshot;
            try {
                joined = await expertSourceCommand<RoomSnapshot>({type:'rejoin',input:{room_id:request.room.room.id,
                    expected_room_revision:request.room.revision,agent_id:request.agent.id,expected_agent_revision:request.agent.revision,at_ms:Date.now()}});
            } catch (cause) {
                // A lost response may follow a committed atomic rejoin. Reconcile
                // the exact old/new child owners before offering another write.
                let observed: RoomSnapshot | null = null;
                try { observed = await expertSourceCommand<RoomSnapshot | null>({type:'read',room_id:request.room.room.id}); }
                catch { throw cause; }
                if (!observed) throw cause;
                try { rejoinedExpertParticipant(request.room, observed, request.agent); }
                catch { throw cause; }
                joined = observed;
            }
            if (!scope() || expertRejoin !== request) return;
            const member = rejoinedExpertParticipant(request.room, joined, request.agent);
            expertRoom = joined;
            const oldMentions=occurrences.filter(m=>m.selection.agent_id===member.agent_id);
            if(oldMentions.length){
                let text=draft,entries=occurrences;
                for(const old of oldMentions.reverse()){
                    const changed=replaceMentionRange(text,entries,old.start,old.start+old.text.length,'@'+member.display_name,candidateIdentity(request.agent,member));
                    text=changed.text;entries=changed.mentions;
                }
                editComposer(text,entries);
            }else insertExpert(request.agent);
            expertRejoin = null;
        } catch (cause) {if (scope()) expertError = t($locale,cause instanceof ExpertSelectionChanged ? 'rooms.expertChanged' : 'rooms.expertAddFailed');}
        finally {if (scope()) expertsLoading = false;}
    }
    function openConversationFromList(conversation: ChatConversation) {
        if (active?.conversationId === conversation.conversationId) {
            if (contentTabs.selected !== conversationTabKey) updateContentTabs(selectContentTab(contentTabs, conversationTabKey));
        } else {
            void selectConversation(conversation);
        }
        if (viewportWidth <= 760) navigationPanel?.closePanel();
    }
    async function selectConversation(conversation: ChatConversation) {
        if (disposed || !workspaceHost || active?.conversationId === conversation.conversationId) return;
        const selection = ++draftSelection;
        const sourceHost = hostId, sourceConversation = active?.conversationId;
        selectingConversation = selection;
        try {
        const sourceWorkspace = workspaceHost;
        captureConversationReading();
        await sourceWorkspace.load(conversation.conversationId);
        if (disposed || selection !== draftSelection || sourceWorkspace !== workspaceHost || hostId !== sourceHost || active?.conversationId !== sourceConversation || workspaceHost.isDeleted(conversation.conversationId)) return;
        if (!workspaceHost.isLoaded(conversation.conversationId)) { localError = 'chatPage.draftStorageFailed'; return; }
        localError = ''; controllerError = '';
        if (userScrollTimer) clearTimeout(userScrollTimer);
        renderedMessageIds = [];
        timelineHydrated = false;
        historyPrepending = false;
        enteringMessageIds = new Set();
        userScrollTimer = null;
        pointerActive = false;
        submissionController?.detach();
        mentionProfile=null;active = conversation;
        contentTabs = selectContentTab(workspaceHost.contentTabs.get(conversation.conversationId) ?? emptyContentTabs(), conversationTabKey);
        ++expertRequest; expertsLoading = false; expertResumeMessage = null; managingExperts = false;
        expertRoom = null; expertSourceEpoch=null; expertDirectory = null; expertSubmissions = []; expertAdmissionPending = false;
        occurrences = workspaceHost.mentions.get(conversation.conversationId)?.occurrences ?? []; expertError = '';
        expertIntentStorageError='';
        try { expertRetryIntent = loadExpertIntent(localStorage, {hostId,clientId}, conversation.conversationId); }
        catch (cause) { expertRetryIntent = null; expertIntentStorageError = cause instanceof Error ? cause.message : String(cause); }
        inspectorTab = null;
        inspectorVisited = {progress: false, resources: false, review: false};
        mountedInspectorSections = [];
        workspaceHost.lastConversationId = conversation.conversationId;
        permissionState = null; reasoningState = null;
        historicalRecoveryOwners = []; contextUsage = null;
        conversationCacheReader.select(conversation.conversationId);
        let controller = workspaceHost.controllers.get(conversation.conversationId);
        if (!controller) {
            controller = createConversationSubmissionController({conversation, hostId, clientId, workspace: workspaceHost});
            workspaceHost.controllers.set(conversation.conversationId, controller);
        } else controller.updateConversation(conversation);
        submissionController = controller;
        generation = controller.owner.generation;
        scroll.reset(conversation.conversationId);
        const readingPosition = workspaceHost.readingPositions.get(conversation.conversationId);
        const readingToken = readingPosition ? scroll.restoreIntent(scroll.token, readingPosition) : null;
        showLatest = false; selectedMessageId = null; attachmentsReading = false;
        closeSearch();
        const selectedController = controller;
        controller.attach(snapshot => {
            if (disposed || submissionController !== selectedController || active?.conversationId !== conversation.conversationId) return;
            const nextIds = snapshot.timeline?.messages.map(message => message.id) ?? [];
            if (timelineHydrated && snapshot.timeline) {
                enteringMessageIds = new Set(appendedMessageIds(renderedMessageIds, nextIds, historyPrepending, !!snapshot.timeline.activeSubmissionId));
            }
            renderedMessageIds = nextIds;
            timelineHydrated = snapshot.timeline !== null;
            timeline = snapshot.timeline; turn = snapshot.turn; challenge = snapshot.challenge; reviewText = snapshot.reviewText;
            observedCanSubmit = snapshot.observedCanSubmit; observedRecovery = snapshot.observedRecovery;
            contextUsage = snapshot.contextUsage; historyLoading = snapshot.historyLoading;
            queuedDispatch = snapshot.queuedDispatch; queuedCancelling = snapshot.queuedCancelling;
            controllerError = snapshot.error; notice = snapshot.notice; pendingText = snapshot.pendingText;
            pendingAttachmentNames = snapshot.pendingAttachmentNames; historicalRecoveryOwners = snapshot.historicalRecoveryOwners;
            draft = snapshot.draft; occurrences=sourceWorkspace.mentions.get(conversation.conversationId)?.occurrences??[]; attachments = snapshot.attachments; textAttachments?.sync();
        }, {
            cacheChanged: () => { conversationCacheReader.refresh(); void conversationManager?.refresh(); },
            returnedToComposer: () => focusComposer(),
            captureHistory: reason => {
                if (reason === 'metadata' && scroll.followsLatest) return null;
                if (!transcript) return null;
                const first = [...transcript.querySelectorAll<HTMLElement>('[data-message-id]')]
                    .find(element => element.getBoundingClientRect().bottom > transcript.getBoundingClientRect().top);
                const anchor = first?.dataset.messageId
                    ? scroll.beginHistoryPrepend(scroll.token, metrics(), {id: first.dataset.messageId, contentTop: contentTop(first)}) : null;
                if (reason !== 'metadata') {
                    historyPrepending = !!anchor;
                    if (anchor) showLatest = true;
                }
                return anchor;
            },
            restoreHistory: async (value, isCurrent, reason) => {
                await tick();
                if (isCurrent && !isCurrent() || submissionController !== selectedController || !transcript) return;
                const anchor = value as NonNullable<ReturnType<typeof scroll.beginHistoryPrepend>>;
                const element = anchorElement(anchor.id);
                const target = scroll.restoreHistoryAnchor(anchor, metrics(), element ? {id: anchor.id, contentTop: contentTop(element)} : null);
                if (target !== null) transcript.scrollTop = target;
                if (reason !== 'metadata') historyPrepending = false;
            },
        });
        void loadAssociatedExpertHistory(conversation.conversationId);
        const url = new URL(location.href);
        if (url.searchParams.get('conversation') !== conversation.conversationId) {
            returnToActivity = null;
            for (const key of ['from_activity', 'focus', 'request_conversation', 'run']) url.searchParams.delete(key);
        }
        url.searchParams.set('conversation', conversation.conversationId); url.searchParams.delete('content');
        history.replaceState(history.state, '', url);
        await controller.initialize();
        if (readingPosition && readingToken) await restoreConversationReading(readingPosition, readingToken);
        } finally { if (selectingConversation === selection) selectingConversation = null; }
    }
    /** Load prior expert replies for ordinary Chat without creating a Room or
     * requiring the participant picker to be opened. */
    async function loadAssociatedExpertHistory(conversationId: string) {
        const current = expertScope(conversationId);
        try {
            const association = await expertSourceCommand<{conversation_id: string; room_id: string; room: RoomSnapshot} | null>({type:'for_conversation', conversation_id:conversationId});
            if (!current()) return;
            if (!association || association.conversation_id !== conversationId) {
                return;
            }
            expertRoom = association.room;
            await refreshExpertSubmissions(conversationId, association.room);
        } catch {
            if (current() && selectedExperts.length) {
                expertError = t($locale, 'chatInspector.expertsUnavailable');
            }
        }
    }
    function loadHistory(mode: 'replace' | 'prepend') { return submissionController?.loadHistory(mode) ?? Promise.resolve(); }
    function observeSelected() { submissionController?.observe(); }
    async function rereadSelected() {
        localError = '';
        // A different browser may only observe the Run. Refresh its canonical
        // history too, so retrying control observation cannot leave a stale reply.
        await loadHistory('replace');
        observeSelected();
    }
    async function send() {
        if (composerUnavailable) return;
        if (!occurrences.length && executeCommand()) return;
        if (attachmentsReading || knowledgeSavePending || busy || expertAdmissionPending || !active || (!draft.trim() && !attachments.length)) return;
        try { expertRetryIntent=loadExpertIntent(localStorage,{hostId,clientId},active.conversationId);expertIntentStorageError=''; }
        catch(cause){expertIntentStorageError=cause instanceof Error?cause.message:String(cause);return;}
        if (expertRetryIntent) { expertError = t($locale, 'chatPage.expertRetryPendingFirst'); return; }
        let selectedReferences:ReturnType<typeof mentionTargets>;
        try { selectedReferences=mentionTargets(occurrences); }
        catch(cause){expertError=cause instanceof Error?cause.message:String(cause);return;}
        const inline=selectedReferences.length?mentionSegments(draft,occurrences):undefined;
        if(inline && !attachments.length && !inline.segments.some(s=>!s.agent_id&&s.text.trim())){
            expertError=t($locale,'rooms.mentionTask');composer?.focus();return;
        }
        if(selectedReferences.length){
            const current=expertScope(),original=draft,originalMentions=JSON.stringify(occurrences),attachmentIds=attachments.map(f=>f.id).join(','),epoch=selectedExpertEpoch();
            if(!epoch){expertError=t($locale,'rooms.expertCatalogUnavailable');return;}
            expertAdmissionPending=true;
            try{await expertSourceCommand<AgentDirectory>({type:'directory'},epoch);}
            catch(cause){if(current())expertError=t($locale,cause instanceof ExpertRoleCatalogUnavailable?'rooms.expertCatalogUnavailable':'rooms.expertChanged');return;}
            finally{if(current())expertAdmissionPending=false;}
            if(!current()||draft!==original||JSON.stringify(occurrences)!==originalMentions||attachments.map(f=>f.id).join(',')!==attachmentIds)return;
        }
        localError = '';
        if ((!active.executionBackend || active.executionBackend === 'nola') && !active.selectedModelId) {
            localError = 'chatSession.invalidModel';
            inspectorTab = 'settings';
            void sessionSettings?.refreshModelCatalog();
            return;
        }
        const submissionId = randomId();
        const text = draft;
        void submissionController?.submit({text: draft, attachments, language: $locale, submissionId, selectedExperts:selectedReferences,expertSourceEpoch:selectedExpertEpoch()??undefined,expertMentions:inline});
    }
    // Historical expert-only intents are reconciled unchanged; new inputs use Main.
    async function sendExpertOnly(conversationId: string, text: string, participantIds: string[]) {
        if (expertAdmissionPending || !workspaceHost || !active || active.conversationId !== conversationId || !canInteract) return;
        const sourceWorkspace = workspaceHost, sourceHost = hostId, sourceClient = clientId;
        const selection = draftSelection, sourceGeneration = generation, sourceController = submissionController;
        const current = () => !disposed && workspaceHost === sourceWorkspace && hostId === sourceHost && clientId === sourceClient
            && active?.conversationId === conversationId && draftSelection === selection && generation === sourceGeneration;
        const owner = {hostId:sourceHost,clientId:sourceClient};
        expertAdmissionPending = true; expertError = '';
        try {
            let input:AdmitExpertMessageInput|null;
            try{input=loadExpertIntent(localStorage,owner,conversationId);if(current())expertIntentStorageError='';}
            catch(cause){if(current())expertIntentStorageError=cause instanceof Error?cause.message:String(cause);throw cause;}
            if (!input) {
                const room = expertRoom, selected = new Set(participantIds);
                if (!text.trim() || !room || !selected.size || !timeline?.projection) throw new Error('Expert selection is unavailable');
                const members = selectedExpertMembers(room,[...selected]);
                const submissionId = randomId();
                input = {conversation_id:conversationId,expected_revision:timeline.projection.revision,
                    room_id:room.room.id,expected_room_revision:room.revision,submission_id:submissionId,user_message_id:`${submissionId}:user`,
                    text,selected_participants:members.map(member=>({participant_id:member.id,agent_id:member.agent_id,
                        agent_revision:member.agent_revision,context_conversation_id:member.context_conversation_id})),created_at_ms:Date.now()};
                try{saveExpertIntent(localStorage,owner,input);}
                catch(cause){if(current())expertIntentStorageError=cause instanceof Error?cause.message:String(cause);throw cause;}
                sourceWorkspace.captureExpertAdmission(conversationId, input.submission_id, input.text);
            }
            if (current()) expertRetryIntent = input;
            // One complete immutable intent enters Core. On uncertain retry the
            // original text, revisions and selected collection remain unchanged.
            const manifest = await admitExpertMessage(input, owner);
            if (!current()) return;
            const receipts:RoomSubmission[] = [];
            for (const entry of manifest.entries) {
                if (!current()) return;
                const receipt = await roomCommand<RoomSubmission|null>({type:'submission',room_id:manifest.input.room_id,submission_id:entry.child_submission_id});
                if (!receipt || receipt.input.submission_id !== entry.child_submission_id || receipt.input.room_id !== manifest.input.room_id
                    || receipt.input.conversation_id !== conversationId || receipt.input.parent_message_id !== manifest.input.user_message_id
                    || receipt.context_conversation_id !== entry.participant.context_conversation_id
                    || receipt.selected_participant_id !== entry.participant.participant_id) throw new Error('Complete expert receipt is unavailable');
                receipts.push(receipt);
            }
            // Reuse Core's exact durable receipt inputs to wake the existing Host
            // dispatcher. No client-generated child subset or replacement IDs.
            const deliveries = await Promise.allSettled(receipts.map(receipt => roomCommand<RoomSubmission>({type:'submit',input:receipt.input})));
            if (!current()) return;
            deliveries.forEach((result,index)=>{
                if (result.status === 'rejected') throw new Error(t($locale, 'chatPage.expertDeliveriesUnconfirmed'));
                if (result.value.input.submission_id !== receipts[index].input.submission_id
                    || result.value.context_conversation_id !== receipts[index].context_conversation_id) throw new Error('Expert dispatch owner changed');
            });
            await sourceController?.loadHistory('replace');
            if (!current()) return;
            if (!timeline?.messages.some(message => message.id === input.user_message_id && message.canonical?.role === 'user')) {
                throw new Error(t($locale, 'chatPage.expertHistoryConfirmPending'));
            }
            // Persist the revision-checked draft acknowledgement before deleting
            // the durable expert intent. An ABA edit remains a newer draft.
            await sourceWorkspace.acknowledgeExpertAdmission(conversationId, input.submission_id);
            if (!current()) return;
            draft = sourceWorkspace.text.read(conversationId);
            clearExpertIntent(localStorage, owner, input);
            expertRetryIntent = null;
            await refreshExpertSubmissions(conversationId, expertRoom);
        } catch (cause) {
            if (current()) expertError = cause instanceof Error ? cause.message : String(cause);
        } finally { if (current()) expertAdmissionPending = false; }
    }
    function retryCompleteExpertIntent() {
        if (expertRetryIntent && active?.conversationId === expertRetryIntent.conversation_id) {
            void sendExpertOnly(expertRetryIntent.conversation_id, expertRetryIntent.text,
                expertRetryIntent.selected_participants.map(member=>member.participant_id));
        }
    }
    function expertsForMessage(messageId: string) { return attributedExperts.filter(item => item.submission.input.parent_message_id === messageId); }
    function composerKeydown(event: KeyboardEvent) {
        if (composerUnavailable) { event.preventDefault(); return; }
        if (commandsVisible && !composing && !event.isComposing && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
            if (event.key === 'Escape') { event.preventDefault(); commandDismissed = draft; return; }
            if (['ArrowUp', 'ArrowDown', 'Enter', 'Tab'].includes(event.key)) {
                const count = commandCandidates.length;
                if (!count) {
                    if (event.key === 'Enter') {
                        event.preventDefault();
                        commandDismissed = draft;
                    }
                    return;
                }
                event.preventDefault();
                commandIndex = Math.min(commandIndex, count - 1);
                if (event.key === 'Enter' || event.key === 'Tab') completeCommand(commandCandidates[commandIndex].name);
                else { commandIndex = (commandIndex + (event.key === 'ArrowUp' ? -1 : 1) + count) % count;
                    document.getElementById(`composer-command-${commandIndex}`)?.scrollIntoView({block: 'nearest'}); }
                return;
            }
        }
        if (!composerEnterSubmits(event, composing)) return;
        event.preventDefault(); send();
    }
    function resume() { if (canInteract) return submissionController?.resume(); }
    function stop() { if (canInteract) return submissionController?.stop(); }
    function clearActiveConversationState() {
        submissionController?.detach(); submissionController = null;
        active = null; timeline = null; turn = null; contentTabs = emptyContentTabs();
        draft = ''; attachments = []; permissionState = null; reasoningState = null;
        modelState = null; selectedModelName = '';
        queuedDispatch = null; queuedCancelling = false;
        ++expertRequest; ++expertHistoryRequest;
        expertRoom = null; expertSourceEpoch = null; expertDirectory = null; expertRejoin = null;
        occurrences = []; expertSubmissions = []; expertRetryIntent = null; expertIntentStorageError = '';
        expertAdmissionPending = false; expertError = ''; expertResumeMessage = null;
        expertsLoading = false; managingExperts = false;
        historicalRecoveryOwners = []; uiSurfaces = []; uiSurfaceFailure = '';
        challenge = null; reviewText = ''; pendingText = ''; pendingAttachmentNames = [];
        selectedMessageId = null; contextUsage = null; conversationCacheReader.select(null);
        inspectorTab = null; controllerError = ''; localError = ''; notice = '';
    }
    async function initialize() {
        if (disposed || initializationInFlight) return;
        initializationInFlight = true; initializing = true; initializationFailed = false; localError = '';
        const requestedContent = new URL(location.href).searchParams.get('content');
        try {
            const session = parseInitializationSession(await api('/api/web/session'));
            // Recheck after notification navigation: the authenticated Host may
            // change between the click's session read and mounting this page.
            const sourceHost = new URL(location.href).searchParams.get('sourceHost');
            if (sourceHost !== null && sourceHost !== session.hostId) throw new Error('notification_host_changed');
            if (disposed) return;
            if (hostId && hostId !== session.hostId) {
                captureConversationReading();
                clearActiveConversationState();
            }
            hostId = session.hostId; clientId = await ensureBrowserClientId(hostId);
            workspaceHost = workspaceDrafts.forHost(hostId);
            drafts = workspaceHost.text;
            const list = parseChatConversations(await api('/api/chat/conversations'));
            if (disposed) return;
            const requested = new URL(location.href).searchParams.get('conversation');
            const remembered = workspaceHost.lastConversationId;
            const selected = list.find(item => item.conversationId === (requested ?? remembered)) ?? (requested ? parseChatConversation(await api(`/api/chat/conversations/${encodeURIComponent(requested)}`)) : list[0]);
            if (selected && !disposed) await selectConversation(selected);
            const continuation=new URL(location.href).searchParams;
            if(selected&&!disposed&&continuation.has('expert_context')){
                const child={conversation_id:continuation.get('expert_context'),participant_id:continuation.get('expert_participant'),
                    agent_id:continuation.get('expert_agent'),agent_revision:Number(continuation.get('expert_revision')),is_current:true,active:true} as ExpertChildConversation;
                try {
                    const sourceRead=await roomCommandAtSource<{room:RoomSnapshot}|null>({type:'for_conversation',conversation_id:selected.conversationId},continuation.get('expert_source_epoch'));
                    const association=sourceRead.result;
                    if(!disposed&&active?.conversationId===selected.conversationId&&session.hostId===hostId){
                        const member=association&&currentExpertChildMember(association.room,child);
                        if(!member)throw new Error('Expert context changed');
                        expertRoom=association.room;expertSourceEpoch=sourceRead.sourceEpoch;
                        expertDirectory=await expertSourceCommand<AgentDirectory>({type:'directory'},sourceRead.sourceEpoch);
                        if(disposed||active?.conversationId!==selected.conversationId)return;
                        const agent=expertDirectory.agents.find(a=>a.id===member.agent_id);
                        if(agent)insertExpert(agent);
                    }
                } catch { if(!disposed&&active?.conversationId===selected.conversationId)notice=t($locale,'rooms.childChanged'); }
                if(!disposed){const url=new URL(location.href);for(const key of ['expert_context','expert_participant','expert_agent','expert_revision','expert_source_epoch'])url.searchParams.delete(key);history.replaceState(history.state,'',url);}
            }
            if (requestedContent && !disposed) openManagementTab(requestedContent);
        } catch (cause) { if (!disposed) {
            localError = chatInitializationError(cause); initializationFailed = true;
            if (requestedContent && contentTabs.selected === conversationTabKey) openManagementTab(requestedContent);
        } }
        finally { initializationInFlight = false; if (!disposed) initializing = false; }
    }
    onMount(() => {
        const motionPreference = window.matchMedia('(prefers-reduced-motion: reduce)');
        const updateMotionPreference = () => reducedMotion = motionPreference.matches;
        updateMotionPreference();
        motionPreference.addEventListener('change', updateMotionPreference);
        const contentNavigation = (event: Event) => {
            if (event instanceof CustomEvent && typeof event.detail === 'string' && openManagementTab(event.detail)) event.preventDefault();
        };
        function restorePluginNavigation(event: PopStateEvent) {
            const address = new URL(window.location.href);
            const leavingPlugin = selectedContentTab?.kind === 'page' && selectedContentTab.path.startsWith('/plugin-page?');
            if (address.pathname !== '/chat' || (!event.state?.nolaPluginDestination && !leavingPlugin)) return;
            if (address.searchParams.get('conversation') !== (active?.conversationId ?? null)) return;
            const next = address.searchParams.get('content');
            if (next) openManagementTab(next);
            else updateContentTabs(selectContentTab(contentTabs, conversationTabKey));
        }
        window.addEventListener('popstate', restorePluginNavigation);
        window.addEventListener(workspaceContentNavigationEvent, contentNavigation);
        const visibility = () => {
            submissionController?.setForeground(document.visibilityState !== 'hidden');
            if (document.visibilityState !== 'hidden') conversationCacheReader.refresh();
        };
        document.addEventListener('visibilitychange', visibility);
        void initialize();
        expertPoll = setInterval(() => { if (document.visibilityState !== 'hidden' && expertRoom && (expertExecutionPending || busy)) void refreshExpertSubmissions(); }, 2500);
        return () => {
            captureConversationReading();
            disposed = true; draftSelection++; submissionController?.detach(); conversationCacheReader.dispose();
            document.removeEventListener('visibilitychange', visibility);
            window.removeEventListener(workspaceContentNavigationEvent, contentNavigation);
            window.removeEventListener('popstate', restorePluginNavigation);
            motionPreference.removeEventListener('change', updateMotionPreference);
            if (userScrollTimer) clearTimeout(userScrollTimer);
            if (followFrame !== null) cancelAnimationFrame(followFrame);
            if (expertPoll) clearInterval(expertPoll);
        };
    });

    function changed(event: { type: string; conversationId: string; conversation?: ChatConversation }) {
        if (event.type === 'deleted') {
            workspaceDrafts.forgetConversation(event.conversationId);
            workspaceHost?.submissions.delete(event.conversationId);
            if (workspaceHost?.lastConversationId === event.conversationId) workspaceHost.lastConversationId = null;
        }
        if (event.type === 'deleted' && active?.conversationId === event.conversationId) { clearActiveConversationState(); history.replaceState(history.state, '', '/chat'); }
        else if (event.type === 'truncated' && active?.conversationId === event.conversationId) void loadHistory('replace');
        else if ((event.type === 'renamed' || event.type === 'refreshed') && event.conversation && active?.conversationId === event.conversationId) active = event.conversation;
    }
    async function preparedTargetController(envelope: import('../lib/editedResend').EditedResendEnvelope,
        sourceWorkspace: NonNullable<typeof workspaceHost>, current:()=>boolean) {
        const target=parseChatConversation(await api(`/api/chat/conversations/${encodeURIComponent(envelope.target.id)}`));
        if(!current()||target.conversationId!==envelope.target.id||target.configurationRevision!==envelope.target.configuration_revision)throw new Error('Edited resend target changed');
        let controller=sourceWorkspace.controllers.get(target.conversationId);
        if(!controller){
            controller=createConversationSubmissionController({conversation:target,hostId:envelope.host_id,clientId:envelope.client_id,workspace:sourceWorkspace});
            sourceWorkspace.controllers.set(target.conversationId,controller);
        } else {
            if(controller.owner.hostId!==envelope.host_id||controller.owner.clientId!==envelope.client_id)throw new Error('Edited child controller owner changed');
            controller.updateConversation(target);
        }
        return controller;
    }
    async function resumeCompleteExpertMessage(message: import('../lib/chatTimeline').TimelineMessage) {
        if(!active||!workspaceHost||!canInteract||expertResumeMessage!==null||expertAdmissionPending)return;
        const source=editedResendSourceForMessage(message,{hostId,clientId,conversationId:active.conversationId});
        if(!source)return;
        const scope=expertScope(),sourceWorkspace=workspaceHost,current=()=>scope()&&canInteract;
        expertResumeMessage=message.id;expertError='';
        try{
            const original=await loadExpertManifest(source);if(!current())return;
            if(!original)throw new Error('Complete expert source is unavailable');
            const options={language:$locale,isCurrent:current,controller:(envelope:import('../lib/editedResend').EditedResendEnvelope)=>preparedTargetController(envelope,sourceWorkspace,current)};
            if(await resumePreparedExpertMessage(original,source,options)){
                if(current())await refreshExpertSubmissions();return;
            }
            const plan=await admitExpertMessage(original.input,source);if(!current())return;
            if(plan.entries.length!==original.entries.length||plan.entries.some((entry,i)=>entry.child_submission_id!==original.entries[i].child_submission_id))throw new Error('Original expert collection changed');
            const receipts:RoomSubmission[]=[];
            for(const entry of plan.entries){
                const receipt=await roomCommand<RoomSubmission|null>({type:'submission',room_id:plan.input.room_id,submission_id:entry.child_submission_id});
                if(!current())return;
                if(!receipt||receipt.input.parent_message_id!==plan.input.user_message_id||receipt.input.conversation_id!==plan.input.conversation_id
                    ||receipt.input.submission_id!==entry.child_submission_id||receipt.context_conversation_id!==entry.participant.context_conversation_id
                    ||receipt.selected_participant_id!==entry.participant.participant_id)throw new Error('Complete expert receipt is unavailable');
                receipts.push(receipt);
            }
            // An existing actual child Run is read, never submitted again.
            for(const receipt of receipts){
                if(!current())return;
                if(!['selected','executing'].includes(receipt.state))continue;
                const childOwner={hostId:source.hostId,clientId:`room:${plan.input.room_id}`,conversationId:receipt.context_conversation_id!};
                const observation=await api('/api/chat/agent/v2/observe',{method:'POST',body:JSON.stringify({operation:'discover',
                    expected_host_id:childOwner.hostId,client_id:childOwner.clientId,conversation_id:childOwner.conversationId})});
                const actual=parseRunObservation(observation,childOwner);if(!current())return;
                if(actual){if(actual.run_id!==receipt.input.submission_id||actual.assistant_message_id!==`${receipt.input.submission_id}:assistant`)throw new Error('Expert execution owner changed');continue;}
                await roomCommand<RoomSubmission>({type:'submit',input:receipt.input});
            }
            if(current())await refreshExpertSubmissions();
        }catch(cause){if(current())expertError=cause instanceof Error?cause.message:String(cause);}
        finally {if(scope())expertResumeMessage=null;}
    }
    async function editAndResend(edited: EditedResendDraft) {
        const sourceWorkspace = workspaceHost, selected = draftSelection, sourceGeneration = generation;
        const current = () => !disposed && workspaceHost === sourceWorkspace && hostId === edited.source.hostId
            && clientId === edited.source.clientId && active?.conversationId === edited.source.conversationId
            && generation === sourceGeneration && draftSelection === selected;
        if (!sourceWorkspace || !current() || busy || expertAdmissionPending || expertExecutionPending || conversationWriting || !canInteract) throw new Error(t($locale, 'chatPage.sendFailed'));
        const envelope = await prepareInPlaceResend(edited);
        if(!current())throw new Error('Edit owner changed before resend');
        const p=envelope.prepared;
        let completion:InPlaceResendCompletion;
        if(p.experts.length){
            const association=await roomCommand<{room_id:string;room:RoomSnapshot}|null>({type:'for_conversation',conversation_id:p.input.conversation_id});
            if(!association||!current())throw new Error('Expert room is unavailable');
            const manifest=await admitExpertMessage({conversation_id:p.input.conversation_id,expected_revision:envelope.target.revision,
                room_id:association.room_id,expected_room_revision:association.room.revision,submission_id:p.run_id,
                user_message_id:p.user_message_id,text:p.input.edited_text,selected_participants:p.experts,created_at_ms:p.input.created_at_ms},edited.source);
            for(const entry of manifest.entries){
                if(!current())throw new Error('Expert edit owner changed');
                const receipt=await roomCommand<RoomSubmission|null>({type:'submission',room_id:manifest.input.room_id,submission_id:entry.child_submission_id});
                if(!receipt||receipt.input.parent_message_id!==p.user_message_id||receipt.context_conversation_id!==entry.participant.context_conversation_id)
                    throw new Error('Expert receipt owner changed');
                await roomCommand<RoomSubmission>({type:'submit',input:receipt.input});
            }
            completion={inPlace:true,envelope};
            if(current())await refreshExpertSubmissions();
        }else{
            if(!submissionController)throw new Error('Conversation controller is unavailable');
            const receipt=await submissionController.submitInPlaceResend(envelope,$locale);
            completion={inPlace:true,envelope,receipt};
        }
        if(current()){
            await conversationManager?.refresh();
            // A local resend already rewinds the visible timeline at admission.
            // Replacing its history while its stream is active discards live journal cursors.
            if (current() && p.experts.length) await submissionController?.loadHistory('replace');
        }
        return completion;
    }

    function conversationSettingsChanged(conversation: ChatConversation) {
        // Updating session settings must not create a new timeline owner or
        // discard the visible transcript and composer draft.
        if (active?.conversationId === conversation.conversationId && !disposed) { active = conversation; submissionController?.updateConversation(conversation); }
    }
    function focusComposer() {
        void tick().then(() => {
            if (!disposed && contentTabs.selected === conversationTabKey && !document.querySelector('dialog:modal')) {
                (composer ?? document.getElementById('chat-composer'))?.focus();
            }
        });
    }
    function permissionAccepted(identity: string, choice?: import('../lib/chatPermission').PermissionChoice) {
        submissionController?.permissionAccepted(identity);
        if (choice === 'return_to_composer') focusComposer();
    }
</script>

<svelte:head>{#if !managementContentSelected}<title>{t($locale, 'chatPage.title')} — Nola</title>{/if}</svelte:head>
<svelte:window onkeydown={windowKeydown} onpointerup={transcriptPointerEnded} onpointercancel={transcriptPointerEnded} bind:innerWidth={viewportWidth} />
<ConfirmationDialog bind:this={messageConfirmation} />
{#if managingExperts}
    {#key hostId}<ExpertManager {hostId} onclose={() => {managingExperts = false; void openExperts(true);}} />{/key}
{/if}
{#snippet inspectorControls()}
<div class="inspector-controls">
            <button class="panel-toggle" title={t($locale, 'chatPage.find')} aria-label={t($locale, 'chatPage.find')} aria-pressed={searchVisible} onclick={openSearch}><svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="7" /><path d="m16 16 5 5" /></svg></button>
            {#each inspectorGroups as group}
                {@const selected = inspectorTab !== null && group.tabs.includes(inspectorTab)}
                <button class="panel-toggle" data-panel={group.label} title={t($locale, `chatInspector.${group.label}`)} aria-label={t($locale, `chatInspector.${group.label}`)} aria-controls="chat-details" aria-expanded={selected} aria-pressed={selected} onclick={() => toggleInspector(selected ? inspectorTab ?? group.tabs[0] : group.tabs[0])}>
                    <PrototypeIcon name={group.icon} />
                </button>
            {/each}
        </div>
{/snippet}
{#snippet inspectorTitle()}<strong>{inspectorLabel}</strong>{/snippet}
{#snippet expertPicker()}
<section class="expert-picker" aria-label={t($locale, 'rooms.experts')}>
        <div class="expert-picker-heading"><strong>{t($locale, 'rooms.experts')}</strong>
        <button type="button" disabled={!canInteract} onclick={() => {managingExperts = true;}}>{t($locale, 'rooms.manageExperts')}</button></div>
        {#if expertsLoading}<span role="status">{t($locale, 'rooms.loading')}</span>
        {:else if expertDirectory}
            {#each expertDirectory.agents.filter(agent => agent.availability === 'active') as agent (agent.id)}
                {@const member = expertRoom?.room.participants.find(person => person.agent_id === agent.id && person.active)}
                <div class="expert-choice"><label><input type="checkbox" checked={selectedExperts.includes(agent.id)} disabled={expertsLoading || !canInteract} onchange={() => void toggleExpert(agent.id)} /> <span>{agent.name}</span></label>
                    {#if member && expertRoom && active && member.agent_revision!==agent.revision}
                        <button type="button" onclick={()=>{expertRejoin={owner:active!.conversationId,room:expertRoom!,agent};void rejoinExpert();}}>{t($locale,'rooms.rejoinExpert')}</button>
                    {/if}</div>
            {:else}<span>{t($locale, 'rooms.noExperts')}</span>{/each}
        {/if}
        {#if expertRejoin}<button type="button" disabled={expertsLoading} onclick={rejoinExpert}>{t($locale,'rooms.rejoinExpert')}</button>{/if}
        {#if expertError}<p role="alert">{expertError}</p><button type="button" disabled={expertsLoading} onclick={() => void openExperts(true)}>{t($locale, 'shell.retry')}</button>{/if}
        {#if selectedExperts.length}<button type="button" onclick={convertToPlainText}>{t($locale, 'chatInspector.clearExperts')}</button>{/if}
    </section>
{/snippet}
{#if mentionProfile}
    <dialog class="mention-profile" use:showMentionProfile onclose={()=>mentionProfile=null} onclick={event => { if (event.target === event.currentTarget) mentionProfile = null; }}>
        <strong>{mentionProfile.name}</strong>{#if mentionProfile.member?.description}<p>{mentionProfile.member.description}</p>{/if}
        {#if active?.conversationId===mentionProfile.parent && hostId===mentionProfile.host && mentionProfile.ref.participant_id && mentionProfile.ref.context_conversation_id}
            <ExpertChildHistory parentId={mentionProfile.parent} reference={mentionProfile.ref} expectedEpoch={mentionProfile.epoch} expectedHost={mentionProfile.host}/>
        {/if}
        <button type="button" onclick={()=>mentionProfile=null}>{t($locale,'rooms.close')}</button>
    </dialog>
{/if}
<div class="chat-page" ontouchstart={beginEdgeSwipe} ontouchend={finishEdgeSwipe} ontouchcancel={() => edgeSwipe = undefined} role="presentation" style:--sidebar-width={`${widths.sidebar}px`}>
    <header class="chat-header">
        <div class="global-navigation" class:collapsed={viewportWidth > 760 && !navigationExpanded}><a class="desktop-mark" href="/today" aria-label="Nola">n</a><WorkspaceListNavigation category={listCategory} visible={navigationExpanded} onchange={setListCategory} /><button class="sidebar-toggle panel-toggle" title={t($locale, 'shell.toggleSidebar')} aria-label={t($locale, 'shell.toggleSidebar')} aria-expanded={navigationExpanded} onclick={() => navigationExpanded = !navigationExpanded}><PrototypeIcon name="sidebar" size={18} /></button></div>
        <button class="navigation-toggle" aria-label={t($locale, contentTabs.selected === conversationTabKey ? 'shell.workspace' : 'chatPage.title')} onclick={() => contentTabs.selected === conversationTabKey ? navigateWorkspace('/conversations') : updateContentTabs(selectContentTab(contentTabs, conversationTabKey))}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m15 5-7 7 7 7" /></svg></button>
        <h1 class="sr-only">{selectedContentTab ? selectedContentTab.kind === 'pdf' ? selectedContentTab.resource.displayName : pluginTitles[selectedContentTab.path] ?? t($locale, selectedContentTab.titleKey) : active?.title || t($locale, 'chatPage.title')}</h1>
        {#if viewportWidth <= 760}<div class="mobile-chat-heading"><strong>{selectedContentTab ? selectedContentTab.kind === 'pdf' ? selectedContentTab.resource.displayName : pluginTitles[selectedContentTab.path] ?? t($locale, selectedContentTab.titleKey) : active?.title || t($locale, 'chatPage.title')}</strong><small>Nola</small></div>{#if selectedContentTab}<button class="panel-toggle mobile-content-close" aria-label={t($locale, 'chatInspector.closeTab')} onclick={() => closeWorkspaceContentTab(contentTabs.selected)}><PrototypeIcon name="close" size={16} /></button>{:else}<button class="panel-toggle" aria-label={t($locale, 'chatInspector.context')} disabled={!active} onclick={() => toggleInspector('resources')}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="M6 3h8l4 4v14H6zM14 3v5h4M9 12h6m-6 4h6"/></svg></button><button class="panel-toggle" aria-label={t($locale, 'rooms.experts')} onclick={() => void openExperts()}><svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="7" r="4"/><path d="M5 22v-4a7 7 0 0 1 14 0v4"/></svg></button>{/if}{:else}<ContentTabStrip pageTitles={pluginTitles} ownerKey={contentWorkspaceKey} state={contentTabs} title={active?.title || t($locale, 'chatPage.title')}
            conversationId={active?.conversationId ?? null}
            canRename={!!active?.conversationId && canInteract && !conversationWriting && !busy && contentTabs.selected === conversationTabKey}
            onselect={key => updateContentTabs(selectContentTab(contentTabs, key))}
            onclose={closeWorkspaceContentTab}
            onrename={(id, nextTitle) => conversationManager?.renameConversationById(id, nextTitle)} />{/if}
        {#if active}{@render inspectorControls()}{/if}
        {#if active}<ToolbarPopover label={t($locale, 'chatInspector.more')}>
            <details class="attachment-help"><summary>{t($locale, 'chatTextAttachment.helpTitle')}</summary><p>{t($locale, 'chatTextAttachment.help')}</p><p>{t($locale, 'chatTextAttachment.dropPaste')}</p></details>
            <button type="button" onclick={openSearch}>{t($locale, 'chatPage.find')}</button>
            <button type="button" onclick={() => loadHistory('replace')} disabled={historyLoading || (busy && turn?.phase !== 'recovering' && turn?.phase !== 'cancel_requested')}>{t($locale, 'chatPage.refresh')}</button>
            {#if viewportWidth <= 760}
                {#each inspectorGroups as group}
                    {#if group.label !== 'context' || selectedContentTab}
                        <button type="button" onclick={() => toggleInspector(group.tabs[0])}>{t($locale, `chatInspector.${group.label}`)}</button>
                    {/if}
                {/each}
            {/if}
                <button type="button" onclick={() => toggleInspector('execution')}>{t($locale, 'chatInspector.execution')}</button>
                {#if canInteract}<button type="button" onclick={() => toggleInspector('settings')}>{t($locale, 'chatInspector.settings')}</button>{/if}
                {#if viewportWidth > 760 || selectedContentTab}<button type="button" aria-expanded={inspectorTab === 'participants'} onclick={() => void openExperts()}>{t($locale, 'rooms.experts')}{selectedExperts.length ? ` · ${selectedExperts.length}` : ''}</button>{/if}
        </ToolbarPopover>{/if}
    </header>
    <div class="chat-columns" class:without-navigation={viewportWidth > 760 && !navigationExpanded} class:without-conversation={!active || managementContentSelected || inspectorTab === null} style:--sidebar-width={`${widths.sidebar}px`} style:--inspector-width={`${widths.inspector}px`}>
        <ChatResponsivePanel bind:this={navigationPanel} breakpoint={760} side="start" expanded={viewportWidth > 760 ? navigationExpanded : null} label={t($locale, 'shell.workspace')} closeLabel={t($locale, 'shell.cancel')}>
        <div class="panel-column">
        <aside class="chat-navigation" id="chat-workspace-navigation">
            <div class="compact-list-categories"><WorkspaceListNavigation category={listCategory} onchange={setListCategory} /></div>
            <div class="sidebar-section" hidden={listCategory !== 'conversations'}>
            <ChatConversationManager navigationDisabled={initializing || initializationFailed} bind:this={conversationManager} bind:writing={conversationWriting} confirm={request => messageConfirmation.ask(request)} messageIds={messages.map(messageMutationId).filter((id): id is string => id !== null)} activeId={active?.conversationId ?? null} highlightSelection={!selectedContentTab} {generation} busy={busy || !!active && !canInteract} hasMessages={messages.length > 0} {canBranchDuringMaintenance} {selectedMessageId} onselect={openConversationFromList} onchange={changed} />
            </div>
            <div class="sidebar-section" hidden={listCategory !== 'resources'}><WorkspaceResourceNavigation onopen={openManagementTab}
                selectedPath={selectedContentTab?.kind === 'page' ? selectedContentTab.path : ''} /></div>
            {#if viewportWidth > 760 || 'NolaDesktop' in window}
                <DesktopPluginNavigation selectedPath={selectedContentTab?.kind === 'page' ? selectedContentTab.path : undefined} onpages={reconcilePluginPages} />
            {/if}
            <footer class="workspace-footer">
                <button class="workspace-identity" onclick={() => openManagementTab('/settings?section=general')}><span aria-hidden="true">n</span><span>{t($locale, 'shell.workspace')}<small>Nola</small></span></button>
                <button aria-label={t($locale, 'shell.toggleAppearance')} title={t($locale, 'shell.toggleAppearance')} onclick={toggleAppearance}><svg aria-hidden="true" viewBox="0 0 24 24">{#if $darkAppearance}<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.41 1.41m11.32 11.32 1.41 1.41M2 12h2m16 0h2M4.93 19.07l1.41-1.41m11.32-11.32 1.41-1.41"/>{:else}<path d="M20 14A8 8 0 0 1 10 4a8 8 0 1 0 10 10Z"/>{/if}</svg></button>
                <button aria-label={t($locale, 'shell.settings')} title={t($locale, 'shell.settings')} onclick={() => openManagementTab('/settings')}><svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2v3m0 14v3M2 12h3m14 0h3M5 5l2 2m10 10 2 2M5 19l2-2M17 7l2-2"/></svg></button>
            </footer>
        </aside>
        {#if viewportWidth > 760}<PanelResizeHandle controls="chat-workspace-navigation" label={t($locale, 'shell.resizeSidebar')} value={widths.sidebar} min={220} max={widths.sidebarMax} defaultValue={layoutDefaults.sidebar} onchange={value => setWorkspaceWidth('sidebar', value)} />{/if}
        </div>
        </ChatResponsivePanel>
        <section id="page-main" tabindex="-1" role="main" class="chat-main" aria-label={selectedContentTab ? selectedContentTab.kind === 'pdf' ? selectedContentTab.resource.displayName : pluginTitles[selectedContentTab.path] ?? t($locale, selectedContentTab.titleKey) : active?.title || t($locale, 'chatPage.title')}>
            <div class="conversation-panel" role="tabpanel" id={contentTabDomId('panel', contentWorkspaceKey, conversationTabKey)} aria-labelledby={contentTabDomId('tab', contentWorkspaceKey, conversationTabKey)} hidden={contentTabs.selected !== conversationTabKey}>
            {#if returnToActivity}<a class="activity-source-return" href={returnToActivity}><PrototypeIcon name="back" size={16} />{t($locale, 'activity.backToActivity')}</a>{/if}
            {#if searchVisible}<div class="chat-search"><label for="chat-search-input">{t($locale, 'chatPage.find')}</label><input id="chat-search-input" bind:this={searchInput} type="search" bind:value={searchQuery} autocomplete="off" /><span role="status">{searchQuery.trim() ? t($locale, 'chatPage.found', { count: matchingMessageIds.size }) : ''}</span><button type="button" onclick={closeSearch} aria-label={t($locale, 'chatPage.closeSearch')}><PrototypeIcon name="close" size={16} /></button></div>{/if}
            {#if queuedDispatch}
                <section class="notice queued-action" aria-live="polite">
                    <p>{t($locale, ['rejected', 'cancelled'].includes(queuedDispatch.phase) ? 'recovery.queuedNotExecuted' : 'recovery.queuedChecking')}</p>
                    {#if ['rejected', 'cancelled'].includes(queuedDispatch.phase)}
                        <button disabled={queuedCancelling} onclick={() => submissionController?.cancelQueued()}>{t($locale, 'recovery.cancelQueued')}</button>
                    {:else}
                        <button onclick={() => { localError = ''; void submissionController?.checkQueued().catch(() => { localError = 'chatPage.uncertain'; }); }}>{t($locale, 'recovery.title')}</button>
                    {/if}
                </section>
            {/if}
            {#if notice}<p class="notice" role="status">{t($locale, notice)}</p>{/if}
            {#if error === 'chatPage.controlUnconfirmed'}
                <p class="notice" role="status">{t($locale, error)}</p>
                <button class="initialization-retry" type="button" disabled={historyLoading} onclick={rereadSelected}>{t($locale, 'chatPage.retryInitialization')}</button>
            {:else if error}
                <p class="notice error" class:initialization-error={initializationFailed} role="alert">{t($locale, error)}</p>
            {/if}
            {#if initializationFailed}<button class="initialization-retry" type="button" onclick={() => void initialize()} disabled={initializing}>{t($locale, 'chatPage.retryInitialization')}</button>{/if}
            {#if active && workspaceHost?.persistenceErrors.has(active.conversationId)}<p class="notice error" role="alert">{t($locale, 'chatPage.draftStorageFailed')}</p>{/if}
            {#if uiSurfaceFailure}<p class="notice error" role="alert">{uiSurfaceFailure}</p>{/if}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex (The named scrollable log needs a keyboard reading target.) -->
            <div class="transcript-shell">
            <div class="transcript" bind:this={transcript} onwheel={userScrollStarted} ontouchstart={transcriptPointerStarted} ontouchmove={userScrollStarted} onpointerdown={transcriptPointerStarted} onscroll={observedScroll} tabindex="0" aria-label={t($locale, 'chatPage.title')} role="log" aria-live="polite" aria-relevant="additions text">
                {#if timeline?.window?.hasMore}<button class="older" onclick={() => loadHistory('prepend')} disabled={historyLoading || busy}>{t($locale, 'chatPage.history')}</button>{/if}
                {#if !messages.length && (initializing || historyLoading)}
                    <p class="placeholder">{t($locale, 'chatPage.loading')}</p>
                {:else if !active && !initializationFailed}
                    <div class="empty-conversation">
                        <div class="empty-mark" aria-hidden="true">n</div>
                        <h2>{t($locale, 'chatPage.choose')}</h2>
                        <button class="empty-navigation" onclick={() => navigationPanel.openPanel()}>{t($locale, 'shell.workspace')}</button>
                    </div>
                {:else if active && !messages.length && !busy && !error}
                    <div class="empty-conversation">
                        <div class="empty-mark" aria-hidden="true">n</div>
                        <h2>{t($locale, 'chatPage.welcome')}</h2>
                    </div>
                {/if}
                {#each messages as message (JSON.stringify([hostId, active?.conversationId, message.id]))}
                    {@const attachmentDisplay = message.role === 'user' ? displayTextAttachmentMessage(timelineMessageText(message)) : null}
                    {@const visibleUserText=attachmentDisplay?.displayText??timelineMessageText(message)}
                    {@const messageMentions=message.role==='user'&&message.parts.length===1?partMentions(message.parts[0].value,visibleUserText,message.parts[0].source==='optimistic'):[]}

                    {#if (pendingText || pendingAttachmentNames.length) && busy && !messages.some(item => item.id === turn?.identity.userMessageId) && message.id === turn?.identity.assistantMessageId}<article class="user pending"><div class="message-content">{#if pendingText}<Markdown source={pendingText} />{/if}{#if pendingAttachmentNames.length}<ul class="attached-files">{#each pendingAttachmentNames as name}<li>{name}</li>{/each}</ul>{/if}<small>{t($locale, 'chatPage.pendingUser')}</small></div></article>{/if}
                    <article class:user={message.role === 'user'} class:selected={selectedMessageId === message.id} class:search-match={matchingMessageIds.has(message.id)} data-message-id={message.id}
                        in:fly={enteringMessageIds.has(message.id) && !reducedMotion ? { y: 8, duration: 180, easing: cubicOut } : { y: 0, duration: 0 }}
                        onintroend={() => { if (enteringMessageIds.has(message.id)) { const next = new Set(enteringMessageIds); next.delete(message.id); enteringMessageIds = next; } }}>
                        {#if message.role === 'assistant'}<div class="response-identity"><span class="response-avatar" aria-hidden="true">n</span><strong>Nola</strong></div>{/if}
                        <div class="message-column"><div class="message-content">
                            {#if message.role === 'assistant' && turn && busy && message.id === turn.identity.assistantMessageId}
                                {#if runStatusKey !== 'chatPage.working' || !message.parts.some(isExecutionPart) && !timelineMessageText(message)}
                                    <p class="run-status live" role="status">{t($locale, runStatusKey)}</p>
                                {/if}
                            {:else if message.role === 'assistant' && message.state === 'cancelled'}
                                <p class="run-status" role="status">{t($locale, 'activity.status.cancelled')}</p>
                            {/if}
                            {#if messageMentions.length}
                                <p class="mentioned-message">{#each mentionSegments(visibleUserText,messageMentions).segments as part}{#if part.agent_id}<button class="inline-name" type="button" onclick={()=>{
                                    const ref=messageMentions.find(m=>m.selection.agent_id===part.agent_id)!.selection;
                                    mentionProfile={name:part.text,parent:active!.conversationId,host:message.canonicalSnapshot?.sourceHostId??hostId,epoch:expertSourceEpoch,ref,member:expertRoom?.room.participants.find(m=>m.id===ref.participant_id&&m.context_conversation_id===ref.context_conversation_id)};
                                }}>{part.text}</button>{:else}{part.text}{/if}{/each}</p>
                            {/if}
                            {#if attachmentDisplay}{#if attachmentDisplay.displayText && !messageMentions.length}<Markdown source={attachmentDisplay.displayText} />{/if}<ul class="attached-files">{#each attachmentDisplay.files as file}<li>{file.name}{file.clipped ? ` · ${t($locale, 'chatTextAttachment.clipped')}` : ''}</li>{/each}</ul>
                            {/if}
                            <ChatMessageContent {message} ownerKey={JSON.stringify([hostId,active?.conversationId,generation])} readingActive={contentTabs.selected===conversationTabKey} skipText={!!attachmentDisplay || !!messageMentions.length} taskClientId={clientId} onTaskRecovered={() => { void loadHistory('replace'); observeSelected(); }} taskOwner={canInteract && active && turn?.runId ? {conversationId: active.conversationId, runId: turn.runId, hostId} : undefined}>
                                {#snippet surface(part)}
                                    {@const reference = uiSurfaceReference(part.value, active?.conversationId ?? '', message.id)}
                                    {#if reference}
                                        <ChatSurfaceReference readOnly={!canInteract || part.value.kind !== 'ui_surface'} {reference} recoverable={part.value.kind === 'ui_surface'} summary={typeof part.value.summary === 'string' ? part.value.summary : ''}
                                            snapshot={part.value.kind === 'ui_surface' ? uiSurfaces.find(value => uiIdentityKey(value.identity) === uiIdentityKey(reference.identity)) : undefined}
                                            oninteraction={submitUiInteraction}
                                            onrefresh={async () => { const selected = owner(); if (selected) await refreshUiSurfaces(selected); }} />
                                    {/if}
                                {/snippet}
                            </ChatMessageContent>
                        </div>
                            <ChatMessageActions mentionDirectory={expertDirectory?.agents??[]} mentionMembers={expertRoom?.room.participants??[]} loadMentions={loadMentionDirectory} mentionEpoch={()=>expertSourceEpoch} {message} ownerKey={JSON.stringify([hostId, active?.conversationId, generation])}
                                editSource={active ? editedResendSourceForMessage(message, {hostId, clientId, conversationId: active.conversationId}) ?? undefined : undefined}
                                oneditresend={canInteract ? editAndResend : undefined} ownerCurrent={() => !!active && canInteract && !disposed}
                                busy={busy || expertAdmissionPending || expertExecutionPending || conversationWriting || !canInteract} selected={selectedMessageId === message.id} onselect={() => selectedMessageId = selectedMessageId === message.id ? null : message.id} onoperation={canInteract ? (operation, messageId) => conversationManager.mutateMessage(operation, messageId, {conversationId: active?.conversationId ?? null, generation}) : undefined} />
                        </div>
                    </article>
                    {#if message.role === 'user'}
                        {@const messageExperts = expertsForMessage(message.id)}
                        {#each messageExperts as item, index (item.submission.input.submission_id)}
                                <article class="expert-message" data-expert-submission={item.submission.input.submission_id}>
                                    {#if item.submission.context_conversation_id && item.submission.execution_id}
                                        <ExpertConversationReply active={contentTabs.selected===conversationTabKey} {hostId}
                                            agentId={item.participant.agent_id} name={item.participant.display_name}
                                            status={t($locale, `rooms.${item.submission.state}`)}
                                            roomId={item.submission.input.room_id} contextId={item.submission.context_conversation_id}
                                            executionId={item.submission.execution_id} running={item.submission.state === 'executing'} />
                                    {:else}
                                        <ExpertReplyFrame {hostId} agentId={item.participant.agent_id} name={item.participant.display_name}>
                                            <p class="run-status" class:live={item.submission.state === 'selected' || item.submission.state === 'accepted' || item.submission.state === 'executing'} role="status">{t($locale, item.submission.state === 'selected' || item.submission.state === 'accepted' ? 'rooms.expertStarting' : `rooms.${item.submission.state}`)}</p>
                                        </ExpertReplyFrame>
                                    {/if}
                                </article>
                        {/each}
                        {#if messageExperts.some(item=>['selected','executing'].includes(item.submission.state))}
                            <button type="button" class="expert-retry-button" disabled={expertResumeMessage!==null||expertAdmissionPending||!canInteract}
                                onclick={()=>void resumeCompleteExpertMessage(message)}>{editedResendLabel($locale,'resumeExperts')}</button>
                        {/if}
                    {/if}
                    {#each replyNotices.filter(item => item.assistantMessageId === message.id || item.userMessageId === message.id && !messages.some(candidate => candidate.id === item.assistantMessageId)) as reply (reply.submissionId)}
                        <article class="reply-notice" data-reply-submission={reply.submissionId}><div class="message-content"><ChatReplyError kind={reply.kind} detail={reply.detail ?? ''} oncontinue={continueConversation} continuing={conversationWriting} /></div></article>
                    {/each}
                {/each}
                {#each replyNotices.filter(item => !messages.some(message => message.id === item.userMessageId || message.id === item.assistantMessageId)) as reply (reply.submissionId)}
                    <article class="reply-notice" data-reply-submission={reply.submissionId}><div class="message-content"><ChatReplyError kind={reply.kind} detail={reply.detail ?? ''} oncontinue={continueConversation} continuing={conversationWriting} /></div></article>
                {/each}
                {#if (pendingText || pendingAttachmentNames.length) && busy && !messages.some(message => message.id === turn?.identity.userMessageId || message.id === turn?.identity.assistantMessageId)}<article class="user pending"><div class="message-content">{#if pendingText}<Markdown source={pendingText} />{/if}{#if pendingAttachmentNames.length}<ul class="attached-files">{#each pendingAttachmentNames as name}<li>{name}</li>{/each}</ul>{/if}<small>{t($locale, 'chatPage.pendingUser')}</small></div></article>{/if}
                {#if turn && busy && !messages.some(message => message.id === turn?.identity.assistantMessageId)}<p class="run-status live" role="status">{t($locale, runStatusKey)}</p>{/if}
                {#if active && canInteract}{#key `${generation}:${active.conversationId}`}
                    <ChatUserQuestions conversationId={active.conversationId} {busy} onresolved={observeSelected} />
                {/key}{/if}
            </div>
            {#if showLatest}<button class="latest" transition:fly={{ y: 6, duration: reducedMotion ? 0 : 180 }} onclick={jumpToLatest}>{t($locale, 'chatPage.latest')}</button>{/if}
            </div>
            {#if canInteract && challenge && turn?.permission}<ChatPermissionPanel {challenge} runId={turn.runId} active={busy} {reviewText} onAccepted={permissionAccepted} onRefresh={observeSelected} />{/if}
            {#if canInteract && challenge && turn?.permission && !reviewText}<p class="muted">{t($locale, 'chatPage.noReview')}</p>{/if}

            {#if canInteract && turn?.runId}
                {#key `${generation}:${turn.runId}:${turn.identity.assistantMessageId}`}
                    <ChatUnknownEffectRecovery available={turn.phase === 'recovering' && observedRecovery === 'resume_required'} owner={{ expected_host_id: turn.owner.hostId, client_id: turn.owner.clientId,
                        conversation_id: turn.owner.conversationId, run_id: turn.runId, assistant_message_id: turn.identity.assistantMessageId }}
                        onResume={() => { void resume(); }} onObserved={observeSelected} />
                {/key}
            {/if}
            {#each historicalRecoveryOwners.filter(owner => canInteract && owner.run_id !== turn?.runId) as recoveryOwner (recoveryOwner.run_id)}
                {#key `${generation}:${recoveryOwner.run_id}`}
                    <ChatUnknownEffectRecovery owner={recoveryOwner} onResume={observeSelected}
                        onObserved={() => { void loadHistory('replace'); observeSelected(); }} />
                {/key}
            {/each}
            {#if active && canInteract}
<form class="composer" onsubmit={event => { event.preventDefault(); send(); }}
                ondragover={event => { if (event.dataTransfer?.types.includes('Files')) { event.preventDefault(); event.dataTransfer.dropEffect = busy || attachmentsReading ? 'none' : 'copy'; } }}
                ondrop={event => { if (event.dataTransfer?.types.includes('Files')) { event.preventDefault(); void textAttachments?.addFiles([...event.dataTransfer.files]); } }}
                onpaste={event => { if (event.clipboardData?.files.length) { event.preventDefault(); void textAttachments?.addFiles([...event.clipboardData.files]); } }}>
                <label class="sr-only" for="chat-composer">{t($locale, 'chatPage.placeholder')}</label>
                {#if commandError}<p role="alert" class="error">{commandError}</p>{/if}
                {#if commandsVisible}
                    <div class="expert-mentions" id="composer-commands" role="listbox" aria-label={t($locale, 'commands.help')}
                        in:fly={{ y: 4, duration: reducedMotion ? 0 : 100 }} out:fade={{ duration: reducedMotion ? 0 : 100 }}>
                        {#each commandCandidates as command, index (command.name)}
                            <button type="button" role="option" tabindex="-1" id={`composer-command-${index}`} aria-selected={index === commandIndex}
                                onpointerdown={event => event.preventDefault()} onclick={() => completeCommand(command.name)}>
                                <span class="mention-copy"><strong>/{command.name}</strong><small>{t($locale, command.label)}</small></span>
                            </button>
                        {:else}<p>{t($locale, 'commands.invalid')}</p>{/each}
                    </div>
                {/if}
                {#key active.conversationId}
                    <InlineExpertInput id="chat-composer" bind:field={composer} value={draft} mentions={occurrences} directory={expertDirectory?.agents??[]} members={expertRoom?.room.participants??[]}
                        onedit={editComposer} onload={loadMentionDirectory} onfocus={()=>mentionFocused=true} onblur={()=>mentionFocused=false} onkeydown={composerKeydown}
                        commandList={commandsVisible?'composer-commands':undefined} commandOption={commandsVisible&&commandCandidates.length?'composer-command-'+commandIndex:undefined}
                        placeholder={t($locale,'chatPage.placeholder')} disabled={composerUnavailable}/>
                {/key}
                <div class="composer-footer">
                {#if active}<div class="composer-attachments"><ChatTextAttachments showHelp={false} bind:this={textAttachments} draftStore={workspaceHost?.attachments} conversationId={active.conversationId} {generation} busy={busy || composerUnavailable} onattachments={(files, reading) => { attachments = [...files]; attachmentsReading = reading; }} /></div>{/if}
                <div class="composer-execution">
                    <ChatModelMenu selection={modelState} onopen={() => sessionSettings?.refreshModelCatalog()} onselect={choice => sessionSettings?.selectModel(choice)} onsettings={() => { inspectorTab = 'settings'; }} onreload={() => sessionSettings?.reloadSettings()} />
                    <ChatReasoningMenu state={reasoningState} {busy} onselect={value => sessionSettings?.selectReasoningPreference(value)} />
                    <div class="composer-actions">
                        {#if busy && turn && !draft.startsWith('/')}<button type="button" in:fade={{ duration: reducedMotion ? 0 : 100 }} onclick={stop} disabled={turn.phase === 'cancel_requested'}>{t($locale, turn.phase === 'cancel_requested' ? 'chatPage.cancelPending' : 'chatPage.stop')}</button>{:else}
                        <button class="primary" in:fade={{ duration: reducedMotion ? 0 : 100 }} aria-label={t($locale, 'chatPage.send')} type="submit" disabled={composerUnavailable || (!draft.startsWith('/') && (busy || expertAdmissionPending || attachmentsReading || knowledgeSavePending || (!draft.trim() && !attachments.length)))}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 19V5m-6 6 6-6 6 6"/></svg></button>{/if}
                    </div>
                </div>
                </div>
                {#if expertIntentStorageError}<p role="alert">{expertIntentStorageError}</p>{/if}
                {#if expertRetryIntent && expertRetryIntent.conversation_id === active.conversationId}
                    <aside class="reply-notice"><p role="status">{expertError || t($locale, 'chatPage.expertDeliveryPendingNotice')}</p><p>{expertRetryIntent.text}</p>
                        <button type="button" disabled={expertAdmissionPending || !canInteract} onclick={retryCompleteExpertIntent}>{t($locale,'shell.retry')}</button>
                    </aside>
                {:else if expertError}<p role="alert">{expertError}</p>{#if occurrences.length}<button type="button" onclick={convertToPlainText}>{t($locale,'rooms.plainMentions')}</button>{/if}{#if expertRejoin}<button type="button" disabled={expertsLoading} onclick={rejoinExpert}>{t($locale,'rooms.rejoinExpert')}</button>{/if}{/if}
                {#if knowledgeSavePending}<p role="status" class="muted">{t($locale, 'chatKnowledge.beforeSend')}</p>{/if}
            </form>{:else if canBranchDuringMaintenance}
                <aside class="reply-notice"><p role="status">{t($locale, 'chatConversations.compactionCopyHint')}</p>
                    <button type="button" disabled={conversationWriting || !messages.length} onclick={() => conversationManager?.duplicateConversation()}>{t($locale,'chatConversations.branchWhole')}</button>
                    <button type="button" disabled={historyLoading || conversationWriting} onclick={() => loadHistory('replace')}>{t($locale,'chatPage.refresh')}</button>
                </aside>
            {:else if active?.expertContextOwner}
                <aside class="task-readonly"><p>{t($locale,'rooms.childReadOnly')}</p>{#if active.expertParentConversationId}<a href={`/chat?conversation=${encodeURIComponent(active.expertParentConversationId)}`}>{t($locale,'chatPage.taskReturnParent')}</a>{/if}</aside>
            {:else if active && executionOwnership.kind === 'task_child'}
                <aside class="task-readonly"><p>{t($locale, 'chatPage.taskReadOnly')}</p><a href={`/chat?conversation=${encodeURIComponent(executionOwnership.parent_conversation_id)}`}>{t($locale, 'chatPage.taskReturnParent')}</a></aside>
            {:else if active && executionOwnership.kind === 'unavailable'}<p role="status">{t($locale, 'chatPage.taskOwnershipUnavailable')}</p>{/if}

            </div>
            {#each retainedContentTabs as entry (entry.ownerKey)}
                {@const tab = entry.tab}
                {@const isCurrent = entry.conversationId === (active?.conversationId ?? null) && contentTabs.selected === tab.key}
                <div class="content-document-panel" role="tabpanel" id={contentTabDomId('panel', entry.workspaceKey, tab.key)}
                    aria-labelledby={isCurrent ? contentTabDomId('tab', entry.workspaceKey, tab.key) : undefined}
                    aria-label={!isCurrent ? tab.kind === 'pdf' ? tab.resource.displayName : (pluginTitles[tab.path] ?? t($locale, tab.titleKey)) : undefined}
                    hidden={!isCurrent} inert={!isCurrent}>
                    {#if tab.kind === 'pdf'}<ConversationPdfTab resource={tab.resource} />
                    {:else}<WorkspacePageTab viewport={tab.viewport} ownerKey={entry.ownerKey} path={tab.path} active={isCurrent}
                        onlocation={path => updateStoredPageLocation(entry.conversationId, entry.hostId, tab.key, path)} />{/if}
                </div>
            {/each}
        </section>
        {#if active}<ChatResponsivePanel bind:this={inspectorPanel} breakpoint={1250} toolbar={inspectorTitle} expanded={!managementContentSelected && inspectorTab !== null} onclose={() => inspectorTab = null} label={inspectorLabel} closeLabel={t($locale, 'chatInspector.close')}>
        <div class="panel-column">
        <div bind:this={inspectorRoot} class="chat-side" id="chat-details">
            <div class="inspector-heading">{@render inspectorTitle()}<button title={t($locale, 'chatInspector.close')} onclick={() => inspectorPanel?.closePanel()} aria-label={t($locale, 'chatInspector.close')}><PrototypeIcon name="close" size={16} /></button></div>
            <details class="inspector-section" data-inspector-section="overview" hidden={!activeInspectorGroup?.tabs.includes('overview')} open={expandedInspectorSections.includes('overview')} ontoggle={event => setInspectorExpanded('overview', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.overview')}</summary><div class="inspector-content">
                <ChatDecisionOverview working={busy || expertAdmissionPending || expertSubmissions.some(item => item.state === 'executing')} needsResponse={turn?.phase === 'awaiting_permission'}
                    onconversation={() => { updateContentTabs(selectContentTab(contentTabs, conversationTabKey)); jumpToLatest(); }} />
            </div>
            </details>
            {#each inspectorActivitySections as section}
            <details class="inspector-section" data-inspector-section={section} hidden={!activeInspectorGroup?.tabs.includes(section)} open={expandedInspectorSections.includes(section)} ontoggle={event => setInspectorExpanded(section, event.currentTarget.open)}>
                <summary>{t($locale, `chatInspector.${section}`)}</summary><div class="inspector-content">
                {#if mountedInspectorSections.includes(section)}
                    <ChatActivityGoal readOnly={!canInteract} section={section} {hostId} conversationId={active.conversationId} draftStore={workspaceHost?.objectives} objectiveRevision={goalDraftRevision} {generation} {busy} oncontrol={observeSelected} refreshKey={turn?.committedRevision ?? 0} />
                {/if}
</div>
            </details>
            {/each}
            <details class="inspector-section" data-inspector-section="review" hidden={!activeInspectorGroup?.tabs.includes('review')} open={expandedInspectorSections.includes('review')} ontoggle={event => setInspectorExpanded('review', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.review')}</summary><div class="inspector-content">
                {#if inspectorVisited.review}<WorkspaceReview conversationId={active.conversationId} {hostId} onopen={() => openManagementTab(`/workspace-review?conversation=${encodeURIComponent(active?.conversationId ?? '')}`)} />{/if}
            </div>
            </details>
            <details class="inspector-section" data-inspector-section="artifacts" hidden={!activeInspectorGroup?.tabs.includes('artifacts')} open={expandedInspectorSections.includes('artifacts')} ontoggle={event => setInspectorExpanded('artifacts', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.artifacts')}</summary><div class="inspector-content"><ChatArtifacts {messages} /></div>
            </details>


            <details class="inspector-section" data-inspector-section="resources" hidden={!activeInspectorGroup?.tabs.includes('resources')} open={expandedInspectorSections.includes('resources')} ontoggle={event => setInspectorExpanded('resources', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.context')}</summary><div class="inspector-content">
                <ChatOverview compact conversation={active} {selectedModelName} {busy} usage={contextUsage} cache={conversationCacheUsage} cacheFailed={conversationCacheFailed} />
                {#if inspectorVisited.resources}
                    <ChatDataSources conversationId={active.conversationId} {generation} busy={busy || !canInteract} />
                    <ChatPdfResources conversationId={active.conversationId} {generation} busy={busy || !canInteract} onopen={openPdfInTab} onremove={id => updateContentTabs(closeContentTab(contentTabs, pdfTabKey(id)))} />
                    <ChatKnowledge conversationId={active.conversationId} {generation} busy={busy || !canInteract} drafts={workspaceHost?.knowledge} />
                {/if}
            </div>
            </details>
            <details class="inspector-section" data-inspector-section="execution" hidden={!activeInspectorGroup?.tabs.includes('execution')} open={expandedInspectorSections.includes('execution')} ontoggle={event => setInspectorExpanded('execution', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.execution')}</summary><div class="inspector-content">
                {#if mountedInspectorSections.includes('execution')}
                    <ChatActivityGoal readOnly={!canInteract} section="execution" {hostId} conversationId={active.conversationId} draftStore={workspaceHost?.objectives} {generation} {busy} oncontrol={observeSelected} refreshKey={turn?.committedRevision ?? 0} />
                    <ChatBrowser {messages} conversationId={active.conversationId} {generation} />
                {/if}
                </div>
            </details>
            <details class="inspector-section" data-inspector-section="participants" hidden={!activeInspectorGroup?.tabs.includes('participants')} open={expandedInspectorSections.includes('participants')} ontoggle={event => setInspectorExpanded('participants', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.participants')}</summary><div class="inspector-content">{@render expertPicker()}</div>
            </details>
            <details class="inspector-section" data-inspector-section="settings" hidden={!activeInspectorGroup?.tabs.includes('settings')} open={expandedInspectorSections.includes('settings')} ontoggle={event => setInspectorExpanded('settings', event.currentTarget.open)}>
                <summary>{t($locale, 'chatInspector.settings')}</summary><div class="inspector-content">
            {#if active && canInteract}<section class="composer-options">

                <div class="composer-quick-bar"><ChatPermissionMode state={permissionState} onselect={(choice: PermissionChoice) => sessionSettings?.setPermission(choice)} onsave={() => { void sessionSettings?.savePermission(); }} onreload={() => { void sessionSettings?.reloadSettings(); }} />


            </div></section>{/if}
                {#if canInteract}<ChatSessionSettings bind:this={sessionSettings} conversation={active} {generation} {busy} onconversation={conversationSettingsChanged} onmodelstate={state => modelState = state} onmodelname={name => selectedModelName = name} onpermissionstate={state => permissionState = state} onreasoningstate={state => reasoningState = state} />{/if}
            </div>
            </details>

        </div>
        {#if viewportWidth > 1250}<PanelResizeHandle controls="chat-details" label={t($locale, 'shell.resizeInspector')} value={widths.inspector} min={320} max={widths.inspectorMax} defaultValue={layoutDefaults.inspector} reverse onchange={value => setWorkspaceWidth('inspector', value)} />{/if}
        </div>
        </ChatResponsivePanel>{/if}
    </div>
</div>

<style>
    .expert-mentions { max-height: min(224px, 38dvh); overflow-y: auto; overscroll-behavior: contain; border: 1px solid var(--border); border-radius: 14px; padding: 6px; margin-bottom: 8px; }
    #composer-commands { position: absolute; bottom: calc(100% + 6px); inset-inline: 0; z-index: 60; margin-bottom: 0; background: var(--surface-elevated, var(--surface, var(--canvas))); box-shadow: 0 8px 28px rgb(0 0 0 / .18); }
    .expert-mentions > button { width: 100%; min-height: 52px; display: flex; align-items: center; gap: 12px; text-align: start; border: 1px solid transparent; background: transparent; box-shadow: none; padding: 6px 10px; border-radius: 10px; transition: border-color var(--motion-fast) ease; }
    .expert-mentions > button[aria-selected='true'] { background: var(--accent-subtle); border-color: transparent; }
    .mentioned-message{white-space:pre-wrap;overflow-wrap:anywhere;margin:0}
    .inline-name{display:inline;padding:0;min-height:0;border:0;background:none;box-shadow:none;color:var(--primary);font:inherit;font-weight:600}
    .mention-profile{max-width:min(85vw,360px);padding:20px;border:1px solid var(--border);border-radius:12px;background:var(--canvas);color:var(--content)}
    .mention-copy { min-width: 0; display: grid; gap: 3px; }
    .mention-copy strong, .mention-copy small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .mention-copy small { color: var(--muted); }

    .chat-page { display: grid; grid-template-columns: minmax(0, 1fr); grid-template-rows: auto minmax(0, 1fr); gap: 0; height: 100dvh; padding: 0; overflow: hidden; }
    .chat-header { position: relative; height: 48px; min-height: 48px; padding: 0 12px 0 0; min-width: 0; display: flex; justify-content: space-between; align-items: center; gap: 8px; isolation: isolate; z-index: 1; background: var(--rail); border-bottom: 1px solid var(--border); }
    .sidebar-toggle { margin-inline-start: auto; flex: none; }
    .global-navigation { display: flex; align-items: center; gap: 10px; flex: none; width: var(--sidebar-width); height: 100%; padding-inline: 12px; box-sizing: border-box; background: var(--rail); border-inline-end: 1px solid var(--border); min-width: 0; }
    .global-navigation.collapsed { width: auto; background: transparent; border-inline-end-color: transparent; padding-inline: 12px 4px; }
    .desktop-mark { font:bold 30px Georgia,serif; color:var(--content); }
    .mobile-chat-heading { flex:1; min-width:0; display:grid; gap:2px; }
    .mobile-chat-heading strong { font-size:16px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
    .mobile-chat-heading small { font-size:12px; color:var(--muted); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
    .compact-list-categories { display: none; }
    .sidebar-section { min-width: 0; flex:1; overflow:auto; }
    .sidebar-section[hidden] { display: none; }
    .chat-header h1 { flex: 1; margin: 0; font-size: 15px; font-weight: 550; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .navigation-toggle { display: none; flex-shrink: 0; min-height: 44px; }
    .inspector-controls { display: flex; gap: 4px; margin-inline-start: auto; flex-shrink: 0; padding-inline-start: 8px; border-inline-start: 1px solid var(--border); }
    .panel-toggle { display: inline-flex; align-items: center; justify-content: center; width: max(36px, var(--control-hit-size)); height: max(36px, var(--control-hit-size)); padding: 0; border: 1px solid transparent; background: transparent; box-shadow: none; color: var(--muted); }
    .panel-toggle:hover:not(:disabled) { background: var(--composer-pill-hover-bg); border-color: transparent; color: var(--content); }
    .panel-toggle[aria-pressed="true"] { background: var(--glass-pill-bg); border-color: var(--glass-pill-border); box-shadow: var(--glass-pill-shadow); color: var(--content); }
    .inspector-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    .inspector-heading button { width: max(32px, var(--control-hit-size)); height: max(32px, var(--control-hit-size)); padding: 0; display: inline-flex; align-items: center; justify-content: center; }
    .chat-header svg, .panel-toggle svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; }
    .transcript article.user :global(.message-actions) { justify-content:flex-end; }
    @media(min-width:1251px) { .chat-side { height:100%; } }
    .inspector-heading button { border:0; background:transparent;box-shadow:none; }
    .composer-footer { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
    .composer-attachments { display: contents; }
    .composer-footer :global(.text-attachments) { display: contents; }
    .composer-footer :global(.text-attachments > :not(.attachment-toolbar)) { order: -1; flex: 1 0 100%; margin-bottom: 4px; }
    .composer-footer :global(.text-attachments > .attachment-toolbar) { flex: 0 0 auto; }
    .composer-execution { flex: 1; display: flex; align-items: center; gap: 4px; min-width: 0; }
    .composer-actions { flex-shrink: 0; margin-inline-start: auto; }
    .composer-options { flex-shrink: 0; }
    .chat-columns { isolation: isolate; z-index: 0; min-width: 0; display: grid; grid-template-columns: var(--sidebar-width) minmax(0, 1fr) var(--inspector-width); min-height: 0; gap: 0; }
    .chat-columns.without-conversation { grid-template-columns: var(--sidebar-width) minmax(0, 1fr); }
    .chat-columns.without-navigation { grid-template-columns: minmax(0, 1fr) var(--inspector-width); }
    .chat-columns.without-navigation.without-conversation { grid-template-columns: minmax(0, 1fr); }
    .empty-navigation { display: none; }
    .empty-conversation { display: grid; justify-items: center; align-content: center; gap: 14px; min-height: 100%; padding: 16px; text-align: center; }
    .empty-mark { font: italic 600 40px/1 Georgia, serif; color: var(--content); }
    .empty-conversation h2 { font-family: var(--font-display); color: var(--content); font-size: clamp(1.5rem, 1.25rem + 1vw, 2rem); line-height: 1.3; font-weight: 600; text-wrap: balance; }
    .panel-column { position: relative; min-width: 0; min-height: 0; }
    .chat-navigation { display: flex; flex-direction: column; gap: 12px; min-width: 0; height: calc(100dvh - 48px); max-height: calc(100dvh - 48px); overflow: hidden; padding: 12px; border: 0; background: var(--canvas); border-right:1px solid var(--border); }
    .activity-source-return { display:flex; align-items:center; gap:8px; min-height:44px; flex-shrink:0; padding:8px 16px; border-bottom:1px solid var(--border); }
    .chat-main { display: flex; min-width: 0; min-height: 0; flex-direction: column; overflow: auto; background: var(--canvas); }
    .conversation-panel { display: flex; flex: 1; flex-direction: column; min-height: 0; overflow: hidden; }
    .conversation-panel[hidden], .content-document-panel[hidden] { display: none; }
    .content-document-panel { min-height: 0; height: 100%; overflow: hidden; }
    .initialization-error { width:calc(100% - 48px); max-width:640px; margin:32px auto 0; }
    .initialization-retry { align-self:center; min-height:44px; margin:8px 24px 24px; }
    .chat-search { display: flex; align-items: center; gap: 8px; padding: 7px 16px; border-bottom: 1px solid var(--border); background: var(--panel); font-size: 12px; }
    .chat-search input { flex: 1; min-width: 0; max-width: 360px; }
    .chat-search span { color: var(--muted); white-space: nowrap; }
    .chat-search button { margin-inline-start: auto; }
    .transcript-shell { position: relative; flex: 1 1 0; min-height: 80px; display: flex; flex-direction: column; overflow: hidden; }
    .transcript:focus-visible { outline-offset: -3px; }
    .transcript { flex: 1 1 0; min-height: 80px; overflow: auto; padding: 24px max(32px, calc((100% - 752px)/2)); scrollbar-gutter: stable; font-size: 1rem; line-height: 1.625; }
    .transcript article { display: flex; align-items: flex-start; gap: 8px; margin: 24px 0; padding: 12px 0; border: 1px solid transparent; position: relative; border-radius: 12px; }
    .transcript article.user { width: fit-content; max-width: 85%; margin-inline-start: auto; padding: 0; }
    .message-column { min-width: 0; flex: 1; }
    .transcript article.user .message-content {
        flex: 0 1 auto;
        width: fit-content;
        max-width: 100%;
        margin-inline-start: auto;
        padding: 10px 16px;
        border-radius: 18px;
        background: var(--panel);
    }
    .transcript article.selected { background: var(--accent-subtle); border-color: transparent; }
    .transcript article:not(.user) .message-content { max-width: 720px; }
    .transcript article:has(> .response-identity) { flex-direction: column; align-items: stretch; gap: 12px; }
    .transcript article:has(> .response-identity) .message-content { background: transparent; box-shadow: none; padding: 0; border-radius: 0; }
    .transcript article.search-match { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, var(--canvas)); }
    .transcript article.pending { border-style: dashed; border-color: var(--outline); } .message-content { min-width: 0; flex: 1; overflow-wrap: anywhere; }
    .attached-files { display: flex; flex-wrap: wrap; gap: 5px; list-style: none; margin: 7px 0; padding: 0; }
    .attached-files li { max-width: 100%; overflow-wrap: anywhere; padding: 3px 7px; border: 1px solid var(--border); border-radius: 7px; background: var(--panel); color: var(--muted); font-size: 11px; }
    .placeholder { color: var(--muted); text-align: center; margin-top: 12vh; } .muted, small { color: var(--muted); font-size: 12px; }
    .older { display: block; margin: 0 auto; }
    .latest { position: absolute; bottom: 12px; inset-inline: 0; margin-inline: auto; width: fit-content; z-index: 5; background: var(--surface-elevated, var(--surface)); border-radius: 999px; padding: 6px 14px; font-size: 13px; box-shadow: 0 4px 16px rgb(0 0 0 / .14); }
    .response-identity { display: flex; flex: none; min-width: 0; align-items: center; gap: 7px; font-size: 14px; line-height: 20px; overflow-wrap: anywhere; }
    .response-avatar { display: grid; place-items: center; width: 19px; height: 19px; border: 1px solid var(--outline); border-radius: 50%; color: var(--content); font-size: 13px; }
    .expert-picker { display: flex; flex-direction:column; align-items:stretch; gap: 8px; padding: 10px 16px; border-bottom: 1px solid var(--border); background: var(--surface); font-size: 12px; }
    .expert-picker-heading { display:flex; align-items:center; justify-content:space-between; gap:12px; }
    .expert-picker label { display: inline-flex; align-items: center; gap: 6px; min-height: 36px; cursor: pointer; }
    .expert-picker input { width: 16px; height: 16px; }
    .expert-picker p { margin: 0; color: var(--danger); }
    .transcript article.expert-message { display: block; max-width: 100%; padding: 0; border: 0; background: transparent; box-shadow: none; }
    .expert-choice { width:100%; display: flex; align-items: center; gap: 4px; }
    .expert-choice label { min-height: 44px; }
    .run-status { margin: 0; padding: 8px 0; color: var(--muted); font-size: 12px; }
    .run-status.live { width: fit-content; background: linear-gradient(90deg, var(--muted) 0%, var(--content) 50%, var(--muted) 100%); background-size: 200% 100%; -webkit-background-clip: text; background-clip: text; color: transparent; animation: nola-shimmer 2.4s ease-in-out infinite; }
    @keyframes nola-shimmer { 0% { background-position: 100% 0; } 100% { background-position: -100% 0; } }
    @media (prefers-reduced-motion: reduce) { .run-status.live { background: none; color: var(--muted); animation: none; } }
    @media (forced-colors: active) { .run-status.live { background: none; color: inherit; animation: none; } }
    .composer { position: relative; flex-shrink: 0; width: calc(100% - 64px); max-width: 720px; align-self: center; margin: 10px 12px 16px; padding: 12px 16px; border: 1px solid var(--composer-border); border-radius: 16px; background: var(--composer-bg); box-shadow: var(--composer-shadow); transition: border-color var(--motion-fast) ease, box-shadow var(--motion-fast) ease, background-color var(--motion-base) var(--motion-disclosure); }
    .composer:focus-within { outline: none; border-color: var(--accent); box-shadow: var(--composer-focus-shadow); }
    @media (prefers-color-scheme: dark) { :global(:root:not([data-appearance="light"])) .composer:focus-within { border-color: var(--composer-focus-border); box-shadow: var(--composer-focus-shadow); } }
    :global(:root[data-appearance="dark"]) .composer:focus-within { border-color: var(--composer-focus-border); box-shadow: var(--composer-focus-shadow); }
    @media (forced-colors: active) { .composer:focus-within { outline: 2px solid Highlight; outline-offset: 2px; box-shadow: none; } }
    .composer-footer :global(.picker),
    .composer-footer :global(.model-menu > summary),
    .composer-footer :global(.reasoning-menu > summary),
    .composer-actions button:not(.primary) { background: transparent; border: 1px solid transparent; border-radius: 8px; color: var(--muted); transition: background-color var(--motion-fast) ease, color var(--motion-fast) ease; }
    .composer-footer :global(.model-menu > summary),
    .composer-footer :global(.reasoning-menu > summary) { min-height: 32px; padding: 4px 8px; font-size: 12.5px; }
    .composer-footer :global(.picker:hover:not(.disabled)),
    .composer-footer :global(.model-menu > summary:hover),
    .composer-footer :global(.reasoning-menu > summary:hover:not([aria-disabled="true"])),
    .composer-actions button:not(.primary):not(:disabled):hover { background: var(--composer-pill-hover-bg); border-color: transparent; color: var(--content); }
    .composer-actions { display: flex; justify-content: flex-end; gap: 8px; }
    .composer-quick-bar { display: flex; flex-shrink: 0; flex-wrap: wrap; align-items: center; gap: 16px; padding: 0 0 16px; width: 100%; margin: 0; }
    .chat-side { position:relative; display: grid; align-content: start; gap: 8px; min-width: 0; min-height:0; overflow:auto; overscroll-behavior:contain; padding: 12px; border-left: 1px solid var(--border); background: var(--canvas); }
    .inspector-content { display: grid; align-content: start; gap: 16px; min-width: 0; padding: 10px 2px 14px; }
    .inspector-section { min-width:0; border-bottom:1px solid var(--border); interpolate-size:allow-keywords; }
    @supports (interpolate-size:allow-keywords) {
        .inspector-section::details-content { block-size:0; opacity:0; overflow:clip; transition:block-size var(--motion-base) var(--motion-disclosure), opacity var(--motion-fast) ease, content-visibility var(--motion-base) allow-discrete; }
        .inspector-section[open]::details-content { block-size:auto; opacity:1; }
    }
    .inspector-section > summary { min-height:44px; display:flex; align-items:center; gap:10px; cursor:pointer; list-style:none; font-weight:600; font-size:.875rem; border-radius:8px; padding:6px 8px; }
    .inspector-section > summary::-webkit-details-marker { display:none; }
    .inspector-section > summary::before { content:'›'; font-size:1.1rem; transition:transform var(--motion-base) ease; }
    .inspector-section[open] > summary::before { transform:rotate(90deg); }
    .inspector-section > summary:hover { background:var(--accent-subtle); }
    .inspector-section > summary:focus-visible { outline:2px solid var(--primary); outline-offset:-2px; }
    .inspector-heading { position:sticky; top:-12px; z-index:2; background:var(--canvas); padding-block:6px; }
    @media (prefers-reduced-motion: reduce) { .inspector-section > summary::before, .inspector-section::details-content { transition:none; } }
    .queued-action { border: 1px solid var(--border); border-radius: 12px; padding: 12px 16px; }
    .queued-action button { min-height: 44px; padding: 8px 16px; }
    .notice { margin: 8px 16px 0; color: var(--muted); } .notice.error { color: var(--danger); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    @media (max-width: 1250px) { .chat-columns { grid-template-columns: var(--sidebar-width) minmax(0, 1fr); } .chat-columns.without-navigation { grid-template-columns: minmax(0, 1fr); } .inspector-content { max-height: none; } .inspector-heading { display: none; } }
    @media (max-width: 760px) {
        .expert-picker { gap: 5px 10px; }
        .expert-picker label { min-height: 44px; }
        .transcript article.expert-message { max-width: 100%; padding: 0; }
        .chat-page { padding: env(safe-area-inset-top, 0px) 0 env(safe-area-inset-bottom, 0px); gap: 0; background: var(--canvas); }
        .chat-columns, .chat-columns.without-conversation { grid-template-columns: minmax(0, 1fr); gap: 0; }
        .chat-header { height:76px; gap:2px; padding:8px 12px; flex-wrap:nowrap; background:var(--canvas); }
        .global-navigation { display: none; }
        .compact-list-categories { display: block; }
        .chat-header :global(.model-menu) { display: none; }
        .chat-header > .inspector-controls { display: none; }
        .empty-navigation { display: inline-flex; }
        .chat-header :global(.model-menu > summary) { justify-content: center; }
        .chat-header :global(.model-menu summary span:first-child) { max-width: 65vw; }
        .chat-header h1 { flex: 1; font-size: 16px; font-weight: 600; }
        .navigation-toggle { display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0; border: 0; background: transparent; box-shadow: none; color: var(--muted); flex-shrink: 0; }
        .panel-toggle { width: 44px; height: 44px; }
        .inspector-controls { gap: 0; }
        .chat-navigation { height: auto; max-height: none; }
        .chat-main { border: 0; border-radius: 0; }
        .transcript { min-height: 80px; padding: 12px 16px; scrollbar-gutter: auto; overscroll-behavior-y: contain; font-size: 1.0625rem; line-height: 28px; }
        .transcript article { padding: 10px 0; margin: 12px 0; gap: 4px; }
        .transcript article.user { padding: 0; margin-inline-start: auto; }
        .transcript article.user .message-content { padding: 10px 12px; }
        .transcript article:has(> .response-identity) { gap: 10px; }
        .composer { width: calc(100% - 40px); margin:10px 20px max(12px, env(safe-area-inset-bottom)); padding:4px; border-radius:24px; background:var(--composer-bg); }
        .composer-footer { align-items: center; gap: 0; }
        .composer-execution { flex: 1; justify-content: flex-end; }
        .composer-footer :global(.model-menu) { flex: 1; min-width: 0; max-width: none; }
        .composer-footer :global(.model-menu > summary) { min-height: 44px; justify-content: flex-start; font-size: 12px; padding-inline: 8px; }
        .composer-footer :global(.model-menu summary span:first-child) { max-width: none; min-width: 0; }
        .composer-footer :global(.model-menu .choices) { position: absolute; inset: auto auto calc(100% + 8px) 0; translate: none; width: min(340px, calc(100vw - 32px)); max-height: min(52dvh, 400px); }
        .composer-actions button { min-height: 44px; border-radius: 14px; padding: 8px 12px; }
        .composer-options { margin: 0 12px; max-height: 35dvh; overflow: auto; }
        .composer-quick-bar { width: 100%; padding: 0 4px 12px; }
        .chat-search { flex-wrap: wrap; padding: 8px 16px; }
        .chat-search label { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    }
    .composer-actions button.primary { width:36px; height:36px; min-width:36px; min-height:36px; padding:7px; border-radius:50%; display:grid; place-items:center; }
    .composer-actions svg { width:20px; height:20px; fill:none; stroke:currentColor; stroke-width:1.6; }
    .composer-footer :global(.model-menu) { align-self:center; }
    @media(min-width:761px) {
        .composer-footer :global(.model-menu .choices) { top:auto; bottom:calc(100% + 8px); left:0; right:auto; }
        .composer-footer :global(.model-menu[open] .choices) { animation: composer-reveal-up var(--motion-base) ease-out; }
    }
    @keyframes composer-reveal-up { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: translateY(0); } }
    @media (prefers-reduced-motion: reduce) { .composer-footer :global(.model-menu[open] .choices) { animation: none; } }
    @media(max-width:760px) { .composer-actions button.primary { width:44px; height:44px; min-height:44px; min-width:44px; padding:11px; } }
    @media(max-width:359px) { .composer { width:calc(100% - 32px); margin-inline:16px; } .chat-header { padding-inline:4px; } }
    @media(max-width:359px) {
        .transcript article:has(> .response-identity) { gap: 8px; }
    }
    @media (min-width: 360px) and (max-width: 760px) { .transcript { padding-inline: 20px; } }
    .attachment-help { font-size:13px; color:var(--muted); }
    .attachment-help summary { min-height:44px; padding:12px 8px; cursor:pointer; }
    .attachment-help p { padding:0 8px; line-height:1.6; }
    .workspace-footer { display:flex; align-items:center; gap:2px; flex-shrink:0; padding-top:12px; border-top:1px solid var(--border); }
    .workspace-footer button { background:transparent;box-shadow:none; border:0; padding:6px; min-width:32px; color:var(--muted); }
    .workspace-footer .workspace-identity { display:flex; gap:8px; align-items:center; flex:1; min-width:0; text-align:start; font-size:12px; }
    .workspace-identity > span:first-child { font:bold 24px Georgia,serif; }
    .workspace-identity small { display:block; font-size:10px; }
    .workspace-footer svg { width:16px; height:16px; fill:none; stroke:currentColor; stroke-width:1.6; }
    @media(max-width:760px) { .workspace-footer { display:none; } }
</style>
