<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { onDestroy } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { modelProbeConfirmation, type ConnectionSnapshot, type CatalogModel, type ExecutionCapabilities } from '../lib/modelConnections';
    let { snapshot, model, accountRevision, refresh }: {snapshot: ConnectionSnapshot; model: CatalogModel; accountRevision: number | null; refresh: () => Promise<void>} = $props();
    let confirmed = $state(false), pending = $state(false), message = $state('');
    let request = $state<{request_id: string; connection_id: string; expected_revision: number} | null>(null);
    const assessment = $derived(snapshot.document.capability_assessments?.find(item => item.model_id === model.model_id && item.protocol === model.protocol));
    const credentialBinding = $derived(snapshot.probe_credential_bindings?.[model.protocol]);
    let override = $state<ExecutionCapabilities>({toolCalls:false,imageInput:false,reasoning:false,samplingParameters:false,samplingRequiresNoReasoning:false,chatOutputTokenField:'auto'});
    $effect(() => { override = {...(assessment?.user_override ?? {toolCalls:false,imageInput:false,reasoning:false,samplingParameters:false,samplingRequiresNoReasoning:false,chatOutputTokenField:'auto'})}; });
    $effect(() => { confirmed = false; snapshot.revision; accountRevision; credentialBinding; });
    async function probe() {
        if (!confirmed || pending || !credentialBinding) return;
        pending = true; message = '';
        const active = {request_id:crypto.randomUUID(), connection_id:snapshot.document.connection.id, expected_revision:snapshot.revision};
        request = active;
        try {
            await api('/api/model-connections/probe', {method:'POST', body:JSON.stringify(
                modelProbeConfirmation(snapshot, model, accountRevision, active.request_id, confirmed))});
            message = t($locale, 'connections.probeVerified');
        } catch {
            message = t($locale, 'connections.probeUnconfirmed');
        } finally {
            request = null; pending = false; confirmed = false; await refresh();
        }
    }
    async function cancel() {
        const active = request;
        if (!active) return;
        const result = await api<{cancelled:boolean}>('/api/model-connections/discover/cancel', {method:'POST', body:JSON.stringify(active)});
        message = result.cancelled
            ? t($locale, 'connections.probeCancelled')
            : t($locale, 'connections.probeFinished');
        if (!result.cancelled) await refresh();
    }
    async function saveOverride(clear: boolean) {
        pending = true;
        try {
            await api('/api/model-connections', {method:'POST', body:JSON.stringify({
                type:'set_connection_capability_override', connection_id:snapshot.document.connection.id,
                expected_revision:snapshot.revision, model_id:model.model_id, protocol:model.protocol,
                capabilities:clear ? null : override, updated_at_ms:Date.now()})});
            message = clear ? t($locale, 'connections.overrideCleared') : t($locale, 'connections.overrideSaved');
            await refresh();
        } catch { message = t($locale, 'connections.overrideFailed'); }
        finally { pending = false; }
    }
    onDestroy(() => { if (request) void cancel().catch(() => {}); });
</script>

<details class="capability">
    <summary>{t($locale, 'connections.probeTitle')}</summary>
    <p>{t($locale, assessment?.evidence?.text_generation && assessment.evidence.account_revision === accountRevision && assessment.evidence.credential_binding === credentialBinding ? 'connections.probeVerified' : 'connections.probeUnknown')}</p>
    <p>{snapshot.document.connection.display_name} · {model.display_name} · {model.protocol}</p>
    <p>{t($locale, 'connections.probeCost')}</p>
    <label><input type="checkbox" bind:checked={confirmed} disabled={pending} /> {t($locale, 'connections.probeConsent')}</label>
    <button disabled={!confirmed || pending || !credentialBinding || !snapshot.document.connection.enabled} onclick={() => void probe()}>{t($locale, 'connections.probeSend')}</button>
    {#if request}<button onclick={() => void cancel().catch(() => { message = t($locale, 'connections.probeCancelFailed'); })}>{t($locale, 'rooms.cancel')}</button>{/if}
    <details>
        <summary>{t($locale, 'connections.overrideTitle')} · {t($locale, assessment?.user_override ? 'connections.enabled' : 'connections.disabled')}</summary>
        <p>{t($locale, 'connections.overrideHint')}</p>
        <label><input type="checkbox" bind:checked={override.toolCalls} /> {t($locale, 'remote.execution.tool_calls')}</label>
        <label><input type="checkbox" bind:checked={override.imageInput} /> {t($locale, 'remote.execution.image_input')}</label>
        <label><input type="checkbox" bind:checked={override.reasoning} /> {t($locale, 'chatSession.reasoning')}</label>
        <label><input type="checkbox" bind:checked={override.samplingParameters} /> {t($locale, 'connections.sampling')}</label>
        <label><input type="checkbox" bind:checked={override.samplingRequiresNoReasoning} /> {t($locale, 'connections.samplingNoReasoning')}</label>
        <label>{t($locale, 'connections.outputField')}
            <select bind:value={override.chatOutputTokenField}>
                <option value="auto">{t($locale, 'chatSession.automatic')}</option>
                <option value="max_tokens">max_tokens</option>
                <option value="max_completion_tokens">max_completion_tokens</option>
            </select>
        </label>
        <button disabled={pending} onclick={() => void saveOverride(false)}>{t($locale, 'chatSession.save')}</button>
        <button disabled={pending || !assessment?.user_override} onclick={() => void saveOverride(true)}>{t($locale, 'connections.overrideClear')}</button>
    </details>
    {#if message}<p role="status">{message}</p>{/if}
</details>
<style>
    .capability { margin: .5rem 0 1rem; padding: .75rem; border: 1px solid var(--border, #8886); border-radius: .5rem; overflow-wrap: anywhere; }
    p { margin: .5rem 0; font-size: .875rem; }
    label { display: block; margin: .5rem 0; }
    button { margin: .25rem; }
</style>
