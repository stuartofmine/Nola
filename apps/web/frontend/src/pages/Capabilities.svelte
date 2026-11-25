<script lang="ts">
    import { randomId } from '../lib/randomId';
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
    import { builtin, capabilitySnapshot, capabilityName, capabilityDraft, capabilityMutation, updatedCapability, capabilityCatalog, capabilitySections, restoredSkill, stdioMcp, CapabilityValidationError } from '../lib/capabilities';
    import { locale, t } from '../lib/i18n';
    import type { Capability, CapabilityMode, CapabilitySettings } from '../lib/capabilities';
    import HarnessPlugins from '../components/HarnessPlugins.svelte';
    const initialTab = pageLocation.read().searchParams.get('tab');
    let activeTab = $state<'skills' | 'mcp' | 'plugins'>(initialTab === 'plugins' ? 'plugins' : initialTab === 'mcp' ? 'mcp' : 'skills');
    const plugins = $derived(activeTab === 'plugins');
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    let confirmation = $state<ConfirmationHandle>(null!);
    let mode = $state<CapabilityMode>(initialTab === 'mcp' ? 'mcp' : 'skills');
    let settings = $state<CapabilitySettings>();
    let executionBackends = $state<Array<{id:string; display_name:string; capabilities:{supports_permission_bridge:boolean}}>>([]);
    let skillReview = $state<{ expected_revision: number; pending: boolean; previous: unknown; current: Capability[] }>();
    let reviewDialog = $state<HTMLDialogElement>(null!);
    let query = $state('');
    let source = $state('all');
    let busy = $state(false);
    let error = $state('');
    let errorDetail = $state('');
    let notice = $state('');
    let original = $state<Capability>();
    let selected = $state<number | null>(null);
    let draft = $state(capabilityDraft({ name: '' }));
    let dialog = $state<HTMLDialogElement>(null!);
    let retryButton = $state<HTMLButtonElement>();
    let read: AbortController | undefined;
    let reviewRead: AbortController | undefined;
    let reviewLoading = $state(false);
    let disposed = false;
    const builtInCategories = [
        'documents', 'data-analysis', 'design-presentations', 'communication',
        'planning-automation', 'knowledge-research', 'workspace',
    ] as const;
    function isBuiltIn(item: Capability): boolean {
        return builtin(item) || item.isBuiltIn === true || item.source === 'built-in';
    }
    const items = $derived(settings ? mode === 'skills' ? settings.skills : settings.mcp_servers : []);
    const filtered = $derived(items.map((item, index) => ({ item, index })).filter(({ item }) =>
        (source === 'all' || (source === 'builtin') === isBuiltIn(item)) &&
        `${capabilityName(item)} ${item.description || item.summary || ''} ${item.category_name || ''}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));
    const dirty = $derived(!!original && JSON.stringify(draft) !== JSON.stringify(capabilityDraft(original)));
    const sections = $derived.by(() => {
        if (mode === 'mcp') return capabilitySections(filtered, mode);
        const groups = [...builtInCategories, 'builtin', 'custom'] as const;
        return groups.map(id => ({
            id,
            titleKey: `capability.category.${id}`,
            entries: filtered.filter(({ item }) => {
                if (!isBuiltIn(item)) return id === 'custom';
                const category = builtInCategories.some(key => key === item.category_id) ? item.category_id : 'builtin';
                return category === id;
            }).sort((a, b) => Number(isBuiltIn(b.item)) - Number(isBuiltIn(a.item)) || capabilityName(a.item).localeCompare(capabilityName(b.item))),
        })).filter(group => group.entries.length > 0);
    });
    function displayName(item: Capability) { return capabilityName(item, t($locale, 'capability.unnamed')); }
    function showError(cause: unknown) {
        if (disposed) return;
        errorDetail = '';
        if (cause instanceof CapabilityValidationError) error = cause.key;
        else { error = 'capability.requestFailed'; errorDetail = errorMessage(cause); }
    }
    async function load() {
        if (busy) return;
        read?.abort(); const controller = new AbortController(); read = controller;
        busy = true;
        try {
            const result = capabilitySnapshot(await api<unknown>('/api/agent/settings', { signal: controller.signal }));
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
    onMount(() => { if (!plugins) void load(); return () => { disposed = true; read?.abort(); reviewRead?.abort(); }; });
    function edit(item?: Capability, index: number | null = null) {
        if (busy) return;
        try {
            const next = item ?? { name: t($locale, mode === 'skills' ? 'capability.newSkill' : 'capability.newMcp'), enabled: mode === 'skills', ...(mode === 'skills' ? { id: randomId(), category_id: 'custom' } : { server_id: randomId() }) };
            selected = index; original = next;
            draft = capabilityDraft(original); error = ''; notice = ''; dialog.showModal();
        } catch (cause) { showError(cause); }
    }
    function addLocalMcp() {
        try { edit({ server_id: randomId(), name: t($locale, 'capability.newMcp'), command: '', enabled: false }); }
        catch (cause) { showError(cause); }
    }
    async function openSkillReview() {
        if (disposed || reviewLoading) return;
        const controller = new AbortController(); reviewRead = controller;
        reviewLoading = true;
        try {
            const result = await api<typeof skillReview>('/api/agent/skill-settings-review', {signal: controller.signal});
            if (disposed || controller.signal.aborted) return;
            skillReview = result;
            await tick();
            if (!disposed && !controller.signal.aborted) reviewDialog.showModal();
        } catch (cause) { if (!controller.signal.aborted) showError(cause); }
        finally { if (!disposed) reviewLoading = false; }
    }
    async function confirmSkillReview() {
        if (!skillReview?.pending || busy) return;
        if (!await confirmation.ask({ title: t($locale, 'capability.reviewTitle'), description: t($locale, 'capability.reviewHint'), confirmLabel: t($locale, 'capability.useCore'), cancelLabel: t($locale, 'capability.cancel') })) return;
        if (disposed) return;
        busy = true;
        try {
            await api('/api/agent/skill-settings-review', { method: 'POST', body: JSON.stringify({ expected_revision: skillReview.expected_revision, use_core_settings: true }) });
            if (disposed) return;
            reviewDialog.close(); skillReview = undefined; error = ''; errorDetail = '';
        } catch (cause) { showError(cause); return; }
        finally { busy = false; }
        await load();
    }
    async function close(event?: Event) {
        event?.preventDefault(); if (busy) return;
        if (dirty && !await confirmation.ask({ title: t($locale, 'capability.discardTitle'), description: t($locale, 'capability.discardBody'), confirmLabel: t($locale, 'capability.discard'), cancelLabel: t($locale, 'capability.keepEditing'), destructive: true })) return;
        if (!disposed) { dialog.close(); original = undefined; }
    }
    async function persist(replacement?: Capability) {
        if (disposed || busy || !settings) return;
        busy = true; error = ''; notice = '';
        try {
            const result = capabilitySnapshot(await api<unknown>('/api/agent/settings', { method: 'POST', body: JSON.stringify(capabilityMutation(settings, mode, selected, replacement)) }));
            if (disposed) return;
            settings = result;
            executionBackends = result.execution_backends;
            dialog.close(); original = undefined; notice = replacement ? 'capability.saved' : 'capability.deleted';
        } catch (cause) {
            showError(cause instanceof CapabilityValidationError && cause.key === 'capability.loadFailed'
                ? new CapabilityValidationError('capability.requestFailed') : cause);
        } finally { busy = false; }
    }
    async function toggleInline(item: Capability, index: number, enabled: boolean) {
        if (disposed || busy || !settings) return;
        busy = true; error = ''; notice = '';
        try {
            const replacement = { ...item, enabled };
            const result = capabilitySnapshot(await api<unknown>('/api/agent/settings', { method: 'POST', body: JSON.stringify(capabilityMutation(settings, mode, index, replacement)) }));
            if (disposed) return;
            settings = result;
            executionBackends = result.execution_backends;
            notice = 'capability.saved';
        } catch (cause) {
            showError(cause instanceof CapabilityValidationError && cause.key === 'capability.loadFailed'
                ? new CapabilityValidationError('capability.requestFailed') : cause);
        } finally { busy = false; }
    }
    async function save(event: SubmitEvent) {
        event.preventDefault(); if (!original || busy) return;
        try { await persist(updatedCapability(original, draft, mode)); } catch (cause) { showError(cause); }
    }
    async function remove() {
        if (busy || !original || (mode === 'skills' && isBuiltIn(original))) return;
        if (!await confirmation.ask({ title: t($locale, 'capability.deleteTitle'), description: t($locale, 'capability.deleteBody'), confirmLabel: t($locale, 'capability.delete'), destructive: true })) return;
        if (disposed) return;
        if (selected === null) { dialog.close(); original = undefined; return; }
        await persist();
    }
    async function restore() {
        if (busy || !original || mode !== 'skills') return;
        if (!await confirmation.ask({ title: t($locale, 'capability.restoreTitle'), description: t($locale, 'capability.restoreBody'), confirmLabel: t($locale, 'capability.restore') })) return;
        if (disposed || !original) return;
        try { await persist(restoredSkill(original)); } catch (cause) { showError(cause); }
    }
    const allTabs = ['skills', 'mcp', 'plugins'] as const;
    function switchTab(next: typeof allTabs[number]) {
        if (busy) return;
        activeTab = next;
        if (next !== 'plugins') {
            mode = next; source = 'all'; query = ''; notice = '';
            if (settings) { error = ''; errorDetail = ''; }
            else void load();
        }
        const url = pageLocation.read(); url.searchParams.set('tab', next); pageLocation.replace(url);
    }
    function tabKey(event: KeyboardEvent) {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        const index = allTabs.indexOf(activeTab);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? allTabs.length - 1
            : event.key === 'ArrowRight' ? (index + 1) % allTabs.length : (index + allTabs.length - 1) % allTabs.length;
        switchTab(allTabs[next]);
        pageElements.element(`capability-${allTabs[next]}`)?.focus();
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return; if (dirty || busy && original) { event.preventDefault(); event.returnValue = ''; } }
</script>

<svelte:window onbeforeunload={beforeUnload} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, plugins ? 'plugin.title' : 'capability.title')} — Nola</title>{/if}</svelte:head>
<ConfirmationDialog bind:this={confirmation} />
<header class="page-header">
    <div><h1>{t($locale, 'capability.title')}</h1><p class="muted">{t($locale, 'capability.subtitle')}</p></div>
    <div class="actions">
        {#if !plugins}
            {#if mode === 'mcp'}<button type="button" disabled={busy || !settings} onclick={addLocalMcp}>{t($locale, 'capability.addLocalMcp')}</button>{/if}
            <button type="button" class="primary" disabled={busy || !settings} onclick={() => edit()}>{t($locale, mode === 'skills' ? 'capability.addSkill' : 'capability.addMcp')}</button>
        {/if}
    </div>
</header>
<div class="content management-content">
    <div class="tabs" role="tablist" aria-label={t($locale, 'capability.type')}>
        {#each allTabs as value}<button type="button" id="capability-{value}" role="tab" aria-selected={activeTab === value} aria-controls="capability-panel" tabindex={activeTab === value ? 0 : -1} onkeydown={tabKey} disabled={busy} onclick={() => switchTab(value)}>{t($locale, value === 'skills' ? 'capability.skills' : value === 'mcp' ? 'capability.mcp' : 'plugin.title')}</button>{/each}
    </div>
    {#if plugins}
    <div id="capability-panel" role="tabpanel" aria-labelledby="capability-plugins"><HarnessPlugins /></div>
    {:else}
    <aside aria-label={t($locale, 'execution.scope.host')}>
        <p class="muted">{t($locale, 'execution.scope.host')}</p><p class="muted">{t($locale, 'execution.scope.child')}</p>
        {#each executionBackends.filter(backend=>!backend.capabilities.supports_permission_bridge) as backend}
            <p class="muted">{backend.display_name}: {t($locale, 'execution.scope.noApprovalBridge')}</p>
        {/each}
    </aside>
    <div class="retained-settings">{#if skillReview?.pending || settings?.skill_settings_backup_key}<button type="button" disabled={busy || reviewLoading} aria-busy={reviewLoading} onclick={openSkillReview}>{t($locale, 'capability.reviewTitle')}</button>{/if}</div>
    {#if skillReview?.pending}<p class="notice" role="status">{t($locale, 'capability.reviewHint')}</p>{/if}

    <div class="fields"><label>{t($locale, 'capability.search')}<input type="search" bind:value={query} placeholder={t($locale, 'capability.searchHint')} /></label><label>{t($locale, 'capability.source')}<select bind:value={source}><option value="all">{t($locale, 'capability.all')}</option><option value="builtin">{t($locale, 'capability.builtin')}</option><option value="custom">{t($locale, 'capability.custom')}</option></select></label></div>
    {#if error && !original}<div class="notice error" role="alert">{t($locale, error, { detail: errorDetail })} <button type="button" bind:this={retryButton} onclick={load} aria-disabled={busy}>{t($locale, 'capability.reload')}</button></div>{/if}
    {#if notice}<p role="status">{t($locale, notice)}</p>{/if}
    <div id="capability-panel" role="tabpanel" aria-labelledby="capability-{mode}" aria-busy={busy}>
        {#if !settings && busy}<p role="status">{t($locale, 'capability.loading')}</p>{:else if settings}
        {#each sections as section (section.id)}
        <section class="category" aria-label={t($locale, section.titleKey)}><h2>{t($locale, section.titleKey)} <span class="muted">{section.entries.length.toLocaleString($locale)}</span></h2>
        <div class="gallery">{#each section.entries as { item, index }}
            <div class="card capability">
                <div class="capability-row">
                    <button type="button" class="capability-open" onclick={() => edit(item, index)} disabled={busy}>
                        <span class="capability-title-row">
                            <strong>{displayName(item)}</strong>
                            <span class="capability-badge" data-builtin={isBuiltIn(item) || undefined}>{t($locale, isBuiltIn(item) ? 'capability.builtin' : 'capability.custom')}</span>
                        </span>
                        <span class="summary muted">{String(item.description || item.summary || t($locale, 'capability.noDescription'))}</span>
                        {#if item.category_name}<span class="capability-category muted">{item.category_name}</span>{/if}
                    </button>
                    <label class="capability-switch">
                        <input type="checkbox" role="switch" aria-label={displayName(item)} checked={item.enabled !== false} disabled={busy} onchange={event => {
                            const next = event.currentTarget.checked;
                            event.currentTarget.checked = item.enabled !== false;
                            void toggleInline(item, index, next);
                        }} />
                    </label>
                </div>
            </div>
        {/each}</div></section>
        {:else}<div class="card"><h2>{t($locale, query || source !== 'all' ? 'capability.noMatches' : 'capability.empty')}</h2><p class="muted">{t($locale, query || source !== 'all' ? 'capability.noMatchesHint' : 'capability.emptyHint')}</p></div>{/each}
        {/if}
    </div>
    {/if}
</div>
{#if !plugins}
<dialog use:ownDialog bind:this={dialog} oncancel={close} onclick={event => { if (event.target === dialog && !busy) void close(); }} aria-label={t($locale, mode === 'skills' ? 'capability.skillSettings' : 'capability.mcpSettings')}>
    {#if original}<form onsubmit={save} aria-busy={busy}>
        <h2>{displayName(original)}</h2>
        {#if error}<p class="error" role="alert">{t($locale, error, { detail: errorDetail })}</p>{/if}
        <fieldset disabled={busy}>
            <label class="row"><input class="check" type="checkbox" role="switch" bind:checked={draft.enabled} />{t($locale, 'capability.enable')}</label>
            <p class="muted">{t($locale, 'capability.permissionHint')}</p>
            <label>{t($locale, 'capability.name')}<input bind:value={draft.name} readonly={mode === 'skills' && isBuiltIn(original)} required /></label>
            <label>{t($locale, 'capability.description')}<textarea bind:value={draft.description} required={mode === 'skills'} rows="3" readonly={mode === 'skills' && isBuiltIn(original)}></textarea></label>
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
            {#if mode === 'skills' && isBuiltIn(original) && typeof original.default_content === 'string'}<button type="button" disabled={busy || (!dirty && original.content === original.default_content)} onclick={restore}>{t($locale, 'capability.restore')}</button>{/if}
            {#if selected !== null && (mode !== 'skills' || !isBuiltIn(original))}<button type="button" class="danger" disabled={busy} onclick={remove}>{t($locale, 'capability.delete')}</button>{/if}
            <button type="button" disabled={busy} onclick={() => close()}>{t($locale, 'capability.cancel')}</button><button class="primary" disabled={busy || (selected !== null && !dirty)}>{t($locale, busy ? 'capability.saving' : 'capability.save')}</button>
        </div>
    </form>{/if}
</dialog>
<dialog use:ownDialog bind:this={reviewDialog} oncancel={event => { if (busy) event.preventDefault(); }} onclick={event => { if (event.target === reviewDialog && !busy) reviewDialog.close(); }} aria-label={t($locale, 'capability.reviewTitle')}>
    {#if skillReview}
        <h2>{t($locale, 'capability.reviewTitle')}</h2>
        {#if skillReview.pending}<p>{t($locale, 'capability.reviewHint')}</p>{/if}
        {#if error}<p class="error" role="alert">{t($locale, error, { detail: errorDetail })}</p>{/if}
        <h3>{t($locale, 'capability.previousSettings')}</h3>
        {#if Array.isArray(skillReview.previous)}
            {#each skillReview.previous as prev}<p>{typeof prev === 'object' && prev !== null ? displayName(prev as Capability) : String(prev)}{#if typeof prev === 'object' && prev !== null && 'enabled' in prev} · {t($locale, (prev as { enabled?: boolean }).enabled === false ? 'capability.disabled' : 'capability.enabled')}{/if}</p>{/each}
        {:else if skillReview.previous && typeof skillReview.previous === 'object'}
            {#each Object.entries(skillReview.previous as Record<string, unknown>) as [key, value]}
                <p>{key}{#if typeof value === 'boolean'} · {t($locale, value ? 'capability.enabled' : 'capability.disabled')}{:else if value && typeof value === 'object' && 'enabled' in value} · {t($locale, (value as { enabled?: boolean }).enabled === false ? 'capability.disabled' : 'capability.enabled')}{/if}</p>
            {/each}
        {:else}
            <p class="muted">{t($locale, 'capability.empty')}</p>
        {/if}
        <details><summary>{t($locale, 'capability.coreSettings')}</summary>
            {#each skillReview.current as item}<p>{displayName(item)} · {t($locale, item.enabled === false ? 'capability.disabled' : 'capability.enabled')}</p>{/each}
        </details>
        <div class="actions"><button disabled={busy} onclick={() => reviewDialog.close()}>{t($locale, 'capability.cancel')}</button>{#if skillReview.pending}<button class="primary" disabled={busy} onclick={confirmSkillReview}>{t($locale, 'capability.useCore')}</button>{/if}</div>
    {/if}
</dialog>
{/if}
<style>
    .retained-settings:has(button) { margin-bottom: 16px; }
    .notice button[aria-disabled="true"] { color: var(--muted); background: var(--panel); border-color: var(--border); cursor: progress; }
    .category { display: grid; gap: 12px; margin-bottom: 24px; }
    .gallery { display: grid; grid-template-columns: minmax(0, 1fr); gap: 0; border-bottom: 1px solid var(--border); }
    .capability { text-align: start; min-width: 0; overflow-wrap: anywhere; background: transparent; border: 0; border-top: 1px solid var(--border); border-radius: 0; padding: 16px 8px; gap: 6px; }
    .capability-row { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
    .capability-open { display: grid; gap: 6px; flex: 1; min-width: 0; text-align: start; border: 0; background: transparent; box-shadow: none; padding: 2px 0; min-height: 0; }
    .capability-open:hover:not(:disabled) { background: transparent; border-color: transparent; }
    .capability-title-row { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
    .capability-badge { display: inline-flex; align-items: center; padding: 1px 8px; border-radius: 999px; border: 1px solid var(--border); background: var(--panel); color: var(--muted); font-size: .75rem; font-weight: 500; line-height: 1.5; }
    .capability-badge[data-builtin] { background: var(--accent-subtle); color: var(--content); }
    .capability-category { font-size: .8125rem; }
    .capability-switch { display: inline-flex; align-items: center; flex: none; padding: 0; min-height: 36px; }
    .capability .summary { font-size: var(--text-body); }
    .capability strong { font-size: 1rem; font-weight: 600; }
    .summary { display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
    fieldset { border: 0; padding: 0; margin: 0; min-width: 0; display: grid; gap: 12px; }
    textarea { width: 100%; resize: vertical; min-height: 72px; font: inherit; color: inherit; background: var(--input-bg); border: 1px solid var(--input-border); border-radius: var(--radius-control); padding: 12px; }
    @media (max-width: 560px) { .capability-row { gap: 8px; } }
    .instructions { font-family: ui-monospace, monospace; } .check { width: auto; }
    details { border-bottom: 1px solid var(--border); padding: 10px 0; overflow-wrap: anywhere; } summary { cursor: pointer; }
</style>
