<script lang="ts">
    import { onMount, tick } from 'svelte';
    import { locale, t, type Locale } from '../lib/i18n';
    import { darkAppearance, toggleAppearance } from '../lib/appearance';
    import { ApiError, errorMessage } from '../lib/api';
    import { pushToast } from '../lib/toast';
    import { workspaceContentNavigationEvent, navigateWorkspace } from '../lib/workspaceNavigation';
    import { workspaceTabCloseEvent } from '../lib/workspacePageLocation';
    import { layoutDefaults, setWorkspaceWidth, visiblePanelWidths, workspaceWidths } from '../lib/workspaceLayout';
    import {
        expertAccentPalette,
        expertBubbleColor,
        expertBubblePalette,
        observeExpertBubbleColors,
    } from '../lib/expertBubbleColors';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import {
        emptyContentTabs,
        managementContentPath,
        openPageContentTab,
        selectContentTab,
        closeContentTab,
        updatePageContentLocation,
        conversationTabKey,
        contentTabDomId,
        type ConversationContentTabs,
    } from '../lib/conversationContentTabs';
    import { displayConversationTitle, type ChatConversation } from '../lib/chatConversations';
    import type { UiInteraction, UiSnapshot, UiSubmissionOutcome } from '../lib/generativeUi';
    import {
        branchV2Conversation,
        cancelV2Turn,
        compactV2Conversation,
        createV2Conversation,
        emptyV2TokenUsage,
        fetchV2AgentRoomsDirectory,
        formatPermissionSummary,
        getV2Conversation,
        getV2ConversationConfiguration,
        getV2ConversationPlan,
        hasConfiguredChatModel,
        listV2Conversations,
        listV2Experts,
        listV2ModelChoices,
        loadV2Messages,
        loadV2MessagesWithUsage,
        mergeV2TokenUsage,
        normalizeV2PermissionProfile,
        normalizeV2ReasoningEffort,
        observeV2ConversationProgress,
        parseV2Attachment,
        patchV2ConversationConfiguration,
        respondV2Permission,
        respondV2UserInput,
        streamV2ChatMessage,
        streamV2ConversationObserve,
        submitV2UiInteraction,
        truncateV2Conversation,
        updateV2ConversationPlan,
        type V2AgentRoom,
        type V2Attachment,
        type V2ConversationPlan,
        type V2Expert,
        type V2Message,
        type V2ModelChoice,
        type V2PermissionProfile,
        type V2PermissionPrompt,
        type V2PermissionScope,
        type V2ReasoningEffort,
        type V2TokenUsage,
        type V2ToolCall,
        type V2UiActionPayload,
        type V2UserQuestion,
    } from '../lib/v2Chat';
    import ChatConversationManager from '../components/ChatConversationManager.svelte';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import ConversationInspector from '../components/ConversationInspector.svelte';
    import type { InspectorPreviewItem } from '../lib/inspectorPreview';
    import DesktopPluginNavigation from '../components/DesktopPluginNavigation.svelte';
    import ExpertRoomManager from '../components/ExpertRoomManager.svelte';
    import PanelResizeHandle from '../components/PanelResizeHandle.svelte';
    import WorkspaceListNavigation from '../components/WorkspaceListNavigation.svelte';
    import WorkspaceResourceNavigation from '../components/WorkspaceResourceNavigation.svelte';
    import ConversationContentTabsView from '../components/ConversationContentTabs.svelte';
    import WorkspacePageTab from '../components/WorkspacePageTab.svelte';
    import Markdown from '../components/Markdown.svelte';
    import PrototypeIcon from '../components/PrototypeIcon.svelte';
    import Surface from '../vendor/a2ui-svelte/render/Surface.svelte';

    const NEW_DRAFT_KEY = '__new__';

    let viewportWidth = $state(typeof window === 'undefined' ? 1280 : window.innerWidth);
    let sidebarOpen = $state(true);
    let inspectorOpen = $state(false);
    let listCategory = $state<'conversations' | 'resources'>('conversations');
    let conversationManager = $state<ChatConversationManager>();
    let confirmation = $state<ConfirmationHandle>();
    let isDragOver = $state(false);

    let active = $state<ChatConversation | null>(null);
    let messages = $state<V2Message[]>([]);
    let sessionUsage = $state<V2TokenUsage>(emptyV2TokenUsage());
    let draft = $state('');
    const conversationDrafts = new Map<string, string>();
    let attachments = $state<V2Attachment[]>([]);
    let inspectorPreview = $state<InspectorPreviewItem | null>(null);
    let loading = $state(false);
    let streaming = $state(false);

    function openAttachmentInInspector(att: V2Attachment) {
        const isImage = Boolean(att.dataUrl) || att.mimeType.toLowerCase().startsWith('image/');
        inspectorPreview = {
            title: att.name,
            subtitle: att.mimeType,
            kind: isImage ? 'image' : 'text',
            mimeType: att.mimeType,
            imageUrl: isImage ? att.dataUrl : undefined,
            text: isImage ? undefined : att.text || att.name,
        };
        inspectorOpen = true;
    }

    $effect(() => {
        active?.conversationId;
        inspectorPreview = null;
    });

    function stickReasoningToBottom(node: HTMLElement, value: string) {
        const scroll = (v: string) => {
            if (!v) return;
            queueMicrotask(() => {
                node.scrollTop = node.scrollHeight;
            });
        };
        scroll(value);
        return {
            update(next: string) {
                scroll(next);
            },
        };
    }
    let error = $state('');
    let statusNotice = $state('');
    let needsModelConfig = $state(false);
    let generation = $state(0);
    let streamController: AbortController | null = null;
    let transcriptEl = $state<HTMLDivElement>();
    let composerEl = $state<HTMLTextAreaElement>();
    let fileInputEl = $state<HTMLInputElement>();
    let expertPopupEl = $state<HTMLDivElement>();
    let expertToggleBtnEl = $state<HTMLButtonElement>();

    // Smart scroll anchor
    let isPinnedToBottom = $state(true);

    // Model, strict 2-mode permission, and reasoning effort
    let modelChoices = $state<V2ModelChoice[]>([]);
    let selectedModelId = $state('');
    let permissionProfile = $state<V2PermissionProfile>('cautious');
    let permissionSaving = $state(false);
    let reasoningEffort = $state<V2ReasoningEffort>('automatic');

    // Interactive prompts & submitting lock
    let pendingPermission = $state<V2PermissionPrompt | null>(null);
    let permissionSubmitting = $state(false);
    let pendingQuestion = $state<V2UserQuestion | null>(null);
    let questionAnswerDraft = $state('');

    // Inline Edit & Resend confirmation
    let confirmEditIndex = $state<number | null>(null);

    // Per-message cancellable 1800ms copy feedback
    let copiedMessageIds = $state<Record<string, boolean>>({});
    const copyTimers = new Map<string, ReturnType<typeof setTimeout>>();

    // @ Expert mention picker & Rooms
    let availableExperts = $state<V2Expert[]>([]);
    let availableRooms = $state<V2AgentRoom[]>([]);
    let attachedRoom = $state<V2AgentRoom | null>(null);
    let hostId = $state('local');
    let expertManagerOpen = $state(false);
    let expertColorVersion = $state(0);
    let expertsLoading = $state(false);
    let expertsLoaded = $state(false);
    let expertPickerOpen = $state(false);
    let expertQuery = $state('');
    let expertTriggerRange = $state<{ start: number; end: number } | null>(null);
    let expertHighlightIdx = $state(0);
    let selectedExperts = $state<V2Expert[]>([]);

    // Plan Mode & Active Stream Reconnection
    let conversationPlan = $state<V2ConversationPlan>({ enabled: false, status: 'idle', steps: [] });
    let planBusy = $state(false);
    let reconnecting = $state(false);

    const panelLayout = $derived(visiblePanelWidths(viewportWidth, $workspaceWidths, true));
    const showPlanBanner = $derived(
        conversationPlan.status === 'awaiting_approval' ||
            conversationPlan.status === 'proposed' ||
            conversationPlan.status === 'pending' ||
            (conversationPlan.enabled &&
                conversationPlan.status === 'draft' &&
                (Boolean(conversationPlan.markdown) || conversationPlan.steps.length > 0)),
    );

    const filteredExperts = $derived.by(() => {
        const q = expertQuery.trim().toLowerCase();
        const selectedIds = new Set(selectedExperts.map(e => e.id));
        return availableExperts.filter(exp => {
            if (selectedIds.has(exp.id)) return false;
            if (!q) return true;
            return (
                exp.name.toLowerCase().includes(q) ||
                exp.id.toLowerCase().includes(q) ||
                (exp.description ?? '').toLowerCase().includes(q)
            );
        });
    });

    const permissionSummary = $derived(pendingPermission ? formatPermissionSummary(pendingPermission) : null);

    function shortenDisplayPath(raw: string): string {
        const cleaned = raw.trim().replace(/^['"]|['"]$/g, '');
        if (!cleaned) return '';
        const normalized = cleaned.replace(/\\/g, '/');
        const tempMatch = normalized.match(/(?:^|\/)(?:var\/folders\/[^/]+\/[^/]+\/T|private\/tmp|tmp)\/[^/]+\/(.+)$/);
        if (tempMatch?.[1]) {
            const parts = tempMatch[1].split('/').filter(Boolean);
            return parts.length > 2 ? parts.slice(-2).join('/') : parts.join('/');
        }
        const parts = normalized.split('/').filter(Boolean);
        if (parts.length > 2) return parts.slice(-2).join('/');
        return cleaned;
    }

    function formatToolInlinePreview(tc: V2ToolCall): string {
        const summary = (tc.summary ?? '').trim();
        const args =
            tc.arguments && typeof tc.arguments === 'object' && !Array.isArray(tc.arguments)
                ? (tc.arguments as Record<string, unknown>)
                : null;
        if (tc.status === 'failed' && summary) {
            const exitMatch = summary.match(/shell exited with status (\d+)[^:]*:\s*(.+)/i);
            if (exitMatch) {
                return `exit ${exitMatch[1]} · ${exitMatch[2].trim().split('\n')[0]}`;
            }
            const firstLine = summary.split('\n')[0].trim();
            return firstLine.replace(/\/(?:var\/folders\/[^\s]+|private\/tmp\/[^\s]+|tmp\/[^\s]+)/g, m => shortenDisplayPath(m));
        }
        if (args) {
            for (const key of ['path', 'file_path', 'filePath', 'target_path', 'target']) {
                if (typeof args[key] === 'string' && args[key].trim()) {
                    const shortened = shortenDisplayPath(args[key]);
                    const basename = shortened.split('/').pop() || shortened;
                    return basename;
                }
            }
            for (const key of ['command', 'cmd', 'query', 'pattern', 'url', 'uri']) {
                if (typeof args[key] === 'string' && args[key].trim()) {
                    return args[key].trim().split('\n')[0];
                }
            }
        }
        if (summary) {
            const firstLine = summary.split('\n')[0].trim();
            const readMatch = firstLine.match(/^Read\s+(.+?)(\s*\([^)]+\))?$/i);
            if (readMatch) {
                const file = shortenDisplayPath(readMatch[1]).split('/').pop() || shortenDisplayPath(readMatch[1]);
                return `${file}${readMatch[2] ?? ''}`;
            }
            const wroteMatch = firstLine.match(/^Wrote\s+(.+?)(\s*\([^)]+\))?$/i);
            if (wroteMatch) {
                const file = shortenDisplayPath(wroteMatch[1]).split('/').pop() || shortenDisplayPath(wroteMatch[1]);
                return `${file}${wroteMatch[2] ?? ''}`;
            }
            return firstLine.replace(/\/(?:var\/folders\/[^\s]+|private\/tmp\/[^\s]+|tmp\/[^\s]+)/g, m => shortenDisplayPath(m));
        }
        return '';
    }

    function formatPermissionReason(prompt: V2PermissionPrompt, currentLocale: Locale): string {
        const raw = (prompt.reason ?? '').trim();
        if (!raw) return t(currentLocale, 'chatPermission.ready');
        const cautiousMatch = raw.match(/^(?:cautious profile requires approval for|approval required for)\s+(.+)$/i);
        if (cautiousMatch) {
            return t(currentLocale, 'chatPermission.cautiousRequiresApproval', {
                tool: cautiousMatch[1].trim(),
            });
        }
        return raw;
    }

    function formatDisplayError(raw: string): string {
        const trimmed = raw.trim();
        if (!trimmed) return '';
        const statusJsonMatch = trimmed.match(/^(?:llm error:\s*)?(?:provider returned\s+)?status\s+(\d{3})\s*:\s*(\{[\s\S]*\})$/i);
        if (statusJsonMatch) {
            const status = statusJsonMatch[1];
            try {
                const parsed = JSON.parse(statusJsonMatch[2]) as {
                    error?: { message?: unknown; code?: unknown };
                    message?: unknown;
                    detail?: unknown;
                };
                const msg =
                    (typeof parsed?.error?.message === 'string' && parsed.error.message.trim()) ||
                    (typeof parsed?.message === 'string' && parsed.message.trim()) ||
                    (typeof parsed?.detail === 'string' && parsed.detail.trim()) ||
                    '';
                if (msg) {
                    return msg.includes(status) ? msg : `HTTP ${status} · ${msg}`;
                }
            } catch {
                // Fall through to generic cleanup.
            }
        }
        const jsonIdx = trimmed.indexOf('{');
        if (jsonIdx > 0 && trimmed.endsWith('}')) {
            const prefix = trimmed.slice(0, jsonIdx).replace(/[:\s]+$/, '');
            try {
                const parsed = JSON.parse(trimmed.slice(jsonIdx)) as {
                    error?: { message?: unknown };
                    message?: unknown;
                    detail?: unknown;
                };
                const msg =
                    (typeof parsed?.error?.message === 'string' && parsed.error.message.trim()) ||
                    (typeof parsed?.message === 'string' && parsed.message.trim()) ||
                    (typeof parsed?.detail === 'string' && parsed.detail.trim()) ||
                    '';
                if (msg) {
                    const statusMatch = prefix.match(/\b([45]\d{2})\b/);
                    if (statusMatch && !msg.includes(statusMatch[1])) {
                        return `HTTP ${statusMatch[1]} · ${msg}`;
                    }
                    return msg;
                }
            } catch {
                // Keep original message.
            }
        }
        return trimmed;
    }

    const formattedError = $derived(error ? formatDisplayError(error) : '');

    let contentTabs = $state<ConversationContentTabs>(emptyContentTabs());
    const tabViewports = new Map<string, { top: number }>();
    const selectedContentTab = $derived(contentTabs.items.find(item => item.key === contentTabs.selected));
    const managementContentSelected = $derived(selectedContentTab?.kind === 'page');
    const isConversationTabActive = $derived(contentTabs.selected === conversationTabKey);
    const activeTabTitle = $derived(displayConversationTitle(active?.title, t($locale, 'shell.conversations.new')));
    const ownerKey = $derived(active?.conversationId ?? 'workspace');

    function currentDraftKey(conv: ChatConversation | null = active): string {
        return conv?.conversationId ?? NEW_DRAFT_KEY;
    }

    function saveDraftForKey(key: string, text: string) {
        if (text) conversationDrafts.set(key, text);
        else conversationDrafts.delete(key);
    }

    function handleDraftInput() {
        saveDraftForKey(currentDraftKey(), draft);
        detectExpertMentionTrigger();
    }

    function viewportFor(key: string): { top: number } {
        let entry = tabViewports.get(key);
        if (!entry) {
            entry = { top: 0 };
            tabViewports.set(key, entry);
        }
        return entry;
    }

    function syncUrl(conversationId: string | null, contentPath?: string | null) {
        if (typeof window === 'undefined') return;
        const url = new URL(window.location.href);
        if (conversationId) url.searchParams.set('conversation', conversationId);
        else url.searchParams.delete('conversation');
        if (contentPath !== undefined) {
            if (contentPath) url.searchParams.set('content', contentPath);
            else url.searchParams.delete('content');
        }
        window.history.replaceState(window.history.state, '', url);
    }

    function handleTranscriptScroll() {
        if (!transcriptEl) return;
        isPinnedToBottom = transcriptEl.scrollHeight - transcriptEl.scrollTop - transcriptEl.clientHeight < 80;
    }

    async function scrollToBottom(force = false) {
        if (!force && !isPinnedToBottom) return;
        if (force) isPinnedToBottom = true;
        await tick();
        if (transcriptEl) transcriptEl.scrollTop = transcriptEl.scrollHeight;
    }

    /** Detaches only the browser SSE reader without cancelling the Host run. */
    function detachStream() {
        streamController?.abort();
        streamController = null;
        pendingPermission = null;
        permissionSubmitting = false;
        pendingQuestion = null;
        streaming = false;
    }

    let cancelling = $state(false);

    /** Explicit user Stop button: cancels the active Host run and detaches the SSE reader once confirmed. */
    async function cancelActiveRun() {
        if (cancelling) return;
        const targetId = active?.conversationId ?? null;
        if (!targetId) {
            detachStream();
            return;
        }
        const serial = generation;
        cancelling = true;
        error = '';
        try {
            await cancelV2Turn(targetId);
            // Wait briefly for the host run to release its active turn lock so an
            // immediate follow-up send does not collide with HTTP 409 conversation_busy.
            const settleDeadline = Date.now() + 3000;
            while (serial === generation && streaming && Date.now() < settleDeadline) {
                try {
                    const obs = await observeV2ConversationProgress(targetId);
                    if (!obs.active) break;
                } catch {
                    break;
                }
                await new Promise(resolve => setTimeout(resolve, 100));
            }
            if (serial === generation) {
                detachStream();
            }
        } catch (cause) {
            const msg = errorMessage(cause);
            if (serial === generation) {
                error = msg;
            }
            pushToast(msg, 'error');
        } finally {
            cancelling = false;
        }
    }

    async function refreshModelChoices() {
        try {
            const list = await listV2ModelChoices();
            modelChoices = list;
            if (!selectedModelId || !list.some(m => m.id === selectedModelId)) {
                selectedModelId = list.find(m => m.selected)?.id ?? list[0]?.id ?? '';
            }
            if (list.length > 0) {
                needsModelConfig = false;
            }
        } catch {
            // Optional toolbar metadata.
        }
    }

    async function ensureExpertsLoaded(forceRefresh = false) {
        if (expertsLoading || (expertsLoaded && !forceRefresh)) return;
        expertsLoading = true;
        try {
            const dir = await fetchV2AgentRoomsDirectory();
            hostId = dir.hostId;
            availableExperts = dir.experts;
            availableRooms = dir.rooms;
            if (active) {
                attachedRoom = dir.rooms.find(r => r.sessionId === active?.conversationId) ?? attachedRoom;
            }
            expertsLoaded = true;
        } catch {
            try {
                availableExperts = await listV2Experts(forceRefresh);
                expertsLoaded = true;
            } catch {
                // Keep graceful empty list if agent-rooms is unavailable.
            }
        } finally {
            expertsLoading = false;
        }
    }

    function resolveExpertAccent(expertId?: string, expertName?: string, explicitColor?: string) {
        void expertColorVersion;
        const key = expertId || expertName || 'expert';
        const color = expertBubbleColor(hostId, key, explicitColor);
        return {
            accent: expertAccentPalette[color],
            light: expertBubblePalette[color][0],
            dark: expertBubblePalette[color][1],
        };
    }

    async function handleTogglePlanMode() {
        if (!active || planBusy) return;
        const convId = active.conversationId;
        planBusy = true;
        error = '';
        try {
            const nextEnabled = !conversationPlan.enabled;
            const updated = await updateV2ConversationPlan(convId, {
                enabled: nextEnabled,
                action: nextEnabled ? 'enable' : 'disable',
            });
            if (active?.conversationId === convId) {
                conversationPlan = updated;
                statusNotice = updated.enabled
                    ? t($locale, 'chatInspector.planActive')
                    : t($locale, 'chatInspector.planMode');
            }
        } catch (cause) {
            if (active?.conversationId === convId) error = errorMessage(cause);
        } finally {
            planBusy = false;
        }
    }

    async function handlePlanDecision(action: 'approve' | 'reject' | 'clear') {
        if (!active || planBusy) return;
        const convId = active.conversationId;
        planBusy = true;
        error = '';
        try {
            const updated = await updateV2ConversationPlan(convId, {
                action,
                ...(action === 'clear' ? { enabled: false } : {}),
            });
            if (active?.conversationId === convId) {
                conversationPlan = updated;
            }
        } catch (cause) {
            if (active?.conversationId === convId) error = errorMessage(cause);
        } finally {
            planBusy = false;
        }
    }

    async function reconnectActiveTurn(convId: string, serial: number) {
        try {
            const obs = await observeV2ConversationProgress(convId);
            if (serial !== generation || active?.conversationId !== convId) return;
            if (obs.pendingPermission) pendingPermission = obs.pendingPermission;
            if (obs.pendingQuestion) pendingQuestion = obs.pendingQuestion;
            if (!obs.active) return;
            reconnecting = true;
            streaming = true;
            statusNotice = t($locale, 'chatInspector.reconnecting');
            const assistantId = `obs-${Date.now()}`;
            const hasTrailingAssistant =
                messages.length > 0 && messages[messages.length - 1].role === 'assistant' && !messages[messages.length - 1].seq;
            const targetId = hasTrailingAssistant ? messages[messages.length - 1].id : assistantId;
            if (!hasTrailingAssistant) {
                messages = [
                    ...messages,
                    {
                        id: assistantId,
                        role: 'assistant',
                        content: obs.text ?? '',
                        reasoning: obs.reasoning ?? '',
                        toolCalls: obs.toolCalls ?? [],
                        createdAtMs: Date.now(),
                    },
                ];
            }
            if (obs.pendingPermission) pendingPermission = obs.pendingPermission;
            if (obs.pendingQuestion) pendingQuestion = obs.pendingQuestion;
            await scrollToBottom(true);

            const controller = new AbortController();
            streamController = controller;
            const updateObserved = (fn: (m: V2Message) => V2Message) => {
                messages = messages.map(m => (m.id === targetId ? fn(m) : m));
            };

            try {
                await streamV2ConversationObserve(
                    convId,
                    {
                        onSnapshot: snap => {
                            if (serial !== generation) return;
                            updateObserved(m => ({
                                ...m,
                                content: snap.text !== undefined ? snap.text : m.content,
                                reasoning: snap.reasoning !== undefined ? snap.reasoning : m.reasoning,
                                toolCalls: snap.toolCalls?.length ? snap.toolCalls : m.toolCalls,
                            }));
                            if (snap.pendingPermission !== undefined) pendingPermission = snap.pendingPermission;
                            if (snap.pendingQuestion !== undefined) pendingQuestion = snap.pendingQuestion;
                            void scrollToBottom(false);
                        },
                        onTextDelta: delta => {
                            if (serial !== generation) return;
                            updateObserved(m => ({ ...m, content: m.content + delta }));
                            void scrollToBottom(false);
                        },
                        onReasoningDelta: delta => {
                            if (serial !== generation) return;
                            updateObserved(m => ({ ...m, reasoning: (m.reasoning ?? '') + delta }));
                        },
                        onToolStarted: (callId, name, args) => {
                            if (serial !== generation) return;
                            updateObserved(m => ({
                                ...m,
                                toolCalls: [...(m.toolCalls ?? []), { callId, name, arguments: args, status: 'running' }],
                            }));
                        },
                        onToolFinished: (callId, name, ok, summary) => {
                            if (serial !== generation) return;
                            updateObserved(m => {
                                const list = m.toolCalls ?? [];
                                const idx = list.findIndex(tc => tc.callId === callId);
                                const next = {
                                    callId,
                                    name,
                                    status: ok ? ('completed' as const) : ('failed' as const),
                                    summary,
                                };
                                if (idx >= 0) {
                                    const copy = [...list];
                                    copy[idx] = { ...copy[idx], ...next };
                                    return { ...m, toolCalls: copy };
                                }
                                return { ...m, toolCalls: [...list, next] };
                            });
                        },
                        onPermissionRequested: prompt => {
                            if (serial !== generation) return;
                            pendingPermission = prompt;
                        },
                        onUserQuestion: question => {
                            if (serial !== generation) return;
                            pendingQuestion = question;
                        },
                        onTurnCompleted: (_finalText, turnUsage) => {
                            if (serial !== generation) return;
                            if (turnUsage) sessionUsage = mergeV2TokenUsage(sessionUsage, turnUsage);
                        },
                        onError: msg => {
                            if (serial !== generation) return;
                            error = msg;
                        },
                    },
                    controller.signal,
                );
                if (serial === generation && active?.conversationId === convId) {
                    await reloadActiveConversation(convId, serial).catch(() => {});
                }
            } finally {
                if (streamController === controller) streamController = null;
                if (serial === generation) {
                    streaming = false;
                    reconnecting = false;
                    statusNotice = '';
                }
            }
        } catch {
            // Optional active turn reconnection if host does not report an active run.
            if (serial === generation) reconnecting = false;
        }
    }

    function detectExpertMentionTrigger() {
        if (!composerEl) return;
        const cursor = composerEl.selectionStart ?? draft.length;
        const before = draft.slice(0, cursor);
        const match = before.match(/(?:^|\s)@([^\s@]{0,40})$/);
        if (match) {
            const query = match[1];
            const atPos = cursor - query.length - 1;
            expertTriggerRange = { start: atPos, end: cursor };
            expertQuery = query;
            expertHighlightIdx = 0;
            expertPickerOpen = true;
            void ensureExpertsLoaded();
        } else if (expertTriggerRange !== null) {
            expertPickerOpen = false;
            expertTriggerRange = null;
            expertQuery = '';
        }
    }

    function closeExpertPicker(restoreFocus = false) {
        expertPickerOpen = false;
        expertTriggerRange = null;
        expertQuery = '';
        expertHighlightIdx = 0;
        if (restoreFocus) composerEl?.focus();
    }

    function scrollHighlightedExpertIntoView() {
        void tick().then(() => {
            expertPopupEl
                ?.querySelectorAll<HTMLElement>('.expert-option')
                [expertHighlightIdx]?.scrollIntoView({ block: 'nearest' });
        });
    }

    function toggleExpertPicker() {
        if (expertPickerOpen) {
            closeExpertPicker();
            return;
        }
        expertTriggerRange = null;
        expertQuery = '';
        expertHighlightIdx = 0;
        expertPickerOpen = true;
        void ensureExpertsLoaded();
    }

    function selectExpert(expert: V2Expert) {
        if (!selectedExperts.some(e => e.id === expert.id)) {
            selectedExperts = [...selectedExperts, expert];
        }
        if (expertTriggerRange) {
            const { start, end } = expertTriggerRange;
            const before = draft.slice(0, start);
            const after = draft.slice(end);
            const cleaned = `${before}${after}`.replace(/  +/g, ' ');
            draft = cleaned;
            saveDraftForKey(currentDraftKey(), draft);
        }
        expertPickerOpen = false;
        expertTriggerRange = null;
        expertQuery = '';
        expertHighlightIdx = 0;
        composerEl?.focus();
    }

    function removeSelectedExpert(id: string) {
        selectedExperts = selectedExperts.filter(e => e.id !== id);
    }

    async function handlePermissionModeChange(nextMode: V2PermissionProfile) {
        const normalized = normalizeV2PermissionProfile(nextMode);
        permissionProfile = normalized;
        if (!active) return;
        const targetId = active.conversationId;
        permissionSaving = true;
        try {
            await patchV2ConversationConfiguration(targetId, { permissionProfile: normalized });
            if (active?.conversationId === targetId) {
                pushToast(
                    streaming
                        ? t($locale, 'chatPermissionMode.active')
                        : t($locale, 'chatPermissionMode.saved'),
                    'info',
                );
            }
        } catch (cause) {
            if (active?.conversationId === targetId) {
                error = errorMessage(cause);
            }
        } finally {
            permissionSaving = false;
        }
    }

    function resolveModelSource(modelId: string): string {
        if (!modelId) return '';
        const choice = modelChoices.find(m => m.id === modelId);
        if (!choice) return '';
        if (choice.source === 'local') return 'local';
        return choice.endpointId || '';
    }

    async function handleModelChange(nextModelId: string) {
        selectedModelId = nextModelId;
        const nextModelSource = resolveModelSource(nextModelId);
        if (!active) return;
        const targetId = active.conversationId;
        try {
            await patchV2ConversationConfiguration(targetId, {
                selectedModelId: nextModelId,
                selectedModelSource: nextModelSource,
            });
            if (active?.conversationId === targetId) {
                active = {
                    ...active,
                    selectedModelId: nextModelId,
                    selectedModelSource: nextModelSource,
                };
            }
        } catch (cause) {
            if (active?.conversationId === targetId) {
                error = errorMessage(cause);
            }
        }
    }

    async function handleReasoningEffortChange(nextEffort: V2ReasoningEffort) {
        const normalized = normalizeV2ReasoningEffort(nextEffort);
        reasoningEffort = normalized;
        if (!active) return;
        const targetId = active.conversationId;
        try {
            await patchV2ConversationConfiguration(targetId, { reasoningEffort: normalized });
        } catch (cause) {
            if (active?.conversationId === targetId) {
                error = errorMessage(cause);
            }
        }
    }

    async function openConversation(conv: ChatConversation, preserveTab = false) {
        // Persist draft for the outgoing conversation, then detach SSE WITHOUT killing Host runs.
        saveDraftForKey(currentDraftKey(), draft);
        if (streaming) detachStream();
        const serial = ++generation;
        active = conv;
        draft = conversationDrafts.get(conv.conversationId) ?? '';
        error = '';
        statusNotice = '';
        needsModelConfig = false;
        pendingPermission = null;
        permissionSubmitting = false;
        pendingQuestion = null;
        confirmEditIndex = null;
        expertPickerOpen = false;
        expertTriggerRange = null;
        isPinnedToBottom = true;
        sessionUsage = emptyV2TokenUsage();
        if (conv.selectedModelId) selectedModelId = conv.selectedModelId;
        if (!preserveTab) {
            contentTabs = selectContentTab(contentTabs, conversationTabKey);
            syncUrl(conv.conversationId, null);
        } else {
            syncUrl(conv.conversationId);
        }
        loading = true;
        conversationPlan = { enabled: false, status: 'idle', steps: [] };
        attachedRoom = availableRooms.find(r => r.sessionId === conv.conversationId) ?? null;
        try {
            const [loaded, cfgResult, planResult] = await Promise.allSettled([
                loadV2MessagesWithUsage(conv.conversationId),
                getV2ConversationConfiguration(conv.conversationId),
                getV2ConversationPlan(conv.conversationId),
            ]);
            if (serial !== generation) return;
            if (loaded.status === 'fulfilled') {
                messages = loaded.value.messages;
                sessionUsage = loaded.value.usage;
            } else {
                error = errorMessage(loaded.reason);
            }
            if (cfgResult.status === 'fulfilled') {
                permissionProfile = normalizeV2PermissionProfile(cfgResult.value.permissionProfile);
                reasoningEffort = normalizeV2ReasoningEffort(cfgResult.value.reasoningEffort);
                if (cfgResult.value.selectedModelId) {
                    selectedModelId = cfgResult.value.selectedModelId;
                }
            }
            if (planResult.status === 'fulfilled') {
                conversationPlan = planResult.value;
            }
            await scrollToBottom(true);
            void reconnectActiveTurn(conv.conversationId, serial);
        } catch (cause) {
            if (serial !== generation) return;
            error = errorMessage(cause);
        } finally {
            if (serial === generation) loading = false;
        }
    }

    function isModelsTabPath(path: string): boolean {
        return path === '/models' || path.startsWith('/models?') || path.startsWith('/models#') || path.startsWith('/models/');
    }

    function openManagementTab(path: string) {
        if (viewportWidth <= 760) {
            navigateWorkspace(path);
            return;
        }
        const origin = typeof window === 'undefined' ? 'http://nola.invalid' : window.location.origin;
        const resolved = managementContentPath(path, origin);
        const resolvedPath = resolved?.path ?? path;
        contentTabs = openPageContentTab(
            contentTabs,
            resolvedPath,
            resolved?.titleKey ?? 'shell.settings',
        );
        syncUrl(active?.conversationId ?? null, resolvedPath);
    }

    function handleSelectContentTab(key: string) {
        const prevTab = selectedContentTab;
        contentTabs = selectContentTab(contentTabs, key);
        const sel = contentTabs.items.find(item => item.key === contentTabs.selected);
        syncUrl(active?.conversationId ?? null, sel?.kind === 'page' ? sel.path : null);
        if (key === conversationTabKey && prevTab?.kind === 'page' && isModelsTabPath(prevTab.path)) {
            void refreshModelChoices();
        }
    }

    async function handleCloseContentTab(key: string) {
        const closingTab = contentTabs.items.find(item => item.key === key);
        if (closingTab?.kind === 'page' && typeof window !== 'undefined') {
            const unloadEvent = workspaceTabCloseEvent(key);
            window.dispatchEvent(unloadEvent);
            if (unloadEvent.defaultPrevented) {
                const confirmed = confirmation
                    ? await confirmation.ask({
                          title: t($locale, 'shell.confirm'),
                          description: t($locale, 'shell.navigationDiscard'),
                          confirmLabel: t($locale, 'shell.confirm'),
                          destructive: true,
                      })
                    : false;
                if (!confirmed) return;
            }
        }
        const wasModelsTab = closingTab?.kind === 'page' && isModelsTabPath(closingTab.path);
        contentTabs = closeContentTab(contentTabs, key);
        tabViewports.delete(key);
        const sel = contentTabs.items.find(item => item.key === contentTabs.selected);
        syncUrl(active?.conversationId ?? null, sel?.kind === 'page' ? sel.path : null);
        if (wasModelsTab && contentTabs.selected === conversationTabKey) {
            void refreshModelChoices();
        }
    }

    const MAX_NON_IMAGE_ATTACHMENT_BYTES = 10 * 1024 * 1024;
    const TEXT_FILE_EXTENSIONS = new Set([
        'txt', 'md', 'markdown', 'json', 'yaml', 'yml', 'toml', 'xml', 'csv', 'tsv',
        'html', 'htm', 'css', 'scss', 'js', 'mjs', 'cjs', 'ts', 'tsx', 'jsx', 'svelte', 'vue',
        'py', 'rs', 'go', 'java', 'kt', 'c', 'h', 'cpp', 'hpp', 'cs', 'rb', 'php',
        'sh', 'bash', 'zsh', 'sql', 'env', 'ini', 'conf', 'cfg', 'log', 'svg', 'gitignore',
    ]);

    function isLikelyTextAttachment(file: File): boolean {
        const mime = (file.type || '').toLowerCase();
        if (mime.startsWith('text/')) return true;
        if (
            mime === 'application/json' ||
            mime === 'application/xml' ||
            mime === 'application/javascript' ||
            mime === 'application/typescript' ||
            mime === 'application/x-yaml' ||
            mime === 'application/yaml' ||
            mime === 'application/toml' ||
            mime === 'application/sql' ||
            mime === 'application/x-sh' ||
            mime === 'application/xhtml+xml' ||
            mime.endsWith('+json') ||
            mime.endsWith('+xml')
        ) {
            return true;
        }
        if (mime && mime !== 'application/octet-stream') return false;
        const ext = file.name.split('.').pop()?.toLowerCase() ?? '';
        return TEXT_FILE_EXTENSIONS.has(ext);
    }

    function bytesToBase64(bytes: Uint8Array): string {
        let binary = '';
        const chunk = 0x8000;
        for (let i = 0; i < bytes.length; i += chunk) {
            binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
        }
        return btoa(binary);
    }

    async function ensureActiveConversation(): Promise<ChatConversation> {
        if (active) return active;
        const created = await createV2Conversation({
            ...(selectedModelId
                ? {
                      selectedModelId,
                      selectedModelSource: resolveModelSource(selectedModelId),
                  }
                : {}),
            permissionProfile,
            reasoningEffort,
        });
        active = created;
        syncUrl(created.conversationId);
        void conversationManager?.refresh();
        return created;
    }

    async function reloadActiveConversation(conversationId: string, serial = generation): Promise<void> {
        const canonical = await loadV2MessagesWithUsage(conversationId);
        if (serial !== generation || active?.conversationId !== conversationId) return;
        messages = canonical.messages;
        sessionUsage = canonical.usage;
    }

    async function addAttachmentFiles(files: FileList | File[]) {
        const list = Array.from(files);
        if (!list.length) return;
        error = '';
        const next: V2Attachment[] = [...attachments];
        const rejected: string[] = [];
        for (const file of list) {
            if (file.type.startsWith('image/')) {
                const dataUrl = await new Promise<string>((resolve, reject) => {
                    const reader = new FileReader();
                    reader.onload = () => resolve(String(reader.result ?? ''));
                    reader.onerror = () => reject(reader.error);
                    reader.readAsDataURL(file);
                });
                next.push({ name: file.name || 'image', mimeType: file.type, dataUrl });
                continue;
            }
            if (file.size > MAX_NON_IMAGE_ATTACHMENT_BYTES) {
                rejected.push(file.name);
                continue;
            }
            const mimeType = file.type || 'application/octet-stream';
            const bytes = new Uint8Array(await file.arrayBuffer());
            if (isLikelyTextAttachment(file) && !bytes.includes(0)) {
                try {
                    const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
                    next.push({
                        name: file.name,
                        mimeType: file.type || 'text/plain',
                        text: text.slice(0, 64_000),
                    });
                    continue;
                } catch {
                    // Fall through to binary parser if bytes are not valid UTF-8.
                }
            }
            const fallbackLabel = `[Binary attachment '${file.name}' (${mimeType}, ${file.size} bytes)]`;
            let parsedText = fallbackLabel;
            try {
                const parsed = await parseV2Attachment({
                    filename: file.name,
                    mimeType,
                    dataBase64: bytesToBase64(bytes),
                });
                if (parsed.text) parsedText = parsed.text.slice(0, 64_000);
            } catch {
                // Keep structured binary marker without corrupting bytes via file.text().
            }
            next.push({ name: file.name, mimeType, text: parsedText });
        }
        attachments = next;
        if (rejected.length) {
            error = `${t($locale, 'chatTextAttachment.invalidFile')} (${rejected.join(', ')})`;
        }
    }

    async function handleFileSelect(event: Event) {
        const input = event.currentTarget as HTMLInputElement;
        const files = input.files;
        if (!files?.length) return;
        await addAttachmentFiles(files);
        input.value = '';
    }

    function handleComposerDragOver(event: DragEvent) {
        if (streaming) return;
        if (!event.dataTransfer?.types.includes('Files')) return;
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
        isDragOver = true;
    }

    function handleComposerDragLeave(event: DragEvent) {
        const current = event.currentTarget as HTMLElement | null;
        const related = event.relatedTarget as Node | null;
        if (current && related && current.contains(related)) return;
        isDragOver = false;
    }

    async function handleComposerDrop(event: DragEvent) {
        isDragOver = false;
        if (streaming) return;
        const files = event.dataTransfer?.files;
        if (!files?.length) return;
        event.preventDefault();
        await addAttachmentFiles(files);
    }

    async function handleComposerPaste(event: ClipboardEvent) {
        if (streaming) return;
        const items = event.clipboardData?.items;
        const files: File[] = [];
        if (items) {
            for (const item of Array.from(items)) {
                if (item.kind === 'file') {
                    const file = item.getAsFile();
                    if (file) files.push(file);
                }
            }
        }
        if (!files.length && event.clipboardData?.files?.length) {
            files.push(...Array.from(event.clipboardData.files));
        }
        if (!files.length) return;
        event.preventDefault();
        await addAttachmentFiles(files);
    }

    function removeAttachment(index: number) {
        attachments = attachments.filter((_, idx) => idx !== index);
    }

    async function sendPrompt(
        rawText: string,
        attached: V2Attachment[] = [],
        uiAction?: V2UiActionPayload,
    ) {
        const text = rawText.trim();
        if ((!text && !attached.length && !uiAction) || streaming) return;
        error = '';
        statusNotice = '';
        needsModelConfig = false;
        confirmEditIndex = null;
        expertPickerOpen = false;
        expertTriggerRange = null;

        // Lightweight slash commands
        if (text === '/compact' && active) {
            const convId = active.conversationId;
            const serial = generation;
            try {
                const res = await compactV2Conversation(convId);
                statusNotice = res.summary || t($locale, 'chatPage.compactionRecord');
                draft = '';
                conversationDrafts.delete(convId);
                await reloadActiveConversation(convId, serial).catch(() => {});
            } catch (cause) {
                error = errorMessage(cause);
            }
            return;
        }
        if (text === '/clear') {
            draft = '';
            conversationDrafts.delete(currentDraftKey());
            const fresh = await createV2Conversation();
            await openConversation(fresh);
            void conversationManager?.refresh();
            return;
        }

        try {
            const modelReady = await hasConfiguredChatModel();
            if (!modelReady) {
                needsModelConfig = true;
                return;
            }
        } catch {
            // Let the host decide if model catalog check fails transiently.
        }

        let conv: ChatConversation;
        try {
            conv = await ensureActiveConversation();
        } catch (cause) {
            error = errorMessage(cause);
            return;
        }

        const sentText = text;
        const sentAttachments = [...attached];
        const sentExperts = [...selectedExperts];
        draft = '';
        conversationDrafts.delete(NEW_DRAFT_KEY);
        conversationDrafts.delete(conv.conversationId);
        attachments = [];
        selectedExperts = [];
        streaming = true;
        pendingPermission = null;
        permissionSubmitting = false;
        pendingQuestion = null;

        const userMsg: V2Message = {
            id: `u-${Date.now()}`,
            role: 'user',
            content: sentText,
            attachments: sentAttachments.length ? sentAttachments : undefined,
            createdAtMs: Date.now(),
        };
        const assistantId = `a-${Date.now()}`;
        const assistantMsg: V2Message = {
            id: assistantId,
            role: 'assistant',
            content: '',
            reasoning: '',
            toolCalls: [],
            uiSurfaces: [],
            createdAtMs: Date.now(),
        };
        messages = [...messages, userMsg, assistantMsg];
        await scrollToBottom(true);

        const controller = new AbortController();
        streamController = controller;
        const serial = generation;

        const updateAssistant = (fn: (m: V2Message) => V2Message) => {
            messages = messages.map(m => (m.id === assistantId ? fn(m) : m));
        };

        let needsTextBreakAfterTool = false;

        try {
            await streamV2ChatMessage(
                conv.conversationId,
                sentText,
                {
                    onTextDelta: delta => {
                        if (serial !== generation || !delta) return;
                        updateAssistant(m => {
                            let prefix = '';
                            if (needsTextBreakAfterTool) {
                                needsTextBreakAfterTool = false;
                                if (m.content.trim().length > 0 && !m.content.endsWith('\n\n') && !delta.startsWith('\n')) {
                                    prefix = m.content.endsWith('\n') ? '\n' : '\n\n';
                                }
                            }
                            return { ...m, content: m.content + prefix + delta };
                        });
                        void scrollToBottom(false);
                    },
                    onReasoningDelta: delta => {
                        if (serial !== generation) return;
                        updateAssistant(m => ({ ...m, reasoning: (m.reasoning ?? '') + delta }));
                        void scrollToBottom(false);
                    },
                    onToolStarted: (callId, name, args) => {
                        if (serial !== generation) return;
                        needsTextBreakAfterTool = true;
                        updateAssistant(m => ({
                            ...m,
                            toolCalls: [
                                ...(m.toolCalls ?? []),
                                { callId, name, arguments: args, status: 'running' },
                            ],
                        }));
                        void scrollToBottom(false);
                    },
                    onToolFinished: (callId, name, ok, summary) => {
                        if (serial !== generation) return;
                        updateAssistant(m => {
                            const list = m.toolCalls ?? [];
                            const idx = list.findIndex(tc => tc.callId === callId);
                            const next = {
                                callId,
                                name,
                                status: ok ? ('completed' as const) : ('failed' as const),
                                summary,
                            };
                            if (idx >= 0) {
                                const copy = [...list];
                                copy[idx] = { ...copy[idx], ...next };
                                return { ...m, toolCalls: copy };
                            }
                            return { ...m, toolCalls: [...list, next] };
                        });
                    },
                    onUiSurface: snapshot => {
                        if (serial !== generation) return;
                        updateAssistant(m => {
                            const existing = m.uiSurfaces ?? [];
                            const idx = existing.findIndex(
                                s => s.identity.surface_id === snapshot.identity.surface_id,
                            );
                            const next = idx >= 0
                                ? existing.map((s, i) => (i === idx ? snapshot : s))
                                : [...existing, snapshot];
                            return { ...m, uiSurfaces: next };
                        });
                        void scrollToBottom(false);
                    },
                    onPermissionRequested: prompt => {
                        if (serial !== generation) return;
                        pendingPermission = prompt;
                        permissionSubmitting = false;
                        void scrollToBottom(false);
                    },
                    onUserQuestion: question => {
                        if (serial !== generation) return;
                        pendingQuestion = question;
                        questionAnswerDraft = '';
                        void scrollToBottom(false);
                    },
                    onCompacted: summary => {
                        if (serial !== generation) return;
                        if (summary) statusNotice = summary;
                    },
                    onExpertReply: reply => {
                        if (serial !== generation) return;
                        updateAssistant(m => ({
                            ...m,
                            expertReplies: [...(m.expertReplies ?? []), reply],
                        }));
                        void scrollToBottom(false);
                    },
                    onTurnCompleted: (finalText, turnUsage) => {
                        if (serial !== generation) return;
                        if (finalText) {
                            updateAssistant(m => {
                                if (!m.content) return { ...m, content: finalText };
                                if (needsTextBreakAfterTool && finalText.trim() && !m.content.includes(finalText.trim())) {
                                    needsTextBreakAfterTool = false;
                                    const sep = m.content.endsWith('\n\n') ? '' : m.content.endsWith('\n') ? '\n' : '\n\n';
                                    return { ...m, content: m.content + sep + finalText };
                                }
                                return m;
                            });
                        }
                        if (turnUsage) {
                            sessionUsage = mergeV2TokenUsage(sessionUsage, turnUsage);
                        }
                    },
                    onError: msg => {
                        if (serial !== generation) return;
                        if (msg.includes('model_not_configured') || msg.includes('no_model_configured')) {
                            needsModelConfig = true;
                        }
                        error = msg;
                    },
                },
                controller.signal,
                {
                    modelId: selectedModelId || undefined,
                    modelSource: selectedModelId ? resolveModelSource(selectedModelId) : undefined,
                    permissionProfile,
                    reasoningEffort: reasoningEffort === 'automatic' ? undefined : reasoningEffort,
                    attachments: sentAttachments,
                    selectedExperts: sentExperts.length ? sentExperts.map(e => e.id) : undefined,
                    uiAction,
                },
            );
            if (serial === generation) {
                void conversationManager?.refresh();
                // Await canonical messages so persisted seq numbers and session usage are ready before Edit/Regenerate.
                try {
                    const canonical = await loadV2MessagesWithUsage(conv.conversationId);
                    if (serial === generation) {
                        if (canonical.messages.length) {
                            messages = canonical.messages;
                        }
                        sessionUsage = canonical.usage;
                    }
                } catch {
                    // Keep streamed transcript if canonical reload fails transiently.
                }
            }
        } catch (cause) {
            if (serial !== generation) return;
            if (!controller.signal.aborted) {
                const msg = errorMessage(cause);
                const missingModel =
                    (cause instanceof ApiError &&
                        (cause.code === 'model_not_configured' || cause.code === 'no_model_configured')) ||
                    msg.includes('model_not_configured') ||
                    msg.includes('no_model_configured');
                if (missingModel) {
                    needsModelConfig = true;
                    draft = sentText;
                    saveDraftForKey(conv.conversationId, sentText);
                    selectedExperts = sentExperts;
                    messages = messages.filter(m => m.id !== userMsg.id && m.id !== assistantId);
                } else {
                    error = msg;
                }
            }
        } finally {
            if (streamController === controller) streamController = null;
            if (serial === generation) {
                streaming = false;
                pendingPermission = null;
                permissionSubmitting = false;
                pendingQuestion = null;
            }
        }
    }

    async function send() {
        await sendPrompt(draft, attachments);
    }

    async function handlePermissionDecision(allow: boolean, scope?: V2PermissionScope) {
        const prompt = pendingPermission;
        if (!prompt || !active || permissionSubmitting) return;
        permissionSubmitting = true;
        error = '';
        try {
            await respondV2Permission(active.conversationId, prompt.requestId, allow, scope);
            if (pendingPermission?.requestId === prompt.requestId) {
                pendingPermission = null;
            }
        } catch (cause) {
            error = errorMessage(cause);
        } finally {
            permissionSubmitting = false;
        }
    }

    async function handleQuestionAnswer(answer: string) {
        const q = pendingQuestion;
        if (!q || !active) return;
        pendingQuestion = null;
        questionAnswerDraft = '';
        try {
            await respondV2UserInput(active.conversationId, q.requestId, answer);
        } catch (cause) {
            error = errorMessage(cause);
        }
    }

    function replaceSurfaceSnapshot(nextSnap: UiSnapshot) {
        messages = messages.map(m => {
            if (!m.uiSurfaces?.length) return m;
            const hasMatch = m.uiSurfaces.some(
                s => s.identity.surface_id === nextSnap.identity.surface_id,
            );
            if (!hasMatch) return m;
            return {
                ...m,
                uiSurfaces: m.uiSurfaces.map(s =>
                    s.identity.surface_id === nextSnap.identity.surface_id ? nextSnap : s,
                ),
            };
        });
    }

    async function handleSurfaceInteraction(
        interaction: UiInteraction,
        requestId: string,
        submittedAtMs: number,
    ): Promise<UiSubmissionOutcome> {
        if (!active) return { status: 'rejected', message: t($locale, 'chatPage.noSession') };
        const result = await submitV2UiInteraction(active.conversationId, interaction, requestId, submittedAtMs);
        if (result.snapshot) {
            replaceSurfaceSnapshot(result.snapshot);
        }
        if (result.outcome.status === 'committed' && result.actionId) {
            const summary = result.summary || result.actionId;
            const uiAction: V2UiActionPayload = {
                actionId: result.actionId,
                surfaceId: interaction.identity.surface_id,
                dataModel: result.dataModel ?? {},
                summary,
            };
            // Stream the agent's follow-up reaction with structured ui_action metadata
            void sendPrompt(summary, [], uiAction);
        }
        return result.outcome;
    }

    async function copyMessage(msg: V2Message) {
        try {
            await navigator.clipboard?.writeText(msg.content);
            const prevTimer = copyTimers.get(msg.id);
            if (prevTimer !== undefined) clearTimeout(prevTimer);
            copiedMessageIds = { ...copiedMessageIds, [msg.id]: true };
            const timer = setTimeout(() => {
                copyTimers.delete(msg.id);
                const next = { ...copiedMessageIds };
                delete next[msg.id];
                copiedMessageIds = next;
            }, 1800);
            copyTimers.set(msg.id, timer);
        } catch {
            // Ignore clipboard errors in restricted contexts.
        }
    }

    function requestEditAndResend(index: number) {
        if (streaming || !active) return;
        confirmEditIndex = confirmEditIndex === index ? null : index;
    }

    async function confirmEditAndResend(msg: V2Message, index: number) {
        if (streaming || !active) return;
        confirmEditIndex = null;
        const convId = active.conversationId;
        const serial = generation;
        try {
            let targetSeq = msg.seq;
            if (typeof targetSeq !== 'number') {
                const canonical = await loadV2MessagesWithUsage(convId);
                if (serial !== generation || active?.conversationId !== convId) return;
                targetSeq = canonical.messages[index]?.seq;
            }
            if (typeof targetSeq === 'number') {
                await truncateV2Conversation(convId, targetSeq);
            }
            try {
                await reloadActiveConversation(convId, serial);
            } catch {
                messages = messages.slice(0, index);
            }
            draft = msg.content;
            attachments = msg.attachments?.length ? msg.attachments.map(att => ({ ...att })) : [];
            saveDraftForKey(convId, draft);
            await tick();
            composerEl?.focus();
        } catch (cause) {
            error = errorMessage(cause);
        }
    }

    async function regenerateFrom(index: number) {
        if (streaming || !active) return;
        let userIdx = index - 1;
        while (userIdx >= 0 && messages[userIdx].role !== 'user') userIdx--;
        if (userIdx < 0) return;
        const userMsg = messages[userIdx];
        const convId = active.conversationId;
        const serial = generation;
        try {
            let targetSeq = userMsg.seq;
            if (typeof targetSeq !== 'number') {
                const canonical = await loadV2MessagesWithUsage(convId);
                if (serial !== generation || active?.conversationId !== convId) return;
                targetSeq = canonical.messages[userIdx]?.seq;
            }
            if (typeof targetSeq === 'number') {
                await truncateV2Conversation(convId, targetSeq);
            }
            try {
                await reloadActiveConversation(convId, serial);
            } catch {
                messages = messages.slice(0, userIdx);
            }
            await sendPrompt(userMsg.content, userMsg.attachments ?? []);
        } catch (cause) {
            error = errorMessage(cause);
        }
    }

    async function branchActiveConversation() {
        if (!active || streaming) return;
        try {
            const branched = await branchV2Conversation(active.conversationId);
            await openConversation(branched);
            void conversationManager?.refresh();
        } catch (cause) {
            error = errorMessage(cause);
        }
    }

    function onComposerKeydown(event: KeyboardEvent) {
        if (expertPickerOpen) {
            if (event.key === 'Escape') {
                event.preventDefault();
                closeExpertPicker();
                return;
            }
            if (event.key === 'ArrowDown') {
                event.preventDefault();
                if (filteredExperts.length > 0) {
                    expertHighlightIdx = (expertHighlightIdx + 1) % filteredExperts.length;
                    scrollHighlightedExpertIntoView();
                }
                return;
            }
            if (event.key === 'ArrowUp') {
                event.preventDefault();
                if (filteredExperts.length > 0) {
                    expertHighlightIdx = (expertHighlightIdx - 1 + filteredExperts.length) % filteredExperts.length;
                    scrollHighlightedExpertIntoView();
                }
                return;
            }
            if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && filteredExperts.length > 0) {
                event.preventDefault();
                const target = filteredExperts[expertHighlightIdx] ?? filteredExperts[0];
                if (target) selectExpert(target);
                return;
            }
        }
        if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
            event.preventDefault();
            void send();
        }
    }

    function setListCategory(category: 'conversations' | 'resources') {
        if (viewportWidth > 760) {
            sidebarOpen = category === listCategory ? !sidebarOpen : true;
        }
        listCategory = category;
    }

    onMount(() => {
        const handleContentNav = (event: Event) => {
            if (!(event instanceof CustomEvent) || typeof event.detail !== 'string') return;
            const target = new URL(event.detail, window.location.origin);
            if (target.pathname !== '/chat') {
                event.preventDefault();
                openManagementTab(target.pathname + target.search + target.hash);
            }
        };
        const handleModelsUpdated = () => {
            void refreshModelChoices();
        };
        window.addEventListener(workspaceContentNavigationEvent, handleContentNav);
        window.addEventListener('nola:models-updated', handleModelsUpdated);
        const stopColorObserver = observeExpertBubbleColors(() => {
            expertColorVersion++;
        });
        void refreshModelChoices();
        void ensureExpertsLoaded();

        void (async () => {
            try {
                const url = new URL(window.location.href);
                const initialContent = url.searchParams.get('content');
                if (initialContent) {
                    openManagementTab(initialContent);
                }
                const requestedId = url.searchParams.get('conversation');
                if (requestedId) {
                    const conv = await getV2Conversation(requestedId);
                    await openConversation(conv, Boolean(initialContent));
                } else {
                    const existing = await listV2Conversations();
                    if (existing.length > 0) {
                        await openConversation(existing[0], Boolean(initialContent));
                    }
                }
            } catch {
                // Start cleanly with an empty workspace when no conversation exists yet.
            }
        })();

        return () => {
            window.removeEventListener(workspaceContentNavigationEvent, handleContentNav);
            window.removeEventListener('nola:models-updated', handleModelsUpdated);
            stopColorObserver();
            // Detach SSE reader on unmount WITHOUT killing the Host run!
            detachStream();
            for (const timer of copyTimers.values()) clearTimeout(timer);
            copyTimers.clear();
        };
    });
</script>

<svelte:window
    bind:innerWidth={viewportWidth}
    onclick={event => {
        if (!expertPickerOpen) return;
        const target = event.target;
        if (
            target instanceof Node &&
            !expertPopupEl?.contains(target) &&
            !expertToggleBtnEl?.contains(target) &&
            !composerEl?.contains(target)
        ) {
            closeExpertPicker();
        }
    }}
    onkeydown={event => {
        if (expertPickerOpen && event.key === 'Escape') {
            event.preventDefault();
            closeExpertPicker(true);
        }
    }}
/>
<svelte:head><title>{activeTabTitle} — Nola</title></svelte:head>
<ConfirmationDialog bind:this={confirmation} />
<ExpertRoomManager
    open={expertManagerOpen}
    conversationId={active?.conversationId ?? null}
    attachedRoomId={attachedRoom?.id ?? null}
    onclose={() => (expertManagerOpen = false)}
    onupdated={(nextExperts, nextRooms) => {
        availableExperts = nextExperts;
        availableRooms = nextRooms;
        expertsLoaded = true;
    }}
    onattachRoom={room => {
        attachedRoom = room;
    }}
/>

<div
    class="chat-shell"
    class:sidebar-collapsed={!sidebarOpen}
    style:--sidebar-width="{panelLayout.sidebar}px"
>
    <aside id="chat-sidebar-panel" class="chat-sidebar" aria-label={t($locale, 'chatInspector.list.conversations')}>
        <div class="sidebar-top">
            <WorkspaceListNavigation category={listCategory} visible={sidebarOpen} collapsed={!sidebarOpen} onchange={setListCategory} />
            <div class="sidebar-tools">
                <button
                    type="button"
                    class="icon-btn sidebar-toggle"
                    title={t($locale, 'shell.toggleSidebar')}
                    aria-label={t($locale, 'shell.toggleSidebar')}
                    aria-expanded={sidebarOpen}
                    onclick={() => (sidebarOpen = !sidebarOpen)}
                >
                    <PrototypeIcon name="sidebar" size={16} />
                </button>
                <button
                    type="button"
                    class="icon-btn"
                    title={t($locale, 'shell.toggleAppearance')}
                    aria-label={t($locale, 'shell.toggleAppearance')}
                    onclick={toggleAppearance}
                >
                    <PrototypeIcon name={$darkAppearance ? 'sun' : 'moon'} size={16} />
                </button>
            </div>
        </div>

        {#if sidebarOpen}
            <div class="sidebar-body">
                {#if listCategory === 'conversations'}
                    <ChatConversationManager
                        bind:this={conversationManager}
                        layout="sidebar"
                        activeId={active?.conversationId ?? null}
                        {generation}
                        busy={streaming}
                        hasMessages={messages.length > 0}
                        onselect={conv => void openConversation(conv)}
                        onchange={ev => {
                            if (ev.type === 'deleted' && ev.conversationId === active?.conversationId) {
                                detachStream();
                                conversationDrafts.delete(ev.conversationId);
                                active = null;
                                messages = [];
                                sessionUsage = emptyV2TokenUsage();
                                draft = conversationDrafts.get(NEW_DRAFT_KEY) ?? '';
                                syncUrl(null);
                            } else if ((ev.type === 'renamed' || ev.type === 'refreshed') && ev.conversation && ev.conversationId === active?.conversationId) {
                                active = ev.conversation;
                            }
                        }}
                    />
                {:else}
                    <WorkspaceResourceNavigation
                        selectedPath={selectedContentTab?.kind === 'page' ? selectedContentTab.path : ''}
                        onopen={openManagementTab}
                    />
                {/if}
            </div>
        {/if}

        <DesktopPluginNavigation
            selectedPath={selectedContentTab?.kind === 'page' ? selectedContentTab.path : ''}
            compact={!sidebarOpen}
            onopen={openManagementTab}
        />

        <footer class="sidebar-footer">
            <button
                type="button"
                class="sidebar-action sidebar-footer-item"
                title={t($locale, 'shell.settings')}
                aria-label={t($locale, 'shell.settings')}
                onclick={() => openManagementTab('/settings?section=general')}
            >
                <PrototypeIcon name="settings" size={16} />
                {#if sidebarOpen}
                    <span>{t($locale, 'shell.settings')}</span>
                {/if}
            </button>
        </footer>
        {#if sidebarOpen && viewportWidth > 760}
            <PanelResizeHandle
                label={t($locale, 'shell.resizeSidebar')}
                controls="chat-sidebar-panel"
                value={panelLayout.sidebar}
                min={220}
                max={panelLayout.sidebarMax}
                defaultValue={layoutDefaults.sidebar}
                onchange={next => setWorkspaceWidth('sidebar', next)}
            />
        {/if}
    </aside>

    <main id="page-main" tabindex="-1" class="chat-main">
        <header class="workspace-topbar">
            {#if viewportWidth <= 760}
                <button
                    type="button"
                    class="icon-btn"
                    aria-label={t($locale, 'chatPage.openConversations')}
                    onclick={() => navigateWorkspace('/conversations')}
                >
                    <PrototypeIcon name="back" size={18} />
                </button>
            {/if}
            <ConversationContentTabsView
                state={contentTabs}
                title={activeTabTitle}
                {ownerKey}
                conversationId={active?.conversationId ?? null}
                canRename={!!active && !streaming}
                onselect={handleSelectContentTab}
                onclose={handleCloseContentTab}
                onrename={(id, nextTitle) => conversationManager?.renameConversationById(id, nextTitle)}
            />
            {#if isConversationTabActive}
                <div class="topbar-actions">
                    <button
                        type="button"
                        class="topbar-pill topbar-icon-pill"
                        class:active={Boolean(attachedRoom)}
                        title={t($locale, 'rooms.manageExperts')}
                        aria-label={t($locale, 'rooms.manageExperts')}
                        onclick={() => (expertManagerOpen = true)}
                    >
                        <span>@</span>
                        <span class="topbar-btn-label">
                            {attachedRoom ? attachedRoom.name : t($locale, 'rooms.experts')}
                        </span>
                    </button>
                    {#if active}
                        <button
                            type="button"
                            class="topbar-pill topbar-icon-pill"
                            title={t($locale, 'chatInspector.review')}
                            aria-label={t($locale, 'chatInspector.review')}
                            onclick={() => openManagementTab(`/workspace-review?conversation=${encodeURIComponent(active!.conversationId)}`)}
                        >
                            <PrototypeIcon name="review" size={14} />
                            <span class="topbar-btn-label">{t($locale, 'chatInspector.review')}</span>
                        </button>
                        <button
                            type="button"
                            class="topbar-pill topbar-icon-pill branch-btn"
                            title={t($locale, 'chatInspector.branch')}
                            aria-label={t($locale, 'chatInspector.branch')}
                            disabled={streaming || !messages.length}
                            onclick={branchActiveConversation}
                        >
                            <svg class="topbar-svg-icon" aria-hidden="true" viewBox="0 0 16 16">
                                <circle cx="4.5" cy="3.5" r="1.75" />
                                <circle cx="4.5" cy="12.5" r="1.75" />
                                <circle cx="11.5" cy="5.5" r="1.75" />
                                <path d="M4.5 5.25v5.5M11.5 7.25c0 2.5-2.5 3.5-7 3.5" />
                            </svg>
                            <span class="topbar-btn-label">{t($locale, 'chatInspector.branch')}</span>
                        </button>
                    {/if}
                    <button
                        type="button"
                        class="topbar-pill topbar-icon-pill inspector-toggle-btn"
                        class:active={inspectorOpen}
                        data-testid="inspector-toggle"
                        title={t($locale, 'chatPage.details')}
                        aria-expanded={inspectorOpen}
                        aria-label={t($locale, 'chatPage.details')}
                        onclick={() => (inspectorOpen = !inspectorOpen)}
                    >
                        <PrototypeIcon name="overview" size={14} />
                        <span class="topbar-btn-label">{t($locale, 'chatPage.details')}</span>
                    </button>
                </div>
            {/if}
        </header>

        <div
            class="chat-stage"
            role="tabpanel"
            id={contentTabDomId('panel', ownerKey, conversationTabKey)}
            aria-labelledby={contentTabDomId('tab', ownerKey, conversationTabKey)}
            hidden={managementContentSelected}
        >
            <div class="chat-stage-columns">
                <div class="chat-conversation-column">
                    <div class="transcript-stage">
                    <div
                        class="transcript"
                        bind:this={transcriptEl}
                        onscroll={handleTranscriptScroll}
                    >
                        {#if loading}
                            <p class="status-note muted" role="status">{t($locale, 'chatPage.loading')}</p>
                        {:else if messages.length === 0}
                            <div class="empty-state">
                                <h1>{t($locale, 'chatPage.welcome')}</h1>
                                <p class="muted">{t($locale, 'chatPage.choose')}</p>
                            </div>
                        {:else}
                            <div class="messages-list">
                                {#each messages as msg, msgIndex (msg.id)}
                                    {@const msgExpertPalette = msg.expertName ? resolveExpertAccent(msg.expertId, msg.expertName, msg.expertColor) : null}
                                    <article
                                        class="message"
                                        class:user={msg.role === 'user'}
                                        class:assistant={msg.role === 'assistant'}
                                        class:expert-styled={Boolean(msgExpertPalette)}
                                        style:--expert-accent={msgExpertPalette?.accent}
                                        style:--expert-light={msgExpertPalette?.light}
                                        style:--expert-dark={msgExpertPalette?.dark}
                                        data-role={msg.role}
                                        data-message-id={msg.id}
                                        aria-label={msg.role === 'user' ? t($locale, 'chatInspector.you') : msg.expertName ? `@${msg.expertName}` : 'Nola'}
                                    >
                                        {#if msg.role === 'user'}
                                            <span class="sr-only">{t($locale, 'chatInspector.you')}</span>
                                            <div class="message-actions user-floating-actions">
                                                <button type="button" class="msg-action" onclick={() => copyMessage(msg)}>
                                                    {copiedMessageIds[msg.id] ? t($locale, 'chatInspector.copied') : t($locale, 'chatInspector.copy')}
                                                </button>
                                                {#if !streaming}
                                                    <button type="button" class="msg-action" onclick={() => requestEditAndResend(msgIndex)}>
                                                        {t($locale, 'chatInspector.edit')}
                                                    </button>
                                                {/if}
                                            </div>
                                        {:else}
                                            <div class="message-head">
                                                <div class="message-role">
                                                    {#if msg.expertName}
                                                        <span class="expert-role-badge">
                                                            <span class="expert-dot" aria-hidden="true"></span>
                                                            @{msg.expertName}
                                                        </span>
                                                    {:else}
                                                        <span class="nola-role-mark">
                                                            <span class="nola-role-glyph" aria-hidden="true">n</span>
                                                            <span>Nola</span>
                                                        </span>
                                                    {/if}
                                                </div>
                                                <div class="message-actions assistant-hover-actions">
                                                    <button type="button" class="msg-action" onclick={() => copyMessage(msg)}>
                                                        {copiedMessageIds[msg.id] ? t($locale, 'chatInspector.copied') : t($locale, 'chatInspector.copy')}
                                                    </button>
                                                    {#if !streaming}
                                                        <button type="button" class="msg-action" onclick={() => regenerateFrom(msgIndex)}>
                                                            {t($locale, 'chatInspector.regenerate')}
                                                        </button>
                                                    {/if}
                                                </div>
                                            </div>
                                        {/if}

                                        {#if confirmEditIndex === msgIndex && msg.role === 'user'}
                                            <div class="inline-confirm" role="alertdialog" aria-label={t($locale, 'chatInspector.editTruncateConfirm')}>
                                                <span>{t($locale, 'chatInspector.editTruncateConfirm')}</span>
                                                <div class="inline-confirm-actions">
                                                    <button
                                                        type="button"
                                                        class="primary compact-action"
                                                        onclick={() => void confirmEditAndResend(msg, msgIndex)}
                                                    >
                                                        {t($locale, 'shell.confirm')}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="compact-action"
                                                        onclick={() => (confirmEditIndex = null)}
                                                    >
                                                        {t($locale, 'shell.cancel')}
                                                    </button>
                                                </div>
                                            </div>
                                        {/if}

                                        {#if msg.attachments?.length}
                                            <div class="attachment-chips">
                                                {#each msg.attachments as att}
                                                    <button
                                                        type="button"
                                                        class="attachment-chip attachment-chip-btn"
                                                        class:has-thumb={Boolean(att.dataUrl)}
                                                        title={att.name}
                                                        onclick={() => openAttachmentInInspector(att)}
                                                    >
                                                        {#if att.dataUrl}
                                                            <img class="attachment-thumb message-thumb" src={att.dataUrl} alt={att.name} />
                                                        {/if}
                                                        <span>{att.name}</span>
                                                    </button>
                                                {/each}
                                            </div>
                                        {/if}

                                        {#if msg.reasoning}
                                            {@const isLiveReasoning = streaming && msgIndex === messages.length - 1 && msg.role === 'assistant' && !msg.content}
                                            <details class="reasoning-box" open={isLiveReasoning || undefined}>
                                                <summary>
                                                    <span class="disclosure-label">{t($locale, 'chatPage.thinking')}</span>
                                                    <svg class="disclosure-chevron" viewBox="0 0 16 16" aria-hidden="true">
                                                        <path d="M4 6l4 4 4-4" />
                                                    </svg>
                                                </summary>
                                                <pre class="reasoning-scroll" use:stickReasoningToBottom={isLiveReasoning ? msg.reasoning : ''}>{msg.reasoning}</pre>
                                            </details>
                                        {/if}
                                        {#if msg.toolCalls?.length}
                                            <div class="tool-calls">
                                                {#each msg.toolCalls as tc (tc.callId)}
                                                    {@const inlinePreview = formatToolInlinePreview(tc)}
                                                    <details class="tool-card" data-status={tc.status}>
                                                        <summary>
                                                            <span class="tool-name">{tc.name}</span>
                                                            {#if inlinePreview}
                                                                <span class="tool-inline-preview" title={inlinePreview}>{inlinePreview}</span>
                                                            {/if}
                                                            <span class="tool-status">
                                                                {tc.status === 'running'
                                                                    ? t($locale, 'chatPage.toolRunning')
                                                                    : tc.status === 'failed'
                                                                      ? t($locale, 'chatPage.toolFailed')
                                                                      : t($locale, 'chatPage.toolCompleted')}
                                                            </span>
                                                            <svg class="disclosure-chevron" viewBox="0 0 16 16" aria-hidden="true">
                                                                <path d="M4 6l4 4 4-4" />
                                                            </svg>
                                                        </summary>
                                                        {#if tc.summary}<pre class="tool-summary">{tc.summary}</pre>{/if}
                                                    </details>
                                                {/each}
                                            </div>
                                        {/if}

                                        {#if msg.uiSurfaces?.length}
                                            <div class="ui-surfaces">
                                                {#each msg.uiSurfaces as snap (snap.identity.surface_id + ':' + snap.identity.generation)}
                                                    <Surface
                                                        snapshot={snap}
                                                        oninteraction={(input, requestId, submittedAtMs) =>
                                                            handleSurfaceInteraction(input, requestId, submittedAtMs)}
                                                        onrefresh={async () => {
                                                            if (active) messages = await loadV2Messages(active.conversationId);
                                                        }}
                                                    />
                                                {/each}
                                            </div>
                                        {/if}

                                        {#if msg.expertReplies?.length}
                                            <div class="expert-replies" aria-label={t($locale, 'chatInspector.expertReplies')}>
                                                {#each msg.expertReplies as reply, rIdx (`${reply.expertId ?? reply.expertName}-${rIdx}`)}
                                                    {@const palette = resolveExpertAccent(reply.expertId, reply.expertName, reply.color)}
                                                    <section
                                                        class="expert-reply-card"
                                                        style:--expert-accent={palette.accent}
                                                        style:--expert-light={palette.light}
                                                        style:--expert-dark={palette.dark}
                                                    >
                                                        <header class="expert-reply-head">
                                                            <span class="expert-reply-avatar" aria-hidden="true">
                                                                {Array.from(reply.expertName.trim())[0] ?? 'E'}
                                                            </span>
                                                            <strong>@{reply.expertName}</strong>
                                                            {#if reply.role}<span class="expert-reply-meta">{reply.role}</span>{/if}
                                                            {#if reply.status}<span class="expert-reply-status">{reply.status}</span>{/if}
                                                        </header>
                                                        <div class="expert-reply-body">
                                                            <Markdown source={reply.content} />
                                                        </div>
                                                    </section>
                                                {/each}
                                            </div>
                                        {/if}

                                        <div class="message-body">
                                            {#if msg.role === 'assistant'}
                                                {#if msg.content}
                                                    <Markdown source={msg.content} streaming={streaming && msgIndex === messages.length - 1} />
                                                {:else if streaming && msgIndex === messages.length - 1}
                                                    <p class="muted">{t($locale, 'chatPage.working')}</p>
                                                {/if}
                                            {:else}
                                                <p class="user-text">{msg.content}</p>
                                            {/if}
                                        </div>
                                    </article>
                                {/each}

                                {#if pendingPermission && permissionSummary}
                                    <div class="interactive-card permission-card" role="region" aria-label={t($locale, 'chatPermission.title')}>
                                        <header class="interactive-head">
                                            <div class="permission-title-row">
                                                <span class="permission-accent-dot" aria-hidden="true"></span>
                                                <strong>{t($locale, 'chatPermission.title')}</strong>
                                                <code class="permission-tool-badge">{pendingPermission.tool}</code>
                                            </div>
                                            {#if permissionSummary.headline && permissionSummary.headline !== pendingPermission.tool && permissionSummary.headline !== permissionSummary.target}
                                                <p class="permission-headline">{permissionSummary.headline}</p>
                                            {/if}
                                            {#if permissionSummary.target}
                                                <code class="permission-target">{permissionSummary.target}</code>
                                            {/if}
                                            <span class="muted permission-reason">{formatPermissionReason(pendingPermission, $locale)}</span>
                                        </header>
                                        <details class="permission-raw-details">
                                            <summary>
                                                <span class="disclosure-label">{t($locale, 'chatPage.toolTechnicalDetails')}</span>
                                                <svg class="disclosure-chevron" viewBox="0 0 16 16" aria-hidden="true">
                                                    <path d="M4 6l4 4 4-4" />
                                                </svg>
                                            </summary>
                                            <pre class="interactive-args">{JSON.stringify(pendingPermission.arguments, null, 2)}</pre>
                                        </details>
                                        {#if permissionSubmitting}
                                            <p class="muted permission-status" role="status">{t($locale, 'chatPermission.submitting')}</p>
                                        {/if}
                                        <div class="interactive-actions">
                                            <button
                                                type="button"
                                                class="primary"
                                                disabled={permissionSubmitting}
                                                onclick={() => void handlePermissionDecision(true, 'once')}
                                            >
                                                {t($locale, 'chatPermission.allowOnce')}
                                            </button>
                                            <button
                                                type="button"
                                                disabled={permissionSubmitting}
                                                onclick={() => void handlePermissionDecision(true, 'session')}
                                            >
                                                {t($locale, 'chatPermission.allowConversation')}
                                            </button>
                                            <button
                                                type="button"
                                                class="danger"
                                                disabled={permissionSubmitting}
                                                onclick={() => void handlePermissionDecision(false)}
                                            >
                                                {t($locale, 'chatPermission.deny')}
                                            </button>
                                        </div>
                                    </div>
                                {/if}

                                {#if pendingQuestion}
                                    <div class="interactive-card question-card" role="region" aria-label={t($locale, 'chatPage.stateWaitingInput')}>
                                        <header class="interactive-head">
                                            <strong>{pendingQuestion.question}</strong>
                                        </header>
                                        {#if pendingQuestion.options.length}
                                            <div class="question-options">
                                                {#each pendingQuestion.options as opt}
                                                    <button type="button" class="topbar-pill" onclick={() => handleQuestionAnswer(opt)}>
                                                        {opt}
                                                    </button>
                                                {/each}
                                            </div>
                                        {/if}
                                        <form
                                            class="question-form"
                                            onsubmit={e => {
                                                e.preventDefault();
                                                if (questionAnswerDraft.trim()) void handleQuestionAnswer(questionAnswerDraft.trim());
                                            }}
                                        >
                                            <input
                                                type="text"
                                                bind:value={questionAnswerDraft}
                                                placeholder={t($locale, 'chatPage.placeholder')}
                                                aria-label={t($locale, 'chatPage.stateWaitingInput')}
                                            />
                                            <button type="submit" class="primary" disabled={!questionAnswerDraft.trim()}>
                                                {t($locale, 'chatPage.send')}
                                            </button>
                                        </form>
                                    </div>
                                {/if}
                            </div>
                        {/if}
                    </div>

                    {#if !isPinnedToBottom && messages.length > 0}
                        <div class="scroll-anchor-bar">
                            <button
                                type="button"
                                class="scroll-latest-btn"
                                onclick={() => void scrollToBottom(true)}
                            >
                                ↓ {t($locale, 'chatPage.latest')}
                            </button>
                        </div>
                    {/if}
                    </div>

                    <footer class="composer-wrap">
                        {#if needsModelConfig}
                            <div class="notice" role="status">
                                <a
                                    href="/models"
                                    onclick={e => {
                                        e.preventDefault();
                                        openManagementTab('/models');
                                    }}
                                >{t($locale, 'chatSession.configureModel')}</a>
                            </div>
                        {/if}
                        {#if showPlanBanner}
                            <div class="plan-approval-banner" role="region" aria-label={t($locale, 'chatInspector.planMode')}>
                                <div class="plan-banner-head">
                                    <strong>
                                        {conversationPlan.status === 'awaiting_approval' ||
                                        conversationPlan.status === 'proposed' ||
                                        conversationPlan.status === 'pending'
                                            ? t($locale, 'chatInspector.planPendingApproval')
                                            : conversationPlan.summary || t($locale, 'chatInspector.planActive')}
                                    </strong>
                                    <div class="plan-banner-actions">
                                        <button
                                            type="button"
                                            class="primary compact-action"
                                            disabled={planBusy}
                                            onclick={() => void handlePlanDecision('approve')}
                                        >
                                            {t($locale, 'chatInspector.planApprove')}
                                        </button>
                                        <button
                                            type="button"
                                            class="compact-action danger"
                                            disabled={planBusy}
                                            onclick={() => void handlePlanDecision('reject')}
                                        >
                                            {t($locale, 'chatInspector.planReject')}
                                        </button>
                                    </div>
                                </div>
                                {#if conversationPlan.steps.length}
                                    <ol class="plan-banner-steps">
                                        {#each conversationPlan.steps.slice(0, 6) as step, sIdx (step.id ?? sIdx)}
                                            <li>{step.title}</li>
                                        {/each}
                                    </ol>
                                {:else if conversationPlan.markdown}
                                    <p class="plan-banner-summary">{conversationPlan.markdown.slice(0, 260)}</p>
                                {/if}
                            </div>
                        {/if}
                        {#if statusNotice}
                            <p class="notice" role="status">{statusNotice}</p>
                        {/if}
                        {#if formattedError}
                            <div class="notice error error-banner" role="alert">
                                <svg class="error-banner-icon" viewBox="0 0 16 16" aria-hidden="true">
                                    <circle cx="8" cy="8" r="6.5" />
                                    <path d="M8 4.8v3.6M8 11.1h.01" />
                                </svg>
                                <span class="error-banner-text">{formattedError}</span>
                                <button
                                    type="button"
                                    class="notice-dismiss"
                                    title={t($locale, 'shell.dismiss')}
                                    aria-label={t($locale, 'shell.dismiss')}
                                    onclick={() => (error = '')}
                                >×</button>
                            </div>
                        {/if}

                        <div class="composer-anchor">
                            {#if expertPickerOpen}
                                <div bind:this={expertPopupEl} class="expert-popup" role="listbox" aria-label={t($locale, 'rooms.experts')}>
                                    {#if expertsLoading}
                                        <p class="expert-popup-empty">{t($locale, 'rooms.loading')}</p>
                                    {:else if filteredExperts.length === 0}
                                        <p class="expert-popup-empty">{t($locale, 'rooms.noExperts')}</p>
                                    {:else}
                                        {#each filteredExperts as exp, idx (exp.id)}
                                            {@const expPalette = resolveExpertAccent(exp.id, exp.name, exp.color)}
                                            <button
                                                type="button"
                                                role="option"
                                                class="expert-option"
                                                class:highlighted={idx === expertHighlightIdx}
                                                aria-selected={idx === expertHighlightIdx}
                                                onclick={() => selectExpert(exp)}
                                            >
                                                <span class="expert-option-name">
                                                    <span class="expert-dot" style:background={expPalette.accent} aria-hidden="true"></span>
                                                    @{exp.name}
                                                </span>
                                                {#if exp.description}
                                                    <small class="muted">{exp.description}</small>
                                                {/if}
                                            </button>
                                        {/each}
                                    {/if}
                                    <div class="expert-popup-footer">
                                        <button
                                            type="button"
                                            class="expert-manage-btn"
                                            onclick={() => {
                                                closeExpertPicker();
                                                expertManagerOpen = true;
                                            }}
                                        >
                                            {t($locale, 'rooms.manageExperts')}
                                        </button>
                                    </div>
                                </div>
                            {/if}

                            <form
                                class="composer-box"
                                class:drag-over={isDragOver}
                                ondragover={handleComposerDragOver}
                                ondragleave={handleComposerDragLeave}
                                ondrop={event => void handleComposerDrop(event)}
                                onsubmit={event => {
                                    event.preventDefault();
                                    void send();
                                }}
                            >
                                {#if attachments.length || selectedExperts.length}
                                    <div class="attachment-chips composer-staged-chips">
                                        {#each selectedExperts as exp (exp.id)}
                                            <span class="attachment-chip expert-chip">
                                                @{exp.name}
                                                <button
                                                    type="button"
                                                    class="chip-remove"
                                                    aria-label={t($locale, 'chatInspector.clearExperts')}
                                                    onclick={() => removeSelectedExpert(exp.id)}
                                                >×</button>
                                            </span>
                                        {/each}
                                        {#each attachments as att, idx}
                                            <span class="attachment-chip" class:has-thumb={Boolean(att.dataUrl)}>
                                                <button
                                                    type="button"
                                                    class="chip-preview-btn"
                                                    title={att.name}
                                                    onclick={() => openAttachmentInInspector(att)}
                                                >
                                                    {#if att.dataUrl}
                                                        <img class="attachment-thumb" src={att.dataUrl} alt={att.name} />
                                                    {/if}
                                                    <span>{att.name}</span>
                                                </button>
                                                <button type="button" class="chip-remove" aria-label={t($locale, 'shell.cancel')} onclick={() => removeAttachment(idx)}>×</button>
                                            </span>
                                        {/each}
                                    </div>
                                {/if}
                                <textarea
                                    id="chat-composer"
                                    bind:this={composerEl}
                                    bind:value={draft}
                                    rows="2"
                                    placeholder={t($locale, 'chatPage.placeholder')}
                                    aria-label={t($locale, 'chatPage.placeholder')}
                                    disabled={loading}
                                    oninput={handleDraftInput}
                                    onkeydown={onComposerKeydown}
                                    onpaste={event => void handleComposerPaste(event)}
                                ></textarea>
                                <div class="composer-footer">
                                    <div class="composer-toolbar">
                                        <input
                                            bind:this={fileInputEl}
                                            type="file"
                                            multiple
                                            hidden
                                            onchange={handleFileSelect}
                                        />
                                        <button
                                            type="button"
                                            class="toolbar-btn"
                                            disabled={streaming}
                                            onclick={() => fileInputEl?.click()}
                                        >
                                            {t($locale, 'chatInspector.attach')}
                                        </button>
                                        <button
                                            bind:this={expertToggleBtnEl}
                                            type="button"
                                            class="toolbar-btn"
                                            class:active={expertPickerOpen}
                                            disabled={streaming}
                                            aria-expanded={expertPickerOpen}
                                            aria-label={t($locale, 'chatInspector.mentionExpert')}
                                            onclick={toggleExpertPicker}
                                        >
                                            @ {t($locale, 'rooms.experts')}
                                        </button>
                                        {#if active}
                                            <button
                                                type="button"
                                                class="toolbar-btn"
                                                class:active={conversationPlan.enabled}
                                                disabled={planBusy}
                                                aria-pressed={conversationPlan.enabled}
                                                onclick={() => void handleTogglePlanMode()}
                                            >
                                                {conversationPlan.enabled ? t($locale, 'chatInspector.planActive') : t($locale, 'chatInspector.planMode')}
                                            </button>
                                        {/if}
                                        {#if modelChoices.length}
                                            <select
                                                class="toolbar-select"
                                                aria-label={t($locale, 'chatSession.model')}
                                                value={selectedModelId}
                                                disabled={streaming}
                                                onchange={e => void handleModelChange(e.currentTarget.value)}
                                            >
                                                {#each modelChoices as choice (choice.id)}
                                                    <option value={choice.id}>{choice.displayName}</option>
                                                {/each}
                                            </select>
                                        {/if}
                                        <select
                                            class="toolbar-select"
                                            aria-label={t($locale, 'chatSession.permission')}
                                            value={permissionProfile}
                                            disabled={permissionSaving}
                                            onchange={e => void handlePermissionModeChange(e.currentTarget.value as V2PermissionProfile)}
                                        >
                                            <option value="cautious">{t($locale, 'chatSession.cautious')}</option>
                                            <option value="allow_all">{t($locale, 'chatSession.allowAll')}</option>
                                        </select>
                                        <select
                                            class="toolbar-select"
                                            aria-label={t($locale, 'chatSession.reasoning')}
                                            value={reasoningEffort}
                                            disabled={streaming}
                                            onchange={e => void handleReasoningEffortChange(e.currentTarget.value as V2ReasoningEffort)}
                                        >
                                            <option value="automatic">{t($locale, 'chatSession.automatic')}</option>
                                            <option value="none">{t($locale, 'chatSession.none')}</option>
                                            <option value="low">{t($locale, 'chatSession.low')}</option>
                                            <option value="medium">{t($locale, 'chatSession.medium')}</option>
                                            <option value="high">{t($locale, 'chatSession.high')}</option>
                                        </select>
                                    </div>
                                    <div class="composer-actions">
                                        {#if streaming}
                                            <button type="button" class="danger composer-stop-btn" disabled={cancelling} onclick={() => void cancelActiveRun()}>
                                                {t($locale, cancelling ? 'chatPage.cancelPending' : 'chatPage.stop')}
                                            </button>
                                        {:else}
                                            <button type="submit" class="primary composer-send-btn" disabled={(!draft.trim() && !attachments.length) || loading}>
                                                {t($locale, 'chatPage.send')}
                                            </button>
                                        {/if}
                                    </div>
                                </div>
                            </form>
                        </div>
                    </footer>
                </div>

                {#if inspectorOpen}
                    <div class="inspector-split-wrap">
                        {#if viewportWidth > 760}
                            <PanelResizeHandle
                                label={t($locale, 'shell.resizeInspector')}
                                controls="conversation-inspector-panel"
                                value={panelLayout.inspector}
                                min={320}
                                max={panelLayout.inspectorMax}
                                defaultValue={layoutDefaults.inspector}
                                reverse={true}
                                onchange={next => setWorkspaceWidth('inspector', next)}
                            />
                        {/if}
                        <ConversationInspector
                            conversationId={active?.conversationId ?? null}
                            {messages}
                            usage={sessionUsage}
                            {streaming}
                            width={viewportWidth > 760 ? panelLayout.inspector : undefined}
                            plan={conversationPlan}
                            previewItem={inspectorPreview}
                            onclearpreview={() => (inspectorPreview = null)}
                            onclose={() => {
                                inspectorPreview = null;
                                inspectorOpen = false;
                            }}
                            onopenPage={openManagementTab}
                            onplanchange={next => (conversationPlan = next)}
                            oncompacted={summary => {
                                statusNotice = summary;
                                if (active) {
                                    void reloadActiveConversation(active.conversationId, generation).catch(() => {});
                                }
                            }}
                        />
                    </div>
                {/if}
            </div>
        </div>

        {#each contentTabs.items as tab (tab.key)}
            {#if tab.kind === 'page'}
                <div
                    class="management-panel"
                    role="tabpanel"
                    id={contentTabDomId('panel', ownerKey, tab.key)}
                    aria-labelledby={contentTabDomId('tab', ownerKey, tab.key)}
                    hidden={contentTabs.selected !== tab.key}
                >
                    <WorkspacePageTab
                        path={tab.path}
                        ownerKey={tab.key}
                        viewport={viewportFor(tab.key)}
                        active={contentTabs.selected === tab.key}
                        onlocation={nextPath => (contentTabs = updatePageContentLocation(contentTabs, tab.key, nextPath))}
                    />
                </div>
            {/if}
        {/each}
    </main>
</div>

<style>
    .chat-shell {
        display: grid;
        grid-template-columns: var(--sidebar-width, 260px) minmax(0, 1fr);
        height: 100dvh;
        min-height: 0;
        background: var(--canvas);
        color: var(--content);
    }
    .chat-shell.sidebar-collapsed {
        grid-template-columns: 56px minmax(0, 1fr);
    }
    .chat-sidebar {
        position: relative;
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        overflow: visible;
        border-right: 1px solid var(--border);
        background: var(--rail);
        padding: 10px 8px;
        gap: 10px;
    }
    .chat-shell.sidebar-collapsed .chat-sidebar {
        align-items: center;
        padding: 10px 6px;
    }
    .sidebar-top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 6px;
    }
    .chat-shell.sidebar-collapsed .sidebar-top {
        flex-direction: column;
        align-items: center;
        justify-content: flex-start;
        gap: 8px;
    }
    .sidebar-tools {
        display: flex;
        align-items: center;
        gap: 4px;
    }
    .chat-shell.sidebar-collapsed .sidebar-tools {
        flex-direction: column;
        align-items: center;
    }
    .sidebar-body {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
    }
    .sidebar-footer {
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding-top: 8px;
        border-top: 1px solid var(--border);
    }
    .chat-shell.sidebar-collapsed .sidebar-footer {
        margin-top: auto;
        width: 100%;
        align-items: center;
    }
    .chat-shell.sidebar-collapsed .sidebar-footer-item {
        width: 36px;
        height: 36px;
        min-width: 36px;
        min-height: 36px;
        padding: 0;
        justify-content: center;
    }
    .sidebar-footer-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        min-height: 36px;
        padding: 6px 10px;
        border: 1px solid transparent;
        border-radius: 8px;
        background: transparent;
        box-shadow: none;
        color: var(--content);
        text-decoration: none;
        text-align: start;
        font: inherit;
        font-size: 13px;
        line-height: 1.4;
        cursor: pointer;
    }
    .sidebar-footer-item:hover:not(:disabled) {
        background: var(--composer-pill-hover-bg);
        border-color: transparent;
        color: var(--content);
    }
    .sidebar-footer-item:active:not(:disabled) {
        background: var(--control-active-bg);
    }
    .icon-btn {
        display: grid;
        place-items: center;
        width: 32px;
        height: 32px;
        padding: 0;
        border: 1px solid transparent;
        border-radius: 7px;
        background: transparent;
        color: var(--muted);
        cursor: pointer;
    }
    .icon-btn:hover {
        color: var(--content);
        background: var(--composer-pill-hover-bg);
    }
    .chat-main {
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        background: var(--canvas);
    }
    .workspace-topbar {
        display: flex;
        align-items: center;
        min-height: var(--topbar-height, 48px);
        padding: 4px 12px;
        border-bottom: 1px solid var(--border);
        background: var(--canvas);
        gap: 8px;
    }
    .topbar-actions {
        margin-left: auto;
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .topbar-pill {
        width: auto;
        max-width: 220px;
        min-height: 30px;
        min-width: 0;
        padding: 4px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--panel);
        color: var(--content);
        font-size: 12px;
        cursor: pointer;
    }
    .toolbar-btn,
    .toolbar-select {
        width: auto;
        max-width: 200px;
        height: 30px;
        min-height: 30px;
        min-width: 0;
        padding: 0 10px;
        border: 1px solid transparent;
        border-radius: 8px;
        background-color: var(--composer-pill-bg, var(--surface-subtle));
        color: var(--content);
        font-size: 12px;
        line-height: 28px;
        white-space: nowrap;
        flex: 0 0 auto;
        cursor: pointer;
        transition: background-color var(--motion-fast) ease, border-color var(--motion-fast) ease, color var(--motion-fast) ease;
    }
    .toolbar-select {
        padding: 0 26px 0 10px;
        background-color: var(--composer-pill-bg, var(--surface-subtle));
        background-image: var(--select-chevron);
        background-repeat: no-repeat;
        background-position: right 8px center;
        background-size: 12px 12px;
    }
    .toolbar-btn:hover:not(:disabled),
    .toolbar-select:hover:not(:disabled) {
        background-color: var(--composer-pill-hover-bg);
        border-color: var(--border);
    }
    .topbar-icon-pill {
        display: inline-flex;
        align-items: center;
        gap: 5px;
    }
    .topbar-svg-icon {
        width: 14px;
        height: 14px;
        flex-shrink: 0;
        fill: none;
        stroke: currentColor;
        stroke-width: 1.5;
        stroke-linecap: round;
        stroke-linejoin: round;
    }
    .inspector-toggle-btn.active,
    .toolbar-btn.active {
        border-color: color-mix(in srgb, var(--primary) 38%, var(--border));
        background-color: var(--accent-subtle);
        color: var(--content);
        font-weight: 550;
    }
    .topbar-pill:hover:not(:disabled) {
        border-color: var(--outline);
        background: var(--composer-pill-hover-bg);
    }
    .chat-stage {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: column;
    }
    .chat-stage[hidden],
    .management-panel[hidden] {
        display: none;
    }
    .chat-stage-columns {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: row;
    }
    .chat-conversation-column {
        flex: 1;
        min-width: 0;
        min-height: 0;
        display: flex;
        flex-direction: column;
        position: relative;
    }
    .management-panel {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: column;
    }
    .transcript-stage {
        flex: 1;
        min-height: 0;
        position: relative;
        display: flex;
        flex-direction: column;
    }
    .transcript {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        padding: 24px 20px 48px;
    }
    .scroll-anchor-bar {
        position: absolute;
        left: 0;
        right: 0;
        bottom: 14px;
        z-index: 10;
        display: flex;
        justify-content: center;
        padding: 0 20px;
        pointer-events: none;
    }
    .scroll-latest-btn {
        pointer-events: auto;
        min-height: 30px;
        padding: 4px 14px;
        border-radius: 999px;
        font-size: 12px;
        font-weight: 550;
        background: color-mix(in srgb, var(--surface-elevated) 88%, transparent);
        backdrop-filter: blur(10px);
        -webkit-backdrop-filter: blur(10px);
        border: 1px solid var(--border);
        color: var(--content);
        box-shadow: 0 4px 14px rgb(0 0 0 / 0.14);
        transition: transform var(--motion-fast) ease, background-color var(--motion-fast) ease, box-shadow var(--motion-fast) ease;
    }
    .scroll-latest-btn:hover {
        background: var(--surface-elevated);
        transform: translateY(-1px);
        box-shadow: 0 6px 18px rgb(0 0 0 / 0.18);
    }
    .sr-only {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip-path: inset(50%);
        white-space: nowrap;
    }
    .status-note {
        max-width: 780px;
        margin: 24px auto;
        text-align: center;
        font-size: 13px;
    }
    .empty-state {
        max-width: 680px;
        margin: 12vh auto 0;
        text-align: center;
    }
    .empty-state h1 {
        font-size: 26px;
        font-weight: 600;
        margin: 0 0 8px;
    }
    .messages-list {
        max-width: 780px;
        margin: 0 auto;
        display: flex;
        flex-direction: column;
        gap: 20px;
    }
    .message {
        position: relative;
        display: flex;
        flex-direction: column;
        gap: 10px;
        min-width: 0;
    }
    .message.user {
        align-self: flex-end;
        max-width: min(84%, 640px);
        width: fit-content;
        padding: 11px 16px;
        border-radius: 18px 18px 6px 18px;
        border: 1px solid var(--user-bubble-border);
        background: var(--user-bubble-bg);
        color: var(--content);
    }
    .message.assistant {
        width: 100%;
        padding: 4px 2px;
        border: 0;
        border-radius: 0;
        background: transparent;
    }
    .message-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        min-height: 24px;
    }
    .message-role {
        font-size: 12px;
        font-weight: 600;
        color: var(--muted);
    }
    .nola-role-mark {
        display: inline-flex;
        align-items: center;
        gap: 7px;
        color: var(--muted);
        font-size: 12.5px;
        font-weight: 600;
    }
    .nola-role-glyph {
        display: inline-grid;
        place-items: center;
        width: 20px;
        height: 20px;
        border-radius: 6px;
        background: var(--panel);
        border: 1px solid var(--border);
        color: var(--content);
        font-family: var(--font-display);
        font-size: 13px;
        font-weight: 700;
        line-height: 1;
    }
    .message-actions {
        display: flex;
        align-items: center;
        gap: 2px;
    }
    .user-floating-actions {
        position: absolute;
        top: -14px;
        right: 10px;
        padding: 2px;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: color-mix(in srgb, var(--surface-elevated) 92%, transparent);
        backdrop-filter: blur(8px);
        box-shadow: 0 2px 10px rgb(0 0 0 / 0.08);
        opacity: 0;
        transition: opacity 130ms ease;
        z-index: 4;
    }
    .assistant-hover-actions {
        opacity: 0;
        transition: opacity 130ms ease;
    }
    .message.user:hover .user-floating-actions,
    .message.user:focus-within .user-floating-actions,
    .message.assistant:hover .assistant-hover-actions,
    .message.assistant:focus-within .assistant-hover-actions {
        opacity: 1;
    }
    @media (hover: none), (pointer: coarse) {
        .user-floating-actions {
            position: static;
            align-self: flex-end;
            margin-top: -4px;
            margin-bottom: -2px;
            opacity: 1;
            box-shadow: none;
            background: transparent;
            border: 0;
            padding: 0;
        }
        .assistant-hover-actions {
            opacity: 1;
        }
    }
    .msg-action {
        padding: 2px 8px;
        min-height: 24px;
        min-width: 0;
        border: 1px solid transparent;
        border-radius: 6px;
        background: transparent;
        color: var(--muted);
        font-size: 11px;
        cursor: pointer;
    }
    .msg-action:hover {
        color: var(--content);
        background: var(--composer-pill-hover-bg);
        border-color: transparent;
    }
    .inline-confirm {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        padding: 8px 10px;
        border-radius: 8px;
        border: 1px solid var(--warning);
        background: var(--panel);
        font-size: 12px;
    }
    .inline-confirm-actions {
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .compact-action {
        min-height: 28px;
        min-width: 0;
        padding: 3px 10px;
        font-size: 12px;
        border-radius: 6px;
    }
    .user-text {
        margin: 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        line-height: 1.55;
    }
    .reasoning-box,
    .tool-card,
    .permission-raw-details {
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 6px 11px;
        background: var(--panel);
        font-size: 13px;
    }
    .reasoning-box summary,
    .tool-card summary,
    .permission-raw-details summary {
        list-style: none;
        cursor: pointer;
        color: var(--muted);
        display: flex;
        align-items: center;
        gap: 8px;
        min-height: 28px;
        padding-block: 2px;
        font-size: 12px;
    }
    .reasoning-box summary::-webkit-details-marker,
    .tool-card summary::-webkit-details-marker,
    .permission-raw-details summary::-webkit-details-marker {
        display: none;
    }
    .disclosure-label {
        flex: 1;
        min-width: 0;
        font-weight: 500;
    }
    .disclosure-chevron {
        width: 14px;
        height: 14px;
        flex: none;
        fill: none;
        stroke: currentColor;
        stroke-width: 1.9;
        stroke-linecap: round;
        stroke-linejoin: round;
        opacity: 0.65;
        transform-origin: 50% 50%;
        transition: transform 140ms ease, opacity 140ms ease;
    }
    .reasoning-box[open] > summary .disclosure-chevron,
    .tool-card[open] > summary .disclosure-chevron,
    .permission-raw-details[open] > summary .disclosure-chevron {
        transform: rotate(180deg);
        opacity: 0.9;
    }
    .reasoning-box pre,
    .tool-summary,
    .interactive-args {
        margin: 8px 0 2px;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        font-size: 12px;
        color: var(--muted);
    }
    .reasoning-box pre.reasoning-scroll {
        max-height: 180px;
        overflow-y: auto;
        overscroll-behavior: contain;
        line-height: 1.5;
        padding-right: 4px;
    }
    .tool-calls,
    .ui-surfaces {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .tool-name {
        font-family: var(--font-code, monospace);
        font-weight: 550;
        color: var(--content);
        flex: none;
    }
    .tool-inline-preview {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-family: var(--font-code, monospace);
        font-size: 11.5px;
        color: var(--muted);
        opacity: 0.88;
    }
    .tool-card[open] .tool-inline-preview {
        opacity: 0.6;
    }
    .tool-status {
        display: inline-flex;
        align-items: center;
        margin-inline-start: auto;
        padding: 1px 8px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--surface);
        color: var(--muted);
        font-size: 11px;
        font-weight: 600;
        line-height: 1.45;
        flex: none;
    }
    .tool-card[data-status='running'] .tool-status {
        color: var(--info);
        border-color: color-mix(in srgb, var(--info) 40%, var(--border));
        background: color-mix(in srgb, var(--info) 12%, transparent);
    }
    .tool-card[data-status='completed'] .tool-status {
        color: var(--success);
        border-color: color-mix(in srgb, var(--success) 40%, var(--border));
        background: color-mix(in srgb, var(--success) 12%, transparent);
    }
    .tool-card[data-status='failed'] .tool-status,
    .tool-card[data-status='error'] .tool-status {
        color: var(--danger);
        border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
        background: color-mix(in srgb, var(--danger) 12%, transparent);
    }
    .interactive-card {
        border: 1px solid color-mix(in srgb, var(--warning) 44%, var(--border));
        border-left: 3px solid var(--warning);
        border-radius: 12px;
        padding: 14px 16px;
        background: color-mix(in srgb, var(--warning) 6%, var(--surface));
        box-shadow: 0 4px 18px rgb(0 0 0 / 0.06);
        display: flex;
        flex-direction: column;
        gap: 10px;
    }
    .question-card {
        border-color: color-mix(in srgb, var(--info) 42%, var(--border));
        border-left-color: var(--info);
        background: color-mix(in srgb, var(--info) 5%, var(--surface));
    }
    .interactive-head {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .permission-title-row {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
    }
    .permission-accent-dot {
        width: 8px;
        height: 8px;
        border-radius: 999px;
        background: var(--warning);
        flex: none;
    }
    .permission-tool-badge {
        padding: 2px 8px;
        border-radius: 6px;
        background: var(--surface);
        border: 1px solid color-mix(in srgb, var(--warning) 35%, var(--border));
        font-size: 12px;
        color: var(--content);
    }
    .permission-headline {
        margin: 0;
        font-size: 13px;
        font-weight: 500;
    }
    .permission-target {
        display: block;
        padding: 7px 10px;
        border-radius: 8px;
        background: var(--surface);
        border: 1px solid var(--border);
        font-size: 12px;
        overflow-wrap: anywhere;
        white-space: pre-wrap;
    }
    .permission-reason,
    .permission-status {
        margin: 0;
        font-size: 12px;
    }
    .interactive-actions,
    .question-options,
    .question-form {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 8px;
    }
    .question-form input {
        flex: 1;
        min-width: 180px;
        min-height: 36px;
        padding: 6px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        color: var(--content);
    }
    .attachment-chips {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .attachment-chip {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 3px 8px;
        border: 1px solid var(--border);
        border-radius: 999px;
        background: var(--panel);
        font-size: 12px;
    }
    .attachment-chip.has-thumb {
        border-radius: 10px;
        padding: 4px 8px 4px 4px;
    }
    .attachment-chip-btn {
        color: var(--content);
        cursor: pointer;
        min-width: 0;
        min-height: 0;
    }
    .attachment-chip-btn:hover {
        border-color: var(--info);
    }
    .chip-preview-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 0;
        border: 0;
        background: transparent;
        color: inherit;
        font: inherit;
        cursor: pointer;
        min-width: 0;
        min-height: 0;
    }
    .attachment-thumb {
        width: 28px;
        height: 28px;
        object-fit: cover;
        border-radius: 6px;
        border: 1px solid var(--border);
        flex-shrink: 0;
    }
    .attachment-thumb.message-thumb {
        width: 48px;
        height: 48px;
    }
    .expert-chip {
        border-color: var(--info);
        color: var(--content);
    }
    .chip-remove {
        border: 0;
        background: transparent;
        color: var(--muted);
        cursor: pointer;
        padding: 0 2px;
        min-width: 0;
        min-height: 0;
    }
    .composer-wrap {
        max-width: 820px;
        width: 100%;
        margin: 0 auto;
        padding: 6px 20px max(16px, env(safe-area-inset-bottom));
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .composer-footer {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        padding-top: 8px;
        border-top: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    }
    .composer-toolbar {
        flex: 1;
        min-width: 0;
        height: 32px;
        display: flex;
        flex-wrap: nowrap;
        align-items: center;
        gap: 6px;
        overflow-x: auto;
        scrollbar-width: none;
        -webkit-overflow-scrolling: touch;
    }
    .composer-toolbar::-webkit-scrollbar {
        display: none;
    }
    .composer-staged-chips {
        padding-bottom: 4px;
        border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
    }
    .composer-anchor {
        position: relative;
    }
    .expert-popup {
        position: absolute;
        left: 0;
        right: 0;
        bottom: calc(100% + 6px);
        max-height: 280px;
        overflow-y: auto;
        padding: 6px;
        border-radius: 10px;
        border: 1px solid var(--border);
        background: var(--surface-elevated);
        box-shadow: var(--dialog-shadow);
        z-index: 20;
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .expert-popup-empty {
        margin: 0;
        padding: 8px 10px;
        font-size: 12px;
        color: var(--muted);
    }
    .expert-option {
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: 3px;
        flex-shrink: 0;
        height: auto;
        min-height: unset;
        width: 100%;
        padding: 8px 10px;
        border: 1px solid transparent;
        border-radius: 8px;
        background: transparent;
        line-height: 1.4;
        white-space: normal;
        text-align: left;
        cursor: pointer;
    }
    .expert-option small {
        line-height: 1.4;
        white-space: normal;
        overflow-wrap: anywhere;
    }
    .expert-option:hover,
    .expert-option.highlighted {
        background: var(--accent-subtle);
        border-color: var(--border);
    }
    .expert-option-name {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        font-size: 13px;
        font-weight: 600;
    }
    .expert-dot {
        display: inline-block;
        width: 8px;
        height: 8px;
        border-radius: 999px;
        background: var(--expert-accent, var(--info));
        flex: none;
    }
    .expert-role-badge {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        color: var(--content);
    }
    .message.expert-styled {
        padding: 14px 16px;
        border-radius: 12px;
        border: 1px solid var(--border);
        border-left: 3px solid var(--expert-accent, var(--info));
        background: var(--expert-light, var(--panel));
    }
    .expert-replies {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .expert-reply-card {
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px 12px;
        border-radius: 10px;
        border: 1px solid var(--border);
        border-left: 3px solid var(--expert-accent, var(--info));
        background: var(--expert-light, var(--canvas));
    }
    .expert-reply-head {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 6px;
        font-size: 12px;
    }
    .expert-reply-avatar {
        width: 22px;
        height: 22px;
        border-radius: 7px;
        display: grid;
        place-items: center;
        font-size: 11px;
        font-weight: 700;
        color: #fff;
        background: var(--expert-accent, var(--info));
        flex: none;
    }
    .expert-reply-meta,
    .expert-reply-status {
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        font-size: 10px;
        color: var(--muted);
    }
    .expert-reply-body {
        font-size: 13px;
    }
    .expert-popup-footer {
        border-top: 1px solid var(--border);
        margin-top: 4px;
        padding-top: 6px;
    }
    .expert-manage-btn {
        width: 100%;
        min-height: 32px;
        padding: 4px 10px;
        border-radius: 7px;
        font-size: 12px;
        text-align: center;
    }
    .plan-approval-banner {
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 10px 12px;
        border: 1px solid var(--primary);
        border-radius: 10px;
        background: var(--panel);
        font-size: 12px;
    }
    .plan-banner-head {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }
    .plan-banner-actions {
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .plan-banner-steps {
        margin: 0;
        padding-inline-start: 18px;
        color: var(--muted);
    }
    .plan-banner-summary {
        margin: 0;
        color: var(--muted);
        white-space: pre-wrap;
    }
    .inspector-split-wrap {
        position: relative;
        display: flex;
        min-height: 0;
        flex-shrink: 0;
    }
    @media (prefers-color-scheme: dark) {
        :global(:root:not([data-appearance='light'])) .message.expert-styled,
        :global(:root:not([data-appearance='light'])) .expert-reply-card {
            background: var(--expert-dark, var(--panel));
        }
    }
    :global(:root[data-appearance='dark']) .message.expert-styled,
    :global(:root[data-appearance='dark']) .expert-reply-card {
        background: var(--expert-dark, var(--panel));
    }
    .composer-box {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 12px 14px 10px;
        border: 1px solid var(--composer-border);
        border-radius: 16px;
        background: var(--composer-bg);
        box-shadow: var(--composer-shadow);
        transition: border-color var(--motion-fast) ease, box-shadow var(--motion-fast) ease, background-color var(--motion-fast) ease;
    }
    .composer-box.drag-over {
        border-color: var(--info);
        background: color-mix(in srgb, var(--info) 8%, var(--composer-bg));
        box-shadow: 0 0 0 2px color-mix(in srgb, var(--info) 24%, transparent);
    }
    .composer-box:focus-within {
        border-color: var(--composer-focus-border);
        box-shadow: var(--composer-focus-shadow);
    }
    #chat-composer {
        width: 100%;
        min-width: 0;
        min-height: 44px;
        max-height: 220px;
        padding: 2px 2px 0;
        resize: none;
        border: none;
        background: transparent;
        box-shadow: none;
        color: var(--content);
        font: inherit;
        line-height: 1.5;
    }
    #chat-composer:focus,
    #chat-composer:hover {
        border: none;
        outline: none;
        box-shadow: none;
    }
    .composer-actions {
        flex: none;
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .composer-send-btn,
    .composer-stop-btn {
        height: 32px;
        min-height: 32px;
        padding: 0 14px;
        border-radius: 10px;
        font-size: 12.5px;
        font-weight: 600;
        white-space: nowrap;
    }
    .composer-send-btn:disabled {
        background: var(--composer-pill-bg);
        border-color: transparent;
        color: var(--muted);
        opacity: 0.72;
        box-shadow: none;
    }
    .composer-stop-btn {
        border-color: color-mix(in srgb, var(--danger) 55%, var(--border));
        background: color-mix(in srgb, var(--danger) 12%, transparent);
        color: var(--danger);
    }
    .composer-stop-btn:hover:not(:disabled) {
        background: color-mix(in srgb, var(--danger) 20%, transparent);
        border-color: var(--danger);
    }
    .notice {
        margin: 0;
        padding: 9px 12px;
        border-radius: 10px;
        border: 1px solid var(--border);
        background: var(--panel);
        font-size: 13px;
    }
    .notice.error {
        border-color: color-mix(in srgb, var(--danger) 42%, var(--border));
        background: color-mix(in srgb, var(--danger) 8%, var(--surface));
        color: var(--danger);
    }
    .error-banner {
        display: flex;
        align-items: flex-start;
        gap: 9px;
    }
    .error-banner-icon {
        width: 16px;
        height: 16px;
        margin-top: 2px;
        flex: none;
        fill: none;
        stroke: currentColor;
        stroke-width: 1.75;
        stroke-linecap: round;
        stroke-linejoin: round;
    }
    .error-banner-text {
        flex: 1;
        min-width: 0;
        line-height: 1.45;
        overflow-wrap: anywhere;
    }
    .notice-dismiss {
        flex: none;
        display: inline-grid;
        place-items: center;
        width: 24px;
        height: 24px;
        min-width: 24px;
        min-height: 24px;
        margin: -2px -4px -2px 0;
        padding: 0;
        border: 0;
        border-radius: 6px;
        background: transparent;
        color: inherit;
        opacity: 0.72;
        font-size: 16px;
        line-height: 1;
        cursor: pointer;
    }
    .notice-dismiss:hover {
        opacity: 1;
        background: color-mix(in srgb, var(--danger) 14%, transparent);
        border-color: transparent;
    }
    .muted {
        color: var(--muted);
    }
    @media (pointer: coarse) {
        .chip-remove {
            min-width: 36px;
            min-height: 36px;
            display: inline-flex;
            align-items: center;
            justify-content: center;
            margin: -4px -6px -4px 0;
            border-radius: 999px;
        }
        .workspace-topbar .icon-btn,
        .workspace-topbar .topbar-pill {
            min-height: 44px;
            min-width: 44px;
        }
    }
    @media (max-width: 760px) {
        .chat-shell,
        .chat-shell.sidebar-collapsed {
            grid-template-columns: minmax(0, 1fr);
        }
        .chat-sidebar {
            display: none;
        }
        .workspace-topbar {
            padding: 4px 8px;
            gap: 4px;
            flex-wrap: nowrap;
            min-height: 48px;
        }
        .topbar-actions {
            gap: 4px;
            flex-shrink: 0;
        }
        .topbar-btn-label {
            position: absolute;
            width: 1px;
            height: 1px;
            overflow: hidden;
            clip-path: inset(50%);
            white-space: nowrap;
        }
        .workspace-topbar .icon-btn,
        .workspace-topbar .topbar-icon-pill {
            width: 40px;
            min-width: 40px;
            min-height: 40px;
            padding: 0;
            display: inline-flex;
            align-items: center;
            justify-content: center;
        }
        .composer-wrap {
            padding: 6px 12px max(12px, env(safe-area-inset-bottom));
        }
        .composer-box {
            padding: 10px 12px 8px;
            border-radius: 14px;
        }
        .composer-toolbar {
            padding-bottom: 0;
        }
        .chip-remove {
            min-width: 36px;
            min-height: 36px;
            display: inline-flex;
            align-items: center;
            justify-content: center;
            margin: -4px -6px -4px 0;
            border-radius: 999px;
        }
    }
</style>
