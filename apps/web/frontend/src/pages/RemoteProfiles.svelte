<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { anchoredPopover } from '../lib/anchoredPopover';
    import { untrack, tick } from 'svelte';
    import { dismissibleDisclosure } from '../lib/dismissibleDisclosure';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { ModelValidationError, modelValidationText, type ModelValidationIssue } from '../lib/modelValidation';
    import { importExecutionCapabilities, executionParameterValidationError, groupedProfiles, parameterChanges, reasoningDraft, reasoningChanges, metadataDraft, metadataChanges, reasoningOptions, reasoningValidationError, reasoningExecutionValidationError } from '../lib/modelProfiles';
    import type { RemoteProfile } from '../lib/modelProfiles';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import ModelEndpointEditor from '../components/ModelEndpointEditor.svelte';
    import EndpointCredentialEditor from '../components/EndpointCredentialEditor.svelte';
    import CreateRemoteModel from '../components/CreateRemoteModel.svelte';
    import { protocolTitle } from '../lib/modelEndpoints';
    import { catalogDraftDirty, catalogDraftValidationError, catalogReplacement, copyRemoteCatalog, discoveredModelsForEndpoint, endpointDiscoveryMatchesSaved, primaryChatModelId,
        stageCatalogDelete, stageCatalogMove, stageEndpoint, stageModel } from '../lib/remoteCatalogDraft';
    import type { DiscoveredModelCatalog } from '../lib/remoteCatalogDraft';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { locale, t } from '../lib/i18n';
    let confirmation: ConfirmationHandle;
    let endpointEditor: ModelEndpointEditor;
    let credentialEditor: EndpointCredentialEditor;
    let creation: CreateRemoteModel;
    let { profiles, openProfileId = '', reload, onSelectionSaved }: { profiles: RemoteProfile[]; openProfileId?: string; reload: () => Promise<void>; onSelectionSaved: (profiles: RemoteProfile[]) => void } = $props();
    let openedProfileId = '';
    let editing = $state<RemoteProfile>();
    let draft = $state<RemoteProfile['parameters']>({});
    let thinking = $state(reasoningDraft(null));
    let metadata = $state<ReturnType<typeof metadataDraft>>({ displayName: '', enabled: true, priority: 50, capabilities: [],
        executionCapabilities: importExecutionCapabilities(), executionCapabilitiesConfirmed: false });
    const capabilities = ['chat', 'retrieval', 'issue_repair', 'sensitive_data'];
    function capabilityTitle(value: string) { return capabilities.includes(value) ? t($locale, `remote.capability.${value}`) : value; }
    function thinkingTitle(value: string) { return ['concise', 'balanced', 'detailed', 'automatic'].includes(value) ? t($locale, `remote.thinking.${value}`) : ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].includes(value) ? t($locale, `modelDisplay.effort.${value}`) : value; }
    let error = $state('');
    let selectionError = $state<string | ModelValidationIssue>('');
    let selectionNotice = $state('');
    let catalogWarning = $state('');
    let busy = $state(false);
    let discoveringEndpoint = $state('');
    let discoveredModels = $state<Record<string, DiscoveredModelCatalog>>({});
    let credentialGenerations = $state<Record<string, number>>({});
    let discoveryNotice = $state('');
    let discoveryError = $state('');
    let dialog: HTMLDialogElement;
    let savedCatalog = $state<RemoteProfile[]>(untrack(() => copyRemoteCatalog(profiles)));
    let draftCatalog = $state<RemoteProfile[]>(untrack(() => copyRemoteCatalog(profiles)));
    const catalogDirty = $derived(catalogDraftDirty(savedCatalog, draftCatalog));
    const catalogError = $derived(catalogDraftValidationError(draftCatalog));
    $effect(() => {
        const incoming = profiles;
        if (catalogDirty) return;
        savedCatalog = copyRemoteCatalog(incoming);
        draftCatalog = copyRemoteCatalog(incoming);
    });
    const groups = $derived(groupedProfiles(draftCatalog));
    const primaryModelId = $derived(primaryChatModelId(draftCatalog));
    const editableGroups = $derived(groups.filter(group => group.models.some(model => model.provider === 'openai-compatible')));
    const thinkingOptions = $derived(reasoningOptions(thinking, editing?.endpoint.wireProtocol ?? '', Number(draft.maxOutputTokens ?? 16384)));
    const protocolEfforts = $derived(editing?.endpoint.wireProtocol === 'responses'
        ? ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'] : ['low', 'medium', 'high']);
    const displayedEfforts = $derived([...new Set([...protocolEfforts, ...thinking.efforts])]);
    const declarationError = $derived<ModelValidationIssue | null>(metadata.executionCapabilitiesConfirmed
        ? null : { code: 'modelValidation.executionDeclaration' });
    const thinkingError = $derived(reasoningExecutionValidationError(thinking, metadata.executionCapabilities)
        ?? reasoningValidationError(thinking, editing?.endpoint.wireProtocol ?? '', Number(draft.maxOutputTokens ?? 16384)));
    const parameterError = $derived(editing ? executionParameterValidationError({ ...editing,
        parameters: { ...editing.parameters, ...draft }, executionCapabilities: metadata.executionCapabilities }) : null);
    const invalidThinkingDefault = $derived(thinking.defaultPreference != null && thinking.defaultPreference !== 'automatic' && !thinkingOptions.includes(thinking.defaultPreference));
    const dirty = $derived(!!editing && (Object.keys(metadataChanges(editing, metadata)).length > 0 || Object.keys(parameterChanges(editing.parameters, draft)).length > 0 || Object.keys(reasoningChanges(editing.reasoning, thinking)).length > 0));
    const fields = [
        { key: 'temperature', label: 'models.temperature', min: 0, max: 2, step: 0.05 },
        { key: 'topP', label: 'modelDisplay.topP', min: 0, max: 1, step: 0.01 },
        { key: 'maxOutputTokens', label: 'remote.output', min: 64, max: 1000000, step: 1 },
        { key: 'maxContextLength', label: 'models.context', min: 512, max: 4000000, step: 1 },
        { key: 'presencePenalty', label: 'remote.presence', min: -2, max: 2, step: 0.1 },
        { key: 'frequencyPenalty', label: 'remote.frequency', min: -2, max: 2, step: 0.1 },
    ];
    async function edit(model: RemoteProfile) {
        editing = model; metadata = metadataDraft(model); draft = { ...model.parameters }; thinking = reasoningDraft(model.reasoning); error = '';
        await tick();
        if (editing === model && dialog?.isConnected && !dialog.open) dialog.showModal();
    }
    $effect(() => {
        if (!openProfileId || openProfileId === openedProfileId || !dialog) return;
        const model = draftCatalog.find(profile => profile.id === openProfileId);
        if (!model) return;
        openedProfileId = openProfileId;
        edit(model);
    });
    async function discover(group: ReturnType<typeof groupedProfiles>[number]) {
        if (busy || discoveringEndpoint || !endpointDiscoveryMatchesSaved(savedCatalog, draftCatalog, group.endpoint.id)) return;
        discoveringEndpoint = group.endpoint.id;
        const credentialGeneration = credentialGenerations[group.endpoint.id] ?? 0;
        discoveryNotice = ''; discoveryError = '';
        try {
            const result = await api<{ models: string[] }>(`/api/model-endpoints/${encodeURIComponent(group.endpoint.id)}/discover`, { method: 'POST' });
            if ((credentialGenerations[group.endpoint.id] ?? 0) !== credentialGeneration) return;
            const existing = new Set(group.models.map(model => model.modelName));
            const available = result.models.filter(name => !existing.has(name));
            discoveredModels = { ...discoveredModels, [group.endpoint.id]: {
                baseURL: group.endpoint.baseURL, wireProtocol: group.endpoint.wireProtocol, models: available,
            } };
            discoveryNotice = t($locale, available.length ? 'remote.discovery_count' : 'remote.discovery_empty', { count: available.length });
        } catch (cause) {
            if ((credentialGenerations[group.endpoint.id] ?? 0) !== credentialGeneration) return;
            discoveredModels = { ...discoveredModels, [group.endpoint.id]: {
                baseURL: group.endpoint.baseURL, wireProtocol: group.endpoint.wireProtocol, models: [],
            } };
            discoveryError = errorMessage(cause);
        } finally { discoveringEndpoint = ''; }
    }
    function editEndpoint(endpointId: string) {
        try { endpointEditor.open(draftCatalog, endpointId); }
        catch (cause) { selectionError = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
    }
    function setEndpointEnabled(endpointId: string, enabled: boolean) { selectionNotice = ''; draftCatalog = stageEndpoint(draftCatalog, endpointId, { enabled }); }
    function setModelEnabled(model: RemoteProfile, enabled: boolean) { selectionNotice = ''; draftCatalog = stageModel(draftCatalog, model.id, { enabled }); }
    async function mutateCatalog(action: 'move_endpoint' | 'move_model' | 'delete_endpoint' | 'delete_model', targetId: string, name: string, direction?: -1 | 1) {
        if (busy) return;
        if (action.startsWith('delete')) {
            const endpoint = action === 'delete_endpoint';
            const accepted = await confirmation.ask({ title: t($locale, endpoint ? 'remote.delete_endpoint' : 'remote.delete_model'),
                description: t($locale, endpoint ? 'remote.delete_endpoint_body' : 'remote.delete_model_body', { name }),
                confirmLabel: t($locale, endpoint ? 'remote.delete_endpoint' : 'remote.delete_model'), destructive: true });
            if (!accepted) return;
        }
        selectionError = ''; selectionNotice = ''; catalogWarning = '';
        draftCatalog = action.startsWith('delete')
            ? stageCatalogDelete(draftCatalog, action === 'delete_endpoint' ? 'endpoint' : 'model', targetId)
            : stageCatalogMove(draftCatalog, action === 'move_endpoint' ? 'endpoint' : 'model', targetId, direction!);
    }
    async function close(event?: Event, refresh = false) {
        event?.preventDefault();
        if (busy) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'remote.discard_title'), description: t($locale, 'remote.discard_body'), confirmLabel: t($locale, 'models.discard'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true })) return;
        dialog.close(); editing = undefined;
        if (refresh) await reload();
    }
    async function save(event: SubmitEvent) {
        event.preventDefault(); if (busy || !editing || declarationError || thinkingError || parameterError) return;
        busy = true; error = '';
        draftCatalog = stageModel(draftCatalog, editing.id, {
            ...metadataChanges(editing, metadata), parameters: { ...editing.parameters, ...parameterChanges(editing.parameters, draft) },
            reasoning: Object.keys(reasoningChanges(editing.reasoning, thinking)).length ? reasoningDraft(thinking) : editing.reasoning,
        });
        selectionNotice = '';
        dialog.close(); editing = undefined;
        busy = false;
    }
    function discardCatalog() { savedCatalog = copyRemoteCatalog(profiles); draftCatalog = copyRemoteCatalog(profiles); selectionError = ''; }
    async function reloadCatalog() {
        if (catalogDirty && !await confirmation.ask({ title: t($locale, 'remote.discard_title'),
            description: t($locale, 'remote.discard_body'), confirmLabel: t($locale, 'models.discard'), destructive: true })) return;
        discardCatalog();
        await reload();
    }
    async function saveCatalog() {
        if (busy || !catalogDirty || dirty || catalogError) return;
        busy = true; selectionError = ''; catalogWarning = '';
        try {
            const result = await api<{ profiles: RemoteProfile[]; credentialCleanupPending?: boolean }>('/api/model-profiles/catalog/replace', {
                method: 'POST', body: JSON.stringify(catalogReplacement(savedCatalog, draftCatalog)),
            });
            savedCatalog = copyRemoteCatalog(result.profiles);
            draftCatalog = copyRemoteCatalog(result.profiles);
            onSelectionSaved(result.profiles);
            if (result.credentialCleanupPending) catalogWarning = 'remote.credential_cleanup_pending';
            else selectionNotice = t($locale, 'remote.saved');
        } catch (cause) { selectionError = errorMessage(cause); }
        finally { busy = false; }
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return;
        if (dirty || catalogDirty) { event.preventDefault(); event.returnValue = ''; }
    }
</script>

<svelte:window onbeforeunload={beforeUnload} />
<ConfirmationDialog bind:this={confirmation} />
<ModelEndpointEditor bind:this={endpointEditor} onSaved={onSelectionSaved} onBusyChange={value => busy = value}
    onStage={(id, changes) => { selectionNotice = ''; draftCatalog = stageEndpoint(draftCatalog, id, changes); }} />
<EndpointCredentialEditor bind:this={credentialEditor} onBusyChange={value => busy = value}
    onChanged={endpointID => {
        credentialGenerations = { ...credentialGenerations, [endpointID]: (credentialGenerations[endpointID] ?? 0) + 1 };
        const next = { ...discoveredModels }; delete next[endpointID]; discoveredModels = next;
    }} />
<CreateRemoteModel bind:this={creation} profiles={draftCatalog} onStage={profile => { selectionNotice = ''; draftCatalog = [...draftCatalog, profile]; }} />
{#if selectionError}<div class="notice error" role="alert">{modelValidationText($locale, selectionError)} <button disabled={busy} onclick={reloadCatalog}>{t($locale, 'models.reload')}</button></div>{/if}
{#if catalogDirty && catalogError}<div class="notice error" role="alert">{modelValidationText($locale, catalogError)}</div>{/if}
{#if selectionNotice}<p role="status">{selectionNotice}</p>{/if}
{#if catalogWarning}<p class="notice error" role="alert">{t($locale, catalogWarning)}</p>{/if}
{#if discoveryError}<p class="notice error" role="alert">{t($locale, 'remote.discovery_failed')}: {discoveryError}</p>{/if}
{#if discoveryNotice}<p role="status">{discoveryNotice}</p>{/if}
<div class="catalog-actions">
    <button type="button" class:primary={!catalogDirty} disabled={busy} onclick={() => creation.open()}>{t($locale, 'remote.add_endpoint_model')}</button>
    {#if catalogDirty}
        <button type="button" disabled={busy} onclick={discardCatalog}>{t($locale, 'models.discard')}</button>
        <button type="button" class="primary" disabled={busy || !!catalogError} onclick={saveCatalog}>{t($locale, busy ? 'models.saving' : 'models.save')}</button>
    {/if}
</div>
{#if !draftCatalog.length}<p class="empty-catalog">{t($locale, 'remote.empty_catalog')}</p>{/if}
{#if draftCatalog.length}<p class="routing-policy">{t($locale, 'remote.routing_policy')}</p>{/if}
{#if catalogDirty}<p class="muted" role="status">{t($locale, 'remote.unsaved_changes')}</p>{/if}
<div class="groups">
    {#each groups as group (group.endpoint.id)}
        <details class="endpoint-card"><summary class="endpoint-summary"><strong>{group.endpoint.name || group.endpoint.baseURL}</strong><span class="muted">{group.models.length} · {protocolTitle(group.endpoint.wireProtocol)}</span></summary>
            <header class="row between endpoint-head">
                <div class="identity"><h2>{group.endpoint.name || group.endpoint.baseURL}</h2><p class="muted endpoint" title={group.endpoint.baseURL}>{group.endpoint.baseURL}</p><p class="muted">{protocolTitle(group.endpoint.wireProtocol || 'chat_completions')}</p></div>
                {#if group.models.some(model => model.provider === 'openai-compatible')}
                    <label class="inline-toggle"><input type="checkbox" checked={group.endpoint.enabled} disabled={busy} aria-label={t($locale, 'remote.enable_endpoint')} onchange={event => setEndpointEnabled(group.endpoint.id, event.currentTarget.checked)} />{t($locale, 'remote.enable_endpoint')}</label>
                {/if}
                <div class="actions wide-actions">
                    {#if group.models.some(model => model.provider === 'openai-compatible')}
                        {@const position = editableGroups.findIndex(item => item.endpoint.id === group.endpoint.id)}
                        <button aria-label={t($locale, 'remote.move_up')} title={t($locale, 'remote.move_up')} disabled={busy || position <= 0} onclick={() => mutateCatalog('move_endpoint', group.endpoint.id, group.endpoint.name, -1)}>↑</button>
                        <button aria-label={t($locale, 'remote.move_down')} title={t($locale, 'remote.move_down')} disabled={busy || position >= editableGroups.length - 1} onclick={() => mutateCatalog('move_endpoint', group.endpoint.id, group.endpoint.name, 1)}>↓</button>
                        <button class="danger" disabled={busy} onclick={() => mutateCatalog('delete_endpoint', group.endpoint.id, group.endpoint.name)}>{t($locale, 'remote.delete_endpoint')}</button>
                    {/if}
                    <button disabled={busy || catalogDirty || !savedCatalog.some(model => model.endpoint.id === group.endpoint.id)} onclick={() => credentialEditor.open(group.endpoint.id, group.endpoint.name || '')}>{t($locale, 'remote.credentials')}</button>
                    <button disabled={busy || !group.endpoint.id} onclick={() => editEndpoint(group.endpoint.id)}>{t($locale, 'remote.edit_endpoint')}</button>
                </div>
                <details class="compact-actions" use:dismissibleDisclosure><summary>{t($locale, 'remote.more_actions')}</summary><div class="compact-menu" popover="manual" use:anchoredPopover>
                    <button disabled={busy || !group.endpoint.id} onclick={() => editEndpoint(group.endpoint.id)}>{t($locale, 'remote.edit_endpoint')}</button>
                    <button disabled={busy || catalogDirty || !savedCatalog.some(model => model.endpoint.id === group.endpoint.id)} onclick={() => credentialEditor.open(group.endpoint.id, group.endpoint.name || '')}>{t($locale, 'remote.credentials')}</button>
                    {#if group.models.some(model => model.provider === 'openai-compatible')}
                        {@const position = editableGroups.findIndex(item => item.endpoint.id === group.endpoint.id)}
                        <button disabled={busy || position <= 0} onclick={() => mutateCatalog('move_endpoint', group.endpoint.id, group.endpoint.name, -1)}>{t($locale, 'remote.move_up')}</button>
                        <button disabled={busy || position >= editableGroups.length - 1} onclick={() => mutateCatalog('move_endpoint', group.endpoint.id, group.endpoint.name, 1)}>{t($locale, 'remote.move_down')}</button>
                        <button class="danger" disabled={busy} onclick={() => mutateCatalog('delete_endpoint', group.endpoint.id, group.endpoint.name)}>{t($locale, 'remote.delete_endpoint')}</button>
                    {/if}
                </div></details>
            </header>
            <div class="model-toolbar row between"><h3>{t($locale, 'remote.models')} <span class="muted">{group.models.length}</span></h3>
                {#if group.models.some(model => model.provider === 'openai-compatible')}
                    {@const discovered = discoveredModelsForEndpoint(discoveredModels[group.endpoint.id], group.endpoint)
                        .filter(name => !group.models.some(model => model.modelName === name))}
                    <div class="actions">
                    {#if endpointDiscoveryMatchesSaved(savedCatalog, draftCatalog, group.endpoint.id) && discovered.length}
                        <details class="discovered" use:dismissibleDisclosure><summary>{t($locale, 'remote.add_discovered')} ({discovered.length})</summary>
                            <div class="discovered-list" popover="manual" use:anchoredPopover>{#each discovered as name (name)}<button type="button" disabled={busy} onclick={() => creation.open(group.endpoint, name)}>{name}</button>{/each}</div>
                        </details>
                    {/if}
                    <button disabled={busy || !!discoveringEndpoint || !endpointDiscoveryMatchesSaved(savedCatalog, draftCatalog, group.endpoint.id)} onclick={() => discover(group)}>{discoveringEndpoint === group.endpoint.id ? t($locale, 'remote.discovering') : t($locale, 'remote.discover')}</button>
                    <button disabled={busy} onclick={() => creation.open(group.endpoint)}>{t($locale, 'remote.add_model')}</button>
                </div>{/if}
            </div>
            {#each group.models as model (model.id)}
                <article class="model">
                    <div class="row between"><div class="identity"><h2>{model.displayName || model.modelName}</h2><p class="muted">{model.modelName}</p></div><div class="actions wide-actions"><button onclick={() => edit(model)} disabled={busy}>{t($locale, 'models.configure')}…</button>
                        {#if model.provider === 'openai-compatible'}
                            {@const members = group.models.filter(value => value.provider === 'openai-compatible')}
                            {@const position = members.findIndex(value => value.id === model.id)}
                            <button aria-label={t($locale, 'remote.move_up')} title={t($locale, 'remote.move_up')} disabled={busy || position <= 0} onclick={() => mutateCatalog('move_model', model.id, model.displayName || model.modelName, -1)}>↑</button>
                            <button aria-label={t($locale, 'remote.move_down')} title={t($locale, 'remote.move_down')} disabled={busy || position >= members.length - 1} onclick={() => mutateCatalog('move_model', model.id, model.displayName || model.modelName, 1)}>↓</button>
                            <button class="danger" disabled={busy || members.length <= 1} onclick={() => mutateCatalog('delete_model', model.id, model.displayName || model.modelName)}>{t($locale, 'remote.delete_model')}</button>
                        {/if}
                    </div><details class="compact-actions" use:dismissibleDisclosure><summary>{t($locale, 'remote.more_actions')}</summary><div class="compact-menu" popover="manual" use:anchoredPopover>
                        <button onclick={() => edit(model)} disabled={busy}>{t($locale, 'models.configure')}…</button>
                        {#if model.provider === 'openai-compatible'}
                            {@const members = group.models.filter(value => value.provider === 'openai-compatible')}
                            {@const position = members.findIndex(value => value.id === model.id)}
                            <button disabled={busy || position <= 0} onclick={() => mutateCatalog('move_model', model.id, model.displayName || model.modelName, -1)}>{t($locale, 'remote.move_up')}</button>
                            <button disabled={busy || position >= members.length - 1} onclick={() => mutateCatalog('move_model', model.id, model.displayName || model.modelName, 1)}>{t($locale, 'remote.move_down')}</button>
                            <button class="danger" disabled={busy || members.length <= 1} onclick={() => mutateCatalog('delete_model', model.id, model.displayName || model.modelName)}>{t($locale, 'remote.delete_model')}</button>
                        {/if}
                    </div></details></div>
                    {#if model.provider === 'openai-compatible'}<label class="inline-toggle"><input type="checkbox" checked={model.enabled} disabled={busy} aria-label={`${t($locale, 'remote.enable_model')}: ${model.displayName || model.modelName}`} onchange={event => setModelEnabled(model, event.currentTarget.checked)} />{t($locale, 'remote.enable_model')}</label>{/if}
                    <p class="muted">{t($locale, 'remote.priority')} {model.priority}{primaryModelId === model.id ? ` · ${t($locale, 'remote.default_model')}` : ''}</p>
                    <p class="muted">{(model.capabilities || []).map(capabilityTitle).join(' · ')}</p>
                    {#if model.executionCapabilities == null}
                        <p class="capability-notice">{t($locale, 'remote.execution_confirm_required')}
                            <button type="button" onclick={() => edit(model)} disabled={busy}>{t($locale, 'models.configure')}…</button></p>
                    {:else if !model.executionCapabilities.toolCalls}
                        <p class="muted">{t($locale, 'remote.execution_text_only')}</p>
                    {/if}
                    {#if model.reasoning?.control === 'effort'}<p class="muted">{t($locale, 'remote.effort_summary', { values: model.reasoning.efforts?.join(' / ') ?? '' })}</p>{/if}
                    {#if model.reasoning?.control === 'budget'}<p class="muted">{t($locale, 'remote.budget_summary', { values: model.reasoning.budgets?.join(' / ') ?? '' })}</p>{/if}
                </article>
            {/each}
        </details>
    {/each}
</div>
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-label={t($locale, 'remote.configure_title', { name: editing?.displayName || editing?.modelName || '' })}>
    {#if editing}<form onsubmit={save} aria-busy={busy}>
        <div><h2>{editing.displayName || editing.modelName}</h2><p class="muted">{t($locale, 'remote.defaults_hint')}</p></div>
        {#if error}<div role="alert" class="error"><p>{error}</p><button type="button" disabled={busy} onclick={() => close(undefined, true)}>{t($locale, 'remote.discard_refresh')}</button></div>{/if}
        <fieldset disabled={busy}>
            <legend>{t($locale, 'remote.model')}</legend>
            <div class="fields">
                <label>{t($locale, 'remote.display_name')}<input bind:value={metadata.displayName} maxlength="512" required /></label>
                <label>{t($locale, 'remote.priority')}<input type="number" bind:value={metadata.priority} min="0" max="100" step="1" required /></label>
            </div>
            <div class="efforts"><label><input type="checkbox" bind:checked={metadata.enabled} />{t($locale, 'remote.enable_model')}</label></div>
            <p class="muted">{t($locale, 'remote.capabilities_hint')}</p>
            <div class="efforts">{#each capabilities as capability}<label><input type="checkbox" value={capability} bind:group={metadata.capabilities} />{capabilityTitle(capability)}</label>{/each}</div>
            <p class="muted">{t($locale, 'remote.execution_capabilities_hint')}</p>
            <div class="efforts execution-capabilities">
                <label><input type="checkbox" bind:checked={metadata.executionCapabilities.toolCalls} />{t($locale, 'remote.execution.tool_calls')}</label>
                <label><input type="checkbox" bind:checked={metadata.executionCapabilities.imageInput} />{t($locale, 'remote.execution.image_input')}</label>
                <label><input type="checkbox" bind:checked={metadata.executionCapabilities.reasoning} />{t($locale, 'remote.execution.reasoning')}</label>
                <label><input type="checkbox" bind:checked={metadata.executionCapabilities.samplingParameters} />{t($locale, 'remote.execution.sampling')}</label>
                <label><input type="checkbox" bind:checked={metadata.executionCapabilities.samplingRequiresNoReasoning} disabled={!metadata.executionCapabilities.samplingParameters} />{t($locale, 'remote.execution.sampling_none_only')}</label>
            </div>
            {#if editing.endpoint.wireProtocol === 'chat_completions'}
                <label>{t($locale, 'remote.execution.output_field')}
                    <select bind:value={metadata.executionCapabilities.chatOutputTokenField}>
                        <option value="auto">{t($locale, 'remote.execution.output_auto')}</option>
                        <option value="max_tokens">max_tokens</option>
                        <option value="max_completion_tokens">max_completion_tokens</option>
                    </select>
                </label>
            {/if}
            <p class="muted">{t($locale, 'remote.execution_unchecked_hint')}</p>
            {#if declarationError}
                <p class="capability-notice" role="alert">{modelValidationText($locale, declarationError)}</p>
                <button type="button" onclick={() => metadata.executionCapabilitiesConfirmed = true} disabled={busy}>
                    {t($locale, 'remote.execution_confirm_settings')}
                </button>
            {/if}
        </fieldset>
        <div class="fields">
            {#each fields as field}<label>{t($locale, field.label)}<input type="number" value={Number(draft[field.key] ?? 0)} oninput={event => draft[field.key] = event.currentTarget.valueAsNumber} min={field.min} max={field.max} step={field.step === 1 ? 1 : 'any'} disabled={busy} required /></label>{/each}
            <label>{t($locale, 'remote.seed')}<input type="number" value={draft.seed == null ? '' : Number(draft.seed)} oninput={event => draft.seed = event.currentTarget.value === '' ? null : event.currentTarget.valueAsNumber} step="1" disabled={busy} /></label>
            <label>{t($locale, 'remote.stop_sequences')}<textarea value={String(draft.stopSequences || '')} oninput={event => draft.stopSequences = event.currentTarget.value} disabled={busy}></textarea></label>
        </div>
        <fieldset disabled={busy}>
            <legend>{t($locale, 'remote.thinking.title')}</legend>
            <p class="muted">{t($locale, 'remote.thinking.hint')}</p>
            <label>{t($locale, 'remote.thinking.control')}<select bind:value={thinking.control} onchange={() => { thinking.defaultPreference = null; }}>
                <option value="automatic">{t($locale, 'remote.thinking.automatic')}</option>
                {#if editing.endpoint.wireProtocol === 'anthropic'}
                    <option value="budget">{t($locale, 'remote.thinking.budget')}</option>
                {:else}<option value="effort">{t($locale, 'remote.thinking.effort')}</option>{/if}
            </select></label>
            {#if thinking.control === 'effort'}
                <div class="efforts">
                    {#each displayedEfforts as effort}
                        <label><input type="checkbox" value={effort} bind:group={thinking.efforts} />{protocolEfforts.includes(effort) ? thinkingTitle(effort) : t($locale, 'remote.thinking.check', { value: effort })}</label>
                    {/each}
                </div>
                {#if editing.endpoint.wireProtocol !== 'responses'}<p class="muted">{t($locale, 'remote.thinking.tiers_hint')}</p>{/if}
            {:else if thinking.control === 'budget'}
                <div class="fields">
                    {#each ['concise', 'balanced', 'detailed'] as title, index}
                        <label>{thinkingTitle(title)}<input type="number" bind:value={thinking.budgets[index]} min="1024" max={Number(draft.maxOutputTokens ?? 16384) - 1} step="1" required /></label>
                    {/each}
                </div>
                <p class="muted">{t($locale, 'remote.thinking.budgets_hint')}</p>
            {/if}
            {#if thinkingOptions.length > 0 || invalidThinkingDefault}
                <label>{t($locale, 'remote.thinking.default')}<select value={thinking.defaultPreference ?? 'automatic'} onchange={event => { thinking.defaultPreference = event.currentTarget.value; }}>
                    <option value="automatic">{t($locale, 'remote.thinking.automatic')}</option>
                    {#if invalidThinkingDefault}<option value={thinking.defaultPreference ?? ''} disabled>{t($locale, 'remote.thinking.unsupported', { value: thinking.defaultPreference ?? '' })}</option>{/if}
                    {#each thinkingOptions as option}
                        <option value={option}>{thinkingTitle(option)}</option>
                    {/each}
                </select></label>
            {/if}
            {#if thinkingError}<p role="alert" class="error">{modelValidationText($locale, thinkingError)}</p>{/if}
        </fieldset>
        {#if parameterError}<p role="alert" class="error">{modelValidationText($locale, parameterError)}</p>{/if}
        <div class="actions"><button type="button" onclick={() => close()} disabled={busy}>{t($locale, 'shell.cancel')}</button><button class="primary" disabled={busy || !dirty || !!declarationError || !!thinkingError || !!parameterError}>{t($locale, busy ? 'models.saving' : 'models.save')}</button></div>
    </form>{/if}
</dialog>
<style>
    .endpoint-summary { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
    .endpoint-card[open] > summary { margin-bottom: 16px; }
    .catalog-actions { display:flex;flex-wrap:wrap;gap:8px;justify-content:flex-end;margin:16px 0 }
    .empty-catalog,.routing-policy{padding:12px 0;color:var(--muted);line-height:1.6}
    .empty-catalog{text-align:center}
    .groups { display:grid;gap:0 }
    .endpoint-card { padding:16px 0; border-bottom:1px solid var(--border); min-width:0; }
    .endpoint-summary strong { min-width:0; overflow-wrap:anywhere; font-size:1rem; }
    .endpoint-summary > span { flex-shrink:0; font-size:.8125rem; }
    .endpoint-card,.model { container-type:inline-size }
    .model { border-top:1px solid var(--border);padding:20px 0 }
    .model .wide-actions button, .endpoint-head .wide-actions button { background:transparent; border-color:transparent; }
    .model-toolbar { border-top:1px solid var(--border);padding-top:14px }
    .model-toolbar h3 { font-size:1rem;margin:0;white-space:nowrap }
    .model-toolbar .actions { justify-content:flex-start }
    .inline-toggle { display:inline-flex;align-items:center;gap:8px;white-space:nowrap;font-size:.9rem }
    .inline-toggle input { width:auto;margin:0 }
    .capability-notice { color:var(--warning, #a66a13);font-size:.9rem }
    .identity{min-width:0;flex:1 1 190px;overflow-wrap:anywhere}
    .model .actions{flex:0 1 auto}
    .endpoint { overflow-wrap:anywhere }
    .compact-actions { display:none;position:relative;flex:none }
    .compact-actions summary,.discovered summary { cursor:pointer;border:1px solid var(--border);border-radius:8px;padding:7px 10px;white-space:nowrap;list-style:none }
    .compact-actions summary::-webkit-details-marker,.discovered summary::-webkit-details-marker { display:none }
    .compact-menu[popover],.discovered-list[popover] { position: fixed; margin: 0; right: auto; min-width: 0; box-sizing: border-box; }
    .compact-menu,.discovered-list { position:absolute;z-index:3;right:0;top:calc(100% + 4px);max-height:260px;min-width:220px;max-width:min(440px,90vw);overflow:auto;background:var(--canvas);border:1px solid var(--border);border-radius:10px;box-shadow:0 12px 30px #0003;padding:6px;display:grid;gap:2px }
    .compact-actions[open] > .compact-menu,.discovered[open] > .discovered-list { animation: menu-enter var(--motion-base) ease-out; }
    @keyframes menu-enter { from { opacity:0; transform:translateY(-3px); } to { opacity:1; transform:translateY(0); } }
    .compact-menu button,.discovered-list button { text-align:start;overflow-wrap:anywhere }
    .discovered{position:relative}
    @container (max-width:650px) { .wide-actions { display:none } .compact-actions { display:block } }
    textarea { font:inherit;color:inherit;background:var(--canvas);border:1px solid var(--border);border-radius:7px;padding:8px }
    dialog { max-width:760px; padding:32px; }
    dialog form > div:first-child h2 { font-size:1.5rem; overflow-wrap:anywhere; }
    dialog form > .actions { border-top:1px solid var(--border);padding-top:20px; }
    fieldset { border:0;border-top:1px solid var(--border);border-radius:0;padding:20px 0 0;display:grid;gap:16px;min-width:0 }
    legend { padding:0 12px 0 0;font-weight:600;font-size:1rem; }
    .efforts { display:flex;flex-wrap:wrap;gap:12px }
    .efforts label { display:flex;align-items:center;gap:6px }
    .efforts input { width:auto }
    .execution-capabilities { display:grid;gap:0; }
    .execution-capabilities label { justify-content:space-between;flex-direction:row-reverse;min-height:48px;padding:12px 0;border-bottom:1px solid var(--border); }
    @media (max-width:760px) { dialog { padding:20px 16px; } .endpoint-summary { align-items:start; } .endpoint-summary > span { flex:0 1 110px;text-align:end; } }
</style>
