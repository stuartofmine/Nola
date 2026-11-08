<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    $effect(() => { if (pageLocation.active()) untrack(() => { void load(); }); });
    import { onMount, untrack } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { api as unscopedApi } from '../lib/api';
    import { activityErrorKey } from '../lib/activityErrors';
    import { activeTask, dashboardFocus, dashboardSnapshot, scheduleLabel } from '../lib/dashboard';
    import type { DashboardSnapshot } from '../lib/dashboard';
    import { resultStatus, resultTime } from '../lib/results';
    let snapshot = $state<DashboardSnapshot>();
    let loading = $state(false);
    let error = $state('');
    let updatedAt = $state(0);
    let controller: AbortController | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const focus = $derived(dashboardFocus(snapshot?.tasks ?? [], updatedAt));
    const enabled = $derived(snapshot?.tasks.filter(task => task.enabled).length ?? 0);
    const active = $derived(snapshot?.tasks.filter(activeTask).length ?? 0);
    async function load() {
        if (loading || disposed || document.hidden || !pageLocation.active()) return;
        clearTimeout(timer); loading = true; controller = new AbortController();
        try {
            const value = dashboardSnapshot(await api<unknown>('/api/home', { signal: controller.signal }), $locale);
            if (disposed) return;
            snapshot = value; updatedAt = Date.now(); error = '';
        } catch (cause) { if (!disposed) error = activityErrorKey(cause); }
        finally {
            loading = false;
            if (!disposed && pageLocation.active() && !document.hidden && snapshot?.tasks.some(activeTask)) timer = setTimeout(load, 5000);
        }
    }
    function visibility() { clearTimeout(timer); if (!document.hidden) void load(); }
    onMount(() => {
        void load(); document.addEventListener('visibilitychange', visibility);
        return () => { disposed = true; clearTimeout(timer); controller?.abort(); document.removeEventListener('visibilitychange', visibility); };
    });
</script>

<svelte:window onfocus={() => void load()} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'activity.overview')} — Nola</title>{/if}</svelte:head>
<header class="page-header">
    <div><h1>{t($locale, 'activity.overview')}</h1><p class="muted">{t($locale, 'activity.dashboard_hint')}</p></div>
    <div class="actions"><a class="button" href="/chat">{t($locale, 'activity.ask')}</a><button onclick={load} disabled={loading}>{loading ? t($locale, 'activity.refreshing') : t($locale, 'activity.refresh')}</button></div>
</header>
<div class="content" aria-busy={loading}>
    {#if error}<div class="notice error" role="alert"><p>{t($locale, error)}</p>{#if snapshot}<p>{t($locale, 'activity.stale')}</p>{/if}<button onclick={load} disabled={loading}>{t($locale, 'activity.retry')}</button></div>{/if}
    {#if !snapshot && loading}<p role="status">{t($locale, 'activity.loading_overview')}</p>{/if}
    {#if snapshot}
        <section class="card focus" aria-labelledby="dashboard-focus-title">
            <div class="row between">
                <div class="focus-copy">
                    <p class="muted">{focus.kind === 'attention' ? t($locale, 'activity.attention') : focus.kind === 'active' ? t($locale, 'activity.active') : focus.kind === 'next' ? t($locale, 'activity.next') : t($locale, 'activity.automation_overview')}</p>
                    <h2 id="dashboard-focus-title">{focus.task?.name || focus.task?.sourceId || (snapshot.tasks.length ? t($locale, 'activity.nothing_pending') : t($locale, 'activity.create_first'))}</h2>
                    {#if focus.task}<p class:error={focus.kind === 'attention'}>{focus.kind === 'next' ? resultTime(focus.task.nextScheduledAt, $locale) : resultStatus(focus.task.latestResult?.status ?? '', $locale)}</p>
                    {:else}<p class="muted">{snapshot.tasks.length ? t($locale, 'activity.counts', { enabled, active }) : t($locale, 'activity.create_hint')}</p>{/if}
                </div>
                {#if focus.kind === 'attention' || focus.kind === 'active'}<a class="button" href="/results">{t($locale, 'activity.view_runs')}</a>
                {:else}<a class="button" href={snapshot.tasks.length ? '/scheduled-actions' : '/chat'}>{snapshot.tasks.length ? t($locale, 'activity.manage') : t($locale, 'activity.create_ai')}</a>{/if}
            </div>
        </section>
        <section aria-label={t($locale, 'activity.stats')} class="stats">
            <a class="card stat" href="/scheduled-actions"><span class="muted">{t($locale, 'activity.tasks')}</span><strong>{snapshot.stats.totalTasks}</strong><small>{t($locale, 'activity.enabled_count', { count: enabled })}</small></a>
            <a class="card stat" href="/results"><span class="muted">{t($locale, 'activity.success_runs')}</span><strong>{snapshot.stats.successCount}</strong><small>{t($locale, 'activity.saved_runs')}</small></a>
            <a class="card stat" href="/results"><span class="muted">{t($locale, 'activity.failed_runs')}</span><strong>{snapshot.stats.failedCount}</strong><small>{t($locale, 'activity.saved_runs')}</small></a>
        </section>
        <section class="runtime row between" aria-label={t($locale, 'activity.host')}>
            <div><strong>{error ? t($locale, 'activity.host_unknown') : snapshot.backend === 'cli-rust' ? t($locale, 'activity.host_cli') : t($locale, 'activity.host_web')}</strong><p class="muted">{snapshot.localFirst ? t($locale, 'activity.local_data') : t($locale, 'activity.host_data')} {t($locale, 'activity.updated', { time: resultTime(new Date(updatedAt).toISOString(), $locale) })}</p></div>
            <a class="button" href="/settings">{t($locale, 'activity.settings')}</a>
        </section>
        <section class="card" aria-labelledby="dashboard-tasks-title">
            <div class="row between"><h2 id="dashboard-tasks-title">{t($locale, 'activity.automations')}</h2><a class="button" href="/scheduled-actions">{t($locale, 'activity.view_all', { count: snapshot.tasks.length })}</a></div>
            {#each snapshot.tasks.slice(0, 6) as task (task.sourceId)}
                <a class="task" href={task.latestResult ? '/results' : '/scheduled-actions'}>
                    <div class="row between"><strong>{task.name || task.sourceId}</strong><span class:error={['failed', 'partial'].includes(task.latestResult?.status ?? '')}>{task.latestResult ? resultStatus(task.latestResult.status, $locale) : t($locale, 'activity.not_run')}</span></div>
                    <p class="muted">{task.enabled ? t($locale, 'activity.enabled') : t($locale, 'activity.disabled')} · {scheduleLabel(task, $locale)}</p>
                    <small>{t($locale, 'activity.last_run', { time: resultTime(task.latestResult?.createdAt, $locale) })}{#if task.enabled && task.nextScheduledAt} · {t($locale, 'activity.next_run', { time: resultTime(task.nextScheduledAt, $locale) })}{/if}</small>
                </a>
            {:else}<p class="muted">{t($locale, 'activity.empty_tasks')}</p><div class="row"><a class="button" href="/chat">{t($locale, 'activity.create_ai')}</a><a class="button" href="/scheduled-actions">{t($locale, 'activity.manual')}</a></div>{/each}
        </section>
    {/if}
</div>

<style>
    .stats { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
    .stat { gap: 4px; } .stat strong { font-size: 26px; font-weight: 600; font-variant-numeric: tabular-nums; }
    .focus-copy { min-width: 0; flex: 1 1 240px; overflow-wrap: anywhere; }
    .runtime { padding: 4px; } .runtime p { font-size: 12px; }
    .task { display: block; border-top: 1px solid var(--border); padding-top: 16px; overflow-wrap: anywhere; }
    .task:hover strong, .stat:hover strong { color: var(--accent); }
    @media (max-width: 560px) { .stats { grid-template-columns: 1fr; } .stat { grid-template-columns: 1fr auto; align-items: center; } .stat small { grid-column: 1 / -1; } }
</style>
