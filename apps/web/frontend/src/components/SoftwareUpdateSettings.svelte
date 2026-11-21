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
    function parseDispositionFilename(header: string | null): string | undefined {
        if (!header) return undefined;
        const utf8 = /filename\*=UTF-8''([^;\s]+)/i.exec(header);
        if (utf8) { try { return decodeURIComponent(utf8[1]); } catch { /* fall through */ } }
        const basic = /filename="([^"]+)"|filename=([^;\s]+)/i.exec(header);
        return (basic?.[1] ?? basic?.[2])?.trim() || undefined;
    }
    async function requestDownload(release: NonNullable<Snapshot['release']>) {
        if (downloaded || busy || disposed) return;
        const owner = ++generation;
        downloaded = true; failure = ''; controller = new AbortController();
        const url = `/api/software-update/package?release_id=${encodeURIComponent(release.release_id)}`;
        try {
            const response = await fetch(url, { credentials: 'same-origin', signal: controller.signal });
            if (!response.ok) {
                let detail = `HTTP ${response.status}`;
                try {
                    const text = await response.text();
                    try {
                        const parsed = JSON.parse(text) as { error?: string; message?: string };
                        detail = parsed.error || parsed.message || text || detail;
                    } catch { if (text.trim()) detail = text.trim(); }
                } catch { /* keep HTTP status */ }
                throw new Error(detail);
            }
            const blob = await response.blob();
            if (owner !== generation || disposed) return;
            const filename = parseDispositionFilename(response.headers.get('content-disposition'))
                || release.artifact.filename
                || `nola-${release.version}.bin`;
            const objectUrl = URL.createObjectURL(blob);
            const anchor = document.createElement('a');
            anchor.href = objectUrl;
            anchor.download = filename;
            document.body.append(anchor);
            anchor.click();
            anchor.remove();
            setTimeout(() => URL.revokeObjectURL(objectUrl), 1000);
        } catch (error) {
            if (owner === generation && !disposed) {
                downloaded = false;
                failure = errorMessage(error);
            }
        }
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
            {:else}<a class="button" href={`/api/software-update/package?release_id=${encodeURIComponent(snapshot.release.release_id)}`} rel="noopener noreferrer" aria-disabled={downloaded || busy} onclick={event => { event.preventDefault(); if (!downloaded && !busy && snapshot?.release) void requestDownload(snapshot.release); }}>{t($locale,'softwareUpdate.download')}</a>{/if}
        {/if}
    {/if}
    {#if busy}<p role="status">{t($locale,'softwareUpdate.working')}</p>{/if}
    {#if failure}<p role="alert" class="error">{t($locale,'softwareUpdate.failed')}</p><p class="detail">{failure}</p>{/if}
    {#if downloaded}<p role="status">{t($locale,'softwareUpdate.downloadRequested')}</p>{/if}
    <div class="actions"><button disabled={busy} onclick={() => check(true)}>{t($locale,'softwareUpdate.check')}</button></div>
    <p class="muted">{t($locale,'softwareUpdate.dataHint')}</p>
</section>
<style>.notes {white-space:pre-wrap;overflow-wrap:anywhere}.detail{overflow-wrap:anywhere}</style>
