<script lang="ts">
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onMount, untrack } from 'svelte';
    import { api, ApiError } from '../lib/api';
    import { committedConversationConfiguration } from '../lib/conversationConfigurationCommit';
    import { locale, t } from '../lib/i18n';
    import type { ChatConversation } from '../lib/chatConversations';
    import { conversationUrl, parseChatConversation } from '../lib/chatConversations';
    import type { RemoteProfile } from '../lib/modelProfiles';
    import { commitSessionModel, recoverSessionModel } from '../lib/sessionModelCommit';
    import { canSelectProfile } from '../lib/modelProfiles';
    import { sessionConfigurationPatchConfirmed, sessionPermissionApplicationStatus } from '../lib/chatSessionSettings';
    import { localSessionProfile, parseSessionConfiguration, parseSessionLocalModels, parseSessionProfiles, permissionPatchConfirmed, sessionConfigurationPatch, sessionDraft, sessionDraftError, sessionModelKey, sessionModelSelectionConfirmed, sessionPatchHasChanges, sessionPermissionPatch, sessionReasoningOptions, selectableSessionProfile } from '../lib/chatSessionSettings';
    import type { PermissionChoice, SessionConfiguration, SessionDraft, SessionLocalModel, SessionNumberField, SessionPermissionModeState, SessionReasoningQuickState, SessionModelChoiceState } from '../lib/chatSessionSettings';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';

    interface Props { conversation: ChatConversation; generation: number; busy?: boolean; onconversation?: (conversation: ChatConversation) => void; onmodelstate?: (state: SessionModelChoiceState) => void; onmodelname?: (name: string) => void; onpermissionstate?: (state: SessionPermissionModeState) => void; onreasoningstate?: (state: SessionReasoningQuickState) => void }
    let { conversation, generation, busy = false, onconversation, onmodelstate, onmodelname, onpermissionstate, onreasoningstate }: Props = $props();
    let confirmation: ConfirmationHandle;
    let snapshot = $state<SessionConfiguration | null>(null);
    let draft = $state<SessionDraft | null>(null);
    let profiles = $state<RemoteProfile[]>([]);
    let localModels = $state<SessionLocalModel[]>([]);
    let loading = $state(false);
    let saving = $state(false);
    let error = $state('');
    let modelError = $state('');
    let notice = $state('');
    let catalogError = $state(false);
    let conflict = $state(false);
    let committedPermissionRevision = $state(0);
    let mounted = false;
    let disposed = false;
    let version = 0;
    let catalogVersion = 0;
    let controller: AbortController | undefined;
    const modelKey = $derived(sessionModelKey(conversation));
    const profile = $derived(localSessionProfile(conversation, localModels) ?? selectableSessionProfile(conversation, profiles));
    $effect(() => { const name = profile?.displayName ?? ''; untrack(() => onmodelname?.(name)); });
    const editKey = $derived(profile ? modelKey : null);
    const override = $derived(editKey && snapshot ? snapshot.modelOverrides[editKey] ?? {} : {});
    const reasoningOptions = $derived(profile ? sessionReasoningOptions(profile, override) : []);
    const validation = $derived(draft ? sessionDraftError(draft, reasoningOptions) : null);
    const original = $derived(snapshot ? sessionDraft(snapshot, editKey) : null);
    const changed = $derived(!!draft && !!original && JSON.stringify(draft) !== JSON.stringify(original));
    const locked = $derived(saving || loading);
    const observedRunID = $derived(snapshot?.activeRunID ?? null);
    const permissionState: SessionPermissionModeState = $derived({
        choice: draft?.permission ?? 'cautious', savedChoice: snapshot?.permissionProfile ?? 'cautious', ready: !!snapshot && !!draft,
        busy: locked, saving, conflict, canSave: !!snapshot && !!draft && !locked && !conflict && draft.permission !== (snapshot.permissionProfile ?? 'cautious'),
        errorKey: error, noticeKey: notice,
        applicationStatus: !snapshot ? undefined : sessionPermissionApplicationStatus(snapshot, draft?.permission ?? null, saving, !!error, busy, committedPermissionRevision),
    });
    $effect(() => { const state = permissionState; untrack(() => onpermissionstate?.(state)); });
    const reasoningState: SessionReasoningQuickState = $derived({
        preference: draft?.reasoningPreference ?? 'inherit',
        options: reasoningOptions,
        ready: !!snapshot && !!draft && !!profile,
        busy: locked || conflict || changed,
        errorKey: error,
    });
    $effect(() => { const state = reasoningState; untrack(() => onreasoningstate?.(state)); });
    const modelChoice = $derived(conversation.selectedModelSource === 'cloud' ? `cloud:${conversation.selectedModelId}` : conversation.selectedModelSource === 'local' ? `local:${conversation.selectedModelId}` : 'auto');
    const modelState: SessionModelChoiceState = $derived({
        choice: modelChoice, disabled: locked || conflict || !snapshot || catalogError || changed || !onconversation,
        errorKey: catalogError ? 'chatSession.catalogFailed' : modelError,
        options: [
            ...profiles.filter(canSelectProfile).map(model => ({ value: `cloud:${model.id}`, name: model.displayName || model.modelName, detail: model.endpoint.name || model.endpoint.baseURL, local: false })),
            ...localModels.filter(model => model.installed).map(model => ({ value: `local:${model.id}`, name: model.name, detail: '', local: true })),
        ],
    });
    $effect(() => { const state = modelState; untrack(() => onmodelstate?.(state)); });
    const fields: { key: SessionNumberField; label: string }[] = [
        { key: 'temperature', label: 'temperature' },
        { key: 'topP', label: 'topP' },
        { key: 'maxOutputTokens', label: 'maxOutputTokens' },
        { key: 'maxContextLength', label: 'maxContextLength' },
    ];
    const sameOwner = (id: string, epoch: number) => !disposed && conversation.conversationId === id && generation === epoch;

    async function load(discardDraft = false) {
        const id = conversation.conversationId, epoch = generation, request = ++version;
        controller?.abort(); const next = new AbortController(); controller = next;
        loading = true; error = ''; modelError = ''; notice = '';
        try {
            const [configuration, catalog, localCatalog] = await Promise.allSettled([
                api<unknown>(`${conversationUrl(id)}/configuration`, { signal: next.signal }).then(parseSessionConfiguration),
                api<unknown>('/api/model-profiles', { signal: next.signal }).then(parseSessionProfiles),
                api<unknown>('/api/local-models', { signal: next.signal }).then(parseSessionLocalModels),
            ]);
            if (!sameOwner(id, epoch) || request !== version) return;
            catalogError = catalog.status === 'rejected' || localCatalog.status === 'rejected';
            profiles = catalog.status === 'fulfilled' ? catalog.value : [];
            localModels = localCatalog.status === 'fulfilled' ? localCatalog.value : [];
            if (configuration.status === 'rejected') throw configuration.reason;
            const parsed = configuration.value;
            snapshot = parsed;
            if (discardDraft || !draft) draft = sessionDraft(parsed,
                localSessionProfile(conversation, localModels) || selectableSessionProfile(conversation, profiles) ? sessionModelKey(conversation) : null);
            conflict = false;
        } catch {
            if (sameOwner(id, epoch) && request === version && !next.signal.aborted) error = 'chatSession.loadFailed';
        } finally {
            if (sameOwner(id, epoch) && request === version) loading = false;
        }
    }
    $effect(() => {
        conversation.conversationId; conversation.selectedModelSource; conversation.selectedModelId; generation;
        if (mounted) untrack(() => { snapshot = null; draft = null; saving = false; profiles = []; localModels = []; catalogError = false; conflict = false; committedPermissionRevision = 0; void load(); });
    });
    // Depend on the polling boundary, not the snapshot replaced by each ACK.
    // Tracking snapshot here restarts the zero-delay timer on every response.
    const shouldPollPermission = $derived(snapshot !== null && (busy || !!observedRunID || snapshot.permissionState === undefined));
    $effect(() => {
        const owner = conversation.conversationId, epoch = generation;
        if (!shouldPollPermission) return;
        let retired = false;
        const polling = new AbortController();
        let timer: ReturnType<typeof setTimeout>;
        const refreshPolicy = async () => {
            try {
                const minimum = committedPermissionRevision || snapshot?.configurationRevision || 0;
                const current = parseSessionConfiguration(await api(`${conversationUrl(owner)}/configuration${minimum ? `?minimum_policy_revision=${minimum}` : ''}`, { signal: polling.signal }));
                if (!retired && sameOwner(owner, epoch) && snapshot && current.configurationRevision >= snapshot.configurationRevision) {
                    // A read observes ACKs but never rebases or replays a draft.
                    snapshot = { ...snapshot, activeRunID: current.activeRunID, activePermissionPolicy: current.activePermissionPolicy, permissionState: current.permissionState };
                }
            } catch { /* Saved configuration remains saved; missing ACK never becomes active. */ }
            if (!retired) timer = setTimeout(refreshPolicy, 750);
        };
        timer = setTimeout(refreshPolicy, 0);
        return () => { retired = true; clearTimeout(timer); polling.abort(); };
    });
    onMount(() => { mounted = true; void load(); return () => { disposed = true; controller?.abort(); }; });

    // Opening the composer menu refreshes only its read-only catalog. Parameter
    // drafts, permission snapshots and pending mutations are left untouched.
    export async function refreshModelCatalog() {
        const id = conversation.conversationId, epoch = generation, request = ++catalogVersion;
        const results = await Promise.allSettled([
            api<unknown>('/api/model-profiles').then(parseSessionProfiles),
            api<unknown>('/api/local-models').then(parseSessionLocalModels),
        ]);
        if (!sameOwner(id, epoch) || request !== catalogVersion) return;
        catalogError = results.some(result => result.status === 'rejected');
        if (results[0].status === 'fulfilled') profiles = results[0].value;
        if (results[1].status === 'fulfilled') localModels = results[1].value;
    }

    export async function reloadSettings() {
        if (locked) return;
        if (changed && !await confirmation.ask({ title: t($locale, 'chatSession.reloadTitle'), description: t($locale, 'chatSession.reloadBody'), confirmLabel: t($locale, 'chatSession.reloadConfirm'), destructive: true })) return;
        if (modelError) {
            const id = conversation.conversationId, epoch = generation;
            loading = true;
            try {
                const recovered = await recoverSessionModel(id, api, () => sameOwner(id, epoch));
                if (!recovered) return;
                onconversation?.(recovered);
            } catch { if (sameOwner(id, epoch)) modelError = 'chatSession.selectFailed'; return; }
            finally { if (sameOwner(id, epoch)) loading = false; }
            if (!sameOwner(id, epoch)) return;
        }
        draft = null;
        await load(true);
    }
    export function setPermission(choice: PermissionChoice): void {
        if (!draft || !snapshot || locked || conflict || !['cautious', 'allow_all'].includes(choice)) return;
        draft.permission = choice;
        error = ''; notice = '';
        void savePermission();
    }
    export async function savePermission(): Promise<void> {
        if (locked || conflict || !snapshot || !draft) return;
        const id = conversation.conversationId, epoch = generation;
        let patch: Record<string, unknown> | null;
        try { patch = sessionPermissionPatch(snapshot, draft.permission); }
        catch { error = 'chatSession.invalidPermission'; return; }
        if (!patch) return;
        saving = true; error = ''; notice = '';
        try {
            if (draft.permission === 'allow_all' && snapshot.permissionProfile !== 'allow_all') {
                const accepted = await confirmation.ask({ title: t($locale, 'chatPermissionMode.allowAllTitle'),
                    description: t($locale, 'chatPermissionMode.allowAllBody'), confirmLabel: t($locale, 'chatPermissionMode.allowAllConfirm'), destructive: true });
                if (!sameOwner(id, epoch)) return;
                if (!accepted) { draft.permission = snapshot.permissionProfile ?? 'cautious'; return; }
            }
            const saved = parseSessionConfiguration(committedConversationConfiguration(await api(`${conversationUrl(id)}/configuration`, { method: 'PATCH', body: JSON.stringify(patch) })));
            if (!sameOwner(id, epoch)) return;
            if (!permissionPatchConfirmed(saved, patch)) throw new Error('chatSession.invalidResponse');
            snapshot = saved;
            committedPermissionRevision = saved.configurationRevision;
            // Keep pending model-field edits. The fresh revision is shared by
            // both surfaces; permission-only save does not submit those edits.
            notice = 'chatSession.saved'; conflict = false;
        } catch (cause) {
            if (sameOwner(id, epoch)) {
                const stale = cause instanceof ApiError && cause.status === 409;
                conflict = true;
                error = stale ? 'chatSession.conflict' : 'chatSession.saveFailed';
            }
        } finally { if (sameOwner(id, epoch)) saving = false; }
    }
    export async function selectReasoningPreference(preference: string): Promise<void> {
        if (!snapshot || !draft || !profile || locked || conflict || changed
            || !['inherit', 'automatic', ...reasoningOptions].includes(preference)
            || preference === draft.reasoningPreference) return;
        const id = conversation.conversationId, epoch = generation;
        const nextDraft: SessionDraft = { ...draft, reasoningPreference: preference };
        let patch: Record<string, unknown>;
        try { patch = sessionConfigurationPatch(snapshot, editKey, nextDraft, reasoningOptions); }
        catch { error = 'chatSession.invalidModel'; return; }
        if (!sessionPatchHasChanges(patch)) return;
        saving = true; error = ''; notice = '';
        try {
            const saved = parseSessionConfiguration(committedConversationConfiguration(await api(`${conversationUrl(id)}/configuration`, {
                method: 'PATCH', body: JSON.stringify(patch),
            })));
            if (!sameOwner(id, epoch)) return;
            if (!sessionConfigurationPatchConfirmed(saved, patch)) throw new Error('chatSession.invalidResponse');
            snapshot = saved; draft = sessionDraft(saved, editKey); notice = 'chatSession.saved'; conflict = false;
        } catch (cause) {
            if (sameOwner(id, epoch)) {
                const stale = cause instanceof ApiError && cause.status === 409;
                conflict = true;
                error = stale ? 'chatSession.conflict' : 'chatSession.saveFailed';
            }
        } finally { if (sameOwner(id, epoch)) saving = false; }
    }
    async function chooseModel(event: Event) {
        const select = event.currentTarget as HTMLSelectElement;
        const choice = select.value;
        select.value = modelChoice;
        await selectModel(choice);
    }
    export async function selectModel(choice: string) {
        if (modelState.disabled || !onconversation || choice === modelChoice && (choice !== 'auto' || !!conversation.selectedModelId || !modelState.options.length)) return;
        const id = conversation.conversationId, epoch = generation;
        const currentState = modelState;
        saving = true; modelError = ''; notice = '';
        try {
            const result = await commitSessionModel({ conversation, snapshot, state: currentState, choice,
                request: api, isCurrent: () => sameOwner(id, epoch) });
            if (!result) return;
            onconversation(result);
            snapshot = null; draft = null;
        } catch (cause) {
            if (sameOwner(id, epoch)) {
                conflict = true;
                modelError = cause instanceof ApiError && cause.status === 409 ? 'chatSession.conflict' : 'chatSession.selectFailed';
            }
        } finally { if (sameOwner(id, epoch)) saving = false; }
    }

    async function save(event: SubmitEvent) {
        event.preventDefault();
        if (locked || conflict || !snapshot || !draft || validation || !changed) return;
        const id = conversation.conversationId, epoch = generation;
        let patch: Record<string, unknown>;
        try { patch = sessionConfigurationPatch(snapshot, editKey, draft, reasoningOptions); }
        catch { error = 'chatSession.invalidModel'; return; }
        if (!sessionPatchHasChanges(patch)) return;
        saving = true; error = ''; notice = '';
        try {
            if (draft.permission === 'allow_all' && snapshot.permissionProfile !== 'allow_all') {
                const accepted = await confirmation.ask({ title: t($locale, 'chatPermissionMode.allowAllTitle'),
                    description: t($locale, 'chatPermissionMode.allowAllBody'), confirmLabel: t($locale, 'chatPermissionMode.allowAllConfirm'), destructive: true });
                if (!sameOwner(id, epoch)) return;
                if (!accepted) { draft.permission = snapshot.permissionProfile ?? 'cautious'; return; }
            }
            const saved = parseSessionConfiguration(committedConversationConfiguration(await api(`${conversationUrl(id)}/configuration`, { method: 'PATCH', body: JSON.stringify(patch) })));
            if (!sameOwner(id, epoch)) return;
            if (!sessionConfigurationPatchConfirmed(saved, patch)) throw new Error('chatSession.invalidResponse');
            snapshot = saved; draft = sessionDraft(saved, editKey); notice = 'chatSession.saved'; conflict = false;
        } catch (cause) {
            if (sameOwner(id, epoch)) {
                const stale = cause instanceof ApiError && cause.status === 409;
                conflict = true;
                error = stale ? 'chatSession.conflict' : 'chatSession.saveFailed';
            }
        } finally { if (sameOwner(id, epoch)) saving = false; }
    }
    function reasoningLabel(value: string) { return t($locale, `chatSession.${value}`); }
    function defaultValue(field: SessionNumberField): string {
        const value = profile?.parameters?.[field];
        return t($locale, 'chatSession.inheritedValue', { value: typeof value === 'number' ? value : '—' });
    }
</script>

<ConfirmationDialog bind:this={confirmation} />
<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (changed || saving || conflict) { event.preventDefault(); event.returnValue = ''; } }} />
<section class="session-settings" aria-label={t($locale, 'chatSession.title')}>
    <header><h2>{t($locale, 'chatSession.title')}</h2><p class="muted">{t($locale, 'chatSession.scope')}</p></header>
    {#if error}<p class="error" role="alert">{t($locale, error)} <button type="button" onclick={reloadSettings} disabled={locked}>{t($locale, 'chatSession.reload')}</button></p>{/if}
    {#if modelError}<p class="error" role="alert">{t($locale, modelError)} <button type="button" onclick={reloadSettings} disabled={locked}>{t($locale, 'chatSession.reload')}</button></p>{/if}
    {#if catalogError}<p class="error" role="alert">{t($locale, 'chatSession.catalogFailed')} <button onclick={reloadSettings} disabled={locked}>{t($locale, 'chatSession.reload')}</button></p>{/if}
    {#if notice}<p class="notice" role="status">{t($locale, notice)}</p>{/if}
    {#if busy}<p class="muted">{t($locale, 'chatPermissionMode.nextMessage')}</p>{/if}
    <label class="field">{t($locale, 'chatSession.model')}
        <select value={modelChoice} onchange={chooseModel} disabled={modelState.disabled}>
            <option value="auto" disabled={!modelState.options.length}>{t($locale, modelState.options.length ? 'chatSession.autoModel' : 'chatSession.invalidModel')}</option>
            {#if modelChoice.startsWith('local:') && !localModels.some(model => model.installed && `local:${model.id}` === modelChoice)}<option value={modelChoice}>{t($locale, 'chatSession.unavailableModel')}</option>{/if}
            {#if modelChoice.startsWith('cloud:') && !profiles.some(value => `cloud:${value.id}` === modelChoice)}<option value={modelChoice}>{t($locale, 'chatSession.unavailableModel')}</option>{/if}
            {#each profiles.filter(canSelectProfile) as model (model.id)}<option value={`cloud:${model.id}`}>{model.displayName || model.modelName} · {model.endpoint.name || model.endpoint.baseURL}</option>{/each}
            {#each localModels.filter(model => model.installed) as model (model.id)}<option value={`local:${model.id}`}>{model.name} · {t($locale, model.running ? 'models.running' : 'models.installed')}</option>{/each}
        </select>
    </label>
    <p class="muted">{t($locale, 'chatSession.modelScope')}</p>
    {#if loading}<p role="status">{t($locale, 'chatPage.loading')}</p>{/if}
    {#if snapshot && draft}
        <form onsubmit={save} aria-busy={saving}>
            <label class="field">{t($locale, 'chatSession.memory')}
                <select bind:value={draft.memory} disabled={locked || conflict} aria-describedby="session-memory-hint">
                    <option value="inherit">{t($locale, 'chatSession.inherit')}</option>
                    <option value="on">{t($locale, 'chatSession.memoryOn')}</option>
                    <option value="off">{t($locale, 'chatSession.memoryOff')}</option>
                </select>
            </label>
            <p class="muted" id="session-memory-hint">{t($locale, 'chatSession.memoryScope')}</p>
            <label class="field">{t($locale, 'chatSession.retrieval')}
                <select bind:value={draft.retrieval} disabled={locked || conflict} aria-describedby="session-retrieval-hint">
                    <option value="inherit">{t($locale, 'chatSession.inherit')}</option>
                    <option value="on">{t($locale, 'chatSession.memoryOn')}</option>
                    <option value="off">{t($locale, 'chatSession.memoryOff')}</option>
                </select>
            </label>
            <p class="muted" id="session-retrieval-hint">{t($locale, 'chatSession.retrievalScope')}</p>
            {#if modelKey && !profile && !catalogError}<p class="error" role="status">{t($locale, 'chatSession.unavailableModel')}</p>{/if}
            {#if profile}
                <label class="field">{t($locale, 'chatSession.reasoning')}
                    <select bind:value={draft.reasoningPreference} disabled={locked}>
                        <option value="inherit">{t($locale, 'chatSession.inherit')}</option><option value="automatic">{t($locale, 'chatSession.automatic')}</option>
                        {#each reasoningOptions as effort (effort)}<option value={effort}>{reasoningLabel(effort)}</option>{/each}
                        {#if !['inherit', 'automatic', ...reasoningOptions].includes(draft.reasoningPreference)}<option value={draft.reasoningPreference} disabled>{draft.reasoningPreference}</option>{/if}
                    </select>
                </label>
                <div class="numbers">
                    <label class="field">{t($locale, 'chatSession.seed')}
                        <select bind:value={draft.seedMode} disabled={locked}>
                            <option value="inherit">{t($locale, 'chatSession.inherit')}</option>
                            <option value="clear">{t($locale, 'chatSession.seedClear')}</option>
                            <option value="value">{t($locale, 'chatSession.seedValue')}</option>
                        </select>
                        {#if draft.seedMode === 'value'}
                            <input type="text" inputmode="text" bind:value={draft.seedValue} aria-label={t($locale, 'chatSession.seedValue')} disabled={locked} />
                        {/if}
                    </label>
                    {#each fields as field (field.key)}
                        <label class="field">{t($locale, `chatSession.${field.label}`)}
                            <input type="text" inputmode={field.key === 'temperature' || field.key === 'topP' ? 'decimal' : 'numeric'} bind:value={draft.numbers[field.key]} placeholder={defaultValue(field.key)} disabled={locked} />
                        </label>
                    {/each}
                </div>
            {/if}
            {#if validation}<p class="error" role="alert">{t($locale, validation)}</p>{/if}
            <div class="actions"><button type="button" onclick={reloadSettings} disabled={locked}>{t($locale, 'chatSession.reload')}</button><button class="primary" type="submit" disabled={locked || conflict || !changed || !!validation}>{t($locale, saving ? 'chatSession.saving' : 'chatSession.save')}</button></div>
        </form>
    {/if}
</section>
<style>
    .session-settings{display:grid;gap:12px;min-width:0}.session-settings h2{margin:0;font-size:1rem}.session-settings p{margin:0}.muted{font-size:.8125rem;color:var(--muted,#aab0bb);line-height:1.6}.field{display:grid;gap:8px;min-width:0;font-size:.8125rem;font-weight:600}.field input,.field select{width:100%;min-width:0}.numbers{display:grid;grid-template-columns:repeat(auto-fit,minmax(135px,1fr));gap:16px}.actions{display:flex;justify-content:flex-end;gap:8px;margin-top:14px}.error{color:var(--danger,#f28484);font-size:12px}.notice{color:var(--accent,#76c7ad);font-size:12px}
</style>
