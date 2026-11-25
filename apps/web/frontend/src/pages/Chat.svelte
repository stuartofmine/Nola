<script lang="ts">
    import { onMount, tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { darkAppearance, toggleAppearance } from '../lib/appearance';
    import { errorMessage } from '../lib/api';
    import { workspaceContentNavigationEvent, navigateWorkspace } from '../lib/workspaceNavigation';
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
        getV2Conversation,
        hasConfiguredChatModel,
        listV2Conversations,
        listV2ModelChoices,
        loadV2Messages,
        respondV2Permission,
        respondV2UserInput,
        streamV2ChatMessage,
        submitV2UiInteraction,
        truncateV2Conversation,
        type V2Attachment,
        type V2Message,
        type V2ModelChoice,
        type V2PermissionPrompt,
        type V2UserQuestion,
    } from '../lib/v2Chat';
    import ChatConversationManager from '../components/ChatConversationManager.svelte';
    import WorkspaceListNavigation from '../components/WorkspaceListNavigation.svelte';
    import WorkspaceResourceNavigation from '../components/WorkspaceResourceNavigation.svelte';
    import ConversationContentTabsView from '../components/ConversationContentTabs.svelte';
    import WorkspacePageTab from '../components/WorkspacePageTab.svelte';
    import Markdown from '../components/Markdown.svelte';
    import PrototypeIcon from '../components/PrototypeIcon.svelte';
    import Surface from '../vendor/a2ui-svelte/render/Surface.svelte';

    let viewportWidth = $state(typeof window === 'undefined' ? 1280 : window.innerWidth);
    let sidebarOpen = $state(true);
    let listCategory = $state<'conversations' | 'resources'>('conversations');
    let conversationManager = $state<ChatConversationManager>();

    let active = $state<ChatConversation | null>(null);
    let messages = $state<V2Message[]>([]);
    let draft = $state('');
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

    let modelChoices = $state<V2ModelChoice[]>([]);
    let selectedModelId = $state('');
    let permissionProfile = $state<'cautious' | 'accept_edits' | 'allow_all'>('accept_edits');

    let pendingPermission = $state<V2PermissionPrompt | null>(null);
    let pendingQuestion = $state<V2UserQuestion | null>(null);
    let questionAnswerDraft = $state('');
    let copiedMessageId = $state('');

    let contentTabs = $state<ConversationContentTabs>(emptyContentTabs());
    const tabViewports = new Map<string, { top: number }>();
    const selectedContentTab = $derived(contentTabs.items.find(item => item.key === contentTabs.selected));
    const managementContentSelected = $derived(selectedContentTab?.kind === 'page');
    const activeTabTitle = $derived(active?.title || t($locale, 'shell.conversations.new'));
    const ownerKey = $derived(active?.conversationId ?? 'workspace');

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

    async function scrollToBottom() {
        await tick();
        if (transcriptEl) transcriptEl.scrollTop = transcriptEl.scrollHeight;
    }

    async function refreshModelChoices() {
        try {
            const list = await listV2ModelChoices();
            modelChoices = list;
            if (!selectedModelId) {
                selectedModelId = list.find(m => m.selected)?.id ?? list[0]?.id ?? '';
            }
        } catch {
            // Optional toolbar metadata.
        }
    }

    async function openConversation(conv: ChatConversation, preserveTab = false) {
        if (streaming) stopStreaming();
        const serial = ++generation;
        active = conv;
        error = '';
        statusNotice = '';
        needsModelConfig = false;
        pendingPermission = null;
        pendingQuestion = null;
        if (conv.selectedModelId) selectedModelId = conv.selectedModelId;
        if (!preserveTab) {
            contentTabs = selectContentTab(contentTabs, conversationTabKey);
            syncUrl(conv.conversationId, null);
        } else {
            syncUrl(conv.conversationId);
        }
        loading = true;
        try {
            const loaded = await loadV2Messages(conv.conversationId);
            if (serial !== generation) return;
            messages = loaded;
            await scrollToBottom();
        } catch (cause) {
            if (serial !== generation) return;
            error = errorMessage(cause);
        } finally {
            if (serial === generation) loading = false;
        }
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

    async function ensureActiveConversation(): Promise<ChatConversation> {
        if (active) return active;
        const created = await createV2Conversation({
            ...(selectedModelId ? { selectedModelId } : {}),
        });
        active = created;
        syncUrl(created.conversationId);
        void conversationManager?.refresh();
        return created;
    }

    function stopStreaming() {
        streamController?.abort();
        streamController = null;
        pendingPermission = null;
        pendingQuestion = null;
        if (active) void cancelV2Turn(active.conversationId).catch(() => {});
        streaming = false;
    }

    async function handleFileSelect(event: Event) {
        const input = event.currentTarget as HTMLInputElement;
        const files = input.files;
        if (!files?.length) return;
        const next: V2Attachment[] = [...attachments];
        for (const file of Array.from(files)) {
            if (file.type.startsWith('image/')) {
                const dataUrl = await new Promise<string>((resolve, reject) => {
                    const reader = new FileReader();
                    reader.onload = () => resolve(String(reader.result ?? ''));
                    reader.onerror = () => reject(reader.error);
                    reader.readAsDataURL(file);
                });
                next.push({ name: file.name, mimeType: file.type, dataUrl });
            } else {
                const text = await file.text();
                next.push({ name: file.name, mimeType: file.type || 'text/plain', text: text.slice(0, 64_000) });
            }
        }
        attachments = next;
        input.value = '';
    }

    function removeAttachment(index: number) {
        attachments = attachments.filter((_, idx) => idx !== index);
    }

    async function sendPrompt(rawText: string, attached: V2Attachment[] = []) {
        const text = rawText.trim();
        if ((!text && !attached.length) || streaming) return;
        error = '';
        statusNotice = '';
        needsModelConfig = false;

        // Lightweight slash commands
        if (text === '/compact' && active) {
            try {
                const res = await compactV2Conversation(active.conversationId);
                statusNotice = res.summary || 'Context compacted';
                draft = '';
            } catch (cause) {
                error = errorMessage(cause);
            }
            return;
        }
        if (text === '/clear') {
            draft = '';
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
        draft = '';
        attachments = [];
        streaming = true;
        pendingPermission = null;
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
        await scrollToBottom();

        const controller = new AbortController();
        streamController = controller;
        const serial = generation;

        const updateAssistant = (fn: (m: V2Message) => V2Message) => {
            messages = messages.map(m => (m.id === assistantId ? fn(m) : m));
        };

        try {
            await streamV2ChatMessage(
                conv.conversationId,
                sentText,
                {
                    onTextDelta: delta => {
                        if (serial !== generation) return;
                        updateAssistant(m => ({ ...m, content: m.content + delta }));
                        void scrollToBottom();
                    },
                    onReasoningDelta: delta => {
                        if (serial !== generation) return;
                        updateAssistant(m => ({ ...m, reasoning: (m.reasoning ?? '') + delta }));
                    },
                    onToolStarted: (callId, name, args) => {
                        if (serial !== generation) return;
                        updateAssistant(m => ({
                            ...m,
                            toolCalls: [
                                ...(m.toolCalls ?? []),
                                { callId, name, arguments: args, status: 'running' },
                            ],
                        }));
                        void scrollToBottom();
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
                        void scrollToBottom();
                    },
                    onPermissionRequested: prompt => {
                        if (serial !== generation) return;
                        pendingPermission = prompt;
                        void scrollToBottom();
                    },
                    onUserQuestion: question => {
                        if (serial !== generation) return;
                        pendingQuestion = question;
                        questionAnswerDraft = '';
                        void scrollToBottom();
                    },
                    onTurnCompleted: finalText => {
                        if (serial !== generation) return;
                        if (finalText) {
                            updateAssistant(m => (m.content ? m : { ...m, content: finalText }));
                        }
                    },
                    onError: msg => {
                        if (serial !== generation) return;
                        if (msg.includes('no_model_configured') || msg.includes('model')) {
                            needsModelConfig = true;
                        }
                        error = msg;
                    },
                },
                controller.signal,
                {
                    modelId: selectedModelId || undefined,
                    permissionProfile,
                    attachments: sentAttachments,
                },
            );
            if (serial === generation) {
                void conversationManager?.refresh();
                // Reload canonical seq numbers in background without flashing UI
                void loadV2Messages(conv.conversationId)
                    .then(canonical => {
                        if (serial === generation && !streaming && canonical.length) {
                            messages = canonical;
                        }
                    })
                    .catch(() => {});
            }
        } catch (cause) {
            if (serial !== generation) return;
            if (!controller.signal.aborted) {
                const msg = errorMessage(cause);
                if (msg.includes('no_model_configured')) {
                    needsModelConfig = true;
                    draft = sentText;
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
                pendingQuestion = null;
            }
        }
    }

    async function send() {
        await sendPrompt(draft, attachments);
    }

    async function handlePermissionDecision(allow: boolean) {
        const prompt = pendingPermission;
        if (!prompt || !active) return;
        pendingPermission = null;
        try {
            await respondV2Permission(active.conversationId, prompt.requestId, allow);
        } catch (cause) {
            error = errorMessage(cause);
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
    ): Promise<UiSubmissionOutcome> {
        if (!active) return { status: 'rejected', message: 'No active conversation' };
        const result = await submitV2UiInteraction(active.conversationId, interaction);
        if (result.snapshot) {
            replaceSurfaceSnapshot(result.snapshot);
        }
        if (result.outcome.status === 'committed' && result.actionId && result.summary) {
            // Stream the agent's follow-up reaction to the submitted A2UI form/action!
            void sendPrompt(result.summary);
        }
        return result.outcome;
    }

    async function copyMessage(msg: V2Message) {
        try {
            await navigator.clipboard?.writeText(msg.content);
            copiedMessageId = msg.id;
            setTimeout(() => {
                if (copiedMessageId === msg.id) copiedMessageId = '';
            }, 1500);
        } catch {
            // Ignore clipboard errors in restricted contexts.
        }
    }

    async function editAndResend(msg: V2Message, index: number) {
        if (streaming || !active) return;
        if (typeof msg.seq === 'number') {
            await truncateV2Conversation(active.conversationId, msg.seq);
        }
        messages = messages.slice(0, index);
        draft = msg.content;
        await tick();
        composerEl?.focus();
    }

    async function regenerateFrom(index: number) {
        if (streaming || !active) return;
        // Find the preceding user message
        let userIdx = index - 1;
        while (userIdx >= 0 && messages[userIdx].role !== 'user') userIdx--;
        if (userIdx < 0) return;
        const userMsg = messages[userIdx];
        if (typeof userMsg.seq === 'number') {
            await truncateV2Conversation(active.conversationId, userMsg.seq);
        }
        messages = messages.slice(0, userIdx);
        await sendPrompt(userMsg.content, userMsg.attachments ?? []);
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
        window.addEventListener(workspaceContentNavigationEvent, handleContentNav);
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
            streamController?.abort();
        };
    });
</script>

<svelte:window bind:innerWidth={viewportWidth} />
<svelte:head><title>{active?.title || t($locale, 'chatPage.title')} — Nola</title></svelte:head>

<div class="chat-shell" class:sidebar-collapsed={!sidebarOpen}>
    <aside class="chat-sidebar" aria-label={t($locale, 'chatInspector.list.conversations')}>
        <div class="sidebar-top">
            <WorkspaceListNavigation category={listCategory} visible={sidebarOpen} onchange={setListCategory} />
            <div class="sidebar-tools">
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
                                active = null;
                                messages = [];
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

            <footer class="sidebar-footer">
                <button
                    type="button"
                    class="sidebar-action"
                    onclick={() => openManagementTab('/settings')}
                >
                    <PrototypeIcon name="settings" size={16} />
                    <span>{t($locale, 'shell.settings')}</span>
                </button>
                <a class="sidebar-action" href="/models" onclick={e => { e.preventDefault(); openManagementTab('/models'); }}>
                    <PrototypeIcon name="models" size={16} />
                    <span>{t($locale, 'shell.nav.models')}</span>
                </a>
            </footer>
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
                onselect={key => {
                    contentTabs = selectContentTab(contentTabs, key);
                    const sel = contentTabs.items.find(item => item.key === contentTabs.selected);
                    syncUrl(active?.conversationId ?? null, sel?.kind === 'page' ? sel.path : null);
                }}
                onclose={key => {
                    contentTabs = closeContentTab(contentTabs, key);
                    const sel = contentTabs.items.find(item => item.key === contentTabs.selected);
                    syncUrl(active?.conversationId ?? null, sel?.kind === 'page' ? sel.path : null);
                }}
                onrename={(id, nextTitle) => conversationManager?.renameConversationById(id, nextTitle)}
            />
            <div class="topbar-actions">
                {#if active}
                    <button
                        type="button"
                        class="topbar-pill"
                        onclick={() => openManagementTab(`/workspace-review?conversation=${encodeURIComponent(active!.conversationId)}`)}
                    >
                        Diff
                    </button>
                    <button
                        type="button"
                        class="topbar-pill"
                        disabled={streaming || !messages.length}
                        onclick={branchActiveConversation}
                    >
                        Branch
                    </button>
                {/if}
            </div>
        </header>

        <div
            class="chat-stage"
            role="tabpanel"
            id={contentTabDomId('panel', ownerKey, conversationTabKey)}
            aria-labelledby={contentTabDomId('tab', ownerKey, conversationTabKey)}
            hidden={managementContentSelected}
        >
            <div class="transcript" bind:this={transcriptEl}>
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
                                    <div class="message-role">{msg.role === 'user' ? 'You' : 'Nola'}</div>
                                    <div class="message-actions">
                                        <button type="button" class="msg-action" onclick={() => copyMessage(msg)}>
                                            {copiedMessageId === msg.id ? 'Copied' : 'Copy'}
                                        </button>
                                        {#if msg.role === 'user' && !streaming}
                                            <button type="button" class="msg-action" onclick={() => editAndResend(msg, msgIndex)}>
                                                Edit
                                            </button>
                                        {:else if msg.role === 'assistant' && !streaming}
                                            <button type="button" class="msg-action" onclick={() => regenerateFrom(msgIndex)}>
                                                Regenerate
                                            </button>
                                        {/if}
                                    </div>
                                </div>

                                {#if msg.attachments?.length}
                                    <div class="attachment-chips">
                                        {#each msg.attachments as att}
                                            <span class="attachment-chip">{att.name}</span>
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
                                                oninteraction={input => handleSurfaceInteraction(input)}
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
                                            <Markdown source={msg.content} />
                                        {:else if streaming}
                                            <p class="muted">{t($locale, 'chatPage.working')}</p>
                                        {/if}
                                    {:else}
                                        <p class="user-text">{msg.content}</p>
                                    {/if}
                                </div>
                            </article>
                        {/each}

                        {#if pendingPermission}
                            <div class="interactive-card permission-card" role="region" aria-label="Tool approval required">
                                <header class="interactive-head">
                                    <strong>Approval Required: <code>{pendingPermission.tool}</code></strong>
                                    <span class="muted">{pendingPermission.reason}</span>
                                </header>
                                <pre class="interactive-args">{JSON.stringify(pendingPermission.arguments, null, 2)}</pre>
                                <div class="interactive-actions">
                                    <button type="button" class="primary" onclick={() => handlePermissionDecision(true)}>
                                        Approve
                                    </button>
                                    <button type="button" class="danger" onclick={() => handlePermissionDecision(false)}>
                                        Deny
                                    </button>
                                </div>
                            </div>
                        {/if}

                        {#if pendingQuestion}
                            <div class="interactive-card question-card" role="region" aria-label="Agent question">
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
                                        placeholder="Type your response..."
                                        aria-label="Response to agent question"
                                    />
                                    <button type="submit" class="primary" disabled={!questionAnswerDraft.trim()}>
                                        Submit
                                    </button>
                                </form>
                            </div>
                        {/if}
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
                {#if statusNotice}
                    <p class="notice" role="status">{statusNotice}</p>
                {/if}
                {#if error}
                    <p class="notice error" role="alert">{error}</p>
                {/if}
                {#if attachments.length}
                    <div class="attachment-chips">
                        {#each attachments as att, idx}
                            <span class="attachment-chip">
                                {att.name}
                                <button type="button" class="chip-remove" aria-label="Remove attachment" onclick={() => removeAttachment(idx)}>×</button>
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
                        Attach
                    </button>
                    {#if modelChoices.length}
                        <select
                            class="toolbar-select"
                            aria-label="Model"
                            bind:value={selectedModelId}
                            disabled={streaming}
                        >
                            {#each modelChoices as choice (choice.id)}
                                <option value={choice.id}>{choice.displayName}</option>
                            {/each}
                        </select>
                    {/if}
                    <select
                        class="toolbar-select"
                        aria-label="Permission mode"
                        bind:value={permissionProfile}
                        disabled={streaming}
                    >
                        <option value="cautious">Cautious (Ask)</option>
                        <option value="accept_edits">Accept Edits</option>
                        <option value="allow_all">Allow All</option>
                    </select>
                </div>
                <form
                    class="composer-box"
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
                        disabled={loading}
                        onkeydown={onComposerKeydown}
                    ></textarea>
                    <div class="composer-actions">
                        {#if streaming}
                            <button type="button" class="danger" onclick={stopStreaming}>
                                {t($locale, 'chatPage.stop')}
                            </button>
                        {:else}
                            <button type="submit" class="primary" disabled={(!draft.trim() && !attachments.length) || loading}>
                                {t($locale, 'chatPage.send')}
                            </button>
                        {/if}
                    </div>
                </form>
            </footer>
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
                        ownerKey={`${ownerKey}:${tab.key}`}
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
        border-right: 1px solid var(--border);
        background: var(--panel);
        padding: 10px 8px;
        gap: 10px;
    }
    .sidebar-top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 6px;
    }
    .sidebar-tools {
        display: flex;
        align-items: center;
        gap: 4px;
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
    .sidebar-action {
        display: flex;
        align-items: center;
        gap: 8px;
        min-height: 36px;
        padding: 6px 10px;
        border: 1px solid transparent;
        border-radius: 8px;
        background: transparent;
        color: var(--content);
        text-decoration: none;
        font-size: 13px;
        cursor: pointer;
    }
    .sidebar-action:hover {
        background: var(--composer-pill-hover-bg);
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
        padding: 4px 10px;
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--panel);
        color: var(--content);
        font-size: 12px;
        cursor: pointer;
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
    .user-text {
        margin: 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }
    .reasoning-box,
    .tool-card {
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 6px 10px;
        background: var(--canvas);
        font-size: 13px;
    }
    .reasoning-box summary,
    .tool-card summary {
        cursor: pointer;
        color: var(--muted);
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
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
        font-family: var(--font-mono, monospace);
        color: var(--content);
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
        gap: 4px;
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
    .chip-remove {
        border: 0;
        background: transparent;
        color: var(--muted);
        cursor: pointer;
        padding: 0 2px;
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
    .composer-box {
        display: flex;
        align-items: flex-end;
        gap: 10px;
        padding: 10px 12px;
        border: 1px solid var(--border);
        border-radius: 12px;
        background: var(--panel);
    }
    #chat-composer {
        flex: 1;
        min-width: 0;
        min-height: 44px;
        max-height: 220px;
        resize: vertical;
        border: 0;
        background: transparent;
        color: var(--content);
        font: inherit;
        line-height: 1.5;
    }
    #chat-composer:focus {
        outline: none;
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
    @media (max-width: 760px) {
        .chat-shell {
            grid-template-columns: minmax(0, 1fr);
        }
        .chat-sidebar {
            display: none;
        }
    }
</style>
