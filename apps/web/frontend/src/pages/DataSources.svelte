<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    import { onMount, tick } from 'svelte';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { latestRequest } from '../lib/latestRequest';
    import { resultTime } from '../lib/results';
    import { conversationHref } from '../lib/conversations';
    import { DataSourceError, newSourceId, parseSources, parseUsage, sourceDraft, sourcePayload, sourceReceipt, deletionReceipt, sourceState, disclosureKey, replaceSource, sourcePolicies, sourceKinds } from '../lib/dataSources';
    import type { DataSource, SourceUsage } from '../lib/dataSources';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import Automations from './Automations.svelte';
    import MemoryCenter from '../components/MemoryCenter.svelte';
    import InformationKnowledge from '../components/InformationKnowledge.svelte';
    let viewportWidth = $state(window.innerWidth);
    let confirmation: ConfirmationDialog;
    let dialog: HTMLDialogElement;
    let sources = $state<DataSource[]>([]);
    let usage = $state<SourceUsage[]>([]);
    type InformationTab = 'overview' | 'knowledge' | 'sources' | 'memory' | 'plans' | 'activity' | 'usage';
    const tabs = $derived<InformationTab[]>(viewportWidth <= 700
        ? ['knowledge', 'sources', 'memory', 'plans', 'activity']
        : ['sources', 'memory', 'plans', 'usage']);
    const requestedTab = pageLocation.read().searchParams.get('tab');
    let memoryVisited = $state(requestedTab === 'memory');
    const initialTabs: InformationTab[] = window.innerWidth <= 700 ? ['knowledge', 'sources', 'memory', 'plans', 'activity'] : ['sources', 'memory', 'plans', 'usage'];
    const initialTab = initialTabs.find(item => item === requestedTab) ?? (window.innerWidth <= 700 ? 'overview' : 'sources');
    let tab = $state<InformationTab>(initialTab);
    let knowledgeVisited = $state(initialTab === 'knowledge');
    let activityVisited = $state(initialTab === 'activity');
    function selectTab(next: InformationTab) {
        tab = next;
        if (next === 'memory') memoryVisited = true;
        if (next === 'plans') plansVisited = true;
        if (next === 'knowledge') knowledgeVisited = true;
        if (next === 'activity') activityVisited = true;
        if (pageLocation.active()) {
            const target = pageLocation.read();
            if (next === 'overview') target.searchParams.delete('tab'); else target.searchParams.set('tab', next);
            pageLocation.replace(target);
        }
    }
    $effect(() => { if (viewportWidth > 700 && tab === 'overview' || tab !== 'overview' && !tabs.includes(tab)) selectTab('sources'); });
    const tabLabel = (item: InformationTab) => item === 'knowledge' ? 'chatKnowledge.knowledgePoints'
        : item === 'activity' ? 'shell.informationActivity'
        : item === 'memory' && viewportWidth <= 700 ? 'memoryCenter.title' : `data_sources.tab.${item}`;
    const tabHint = (item: InformationTab) => item === 'overview' ? 'shell.informationOverviewHint'
        : item === 'usage' ? 'data_sources.section.usage_hint' : `shell.informationRow.${item}.hint`;
    const informationIcons: Partial<Record<InformationTab, string>> = {
        knowledge: 'M6 3h8l4 4v14H6z M14 3v5h4 M9 12h6 M9 16h6',
        sources: 'M3 6h7l2 2h9v12H3z',
        memory: 'M8 7a4 4 0 1 0 8 0a4 4 0 1 0-8 0 M4 22v-4a8 8 0 0 1 16 0v4',
        plans: 'M4 5h16v16H4z M8 2v6 M16 2v6 M4 10h16',
        activity: 'M21 12a9 9 0 1 1-18 0a9 9 0 1 1 18 0 M12 6v6l4 2',
    };
    const overviewLabel = (item: InformationTab) => ['knowledge', 'sources', 'activity'].includes(item) ? `shell.informationRow.${item}.title` : tabLabel(item);
    function openInformation(event: MouseEvent, item: InformationTab) {
        if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
        event.preventDefault(); selectTab(item);
    }
    let plansVisited = $state(requestedTab === 'plans');
    function tabKey(event: KeyboardEvent, index: number) {
        if (event.altKey || event.ctrlKey || event.metaKey) return;
        const next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index + tabs.length - 1) % tabs.length
            : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : null;
        if (next === null) return;
        event.preventDefault(); selectTab(tabs[next]);
        (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
    }
    let sourcesLoaded = $state(false), usageLoaded = $state(false);
    let sourcesLoading = $state(false), usageLoading = $state(false);
    let sourcesError = $state<unknown>(), usageError = $state<unknown>(), formError = $state<unknown>();
    let notice = $state('');
    let operation = $state('');
    let confirming = $state(false);
    let editorId = $state('');
    let existing = $state<DataSource>();
    let draft = $state(sourceDraft());
    let baseline = $state('');
    let disposed = false, editorVersion = 0, sourcesVersion = 0, usageVersion = 0;
    const sourceRead = latestRequest<DataSource[]>(), usageRead = latestRequest<SourceUsage[]>();
    const busy = $derived(!!operation || confirming);
    const dirty = $derived(!!editorId && JSON.stringify(draft) !== baseline);
    const policyKeys: Record<string, string> = { defaultEnabled: 'default', conversationOptIn: 'opt_in', askWhenRelevant: 'ask', blocked: 'blocked' };
    function message(error: unknown) { return error instanceof DataSourceError ? t($locale, error.key) : errorMessage(error); }
    function policyKey(policy: string, hint = false) { return `data_sources.policy.${policyKeys[policy]}${hint ? '_hint' : ''}`; }
    async function loadSources() {
        if (disposed) return;
        const version = ++sourcesVersion; sourcesLoading = true;
        try {
            const value = await sourceRead.run(async signal => parseSources(await api('/api/data-sources', { signal })));
            if (value && !disposed) { sources = value; sourcesLoaded = true; sourcesError = undefined; }
        } catch (cause) { if (!disposed && version === sourcesVersion) sourcesError = cause; }
        finally { if (!disposed && version === sourcesVersion) sourcesLoading = false; }
    }
    async function loadUsage() {
        if (disposed) return;
        const version = ++usageVersion; usageLoading = true;
        try {
            const value = await usageRead.run(async signal => parseUsage(await api('/api/data-sources/usage', { signal })));
            if (value && !disposed) { usage = value; usageLoaded = true; usageError = undefined; }
        } catch (cause) { if (!disposed && version === usageVersion) usageError = cause; }
        finally { if (!disposed && version === usageVersion) usageLoading = false; }
    }
    function cancelReads() { sourceRead.cancel(); usageRead.cancel(); ++sourcesVersion; ++usageVersion; sourcesLoading = false; usageLoading = false; }
    async function refresh() { if (!busy) await Promise.all([loadSources(), loadUsage()]); }
    onMount(() => { void refresh(); return () => { disposed = true; ++editorVersion; cancelReads(); }; });
    async function edit(source?: DataSource) {
        if (busy || editorId) return;
        ++editorVersion; existing = source ? structuredClone($state.snapshot(source)) : undefined;
        // The host accepts draft.id. Keep it across failed creates to avoid duplicate sources.
        try { editorId = source?.id ?? newSourceId(); }
        catch (cause) { formError = cause; notice = 'data_sources.error.mutation'; return; }
        draft = sourceDraft(source); baseline = JSON.stringify(draft);
        formError = undefined; notice = ''; await tick(); if (!disposed) dialog.showModal();
    }
    function closeNow() { dialog.close(); editorId = ''; existing = undefined; formError = undefined; ++editorVersion; }
    async function close(event?: Event) {
        event?.preventDefault(); if (busy) return;
        const version = editorVersion;
        if (dirty) {
            confirming = true;
            const approved = await confirmation.ask({ title: t($locale, 'data_sources.discard.title'), description: t($locale, 'data_sources.discard.body'), confirmLabel: t($locale, 'data_sources.discard.confirm'), destructive: true });
            confirming = false;
            if (!approved || disposed || version !== editorVersion) return;
        }
        closeNow();
    }
    async function mutate(kind: 'save' | 'test' | 'delete', source?: DataSource) {
        if (busy || disposed) return;
        const id = kind === 'test' ? source?.id : editorId;
        if (!id) return;
        let payload;
        if (kind === 'save') {
            try { payload = { ...sourcePayload(draft), id }; } catch (cause) { formError = cause; return; }
        }
        if (kind === 'delete') {
            const version = editorVersion; confirming = true;
            const approved = await confirmation.ask({ title: t($locale, 'data_sources.delete.title'), description: t($locale, 'data_sources.delete.body'), confirmLabel: t($locale, 'data_sources.action.delete'), destructive: true });
            confirming = false;
            if (!approved || disposed || version !== editorVersion || editorId !== id) return;
        }
        cancelReads(); operation = kind; formError = undefined; notice = '';
        try {
            const path = `/api/data-sources/${encodeURIComponent(id)}`;
            const response = await api(kind === 'save' && !existing ? '/api/data-sources' : path + (kind === 'test' ? '/test' : ''), {
                method: kind === 'delete' ? 'DELETE' : kind === 'save' && existing ? 'PATCH' : 'POST',
                ...(payload ? { body: JSON.stringify(payload) } : {}),
            });
            if (disposed) return;
            if (kind === 'delete') {
                deletionReceipt(response, id); sources = sources.filter(item => item.id !== id);
                notice = 'data_sources.success.deleted';
            } else { sources = replaceSource(sources, sourceReceipt(response, id)); notice = 'data_sources.success.saved'; }
            if (kind !== 'test') closeNow();
        } catch (cause) {
            if (!disposed) { formError = cause; notice = 'data_sources.error.mutation'; }
        } finally {
            if (!disposed) {
                // Failure may have persisted a draft before validation. Never reset the editor.
                operation = 'refresh'; await Promise.all([loadSources(), loadUsage()]); operation = '';
            }
        }
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return; if (dirty || operation) { event.preventDefault(); event.returnValue = ''; } }
</script>

<svelte:window bind:innerWidth={viewportWidth} onbeforeunload={beforeUnload} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'data_sources.header.title')} — Nola</title>{/if}</svelte:head>
<ConfirmationDialog bind:this={confirmation} />
{#snippet usageItems(records: SourceUsage[])}
    {#each records as record, index (`${record.id}:${index}`)}
        <article class="source"><div class="row between"><strong>{record.sourceTitle || record.sourceId}</strong><span>{t($locale, disclosureKey(record))}</span></div><p>{record.purpose}</p><p class="muted">{t($locale, 'data_sources.usage.consumer')}: {record.consumer} · {resultTime(record.usedAt, $locale)}</p>{#if record.conversationId}<a class="button" href={conversationHref(record.conversationId)}>{t($locale, 'data_sources.usage.conversation')}</a>{/if}</article>
    {:else}<p class="muted">{t($locale, 'data_sources.usage.empty')}</p>{/each}
{/snippet}
{#if viewportWidth <= 700 && tab !== 'overview'}<div class="information-back"><button type="button" onclick={() => selectTab('overview')}><span aria-hidden="true">‹</span>{t($locale, 'shell.information')}</button></div>{/if}
<header class="page-header" class:mobile-overview={viewportWidth <= 700 && tab === 'overview'}><div><h1 id={viewportWidth <= 700 ? `data-tab-${tab}` : undefined}>{t($locale, tab !== 'overview' ? tabLabel(tab) : 'shell.information')}</h1><p class="muted">{t($locale, tabHint(tab))}</p></div><div class="actions page-actions">{#if tab === 'sources'}<button class="primary" disabled={busy || !sourcesLoaded} onclick={() => edit()}>{t($locale, 'data_sources.action.add')}</button>{/if}</div></header>
<div class="content management-content" class:mobile-overview={viewportWidth <= 700 && tab === 'overview'}>
    {#if viewportWidth <= 700 && tab === 'overview'}
        <nav class="information-navigation" aria-label={t($locale, 'shell.information')}>
            {#each tabs as item}<a href={`/data-sources?tab=${item}`} onclick={event => openInformation(event, item)}><span class="information-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d={informationIcons[item]} /></svg></span><span class="information-copy"><strong>{t($locale, overviewLabel(item))}</strong></span></a>{/each}
        </nav>
    {/if}
    {#if viewportWidth > 700}<div class="data-tabs" role="tablist" aria-label={t($locale, 'data_sources.header.title')}>
        {#each tabs as item, index}<button type="button" role="tab" id={`data-tab-${item}`} aria-controls={`data-panel-${item}`} aria-selected={tab === item} tabindex={tab === item ? 0 : -1} onclick={() => selectTab(item)} onkeydown={event => tabKey(event, index)}>{t($locale, tabLabel(item))}</button>{/each}
    </div>{/if}
    {#if knowledgeVisited}<div class="data-panel" id="data-panel-knowledge" role={viewportWidth <= 700 ? 'region' : 'tabpanel'} aria-labelledby="data-tab-knowledge" hidden={tab !== 'knowledge'}><InformationKnowledge /></div>{/if}
    {#if activityVisited}<div class="data-panel" id="data-panel-activity" role={viewportWidth <= 700 ? 'region' : 'tabpanel'} aria-labelledby="data-tab-activity" hidden={tab !== 'activity'}><section class="card"><h2>{t($locale, 'chatInspector.list.activity')}</h2><p class="muted">{t($locale, 'activity.results_hint')}</p><a class="button" href="/activity">{t($locale, 'chatInspector.list.activity')}</a></section></div>{/if}
    {#if notice && !editorId}<p role="status" class:error={!!formError}>{t($locale, notice)} {#if formError}{message(formError)}{/if}</p>{/if}
    <div class="card data-panel" id="data-panel-sources" role={viewportWidth <= 700 ? 'region' : 'tabpanel'} aria-labelledby="data-tab-sources" hidden={tab !== 'sources'} aria-busy={sourcesLoading || !!operation}>
        <div class="row between"><h2>{t($locale, 'data_sources.section.sources')}</h2><button disabled={busy || sourcesLoading} onclick={loadSources}>{t($locale, 'data_sources.action.refresh')}</button></div>
        <p class="muted">{t($locale, 'data_sources.refresh_hint')}</p>
        {#if sourcesError}<div role="alert" class="error">{t($locale, 'data_sources.error.load')}{message(sourcesError)}{#if sourcesLoaded}<p>{t($locale, 'data_sources.stale')}</p>{/if}</div>{/if}
        {#if sourcesLoading}<p role="status">{t($locale, 'data_sources.status.loading')}</p>{/if}
        {#if operation === 'test'}<p role="status">{t($locale, 'data_sources.status.testing')}</p>{/if}
        {#if sourcesLoaded}
            {#each sources as source (source.id)}
                <article class="source">
                    <div class="row between"><div><h2>{source.name}</h2><p class="muted location">{source.location}</p></div><span>{t($locale, `data_sources.state.${sourceState(source)}`)}</span></div>
                    <p>{Object.hasOwn(policyKeys, source.accessPolicy) ? t($locale, policyKey(source.accessPolicy)) : source.accessPolicy} · {source.kind === 'http' ? 'HTTP / HTTPS' : source.kind === 'localFile' ? t($locale, 'data_sources.kind.local') : source.kind}</p>
                    {#if source.validationContract}<p class="muted">{source.validationContract.recordCount} {t($locale, 'data_sources.contract.records')} · {resultTime(source.validationContract.validatedAt, $locale)}</p>{/if}
                    <div class="actions"><button disabled={busy} onclick={() => edit(source)}>{t($locale, 'data_sources.action.edit')}</button><button disabled={busy} onclick={() => mutate('test', source)}>{t($locale, 'data_sources.action.test')}</button></div>
                </article>
            {:else}<h2>{t($locale, 'data_sources.empty.title')}</h2><p class="muted">{t($locale, 'data_sources.empty.body')}</p>{/each}
        {/if}
    </div>
    <div class="data-panel" id="data-panel-plans" role={viewportWidth <= 700 ? 'region' : 'tabpanel'} aria-labelledby="data-tab-plans" hidden={tab !== 'plans'}>{#if plansVisited}<Automations embedded />{/if}</div>
    <div class="data-panel" id="data-panel-memory" role={viewportWidth <= 700 ? 'region' : 'tabpanel'} aria-labelledby="data-tab-memory" hidden={tab !== 'memory'}>{#if memoryVisited}<MemoryCenter />{/if}</div>
    <div class="card data-panel" id="data-panel-usage" role={viewportWidth <= 700 ? "region" : "tabpanel"} aria-labelledby={viewportWidth <= 700 ? "data-tab-activity" : "data-tab-usage"} hidden={tab !== 'usage' && tab !== 'activity'} aria-busy={usageLoading}>
        <div class="row between"><h2>{t($locale, 'data_sources.section.usage')}</h2><button disabled={busy || usageLoading} onclick={loadUsage}>{t($locale, 'data_sources.action.refresh')}</button></div>
        <p class="muted">{t($locale, 'data_sources.section.usage_hint')}</p>
        {#if usageError}<div class="error" role="alert">{t($locale, 'data_sources.usage.load_failed')}: {message(usageError)}{#if usageLoaded}<p>{t($locale, 'data_sources.stale')}</p>{/if}</div>{/if}
        {#if usageLoading}<p role="status">{t($locale, 'activity.refreshing')}</p>{/if}
        {#if usageLoaded}{@render usageItems(usage)}{/if}
    </div>
</div>
<dialog use:ownDialog bind:this={dialog} oncancel={close} aria-labelledby="source-editor-title">
    {#if editorId}<form onsubmit={event => { event.preventDefault(); void mutate('save'); }} aria-busy={!!operation}>
        <h2 id="source-editor-title">{t($locale, existing ? 'data_sources.modal.edit_title' : 'data_sources.modal.add_title')}</h2>
        {#if formError}<div class="error" role="alert">{#if notice}{t($locale, notice)} {/if}{message(formError)}</div>{/if}
        {#if operation}<p role="status">{t($locale, operation === 'refresh' ? 'activity.refreshing' : operation === 'delete' ? 'data_sources.busy.delete' : 'data_sources.status.testing')}</p>{/if}
        <fieldset disabled={busy}>
            <label>{t($locale, 'data_sources.form.name')}<input bind:value={draft.name} required autocomplete="off" /></label>
            <div class="fields"><label>{t($locale, 'data_sources.form.kind')}<select bind:value={draft.kind}>{#if !sourceKinds.includes(draft.kind as 'http' | 'localFile')}<option value={draft.kind} disabled>{draft.kind}</option>{/if}<option value="localFile">{t($locale, 'data_sources.kind.local')}</option><option value="http">HTTP / HTTPS</option></select></label><label>{t($locale, 'data_sources.form.freshness')}<input type="text" inputmode="numeric" bind:value={draft.maximumAgeMinutes} required /></label></div>
            <label>{t($locale, 'data_sources.form.location')}<input bind:value={draft.location} required autocomplete="off" spellcheck="false" placeholder={draft.kind === 'http' ? 'https://example.com/data.json' : '~/Documents/notes.json'} /></label>
            <p class="muted">{t($locale, draft.kind === 'http' ? 'data_sources.form.http_hint' : 'data_sources.form.local_hint')}</p>
            <label>{t($locale, 'data_sources.form.policy')}<select bind:value={draft.accessPolicy}>{#if !Object.hasOwn(policyKeys, draft.accessPolicy)}<option value={draft.accessPolicy} disabled>{draft.accessPolicy}</option>{/if}{#each sourcePolicies as policy}<option value={policy}>{t($locale, policyKey(policy))}</option>{/each}</select></label>
            {#if Object.hasOwn(policyKeys, draft.accessPolicy)}<p class="muted">{t($locale, policyKey(draft.accessPolicy, true))}</p>{/if}
        </fieldset>
        {#if existing?.validationContract}
            <section class="card"><h3>{t($locale, 'data_sources.contract.saved')}</h3><p>{t($locale, `data_sources.state.${sourceState(existing)}`)}</p><p>{existing.validationContract.redactedSample}</p><dl><dt>{t($locale, 'data_sources.contract.reader')}</dt><dd>{existing.validationContract.readerDescription}</dd><dt>{t($locale, 'data_sources.contract.fields')}</dt><dd>{existing.validationContract.schemaFields.join(', ') || '—'}</dd><dt>{t($locale, 'data_sources.contract.validated_at')}</dt><dd>{resultTime(existing.validationContract.validatedAt, $locale)}</dd></dl>{#if existing.lastError || existing.validationContract.errorMessage}<p class="error">{existing.lastError || existing.validationContract.errorMessage}</p>{/if}</section>
        {/if}
        <div class="actions">{#if existing}<button type="button" class="danger" disabled={busy} onclick={() => mutate('delete')}>{t($locale, 'data_sources.action.delete')}</button>{/if}<button type="button" disabled={busy} onclick={() => close()}>{t($locale, 'shell.cancel')}</button><button class="primary" disabled={busy}>{t($locale, 'data_sources.action.test_save')}</button></div>
    </form>{/if}
</dialog>
<style>
    .data-tabs { display: flex; flex-wrap: wrap; gap: 8px; padding: 0; background: transparent; border-radius: 0; }
    .data-tabs button { flex: 0 1 auto; min-height: 44px; border-color: transparent; background: transparent; transition: border-color 160ms ease; }
    .data-tabs button[aria-selected='true'] { background: var(--accent-subtle); border-color: transparent; }
    .data-panel { display: grid; gap: 16px; min-width: 0; } .data-panel[hidden] { display: none; }
    .source { border-top: 1px solid var(--border); padding-top: 16px; display: grid; gap: 8px; overflow-wrap: anywhere; }
    .location, dd { overflow-wrap: anywhere; } form, fieldset { display: grid; gap: 16px; } fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
    dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 8px 16px; } dt { color: var(--muted); } dd { margin: 0; }
    dialog { max-width: 760px; } p { margin: 0; } .source h2 { font-size: 16px; }
    .information-back { padding: 4px 16px 0; }
    .information-back button { display: flex; gap: 8px; align-items: center; min-height: 44px; padding: 4px; background: transparent; border: 0; }
    .information-back span { font-size: 24px; }
    .information-navigation { display: grid; gap: 0; overflow: visible; border-top: 1px solid var(--border); }
    .information-navigation a { display: flex; align-items: center; gap: 14px; min-height: 60px; padding: 12px; border: 0; border-bottom: 1px solid var(--border); border-radius: 0; white-space: normal; }
    .information-navigation a:hover { background: var(--accent-subtle); }
    .information-icon { width: 36px; height: 36px; flex: 0 0 36px; display: grid; place-items: center; border: 1px solid var(--border); border-radius: 9px; }
    .information-icon svg { width: 21px; height: 21px; fill: none; stroke: currentColor; stroke-width: 1.25; stroke-linejoin: round; stroke-linecap: round; }
    .information-copy { flex: 1; min-width: 0; display: grid; gap: 3px; }
    .information-copy strong { font-size: 17px; line-height: 24px; font-weight: 600; }
    @media (max-width: 700px) {
        .page-header.mobile-overview { padding: 24px 20px 22px; }
        .page-header.mobile-overview h1 { font-size: 28px; line-height: 36px; font-weight: 650; }
        .page-header.mobile-overview p { font-size: 15px; line-height: 24px; margin-top: 6px; }
        .content.mobile-overview { padding: 0 20px 28px; gap: 24px; }

        .data-tabs { flex-wrap: nowrap; overflow-x: auto; min-width: 0; width: 100%; scrollbar-width: thin; }
        .data-tabs button { flex: 0 0 auto; white-space: nowrap; min-height: 44px; }
    }
</style>
