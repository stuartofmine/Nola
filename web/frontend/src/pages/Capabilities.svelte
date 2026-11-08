<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceElements } from '../lib/workspacePageLocation';
    const pageElements = workspaceElements();
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    import { onMount, tick } from 'svelte';
    import { api as unscopedApi, ApiError, errorMessage } from '../lib/api';
    import { builtin, capabilityName, capabilityDraft, capabilityMutation, updatedCapability, capabilityCatalog, capabilitySections, restoredSkill, stdioMcp, CapabilityValidationError } from '../lib/capabilities';
    import { locale, t } from '../lib/i18n';
    import type { Capability, CapabilityMode, CapabilitySettings } from '../lib/capabilities';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    let confirmation: ConfirmationHandle;
    let mode = $state<CapabilityMode>(pageLocation.read().searchParams.get('tab') === 'mcp' ? 'mcp' : 'skills');
    let settings = $state<CapabilitySettings>();
    let executionBackends = $state<Array<{id:string; display_name:string; capabilities:{supports_permission_bridge:boolean}}>>([]);
    let skillReview = $state<{ expected_revision: number; pending: boolean; previous: unknown; current: Capability[] }>();
    let reviewDialog: HTMLDialogElement;
    let query = $state('');
    let source = $state('all');
    let busy = $state(false);
    let error = $state('');
    let errorDetail = $state('');
    let notice = $state('');
    let original = $state<Capability>();
    let selected = $state<number | null>(null);
    let draft = $state(capabilityDraft({ name: '' }));
    let dialog: HTMLDialogElement;
    let retryButton = $state<HTMLButtonElement>();
    let read: AbortController | undefined;
    const items = $derived(settings ? mode === 'skills' ? settings.skills : settings.mcp_servers : []);
    const filtered = $derived(items.map((item, index) => ({ item, index })).filter(({ item }) =>
        (source === 'all' || (source === 'builtin') === builtin(item)) &&
        `${capabilityName(item)} ${item.description || item.summary || ''} ${item.category_name || ''}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));
    const dirty = $derived(!!original && JSON.stringify(draft) !== JSON.stringify(capabilityDraft(original)));
    const sections = $derived(capabilitySections(filtered, mode));
    function displayName(item: Capability) { return capabilityName(item, t($locale, 'capability.unnamed')); }
    function showError(cause: unknown) {
        errorDetail = '';
        if (cause instanceof CapabilityValidationError) error = cause.key;
        else { error = 'capability.requestFailed'; errorDetail = errorMessage(cause); }
    }
    async function load() {
        if (busy) return;
        read?.abort(); const controller = new AbortController(); read = controller;
        busy = true;
        try {
            const result = await api<CapabilitySettings & {execution_backends: typeof executionBackends}>('/api/agent/settings', { signal: controller.signal });
            if (!controller.signal.aborted) {
                const restoreResultFocus = !!retryButton && document.activeElement === retryButton;
                executionBackends = result.execution_backends;
                settings = { skills: result.skills ?? [], mcp_servers: result.mcp_servers ?? [], skill_settings_backup_key: result.skill_settings_backup_key };
                error = ''; errorDetail = '';
                await tick();
                if (restoreResultFocus && !controller.signal.aborted && document.activeElement === document.body) pageElements.element('capability-panel')?.focus({ preventScroll: true });
            }
        } catch (cause) {
            if (!controller.signal.aborted) {
                if (cause instanceof ApiError && cause.code === 'skill_settings_review_required') { error = ''; errorDetail = ''; await openSkillReview(); }
                else {
                    showError(cause);
                    if (error === 'capability.requestFailed') error = 'capability.loadFailed';
                }
            }
        }
        finally { busy = false; }
    }
    onMount(() => { void load(); return () => read?.abort(); });
    function edit(item?: Capability, index: number | null = null) {
        if (busy) return;
        selected = index; original = item ?? { name: t($locale, mode === 'skills' ? 'capability.newSkill' : 'capability.newMcp'), enabled: mode === 'skills', ...(mode === 'skills' ? { id: crypto.randomUUID(), category_id: 'custom' } : { server_id: crypto.randomUUID() }) };
        draft = capabilityDraft(original); error = ''; notice = ''; dialog.showModal();
    }
    function addLocalMcp() {
        edit({ server_id: crypto.randomUUID(), name: t($locale, 'capability.newMcp'), command: '', enabled: false });
    }
    async function openSkillReview() {
        try {
            skillReview = await api('/api/agent/skill-settings-review');
            await tick(); reviewDialog.showModal();
        } catch (cause) { showError(cause); }
    }
    async function confirmSkillReview() {
        if (!skillReview?.pending || busy) return;
        if (!await confirmation.ask({ title: t($locale, 'capability.reviewTitle'), description: t($locale, 'capability.reviewHint'), confirmLabel: t($locale, 'capability.useCore'), cancelLabel: t($locale, 'capability.cancel') })) return;
        busy = true;
        try {
            await api('/api/agent/skill-settings-review', { method: 'POST', body: JSON.stringify({ expected_revision: skillReview.expected_revision, use_core_settings: true }) });
            reviewDialog.close(); skillReview = undefined; error = ''; errorDetail = '';
        } catch (cause) { showError(cause); return; }
        finally { busy = false; }
        await load();
    }
    async function close(event?: Event) {
        event?.preventDefault(); if (busy) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'capability.discardTitle'), description: t($locale, 'capability.discardBody'), confirmLabel: t($locale, 'capability.discard'), cancelLabel: t($locale, 'capability.keepEditing'), destructive: true })) return;
        dialog.close(); original = undefined;
    }
    async function persist(replacement?: Capability) {
        if (busy || !settings) return;
        busy = true; error = ''; notice = '';
        try {
            const result = await api<CapabilitySettings>('/api/agent/settings', { method: 'POST', body: JSON.stringify(capabilityMutation(settings, mode, selected, replacement)) });
            settings = { skills: result.skills ?? settings.skills, mcp_servers: result.mcp_servers ?? settings.mcp_servers, skill_settings_backup_key: result.skill_settings_backup_key ?? settings.skill_settings_backup_key };
            dialog.close(); original = undefined; notice = replacement ? 'capability.saved' : 'capability.deleted';
        } catch (cause) {
            showError(cause);
        } finally { busy = false; }
    }
    async function save(event: SubmitEvent) {
        event.preventDefault(); if (!original || busy) return;
        try { await persist(updatedCapability(original, draft, mode)); } catch (cause) { showError(cause); }
    }
    async function remove() {
        if (busy || !original || (mode === 'skills' && builtin(original))) return;
        if (!await confirmation.ask({ title: t($locale, 'capability.deleteTitle'), description: t($locale, 'capability.deleteBody'), confirmLabel: t($locale, 'capability.delete'), destructive: true })) return;
        if (selected === null) { dialog.close(); original = undefined; return; }
        await persist();
    }
    async function restore() {
        if (busy || !original || mode !== 'skills') return;
        if (!await confirmation.ask({ title: t($locale, 'capability.restoreTitle'), description: t($locale, 'capability.restoreBody'), confirmLabel: t($locale, 'capability.restore') })) return;
        try { await persist(restoredSkill(original)); } catch (cause) { showError(cause); }
    }
    function switchMode(next: CapabilityMode) {
        if (busy) return;
        mode = next; source = 'all'; query = ''; notice = '';
        if (settings) { error = ''; errorDetail = ''; }
        const url = pageLocation.read(); url.searchParams.set('tab', mode); pageLocation.replace(url);
    }
    function tabKey(event: KeyboardEvent) {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault(); switchMode(event.key === 'Home' ? 'skills' : event.key === 'End' ? 'mcp' : mode === 'skills' ? 'mcp' : 'skills');
        pageElements.element(`capability-${mode}`)?.focus();
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return; if (dirty || busy && original) { event.preventDefault(); event.returnValue = ''; } }
</script>

<svelte:window onbeforeunload={beforeUnload} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'capability.title')} — Nola</title>{/if}</svelte:head>
<ConfirmationDialog bind:this={confirmation} />
<header class="page-header"><div><h1>{t($locale, 'capability.title')}</h1><p class="muted">{t($locale, 'capability.subtitle')}</p></div><div class="actions">{#if mode === 'mcp'}<button disabled={busy || !settings} onclick={addLocalMcp}>{t($locale, 'capability.addLocalMcp')}</button>{/if}<button class="primary" disabled={busy || !settings} onclick={() => edit()}>{t($locale, mode === 'skills' ? 'capability.addSkill' : 'capability.addMcp')}</button></div></header>
<div class="content management-content">
    <aside aria-label={t($locale, 'execution.scope.host')}>
        <p class="muted">{t($locale, 'execution.scope.host')}</p><p class="muted">{t($locale, 'execution.scope.child')}</p>
        {#each executionBackends.filter(backend=>!backend.capabilities.supports_permission_bridge) as backend}
            <p class="muted">{backend.display_name}: {t($locale, 'execution.scope.noApprovalBridge')}</p>
        {/each}
    </aside>
    <div class="retained-settings">{#if skillReview?.pending || settings?.skill_settings_backup_key}<button disabled={busy} onclick={openSkillReview}>{t($locale, 'capability.reviewTitle')}</button>{/if}</div>
    {#if skillReview?.pending}<p class="notice" role="status">{t($locale, 'capability.reviewHint')}</p>{/if}
    <div class="tabs" role="tablist" aria-label={t($locale, 'capability.type')}>
        {#each ['skills', 'mcp'] as value}<button id="capability-{value}" role="tab" aria-selected={mode === value} aria-controls="capability-panel" tabindex={mode === value ? 0 : -1} onkeydown={tabKey} disabled={busy} onclick={() => switchMode(value as CapabilityMode)}>{t($locale, value === 'skills' ? 'capability.skills' : 'capability.mcp')}</button>{/each}
    </div>
    <div class="fields"><label>{t($locale, 'capability.search')}<input type="search" bind:value={query} placeholder={t($locale, 'capability.searchHint')} /></label><label>{t($locale, 'capability.source')}<select bind:value={source}><option value="all">{t($locale, 'capability.all')}</option><option value="builtin">{t($locale, 'capability.builtin')}</option><option value="custom">{t($locale, 'capability.custom')}</option></select></label></div>
    {#if error && !original}<div class="notice error" role="alert">{t($locale, error, { detail: errorDetail })} <button bind:this={retryButton} onclick={load} aria-disabled={busy}>{t($locale, 'capability.reload')}</button></div>{/if}
    {#if notice}<p role="status">{t($locale, notice)}</p>{/if}
    <div id="capability-panel" role="tabpanel" aria-labelledby="capability-{mode}" aria-busy={busy} tabindex="0">
        {#if !settings && busy}<p role="status">{t($locale, 'capability.loading')}</p>{:else if settings}
        {#each sections as section (section.id)}
        <section class="category" aria-label={t($locale, section.titleKey)}><h2>{t($locale, section.titleKey)} <span class="muted">{section.entries.length.toLocaleString($locale)}</span></h2>
        <div class="gallery">{#each section.entries as { item, index }}
            <button class="card capability" onclick={() => edit(item, index)} disabled={busy}>
                <span class="row between"><strong>{displayName(item)}</strong><span class="muted">{t($locale, item.enabled === false ? 'capability.disabled' : 'capability.enabled')}</span></span>
                <span class="summary muted">{String(item.description || item.summary || t($locale, 'capability.noDescription'))}</span>
                <span class="muted">{t($locale, builtin(item) ? 'capability.builtin' : 'capability.custom')}{item.category_name ? ` · ${item.category_name}` : ''}</span>
            </button>
        {/each}</div></section>
        {:else}<div class="card"><h2>{t($locale, query || source !== 'all' ? 'capability.noMatches' : 'capability.empty')}</h2><p class="muted">{t($locale, query || source !== 'all' ? 'capability.noMatchesHint' : 'capability.emptyHint')}</p></div>{/each}
        {/if}
    </div>
</div>
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-label={t($locale, mode === 'skills' ? 'capability.skillSettings' : 'capability.mcpSettings')}>
    {#if original}<form onsubmit={save} aria-busy={busy}>
        <h2>{displayName(original)}</h2>
        {#if error}<p class="error" role="alert">{t($locale, error, { detail: errorDetail })}</p>{/if}
        <fieldset disabled={busy}>
            <label class="row"><input class="check" type="checkbox" bind:checked={draft.enabled} />{t($locale, 'capability.enable')}</label>
            <p class="muted">{t($locale, 'capability.permissionHint')}</p>
            <label>{t($locale, 'capability.name')}<input bind:value={draft.name} readonly={mode === 'skills' && builtin(original)} required /></label>
            <label>{t($locale, 'capability.description')}<textarea bind:value={draft.description} required={mode === 'skills'} rows="3" readonly={mode === 'skills' && builtin(original)}></textarea></label>
            {#if mode === 'skills'}<label>{t($locale, 'capability.content')}<textarea class="instructions" required bind:value={draft.primary} rows="12"></textarea></label><small>{t($locale, 'capability.characterCount', { count: draft.primary.length.toLocaleString($locale) })}</small>
            {:else if stdioMcp(original)}<label>{t($locale, 'capability.launch')}<textarea class="instructions" bind:value={draft.primary} required rows="7"></textarea></label>
            {:else}<label>{t($locale, 'capability.endpoint')}<input type="url" bind:value={draft.primary} required /></label><p class="muted">{t($locale, 'capability.discoveryHint')}</p>{/if}
        </fieldset>
        {#if mode === 'mcp'}
            <section><h2>{t($locale, 'capability.catalogTitle')}</h2><p class="muted">{original.protocol_version ? `MCP ${original.protocol_version}` : t($locale, 'capability.notDiscovered')}</p>
                {#each capabilityCatalog(original) as group}<details open={group.entries.length > 0 && group.entries.length <= 6}><summary>{t($locale, group.titleKey)} · {group.entries.length.toLocaleString($locale)}</summary>{#each group.entries as entry}<p title={entry.description}>{entry.name}</p>{/each}</details>{/each}
            </section>
        {/if}
        <div class="actions">
            {#if mode === 'skills' && builtin(original) && typeof original.default_content === 'string'}<button type="button" disabled={busy || (!dirty && original.content === original.default_content)} onclick={restore}>{t($locale, 'capability.restore')}</button>{/if}
            {#if selected !== null && (mode !== 'skills' || !builtin(original))}<button type="button" class="danger" disabled={busy} onclick={remove}>{t($locale, 'capability.delete')}</button>{/if}
            <button type="button" disabled={busy} onclick={() => close()}>{t($locale, 'capability.cancel')}</button><button class="primary" disabled={busy || (selected !== null && !dirty)}>{t($locale, busy ? 'capability.saving' : 'capability.save')}</button>
        </div>
    </form>{/if}
</dialog>
<dialog use:ownDialog bind:this={reviewDialog} aria-label={t($locale, 'capability.reviewTitle')}>
    {#if skillReview}
        <h2>{t($locale, 'capability.reviewTitle')}</h2>
        {#if skillReview.pending}<p>{t($locale, 'capability.reviewHint')}</p>{/if}
        {#if error}<p class="error" role="alert">{t($locale, error, { detail: errorDetail })}</p>{/if}
        <h3>{t($locale, 'capability.previousSettings')}</h3><pre>{JSON.stringify(skillReview.previous, null, 2)}</pre>
        <details><summary>{t($locale, 'capability.coreSettings')}</summary>
            {#each skillReview.current as item}<p>{displayName(item)} · {t($locale, item.enabled === false ? 'capability.disabled' : 'capability.enabled')}</p>{/each}
        </details>
        <div class="actions"><button disabled={busy} onclick={() => reviewDialog.close()}>{t($locale, 'capability.cancel')}</button>{#if skillReview.pending}<button class="primary" disabled={busy} onclick={confirmSkillReview}>{t($locale, 'capability.useCore')}</button>{/if}</div>
    {/if}
</dialog>
<style>
    .retained-settings:has(button) { margin-bottom: 16px; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 18rem; overflow: auto; }
    .notice button[aria-disabled="true"] { color: var(--muted); background: var(--panel); border-color: var(--border); cursor: progress; }
    .category { display: grid; gap: 12px; margin-bottom: 24px; }
    .gallery { display: grid; grid-template-columns: minmax(0, 1fr); gap: 0; border-bottom: 1px solid var(--border); }
    .capability { text-align: start; min-width: 0; overflow-wrap: anywhere; background: transparent; border: 0; border-top: 1px solid var(--border); border-radius: 0; padding: 20px 8px; gap: 8px; }
    .capability .summary { font-size: var(--text-body); }
    .capability strong { font-size: 1rem; font-weight: 600; }
    .capability > span:last-child { font-size: .8125rem; }
    .summary { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
    fieldset { border: 0; padding: 0; margin: 0; min-width: 0; display: grid; gap: 12px; }
    textarea { width: 100%; resize: vertical; min-height: 72px; font: inherit; color: inherit; background: var(--surface); border: 1px solid var(--outline); border-radius: var(--radius-control); padding: 12px; }
    @media (max-width: 560px) { .capability > .row { display: grid; gap: 4px; } .capability > .row > .muted { font-size: var(--text-detail); } }
    .instructions { font-family: ui-monospace, monospace; } .check { width: auto; }
    details { border-bottom: 1px solid var(--border); padding: 10px 0; overflow-wrap: anywhere; } summary { cursor: pointer; }
</style>
