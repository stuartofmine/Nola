<script lang="ts">
    import { onDestroy } from 'svelte';
    import { locale, t, type Locale } from '../lib/i18n';
    import { errorMessage } from '../lib/api';
    import {
        clearV2ConversationGoal,
        compactV2Conversation,
        getV2ConversationConfiguration,
        getV2ConversationGoal,
        getV2ConversationPlan,
        getV2WorkspaceDiff,
        indexV2DataSource,
        listV2ConversationDataSources,
        listV2WorkspaceCheckpoints,
        patchV2ConversationConfiguration,
        redoV2Workspace,
        saveV2ConversationDataSources,
        saveV2ConversationGoal,
        stopV2ConversationTask,
        subscribeV2ConversationTasks,
        undoV2Workspace,
        updateV2ConversationPlan,
        type V2ConversationGoal,
        type V2ConversationPlan,
        type V2DataSourceItem,
        type V2Message,
        type V2SubagentTask,
        type V2TokenUsage,
        type V2WorkspaceCheckpoint,
        type V2WorkspaceDiffSnapshot,
    } from '../lib/v2Chat';
    import InspectorArtifactsPanel from './InspectorArtifactsPanel.svelte';
    import InspectorBrowserPdfPanel from './InspectorBrowserPdfPanel.svelte';
    import InspectorRecallPanel from './InspectorRecallPanel.svelte';
    import PrototypeIcon from './PrototypeIcon.svelte';
    import WorkspaceWorktreesPanel from './WorkspaceWorktreesPanel.svelte';

    type InspectorTab =
        | 'tasks'
        | 'artifacts'
        | 'recall'
        | 'diff'
        | 'browser'
        | 'context'
        | 'data-sources'
        | 'knowledge';

    let {
        conversationId,
        messages,
        usage,
        streaming = false,
        width,
        plan = null,
        onclose,
        onopenPage,
        oncompacted,
        onplanchange,
    }: {
        conversationId: string | null;
        messages: V2Message[];
        usage: V2TokenUsage;
        streaming?: boolean;
        width?: number;
        plan?: V2ConversationPlan | null;
        onclose: () => void;
        onopenPage?: (path: string) => void;
        oncompacted?: (summary: string) => void;
        onplanchange?: (next: V2ConversationPlan) => void;
    } = $props();

    let activeTab = $state<InspectorTab>('tasks');
    let refreshToken = $state(0);

    const inspectorTabs: ReadonlyArray<{ id: InspectorTab; labelKey: string }> = [
        { id: 'tasks', labelKey: 'chatInspector.tabTasks' },
        { id: 'artifacts', labelKey: 'chatInspector.tabArtifacts' },
        { id: 'recall', labelKey: 'chatInspector.tabRecall' },
        { id: 'diff', labelKey: 'chatInspector.tabDiff' },
        { id: 'browser', labelKey: 'chatInspector.tabBrowser' },
        { id: 'context', labelKey: 'chatInspector.tabContext' },
        { id: 'data-sources', labelKey: 'chatInspector.tabDataSources' },
        { id: 'knowledge', labelKey: 'chatInspector.tabKnowledge' },
    ];

    const shortTabLabels: Record<InspectorTab, Record<Locale, string>> = {
        tasks: { 'zh-Hans': '任务', en: 'Tasks', de: 'Aufg.', es: 'Tareas', fr: 'Tâches', ja: 'タスク', ru: 'Задачи' },
        artifacts: { 'zh-Hans': '产物', en: 'Artifacts', de: 'Artef.', es: 'Artef.', fr: 'Livr.', ja: '成果物', ru: 'Артеф.' },
        recall: { 'zh-Hans': '召回', en: 'Recall', de: 'Suche', es: 'Buscar', fr: 'Rappel', ja: '検索', ru: 'Поиск' },
        diff: { 'zh-Hans': '差异', en: 'Diff', de: 'Diff', es: 'Diff', fr: 'Diff', ja: '差分', ru: 'Diff' },
        browser: { 'zh-Hans': '浏览', en: 'Browser', de: 'Browser', es: 'Web/PDF', fr: 'Web/PDF', ja: 'ブラウズ', ru: 'Веб/PDF' },
        context: { 'zh-Hans': '上下文', en: 'Context', de: 'Kontext', es: 'Contexto', fr: 'Contexte', ja: '文脈', ru: 'Контекст' },
        'data-sources': { 'zh-Hans': '数据源', en: 'Sources', de: 'Quellen', es: 'Fuentes', fr: 'Sources', ja: 'ソース', ru: 'Источн.' },
        knowledge: { 'zh-Hans': '知识', en: 'Knowledge', de: 'Wissen', es: 'Saber', fr: 'Savoir', ja: '知識', ru: 'Знания' },
    };

    function handleTabKeydown(event: KeyboardEvent & { currentTarget: HTMLButtonElement }, index: number) {
        const nextIndex =
            event.key === 'ArrowRight'
                ? (index + 1) % inspectorTabs.length
                : event.key === 'ArrowLeft'
                  ? (index + inspectorTabs.length - 1) % inspectorTabs.length
                  : event.key === 'Home'
                    ? 0
                    : event.key === 'End'
                      ? inspectorTabs.length - 1
                      : null;
        if (nextIndex === null) return;
        event.preventDefault();
        activeTab = inspectorTabs[nextIndex].id;
        const buttons = event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
        const target = buttons?.[nextIndex];
        target?.focus();
        target?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    }

    // Tab 1: Plan Mode, Goal & Live Subagent Tasks
    let localPlan = $state<V2ConversationPlan>({ enabled: false, status: 'idle', steps: [] });
    let planBusy = $state(false);
    let planError = $state('');
    let goal = $state<V2ConversationGoal | null>(null);
    let goalTitleDraft = $state('');
    let goalDescDraft = $state('');
    let goalLoading = $state(false);
    let goalSaving = $state(false);
    let goalError = $state('');
    let subagentTasks = $state<V2SubagentTask[]>([]);
    let stoppingTaskId = $state<string | null>(null);
    let taskError = $state('');

    const effectivePlan = $derived(plan ?? localPlan);

    // Tab 2: Diff, Checkpoints & Worktrees
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

    // Tab 4: Data Sources + Index Now
    let dataSources = $state<V2DataSourceItem[]>([]);
    let dataSourcesLoading = $state(false);
    let dataSourcesSaving = $state(false);
    let indexingSourceIds = $state<Record<string, boolean>>({});
    let dataSourcesNotice = $state('');
    let dataSourcesError = $state('');

    // Tab 5: Knowledge (key points and wiki kept strictly separate)
    let keyPointsDraft = $state('');
    let savedKeyPoints = $state('');
    let wikiDraft = $state('');
    let savedWiki = $state('');
    let knowledgeLoading = $state(false);
    let knowledgeSaving = $state(false);
    let knowledgeNotice = $state('');
    let knowledgeError = $state('');

    let disposed = false;
    let tabController: AbortController | null = null;
    let tasksSseController: AbortController | null = null;
    let actionController: AbortController | null = null;

    function isAbortError(cause: unknown): boolean {
        return cause instanceof DOMException
            ? cause.name === 'AbortError'
            : cause instanceof Error && cause.name === 'AbortError';
    }

    function nextTabSignal(): AbortSignal {
        tabController?.abort();
        tabController = new AbortController();
        return tabController.signal;
    }

    onDestroy(() => {
        disposed = true;
        tabController?.abort();
        tabController = null;
        tasksSseController?.abort();
        tasksSseController = null;
        actionController?.abort();
        actionController = null;
    });

    const fallbackToolTasks = $derived.by(() => {
        const items: V2SubagentTask[] = [];
        for (const msg of messages) {
            for (const tc of msg.toolCalls ?? []) {
                const args = tc.arguments && typeof tc.arguments === 'object' ? (tc.arguments as Record<string, unknown>) : null;
                const rawTodos = args && Array.isArray(args.todos) ? args.todos : args && Array.isArray(args.tasks) ? args.tasks : null;
                if (rawTodos && rawTodos.length > 0) {
                    for (let i = 0; i < rawTodos.length; i++) {
                        const row = rawTodos[i] as Record<string, unknown>;
                        const title = String(row.content ?? row.title ?? row.subject ?? row.task ?? '').trim();
                        if (!title) continue;
                        const st = String(row.status ?? '').toLowerCase();
                        const mappedStatus: V2SubagentTask['status'] =
                            st === 'completed' || st === 'done'
                                ? 'completed'
                                : st === 'failed' || st === 'error'
                                  ? 'failed'
                                  : 'running';
                        items.push({ id: `${tc.callId}-todo-${i}`, title, status: mappedStatus, kind: 'todo' });
                    }
                } else {
                    items.push({
                        id: tc.callId,
                        title: tc.name,
                        status: tc.status,
                        detail: tc.summary,
                        kind: 'tool',
                    });
                }
            }
        }
        return items.slice(-20).reverse();
    });

    const displayedTasks = $derived(subagentTasks.length > 0 ? subagentTasks : fallbackToolTasks);

    const rootTasks = $derived.by(() => {
        const ids = new Set(displayedTasks.map(item => item.id));
        return displayedTasks.filter(item => !item.parentId || !ids.has(item.parentId));
    });

    function childTasksOf(parentId: string): V2SubagentTask[] {
        return displayedTasks.filter(item => item.parentId === parentId);
    }

    const userMsgCount = $derived(messages.filter(m => m.role === 'user').length);
    const assistantMsgCount = $derived(messages.filter(m => m.role === 'assistant').length);
    const knowledgeDirty = $derived(keyPointsDraft !== savedKeyPoints || wikiDraft !== savedWiki);

    async function loadGoalAndPlan(convId: string, signal: AbortSignal) {
        goalLoading = true;
        goalError = '';
        planError = '';
        try {
            const [goalRes, planRes] = await Promise.allSettled([
                getV2ConversationGoal(convId, signal),
                getV2ConversationPlan(convId, signal),
            ]);
            if (disposed || signal.aborted || conversationId !== convId) return;
            if (goalRes.status === 'fulfilled') {
                goal = goalRes.value;
                goalTitleDraft = goalRes.value?.title ?? '';
                goalDescDraft = goalRes.value?.description ?? '';
            } else if (!isAbortError(goalRes.reason)) {
                goalError = errorMessage(goalRes.reason);
            }
            if (planRes.status === 'fulfilled') {
                localPlan = planRes.value;
                onplanchange?.(planRes.value);
            }
        } finally {
            if (!disposed && !signal.aborted && conversationId === convId) goalLoading = false;
        }
    }

    function connectTasksEvents(convId: string) {
        tasksSseController?.abort();
        tasksSseController = new AbortController();
        const signal = tasksSseController.signal;
        taskError = '';
        void subscribeV2ConversationTasks(
            convId,
            {
                onTasks: items => {
                    if (!disposed && !signal.aborted && conversationId === convId) {
                        subagentTasks = items;
                    }
                },
                onTaskUpdate: item => {
                    if (disposed || signal.aborted || conversationId !== convId) return;
                    const idx = subagentTasks.findIndex(t => t.id === item.id);
                    if (idx >= 0) {
                        subagentTasks = subagentTasks.map((t, i) => (i === idx ? item : t));
                    } else {
                        subagentTasks = [item, ...subagentTasks];
                    }
                },
            },
            signal,
        ).catch(cause => {
            if (!disposed && !signal.aborted && !isAbortError(cause)) {
                // Graceful fallback to transcript-derived tasks without noisy error banners
            }
        });
    }

    async function handleStopTask(task: V2SubagentTask) {
        if (!conversationId || stoppingTaskId) return;
        stoppingTaskId = task.id;
        taskError = '';
        try {
            await stopV2ConversationTask(conversationId, task.id);
            if (!disposed) {
                subagentTasks = subagentTasks.map(item =>
                    item.id === task.id ? { ...item, status: 'cancelled' } : item,
                );
            }
        } catch (cause) {
            if (!disposed) taskError = errorMessage(cause);
        } finally {
            if (!disposed) stoppingTaskId = null;
        }
    }

    async function handlePlanAction(
        payload: { enabled?: boolean; action?: 'enable' | 'disable' | 'approve' | 'reject' | 'clear' },
    ) {
        if (!conversationId || planBusy) return;
        const convId = conversationId;
        planBusy = true;
        planError = '';
        try {
            const updated = await updateV2ConversationPlan(convId, payload);
            if (disposed || conversationId !== convId) return;
            localPlan = updated;
            onplanchange?.(updated);
        } catch (cause) {
            if (!disposed && conversationId === convId) planError = errorMessage(cause);
        } finally {
            if (!disposed && conversationId === convId) planBusy = false;
        }
    }

    async function saveGoal() {
        if (!conversationId || !goalTitleDraft.trim() || goalSaving) return;
        const convId = conversationId;
        goalSaving = true;
        goalError = '';
        actionController?.abort();
        actionController = new AbortController();
        const signal = actionController.signal;
        try {
            const saved = await saveV2ConversationGoal(
                convId,
                { title: goalTitleDraft, description: goalDescDraft, status: 'active' },
                signal,
            );
            if (disposed || signal.aborted || conversationId !== convId) return;
            goal = saved;
            goalTitleDraft = saved?.title ?? goalTitleDraft.trim();
            goalDescDraft = saved?.description ?? '';
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                goalError = errorMessage(cause);
            }
        } finally {
            if (!disposed && conversationId === convId) goalSaving = false;
        }
    }

    async function clearGoal() {
        if (!conversationId || goalSaving) return;
        const convId = conversationId;
        goalSaving = true;
        goalError = '';
        actionController?.abort();
        actionController = new AbortController();
        const signal = actionController.signal;
        try {
            await clearV2ConversationGoal(convId, signal);
            if (disposed || signal.aborted || conversationId !== convId) return;
            goal = null;
            goalTitleDraft = '';
            goalDescDraft = '';
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                goalError = errorMessage(cause);
            }
        } finally {
            if (!disposed && conversationId === convId) goalSaving = false;
        }
    }

    async function loadDiffAndCheckpoints(convId: string, signal: AbortSignal) {
        diffLoading = true;
        diffError = '';
        try {
            const [diffRes, cpRes] = await Promise.allSettled([
                getV2WorkspaceDiff(convId, signal),
                listV2WorkspaceCheckpoints(convId, signal),
            ]);
            if (disposed || signal.aborted || conversationId !== convId) return;
            if (diffRes.status === 'fulfilled') {
                diffSnapshot = diffRes.value;
            } else if (!isAbortError(diffRes.reason)) {
                diffError = errorMessage(diffRes.reason);
            }
            if (cpRes.status === 'fulfilled') {
                checkpoints = cpRes.value;
            }
        } finally {
            if (!disposed && !signal.aborted && conversationId === convId) diffLoading = false;
        }
    }

    async function handleWorkspaceAction(kind: 'undo' | 'redo') {
        if (!conversationId || diffBusy) return;
        const convId = conversationId;
        diffBusy = true;
        diffError = '';
        diffNotice = '';
        actionController?.abort();
        actionController = new AbortController();
        const signal = actionController.signal;
        try {
            const res = kind === 'undo'
                ? await undoV2Workspace(convId, signal)
                : await redoV2Workspace(convId, signal);
            if (disposed || signal.aborted || conversationId !== convId) return;
            diffNotice = res.message || (kind === 'undo' ? t($locale, 'chatInspector.undo') : t($locale, 'chatInspector.redo'));
            await loadDiffAndCheckpoints(convId, nextTabSignal());
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                diffError = errorMessage(cause);
            }
        } finally {
            if (!disposed && conversationId === convId) diffBusy = false;
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
            if (disposed || conversationId !== convId) return;
            compactNotice = res.summary || t($locale, 'chatPage.compactionRecord');
            oncompacted?.(compactNotice);
        } catch (cause) {
            if (!disposed && conversationId === convId) compactError = errorMessage(cause);
        } finally {
            if (!disposed && conversationId === convId) compacting = false;
        }
    }

    async function loadDataSources(convId: string, signal: AbortSignal) {
        dataSourcesLoading = true;
        dataSourcesError = '';
        dataSourcesNotice = '';
        try {
            const items = await listV2ConversationDataSources(convId, signal);
            if (disposed || signal.aborted || conversationId !== convId) return;
            dataSources = items;
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                dataSourcesError = errorMessage(cause);
            }
        } finally {
            if (!disposed && !signal.aborted && conversationId === convId) dataSourcesLoading = false;
        }
    }

    async function toggleDataSource(sourceId: string) {
        if (!conversationId || dataSourcesSaving) return;
        const target = dataSources.find(item => item.id === sourceId);
        if (!target || target.blocked || target.accessPolicy === 'blocked') return;
        const convId = conversationId;
        const previous = dataSources;
        const next = dataSources.map(item => (item.id === sourceId ? { ...item, enabled: !item.enabled } : item));
        dataSources = next;
        dataSourcesSaving = true;
        dataSourcesError = '';
        actionController?.abort();
        actionController = new AbortController();
        const signal = actionController.signal;
        try {
            const enabledIds = next
                .filter(item => !item.blocked && item.accessPolicy !== 'blocked' && item.enabled)
                .map(item => item.id);
            const disabledIds = next
                .filter(item => !item.blocked && item.accessPolicy !== 'blocked' && !item.enabled)
                .map(item => item.id);
            await saveV2ConversationDataSources(convId, enabledIds, disabledIds, signal);
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                dataSources = previous;
                dataSourcesError = errorMessage(cause);
            }
        } finally {
            if (!disposed && conversationId === convId) dataSourcesSaving = false;
        }
    }

    async function handleIndexDataSource(src: V2DataSourceItem) {
        if (indexingSourceIds[src.id]) return;
        indexingSourceIds = { ...indexingSourceIds, [src.id]: true };
        dataSourcesError = '';
        dataSourcesNotice = '';
        try {
            const res = await indexV2DataSource(src.id);
            if (disposed) return;
            const chunksNote = typeof res.indexedChunks === 'number' ? ` (${res.indexedChunks})` : '';
            dataSourcesNotice = res.message || `${src.name}: ${t($locale, 'chatInspector.indexed')}${chunksNote}`;
        } catch (cause) {
            if (!disposed) dataSourcesError = errorMessage(cause);
        } finally {
            if (!disposed) {
                const copy = { ...indexingSourceIds };
                delete copy[src.id];
                indexingSourceIds = copy;
            }
        }
    }

    async function loadKnowledge(convId: string, signal: AbortSignal) {
        knowledgeLoading = true;
        knowledgeError = '';
        knowledgeNotice = '';
        try {
            const cfg = await getV2ConversationConfiguration(convId, signal);
            if (disposed || signal.aborted || conversationId !== convId) return;
            savedKeyPoints = cfg.keyPointsMarkdown;
            keyPointsDraft = cfg.keyPointsMarkdown;
            savedWiki = cfg.wikiMarkdown;
            wikiDraft = cfg.wikiMarkdown;
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                knowledgeError = errorMessage(cause);
            }
        } finally {
            if (!disposed && !signal.aborted && conversationId === convId) knowledgeLoading = false;
        }
    }

    async function saveKnowledge() {
        if (!conversationId || knowledgeSaving || !knowledgeDirty) return;
        const convId = conversationId;
        knowledgeSaving = true;
        knowledgeError = '';
        knowledgeNotice = '';
        actionController?.abort();
        actionController = new AbortController();
        const signal = actionController.signal;
        const nextKeyPoints = keyPointsDraft;
        const nextWiki = wikiDraft;
        const patch: { keyPointsMarkdown?: string; wikiMarkdown?: string; summary?: string } = {};
        if (nextKeyPoints !== savedKeyPoints) {
            patch.keyPointsMarkdown = nextKeyPoints;
            patch.summary = nextKeyPoints;
        }
        if (nextWiki !== savedWiki) {
            patch.wikiMarkdown = nextWiki;
        }
        try {
            await patchV2ConversationConfiguration(convId, { knowledge: patch }, signal);
            if (disposed || signal.aborted || conversationId !== convId) return;
            savedKeyPoints = nextKeyPoints;
            savedWiki = nextWiki;
            knowledgeNotice = t($locale, 'chatKnowledge.saved');
        } catch (cause) {
            if (!disposed && !signal.aborted && !isAbortError(cause) && conversationId === convId) {
                knowledgeError = errorMessage(cause);
            }
        } finally {
            if (!disposed && conversationId === convId) knowledgeSaving = false;
        }
    }

    function refreshCurrentTab() {
        if (!conversationId) return;
        refreshToken++;
        const signal = nextTabSignal();
        if (activeTab === 'tasks') {
            void loadGoalAndPlan(conversationId, signal);
            connectTasksEvents(conversationId);
        } else if (activeTab === 'diff') {
            void loadDiffAndCheckpoints(conversationId, signal);
        } else if (activeTab === 'data-sources') {
            void loadDataSources(conversationId, signal);
        } else if (activeTab === 'knowledge') {
            void loadKnowledge(conversationId, signal);
        }
    }

    $effect(() => {
        const convId = conversationId;
        const tab = activeTab;
        if (!convId) {
            tabController?.abort();
            tabController = null;
            tasksSseController?.abort();
            tasksSseController = null;
            goal = null;
            goalTitleDraft = '';
            goalDescDraft = '';
            subagentTasks = [];
            localPlan = { enabled: false, status: 'idle', steps: [] };
            diffSnapshot = null;
            checkpoints = [];
            dataSources = [];
            savedKeyPoints = '';
            keyPointsDraft = '';
            savedWiki = '';
            wikiDraft = '';
            compactNotice = '';
            compactError = '';
            return;
        }
        const signal = nextTabSignal();
        if (tab === 'tasks') {
            void loadGoalAndPlan(convId, signal);
            connectTasksEvents(convId);
        } else {
            tasksSseController?.abort();
            tasksSseController = null;
            if (tab === 'diff') void loadDiffAndCheckpoints(convId, signal);
            else if (tab === 'data-sources') void loadDataSources(convId, signal);
            else if (tab === 'knowledge') void loadKnowledge(convId, signal);
        }
        return () => {
            tabController?.abort();
            tabController = null;
            tasksSseController?.abort();
            tasksSseController = null;
        };
    });
</script>

<svelte:window
    onkeydown={event => {
        if (event.key === 'Escape' && !document.querySelector('dialog:modal')) {
            event.preventDefault();
            onclose();
        }
    }}
/>

<div class="inspector-scrim" aria-hidden="true" onclick={onclose}></div>

<aside
    id="conversation-inspector-panel"
    class="inspector-drawer"
    style:width={width ? `${width}px` : undefined}
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
                    <svg class="inspector-refresh-icon" aria-hidden="true" viewBox="0 0 24 24">
                        <path d="M20 7v5h-5M20 12a8 8 0 1 0-2 5" />
                    </svg>
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

    <div class="inspector-tabs-wrap">
        <div class="inspector-tabs" role="tablist" aria-label={t($locale, 'chatInspector.panel')}>
            {#each inspectorTabs as tab, idx (tab.id)}
                {@const fullLabel = t($locale, tab.labelKey)}
                <button
                    type="button"
                    role="tab"
                    id={`inspector-tab-${tab.id}`}
                    aria-controls={`inspector-panel-${tab.id}`}
                    class="inspector-tab"
                    class:active={activeTab === tab.id}
                    title={fullLabel}
                    aria-label={fullLabel}
                    aria-selected={activeTab === tab.id}
                    tabindex={activeTab === tab.id ? 0 : -1}
                    onclick={() => (activeTab = tab.id)}
                    onkeydown={event => handleTabKeydown(event, idx)}
                >
                    {shortTabLabels[tab.id][$locale]}
                </button>
            {/each}
        </div>
    </div>

    <div
        class="inspector-body"
        role="tabpanel"
        id={`inspector-panel-${activeTab}`}
        aria-labelledby={`inspector-tab-${activeTab}`}
    >
        {#if !conversationId}
            <p class="inspector-muted">{t($locale, 'chatInspector.noConversationSelected')}</p>
        {:else if activeTab === 'tasks'}
            <section class="inspector-section">
                <div class="section-row">
                    <h3>{t($locale, 'chatInspector.planMode')}</h3>
                    <button
                        type="button"
                        class="compact-btn"
                        class:primary={effectivePlan.enabled}
                        disabled={planBusy}
                        onclick={() =>
                            void handlePlanAction({
                                enabled: !effectivePlan.enabled,
                                action: effectivePlan.enabled ? 'disable' : 'enable',
                            })}
                    >
                        {effectivePlan.enabled ? t($locale, 'chatInspector.planActive') : t($locale, 'chatInspector.planMode')}
                    </button>
                </div>
                {#if effectivePlan.summary || effectivePlan.markdown || effectivePlan.steps.length}
                    <div class="plan-card" data-status={effectivePlan.status}>
                        <div class="section-row">
                            <strong>
                                {effectivePlan.status === 'awaiting_approval'
                                    ? t($locale, 'chatInspector.planPendingApproval')
                                    : effectivePlan.summary || t($locale, 'chatInspector.planMode')}
                            </strong>
                            <span class="status-pill">{effectivePlan.status}</span>
                        </div>
                        {#if effectivePlan.steps.length}
                            <ol class="plan-steps">
                                {#each effectivePlan.steps as step, sIdx (step.id ?? sIdx)}
                                    <li>
                                        <span>{step.title}</span>
                                        {#if step.status}<small class="inspector-muted">({step.status})</small>{/if}
                                    </li>
                                {/each}
                            </ol>
                        {:else if effectivePlan.markdown}
                            <pre class="plan-preview">{effectivePlan.markdown.slice(0, 600)}</pre>
                        {/if}
                        <div class="inspector-btn-row">
                            {#if effectivePlan.status === 'awaiting_approval' || effectivePlan.status === 'draft'}
                                <button
                                    type="button"
                                    class="primary compact-btn"
                                    disabled={planBusy}
                                    onclick={() => void handlePlanAction({ action: 'approve' })}
                                >
                                    {t($locale, 'chatInspector.planApprove')}
                                </button>
                                <button
                                    type="button"
                                    class="compact-btn danger"
                                    disabled={planBusy}
                                    onclick={() => void handlePlanAction({ action: 'reject' })}
                                >
                                    {t($locale, 'chatInspector.planReject')}
                                </button>
                            {/if}
                            <button
                                type="button"
                                class="compact-btn"
                                disabled={planBusy}
                                onclick={() => void handlePlanAction({ action: 'clear', enabled: false })}
                            >
                                {t($locale, 'chatInspector.planClear')}
                            </button>
                        </div>
                    </div>
                {:else if !effectivePlan.enabled}
                    <p class="inspector-muted">{t($locale, 'chatInspector.noPlan')}</p>
                {/if}
                {#if planError}<p class="inspector-error" role="alert">{planError}</p>{/if}
            </section>

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
                            aria-label={t($locale, 'chatInspector.goalTitlePlaceholder')}
                            disabled={goalSaving}
                        />
                        <textarea
                            rows="2"
                            bind:value={goalDescDraft}
                            placeholder={t($locale, 'chatInspector.goalDescPlaceholder')}
                            aria-label={t($locale, 'chatInspector.goalDescPlaceholder')}
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
                <h3>{t($locale, 'chatInspector.subagentTasks')}</h3>
                {#if taskError}<p class="inspector-error" role="alert">{taskError}</p>{/if}
                {#if rootTasks.length === 0}
                    <p class="inspector-muted">{t($locale, 'chatInspector.noLiveTasks')}</p>
                {:else}
                    <ul class="task-list">
                        {#each rootTasks as item (item.id)}
                            {@const children = childTasksOf(item.id)}
                            <li class="task-tree-node">
                                <div class="task-item" data-status={item.status}>
                                    <span class="task-dot" aria-hidden="true">
                                        {item.status === 'completed' ? '✓' : item.status === 'failed' ? '!' : item.status === 'cancelled' ? '×' : '•'}
                                    </span>
                                    <div class="task-copy">
                                        <div class="task-title-row">
                                            <span class="task-label">{item.title}</span>
                                            {#if item.agent}<span class="status-pill">@{item.agent}</span>{/if}
                                            <span class="status-pill">{item.status}</span>
                                        </div>
                                        {#if item.detail}
                                            <small class="inspector-muted">{item.detail.slice(0, 140)}</small>
                                        {/if}
                                    </div>
                                    {#if item.status === 'running' || item.status === 'queued' || item.status === 'waiting'}
                                        <button
                                            type="button"
                                            class="task-stop-btn danger"
                                            disabled={stoppingTaskId === item.id}
                                            onclick={() => void handleStopTask(item)}
                                        >
                                            {t($locale, 'chatInspector.stopTask')}
                                        </button>
                                    {/if}
                                </div>
                                {#if children.length}
                                    <ul class="task-children">
                                        {#each children as child (child.id)}
                                            <li class="task-item" data-status={child.status}>
                                                <span class="task-dot" aria-hidden="true">
                                                    {child.status === 'completed' ? '✓' : child.status === 'failed' ? '!' : '•'}
                                                </span>
                                                <div class="task-copy">
                                                    <span class="task-label">{child.title}</span>
                                                    {#if child.detail}
                                                        <small class="inspector-muted">{child.detail.slice(0, 100)}</small>
                                                    {/if}
                                                </div>
                                                {#if child.status === 'running' || child.status === 'queued'}
                                                    <button
                                                        type="button"
                                                        class="task-stop-btn danger"
                                                        disabled={stoppingTaskId === child.id}
                                                        onclick={() => void handleStopTask(child)}
                                                    >
                                                        {t($locale, 'chatInspector.stopTask')}
                                                    </button>
                                                {/if}
                                            </li>
                                        {/each}
                                    </ul>
                                {/if}
                            </li>
                        {/each}
                    </ul>
                {/if}
            </section>
        {:else if activeTab === 'artifacts'}
            <InspectorArtifactsPanel {conversationId} {messages} {refreshToken} />
        {:else if activeTab === 'recall'}
            <InspectorRecallPanel {conversationId} />
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

            <WorkspaceWorktreesPanel
                compact={true}
                {refreshToken}
                onchanged={() => conversationId && void loadDiffAndCheckpoints(conversationId, nextTabSignal())}
            />
        {:else if activeTab === 'browser'}
            <InspectorBrowserPdfPanel {conversationId} {messages} {refreshToken} />
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
                {#if dataSourcesNotice}
                    <p class="inspector-notice" role="status">{dataSourcesNotice}</p>
                {/if}
                {#if dataSourcesLoading}
                    <p class="inspector-muted">{t($locale, 'chatDataSources.loading')}</p>
                {:else if dataSources.length === 0}
                    <p class="inspector-muted">{t($locale, 'chatDataSources.empty')}</p>
                {:else}
                    <ul class="source-list">
                        {#each dataSources as src (src.id)}
                            {@const isBlocked = src.blocked || src.accessPolicy === 'blocked'}
                            <li class="source-item" class:blocked={isBlocked}>
                                <div class="source-row">
                                    <label class="source-toggle">
                                        <input
                                            type="checkbox"
                                            checked={!isBlocked && src.enabled}
                                            disabled={dataSourcesSaving || isBlocked}
                                            onchange={() => void toggleDataSource(src.id)}
                                        />
                                        <div class="source-meta">
                                            <div class="source-meta-head">
                                                <strong>{src.name}</strong>
                                                {#if isBlocked}
                                                    <span class="source-blocked-badge">{t($locale, 'chatDataSources.blocked')}</span>
                                                {/if}
                                            </div>
                                            <small class="inspector-muted">
                                                {src.kind}{src.location ? ` · ${src.location}` : ''}
                                            </small>
                                        </div>
                                    </label>
                                    {#if !isBlocked}
                                        <button
                                            type="button"
                                            class="index-btn"
                                            disabled={Boolean(indexingSourceIds[src.id])}
                                            onclick={() => void handleIndexDataSource(src)}
                                        >
                                            {indexingSourceIds[src.id]
                                                ? t($locale, 'chatInspector.indexing')
                                                : t($locale, 'chatInspector.indexNow')}
                                        </button>
                                    {/if}
                                </div>
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
                    {#if !savedKeyPoints && !keyPointsDraft && !savedWiki && !wikiDraft}
                        <p class="inspector-muted">{t($locale, 'chatInspector.knowledgeEmpty')}</p>
                    {/if}
                    <label class="knowledge-field">
                        <span class="knowledge-field-label">{t($locale, 'chatKnowledge.knowledgePoints')}</span>
                        <textarea
                            class="knowledge-textarea"
                            rows="6"
                            bind:value={keyPointsDraft}
                            placeholder={t($locale, 'chatInspector.knowledgeSummaryPlaceholder')}
                            disabled={knowledgeSaving}
                        ></textarea>
                    </label>
                    <label class="knowledge-field">
                        <span class="knowledge-field-label">{t($locale, 'chatKnowledge.wiki')}</span>
                        <textarea
                            class="knowledge-textarea"
                            rows="6"
                            bind:value={wikiDraft}
                            disabled={knowledgeSaving}
                        ></textarea>
                    </label>
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
                            onclick={() => conversationId && void loadKnowledge(conversationId, nextTabSignal())}
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
    .inspector-scrim {
        display: none;
    }
    .inspector-drawer {
        width: 360px;
        flex-shrink: 0;
        display: flex;
        flex-direction: column;
        min-height: 0;
        border-left: 1px solid var(--border);
        background: var(--panel);
        color: var(--content);
        position: relative;
    }
    .inspector-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 8px 12px;
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
    .inspector-icon-btn,
    .inspector-icon-btn:not(:disabled) {
        display: grid;
        place-items: center;
        width: 36px;
        height: 36px;
        min-width: 36px;
        min-height: 36px;
        padding: 0;
        border: 1px solid transparent;
        border-radius: 8px;
        background: transparent;
        box-shadow: none;
        color: var(--muted);
        cursor: pointer;
    }
    .inspector-icon-btn:not(:disabled):hover {
        color: var(--content);
        border-color: var(--border);
        background: var(--surface);
    }
    .inspector-refresh-icon {
        width: 16px;
        height: 16px;
        fill: none;
        stroke: currentColor;
        stroke-width: 1.6;
        stroke-linecap: round;
        stroke-linejoin: round;
    }
    .inspector-tabs-wrap {
        padding: 8px 10px;
        border-bottom: 1px solid var(--border);
        background: var(--canvas);
    }
    .inspector-tabs {
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: 2px;
        padding: 3px;
        background: var(--panel);
        border: 1px solid var(--border);
        border-radius: 10px;
    }
    .inspector-tab {
        min-width: 0;
        min-height: 26px;
        padding: 4px 3px;
        border: 1px solid transparent;
        border-radius: 7px;
        background: transparent;
        box-shadow: none;
        color: var(--muted);
        font-size: 11px;
        line-height: 1.2;
        text-align: center;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        cursor: pointer;
    }
    .inspector-tab:not(:disabled):hover {
        color: var(--content);
        border-color: transparent;
        background: var(--composer-pill-hover-bg);
    }
    .inspector-tab.active,
    .inspector-tab.active:not(:disabled):hover {
        background: var(--glass-pill-bg);
        border-color: var(--glass-pill-border);
        box-shadow: var(--glass-pill-shadow);
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
    .plan-card,
    .goal-active-card {
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        display: flex;
        flex-direction: column;
        gap: 6px;
        font-size: 12px;
    }
    .plan-card[data-status='awaiting_approval'] {
        border-color: var(--primary);
    }
    .plan-steps {
        margin: 0;
        padding-inline-start: 18px;
        display: flex;
        flex-direction: column;
        gap: 3px;
    }
    .plan-preview {
        margin: 0;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        font-size: 11px;
        color: var(--muted);
    }
    .status-pill {
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        font-size: 10px;
        color: var(--muted);
        white-space: nowrap;
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
    }
    .inspector-form textarea {
        resize: none;
    }
    .knowledge-textarea {
        resize: vertical;
    }
    .knowledge-field {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .knowledge-field-label {
        font-size: 11px;
        font-weight: 600;
        color: var(--muted);
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
        white-space: nowrap;
        flex-shrink: 0;
    }
    .task-list,
    .task-children,
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
    .task-tree-node {
        display: flex;
        flex-direction: column;
        gap: 4px;
    }
    .task-children {
        padding-inline-start: 14px;
        border-inline-start: 2px solid var(--border);
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
    .task-title-row {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 5px;
    }
    .task-stop-btn {
        margin-inline-start: auto;
        min-height: 24px;
        padding: 2px 7px;
        font-size: 11px;
        border-radius: 6px;
        flex: none;
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
        flex: 1;
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
    .source-row {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 8px;
    }
    .index-btn {
        flex: none;
        min-height: 26px;
        padding: 2px 8px;
        font-size: 11px;
        border-radius: 6px;
        white-space: nowrap;
    }
    .source-item.blocked {
        opacity: 0.65;
        border-style: dashed;
        background: var(--panel);
    }
    .source-toggle {
        display: flex;
        align-items: flex-start;
        gap: 8px;
        min-height: 0;
        padding: 0;
        border-radius: 0;
        cursor: pointer;
        flex: 1;
        min-width: 0;
    }
    .source-toggle input[type='checkbox'] {
        margin-top: 2px;
    }
    .source-item.blocked .source-toggle {
        cursor: not-allowed;
    }
    .source-meta {
        display: flex;
        flex-direction: column;
        gap: 2px;
        min-width: 0;
        flex: 1;
        overflow-wrap: anywhere;
        font-size: 12px;
    }
    .source-meta-head {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-wrap: wrap;
    }
    .source-blocked-badge {
        display: inline-flex;
        align-items: center;
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--canvas);
        color: var(--muted);
        font-size: 10px;
        font-weight: 600;
        line-height: 1.4;
    }
    @media (max-width: 1024px) {
        .inspector-drawer {
            width: 300px;
        }
    }
    @media (pointer: coarse) {
        .inspector-icon-btn,
        .inspector-icon-btn:not(:disabled) {
            width: 44px;
            height: 44px;
            min-width: 44px;
            min-height: 44px;
        }
        .inspector-tab {
            min-height: 34px;
        }
    }
    @media (max-width: 760px) {
        .inspector-scrim {
            display: block;
            position: fixed;
            inset: 0;
            z-index: 49;
            background: rgb(0 0 0 / 0.42);
        }
        .inspector-drawer {
            position: fixed;
            inset: 0 0 0 auto;
            padding-top: env(safe-area-inset-top, 0px);
            padding-bottom: env(safe-area-inset-bottom, 0px);
            width: min(92vw, 340px) !important;
            z-index: 50;
            box-shadow: var(--dialog-shadow);
        }
        .inspector-icon-btn,
        .inspector-icon-btn:not(:disabled) {
            width: 44px;
            height: 44px;
            min-width: 44px;
            min-height: 44px;
        }
        .inspector-tab {
            min-height: 34px;
        }
    }
</style>
