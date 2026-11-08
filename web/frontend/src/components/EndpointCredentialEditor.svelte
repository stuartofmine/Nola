<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onDestroy } from 'svelte';
    import { api as unscopedApi, ApiError } from '../lib/api';
    import { credentialPath, credentialRequest, credentialSnapshot, isValidCredentialToken } from '../lib/endpointCredentials';
    import type { EndpointCredential } from '../lib/endpointCredentials';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { locale, t } from '../lib/i18n';
    let { onBusyChange, onChanged }: { onBusyChange: (busy: boolean) => void; onChanged: (endpointID: string) => void } = $props();
    let dialog: HTMLDialogElement;
    let confirmation: ConfirmationHandle;
    let id = $state('');
    let name = $state('');
    let snapshot = $state<EndpointCredential>();
    let secret = $state('');
    let busy = $state(false);
    let loading = $state(false);
    let error = $state('');
    let notice = $state('');
    let controller: AbortController | undefined;
    let generation = 0;

    export function open(endpointID: string, endpointName: string) {
        if (busy) return;
        generation += 1;
        controller?.abort();
        id = endpointID; name = endpointName; snapshot = undefined; secret = ''; error = ''; notice = '';
        dialog.showModal(); void load();
    }
    async function load() {
        if (busy) return;
        const owner = generation;
        const previousSnapshot = snapshot;
        if (secret && !await confirmation.ask({ title: t($locale, 'credential.discardTitle'), description: t($locale, 'credential.reloadDescription'), confirmLabel: t($locale, 'credential.reload'), destructive: true })) return;
        if (owner !== generation || snapshot !== previousSnapshot || busy) return;
        secret = '';
        controller?.abort();
        const request = new AbortController(); controller = request;
        const endpointID = id;
        loading = true; error = ''; snapshot = undefined;
        try {
            const result = await api<EndpointCredential>(credentialPath(endpointID), { signal: request.signal });
            if (controller !== request || request.signal.aborted) return;
            snapshot = credentialSnapshot(result, endpointID);
        } catch { if (controller === request && !request.signal.aborted) error = 'credential.loadFailed'; }
        finally { if (controller === request) loading = false; }
    }
    async function close(event?: Event) {
        event?.preventDefault();
        if (busy) return;
        const owner = generation;
        if (secret && !await confirmation.ask({ title: t($locale, 'credential.discardTitle'), description: t($locale, 'credential.discardDescription'), confirmLabel: t($locale, 'credential.discard'), destructive: true })) return;
        if (owner !== generation || busy) return;
        generation += 1;
        controller?.abort(); controller = undefined;
        secret = ''; snapshot = undefined; id = ''; loading = false; dialog.close();
    }
    async function write(action: 'replace' | 'clear') {
        if (busy || loading || !snapshot || (action === 'replace' && !secret.trim())) return;
        const owner = generation;
        const baseline = snapshot;
        const endpointID = id;
        if (action === 'replace' && !isValidCredentialToken(secret)) { error = 'credential.invalidToken'; return; }
        if (action === 'clear' && !await confirmation.ask({ title: t($locale, 'credential.clearTitle'), description: t($locale, 'credential.clearDescription'), confirmLabel: t($locale, 'credential.clear'), destructive: true })) return;
        if (owner !== generation || id !== endpointID || snapshot !== baseline || busy || loading) return;
        let body: string;
        try { body = JSON.stringify(credentialRequest(baseline, action, action === 'replace' ? secret : '')); }
        catch { error = 'credential.loadFailed'; return; }
        busy = true; onBusyChange(true); error = ''; notice = '';
        // Do not preserve passwords across retries, failures or successful saves.
        secret = '';
        try {
            const result = await api<EndpointCredential>(credentialPath(endpointID), { method: 'POST', body });
            snapshot = credentialSnapshot(result, endpointID);
            onChanged(endpointID);
            notice = action === 'replace' ? 'credential.saved' : 'credential.cleared';
        } catch (cause) {
            snapshot = undefined;
            // Do not render arbitrary server error bodies on a credential surface.
            error = cause instanceof ApiError && cause.status === 409
                ? 'credential.conflict'
                : 'credential.unconfirmed';
        } finally { busy = false; onBusyChange(false); }
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return; if (secret || busy) { event.preventDefault(); event.returnValue = ''; } }
    onDestroy(() => { controller?.abort(); secret = ''; });
</script>

<svelte:window onbeforeunload={beforeUnload} />
<ConfirmationDialog bind:this={confirmation} />
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-labelledby="endpoint-credential-title">
    <form onsubmit={event => { event.preventDefault(); void write('replace'); }} aria-busy={busy || loading}>
        <header><h2 id="endpoint-credential-title">{t($locale, 'credential.title', { name: name || t($locale, 'credential.endpoint') })}</h2><p class="muted">{t($locale, 'credential.storage')}</p></header>
        {#if loading}<p role="status">{t($locale, 'credential.loading')}</p>{/if}
        {#if error}<p class="notice error" role="alert">{t($locale, error)}</p><button type="button" onclick={load} disabled={busy || loading}>{t($locale, 'credential.reload')}</button>{/if}
        {#if notice}<p role="status">{t($locale, notice)}</p>{/if}
        {#if snapshot}
            <dl><dt>{t($locale, 'credential.address')}</dt><dd>{snapshot.baseURL}</dd><dt>{t($locale, 'credential.protocol')}</dt><dd>{snapshot.wireProtocol}</dd><dt>{t($locale, 'credential.endpointCredential')}</dt><dd>{t($locale, snapshot.hasCredential ? 'credential.configured' : 'credential.notConfigured')}</dd></dl>
            <label>{t($locale, 'credential.newKey')}<input type="password" bind:value={secret} autocomplete="new-password" autocapitalize="none" spellcheck="false" disabled={busy} placeholder={t($locale, snapshot.hasCredential ? 'credential.replacePlaceholder' : 'credential.keyPlaceholder')} /></label>
            <p class="muted">{t($locale, 'credential.bindingHint')}</p>
        {/if}
        <div class="actions">
            {#if snapshot}<button type="button" disabled={busy || loading} onclick={() => write('clear')}>{t($locale, 'credential.clear')}</button>{/if}
            <button type="button" disabled={busy} onclick={() => close()}>{t($locale, 'credential.close')}</button>
            <button class="primary" disabled={busy || loading || !snapshot || !secret.trim()}>{t($locale, busy ? 'credential.submitting' : 'credential.save')}</button>
        </div>
    </form>
</dialog>
<style>dd { overflow-wrap: anywhere; } header p { margin-top: 8px; }</style>
