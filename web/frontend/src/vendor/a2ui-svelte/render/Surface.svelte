<script lang="ts">
    import { untrack } from 'svelte';
    import { locale, t } from '../../../lib/i18n';
    import type { UiSnapshot, UiInteraction, UiActionRef, UiEdit, UiSubmissionOutcome } from '../../../lib/generativeUi';
    import { readUiDraftState, saveUiDraftState, restoreUiAttempt, persistUiAttempt, retireUiAttempt } from '../../../lib/uiDraftSession';
    import { uiIdentityKey, previewUiSurface, validateUiDraftPreview } from '../../../lib/generativeUi';
    import type { UiDraftPreviewInput, UiDraftPreview } from '../../../lib/generativeUi';
    import { provideProjection } from './context';
    import Node from './Node.svelte';
    import '../catalog/basic/theme.css';
    let { snapshot, oninteraction, onrefresh, onpreview = previewUiSurface, readOnly = false }: {
        snapshot: UiSnapshot;
        readOnly?: boolean;
        onpreview?: (input: UiDraftPreviewInput, signal?: AbortSignal) => Promise<UiDraftPreview>;
        onrefresh: () => Promise<void>;
        oninteraction: (input: UiInteraction, requestId: string, submittedAtMs: number) => Promise<UiSubmissionOutcome>;
    } = $props();
    const mountedIdentity = untrack(() => uiIdentityKey(snapshot.identity));
    const rememberEdits = untrack(() => !readOnly && snapshot.phase !== 'read_only' && snapshot.phase !== 'deleted');
    const restored = rememberEdits ? readUiDraftState(mountedIdentity) : undefined;
    let drafts = $state<Record<string, UiEdit>>(restored?.drafts ?? {});
    let draftRevision = $state<number | null>(restored?.revision ?? null);
    let pending = $state(false);
    let restoring = $state(rememberEdits);
    let composing = $state(false);
    let failure = $state('');
    let draftGeneration = $state(1);
    let validatedGeneration = $state(0);
    let previewSnapshot = $state<UiSnapshot | null>(null);
    let previewFailure = $state('');
    const hasDrafts = $derived(Object.keys(drafts).length > 0);
    const staleDraft = $derived(hasDrafts && draftRevision !== snapshot.revision);
    const previewReady = () => !hasDrafts || validatedGeneration === draftGeneration
        && previewSnapshot?.revision === snapshot.revision && draftRevision === snapshot.revision;
    const renderedSnapshot = () => hasDrafts && previewReady() && previewSnapshot ? previewSnapshot : snapshot;
    // Drafts remain local. Only Core evaluates checks; no action or edit is
    // committed by this cancellable projection request.
    $effect(() => {
        const generation = draftGeneration;
        const revision = snapshot.revision;
        const identity = snapshot.identity;
        const edits = Object.values(drafts);
        if (!edits.length || composing || restoring || pending || unresolved || snapshot.phase !== 'ready'
            || draftRevision !== revision || uiIdentityKey(identity) !== mountedIdentity) return;
        const controller = new AbortController();
        let deadline: ReturnType<typeof setTimeout> | undefined;
        const timer = setTimeout(() => {
            previewFailure = '';
            deadline = setTimeout(() => {
                controller.abort();
                if (draftGeneration === generation && active()) {
                    validatedGeneration = 0;
                    previewFailure = '草稿校验超时，请重试。';
                }
            }, 15000);
            const input = {identity, expected_surface_revision: revision, edits, draft_generation: generation};
            void onpreview(input, controller.signal).then(result => {
                const projected = validateUiDraftPreview(input, result);
                if (projected.catalog_digest !== snapshot.catalog_digest || projected.catalog_id !== snapshot.catalog_id) {
                    throw new Error('UI draft preview catalog mismatch');
                }
                if (!controller.signal.aborted && draftGeneration === generation && snapshot.revision === revision
                    && active() && !composing) {
                    previewSnapshot = projected;
                    validatedGeneration = generation;
                }
            }).catch(error => {
                if (!controller.signal.aborted && draftGeneration === generation) {
                    previewFailure = error instanceof Error ? error.message : String(error);
                    validatedGeneration = 0;
                }
            }).finally(() => clearTimeout(deadline));
        }, 150);
        return () => { clearTimeout(timer); clearTimeout(deadline); controller.abort(); };
    });
    let unresolved = $state<{ input: UiInteraction; requestId: string; submittedAtMs: number } | null>(restored?.unresolved ?? null);
    // Surface is keyed by the full authority identity at its mounting boundary.
    $effect(() => {
        if (rememberEdits) saveUiDraftState(mountedIdentity, {drafts, revision: draftRevision, unresolved});
    });
    $effect(() => {
        if (!rememberEdits) return;
        let mounted = true;
        void restoreUiAttempt(mountedIdentity).then(value => {
            if (!mounted) return;
            if (value) unresolved = value;
            restoring = false;
        }).catch(error => { if (mounted) failure = error instanceof Error ? error.message : String(error); });
        return () => { mounted = false; };
    });
    const active = () => uiIdentityKey(snapshot.identity) === mountedIdentity;
    const disabled = () => readOnly || restoring || pending || unresolved !== null || !active() || snapshot.phase !== 'ready';
    const editKey = (id: string, binding: string) => JSON.stringify([id, binding]);
    provideProjection({
        snapshot: renderedSnapshot,
        actionsDisabled: () => composing || !previewReady(),
        disabled,
        value: (id, binding, committed) => {
            const draft = drafts[editKey(id, binding)];
            return draft?.type === 'set' ? draft.value : committed;
        },
        edit: (id, binding, value) => {
            if (disabled()) return;
            draftGeneration++;
            validatedGeneration = 0;
            draftRevision ??= snapshot.revision;
            drafts = { ...drafts, [editKey(id, binding)]: { type: 'set', instance_id: id, binding, value } };
        },
        dispatch: action => { void submit(action); }
    });
    function revalidateDrafts() {
        if (disabled() || composing) return;
        const valid = Object.values(drafts).every(edit =>
            Object.hasOwn(snapshot.nodes[edit.instance_id]?.bindings ?? {}, edit.binding));
        if (!valid) {
            previewFailure = '表单已更新，部分编辑项不再存在。你的编辑已保留，请确认后放弃编辑。';
            return;
        }
        // The user explicitly confirms the new revision. Keep all values;
        // only Core may decide whether they are valid for this version.
        draftRevision = snapshot.revision;
        previewFailure = '';
        previewSnapshot = null;
        validatedGeneration = 0;
        draftGeneration++;
    }
    function discardDrafts() {
        if (pending || unresolved || !active()) return;
        drafts = {};
        draftRevision = null;
        failure = '';
        previewFailure = '';
        previewSnapshot = null;
        draftGeneration++;
        validatedGeneration = 0;
    }
    async function refresh() {
        if (pending || unresolved || !active()) return;
        pending = true;
        try { await onrefresh(); }
        catch (error) { failure = error instanceof Error ? error.message : String(error); }
        finally { pending = false; }
    }
    async function submit(action: UiActionRef) {
        if (disabled() || !previewReady()) return;
        if (composing) { failure = '请先完成当前输入法输入，再提交表单。'; return; }
        if (draftRevision !== null && draftRevision !== snapshot.revision) {
            failure = '表单已更新。你的编辑已保留，请刷新并确认后重新填写。';
            return;
        }
        const edits = Object.values(drafts);
        const input: UiInteraction = { identity: snapshot.identity,
            expected_surface_revision: draftRevision ?? snapshot.revision, edits, action };
        await deliver({ input, requestId: crypto.randomUUID(), submittedAtMs: Date.now() });
    }
    async function deliver(attempt: { input: UiInteraction; requestId: string; submittedAtMs: number }) {
        if (pending || !active()) return;
        const identity = uiIdentityKey(attempt.input.identity);
        pending = true;
        failure = '';
        unresolved = attempt;
        if (rememberEdits) saveUiDraftState(mountedIdentity, {drafts, revision: draftRevision, unresolved});
        try {
            await persistUiAttempt(mountedIdentity, attempt);
            const outcome = await oninteraction(attempt.input, attempt.requestId, attempt.submittedAtMs);
            if (identity !== uiIdentityKey(snapshot.identity) || !active()) return;
            if (outcome.status !== 'unknown') await retireUiAttempt(mountedIdentity, attempt);
            if (outcome.status === 'committed') {
                drafts = {};
                draftRevision = null;
                unresolved = null;
            } else {
                failure = outcome.message;
                unresolved = outcome.status === 'unknown' ? attempt : null;
            }
        } catch (error) {
            if (identity === uiIdentityKey(snapshot.identity) && active()) {
                failure = error instanceof Error ? error.message : String(error);
                // Lost acknowledgement: preserve the identical request and
                // payload until its receipt is reconciled, never resubmit anew.
                unresolved = attempt;
            }
        } finally {
            pending = false;
            if (rememberEdits) saveUiDraftState(mountedIdentity, {drafts, revision: draftRevision, unresolved});
        }
    }

</script>
<section oncompositionstart={() => composing = true} oncompositionend={() => composing = false} onfocusout={() => composing = false} class="nola-generative-surface" aria-busy={pending || snapshot.phase === 'pending'} data-ui-surface={snapshot.identity.surface_id}>
    {#if snapshot.phase === 'pending'}
        <p role="status">{t($locale, 'a2ui.pending')}</p>
    {:else if snapshot.phase === 'deleted'}
        <p role="status">{t($locale, 'a2ui.deleted')}</p>
    {:else if snapshot.root_instance_id}
        <Node id={snapshot.root_instance_id} />
    {:else if snapshot.summary}<p>{snapshot.summary}</p>{/if}
    {#if failure}<p role="alert">{failure}</p>{/if}
    {#if staleDraft}<p role="status">表单已更新。你的编辑已保留，请确认后重新校验。</p>{/if}
    {#if previewFailure}<p role="alert">草稿校验未完成：{previewFailure}</p>{/if}
    {#if previewFailure || staleDraft}
        <button type="button" disabled={disabled() || composing} onclick={revalidateDrafts}>重新校验</button>
    {/if}
    {#if !unresolved && (failure || draftRevision !== null && draftRevision !== snapshot.revision)}
        <div class="recovery-actions">
            <button type="button" disabled={pending} onclick={() => void refresh()}>刷新表单（保留编辑）</button>
            {#if Object.keys(drafts).length}
                <button type="button" disabled={pending} onclick={discardDrafts}>放弃编辑，使用当前值</button>
            {/if}
        </div>
    {/if}
    {#if !readOnly && unresolved && (snapshot.phase === 'ready' || snapshot.phase === 'pending')}<button type="button" disabled={pending} onclick={() => { if (unresolved) void deliver(unresolved); }}>查询提交结果</button>{/if}
</section>
<style>
    .nola-generative-surface {
        min-width: 0;
        max-width: 100%;
        --a2ui-control-height: 44px;
        --a2ui-color-surface: var(--surface);
        --a2ui-color-surface-raised: var(--panel);
        --a2ui-color-text: var(--content);
        --a2ui-color-text-muted: var(--muted);
        --a2ui-color-primary: var(--primary);
        --a2ui-color-on-primary: var(--on-primary);
        --a2ui-color-border: var(--outline);
        --a2ui-color-danger: var(--danger);
    }
</style>
