<script lang="ts">
    import {onMount} from 'svelte';
    import {api} from '../lib/api';
    import {locale, t} from '../lib/i18n';
    import {loadModelCatalog} from '../lib/modelCatalog';
    import type {RemoteProfile} from '../lib/modelProfiles';
    import type {AgentDirectory, PersistentAgent} from '../lib/agentRooms';
    import {editedExpertDefaults, confirmsExpertWrite} from '../lib/expertDefinition';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    let {hostId, onclose}: {hostId: string; onclose: () => void} = $props();
    let dialog: HTMLDialogElement, confirmation: ConfirmationDialog;
    let directory = $state<AgentDirectory>({revision: null, agents: []});
    let profiles = $state<RemoteProfile[]>([]);
    let templates = $state<{id: string; name: string}[]>([]);
    let editing = $state<PersistentAgent | null>(null);
    let attempted = $state<PersistentAgent | null>(null);
    let name = $state(''), description = $state(''), model = $state(''), template = $state(''), enabled = $state(true);
    let baseline = $state(''), loading = $state(false), saving = $state(false), uncertain = $state(false), error = $state(false);
    let loaded = $state(false);
    let alive = true;
    const lifetime = new AbortController();
    const draft = $derived(JSON.stringify([name, description, model, template, enabled]));
    const dirty = $derived(baseline !== '' && draft !== baseline);
    const command = <T,>(value: unknown) => api<T>('/api/agent-rooms', {method: 'POST', body: JSON.stringify(value), signal: lifetime.signal});
    function portal(node: HTMLDialogElement) {
        document.body.appendChild(node);
        return {destroy() {node.close(); node.remove();}};
    }
    function reset(agent: PersistentAgent | null = null) {
        editing = agent; name = agent?.name ?? ''; description = agent?.description ?? '';
        model = typeof agent?.defaults.modelSelectionID === 'string' ? agent.defaults.modelSelectionID : '';
        template = typeof agent?.defaults.selectedPromptTemplateID === 'string' ? agent.defaults.selectedPromptTemplateID : '';
        enabled = !agent || agent.availability === 'active';
        baseline = JSON.stringify([name, description, model, template, enabled]);
    }
    async function discard() {
        return !dirty || await confirmation.ask({title: t($locale, 'rooms.manageExperts'), description: t($locale, 'shell.navigationDiscard'),
            confirmLabel: t($locale, 'shell.confirm'), destructive: true});
    }
    async function close(event?: Event) {
        event?.preventDefault(); if (saving || !await discard() || !alive) return;
        dialog.close(); onclose();
    }
    async function select(agent: PersistentAgent | null) {if (!saving && !uncertain && await discard() && alive) reset(agent);}
    async function refresh() {
        if (saving || loading) return;
        loading = true; loaded = false;
        try {
            const session = await api<{authenticated: boolean; hostId: string}>('/api/web/session', {signal: lifetime.signal});
            if (!session.authenticated || session.hostId !== hostId) throw new Error('Host changed');
            const [next, catalog, promptCatalog] = await Promise.all([
                command<AgentDirectory>({type: 'directory'}),
                loadModelCatalog(<T,>(path: string, options: RequestInit = {}) => api<T>(path, {...options, signal: lifetime.signal})),
                api<{revision: number | null; templates: {id: string; name: string}[]}>('/api/expert-prompt-templates', {signal: lifetime.signal}),
            ]);
            if (!alive) return;
            directory = next; error = false; loaded = true;
            profiles = catalog.profiles.filter(p => p.enabled && p.endpoint.enabled && p.capabilities.includes('chat'));
            templates = promptCatalog.templates;
            // An unconfirmed write is reconciled by reading the exact retained ID,
            // never repeated with a new ID after a transport failure.
            if (uncertain && attempted) {
                const committed = next.agents.find(agent => agent.id === attempted!.id);
                if (committed && confirmsExpertWrite(committed, attempted)) {
                    reset(committed); uncertain = false; attempted = null;
                } else error = true;
            }
        } catch {if (alive) error = true;}
        finally {if (alive) loading = false;}
    }
    async function save() {
        if (saving || loading || !loaded || uncertain || !name.trim() || !editing && model !== '' && !profiles.some(p => `configured:${p.id}` === model)) return;
        const current = editing;
        const id = current?.id ?? crypto.randomUUID();
        const defaults = editedExpertDefaults(current, model, template);
        const input = {id, expected_directory_revision: directory.revision, expected_agent_revision: current?.revision ?? null,
            name: name.trim(), description, defaults, availability: enabled ? 'active' : 'archived', updated_at_ms: Date.now()};
        saving = true; error = false;
        try {
            const next = await command<AgentDirectory>({type: 'configure_agent', input});
            if (alive) {directory = next; reset(next.agents.find(agent => agent.id === id)!);}
        } catch {
            if (alive) {
                // Preserve the attempted identity and draft until reconciliation.
                attempted = {id, revision: current?.revision ?? 0, name: input.name, description,
                    defaults, availability: input.availability};
                uncertain = true; error = true;
            }
        } finally {if (alive) saving = false;}
    }
    onMount(() => {
        reset(); dialog.showModal();
        void refresh();
        return () => {alive = false; lifetime.abort();};
    });
</script>
<svelte:window onbeforeunload={event => {if (dirty || saving || uncertain) {event.preventDefault(); event.returnValue = '';}}} />
<ConfirmationDialog bind:this={confirmation} />
<dialog use:portal bind:this={dialog} aria-label={t($locale, 'rooms.manageExperts')} oncancel={close}>
    <header><h2>{t($locale, 'rooms.manageExperts')}</h2><button disabled={saving} onclick={() => close()}>{t($locale, 'rooms.close')}</button></header>
    {#if loading}<p role="status">{t($locale, 'rooms.loading')}</p>{/if}
    {#if error}<p role="alert">{t($locale, 'rooms.failed')}</p><button disabled={saving} onclick={refresh}>{t($locale, 'rooms.refresh')}</button>{/if}
    <nav aria-label={t($locale, 'rooms.experts')}>
        <button disabled={saving || uncertain} aria-pressed={!editing} onclick={() => select(null)}>{t($locale, 'rooms.createAgent')}</button>
        {#each directory.agents as agent (agent.id)}<button disabled={saving || uncertain} aria-pressed={editing?.id === agent.id} onclick={() => select(agent)}>{agent.name}{agent.availability === 'active' ? '' : ` · ${t($locale, 'rooms.inactive')}`}</button>{/each}
    </nav>
    <form onsubmit={event => {event.preventDefault(); void save();}} aria-busy={saving}>
        <fieldset disabled={loading || !loaded || saving || uncertain}>
            <label>{t($locale, 'rooms.agentName')}<input bind:value={name} maxlength="200" required /></label>
            <label>{t($locale, 'rooms.agentDescription')}<textarea bind:value={description} rows="3"></textarea></label>
            <label>{t($locale, 'rooms.model')}<select bind:value={model}>
                <option value="">{t($locale, 'rooms.useConversationModel')}</option>
                {#if model && !profiles.some(p => `configured:${p.id}` === model)}<option value={model}>{model}</option>{/if}
                {#each profiles as profile (profile.id)}<option value={`configured:${profile.id}`}>{profile.displayName}</option>{/each}
            </select></label>
            <label>{t($locale, 'rooms.promptTemplate')}<select bind:value={template}>
                <option value="">{t($locale, 'rooms.defaultPromptTemplate')}</option>
                {#if template && !templates.some(p => p.id === template)}<option value={template}>{template}</option>{/if}
                {#each templates as item (item.id)}<option value={item.id}>{item.name}</option>{/each}
            </select></label>
            <label class="enabled"><input type="checkbox" bind:checked={enabled} />{t($locale, 'rooms.enabled')}</label>
            <button type="submit" disabled={!name.trim() || !dirty}>{t($locale, 'rooms.saveAgent')}</button>
        </fieldset>
    </form>
</dialog>
<style>
    dialog {width: min(560px, calc(100vw - 24px)); max-height: calc(100dvh - 24px); overflow: auto;}
    header {display: flex; justify-content: space-between; align-items: center; gap: 16px;} h2 {margin: 0;}
    nav {display: flex; flex-wrap: wrap; gap: 6px; margin: 16px 0;} nav button[aria-pressed="true"] {border-color: transparent; background: var(--accent-subtle);}
    fieldset, label {display: grid; gap: 8px;} fieldset {padding: 0; border: 0; gap: 16px;} .enabled {display: flex; align-items: center;}
    input:not([type="checkbox"]), select, textarea {width: 100%; min-width: 0; box-sizing: border-box;} button {min-height: 44px;} [role="alert"] {color: var(--danger);}
</style>
