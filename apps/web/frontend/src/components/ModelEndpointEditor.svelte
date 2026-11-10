<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { tick } from 'svelte';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { ModelValidationError, modelValidationText, type ModelValidationIssue } from '../lib/modelValidation';
    import { endpointSnapshot, endpointDirty, endpointRequest, endpointValidationError } from '../lib/modelEndpoints';
    import type { EndpointSnapshot, EndpointDraft } from '../lib/modelEndpoints';
    import type { RemoteProfile } from '../lib/modelProfiles';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { locale, t } from '../lib/i18n';
    let { onSaved, onBusyChange, onStage }: { onSaved: (profiles: RemoteProfile[]) => void; onBusyChange: (busy: boolean) => void;
        onStage?: (id: string, draft: EndpointDraft) => void } = $props();
    let confirmation: ConfirmationHandle;
    let dialog: HTMLDialogElement;
    let snapshot = $state<EndpointSnapshot>();
    let draft = $state<EndpointDraft>();
    let busy = $state(false);
    let error = $state<string | ModelValidationIssue>('');
    const dirty = $derived(!!snapshot && !!draft && endpointDirty(snapshot, draft));
    const validation = $derived(snapshot && draft ? endpointValidationError(snapshot, draft) : null);

    export function open(profiles: RemoteProfile[], id: string) {
        if (busy) return;
        snapshot = endpointSnapshot(profiles, id);
        draft = { ...snapshot.draft };
        error = '';
        const owner = snapshot;
        void tick().then(() => { if (snapshot === owner && dialog?.isConnected && !dialog.open) dialog.showModal(); });
    }
    function setBusy(value: boolean) { busy = value; onBusyChange(value); }
    async function discardConfirmed() {
        return !dirty || await confirmation.ask({ title: t($locale, 'remote.endpoint_discard_title'), description: t($locale, 'remote.endpoint_discard_body'), confirmLabel: t($locale, 'models.discard'), cancelLabel: t($locale, 'models.continueEditing'), destructive: true });
    }
    async function close(event?: Event) {
        event?.preventDefault();
        if (busy || !await discardConfirmed()) return;
        dialog.close(); snapshot = undefined; draft = undefined;
    }
    async function reload() {
        if (busy || !snapshot || !await discardConfirmed()) return;
        const id = snapshot.id;
        setBusy(true); error = '';
        try {
            const current = await api<{ profiles: RemoteProfile[] }>('/api/model-profiles');
            const replacement = endpointSnapshot(current.profiles, id);
            snapshot = replacement;
            draft = { ...replacement.draft };
            onSaved(current.profiles);
        } catch (cause) { error = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
        finally { setBusy(false); }
    }
    async function save(event: SubmitEvent) {
        event.preventDefault();
        if (busy || !snapshot || !draft || validation || !dirty) return;
        if (onStage) {
            onStage(snapshot.id, { ...draft });
            dialog.close(); snapshot = undefined; draft = undefined;
            return;
        }
        setBusy(true); error = '';
        try {
            const result = await api<{ profiles: RemoteProfile[] }>(`/api/model-endpoints/${encodeURIComponent(snapshot.id)}/configuration`, {
                method: 'POST', body: JSON.stringify(endpointRequest(snapshot, draft)),
            });
            onSaved(result.profiles);
            dialog.close(); snapshot = undefined; draft = undefined;
        } catch (cause) { error = cause instanceof ModelValidationError ? cause.issue : errorMessage(cause); }
        finally { setBusy(false); }
    }
    function beforeUnload(event: BeforeUnloadEvent) {
        if (!acceptsWorkspaceUnload(event)) return; if (dirty) { event.preventDefault(); event.returnValue = ''; } }
</script>

<svelte:window onbeforeunload={beforeUnload} />
<ConfirmationDialog bind:this={confirmation} />
<dialog use:ownDialog bind:this={dialog} oncancel={close} onkeydown={event => { if (event.key === 'Escape' && !event.defaultPrevented) { event.stopPropagation(); void close(event); } }} aria-labelledby="endpoint-editor-title">
    {#if snapshot && draft}<form onsubmit={save} aria-busy={busy}>
        <header><h2 id="endpoint-editor-title">{t($locale, 'remote.edit_endpoint')}</h2><p class="muted">{t($locale, 'remote.endpoint_scope', { count: snapshot.profiles.length })}</p></header>
        {#if error}<div class="notice error" role="alert">{modelValidationText($locale, error)}<button type="button" disabled={busy} onclick={reload}>{t($locale, 'models.reload')}</button></div>{/if}
        <fieldset disabled={busy}>
            <label>{t($locale, 'remote.endpoint_name')}<input bind:value={draft.name} maxlength="512" required /></label>
            <label>{t($locale, 'remote.base_url')}<input type="url" bind:value={draft.baseURL} placeholder="https://api.example.com/v1" autocomplete="off" required /></label>
            <label>{t($locale, 'remote.protocol')}<select bind:value={draft.wireProtocol}>
                <option value="chat_completions">Chat Completions</option><option value="responses">Responses</option><option value="anthropic">Anthropic Messages</option>
            </select></label>
            <label class="toggle"><input type="checkbox" bind:checked={draft.enabled} />{t($locale, 'remote.enable_endpoint')}</label>
        </fieldset>
        <p class="muted">{t($locale, 'remote.endpoint_key_hint')}</p>
        {#if validation}<p class="error" role="alert">{modelValidationText($locale, validation)}</p>{/if}
        <div class="actions"><button type="button" disabled={busy} onclick={() => close()}>{t($locale, 'shell.cancel')}</button><button class="primary" disabled={busy || !dirty || !!validation}>{t($locale, busy ? 'models.saving' : 'remote.save_endpoint')}</button></div>
    </form>{/if}
</dialog>
<style>fieldset { display: grid; gap: 16px; border: 0; padding: 0; min-width: 0; } .toggle { display: flex; align-items: center; gap: 8px; } .toggle input { width: auto; } .notice button { display: block; margin-top: 8px; }</style>
