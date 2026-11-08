<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { agentAccessSnapshot, agentAccessReceipt, agentAccessDraft, agentAccessDirty, agentAccessPatch, permissionProfiles, knownPermissionProfile, requiresAllowAllConfirmation } from '../lib/agentAccessSettings';
    import type { AgentAccessSnapshot, AgentAccessDraft } from '../lib/agentAccessSettings';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';

    let confirmation: ConfirmationHandle;
    let snapshot = $state<AgentAccessSnapshot>();
    let draft = $state<AgentAccessDraft>();
    let loading = $state(false);
    let saving = $state(false);
    let uncertain = $state(false);
    let error = $state('');
    let notice = $state('');
    let pendingConfirmation = $state(false);
    let controller: AbortController | undefined;
    let disposed = false;
    const dirty = $derived(!!snapshot && !!draft && agentAccessDirty(snapshot, draft));
    const busy = $derived(loading || saving || pendingConfirmation);

    async function load() {
        if (busy) return;
        if (dirty || uncertain) {
            pendingConfirmation = true;
            const accepted = await confirmation.ask({ title: t($locale, 'agentAccess.reloadTitle'), description: t($locale, 'agentAccess.reloadBody'), confirmLabel: t($locale, 'agentAccess.reloadConfirm'), destructive: true });
            pendingConfirmation = false;
            if (!accepted || disposed) return;
        }
        controller?.abort();
        const request = new AbortController(); controller = request;
        loading = true; error = ''; notice = '';
        try {
            const next = agentAccessSnapshot(await api('/api/agent/settings', { signal: request.signal }));
            if (disposed || request.signal.aborted || controller !== request) return;
            snapshot = next; draft = agentAccessDraft(next); uncertain = false;
        } catch {
            if (!disposed && !request.signal.aborted) error = 'agentAccess.loadFailed';
        } finally { if (controller === request) loading = false; }
    }
    async function save(event: SubmitEvent) {
        event.preventDefault();
        if (busy || uncertain || !snapshot || !draft || !dirty || !knownPermissionProfile(draft.permissionProfile)) return;
        // Capture the exact draft being approved; fields stay locked during confirmation.
        const baseline = snapshot;
        const submitted = { ...draft };
        if (requiresAllowAllConfirmation(baseline, submitted)) {
            pendingConfirmation = true;
            const accepted = await confirmation.ask({ title: t($locale, 'agentAccess.allowAllTitle'), description: t($locale, 'agentAccess.allowAllBody'), confirmLabel: t($locale, 'agentAccess.allowAllConfirm'), destructive: true });
            pendingConfirmation = false;
            if (!accepted || disposed || snapshot !== baseline) return;
        }
        saving = true; error = ''; notice = '';
        const patch = agentAccessPatch(baseline, submitted);
        try {
            const next = agentAccessReceipt(await api('/api/agent/settings', {
                method: 'POST', body: JSON.stringify(patch),
            }), patch);
            if (disposed) return;
            snapshot = next; draft = agentAccessDraft(next); notice = 'agentAccess.saved';
        } catch { if (!disposed) { uncertain = true; error = 'agentAccess.saveFailed'; } }
        finally { saving = false; }
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return;
        if (dirty || saving || uncertain) { event.preventDefault(); event.returnValue = ''; }
    }
    onMount(() => { void load(); return () => { disposed = true; controller?.abort(); }; });
</script>

<svelte:window onbeforeunload={beforeUnload} />
<ConfirmationDialog bind:this={confirmation} />
<section class="card agent-access" id="agent" aria-labelledby="agent-access-title" aria-busy={busy}>
    <header><h2 id="agent-access-title">{t($locale, 'agentAccess.title')}</h2><p class="muted">{t($locale, 'agentAccess.scope')}</p></header>
    {#if loading}<p role="status">{t($locale, 'agentAccess.loading')}</p>{/if}
    {#if error}<p class="notice error" role="alert">{t($locale, error)}</p>{/if}
    {#if notice}<p role="status">{t($locale, notice)}</p>{/if}
    {#if draft}
        <form onsubmit={save}>
            <fieldset disabled={busy || uncertain}>
                <label>{t($locale, 'agentAccess.profile')}<select bind:value={draft.permissionProfile} aria-describedby="agent-access-profile-description">
                    {#if !knownPermissionProfile(draft.permissionProfile)}<option value={draft.permissionProfile}>{t($locale, 'agentAccess.unknownValue', { mode: draft.permissionProfile })}</option>{/if}
                    {#each permissionProfiles as profile}<option value={profile}>{t($locale, `agentAccess.profile.${profile}`)}</option>{/each}
                </select></label>
                <p id="agent-access-profile-description" class="muted" aria-live="polite">{t($locale, knownPermissionProfile(draft.permissionProfile) ? `agentAccess.detail.${draft.permissionProfile}` : 'agentAccess.unknownProfile')}</p>
                <label>{t($locale, 'agentAccess.prompt')}<textarea rows="5" bind:value={draft.customSystemPrompt}></textarea></label>
                <h3>{t($locale, 'agentAccess.devTools')}</h3>
                <label class="toggle"><input type="checkbox" bind:checked={draft.enableLocalDevTools} />{t($locale, 'agentAccess.enableDevTools')}</label>
                <p class="muted">{t($locale, 'agentAccess.devHint')}</p>
                <label>{t($locale, 'agentAccess.directories')}<textarea rows="3" bind:value={draft.allowedDirs} spellcheck="false"></textarea></label>
                <label>{t($locale, 'agentAccess.prefixes')}<textarea rows="3" bind:value={draft.bashPrefixes} spellcheck="false"></textarea></label>
                <p class="muted">{t($locale, 'agentAccess.prefixHint')}</p>
            </fieldset>
            <div class="actions"><button type="button" disabled={busy} onclick={load}>{t($locale, 'agentAccess.reload')}</button><button class="primary" disabled={busy || uncertain || !dirty || !knownPermissionProfile(draft.permissionProfile)}>{t($locale, saving ? 'agentAccess.saving' : 'agentAccess.save')}</button></div>
        </form>
    {:else if !loading}<button disabled={busy} onclick={load}>{t($locale, 'agentAccess.reload')}</button>{/if}
</section>
<style>
    .agent-access, form, fieldset { display: grid; gap: 16px; min-width: 0; }
    fieldset { border: 0; margin: 0; padding: 0; }
    h2, h3, p { margin: 0; }
    header { display: grid; gap: 8px; }
    textarea { width: 100%; min-height: 80px; resize: vertical; padding: 10px; border-radius: 7px; border: 1px solid var(--border); background: var(--canvas); color: inherit; font: inherit; }
    .toggle { display: flex; align-items: center; gap: 8px; }
    .toggle input { width: auto; }
</style>
