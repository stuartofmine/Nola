<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { loadModelCatalog } from '../lib/modelCatalog';
    import type { RemoteProfile } from '../lib/modelProfiles';
    type Plan = {plan_id: string; source: string; path: string; owned_fields: string[]; expires_at_ms: number};
    type Receipt = {receipt_id: string; source: string; path: string; owned_fields: string[]};
    let source = $state('open_code'), path = $state(''), model = $state('');
    let profiles = $state<RemoteProfile[]>([]), receipts = $state<Receipt[]>([]);
    let plan = $state<Plan | null>(null), planIdentity = $state('');
    let busy = $state(false), failed = $state(false), alive = true;
    const identity = $derived(JSON.stringify([source, path, model]));
    $effect(() => { if (plan && planIdentity !== identity) plan = null; });
    async function perform(action: () => Promise<void>) {
        if (busy) return; busy = true; failed = false;
        try { await action(); } catch { if (alive) failed = true; } finally { if (alive) busy = false; }
    }
    function preview() {
        const current = identity;
        void perform(async () => {
            const value = await api<Plan>('/api/model-gateway/clients/plan', {method: 'POST', body: JSON.stringify({source, path: path.trim(), model_id: model})});
            if (alive && current === identity) { planIdentity = current; plan = value; }
        });
    }
    function commit() {
        const current = plan; if (!current) return;
        void perform(async () => {
            const receipt = await api<Receipt>('/api/model-gateway/clients/commit', {method: 'POST', body: JSON.stringify({plan_id: current.plan_id})});
            if (alive) { receipts = [...receipts, receipt]; plan = null; }
        });
    }
    function disconnect(receipt: Receipt) {
        void perform(async () => {
            await api('/api/model-gateway/clients/disconnect', {method: 'POST', body: JSON.stringify({receipt_id: receipt.receipt_id})});
            if (alive) receipts = receipts.filter(item => item.receipt_id !== receipt.receipt_id);
        });
    }
    onMount(() => {
        void api<Receipt[]>('/api/model-gateway/clients').then(value => { if (alive) receipts = value; }).catch(() => { if (alive) failed = true; });
        void loadModelCatalog(api).then(value => { if (alive) profiles = value.profiles.filter(item => item.enabled && item.endpoint.enabled); }).catch(() => { if (alive) failed = true; });
        return () => { alive = false; };
    });
</script>

<details><summary>{t($locale, 'connections.connectClient')}</summary>
    <p>{t($locale, 'connections.clientHint')}</p>
    <form onsubmit={event => { event.preventDefault(); preview(); }}>
        <label>{t($locale, 'connections.source')}<select bind:value={source} disabled={busy}><option value="open_code">OpenCode</option><option value="codex">Codex</option><option value="kimi_code">Kimi Code CLI</option></select></label>
        <label>{t($locale, 'connections.path')}<input bind:value={path} disabled={busy} required /></label>
        <label>{t($locale, 'rooms.model')}<select bind:value={model} disabled={busy} required><option value="">{t($locale, 'rooms.model')}</option>{#each profiles as profile}<option value={`nola/${profile.id}/${profile.revision}`}>{profile.displayName}</option>{/each}</select></label>
        <button disabled={busy || !path.trim() || !model}>{t($locale, 'connections.preview')}</button>
    </form>
    {#if busy}<p role="status">{t($locale, 'rooms.saving')}</p>{/if}
    {#if failed}<p role="alert">{t($locale, 'connections.failed')}</p>{/if}
    {#if plan}<p>{plan.source} · {plan.path}</p><ul>{#each plan.owned_fields as field}<li><code>{field}</code></li>{/each}</ul>
        <button disabled={busy || Date.now() >= plan.expires_at_ms} onclick={commit}>{t($locale, 'connections.confirmClient')}</button>{/if}
    {#each receipts as receipt (receipt.receipt_id)}<p>{receipt.source} · {receipt.path}</p><button disabled={busy} onclick={() => disconnect(receipt)}>{t($locale, 'connections.disconnectClient')}</button>{/each}
</details>

<style>form { display: grid; gap: 12px; padding: 12px 0; } summary { cursor: pointer; min-height: 44px; padding: 10px 0; } p, code { overflow-wrap: anywhere; }</style>
