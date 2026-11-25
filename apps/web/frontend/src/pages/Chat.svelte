<script lang="ts">
    import { onMount, tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { darkAppearance, toggleAppearance } from '../lib/appearance';
    import { ApiError, errorMessage } from '../lib/api';
    import { workspaceContentNavigationEvent, navigateWorkspace } from '../lib/workspaceNavigation';
    import { workspaceTabCloseEvent } from '../lib/workspacePageLocation';
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
    import type { ChatConversation } from '../lib/chatConversations';
    import type { UiInteraction, UiSnapshot, UiSubmissionOutcome } from '../lib/generativeUi';
    import {
        branchV2Conversation,
        cancelV2Turn,
        compactV2Conversation,
        createV2Conversation,
        emptyV2TokenUsage,
        formatPermissionSummary,
        getV2Conversation,
        getV2ConversationConfiguration,
        hasConfiguredChatModel,
        listV2Conversations,
        listV2Experts,
        listV2ModelChoices,
        loadV2Messages,
        loadV2MessagesWithUsage,
        mergeV2TokenUsage,
        normalizeV2PermissionProfile,
        normalizeV2ReasoningEffort,
        parseV2Attachment,
        patchV2ConversationConfiguration,
        respondV2Permission,
        respondV2UserInput,
        streamV2ChatMessage,
        submitV2UiInteraction,
        truncateV2Conversation,
        type V2Attachment,
        type V2Expert,
        type V2Message,
        type V2ModelChoice,
        type V2PermissionProfile,
        type V2PermissionPrompt,
        type V2PermissionScope,
        type V2ReasoningEffort,
        type V2TokenUsage,
        type V2UiActionPayload,
        type V2UserQuestion,
    } from '../lib/v2Chat';
    import ChatConversationManager from '../components/ChatConversationManager.svelte';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import ConversationInspector from '../components/ConversationInspector.svelte';
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
    let loading = $state(false);
    let streaming = $state(false);
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

    // @ Expert mention picker
    let availableExperts = $state<V2Expert[]>([]);
    let expertsLoading = $state(false);
    let expertsLoaded = $state(false);
    let expertPickerOpen = $state(false);
    let expertQuery = $state('');
    let expertTriggerRange = $state<{ start: number; end: number } | null>(null);
    let expertHighlightIdx = $state(0);
    let selectedExperts = $state<V2Expert[]>([]);

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

    let contentTabs = $state<ConversationContentTabs>(emptyContentTabs());
    const tabViewports = new Map<string, { top: number }>();
    const selectedContentTab = $derived(contentTabs.items.find(item => item.key === contentTabs.selected));
    const managementContentSelected = $derived(selectedContentTab?.kind === 'page');
    const isConversationTabActive = $derived(contentTabs.selected === conversationTabKey);
    const activeTabTitle = $derived(active?.title || t($locale, 'shell.conversations.new'));
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

    /** Explicit user Stop button: aborts SSE reader AND cancels the active Host run. */
    function cancelActiveRun() {
        const targetId = active?.conversationId ?? null;
        detachStream();
        if (targetId) {
            void cancelV2Turn(targetId).catch(() => {});
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
            availableExperts = await listV2Experts(forceRefresh);
            expertsLoaded = true;
        } catch {
            // Keep graceful empty list if agent-rooms is unavailable.
        } finally {
            expertsLoading = false;
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
                statusNotice = streaming
                    ? t($locale, 'chatPermissionMode.active')
                    : t($locale, 'chatPermissionMode.saved');
            }
        } catch (cause) {
            if (active?.conversationId === targetId) {
                error = errorMessage(cause);
            }
        } finally {
            permissionSaving = false;
        }
    }

    async function handleModelChange(nextModelId: string) {
        selectedModelId = nextModelId;
        if (!active) return;
        const targetId = active.conversationId;
        try {
            await patchV2ConversationConfiguration(targetId, { selectedModelId: nextModelId });
            if (active?.conversationId === targetId) {
                active = { ...active, selectedModelId: nextModelId };
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
        try {
            const [loaded, cfgResult] = await Promise.allSettled([
                loadV2MessagesWithUsage(conv.conversationId),
                getV2ConversationConfiguration(conv.conversationId),
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
            await scrollToBottom(true);
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
            ...(selectedModelId ? { selectedModelId } : {}),
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
        void refreshModelChoices();

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
<svelte:head><title>{active?.title || t($locale, 'chatPage.title')} — Nola</title></svelte:head>
<ConfirmationDialog bind:this={confirmation} />

<div class="chat-shell" class:sidebar-collapsed={!sidebarOpen}>
    <aside class="chat-sidebar" aria-label={t($locale, 'chatInspector.list.conversations')}>
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

        <footer class="sidebar-footer">
            <button
                type="button"
                class="sidebar-action sidebar-footer-item"
                title={t($locale, 'shell.settings')}
                aria-label={t($locale, 'shell.settings')}
                onclick={() => openManagementTab('/settings')}
            >
                <PrototypeIcon name="settings" size={16} />
                {#if sidebarOpen}
                    <span>{t($locale, 'shell.settings')}</span>
                {/if}
            </button>
        </footer>
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
                                    <article class="message" class:user={msg.role === 'user'} class:assistant={msg.role === 'assistant'} data-message-id={msg.id}>
                                        <div class="message-head">
                                            <div class="message-role">{msg.role === 'user' ? t($locale, 'chatInspector.you') : 'Nola'}</div>
                                            <div class="message-actions">
                                                <button type="button" class="msg-action" onclick={() => copyMessage(msg)}>
                                                    {copiedMessageIds[msg.id] ? t($locale, 'chatInspector.copied') : t($locale, 'chatInspector.copy')}
                                                </button>
                                                {#if msg.role === 'user' && !streaming}
                                                    <button type="button" class="msg-action" onclick={() => requestEditAndResend(msgIndex)}>
                                                        {t($locale, 'chatInspector.edit')}
                                                    </button>
                                                {:else if msg.role === 'assistant' && !streaming}
                                                    <button type="button" class="msg-action" onclick={() => regenerateFrom(msgIndex)}>
                                                        {t($locale, 'chatInspector.regenerate')}
                                                    </button>
                                                {/if}
                                            </div>
                                        </div>

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
                                                    <span class="attachment-chip" class:has-thumb={Boolean(att.dataUrl)}>
                                                        {#if att.dataUrl}
                                                            <img class="attachment-thumb message-thumb" src={att.dataUrl} alt={att.name} />
                                                        {/if}
                                                        <span>{att.name}</span>
                                                    </span>
                                                {/each}
                                            </div>
                                        {/if}

                                        {#if msg.reasoning}
                                            <details class="reasoning-box">
                                                <summary>{t($locale, 'chatPage.thinking')}</summary>
                                                <pre>{msg.reasoning}</pre>
                                            </details>
                                        {/if}
                                        {#if msg.toolCalls?.length}
                                            <div class="tool-calls">
                                                {#each msg.toolCalls as tc (tc.callId)}
                                                    <details class="tool-card" data-status={tc.status}>
                                                        <summary>
                                                            <span class="tool-name">{tc.name}</span>
                                                            <span class="tool-status">
                                                                {tc.status === 'running'
                                                                    ? t($locale, 'chatPage.toolRunning')
                                                                    : tc.status === 'failed'
                                                                      ? t($locale, 'chatPage.toolFailed')
                                                                      : t($locale, 'chatPage.toolCompleted')}
                                                            </span>
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
                                                <strong>{t($locale, 'chatPermission.title')}</strong>
                                                <code class="permission-tool-badge">{pendingPermission.tool}</code>
                                            </div>
                                            {#if permissionSummary.headline && permissionSummary.headline !== pendingPermission.tool && permissionSummary.headline !== permissionSummary.target}
                                                <p class="permission-headline">{permissionSummary.headline}</p>
                                            {/if}
                                            {#if permissionSummary.target}
                                                <code class="permission-target">{permissionSummary.target}</code>
                                            {/if}
                                            <span class="muted permission-reason">{pendingPermission.reason || t($locale, 'chatPermission.ready')}</span>
                                        </header>
                                        <details class="permission-raw-details">
                                            <summary>{t($locale, 'chatPage.toolTechnicalDetails')}</summary>
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
                        {#if statusNotice}
                            <p class="notice" role="status">{statusNotice}</p>
                        {/if}
                        {#if error}
                            <p class="notice error" role="alert">{error}</p>
                        {/if}
                        {#if attachments.length || selectedExperts.length}
                            <div class="attachment-chips">
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
                                        {#if att.dataUrl}
                                            <img class="attachment-thumb" src={att.dataUrl} alt={att.name} />
                                        {/if}
                                        <span>{att.name}</span>
                                        <button type="button" class="chip-remove" aria-label={t($locale, 'shell.cancel')} onclick={() => removeAttachment(idx)}>×</button>
                                    </span>
                                {/each}
                            </div>
                        {/if}

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

                        <div class="composer-anchor">
                            {#if expertPickerOpen}
                                <div bind:this={expertPopupEl} class="expert-popup" role="listbox" aria-label={t($locale, 'rooms.experts')}>
                                    {#if expertsLoading}
                                        <p class="expert-popup-empty">{t($locale, 'rooms.loading')}</p>
                                    {:else if filteredExperts.length === 0}
                                        <p class="expert-popup-empty">{t($locale, 'rooms.noExperts')}</p>
                                    {:else}
                                        {#each filteredExperts as exp, idx (exp.id)}
                                            <button
                                                type="button"
                                                role="option"
                                                class="expert-option"
                                                class:highlighted={idx === expertHighlightIdx}
                                                aria-selected={idx === expertHighlightIdx}
                                                onclick={() => selectExpert(exp)}
                                            >
                                                <span class="expert-option-name">@{exp.name}</span>
                                                {#if exp.description}
                                                    <small class="muted">{exp.description}</small>
                                                {/if}
                                            </button>
                                        {/each}
                                    {/if}
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
                                <div class="composer-actions">
                                    {#if streaming}
                                        <button type="button" class="danger" onclick={cancelActiveRun}>
                                            {t($locale, 'chatPage.stop')}
                                        </button>
                                    {:else}
                                        <button type="submit" class="primary" disabled={(!draft.trim() && !attachments.length) || loading}>
                                            {t($locale, 'chatPage.send')}
                                        </button>
                                    {/if}
                                </div>
                            </form>
                        </div>
                    </footer>
                </div>

                {#if inspectorOpen}
                    <ConversationInspector
                        conversationId={active?.conversationId ?? null}
                        {messages}
                        usage={sessionUsage}
                        {streaming}
                        onclose={() => (inspectorOpen = false)}
                        onopenPage={openManagementTab}
                        oncompacted={summary => {
                            statusNotice = summary;
                            if (active) {
                                void reloadActiveConversation(active.conversationId, generation).catch(() => {});
                            }
                        }}
                    />
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
        grid-template-columns: 280px minmax(0, 1fr);
        height: 100dvh;
        min-height: 0;
        background: var(--canvas);
        color: var(--content);
    }
    .chat-shell.sidebar-collapsed {
        grid-template-columns: 56px minmax(0, 1fr);
    }
    .chat-sidebar {
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
        border-right: 1px solid var(--border);
        background: var(--panel);
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
        min-height: 44px;
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
    .topbar-pill,
    .toolbar-btn,
    .toolbar-select {
        width: auto;
        max-width: 220px;
        min-height: 30px;
        min-width: 0;
        padding: 4px 10px;
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--panel);
        color: var(--content);
        font-size: 12px;
        cursor: pointer;
    }
    .toolbar-select {
        padding: 4px 28px 4px 10px;
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
        border-color: var(--primary);
        background: var(--accent-subtle);
    }
    .topbar-pill:hover:not(:disabled),
    .toolbar-btn:hover:not(:disabled) {
        border-color: var(--primary);
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
    .transcript {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        padding: 24px 20px;
    }
    .scroll-anchor-bar {
        display: flex;
        justify-content: center;
        padding: 0 20px 6px;
    }
    .scroll-latest-btn {
        min-height: 28px;
        padding: 3px 12px;
        border-radius: 999px;
        font-size: 12px;
        background: var(--surface);
        border: 1px solid var(--border);
        color: var(--content);
        box-shadow: var(--glass-pill-shadow);
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
        gap: 18px;
    }
    .message {
        display: flex;
        flex-direction: column;
        gap: 8px;
        padding: 14px 16px;
        border-radius: 12px;
        border: 1px solid var(--border);
        background: var(--panel);
    }
    .message.user {
        background: var(--canvas);
    }
    .message-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }
    .message-role {
        font-size: 12px;
        font-weight: 600;
        color: var(--muted);
    }
    .message-actions {
        display: flex;
        align-items: center;
        gap: 4px;
    }
    .msg-action {
        padding: 2px 8px;
        min-height: 26px;
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
        border-color: var(--border);
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
    }
    .reasoning-box,
    .tool-card,
    .permission-raw-details {
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 6px 10px;
        background: var(--canvas);
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
        justify-content: space-between;
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
    .reasoning-box summary::after,
    .tool-card summary::after,
    .permission-raw-details summary::after {
        content: '';
        width: 12px;
        height: 12px;
        flex: none;
        background: currentColor;
        opacity: 0.65;
        -webkit-mask: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E") center / contain no-repeat;
        mask: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E") center / contain no-repeat;
        transition: transform var(--motion-fast) ease;
    }
    .reasoning-box[open] > summary::after,
    .tool-card[open] > summary::after,
    .permission-raw-details[open] > summary::after {
        transform: rotate(180deg);
    }
    .reasoning-box pre,
    .tool-summary,
    .interactive-args {
        margin: 8px 0 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        font-size: 12px;
        color: var(--muted);
    }
    .tool-calls,
    .ui-surfaces {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .tool-name {
        font-family: var(--font-code, monospace);
        color: var(--content);
        min-width: 0;
        overflow-wrap: anywhere;
    }
    .tool-status {
        display: inline-flex;
        align-items: center;
        margin-inline-start: auto;
        padding: 1px 8px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--panel);
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
        border: 1px solid var(--primary);
        border-radius: 12px;
        padding: 14px 16px;
        background: var(--panel);
        display: flex;
        flex-direction: column;
        gap: 10px;
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
    .permission-tool-badge {
        padding: 2px 7px;
        border-radius: 6px;
        background: var(--canvas);
        border: 1px solid var(--border);
        font-size: 12px;
    }
    .permission-headline {
        margin: 0;
        font-size: 13px;
        font-weight: 500;
    }
    .permission-target {
        display: block;
        padding: 6px 8px;
        border-radius: 6px;
        background: var(--canvas);
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
        padding: 10px 20px max(16px, env(safe-area-inset-bottom));
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .composer-toolbar {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 8px;
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
        font-size: 13px;
        font-weight: 600;
    }
    .composer-box {
        display: flex;
        align-items: flex-end;
        gap: 10px;
        padding: 10px 12px;
        border: 1px solid var(--composer-border);
        border-radius: 12px;
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
        flex: 1;
        min-width: 0;
        min-height: 44px;
        max-height: 220px;
        padding: 0;
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
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .notice {
        margin: 0;
        padding: 8px 12px;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--panel);
        font-size: 13px;
    }
    .notice.error {
        border-color: var(--danger);
        color: var(--danger);
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
            padding: 8px 12px max(12px, env(safe-area-inset-bottom));
        }
        .composer-toolbar {
            flex-wrap: nowrap;
            overflow-x: auto;
            scrollbar-width: none;
            -webkit-overflow-scrolling: touch;
            padding-bottom: 2px;
        }
        .composer-toolbar::-webkit-scrollbar {
            display: none;
        }
        .composer-toolbar > .toolbar-btn,
        .composer-toolbar > .toolbar-select {
            flex: 0 0 auto;
            white-space: nowrap;
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
