<script lang="ts">
    import { onDestroy, onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { sourceAccountStateKey } from '../lib/locales/sourceAccounts';
    import { api, ApiError } from '../lib/api';
    import {sourceLoginSnapshot, type SourceLoginSnapshot as Snapshot} from '../lib/sourceLoginState';
    let executable = $state(''), home = $state(''), name = $state('Codex');
    let consent = $state(false), busy = $state(false), failed = $state(false);
    let snapshot = $state<Snapshot | null>(null);
    let accounts = $state<{revision: number; account: {id: string; display_name: string; source: string}}[]>([]);
    let accountID = $state('');
    let handle: {request_id: string; account_id: string; expected_revision: number | null} | null = null;
    let alive = true;
    let generation = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const active = $derived(snapshot?.state === 'starting' || snapshot?.state === 'pending');
    async function poll() {
        if (!alive || !handle) return;
        const captured = handle, serial = generation;
        try {
            const result = await api<Snapshot>('/api/model-connections/source-login/status', {method: 'POST', body: JSON.stringify(captured)});
            if (!alive || serial !== generation || handle !== captured) return;
            snapshot = sourceLoginSnapshot(result, captured);
            failed = false;
            if (result.state === 'starting' || result.state === 'pending') timer = setTimeout(poll, 1000);
        } catch { if (alive && serial === generation && handle === captured) failed = true; }
    }
    async function start(refresh: boolean) {
        if (busy || active || !consent) return;
        busy = true; failed = false;
        const previous = snapshot?.account ?? accounts.find(item => item.account.id === accountID);
        handle = {request_id: crypto.randomUUID(), account_id: previous?.account.id ?? crypto.randomUUID(), expected_revision: previous?.revision ?? null};
        const captured = handle, serial = ++generation;
        // A lost start response is uncertain, not permission to start another login.
        snapshot = {request_id: captured.request_id, state: 'starting', authorization_url: null,
            inference_supported: false, account: previous ?? null};
        try {
            const result = await api<Snapshot>('/api/model-connections/source-login/start', {method: 'POST', body: JSON.stringify({...captured,
                display_name: name, executable, codex_home: home, refresh_tokens: refresh, acknowledge_source_changes: true})});
            if (!alive || serial !== generation || handle !== captured) {
                // Cancellation can race creation. Settle this exact operation again,
                // never cancel a newer operation or adopt its late response.
                void api('/api/model-connections/source-login/cancel', {method: 'POST', body: JSON.stringify(captured)}).catch(() => {});
                return;
            }
            snapshot = sourceLoginSnapshot(result, captured);
            timer = setTimeout(poll, 500);
        } catch (error) {
            if (alive && serial === generation) {
                failed = true;
                // The start route returns 4xx only before registering an operation.
                // Transport failures and 5xx remain uncertain and must be observed.
                if (error instanceof ApiError && error.status >= 400 && error.status < 500) {
                    snapshot = {...snapshot!, state: 'failed'};
                }
            }
        }
        finally { if (alive && serial === generation) busy = false; }
    }
    onMount(() => { void api<{accounts: typeof accounts}>('/api/model-connections').then(value => {
        if (alive) accounts = value.accounts.filter(item => item.account.source === 'codex');
    }).catch(() => { if (alive) failed = true; }); });
    async function cancel() {
        if (!handle) return;
        const captured = handle, serial = ++generation;
        clearTimeout(timer);
        busy = true;
        try {
            const result = await api<Snapshot>('/api/model-connections/source-login/cancel', {method: 'POST', body: JSON.stringify(captured)});
            if (!alive || serial !== generation || handle !== captured) return;
            snapshot = sourceLoginSnapshot(result, captured); failed = false;
        } catch { if (alive && serial === generation) failed = true; }
        finally { if (alive && serial === generation) busy = false; }
    }
    onDestroy(() => { alive = false; clearTimeout(timer); if (active) void cancel(); });
</script>

<details><summary>{t($locale, 'sourceAccounts.codexTitle')}</summary>
    <p>{t($locale, 'sourceAccounts.codexLimits')}</p>
    <label>{t($locale, 'sourceAccounts.account')}<select bind:value={accountID} disabled={active || busy} onchange={() => { snapshot = null; consent = false; }}>
        <option value="">{t($locale, 'sourceAccounts.newAccount')}</option>
        {#each accounts as item (item.account.id)}<option value={item.account.id}>{item.account.display_name} · {item.account.id}</option>{/each}
    </select></label>
    <label>{t($locale, 'sourceAccounts.label')}<input bind:value={name} disabled={active || busy} /></label>
    <label>{t($locale, 'sourceAccounts.executable')}<input bind:value={executable} disabled={active || busy} /></label>
    <label>{t($locale, 'sourceAccounts.home')}<input bind:value={home} disabled={active || busy} /></label>
    <label class="consent"><input type="checkbox" bind:checked={consent} disabled={active || busy} />{t($locale, 'sourceAccounts.codexConsent')}</label>
    <button disabled={busy || active || !consent || !name || !executable || !home} onclick={() => start(false)}>{t($locale, 'sourceAccounts.startCodex')}</button>
    {#if snapshot?.account || accountID}<button disabled={busy || active || !consent} onclick={() => start(true)}>{t($locale, 'sourceAccounts.refreshCodex')}</button>{/if}
    {#if active}<button disabled={busy} onclick={cancel}>{t($locale, 'sourceAccounts.cancelCodex')}</button>{/if}
    {#if failed && active}<button disabled={busy} onclick={() => void poll()}>{t($locale, 'sourceAccounts.checkCodex')}</button>{/if}
    {#if snapshot}<p role="status">{t($locale, sourceAccountStateKey(snapshot.state))} · {t($locale, 'sourceAccounts.disabled')}</p>{/if}
    {#if snapshot?.authorization_url}<a href={snapshot.authorization_url} target="_blank" rel="noopener noreferrer">{t($locale, 'sourceAccounts.continueCodex')}</a>{/if}
    {#if failed}<p role="alert">{t($locale, 'sourceAccounts.codexError')}</p>{/if}
    <p>{t($locale, 'sourceAccounts.codexCancelLimits')}</p>
</details>
<style>details { margin-block: 16px; } summary { cursor: pointer; padding-block: 12px; } label { display: grid; gap: 4px; margin-block: 12px; } .consent { display: flex; align-items: start; gap: 8px; } input[type=checkbox] { width: auto; } button { margin: 4px; } p { overflow-wrap: anywhere; }</style>
