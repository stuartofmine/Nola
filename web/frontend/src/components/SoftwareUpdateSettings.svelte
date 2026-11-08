<script lang="ts">
    import { onMount, onDestroy } from 'svelte';
    import { api as baseApi, errorMessage } from '../lib/api';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(baseApi);
    import { locale, t } from '../lib/i18n';
    interface Snapshot {
        status: string; current_version: string; current_build: number; channel: string; target: string;
        release: { release_id: string; version: string; build: number; notes: string;
            artifact: {kind: 'installer'|'archive'|'store';url: string;filename?: string;size?: number;sha256?: string} } | null;
    }
    let snapshot = $state<Snapshot>();
    let busy = $state(false), failure = $state(''), downloaded = $state(false);
    let generation = 0, disposed = false;
    let controller: AbortController | undefined;
    onDestroy(() => { disposed = true; generation++; controller?.abort(); });
    async function check(network: boolean) {
        if (busy || disposed) return;
        const owner = ++generation;
        busy = true; failure = ''; downloaded = false; controller = new AbortController();
        try {
            const value = await api<Snapshot>(network ? '/api/software-update/check' : '/api/software-update',
                network ? {method:'POST',body:'{}',signal:controller.signal} : {signal:controller.signal});
            if (owner === generation && !disposed) snapshot = value;
        } catch (error) { if (owner === generation && !disposed) { failure = errorMessage(error); snapshot = undefined; } }
        finally { if (owner === generation && !disposed) busy = false; }
    }
    function storeURL(value: string): string | undefined {
        try { const url = new URL(value); return url.protocol === 'https:' && ['apps.apple.com','testflight.apple.com','play.google.com'].includes(url.hostname) ? url.href : undefined; }
        catch { return undefined; }
    }
    function requestDownload() {
        // Native navigation lets the browser stream to disk. The Host emits an
        // attachment only after verification; failures remain visible in the
        // opened page. Clicking is not reported as a completed download.
        downloaded = true;
    }
    onMount(() => { void check(false); });
</script>
<section class="card" aria-busy={busy}>
    <h2>{t($locale,'softwareUpdate.title')}</h2>
    <p class="muted">{t($locale,'softwareUpdate.hostHint')}</p>
    {#if snapshot}
        <p>Nola {snapshot.current_version} ({snapshot.current_build}) · {snapshot.channel}</p>
        <p role="status">{t($locale,`softwareUpdate.status.${snapshot.status}`)}</p>
        {#if snapshot.release}
            <strong>{snapshot.release.version} ({snapshot.release.build})</strong>
            {#if snapshot.release.notes}<p class="notes">{snapshot.release.notes}</p>{/if}
            {#if snapshot.release.artifact.kind === 'store'}
                {#if storeURL(snapshot.release.artifact.url)}<a class="button" href={storeURL(snapshot.release.artifact.url)} target="_blank" rel="noopener noreferrer">{t($locale,'softwareUpdate.store')}</a>{/if}
            {:else}<a class="button" href={`/api/software-update/package?release_id=${encodeURIComponent(snapshot.release.release_id)}`} target="_blank" rel="noopener noreferrer" aria-disabled={downloaded || busy} onclick={event => { if (downloaded || busy) event.preventDefault(); else requestDownload(); }}>{t($locale,'softwareUpdate.download')}</a>{/if}
        {/if}
    {/if}
    {#if busy}<p role="status">{t($locale,'softwareUpdate.working')}</p>{/if}
    {#if failure}<p role="alert">{t($locale,'softwareUpdate.failed')}</p><p class="detail">{failure}</p>{/if}
    {#if downloaded}<p role="status">{t($locale,'softwareUpdate.downloadRequested')}</p>{/if}
    <div class="actions"><button disabled={busy} onclick={() => check(true)}>{t($locale,'softwareUpdate.check')}</button></div>
    <p class="muted">{t($locale,'softwareUpdate.dataHint')}</p>
</section>
<style>.notes {white-space:pre-wrap;overflow-wrap:anywhere}.detail{overflow-wrap:anywhere}</style>
