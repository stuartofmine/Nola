<script lang="ts">
    let { embedded = false }: { embedded?: boolean } = $props();
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    import ActionDisclosure from '../components/ActionDisclosure.svelte';
    import { onMount, tick } from 'svelte';
    import { automationEditDraft, automationEditRequest, automationCanEdit } from '../lib/automations';
    import { parseAutomationSkills, type AutomationSkill } from '../lib/automations';
    import { api as unscopedApi, ApiError } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { automationFormats, parseAutomations, newAutomationDraft, automationCreateRequest, automationScript, automationApprovalRequest, sameAutomationRevision, automationKnown, automationCanRun, automationHistoryLink, automationWeekday, automationCapabilities, automationCanApprove } from '../lib/automations';
    import { automationRunIntent, automationRunResponse, automationExecutionSpec, automationRunConversationLink } from '../lib/automations';
    import type { Automation, AutomationRunIntent } from '../lib/automations';
    let confirmation: ConfirmationHandle;
    let actions = $state<Automation[]>([]);
    let capabilities = $state(automationCapabilities(null));
    let loaded = $state(false);
    let busy = $state(false);
    let error = $state('');
    let loadFailed = $state(false);
    let notice = $state('');
    let runConversationHref = $state<string>();
    let query = $state('');
    let filter = $state('all');
    let draft = $state(newAutomationDraft());
    let creating = $state(false);
    let editing = $state<Automation>();
    let skills = $state<AutomationSkill[]>([]);
    let skillsFailed = $state(false);
    let skillsLoading = $state(false);
    async function refreshSkills() {
        if (skillsLoading) return;
        skillsLoading = true; skillsFailed = false;
        try { const next = parseAutomationSkills(await api('/api/automation-skills')); if (!disposed) skills = next; }
        catch { if (!disposed) { skillsFailed = true; skills = []; } }
        finally { if (!disposed) skillsLoading = false; }
    }
    let formElement = $state<HTMLFormElement>();
    let editorOpener: HTMLElement | undefined;
    async function openEditor(action?: Automation) {
        if (busy || creating) return;
        editorOpener = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
        editing = action ? structuredClone($state.snapshot(action)) : undefined;
        draft = editing ? automationEditDraft(editing) : newAutomationDraft();
        creating = true; error = '';
        void refreshSkills();
        await tick(); if (!disposed && pageLocation.active()) formElement?.querySelector('input')?.focus();
    }
    async function closeEditor() {
        if (busy) return;
        if (!await confirmation.ask({ title: t($locale, 'automation.cancel'), description: t($locale, 'automation.discardBody'), confirmLabel: t($locale, 'automation.discard'), destructive: true })) return;
        creating = false; editing = undefined; draft = newAutomationDraft();
        await tick(); if (!disposed && pageLocation.active()) editorOpener?.focus();
    }
    let disposed = false;
    let controller: AbortController | undefined;
    const visible = $derived(actions.filter(action => (!query.trim() || `${action.name} ${action.url} ${action.format} ${t($locale, `automation.format.${action.format}`)}`.toLocaleLowerCase($locale).includes(query.trim().toLocaleLowerCase($locale))) && (filter === 'all' || (filter === 'pending' ? action.format === 'script' && action.scriptApproved !== true : action.enabled === (filter === 'enabled')))));
    async function refresh() {
        if (busy) return;
        busy = true; error = ''; loadFailed = false;
        controller = new AbortController();
        try { const payload = await api('/api/scheduled-actions', { signal: controller.signal }); const next = parseAutomations(payload); if (!disposed) { actions = next; capabilities = automationCapabilities(payload); loaded = true; } }
        catch { if (!disposed) loadFailed = true; }
        finally { if (!disposed) busy = false; }
    }
    async function create() {
        if (busy) return;
        let body;
        try { body = editing ? automationEditRequest(editing, draft) : automationCreateRequest(draft); } catch { error = 'automation.invalidDraft'; return; }
        busy = true; error = ''; notice = ''; runConversationHref = undefined;
        try {
            const result = await api<{ success: boolean; action: unknown }>(editing ? `/api/scheduled-actions/${encodeURIComponent(editing.id)}` : '/api/scheduled-actions', { method: editing ? 'PATCH' : 'POST', body: JSON.stringify(body) });
            if (result.success !== true) throw new Error();
            const [saved] = parseAutomations({ actions: [result.action] });
            if (editing && (saved.id !== editing.id || !Number.isSafeInteger(saved.revision) || saved.revision! <= editing.revision!)) throw new Error();
            if (!disposed) { creating = false; draft = newAutomationDraft(); notice = editing ? 'automation.updated' : 'automation.created'; editing = undefined; }
        } catch (cause) { if (!disposed) error = cause instanceof ApiError && cause.code === 'automation_revision_conflict' ? 'automation.approvalChanged' : editing ? 'automation.requestFailed' : 'automation.createFailed'; }
        finally { if (!disposed) busy = false; }
        if (!disposed && !creating) { await refresh(); await tick(); if (!disposed && pageLocation.active()) editorOpener?.focus(); }
    }
    async function mutate(action: Automation, operation: 'toggle' | 'run' | 'approve' | 'delete') {
        if (busy || (operation !== 'delete' && !automationKnown(action))) return;
        if (operation === 'run' && (!capabilities.manualRun || !automationCanRun(action))) return;
        if (operation === 'toggle' && (!Number.isSafeInteger(action.revision) || action.revision! <= 0)) return;
        const captured = { ...action };
        busy = true; error = ''; notice = ''; runConversationHref = undefined;
        try {
            if (operation !== 'toggle') {
                const prefix = operation === 'run' ? 'runConfirm' : operation;
                const accepted = await confirmation.ask({ title: t($locale, `automation.${prefix}Title`), description: t($locale, operation === 'run' ? 'automation.executionContractHint' : `automation.${prefix}Body`, { name: captured.name }), confirmLabel: t($locale, operation === 'run' ? 'automation.runNow' : `automation.${operation}`), destructive: true });
                if (!accepted || disposed) return;
            }
            const payload = await api('/api/scheduled-actions');
            const current = parseAutomations(payload);
            if (disposed) return;
            capabilities = automationCapabilities(payload);
            if (operation === 'run' && !capabilities.manualRun) { error = 'automation.executionUnavailable'; return; }
            if (!sameAutomationRevision(captured, current.find(item => item.id === captured.id))) { error = 'automation.approvalChanged'; actions = current; return; }
            const path = `/api/scheduled-actions/${encodeURIComponent(captured.id)}`;
            const storageKey = `nola.automation.pending.${captured.id}`;
            let intent: AutomationRunIntent | undefined;
            if (operation === 'run') {
                const saved = sessionStorage.getItem(storageKey);
                intent = automationRunIntent(captured, saved ? JSON.parse(saved) : undefined);
                // Persist before dispatch. A lost response or page reload must
                // retry the same occurrence, never manufacture another run.
                sessionStorage.setItem(storageKey, JSON.stringify(intent));
            }
            const options = operation === 'delete' ? { method: 'DELETE' } : operation === 'toggle' ? { method: 'PATCH', body: JSON.stringify({ enabled: !captured.enabled, expectedRevision: captured.revision }) } : operation === 'approve' ? { method: 'POST', body: JSON.stringify(automationApprovalRequest(captured)) } : { method: 'POST', body: JSON.stringify({ occurrenceKey: intent!.occurrenceKey, expectedRevision: intent!.expectedRevision }) };
            const result = await api<{ success?: boolean }>(path + (operation === 'approve' ? '/approve-script' : operation === 'run' ? '/run-now' : ''), options);
            if (!disposed) {
                if (operation === 'run') {
                    const outcome = automationRunResponse(result, intent!.expectedRevision);
                    notice = outcome.message;
                    runConversationHref = automationRunConversationLink(result, intent!.expectedRevision);
                    if (outcome.terminal) sessionStorage.removeItem(storageKey);
                } else if (result.success !== true) error = 'automation.requestFailed';
                else notice = operation === 'delete' ? 'automation.deleted' : 'automation.updated';
                actions = parseAutomations(await api('/api/scheduled-actions'));
            }
        } catch (cause) { if (!disposed) error = cause instanceof ApiError && cause.code === 'automation_revision_conflict' ? 'automation.approvalChanged' : 'automation.requestFailed'; }
        finally { if (!disposed) busy = false; }
    }
    onMount(() => { const initialDraft = pageLocation.read().searchParams.get('draft');
        if (initialDraft) { void openEditor().then(() => { draft.name = initialDraft; }); }
        void refresh(); return () => { disposed = true; controller?.abort(); }; });
</script>

<ConfirmationDialog bind:this={confirmation} />
<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'automation.title')} · Nola</title>{/if}</svelte:head>
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (creating || busy) event.preventDefault(); }} />
<section>
    <header class="page-header" class:embedded-header={embedded}>{#if !embedded}<div><h1>{t($locale, 'automation.title')}</h1><p class="muted">{t($locale, 'automation.subtitle')}</p></div>{/if}<div class="actions"><button disabled={busy} onclick={refresh}>{t($locale, 'automation.refresh')}</button><button class="primary" disabled={busy || creating} onclick={() => openEditor()}>{t($locale, 'automation.create')}</button></div></header>
    <div class="content management-content" class:embedded-content={embedded}>
    {#if loaded}<aside role="status">{t($locale, capabilities.scheduledDispatch ? 'automation.scheduledAvailable' : capabilities.manualRun ? 'automation.hostScheduleHint' : 'automation.executionUnavailable')}</aside>{/if}
    {#if loadFailed}<p role="alert" class="error">{t($locale, 'automation.loadFailed')}</p>{/if}
    {#if error}<p role="alert" class="error">{t($locale, error)}</p>{/if}
    {#if notice}<p role="status">{t($locale, notice)}</p>{/if}
    {#if runConversationHref}<a class="button" href={runConversationHref}>{t($locale, 'modelTest.openConversation')}</a>{/if}
    {#if creating}
        <form bind:this={formElement} class="card" onsubmit={event => { event.preventDefault(); void create(); }}>
            <h2>{t($locale, editing ? 'automation.edit' : 'automation.createTitle')}</h2>
            <p class="muted">{t($locale, 'automation.executionContractHint')}</p>
            {#if editing && draft.format === 'script'}<p>{t($locale, 'automation.editScriptHint')}</p>{/if}
            <fieldset disabled={busy}>
                <label>{t($locale, 'automation.name')}<input bind:value={draft.name} required /></label>
                <label>{t($locale, 'automation.type')}<select bind:value={draft.format}>{#each automationFormats as format}<option value={format}>{t($locale, `automation.format.${format}`)}</option>{/each}</select></label>
                {#if draft.format === 'script'}
                    <label>{t($locale, 'automation.script')}<textarea bind:value={draft.script} rows="8" required spellcheck="false"></textarea></label>
                    <label>{t($locale, 'automation.cwd')}<input bind:value={draft.cwd} /></label>
                    <p>{t($locale, 'automation.policyHint')}</p>
                {:else if draft.format !== 'agent'}
                    <label>{t($locale, draft.format === 'search' ? 'automation.query' : ['file', 'folder'].includes(draft.format) ? 'automation.path' : 'automation.url')}<input bind:value={draft.source} required /></label>
                {/if}
                <label>{t($locale, 'automation.prompt')}<textarea bind:value={draft.prompt} rows="3" required></textarea></label>
                <label>{t($locale, 'automation.workspaceRoot')}<input bind:value={draft.workspaceRoot} required /></label>
                <label>{t($locale, 'automation.permissionProfile')}<select bind:value={draft.permissionProfile} required><option value="" disabled>{t($locale, 'automation.selectPermission')}</option>{#each ['cautious', 'allow_all'] as profile}<option value={profile}>{t($locale, `automation.permission.${profile}`)}</option>{/each}</select></label>
                <label>{t($locale, 'automation.modelSelection')}<select><option value="use_current_default_at_admission">{t($locale, 'automation.currentDefault')}</option></select></label>
                <fieldset aria-label={t($locale, 'automation.skills')}>
                    <legend>{t($locale, 'automation.skills')}</legend>
                    {#if skillsLoading}<p role="status">{t($locale, 'automation.loading')}</p>{/if}
                    {#if skillsFailed}<p role="alert">{t($locale, 'automation.skillsFailed')}</p><button type="button" onclick={refreshSkills}>{t($locale, 'automation.refresh')}</button>{/if}
                    {#each skills as skill (skill.id)}
                        <label class="skill-option"><input type="checkbox" value={skill.id} bind:group={draft.skillIds} /> <span>{skill.name}</span></label>
                    {/each}
                    {#each draft.skillIds.filter(id => !skills.some(skill => skill.id === id)) as id (id)}
                        <label class="skill-option"><input type="checkbox" value={id} bind:group={draft.skillIds} /> <span>{id} — {t($locale, 'automation.skillUnavailable')}</span></label>
                    {/each}
                    <p class="muted">{t($locale, 'automation.skillsHint')}</p>
                </fieldset>
                <fieldset class="schedule-fields" disabled={!capabilities.scheduledDispatch}><legend>{t($locale, 'automation.schedule')}</legend>
                {#if !capabilities.scheduledDispatch}<p class="muted">{t($locale, 'automation.hostScheduleHint')}</p>{/if}
                <div class="grid"><label>{t($locale, 'automation.frequency')}<select bind:value={draft.frequency}><option value="daily">{t($locale, 'automation.daily')}</option><option value="weekly">{t($locale, 'automation.weekly')}</option></select></label><label>{t($locale, 'automation.time')}<input type="time" bind:value={draft.time} required /></label>
                {#if draft.frequency === 'weekly'}<label>{t($locale, 'automation.weekday')}<select bind:value={draft.weekday}>{#each [1, 2, 3, 4, 5, 6, 7] as day}<option value={day}>{automationWeekday(day, $locale)}</option>{/each}</select></label>{/if}</div>
                <p class="muted">{t($locale, 'automation.localTimezone')}</p></fieldset>
                <div class="buttons"><button class="primary" type="submit">{t($locale, 'automation.save')}</button><button type="button" onclick={closeEditor}>{t($locale, 'automation.cancel')}</button></div>
            </fieldset>
        </form>
    {/if}
    <div class="filters"><label>{t($locale, 'automation.search')}<input type="search" bind:value={query} placeholder={t($locale, 'automation.searchPlaceholder')} /></label><label>{t($locale, 'automation.filter')}<select bind:value={filter}>{#each ['all', 'enabled', 'disabled', 'pending'] as value}<option value={value}>{t($locale, `automation.${value}`)}</option>{/each}</select></label></div>
    {#if !loaded && busy}<p role="status">{t($locale, 'automation.loading')}</p>{:else if loaded && !visible.length}<p>{t($locale, actions.length ? 'automation.noMatches' : 'automation.empty')}</p>{/if}
    {#each visible as action (action.id)}
        {@const executionSpec = automationExecutionSpec(action)}
        {@const script = automationScript(action)}
        <article class="automation-row">
            <div class="heading"><div class="action-identity"><h2>{action.name}</h2><span class="badge">{t($locale, action.format === 'script' && action.scriptApproved !== true ? 'automation.pending' : action.enabled ? 'automation.enabled' : 'automation.disabled')}</span></div><button disabled={busy || creating || !automationCanEdit(action)} onclick={() => openEditor(action)}>{t($locale, 'automation.edit')}</button></div>
            {#if action.url && action.url !== executionSpec?.root}<p class="source">{action.url}</p>{/if}
            {#if executionSpec}<p class="task-prompt">{executionSpec.prompt}</p><dl class="execution-context"><div><dt>{t($locale, 'automation.workspaceRoot')}</dt><dd>{executionSpec.root}</dd></div><div><dt>{t($locale, 'automation.permissionProfile')}</dt><dd>{t($locale, `automation.permission.${executionSpec.permission}`)}</dd></div></dl>{/if}
            <p class="muted">{automationKnown(action) || action.format === 'reminder' ? t($locale, `automation.format.${action.format}`) : `${t($locale, 'automation.unknown')}: ${action.format}`} · {t($locale, 'automation.schedule')}: {['daily', 'weekly'].includes(action.frequency) ? t($locale, `automation.${action.frequency}`) : action.frequency} {action.time} {action.frequency === 'weekly' && action.weekday ? automationWeekday(action.weekday, $locale) : ''} · {action.timezone === 'local' ? t($locale, 'modelDisplay.localTimezone') : action.timezone}</p>
            {#if action.format === 'script'}<details open={action.scriptApproved !== true}><summary>{t($locale, 'automation.scriptPreview')}</summary>{#if script}<pre>{script.script}</pre><p>{t($locale, 'automation.cwd')}: {script.cwd || action.scriptCwd || t($locale, 'automation.unspecified')}</p><p class="muted">{t($locale, 'automation.policyHint')}</p>{:else}<p role="alert">{t($locale, 'automation.scriptUnavailable')}</p>{/if}</details>{/if}
            <div class="buttons"><button disabled={busy || !capabilities.manualRun || !automationCanRun(action)} onclick={() => mutate(action, 'run')}>{t($locale, 'automation.runNow')}</button><button disabled={busy || !Number.isSafeInteger(action.revision) || action.revision! <= 0 || !automationKnown(action) || (!action.enabled && !automationCanRun(action))} onclick={() => mutate(action, 'toggle')}>{t($locale, action.enabled ? 'automation.pause' : 'automation.enable')}</button>{#if action.format === 'script' && action.scriptApproved !== true}<button disabled={busy || !automationCanApprove(action)} onclick={() => mutate(action, 'approve')}>{t($locale, 'automation.approve')}</button>{/if}<a href={automationHistoryLink(action)}>{t($locale, 'automation.history')}</a><ActionDisclosure label={t($locale, 'remote.more_actions')}><button class="danger" disabled={busy} onclick={() => mutate(action, 'delete')}>{t($locale, 'automation.delete')}</button></ActionDisclosure></div>
        </article>
    {/each}
    </div>
</section>

<style>
    .embedded-header { padding: 0 0 12px; justify-content: flex-end; }
    .embedded-content { padding: 0; max-width: none; }
    .automation-row { display: grid; gap: 12px; border-top: 1px solid var(--border); padding: 24px 0; min-width: 0; }
    .task-prompt { margin: 0; }
    .execution-context { display: flex; flex-wrap: wrap; gap: 8px 24px; margin: 0; font-size: var(--text-detail); }
    .execution-context > div { display: flex; flex-wrap: wrap; gap: 4px 8px; min-width: 0; }
    .execution-context dt { color: var(--muted); } .execution-context dd { margin: 0; overflow-wrap: anywhere; }
    .action-identity { display: flex; align-items: baseline; flex-wrap: wrap; gap: 12px; min-width: 0; }
    .action-identity h2 { overflow-wrap: anywhere; }
    .heading > button { background: transparent; border-color: transparent; flex-shrink: 0; }
    .buttons > a { display: inline-flex; align-items: center; min-height: var(--control-hit-size); padding: 8px; text-decoration: underline; text-underline-offset: 3px; }
    .buttons > button:not(.primary) { border-color: transparent; background: transparent; }
    .filters { padding-block: 8px 16px; }
    .skill-option{display:flex;align-items:center;gap:8px;min-width:0;overflow-wrap:anywhere}.skill-option input{width:auto;flex:none;margin:0}.skill-option span{min-width:0}
    .heading,.buttons,.filters{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.heading{justify-content:space-between}aside{padding:12px 0;border-bottom:1px solid var(--border);color:var(--muted);line-height:1.6}fieldset{border:0;padding:0;display:grid;gap:14px;min-width:0}label{font-size:.875rem;flex:1;min-width:min(180px,100%)}textarea{box-sizing:border-box;width:100%;padding:10px;border:1px solid var(--border);border-radius:7px;background:var(--canvas);color:inherit}a{color:var(--accent)}.source{overflow-wrap:anywhere;font-size:13px}.badge{font-size:var(--text-detail);color:var(--muted)}.grid{display:flex;gap:12px;flex-wrap:wrap}pre{white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px;padding:14px;background:var(--panel);border-radius:8px;max-height:360px;overflow:auto}summary{cursor:pointer;font-size:13px}
</style>
