<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { errorMessage } from '../lib/api';
    import {
        clearV2ConversationGoal,
        compactV2Conversation,
        getV2ConversationConfiguration,
        getV2ConversationGoal,
        getV2WorkspaceDiff,
        listV2ConversationDataSources,
        listV2WorkspaceCheckpoints,
        patchV2ConversationConfiguration,
        redoV2Workspace,
        saveV2ConversationDataSources,
        saveV2ConversationGoal,
        undoV2Workspace,
        type V2ConversationGoal,
        type V2DataSourceItem,
        type V2Message,
        type V2TokenUsage,
        type V2WorkspaceCheckpoint,
        type V2WorkspaceDiffSnapshot,
    } from '../lib/v2Chat';
    import PrototypeIcon from './PrototypeIcon.svelte';

    type InspectorTab = 'tasks' | 'diff' | 'context' | 'data-sources' | 'knowledge';

    let {
        conversationId,
        messages,
        usage,
        streaming = false,
        onclose,
        onopenPage,
        oncompacted,
    }: {
        conversationId: string | null;
        messages: V2Message[];
        usage: V2TokenUsage;
        streaming?: boolean;
        onclose: () => void;
        onopenPage?: (path: string) => void;
        oncompacted?: (summary: string) => void;
    } = $props();

    let activeTab = $state<InspectorTab>('tasks');

    // Tab 1: Tasks & Goal
    let goal = $state<V2ConversationGoal | null>(null);
    let goalTitleDraft = $state('');
    let goalDescDraft = $state('');
    let goalLoading = $state(false);
    let goalSaving = $state(false);
    let goalError = $state('');

    // Tab 2: Diff & Checkpoints
    let diffSnapshot = $state<V2WorkspaceDiffSnapshot | null>(null);
    let checkpoints = $state<V2WorkspaceCheckpoint[]>([]);
    let diffLoading = $state(false);
    let diffBusy = $state(false);
    let diffNotice = $state('');
    let diffError = $state('');

    // Tab 3: Context & Cache
    let compacting = $state(false);
    let compactNotice = $state('');
    let compactError = $state('');

    // Tab 4: Data Sources
    let dataSources = $state<V2DataSourceItem[]>([]);
    let dataSourcesLoading = $state(false);
    let dataSourcesSaving = $state(false);
    let dataSourcesError = $state('');

    // Tab 5: Knowledge
    let knowledgeDraft = $state('');
    let savedKnowledge = $state('');
    let knowledgeLoading = $state(false);
    let knowledgeSaving = $state(false);
    let knowledgeNotice = $state('');
    let knowledgeError = $state('');

    const liveTasks = $derived.by(() => {
        const items: Array<{ id: string; label: string; status: 'running' | 'completed' | 'failed'; detail?: string }> = [];
        for (const msg of messages) {
            for (const tc of msg.toolCalls ?? []) {
                // Extract structured todos if planning/todo tools were invoked
                const args = tc.arguments && typeof tc.arguments === 'object' ? (tc.arguments as Record<string, unknown>) : null;
                const rawTodos = args && Array.isArray(args.todos) ? args.todos : args && Array.isArray(args.tasks) ? args.tasks : null;
                if (rawTodos && rawTodos.length > 0) {
                    for (let i = 0; i < rawTodos.length; i++) {
                        const row = rawTodos[i] as Record<string, unknown>;
                        const title = String(row.content ?? row.title ?? row.subject ?? row.task ?? '').trim();
                        if (!title) continue;
                        const st = String(row.status ?? '').toLowerCase();
                        const mappedStatus: 'running' | 'completed' | 'failed' =
                            st === 'completed' || st === 'done'
                                ? 'completed'
                                : st === 'failed' || st === 'error'
                                  ? 'failed'
                                  : 'running';
                        items.push({ id: `${tc.callId}-todo-${i}`, label: title, status: mappedStatus });
                    }
                } else {
                    items.push({
                        id: tc.callId,
                        label: tc.name,
                        status: tc.status,
                        detail: tc.summary,
                    });
                }
            }
        }
        return items.slice(-20).reverse();
    });

    const userMsgCount = $derived(messages.filter(m => m.role === 'user').length);
    const assistantMsgCount = $derived(messages.filter(m => m.role === 'assistant').length);
    const knowledgeDirty = $derived(knowledgeDraft !== savedKnowledge);

    async function loadGoal(convId: string) {
        goalLoading = true;
        goalError = '';
        try {
            const loaded = await getV2ConversationGoal(convId);
            if (conversationId !== convId) return;
            goal = loaded;
            goalTitleDraft = loaded?.title ?? '';
            goalDescDraft = loaded?.description ?? '';
        } catch (cause) {
            if (conversationId === convId) goalError = errorMessage(cause);
        } finally {
            if (conversationId === convId) goalLoading = false;
        }
    }

    async function saveGoal() {
        if (!conversationId || !goalTitleDraft.trim() || goalSaving) return;
        const convId = conversationId;
        goalSaving = true;
        goalError = '';
        try {
            const saved = await saveV2ConversationGoal(convId, {
                title: goalTitleDraft,
                description: goalDescDraft,
                status: 'active',
            });
            if (conversationId !== convId) return;
            goal = saved;
            goalTitleDraft = saved?.title ?? goalTitleDraft.trim();
            goalDescDraft = saved?.description ?? '';
        } catch (cause) {
            if (conversationId === convId) goalError = errorMessage(cause);
        } finally {
            if (conversationId === convId) goalSaving = false;
        }
    }

    async function clearGoal() {
        if (!conversationId || goalSaving) return;
        const convId = conversationId;
        goalSaving = true;
        goalError = '';
        try {
            await clearV2ConversationGoal(convId);
            if (conversationId !== convId) return;
            goal = null;
            goalTitleDraft = '';
            goalDescDraft = '';
        } catch (cause) {
            if (conversationId === convId) goalError = errorMessage(cause);
        } finally {
            if (conversationId === convId) goalSaving = false;
        }
    }

    async function loadDiffAndCheckpoints(convId: string) {
        diffLoading = true;
        diffError = '';
        try {
            const [diffRes, cpRes] = await Promise.allSettled([
                getV2WorkspaceDiff(convId),
                listV2WorkspaceCheckpoints(convId),
            ]);
            if (conversationId !== convId) return;
            if (diffRes.status === 'fulfilled') {
                diffSnapshot = diffRes.value;
            } else {
                diffError = errorMessage(diffRes.reason);
            }
            if (cpRes.status === 'fulfilled') {
                checkpoints = cpRes.value;
            }
        } finally {
            if (conversationId === convId) diffLoading = false;
        }
    }

    async function handleWorkspaceAction(kind: 'undo' | 'redo') {
        if (!conversationId || diffBusy) return;
        const convId = conversationId;
        diffBusy = true;
        diffError = '';
        diffNotice = '';
        try {
            const res = kind === 'undo' ? await undoV2Workspace(convId) : await redoV2Workspace(convId);
            if (conversationId !== convId) return;
            diffNotice = res.message || (kind === 'undo' ? t($locale, 'chatInspector.undo') : t($locale, 'chatInspector.redo'));
            await loadDiffAndCheckpoints(convId);
        } catch (cause) {
            if (conversationId === convId) diffError = errorMessage(cause);
        } finally {
            if (conversationId === convId) diffBusy = false;
        }
    }

    async function handleCompact() {
        if (!conversationId || compacting || streaming) return;
        const convId = conversationId;
        compacting = true;
        compactError = '';
        compactNotice = '';
        try {
            const res = await compactV2Conversation(convId);
            if (conversationId !== convId) return;
            compactNotice = res.summary || t($locale, 'chatPage.compactionRecord');
            oncompacted?.(compactNotice);
        } catch (cause) {
            if (conversationId === convId) compactError = errorMessage(cause);
        } finally {
            if (conversationId === convId) compacting = false;
        }
    }

    async function loadDataSources(convId: string) {
        dataSourcesLoading = true;
        dataSourcesError = '';
        try {
            const items = await listV2ConversationDataSources(convId);
            if (conversationId !== convId) return;
            dataSources = items;
        } catch (cause) {
            if (conversationId === convId) dataSourcesError = errorMessage(cause);
        } finally {
            if (conversationId === convId) dataSourcesLoading = false;
        }
    }

    async function toggleDataSource(sourceId: string) {
        if (!conversationId || dataSourcesSaving) return;
        const convId = conversationId;
        const previous = dataSources;
        const next = dataSources.map(item => (item.id === sourceId ? { ...item, enabled: !item.enabled } : item));
        dataSources = next;
        dataSourcesSaving = true;
        dataSourcesError = '';
        try {
            const enabledIds = next.filter(item => item.enabled).map(item => item.id);
            await saveV2ConversationDataSources(convId, enabledIds);
        } catch (cause) {
            if (conversationId === convId) {
                dataSources = previous;
                dataSourcesError = errorMessage(cause);
            }
        } finally {
            if (conversationId === convId) dataSourcesSaving = false;
        }
    }

    async function loadKnowledge(convId: string) {
        knowledgeLoading = true;
        knowledgeError = '';
        knowledgeNotice = '';
        try {
            const cfg = await getV2ConversationConfiguration(convId);
            if (conversationId !== convId) return;
            savedKnowledge = cfg.knowledge;
            knowledgeDraft = cfg.knowledge;
        } catch (cause) {
            if (conversationId === convId) knowledgeError = errorMessage(cause);
        } finally {
            if (conversationId === convId) knowledgeLoading = false;
        }
    }

    async function saveKnowledge() {
        if (!conversationId || knowledgeSaving) return;
        const convId = conversationId;
        knowledgeSaving = true;
        knowledgeError = '';
        knowledgeNotice = '';
        try {
            await patchV2ConversationConfiguration(convId, {
                knowledge: { keyPointsMarkdown: knowledgeDraft, summary: knowledgeDraft },
            });
            if (conversationId !== convId) return;
            savedKnowledge = knowledgeDraft;
            knowledgeNotice = t($locale, 'chatKnowledge.saved');
        } catch (cause) {
            if (conversationId === convId) knowledgeError = errorMessage(cause);
        } finally {
            if (conversationId === convId) knowledgeSaving = false;
        }
    }

    function refreshCurrentTab() {
        if (!conversationId) return;
        if (activeTab === 'tasks') void loadGoal(conversationId);
        else if (activeTab === 'diff') void loadDiffAndCheckpoints(conversationId);
        else if (activeTab === 'data-sources') void loadDataSources(conversationId);
        else if (activeTab === 'knowledge') void loadKnowledge(conversationId);
    }

    $effect(() => {
        const convId = conversationId;
        const tab = activeTab;
        if (!convId) {
            goal = null;
            goalTitleDraft = '';
            goalDescDraft = '';
            diffSnapshot = null;
            checkpoints = [];
            dataSources = [];
            savedKnowledge = '';
            knowledgeDraft = '';
            return;
        }
        if (tab === 'tasks') void loadGoal(convId);
        else if (tab === 'diff') void loadDiffAndCheckpoints(convId);
        else if (tab === 'data-sources') void loadDataSources(convId);
        else if (tab === 'knowledge') void loadKnowledge(convId);
    });
</script>

<aside
    class="inspector-drawer"
    data-testid="conversation-inspector"
    aria-label={t($locale, 'chatPage.details')}
>
    <header class="inspector-header">
        <h2 class="inspector-title">{t($locale, 'chatPage.details')}</h2>
        <div class="inspector-header-actions">
            {#if conversationId && activeTab !== 'context'}
                <button
                    type="button"
                    class="inspector-icon-btn"
                    title={t($locale, 'chatConversations.refresh')}
                    aria-label={t($locale, 'chatConversations.refresh')}
                    onclick={refreshCurrentTab}
                >
                    ↻
                </button>
            {/if}
            <button
                type="button"
                class="inspector-icon-btn"
                title={t($locale, 'chatInspector.close')}
                aria-label={t($locale, 'chatInspector.close')}
                onclick={onclose}
            >
                <PrototypeIcon name="close" size={16} />
            </button>
        </div>
    </header>

    <div class="inspector-tabs" role="tablist" aria-label={t($locale, 'chatInspector.panel')}>
        <button
            type="button"
            role="tab"
            class="inspector-tab"
            class:active={activeTab === 'tasks'}
            aria-selected={activeTab === 'tasks'}
            onclick={() => (activeTab = 'tasks')}
        >
            {t($locale, 'chatInspector.tabTasks')}
        </button>
        <button
            type="button"
            role="tab"
            class="inspector-tab"
            class:active={activeTab === 'diff'}
            aria-selected={activeTab === 'diff'}
            onclick={() => (activeTab = 'diff')}
        >
            {t($locale, 'chatInspector.tabDiff')}
        </button>
        <button
            type="button"
            role="tab"
            class="inspector-tab"
            class:active={activeTab === 'context'}
            aria-selected={activeTab === 'context'}
            onclick={() => (activeTab = 'context')}
        >
            {t($locale, 'chatInspector.tabContext')}
        </button>
        <button
            type="button"
            role="tab"
            class="inspector-tab"
            class:active={activeTab === 'data-sources'}
            aria-selected={activeTab === 'data-sources'}
            onclick={() => (activeTab = 'data-sources')}
        >
            {t($locale, 'chatInspector.tabDataSources')}
        </button>
        <button
            type="button"
            role="tab"
            class="inspector-tab"
            class:active={activeTab === 'knowledge'}
            aria-selected={activeTab === 'knowledge'}
            onclick={() => (activeTab = 'knowledge')}
        >
            {t($locale, 'chatInspector.tabKnowledge')}
        </button>
    </div>

    <div class="inspector-body">
        {#if !conversationId}
            <p class="inspector-muted">{t($locale, 'chatInspector.noConversationSelected')}</p>
        {:else if activeTab === 'tasks'}
            <section class="inspector-section">
                <h3>{t($locale, 'chatInspector.goal')}</h3>
                {#if goalLoading}
                    <p class="inspector-muted">{t($locale, 'rooms.loading')}</p>
                {:else}
                    {#if goal}
                        <div class="goal-active-card">
                            <strong>{goal.title}</strong>
                            {#if goal.description}
                                <p class="inspector-muted">{goal.description}</p>
                            {/if}
                        </div>
                    {:else}
                        <p class="inspector-muted">{t($locale, 'chatInspector.noGoal')}</p>
                    {/if}
                    <form
                        class="inspector-form"
                        onsubmit={e => {
                            e.preventDefault();
                            void saveGoal();
                        }}
                    >
                        <input
                            type="text"
                            bind:value={goalTitleDraft}
                            placeholder={t($locale, 'chatInspector.goalTitlePlaceholder')}
                            disabled={goalSaving}
                        />
                        <textarea
                            rows="2"
                            bind:value={goalDescDraft}
                            placeholder={t($locale, 'chatInspector.goalDescPlaceholder')}
                            disabled={goalSaving}
                        ></textarea>
                        <div class="inspector-btn-row">
                            <button type="submit" class="primary compact-btn" disabled={goalSaving || !goalTitleDraft.trim()}>
                                {goal ? t($locale, 'chatInspector.updateGoal') : t($locale, 'chatInspector.setGoal')}
                            </button>
                            {#if goal}
                                <button type="button" class="compact-btn" disabled={goalSaving} onclick={clearGoal}>
                                    {t($locale, 'chatInspector.clearGoal')}
                                </button>
                            {/if}
                        </div>
                    </form>
                    {#if goalError}
                        <p class="inspector-error" role="alert">{goalError}</p>
                    {/if}
                {/if}
            </section>

            <section class="inspector-section">
                <h3>{t($locale, 'chatInspector.liveTasks')}</h3>
                {#if liveTasks.length === 0}
                    <p class="inspector-muted">{t($locale, 'chatInspector.noLiveTasks')}</p>
                {:else}
                    <ul class="task-list">
                        {#each liveTasks as item (item.id)}
                            <li class="task-item" data-status={item.status}>
                                <span class="task-dot" aria-hidden="true">
                                    {item.status === 'completed' ? '✓' : item.status === 'failed' ? '!' : '•'}
                                </span>
                                <div class="task-copy">
                                    <span class="task-label">{item.label}</span>
                                    {#if item.detail}
                                        <small class="inspector-muted">{item.detail.slice(0, 120)}</small>
                                    {/if}
                                </div>
                            </li>
                        {/each}
                    </ul>
                {/if}
            </section>
        {:else if activeTab === 'diff'}
            <section class="inspector-section">
                <div class="section-row">
                    <h3>{t($locale, 'chatInspector.review')}</h3>
                    <div class="inspector-btn-row">
                        <button type="button" class="compact-btn" disabled={diffBusy || diffLoading} onclick={() => void handleWorkspaceAction('undo')}>
                            {t($locale, 'chatInspector.undo')}
                        </button>
                        <button type="button" class="compact-btn" disabled={diffBusy || diffLoading} onclick={() => void handleWorkspaceAction('redo')}>
                            {t($locale, 'chatInspector.redo')}
                        </button>
                    </div>
                </div>
                {#if diffNotice}
                    <p class="inspector-notice" role="status">{diffNotice}</p>
                {/if}
                {#if diffError}
                    <p class="inspector-error" role="alert">{diffError}</p>
                {/if}
                {#if diffLoading}
                    <p class="inspector-muted">{t($locale, 'rooms.loading')}</p>
                {:else if diffSnapshot}
                    {#if diffSnapshot.isGitRepository}
                        {#if diffSnapshot.entries.length === 0 && !diffSnapshot.stagedText && !diffSnapshot.unstagedText && !diffSnapshot.rawDiffText}
                            <p class="inspector-muted">{t($locale, 'chatInspector.noChanges')}</p>
                        {:else}
                            {#if diffSnapshot.entries.length > 0}
                                <ul class="diff-file-list">
                                    {#each diffSnapshot.entries as entry (entry.path)}
                                        <li class="diff-file-item">
                                            <code class="diff-status">{(entry.index + entry.worktree).trim() || 'M'}</code>
                                            <span class="diff-path">{entry.path}</span>
                                        </li>
                                    {/each}
                                </ul>
                            {/if}
                            {#if diffSnapshot.unstagedText}
                                <details class="diff-details" open>
                                    <summary>{t($locale, 'chatInspector.unstaged')}</summary>
                                    <pre class="diff-pre">{diffSnapshot.unstagedText}</pre>
                                </details>
                            {/if}
                            {#if diffSnapshot.stagedText}
                                <details class="diff-details">
                                    <summary>{t($locale, 'chatInspector.staged')}</summary>
                                    <pre class="diff-pre">{diffSnapshot.stagedText}</pre>
                                </details>
                            {/if}
                            {#if !diffSnapshot.unstagedText && !diffSnapshot.stagedText && diffSnapshot.rawDiffText}
                                <pre class="diff-pre">{diffSnapshot.rawDiffText}</pre>
                            {/if}
                        {/if}
                    {:else if diffSnapshot.files.length > 0}
                        <ul class="diff-file-list">
                            {#each diffSnapshot.files.slice(0, 40) as file (file.path)}
                                <li class="diff-file-item">
                                    <span class="diff-path">{file.isDir ? `${file.path}/` : file.path}</span>
                                </li>
                            {/each}
                        </ul>
                    {:else}
                        <p class="inspector-muted">{t($locale, 'chatInspector.emptyWorkspace')}</p>
                    {/if}
                {/if}
            </section>

            <section class="inspector-section">
                <h3>{t($locale, 'chatInspector.checkpoints')}</h3>
                {#if checkpoints.length === 0}
                    <p class="inspector-muted">{t($locale, 'chatInspector.noCheckpoints')}</p>
                {:else}
                    <ul class="checkpoint-list">
                        {#each checkpoints as cp (cp.id)}
                            <li class="checkpoint-item">
                                <strong>{cp.label}</strong>
                                {#if cp.createdAt}<small class="inspector-muted">{cp.createdAt}</small>{/if}
                                {#if cp.summary}<small class="inspector-muted">{cp.summary}</small>{/if}
                            </li>
                        {/each}
                    </ul>
                {/if}
            </section>
        {:else if activeTab === 'context'}
            <section class="inspector-section">
                <h3>{t($locale, 'chatInspector.tabContext')}</h3>
                <div class="metrics-grid">
                    <div class="metric-card">
                        <span class="metric-label">{t($locale, 'chatInspector.inputTokens')}</span>
                        <strong class="metric-value">{usage.input_tokens.toLocaleString()}</strong>
                    </div>
                    <div class="metric-card">
                        <span class="metric-label">{t($locale, 'chatInspector.outputTokens')}</span>
                        <strong class="metric-value">{usage.output_tokens.toLocaleString()}</strong>
                    </div>
                    <div class="metric-card">
                        <span class="metric-label">{t($locale, 'chatInspector.cacheReadTokens')}</span>
                        <strong class="metric-value">{usage.cache_read_tokens.toLocaleString()}</strong>
                    </div>
                    <div class="metric-card">
                        <span class="metric-label">{t($locale, 'chatInspector.cacheHitRate')}</span>
                        <strong class="metric-value">{usage.cache_hit_rate.toFixed(1)}%</strong>
                    </div>
                </div>
                <div class="metric-card full-row">
                    <span class="metric-label">{t($locale, 'chatInspector.messageCount')}</span>
                    <strong class="metric-value">{messages.length} ({userMsgCount} / {assistantMsgCount})</strong>
                </div>
                <div class="inspector-btn-row">
                    <button
                        type="button"
                        class="primary compact-btn"
                        disabled={compacting || streaming || messages.length === 0}
                        onclick={handleCompact}
                    >
                        {compacting ? t($locale, 'chatInspector.compacting') : t($locale, 'chatInspector.compactContext')}
                    </button>
                </div>
                {#if compactNotice}
                    <p class="inspector-notice" role="status">{compactNotice}</p>
                {/if}
                {#if compactError}
                    <p class="inspector-error" role="alert">{compactError}</p>
                {/if}
            </section>
        {:else if activeTab === 'data-sources'}
            <section class="inspector-section">
                <div class="section-row">
                    <h3>{t($locale, 'chatDataSources.title')}</h3>
                    {#if onopenPage}
                        <button type="button" class="compact-btn" onclick={() => onopenPage?.('/data-sources')}>
                            {t($locale, 'chatDataSources.manage')}
                        </button>
                    {/if}
                </div>
                <p class="inspector-muted">{t($locale, 'chatDataSources.scope')}</p>
                {#if dataSourcesLoading}
                    <p class="inspector-muted">{t($locale, 'chatDataSources.loading')}</p>
                {:else if dataSources.length === 0}
                    <p class="inspector-muted">{t($locale, 'chatDataSources.empty')}</p>
                {:else}
                    <ul class="source-list">
                        {#each dataSources as src (src.id)}
                            <li class="source-item">
                                <label class="source-toggle">
                                    <input
                                        type="checkbox"
                                        checked={src.enabled}
                                        disabled={dataSourcesSaving}
                                        onchange={() => void toggleDataSource(src.id)}
                                    />
                                    <div class="source-meta">
                                        <strong>{src.name}</strong>
                                        <small class="inspector-muted">{src.kind}{src.location ? ` · ${src.location}` : ''}</small>
                                    </div>
                                </label>
                            </li>
                        {/each}
                    </ul>
                {/if}
                {#if dataSourcesError}
                    <p class="inspector-error" role="alert">{dataSourcesError}</p>
                {/if}
            </section>
        {:else if activeTab === 'knowledge'}
            <section class="inspector-section">
                <h3>{t($locale, 'chatInspector.tabKnowledge')}</h3>
                <p class="inspector-muted">{t($locale, 'chatKnowledge.hint')}</p>
                {#if knowledgeLoading}
                    <p class="inspector-muted">{t($locale, 'chatKnowledge.loading')}</p>
                {:else}
                    {#if !savedKnowledge && !knowledgeDraft}
                        <p class="inspector-muted">{t($locale, 'chatInspector.knowledgeEmpty')}</p>
                    {/if}
                    <textarea
                        class="knowledge-textarea"
                        rows="10"
                        bind:value={knowledgeDraft}
                        placeholder={t($locale, 'chatInspector.knowledgeSummaryPlaceholder')}
                        disabled={knowledgeSaving}
                    ></textarea>
                    <div class="inspector-btn-row">
                        <button
                            type="button"
                            class="primary compact-btn"
                            disabled={knowledgeSaving || !knowledgeDirty}
                            onclick={saveKnowledge}
                        >
                            {knowledgeSaving ? t($locale, 'rooms.saving') : t($locale, 'chatKnowledge.save')}
                        </button>
                        <button
                            type="button"
                            class="compact-btn"
                            disabled={knowledgeSaving}
                            onclick={() => conversationId && void loadKnowledge(conversationId)}
                        >
                            {t($locale, 'chatKnowledge.reload')}
                        </button>
                    </div>
                    {#if knowledgeNotice}
                        <p class="inspector-notice" role="status">{knowledgeNotice}</p>
                    {/if}
                    {#if knowledgeError}
                        <p class="inspector-error" role="alert">{knowledgeError}</p>
                    {/if}
                {/if}
            </section>
        {/if}
    </div>
</aside>

<style>
    .inspector-drawer {
        width: 340px;
        flex-shrink: 0;
        display: flex;
        flex-direction: column;
        min-height: 0;
        border-left: 1px solid var(--border);
        background: var(--panel);
        color: var(--content);
    }
    .inspector-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 10px 12px;
        border-bottom: 1px solid var(--border);
        gap: 8px;
    }
    .inspector-title {
        margin: 0;
        font-size: 13px;
        font-weight: 600;
    }
    .inspector-header-actions {
        display: flex;
        align-items: center;
        gap: 4px;
    }
    .inspector-icon-btn {
        display: grid;
        place-items: center;
        width: 28px;
        height: 28px;
        min-width: 28px;
        min-height: 28px;
        padding: 0;
        border: 1px solid transparent;
        border-radius: 6px;
        background: transparent;
        color: var(--muted);
        cursor: pointer;
    }
    .inspector-icon-btn:hover {
        color: var(--content);
        background: var(--composer-pill-hover-bg);
    }
    .inspector-tabs {
        display: flex;
        flex-wrap: wrap;
        gap: 4px;
        padding: 8px 10px;
        border-bottom: 1px solid var(--border);
        background: var(--canvas);
    }
    .inspector-tab {
        padding: 4px 8px;
        min-height: 28px;
        min-width: 0;
        border: 1px solid transparent;
        border-radius: 6px;
        background: transparent;
        color: var(--muted);
        font-size: 12px;
        cursor: pointer;
    }
    .inspector-tab:hover {
        color: var(--content);
        background: var(--composer-pill-hover-bg);
    }
    .inspector-tab.active {
        background: var(--panel);
        border-color: var(--border);
        color: var(--content);
        font-weight: 600;
    }
    .inspector-body {
        flex: 1;
        min-height: 0;
        overflow-y: auto;
        padding: 12px;
        display: flex;
        flex-direction: column;
        gap: 16px;
    }
    .inspector-section {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .inspector-section h3 {
        margin: 0;
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        color: var(--muted);
    }
    .section-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }
    .inspector-muted {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .inspector-notice {
        margin: 0;
        padding: 6px 8px;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--canvas);
        font-size: 12px;
    }
    .inspector-error {
        margin: 0;
        padding: 6px 8px;
        border-radius: 6px;
        border: 1px solid var(--danger);
        color: var(--danger);
        font-size: 12px;
    }
    .goal-active-card {
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        display: flex;
        flex-direction: column;
        gap: 4px;
        font-size: 13px;
    }
    .inspector-form {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .inspector-form input,
    .inspector-form textarea,
    .knowledge-textarea {
        width: 100%;
        padding: 7px 9px;
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--canvas);
        color: var(--content);
        font-size: 12px;
        resize: vertical;
    }
    .inspector-btn-row {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .compact-btn {
        min-height: 30px;
        min-width: 0;
        padding: 4px 10px;
        font-size: 12px;
        border-radius: 7px;
    }
    .task-list,
    .diff-file-list,
    .checkpoint-list,
    .source-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .task-item {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        padding: 6px 8px;
        border-radius: 7px;
        border: 1px solid var(--border);
        background: var(--canvas);
        font-size: 12px;
    }
    .task-dot {
        font-weight: 700;
        color: var(--muted);
    }
    .task-item[data-status='completed'] .task-dot {
        color: var(--success);
    }
    .task-item[data-status='failed'] .task-dot {
        color: var(--danger);
    }
    .task-item[data-status='running'] .task-dot {
        color: var(--info);
    }
    .task-copy {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
        overflow-wrap: anywhere;
    }
    .task-label {
        font-family: var(--font-code, monospace);
        font-size: 12px;
    }
    .diff-file-item {
        display: flex;
        align-items: center;
        gap: 6px;
        padding: 4px 6px;
        border-radius: 6px;
        background: var(--canvas);
        border: 1px solid var(--border);
        font-size: 12px;
    }
    .diff-status {
        font-size: 11px;
        color: var(--info);
        min-width: 18px;
    }
    .diff-path {
        overflow-wrap: anywhere;
        font-family: var(--font-code, monospace);
    }
    .diff-details {
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--canvas);
        padding: 6px 8px;
        font-size: 12px;
    }
    .diff-details summary {
        cursor: pointer;
        color: var(--muted);
    }
    .diff-pre {
        margin: 6px 0 0;
        max-height: 240px;
        overflow: auto;
        font-size: 11px;
        white-space: pre;
    }
    .checkpoint-item {
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding: 6px 8px;
        border-radius: 7px;
        border: 1px solid var(--border);
        background: var(--canvas);
        font-size: 12px;
    }
    .metrics-grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 8px;
    }
    .metric-card {
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding: 8px 10px;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--canvas);
    }
    .metric-card.full-row {
        flex-direction: row;
        align-items: center;
        justify-content: space-between;
    }
    .metric-label {
        font-size: 11px;
        color: var(--muted);
    }
    .metric-value {
        font-size: 14px;
        font-variant-numeric: tabular-nums;
    }
    .source-item {
        padding: 7px 9px;
        border-radius: 8px;
        border: 1px solid var(--border);
        background: var(--canvas);
    }
    .source-toggle {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        cursor: pointer;
    }
    .source-meta {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
        overflow-wrap: anywhere;
        font-size: 12px;
    }
    @media (max-width: 1024px) {
        .inspector-drawer {
            width: 290px;
        }
    }
    @media (max-width: 760px) {
        .inspector-drawer {
            position: fixed;
            inset: 44px 0 0 auto;
            width: min(92vw, 340px);
            z-index: 30;
            box-shadow: var(--dialog-shadow);
        }
    }
</style>
