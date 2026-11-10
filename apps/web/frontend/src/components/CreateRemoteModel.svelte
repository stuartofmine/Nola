<script lang="ts">
    import PrototypeIcon from './PrototypeIcon.svelte';
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { importExecutionCapabilities, type RemoteProfile } from '../lib/modelProfiles';
    import { validEndpointURL } from '../lib/modelEndpoints';
    import { newSourceId } from '../lib/dataSources';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';

    let { profiles, onStage }: { profiles: RemoteProfile[]; onStage: (profile: RemoteProfile) => void } = $props();
    let dialog: HTMLDialogElement;
    let confirmation: ConfirmationHandle;
    let endpointId = $state('');
    let endpointName = $state('');
    let baseURL = $state('');
    let wireProtocol = $state('chat_completions');
    let modelName = $state('');
    let displayName = $state('');
    let priority = $state(50);
    let capabilities = $state<string[]>(['chat']);
    let executionCapabilities = $state(importExecutionCapabilities());
    let opened = $state(false);
    let baseline = $state('');
    function draftSnapshot() {
        return JSON.stringify([endpointId, endpointName, baseURL, wireProtocol, modelName, displayName, priority, capabilities, executionCapabilities]);
    }
    const dirty = $derived(opened && draftSnapshot() !== baseline);
    const uses = ['chat', 'retrieval', 'issue_repair', 'sensitive_data'];
    const valid = $derived(!!modelName.trim() && Number.isSafeInteger(priority) && priority >= 0 && priority <= 100
        && capabilities.some(value => value !== 'sensitive_data')
        && (!!endpointId || (!!endpointName.trim() && validEndpointURL(baseURL)))
        && !profiles.some(profile => profile.endpoint.id === endpointId && profile.modelName.toLowerCase() === modelName.trim().toLowerCase()));

    export function open(endpoint?: RemoteProfile['endpoint'], initialModelName = '') {
        endpointId = endpoint?.id ?? '';
        endpointName = endpoint?.name ?? '';
        baseURL = endpoint?.baseURL ?? '';
        wireProtocol = endpoint?.wireProtocol ?? 'chat_completions';
        modelName = initialModelName; displayName = ''; priority = 50; capabilities = ['chat'];
        executionCapabilities = importExecutionCapabilities();
        baseline = draftSnapshot();
        opened = true;
        void tick().then(() => { if (opened && dialog?.isConnected && !dialog.open) dialog.showModal(); });
    }
    async function close(event?: Event) {
        event?.preventDefault();
        if (dirty
            && !await confirmation.ask({ title: t($locale, 'remote.discard_title'), description: t($locale, 'remote.discard_body'),
                confirmLabel: t($locale, 'models.discard'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true })) return;
        dialog.close(); opened = false;
    }
    function save(event: SubmitEvent) {
        event.preventDefault();
        if (!valid) return;
        const existingEndpoint = profiles.find(profile => profile.endpoint.id === endpointId)?.endpoint;
        const endpoint = existingEndpoint ? { ...existingEndpoint } : {
            id: newSourceId(), name: endpointName.trim(), baseURL: baseURL.trim(), wireProtocol,
            enabled: true, order: new Set(profiles.map(profile => profile.endpoint.id)).size,
        };
        onStage({ id: newSourceId(), provider: 'openai-compatible', displayName: displayName.trim() || modelName.trim(),
            modelName: modelName.trim(), revision: 0, enabled: true, selected: false, priority, endpoint,
            order: profiles.filter(profile => profile.endpoint.id === endpoint.id).length,
            capabilities: [...capabilities], executionCapabilities: { ...executionCapabilities }, reasoning: null,
            parameters: { temperature: 0.7, topP: 0.9, maxOutputTokens: 16384, maxContextLength: 128000,
                presencePenalty: 0, frequencyPenalty: 0, seed: null, stopSequences: '' } });
        dialog.close(); opened = false;
    }
</script>

<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (dirty) { event.preventDefault(); event.returnValue = ''; } }} />
<ConfirmationDialog bind:this={confirmation} />
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-label={t($locale, endpointId ? 'remote.add_model' : 'remote.add_endpoint_model')}>
    <form onsubmit={save}>
        <header><h2>{t($locale, endpointId ? 'remote.add_model' : 'remote.add_endpoint_model')}</h2><button type="button" class="close" aria-label={t($locale, 'shell.cancel')} onclick={() => close()}><PrototypeIcon name="close" /></button></header>
        <div class="fields">
            {#if !endpointId}
                <label>{t($locale, 'remote.endpoint_name')}<input bind:value={endpointName} maxlength="512" required /></label>
                <label>{t($locale, 'remote.base_url')}<input type="url" bind:value={baseURL} placeholder="https://api.example.com/v1" required /></label>
                <label>{t($locale, 'remote.protocol')}<select bind:value={wireProtocol}><option value="chat_completions">Chat Completions</option><option value="responses">Responses</option><option value="anthropic">Anthropic Messages</option></select></label>
            {:else}<p class="muted">{endpointName} · {baseURL}</p>{/if}
            <label>{t($locale, 'remote.model_id')}<input bind:value={modelName} maxlength="512" required /></label>
            <label>{t($locale, 'remote.display_name')}<input bind:value={displayName} maxlength="512" placeholder={modelName} /></label>
            <label>{t($locale, 'remote.priority')}<input type="number" bind:value={priority} min="0" max="100" step="1" required /></label>
            <div class="uses"><span>{t($locale, 'remote.capabilities_hint')}</span>{#each uses as use}<label><input type="checkbox" value={use} bind:group={capabilities} />{t($locale, `remote.capability.${use}`)}</label>{/each}</div>
            <div class="uses"><span>{t($locale, 'remote.execution_capabilities_hint')}</span>
                <label><input type="checkbox" bind:checked={executionCapabilities.toolCalls} />{t($locale, 'remote.execution.tool_calls')}</label>
                <label><input type="checkbox" bind:checked={executionCapabilities.imageInput} />{t($locale, 'remote.execution.image_input')}</label>
                <label><input type="checkbox" bind:checked={executionCapabilities.reasoning} />{t($locale, 'remote.execution.reasoning')}</label>
                <label><input type="checkbox" bind:checked={executionCapabilities.samplingParameters} />{t($locale, 'remote.execution.sampling')}</label>
            </div>
            <p class="muted">{t($locale, 'remote.execution_confirm_add')}</p>
            {#if wireProtocol === 'chat_completions'}
                <label>{t($locale, 'remote.execution.output_field')}
                    <select bind:value={executionCapabilities.chatOutputTokenField}>
                        <option value="auto">{t($locale, 'remote.execution.output_auto')}</option>
                        <option value="max_tokens">max_tokens</option>
                        <option value="max_completion_tokens">max_completion_tokens</option>
                    </select>
                </label>
            {/if}
        </div>
        {#if !valid && opened}<p class="muted">{t($locale, 'remote.create_hint')}</p>{/if}
        <div class="actions"><button type="button" onclick={() => close()}>{t($locale, 'shell.cancel')}</button><button class="primary" disabled={!valid}>{t($locale, 'remote.stage_changes')}</button></div>
    </form>
</dialog>
<style>dialog form{display:flex;flex-direction:column;gap:16px;max-height:calc(100dvh - 96px)}header{display:flex;justify-content:space-between;align-items:center;gap:16px}header h2{margin:0}.close{display:grid;place-items:center;width:44px;min-height:44px;padding:0;border-color:transparent;background:transparent}header,.actions,form>.muted{flex:none}.fields{flex:1;min-height:0;overflow:auto;display:grid;grid-template-columns:minmax(0,1fr);gap:12px;border:0;padding:0;margin:0}.uses{display:flex;flex-wrap:wrap;gap:8px 12px;font-size:12px}.uses span{flex-basis:100%;color:var(--muted)}.uses label{display:flex;align-items:center;gap:5px}.uses input{width:auto}.muted{color:var(--muted);overflow-wrap:anywhere}</style>
