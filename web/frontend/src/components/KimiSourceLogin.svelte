<script lang="ts">
    import { onDestroy, onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { sourceAccountStateKey } from '../lib/locales/sourceAccounts';
    import { api } from '../lib/api';
    import { kimiCanStartLogin } from '../lib/kimiLoginState';
    type Status = {account: {revision: number; account: {id: string}}; origin: string; server_id: string; state: string;
        flow_id: string | null; verification_url: string | null; user_code: string | null;
        poll_interval_seconds: number | null; inference_supported: boolean; cancellation_confirmed: boolean | null};
    let origin = $state(''), bearer = $state(''), name = $state('Kimi Code');
    let consent = $state(false), loginConsent = $state(false), cancelConsent = $state(false);
    let region = $state(''), busy = $state(false), failed = $state(false), observing = $state(false);
    let snapshot = $state<Status | null>(null);
    type Owner = {account_id: string; revision: number; display_name: string; origin: string; server_id: string};
    let owners = $state<Owner[]>([]), ownerID = $state('');
    // Preserve the account identity if a successful save loses its HTTP receipt.
    let newOwnerID = crypto.randomUUID();
    let alive = true, generation = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller: AbortController | undefined;
    function stopObserving() {
        generation++; clearTimeout(timer); controller?.abort(); controller = undefined;
        observing = false; busy = false;
    }
    function operationBody(value: Status) {
        return {account_id: value.account.account.id, expected_revision: value.account.revision,
            server_id: value.server_id, request_id: crypto.randomUUID(), flow_id: value.flow_id,
            region: region || null, acknowledge_provider_login_changes: false};
    }
    async function connect() {
        if (busy || !consent || !origin || !bearer) return;
        stopObserving(); const current = generation; busy = true; failed = false;
        controller = new AbortController();
        try {
            const result = await api<Status>('/api/model-connections/kimi-login/connect', {method: 'POST', signal: controller.signal,
                body: JSON.stringify({account_id: snapshot?.account.account.id ?? newOwnerID, expected_revision: snapshot?.account.revision ?? null, display_name: name,
                    origin, server_bearer: bearer, acknowledge_local_service_access: true})});
            if (alive && current === generation) {snapshot = result; bearer = ''; loginConsent = false; cancelConsent = false;}
        } catch {if (alive && current === generation) failed = true;}
        finally {if (alive && current === generation) busy = false;}
    }
    function selectOwner() {
        stopObserving(); consent = false; loginConsent = false; cancelConsent = false; bearer = '';
        failed = false;
        if (!ownerID && snapshot) newOwnerID = crypto.randomUUID();
        const selected = owners.find(value => value.account_id === ownerID);
        snapshot = selected ? {account: {revision: selected.revision, account: {id: selected.account_id}}, origin: selected.origin, server_id: selected.server_id,
            state: 'unknown', flow_id: null, verification_url: null, user_code: null, poll_interval_seconds: null,
            inference_supported: false, cancellation_confirmed: null} : null;
        if (selected) {origin = selected.origin; name = selected.display_name;}
    }
    async function refreshOwners() {
        try {
            const value = await api<Owner[]>('/api/model-connections/kimi-login/owners', {method:'POST', body:'{}'});
            if (alive) owners = value;
        } catch {if (alive) failed = true;}
    }
    onMount(() => {void refreshOwners();});
    async function request(action: 'start' | 'status' | 'cancel') {
        const previous = snapshot;
        if (!previous || busy || (action === 'start' && (!loginConsent || !kimiCanStartLogin(previous.state))) || (action === 'cancel' && !cancelConsent)) return;
        stopObserving(); const current = generation; busy = true; failed = false;
        controller = new AbortController();
        const body = operationBody(previous);
        if (action === 'start') body.flow_id = null;
        body.acknowledge_provider_login_changes = action === 'start' ? loginConsent : action === 'cancel' && cancelConsent;
        try {
            const result = await api<Status>(`/api/model-connections/kimi-login/${action}`, {method: 'POST', signal: controller.signal, body: JSON.stringify(body)});
            if (!alive || current !== generation) return;
            snapshot = result;
            if (action === 'cancel' || result.flow_id !== previous.flow_id) cancelConsent = false;
            if (result.state === 'pending' && action !== 'cancel') {
                observing = true;
                timer = setTimeout(() => {if (alive && current === generation) void request('status');}, (result.poll_interval_seconds ?? 5) * 1000);
            }
        } catch {
            if (alive && current === generation) {
                failed = true; cancelConsent = false;
                snapshot = {...previous, state: 'unconfirmed', verification_url: null, user_code: null, poll_interval_seconds: null};
            }
        }
        finally {if (alive && current === generation) busy = false;}
    }
    onDestroy(() => {alive = false; stopObserving();}); // Never DELETE a source flow on unmount.
</script>

<details><summary>{t($locale, 'sourceAccounts.kimiTitle')}</summary>
    <p>{t($locale, 'sourceAccounts.kimiLimits')}</p>
    <label>{t($locale, 'sourceAccounts.savedOwner')}<select bind:value={ownerID} disabled={busy} onchange={selectOwner}><option value="">{t($locale, 'sourceAccounts.newKimi')}</option>
        {#each owners as owner (owner.account_id)}<option value={owner.account_id}>{owner.display_name} · {owner.server_id}</option>{/each}
    </select></label>
    <button disabled={busy} onclick={refreshOwners}>{t($locale, 'sourceAccounts.refreshOwners')}</button>
    <label>{t($locale, 'sourceAccounts.label')}<input bind:value={name} disabled={busy} /></label>
    <label>{t($locale, 'sourceAccounts.origin')}<input bind:value={origin} placeholder="http://127.0.0.1:58627" disabled={busy} /></label>
    <label>{t($locale, 'sourceAccounts.bearer')}<input type="password" bind:value={bearer} autocomplete="off" disabled={busy} /></label>
    <label class="consent"><input type="checkbox" bind:checked={consent} disabled={busy} />{t($locale, 'sourceAccounts.inspectConsent')}</label>
    <button disabled={busy || !consent || !origin || !bearer || !name} onclick={connect}>{t($locale, 'sourceAccounts.connectKimi')}</button>
    {#if snapshot}
        <p role="status">{t($locale, 'sourceAccounts.ownerStatus', {origin: snapshot.origin, server: snapshot.server_id})} · {t($locale, sourceAccountStateKey(snapshot.state))} · {t($locale, 'sourceAccounts.disabled')}</p>
        <label>{t($locale, 'sourceAccounts.region')}<select bind:value={region} disabled={busy}><option value="">{t($locale, 'sourceAccounts.serviceRegion')}</option><option value="mainland-cn">{t($locale, 'sourceAccounts.china')}</option><option value="global">{t($locale, 'sourceAccounts.global')}</option></select></label>
        <label class="consent"><input type="checkbox" bind:checked={loginConsent} disabled={busy} />{t($locale, 'sourceAccounts.loginConsent')}</label>
        <button disabled={busy || !loginConsent || !kimiCanStartLogin(snapshot.state)} onclick={() => request('start')}>{t($locale, 'sourceAccounts.startKimi')}</button>
        <button disabled={busy} onclick={() => request('status')}>{t($locale, 'sourceAccounts.readState')}</button>
        {#if snapshot.verification_url}<a href={snapshot.verification_url} target="_blank" rel="noopener noreferrer">{t($locale, 'sourceAccounts.continueKimi')}</a><p>{t($locale, 'sourceAccounts.userCode', {code: snapshot.user_code ?? ''})}</p>{/if}
        {#if snapshot.state === 'pending'}
            <button disabled={!observing && !busy} onclick={stopObserving}>{t($locale, 'sourceAccounts.stopObservation')}</button>
            <label class="consent"><input type="checkbox" bind:checked={cancelConsent} disabled={busy} />{t($locale, 'sourceAccounts.cancelConsent')}</label>
            <button disabled={busy || !cancelConsent} onclick={() => request('cancel')}>{t($locale, 'sourceAccounts.cancelProvider')}</button>
        {/if}
        {#if snapshot.cancellation_confirmed !== null}<p>{t($locale, 'sourceAccounts.cancelResult', {answer: t($locale, snapshot.cancellation_confirmed ? 'sourceAccounts.yes' : 'sourceAccounts.no')})}</p>{/if}
    {/if}
    {#if failed}<p role="alert">{t($locale, 'sourceAccounts.kimiError')}</p>{/if}
</details>
<style>details {margin-block:16px;} summary {cursor:pointer;padding-block:12px;} label {display:grid;gap:4px;margin-block:12px;} .consent {display:flex;align-items:start;gap:8px;} input[type=checkbox] {width:auto;} button {margin:4px;} p {overflow-wrap:anywhere;}</style>
