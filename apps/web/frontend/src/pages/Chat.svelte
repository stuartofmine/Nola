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
    import {
        createV2Conversation,
        getV2Conversation,
        listV2Conversations,
        loadV2Messages,
        streamV2ChatMessage,
        cancelV2Turn,
        hasConfiguredChatModel,
        type V2Message,
    } from '../lib/v2Chat';
    import ChatConversationManager from '../components/ChatConversationManager.svelte';
    import WorkspaceListNavigation from '../components/WorkspaceListNavigation.svelte';
    import WorkspaceResourceNavigation from '../components/WorkspaceResourceNavigation.svelte';
    import ConversationContentTabsView from '../components/ConversationContentTabs.svelte';
    import WorkspacePageTab from '../components/WorkspacePageTab.svelte';
    import Markdown from '../components/Markdown.svelte';
    import PrototypeIcon from '../components/PrototypeIcon.svelte';

    let viewportWidth = $state(typeof window === 'undefined' ? 1280 : window.innerWidth);
    let sidebarOpen = $state(true);
    let listCategory = $state<'conversations' | 'resources'>('conversations');
    let conversationManager = $state<ChatConversationManager>();

    let active = $state<ChatConversation | null>(null);
    let messages = $state<V2Message[]>([]);
    let draft = $state('');
    let loading = $state(false);
    let streaming = $state(false);
    let error = $state('');
    let needsModelConfig = $state(false);
    let generation = $state(0);
    let streamController: AbortController | null = null;
    let transcriptEl = $state<HTMLDivElement>();
    let composerEl = $state<HTMLTextAreaElement>();

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

    async function openConversation(conv: ChatConversation, preserveTab = false) {
        if (streaming) stopStreaming();
        const serial = ++generation;
        active = conv;
        error = '';
        needsModelConfig = false;
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
        const created = await createV2Conversation();
        active = created;
        syncUrl(created.conversationId);
        void conversationManager?.refresh();
        return created;
    }

    function stopStreaming() {
        streamController?.abort();
        streamController = null;
        if (active) void cancelV2Turn(active.conversationId).catch(() => {});
        streaming = false;
    }

    async function send() {
        const text = draft.trim();
        if (!text || streaming) return;
        error = '';
        needsModelConfig = false;

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
        draft = '';
        streaming = true;
        const userMsg: V2Message = {
            id: `u-${Date.now()}`,
            role: 'user',
            content: sentText,
            createdAtMs: Date.now(),
        };
        const assistantId = `a-${Date.now()}`;
        const assistantMsg: V2Message = {
            id: assistantId,
            role: 'assistant',
            content: '',
            reasoning: '',
            toolCalls: [],
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
            );
            if (serial === generation) {
                void conversationManager?.refresh();
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
            if (serial === generation) streaming = false;
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
                        {#each messages as msg (msg.id)}
                            <article class="message" class:user={msg.role === 'user'} class:assistant={msg.role === 'assistant'} data-message-id={msg.id}>
                                <div class="message-role">{msg.role === 'user' ? 'You' : 'Nola'}</div>
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
                {#if error}
                    <p class="notice error" role="alert">{error}</p>
                {/if}
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
                            <button type="submit" class="primary" disabled={!draft.trim() || loading}>
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
    .message-role {
        font-size: 12px;
        font-weight: 600;
        color: var(--muted);
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
    .tool-summary {
        margin: 8px 0 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        font-size: 12px;
        color: var(--muted);
    }
    .tool-calls {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .tool-name {
        font-family: var(--font-mono, monospace);
        color: var(--content);
    }
    .composer-wrap {
        max-width: 820px;
        width: 100%;
        margin: 0 auto;
        padding: 12px 20px max(16px, env(safe-area-inset-bottom));
        display: flex;
        flex-direction: column;
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
        padding: 10px 12px;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--panel);
        font-size: 13px;
    }
    .notice.error {
        color: var(--danger, var(--content));
    }
    .status-note {
        text-align: center;
        margin: 24px 0;
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
