<script lang="ts">
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount, untrack } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { resultTime } from '../lib/results';
    import { chatActivityNeedsPolling, chatActivityUrl, pendingGoalControl, goalControlOutcome, goalActionRequest, goalAvailableActions, parseChatGoal, parseChatTraces, parseChatPlan, type ChatPlanSummary } from '../lib/chatActivity';
    import type { ChatGoalAction, ChatGoalSummary, ChatTraceSummary } from '../lib/chatActivity';
    import { GoalControlIntentStore, dispatchGoalControlIntent } from '../lib/goalControlIntent';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { createConversationDraftStore } from '../lib/chatConversations';

    let { hostId, conversationId, generation, busy = false, refreshKey = 0, readOnly = false, section = 'all', oncontrol, draftStore = createConversationDraftStore() }: {
        hostId: string; conversationId: string; generation: number; section?: 'all' | 'progress' | 'goal' | 'execution'; readOnly?: boolean; busy?: boolean; refreshKey?: number | string;
        oncontrol?: () => void;
        draftStore?: ReturnType<typeof createConversationDraftStore>;
    } = $props();
    let confirmation: ConfirmationHandle;
    let goal = $state<ChatGoalSummary | null>(null);
    let traces = $state<ChatTraceSummary[]>([]);
    let plan = $state<ChatPlanSummary | null>(null);
    let planLoaded = $state(false);
    let planError = $state('');
    let goalLoaded = $state(false);
    let tracesLoaded = $state(false);
    let loading = $state(false);
    let writing = $state(false);
    let goalError = $state('');
    let controlError = $state('');
    let traceError = $state('');
    let objective = $state('');
    let mounted = false;
    let disposed = false;
    let request = 0;
    let controller: AbortController | null = null;
    let mountedOwner = '';
    let pendingRefresh = false;
    let pollTimer: ReturnType<typeof setTimeout> | null = null;
    let pendingOperation = $state<string | null>(null);
    let controlPending = false;
    const intents = () => new GoalControlIntentStore(localStorage, hostId);
    function restoreIntent() {
        try {
            const saved = intents().read(conversationId);
            pendingOperation = saved?.operationId ?? null;
            controlPending = pendingOperation !== null;
            // Recover the submitted objective after a reload as well as its ID.
            // Keep a newer draft intact if the user has already edited it.
            if (saved?.action === 'start' && !objective) {
                objective = saved.objective;
                draftStore.write(conversationId, objective);
            }
        }
        catch { controlError = 'chatActivity.writeFailed'; goalLoaded = false; }
    }
    function clearAppliedStartDraft(id: string, saved: ReturnType<typeof goalActionRequest>, outcome: string) {
        if (saved.action === 'start' && outcome === 'applied' && objective.trim() === saved.objective) {
            objective = ''; draftStore.delete(id);
        }
    }
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
    const current = (source: string, id: string, epoch: number, sequence: number) => !disposed && hostId === source && conversationId === id && generation === epoch && request === sequence;
    const locked = $derived(readOnly || writing || pendingOperation !== null || !goalLoaded);
    const actions = $derived(goalAvailableActions(goal));
    const recent = $derived(traces.slice(-12).reverse());
    const goalStatus = $derived(goal && ['running', 'paused', 'needs_user', 'completed', 'failed', 'cancelled'].includes(goal.status)
        ? `chatActivity.goal${goal.status.charAt(0).toUpperCase()}${goal.status.slice(1)}` : 'chatActivity.unknown');
    function traceStatus(status: string): string {
        return ['pending', 'running', 'waiting', 'succeeded', 'failed', 'cancelled', 'incomplete', 'interrupted'].includes(status)
            ? `chatActivity.trace${status.charAt(0).toUpperCase()}${status.slice(1)}` : 'chatActivity.unknown';
    }
    function scheduleRefresh() {
        if (disposed || refreshTimer !== null) return;
        refreshTimer = setTimeout(() => {
            refreshTimer = null;
            if (writing || loading) pendingRefresh = true;
            else void load();
        }, 250);
    }
    function schedulePoll() {
        if (pollTimer !== null) clearTimeout(pollTimer);
        pollTimer = null;
        if (disposed || !chatActivityNeedsPolling(goal, traces, controlPending)) return;
        pollTimer = setTimeout(() => {
            pollTimer = null;
            if (document.visibilityState === 'visible' && !writing && !loading) void load();
            else schedulePoll();
        }, 2000);
    }
    async function load(reset = false) {
        const source = hostId, id = conversationId, epoch = generation, sequence = ++request;
        controller?.abort(); const next = new AbortController(); controller = next;
        const signal = AbortSignal.any([next.signal, AbortSignal.timeout(15_000)]);
        loading = true; goalError = ''; traceError = ''; planError = '';
        if (reset) { goalLoaded = false; tracesLoaded = false; goal = null; traces = []; plan = null; planLoaded = false; }
        const [goalResult, traceResult, planResult] = await Promise.allSettled([
            api(chatActivityUrl(id, 'goal'), { signal }),
            api(chatActivityUrl(id, 'traces'), { signal }),
            api(chatActivityUrl(id, 'plan'), { signal }),
        ]);
        if (!current(source, id, epoch, sequence) || next.signal.aborted) return;
        try { if (goalResult.status !== 'fulfilled' || (goalResult.value as { source_host_id?: string })?.source_host_id !== source) throw new Error(); goal = parseChatGoal(goalResult.value, id); goalLoaded = true;
            if (pendingOperation) {
                const outcome = goalControlOutcome(goalResult.value, id, pendingOperation);
                if (outcome !== 'pending') {
                    const store = intents(), operation = pendingOperation;
                    const saved = await store.locked(id, async () => {
                        const intent = store.read(id);
                        if (intent?.operationId === operation) store.settle(id, intent, goalResult.value);
                        return intent?.operationId === operation ? intent : null;
                    });
                    if (!current(source, id, epoch, sequence)) return;
                    if (saved) clearAppliedStartDraft(id, saved, outcome);
                    pendingOperation = store.read(id)?.operationId ?? null;
                    controlError = outcome === 'rejected' ? 'chatActivity.controlRejected' : '';
                }
            } }
        catch { goalError = 'chatActivity.loadFailed'; goalLoaded = false; }
        try { if (traceResult.status !== 'fulfilled') throw new Error(); traces = parseChatTraces(traceResult.value, id); tracesLoaded = true; }
        catch { traceError = 'chatActivity.loadFailed'; }
        try { if (planResult.status !== 'fulfilled') throw new Error(); plan = parseChatPlan(planResult.value, id); planLoaded = true; }
        catch { planError = 'chatActivity.loadFailed'; }
        loading = false;
        if (goalLoaded) controlPending = pendingOperation !== null;
        schedulePoll();
        if (pendingRefresh) { pendingRefresh = false; scheduleRefresh(); }
    }
    $effect(() => {
        hostId; conversationId; generation; refreshKey;
        if (mounted) untrack(() => {
            const key = JSON.stringify([hostId, conversationId, generation]);
            if (key !== mountedOwner) { mountedOwner = key; objective = draftStore.read(conversationId); writing = false; pendingOperation = null; controlError = ''; controlPending = false; pendingRefresh = false; restoreIntent(); void load(true); }
            else if (writing) pendingRefresh = true;
            else scheduleRefresh();
        });
    });
    onMount(() => {
        mounted = true;
        mountedOwner = JSON.stringify([hostId, conversationId, generation]);
        objective = draftStore.read(conversationId);
        restoreIntent(); void load();
        const storageChanged = (event: StorageEvent) => {
            if (event.storageArea !== localStorage || (event.key !== null && !event.key.startsWith('nola.goal-intent:'))) return;
            // Another tab may have submitted or settled the shared durable intent.
            // Never keep a stale enabled action or retry button in this panel.
            restoreIntent(); scheduleRefresh();
        };
        window.addEventListener('storage', storageChanged);
        return () => {
            disposed = true; controller?.abort();
            window.removeEventListener('storage', storageChanged);
            if (refreshTimer !== null) clearTimeout(refreshTimer);
            if (pollTimer !== null) clearTimeout(pollTimer);
        };
    });

    async function retryIntent() {
        if (readOnly) return;
        const source = hostId, id = conversationId, epoch = generation;
        if (writing) return;
        writing = true; controlError = '';
        try {
            const saved = intents().read(id);
            if (!saved) return;
            const response = await dispatchGoalControlIntent(id, saved, api, intents());
            if (!disposed && hostId === source && conversationId === id && generation === epoch && !pendingGoalControl(response, id)) {
                const outcome = goalControlOutcome(response, id, saved.operationId);
                clearAppliedStartDraft(id, saved, outcome);
                if (outcome === 'rejected') controlError = 'chatActivity.controlRejected';
            }
        } catch { if (!disposed && hostId === source && conversationId === id && generation === epoch) controlError = 'chatActivity.writeFailed'; }
        finally { if (!disposed && hostId === source && conversationId === id && generation === epoch) { writing = false; restoreIntent(); await load(); } }
    }

    async function control(action: ChatGoalAction) {
        if (locked || (busy && action === 'start') || !actions.includes(action)) return;
        const source = hostId, id = conversationId, epoch = generation;
        let body: ReturnType<typeof goalActionRequest>;
        try { body = goalActionRequest(action, objective, goal); }
        catch { goalError = 'chatActivity.invalidObjective'; return; }
        writing = true; controlError = ''; goalError = '';
        try {
            if (action === 'cancel') {
                const accepted = await confirmation.ask({ title: t($locale, 'chatActivity.cancelTitle'), description: t($locale, 'chatActivity.cancelBody'),
                    confirmLabel: t($locale, 'chatActivity.cancel'), destructive: true });
                if (!accepted || disposed || hostId !== source || conversationId !== id || generation !== epoch) return;
            }
            // A pending pre-mutation read cannot overwrite the authoritative
            // command receipt. The Host owns the status transition.
            ++request; controller?.abort(); controller = null; loading = false;
            pendingOperation = body.operationId; controlPending = true;
            const response = await dispatchGoalControlIntent(id, body, api, intents());
            if (disposed || hostId !== source || conversationId !== id || generation !== epoch) return;
            const queuedOperation = pendingGoalControl(response, id);
            pendingOperation = intents().read(id)?.operationId ?? queuedOperation;
            if (!queuedOperation) { goal = parseChatGoal(response, id); goalLoaded = true; }
            controlPending = true; schedulePoll(); oncontrol?.();
            if (!queuedOperation) {
                const outcome = goalControlOutcome(response, id, body.operationId);
                clearAppliedStartDraft(id, body, outcome);
                if (outcome === 'rejected') controlError = 'chatActivity.controlRejected';
            }
        } catch {
            if (!disposed && hostId === source && conversationId === id && generation === epoch) {
                controlError = 'chatActivity.writeFailed'; restoreIntent();
                // A network failure is not evidence the command did not commit.
                // Read canonical state; keep the error until user sees the result.
                try {
                    const response = await api<{ source_host_id?: string }>(chatActivityUrl(id, 'goal'), { signal: AbortSignal.timeout(15_000) });
                    if (response.source_host_id !== source) throw new Error('Goal Host changed');
                    const recovered = parseChatGoal(response, id);
                    if (!disposed && hostId === source && conversationId === id && generation === epoch) { goal = recovered; goalLoaded = true; }
                } catch { if (!disposed && hostId === source && conversationId === id && generation === epoch) { goal = null; goalLoaded = false; } }
            }
        } finally { if (!disposed && hostId === source && conversationId === id && generation === epoch) {
            writing = false;
            if (pendingOperation) { controlPending = true; schedulePoll(); }
            if (pendingRefresh) { pendingRefresh = false; scheduleRefresh(); }
        } }
    }
</script>

<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (objective.trim() || writing) { event.preventDefault(); event.returnValue = ''; } }} />
<ConfirmationDialog bind:this={confirmation} />
<section class="activity-goal" aria-label={t($locale, 'chatActivity.title')}>
<div class="heading"><button type="button" onclick={() => load()} disabled={loading || writing}>{t($locale, 'chatActivity.refresh')}</button></div>
    {#if loading && !goalLoaded}<p class="muted" role="status">{t($locale, 'chatActivity.loading')}</p>{/if}
    {#if pendingOperation}<p class="muted" role="status">{t($locale, 'chatActivity.controlPending')}</p><button type="button" disabled={readOnly || writing || loading} onclick={retryIntent}>{t($locale, 'chatActivity.retryControl')}</button>{/if}
    {#if section === 'all' || section === 'progress'}<div class="section"><h3>{t($locale, 'chatInspector.requirements')}</h3>
        {#if planError}<p class="error" role="alert">{t($locale, planError)}</p>{/if}
        {#if planLoaded && !plan?.todos.length}<p class="muted">{t($locale, 'chatInspector.noPlan')}</p>{/if}
        {#if plan?.todos.length}<ul class="checklist">
            {#each plan.todos as todo (todo.id)}<li><span aria-hidden="true">{todo.status === 'completed' ? '✓' : '○'}</span><span>{todo.content}<small>{t($locale, traceStatus(todo.status === 'completed' ? 'succeeded' : todo.status === 'in_progress' ? 'running' : todo.status))}</small></span></li>{/each}
        </ul>{/if}
    </div>
    {/if}
    {#if section === 'all' || section === 'goal'}<div class="section"><h3>{t($locale, 'chatActivity.goal')}</h3>
        {#if goalError}<p class="error" role="alert">{t($locale, goalError)}</p>{/if}
        {#if controlError}<p class="error" role="alert">{t($locale, controlError)}</p>{/if}
        {#if goal}
            <p class="objective">{goal.objective}</p>
            {#if goal.executionBlocker}<p class="error" role="alert">{t($locale, 'chatActivity.configurationUnavailable')}</p>{/if}
            <p class="muted">{t($locale, goalStatus)} · {t($locale, 'chatActivity.progress', { done: goal.completedRequirements, total: goal.totalRequirements })}</p>
            {#if goal.requirements.length}<ul class="checklist" aria-label={t($locale, 'chatInspector.requirements')}>
                {#each goal.requirements as requirement (requirement.id)}<li>
                    <span aria-hidden="true">{requirement.status === 'completed' ? '✓' : '○'}</span>
                    <span>{requirement.description}<small>{t($locale, traceStatus(requirement.status === 'completed' ? 'succeeded' : requirement.status))}</small></span>
                </li>{/each}
            </ul>{/if}
            <div class="actions">
                {#if actions.includes('pause')}<button type="button" disabled={locked} onclick={() => control('pause')}>{t($locale, 'chatActivity.pause')}</button>{/if}
                {#if actions.includes('resume')}<button type="button" disabled={locked} onclick={() => control('resume')}>{t($locale, 'chatActivity.resume')}</button>{/if}
                {#if actions.includes('cancel')}<button type="button" class="danger" disabled={locked} onclick={() => control('cancel')}>{t($locale, 'chatActivity.cancel')}</button>{/if}
            </div>
        {:else if goalLoaded}<p class="muted">{t($locale, 'chatActivity.noneGoal')}</p>{/if}
        {#if goalLoaded && actions.includes('start')}
            <form onsubmit={event => { event.preventDefault(); void control('start'); }}>
                <label>{t($locale, 'chatActivity.objective')}<textarea bind:value={objective} oninput={event => draftStore.write(conversationId, event.currentTarget.value)} rows="2" maxlength="20000" disabled={locked}></textarea></label>
                <button type="submit" disabled={busy || locked || !objective.trim()}>{t($locale, 'chatActivity.start')}</button>
            </form>
        {/if}
    </div>
    {/if}
    {#if section === 'all' || section === 'execution'}<div class="section"><h3>{t($locale, 'chatActivity.traces')}</h3>
        {#if traceError}<p class="error" role="alert">{t($locale, traceError)}</p>{/if}
        {#if tracesLoaded && !traces.length}<p class="muted">{t($locale, 'chatActivity.noneTraces')}</p>{/if}
        {#if recent.length}<ol>{#each recent as trace, index (trace.traceId)}<li>
            <span>{t($locale, 'chatActivity.run', { number: traces.length - index })} · {t($locale, traceStatus(trace.status))}</span>
            <small>{resultTime(trace.startedAt, $locale)} · {t($locale, 'chatActivity.sequence', { number: trace.lastSequence })}</small>
        </li>{/each}</ol>{/if}
        {#if traces.length > 12}<p class="muted">{t($locale, 'chatActivity.moreRuns', { count: 12 })}</p>{/if}
    </div>
{/if}
</section>

<style>
    .activity-goal { display: grid; gap: 12px; min-width: 0; }
    .heading { display: flex; justify-content: flex-end; gap: 8px; }
    h3,p { margin: 0; } h3 { font-size: .875rem; }
    .heading button,.actions button,form button { font-size: 11px; }
    .section { display: grid; gap: 8px; padding-top: 10px; border-top: 1px solid var(--border); }
    .section:first-of-type { border-top: 0; padding-top: 0; }
    .objective { font-size: 12px; font-weight: 600; overflow-wrap: anywhere; }
    .muted,small { color: var(--muted); font-size: 11px; line-height: 1.45; }
    .error { color: var(--danger); font-size: 11px; }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; }
    form,label { display: grid; gap: 6px; } label { font-size: 11px; font-weight: 600; }
    textarea { width: 100%; min-width: 0; resize: vertical; }
    ol { display: grid; gap: 8px; margin: 0; padding: 0; list-style: none; }
    li { display: grid; gap: 3px; min-width: 0; font-size: 11px; }
    .checklist { list-style: none; margin: 0; padding: 0; display: grid; gap: 10px; }
    .checklist li { display: flex; gap: 8px; font-size: .875rem; overflow-wrap: anywhere; }
    .checklist small { display: block; }
</style>
