<script lang="ts">
    import { activityRequestFocus } from '../lib/activityRequestFocus';
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { BrowserRecoveryIntentStore, createRecoveryDecision, parseRecoveryInspection, recoverDecision } from '../lib/unknownEffectRecovery';
    import type { RecoveryOwner, RecoveryInspection, RecoveryChoice, OwnedTaskRecoveryContext } from '../lib/unknownEffectRecovery';
    import { parseRunObservation } from '../lib/runObservation';

    // Parent keys this component by conversation generation and exact Run.
    let { owner, available = false, onResume, onObserved, taskContext }: { owner: RecoveryOwner; taskContext?: OwnedTaskRecoveryContext; available?: boolean; onResume: () => void; onObserved: () => void } = $props();
    const store = new BrowserRecoveryIntentStore();
    let inspected = $state<RecoveryInspection | null>(null);
    let checked = $state(false);
    let pending = $state<string | null>(null);
    let busy = $state(true);
    let failed = $state(false);
    let result = $state('');
    let confirming = $state<RecoveryChoice | null>(null);
    let selected = $state(0);
    let reason = $state('');
    let disposed = false;
    const request = (body: string) => taskContext
        ? api('/api/chat/agent/v2/task/recovery', { method: 'POST', body: JSON.stringify({task: taskContext, recovery: JSON.parse(body)}) })
        : api('/api/chat/agent/v2/recovery', { method: 'POST', body });
    onMount(() => {
        void store.read(owner).then(value => { if (!disposed) pending = value; })
            .catch(() => { if (!disposed) failed = true; })
            .finally(() => { if (!disposed) busy = false; });
        return () => { disposed = true; };
    });
    async function inspect() {
        if (busy || disposed || !available) return;
        busy = true; failed = false; confirming = null; result = '';
        const captured = owner;
        try {
            const saved = await store.read(captured);
            if (disposed) return;
            pending = saved;
            if (saved) return;
            const next = parseRecoveryInspection(await request(JSON.stringify({ operation: 'inspect', owner: captured })), captured);
            if (disposed) return;
            inspected = next; checked = true; selected = 0;
        } catch { if (!disposed) failed = true; }
        finally { if (!disposed) busy = false; }
    }
    async function reconcile(wire: string) {
        const outcome = await recoverDecision(owner, wire, store, request);
        if (disposed) return;
        result = outcome;
        if (outcome !== 'pending') pending = null;
        confirming = null; inspected = null; checked = false;
        if (outcome === 'retry_authorized') onResume();
        else if (outcome === 'unknown_preserved_blocked') {
            if (taskContext) {
                const receipt = await api<Record<string,unknown>>('/api/chat/agent/v2/task/cancel/receipt', {method:'POST',body:JSON.stringify({
                    hostId:taskContext.host_id,conversationId:taskContext.conversation_id,parentRunId:taskContext.parent_run_id,taskId:taskContext.task_id})});
                if (receipt.hostId!==taskContext.host_id || receipt.conversationId!==taskContext.conversation_id
                    || receipt.parentRunId!==taskContext.parent_run_id || receipt.taskId!==taskContext.task_id
                    || receipt.terminal!==true) throw new Error('Task blocked result unavailable');
                if (!disposed) onObserved();
                return;
            }
            const observed = parseRunObservation(await api('/api/chat/agent/v2/observe', {
                method: 'POST', body: JSON.stringify({ operation: 'observe', expected_host_id: owner.expected_host_id,
                    client_id: owner.client_id, conversation_id: owner.conversation_id, expected_run_id: owner.run_id,
                    assistant_message_id: owner.assistant_message_id }),
            }), { hostId: owner.expected_host_id, clientId: owner.client_id, conversationId: owner.conversation_id },
            { run_id: owner.run_id, assistant_message_id: owner.assistant_message_id });
            if (!observed?.terminal_seal) throw new Error('Blocked Run seal unavailable');
            if (!disposed) onObserved();
        }
    }
    async function submit() {
        if (busy || disposed || !available || pending || !inspected || !confirming) return;
        busy = true; failed = false;
        try {
            const wire = createRecoveryDecision(owner, inspected, confirming, selected, reason,
                crypto.randomUUID(), crypto.randomUUID(), Date.now());
            await store.create(owner, wire);
            // The confirmed old-owner intent survives component disposal. Do
            // not send it after the user has switched away; it can be checked
            // from its original Run later, using the same frozen command.
            if (disposed) return;
            pending = wire;
            await reconcile(wire);
        } catch { if (!disposed) { failed = true; pending = await store.read(owner).catch(() => null); } }
        finally { if (!disposed) busy = false; }
    }
    async function checkDecision() {
        if (busy || disposed || !pending) return;
        busy = true; failed = false;
        try { await reconcile(pending); }
        catch { if (!disposed) failed = true; }
        finally { if (!disposed) busy = false; }
    }
</script>

{#if available || pending || failed}
<section tabindex="-1" use:activityRequestFocus={{conversationId: owner.conversation_id, runId: owner.run_id, active: available}} class="recovery" aria-label={t($locale, 'recovery.title')} aria-busy={busy}>
    <h3>{t($locale, 'recovery.title')}</h3>
    <p>{t($locale, 'recovery.description')}</p>
    {#if failed}<p role="alert">{t($locale, 'recovery.error')}</p>{/if}
    {#if result}<p role="status">{t($locale, `recovery.${result}`)}</p>{/if}
    {#if pending}
        <p>{t($locale, 'recovery.saved')}</p>
        <button type="button" disabled={busy} onclick={checkDecision}>{t($locale, 'recovery.checkDecision')}</button>
    {:else if available && confirming && inspected}
        <p>{t($locale, confirming === 'authorize_risk_retry' ? 'recovery.risk' : 'recovery.blockWarning')}</p>
        {#if confirming === 'authorize_risk_retry' && inspected.targets.length > 1}
            <label>{t($locale, 'recovery.target')}
                <select bind:value={selected} disabled={busy}>
                    {#each inspected.targets as _, index}<option value={index}>{t($locale, 'recovery.operation', { number: index + 1 })}</option>{/each}
                </select>
            </label>
        {/if}
        <label>{t($locale, 'recovery.reason')}<textarea bind:value={reason} rows="2" disabled={busy}></textarea></label>
        <div class="actions">
            <button type="button" disabled={busy} onclick={() => { confirming = null; reason = ''; }}>{t($locale, 'recovery.cancel')}</button>
            <button type="button" disabled={busy || !available || !reason.trim()} onclick={submit}>{t($locale, confirming === 'authorize_risk_retry' ? 'recovery.confirmRetry' : 'recovery.confirmBlock')}</button>
        </div>
    {:else}
        <div class="actions">
            <button type="button" disabled={busy || !available} onclick={inspect}>{t($locale, 'recovery.inspect')}</button>
            {#if checked && inspected}
                <button type="button" disabled={busy || !available || !inspected.retry_allowed} onclick={() => { confirming = 'authorize_risk_retry'; }}>{t($locale, 'recovery.retry')}</button>
                <button type="button" disabled={busy || !available || inspected.independent_pending_effects !== 0} onclick={() => { confirming = 'keep_unknown_blocked'; }}>{t($locale, 'recovery.block')}</button>
            {:else if checked}
                <button type="button" disabled={busy || !available} onclick={onResume}>{t($locale, 'chatPage.resume')}</button>
            {/if}
        </div>
    {/if}
</section>
{/if}

<style>
    .recovery { padding: 1rem; border: 1px solid var(--border); border-radius: .75rem; margin-block: .75rem; }
    h3 { margin: 0; font-size: 1rem; }
    p { line-height: 1.5; overflow-wrap: anywhere; }
    .actions { display: flex; gap: .75rem; flex-wrap: wrap; }
    label { display: grid; gap: .5rem; margin-block: .75rem; }
    textarea, select { width: 100%; font: inherit; }
    button { min-height: 2.75rem; white-space: normal; }
</style>
