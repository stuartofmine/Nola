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
    let controller: AbortController | undefined;
    let disposed = false;
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
    onMount(() => { void refresh(); return () => { disposed = true; controller?.abort(); }; });
</script>
<section class="card" aria-busy={loading}>
    <header><h2>{t($locale, 'settings.category.diagnostics')}</h2><button disabled={loading} onclick={refresh}>{t($locale, 'models.reload')}</button></header>
    <p class="muted">{t($locale, 'diagnostics.scope')}</p>
    {#if loading}<p role="status">{t($locale, 'diagnostics.checking')}</p>
    {:else if failed}<p role="alert">{t($locale, snapshot ? 'diagnostics.stale' : 'diagnostics.failed')}</p>
    {:else if snapshot}<p role="status">{t($locale, 'diagnostics.reachable')}</p>{/if}
    {#if snapshot}
        <p class="muted">{t($locale, 'activity.updated', { time: resultTime(checkedAt, $locale) })}</p>
        <dl>
            <dt>{t($locale, 'diagnostics.service')}</dt><dd>{snapshot.service}</dd>
            <dt>{t($locale, 'diagnostics.mode')}</dt><dd>{snapshot.mode}</dd>
            <dt>{t($locale, 'settings.host')}</dt><dd>{snapshot.hostId}</dd>
            <dt>{t($locale, 'diagnostics.protocol')}</dt><dd>{snapshot.permissionProtocolVersion}</dd>
        </dl>
        <button disabled={loading || failed} onclick={copy}>{t($locale, 'diagnostics.copy')}</button>
    {/if}
    {#if copyNotice}<p role="status">{t($locale, copyNotice)}</p>{/if}
</section>
<style>
    header{display:flex;gap:12px;justify-content:space-between;align-items:center;flex-wrap:wrap}h2{margin:0}dl{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,2fr);gap:12px}dt{color:var(--muted)}dd{margin:0;overflow-wrap:anywhere}@media(max-width:480px){dl{grid-template-columns:minmax(0,1fr)}dd{margin-bottom:8px}}
</style>
