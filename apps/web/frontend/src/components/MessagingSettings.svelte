<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount, tick } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { latestRequest } from '../lib/latestRequest';
    import { resultTime } from '../lib/results';
    import { workerSnapshot, workerDraft, workerPayload, channelsSnapshot, channelDraft, channelPayload, channelReceipt, callbackURL, messagingErrorKey, providerFields, channelKinds } from '../lib/messagingSettings';
    import type { WorkerSettings, WorkerDraft, Channel, ChannelDraft } from '../lib/messagingSettings';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    let confirmation: ConfirmationDialog, dialog: HTMLDialogElement;
    let settings = $state<WorkerSettings>(), draft = $state<WorkerDraft>();
    let token = $state('');
    let clearToken = $state(false);
    let workerError = $state(''), connectionError = $state(''), editorError = $state('');
    let workerNotice = $state(''), connectionNotice = $state('');
    let workerLoading = $state(false), channelsLoading = $state(false), saving = $state(false), confirming = $state(false);
    let channels = $state<Channel[]>([]), channelsLoaded = $state(false);
    let editing = $state<ChannelDraft>(), baseline = $state<Channel>();
    let savedDraft = $state('');
    let copyNotice = $state('');
    let workerUncertain = $state(false);
    let disposed = false, workerVersion = 0, channelsVersion = 0, editorVersion = 0;
    const workerRead = latestRequest<WorkerSettings>(), channelsRead = latestRequest<Channel[]>();
    const busy = $derived(saving || confirming);
    const workerDirty = $derived(!!draft && !!settings && (JSON.stringify(draft) !== JSON.stringify(workerDraft(settings)) || !!token || clearToken));
    const editorDirty = $derived(!!editing && JSON.stringify(editing) !== savedDraft);
    const callback = $derived(callbackURL(settings, editing));
    async function copyCallback() {
        const value = callback, version = editorVersion; if (!value) return;
        try { await navigator.clipboard.writeText(value); if (!disposed && version === editorVersion && value === callback) copyNotice = 'shell.login.copied'; }
        catch { if (!disposed && version === editorVersion && value === callback) copyNotice = 'messaging.copyFailed'; }
    }
    const commonFields = ['connection_id', 'display_name', 'account_id', 'credential_ref', 'webhook_secret_ref', 'verify_token_ref'] as const;
    const workerFields = ['turn_timeout_seconds', 'poll_seconds', 'project_root'] as const;
    function providerName(kind: string) {
        if (kind === 'wechat_official' || kind === 'we_com') return t($locale, `messaging.provider.${kind}`);
        const names: Record<string, string> = { telegram: 'Telegram', whatsapp: 'WhatsApp' };
        return Object.hasOwn(names, kind) ? names[kind] : kind;
    }
    async function loadWorker() {
        const version = ++workerVersion; workerLoading = true;
        try {
            const value = await workerRead.run(async signal => workerSnapshot(await api('/api/messaging-channel-worker', { signal })));
            if (value && !disposed) { settings = value; draft = workerDraft(value); token = ''; clearToken = false; workerError = ''; workerUncertain = false; }
        } catch (cause) { if (!disposed && version === workerVersion) workerError = messagingErrorKey(cause); }
        finally { if (!disposed && version === workerVersion) workerLoading = false; }
    }
    async function loadChannels() {
        if (!settings || workerLoading || workerError || workerUncertain) return;
        if (!settings.backend_url.trim() || !settings.has_access_token) {
            channelsRead.cancel(); ++channelsVersion; channelsLoading = false;
            channels = []; channelsLoaded = false; connectionError = '';
            connectionNotice = 'messaging.setupRequired';
            return;
        }
        if (connectionNotice === 'messaging.setupRequired') connectionNotice = '';
        const version = ++channelsVersion; channelsLoading = true;
        try {
            const value = await channelsRead.run(async signal => channelsSnapshot(await api('/api/messaging-channels', { signal })));
            if (value && !disposed) { channels = value; channelsLoaded = true; connectionError = ''; }
        } catch (cause) { if (!disposed && version === channelsVersion) connectionError = messagingErrorKey(cause) === 'messaging.authError' ? 'messaging.authError' : 'messaging.loadError'; }
        finally { if (!disposed && version === channelsVersion) channelsLoading = false; }
    }
    async function confirmDiscard() {
        confirming = true;
        const approved = await confirmation.ask({ title: t($locale, 'models.discard'), description: t($locale, 'messaging.reloadHint'), confirmLabel: t($locale, 'models.discard'), destructive: true });
        confirming = false; return approved && !disposed;
    }
    async function reloadWorker() {
        if (busy || editing) return; if (workerDirty && !await confirmDiscard()) return;
        workerUncertain = true; channelsRead.cancel(); ++channelsVersion; channelsLoading = false; channels = []; channelsLoaded = false;
        await loadWorker(); if (!disposed && !workerUncertain) await loadChannels();
    }
    onMount(() => { void loadWorker().then(() => { if (!disposed) return loadChannels(); }); return () => { disposed = true; ++editorVersion; workerRead.cancel(); channelsRead.cancel(); token = ''; }; });
    async function saveWorker(event: SubmitEvent) {
        event.preventDefault(); if (busy || editing || !settings || !draft) return;
        let payload;
        try { payload = workerPayload(draft, settings, token, clearToken); } catch (cause) { workerError = messagingErrorKey(cause); return; }
        saving = true; workerError = ''; workerNotice = ''; workerRead.cancel(); channelsRead.cancel(); ++workerVersion; ++channelsVersion; workerLoading = false; channelsLoading = false;
        token = ''; workerUncertain = true; channels = []; channelsLoaded = false;
        try {
            const value = workerSnapshot(await api('/api/messaging-channel-worker', { method: 'PUT', body: JSON.stringify(payload) }));
            if (!disposed) { settings = value; draft = workerDraft(value); clearToken = false; workerNotice = 'messaging.saved'; workerUncertain = false; await loadChannels(); }
        } catch (cause) { if (!disposed) workerError = messagingErrorKey(cause); }
        finally { if (!disposed) saving = false; }
    }
    async function open(channel?: Channel) {
        if (busy || editing || workerLoading || workerUncertain) return;
        baseline = channel ? structuredClone(channel) : undefined; editing = channelDraft(channel); savedDraft = JSON.stringify(editing); editorError = ''; connectionNotice = ''; copyNotice = ''; ++editorVersion;
        await tick(); if (!disposed) dialog.showModal();
    }
    async function close(event?: Event) {
        event?.preventDefault(); if (busy) return;
        const version = editorVersion;
        if (editorDirty && !await confirmDiscard()) return;
        if (disposed || editorVersion !== version) return;
        dialog.close(); editing = undefined; baseline = undefined; ++editorVersion;
    }
    async function saveChannel(event: SubmitEvent) {
        event.preventDefault(); if (busy || !editing) return;
        let payload;
        try { payload = channelPayload(editing, baseline); } catch (cause) { editorError = messagingErrorKey(cause); return; }
        const version = editorVersion;
        if (payload.enabled) {
            confirming = true;
            const approved = await confirmation.ask({ title: t($locale, 'messaging.apply'), description: t($locale, 'messaging.applyHint'), confirmLabel: t($locale, 'messaging.apply') });
            confirming = false; if (!approved || disposed || version !== editorVersion) return;
        }
        saving = true; editorError = ''; channelsRead.cancel(); ++channelsVersion; channelsLoading = false;
        try {
            const receipt = channelReceipt(await api('/api/messaging-channels', { method: 'PUT', body: JSON.stringify(payload) }), payload);
            if (disposed) return;
            channels = channels.some(channel => channel.connection_id === receipt.connection_id) ? channels.map(channel => channel.connection_id === receipt.connection_id ? receipt : channel) : [...channels, receipt];
            baseline = receipt; editing = channelDraft(receipt); savedDraft = JSON.stringify(editing); connectionNotice = 'messaging.saved';
            dialog.close(); editing = undefined; baseline = undefined; ++editorVersion; await loadChannels();
        } catch (cause) { if (!disposed) editorError = messagingErrorKey(cause); }
        finally { if (!disposed) saving = false; }
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return; if (workerDirty || editorDirty || saving) { event.preventDefault(); event.returnValue = ''; } }
</script>

<svelte:window onbeforeunload={beforeUnload} />
<ConfirmationDialog bind:this={confirmation} />
<section class="card" id="channels">
    <h2>{t($locale, 'messaging.title')}</h2><p class="muted">{t($locale, 'messaging.hint')}</p>
    <form onsubmit={saveWorker} aria-busy={workerLoading || saving}>
        <div class="row between"><h3>{t($locale, 'messaging.worker')}</h3><button type="button" disabled={busy || workerLoading || !!editing} onclick={reloadWorker}>{t($locale, 'models.reload')}</button></div>
        {#if workerLoading}<p role="status">{t($locale, 'remote.loading')}</p>{/if}
        {#if workerError}<p class="error" role="alert">{t($locale, workerError)}</p>{/if}
        {#if workerNotice}<p role="status">{t($locale, workerNotice)}</p>{/if}
        {#if draft && settings}<fieldset disabled={busy || workerLoading || !!editing}>
            <label class="toggle"><input type="checkbox" bind:checked={draft.enabled} />{t($locale, 'messaging.enabled')}</label>
            <label>{t($locale, 'messaging.backend_url')}<input bind:value={draft.backend_url} placeholder="https://api.example.com" autocomplete="off" /></label>
            <label>{t($locale, 'messaging.token')}<input type="password" bind:value={token} disabled={clearToken} autocomplete="new-password" spellcheck="false" /></label>
            <label class="toggle"><input type="checkbox" bind:checked={clearToken} onchange={() => { if (clearToken) token = ''; }} />{t($locale, 'messaging.clearToken')}</label>
            <p class="muted">{t($locale, settings.has_access_token ? 'messaging.tokenSaved' : 'messaging.tokenMissing')} · {t($locale, 'messaging.tokenHint')}</p>
            <label>{t($locale, 'messaging.execution_backend')}<select bind:value={draft.execution_backend}><option value="nola">Nola</option>{#if draft.execution_backend !== 'nola'}<option value={draft.execution_backend}>{draft.execution_backend}</option>{/if}</select></label>
            <div class="fields">{#each workerFields as field}<label>{t($locale, `messaging.${field}`)}<input bind:value={draft[field]} inputmode={field === 'project_root' ? 'text' : 'numeric'} /></label>{/each}</div>
            <p class="muted">{t($locale, 'messaging.workerHint')}</p><button class="primary" disabled={!workerDirty}>{t($locale, saving ? 'models.saving' : 'models.save')}</button>
        </fieldset>{/if}
    </form>
    <div class="row between"><h3>{t($locale, 'messaging.connections')}</h3><div class="actions"><button disabled={busy || channelsLoading || workerLoading || workerUncertain} onclick={loadChannels}>{t($locale, 'models.reload')}</button><button disabled={busy || workerLoading || workerUncertain || !channelsLoaded} onclick={() => open()}>{t($locale, 'messaging.add')}</button></div></div>
    {#if channelsLoading}<p role="status">{t($locale, 'remote.loading')}</p>{/if}
    {#if connectionError}<p role="alert" class="error">{t($locale, connectionError)}</p>{/if}
    {#if connectionNotice}<p role="status">{t($locale, connectionNotice)}</p>{/if}
    {#if channelsLoaded}{#each channels as channel (channel.connection_id)}
        <button class="card connection" disabled={busy || workerLoading} onclick={() => open(channel)}><span class="row between"><strong>{channel.display_name || channel.connection_id}</strong><span>{channel.enabled ? ['connected','degraded','awaiting_verification'].includes(channel.state) ? t($locale, `messaging.${channel.state}`) : channel.state : t($locale, 'remote.disabled')}</span></span><span>{providerName(channel.kind)} · {channel.connection_id}</span><small class="muted">{resultTime(channel.last_received_at, $locale)}</small></button>
    {:else}<p class="muted">{t($locale, 'messaging.empty')}</p>{/each}{/if}
</section>
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-labelledby="messaging-editor-title">
    {#if editing}<form onsubmit={saveChannel} aria-busy={saving}><h2 id="messaging-editor-title">{t($locale, 'messaging.title')}</h2><p class="muted">{t($locale, 'messaging.refsHint')}</p>
        {#if editorError}<p class="error" role="alert">{t($locale, editorError)}</p>{/if}
        <fieldset disabled={busy}><label>{t($locale, 'messaging.kind')}<select value={editing.kind} disabled={!!baseline} onchange={event => { editing = channelDraft(undefined, event.currentTarget.value); }}>
            {#each channelKinds as kind}<option value={kind}>{providerName(kind)}</option>{/each}{#if !channelKinds.includes(editing.kind as typeof channelKinds[number])}<option value={editing.kind}>{editing.kind}</option>{/if}
        </select></label>{#if !baseline}<p class="muted">{t($locale, 'messaging.providerReset')}</p>{/if}
        <div class="fields">{#each commonFields as field}<label>{t($locale, `messaging.${field}`)}<input bind:value={editing[field]} readonly={field === 'connection_id' && !!baseline} autocomplete="off" spellcheck="false" /></label>{/each}
            {#each Object.hasOwn(providerFields, editing.kind) ? providerFields[editing.kind] : [] as field}<label>{t($locale, `messaging.${field}`)}<input value={typeof editing.provider_config[field] === 'string' ? editing.provider_config[field] : ''} oninput={event => { if (editing) editing.provider_config[field] = event.currentTarget.value; }} autocomplete="off" /></label>{/each}
        </div><label class="toggle"><input type="checkbox" bind:checked={editing.enabled} />{t($locale, 'messaging.enableConnection')}</label></fieldset>
        {#if callback}<div><p class="muted">{t($locale, 'messaging.callback')}</p><code>{callback}</code><button type="button" onclick={copyCallback}>{t($locale, 'shell.markdown.copy')}</button>{#if copyNotice}<p role="status">{t($locale, copyNotice)}</p>{/if}</div>{/if}
        <p class="muted">{t($locale, 'messaging.applyHint')}</p><div class="actions"><button type="button" disabled={busy} onclick={() => close()}>{t($locale, 'shell.cancel')}</button><button class="primary" disabled={busy}>{t($locale, saving ? 'models.saving' : editing.enabled ? 'messaging.apply' : 'models.save')}</button></div>
    </form>{/if}
</dialog>
<style>
    form, fieldset { display: grid; gap: 14px; } fieldset { border: 0; min-width: 0; padding: 0; margin: 0; } p { margin: 0; }
    .toggle { display: flex; align-items: center; gap: 8px; } .toggle input { width: auto; }
    .connection { width: 100%; text-align: start; overflow-wrap: anywhere; } code { overflow-wrap: anywhere; } dialog { max-width: 740px; }
</style>
