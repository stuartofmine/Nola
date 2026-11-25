<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { hostDiagnostics, hostDiagnosticReport, type HostDiagnostics } from '../lib/hostDiagnostics';
    import { resultTime } from '../lib/results';
    let snapshot = $state<HostDiagnostics>();
    let loading = $state(false), failed = $state(false);
    let copyNotice = $state('');
    let checkedAt = $state('');
    let canOpenLogs = $state(false);
    let controller: AbortController | undefined;
    let disposed = false;
    type DesktopLogsBridge = { openLogsFolder?: () => Promise<unknown> };
    async function openLogs() {
        const bridge = (window as Window & { NolaDesktop?: DesktopLogsBridge }).NolaDesktop;
        if (!bridge?.openLogsFolder) return;
        try { await bridge.openLogsFolder(); } catch { /* native open failure is non-fatal */ }
    }
    async function refresh() {
        if (loading) return;
        loading = true; failed = false; copyNotice = '';
        const request = new AbortController(); controller = request;
        try {
            const next = hostDiagnostics(await api('/api/health', { signal: request.signal }));
            if (!disposed) { snapshot = next; checkedAt = new Date().toISOString(); }
        } catch { if (!disposed) failed = true; }
        finally { if (!disposed) loading = false; }
    }
    async function copy() {
        if (!snapshot || failed || loading) return;
        const captured = snapshot;
        try { await navigator.clipboard.writeText(hostDiagnosticReport(captured)); if (!disposed && snapshot === captured && !failed && !loading) copyNotice = 'shell.login.copied'; }
        catch { if (!disposed) copyNotice = 'diagnostics.copyFailed'; }
    }
    onMount(() => {
        canOpenLogs = typeof (window as Window & { NolaDesktop?: DesktopLogsBridge }).NolaDesktop?.openLogsFolder === 'function';
        void refresh();
        return () => { disposed = true; controller?.abort(); };
    });
</script>
<section class="card" aria-busy={loading}>
    <header>
        <div class="diag-heading">
            <h2>{t($locale, 'settings.category.diagnostics')}</h2>
            {#if !loading && !failed && snapshot}<span class="health-pill healthy"><span class="dot" aria-hidden="true"></span>{t($locale, 'diagnostics.healthy')}</span>
            {:else if !loading && failed}<span class="health-pill unhealthy"><span class="dot" aria-hidden="true"></span>{t($locale, 'diagnostics.failed')}</span>{/if}
        </div>
        <button type="button" disabled={loading} onclick={refresh}>{t($locale, 'models.reload')}</button>
    </header>
    <p class="muted">{t($locale, 'diagnostics.scope')}</p>
    {#if loading}<p role="status">{t($locale, 'diagnostics.checking')}</p>
    {:else if failed}<p role="alert" class="error">{t($locale, snapshot ? 'diagnostics.stale' : 'diagnostics.failed')}</p>
    {:else if snapshot}<p role="status">{t($locale, 'diagnostics.reachable')}</p>{/if}
    {#if snapshot}
        <p class="muted">{t($locale, 'activity.updated', { time: resultTime(checkedAt, $locale) })}</p>
        <dl>
            <dt>{t($locale, 'diagnostics.service')}</dt><dd>{snapshot.service}</dd>
            <dt>{t($locale, 'diagnostics.mode')}</dt><dd>{snapshot.mode}</dd>
            <dt>{t($locale, 'settings.host')}</dt><dd>{snapshot.hostId}</dd>
            <dt>{t($locale, 'diagnostics.protocol')}</dt><dd>{snapshot.permissionProtocolVersion}</dd>
        </dl>
        <div class="actions">
            <button type="button" disabled={loading || failed} onclick={copy}>{t($locale, 'diagnostics.copy')}</button>
            {#if canOpenLogs}<button type="button" onclick={openLogs}>{t($locale, 'diagnostics.openLogs')}</button>{/if}
        </div>
    {:else if canOpenLogs}
        <div class="actions"><button type="button" onclick={openLogs}>{t($locale, 'diagnostics.openLogs')}</button></div>
    {/if}
    {#if copyNotice}<p role="status">{t($locale, copyNotice)}</p>{/if}
</section>
<style>
    header{display:flex;gap:12px;justify-content:space-between;align-items:center;flex-wrap:wrap}
    .diag-heading{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
    h2{margin:0}
    .health-pill{display:inline-flex;align-items:center;gap:6px;padding:3px 10px;border-radius:999px;font-size:12px;font-weight:600}
    .health-pill.healthy{color:var(--success);background:color-mix(in srgb,var(--success) 12%,transparent)}
    .health-pill.unhealthy{color:var(--danger);background:color-mix(in srgb,var(--danger) 12%,transparent)}
    .dot{width:7px;height:7px;border-radius:50%;background:currentColor}
    .actions{display:flex;flex-wrap:wrap;gap:8px}
    dl{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,2fr);gap:12px}dt{color:var(--muted)}dd{margin:0;overflow-wrap:anywhere}@media(max-width:480px){dl{grid-template-columns:minmax(0,1fr)}dd{margin-bottom:8px}}
</style>
