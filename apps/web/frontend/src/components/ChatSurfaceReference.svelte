<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { restoreUiAttempt, retireUiAttempt } from '../lib/uiDraftSession';
    import type { UiPendingAttempt } from '../lib/uiDraftSession';
    import { uiIdentityKey } from '../lib/generativeUi';
    import Surface from '../vendor/a2ui-svelte/render/Surface.svelte';
    import { readUiSurfaceRevision } from '../lib/generativeUi';
    import type { UiSurfaceRevisionRef, UiSnapshot, UiInteraction, UiSubmissionOutcome } from '../lib/generativeUi';
    let { reference, snapshot, summary, oninteraction, onrefresh, onpreview, recoverable = true, readOnly = false }: {
        onpreview?: (input: import('../lib/generativeUi').UiDraftPreviewInput, signal?: AbortSignal) => Promise<import('../lib/generativeUi').UiDraftPreview>;
        reference: UiSurfaceRevisionRef; snapshot?: UiSnapshot; summary: string; recoverable?: boolean; readOnly?: boolean;
        oninteraction: (input: UiInteraction, id: string, at: number) => Promise<UiSubmissionOutcome>;
        onrefresh: () => Promise<void>;
    } = $props();
    const activeCurrent = $derived(snapshot?.phase === 'ready' || snapshot?.phase === 'pending');
    let pendingAttempt = $state<UiPendingAttempt | null>(null);
    let recoveryFailure = $state('');
    let recovering = $state(false);
    $effect(() => {
        if (readOnly || activeCurrent || !recoverable) return;
        const identity = uiIdentityKey(reference.identity);
        let mounted = true;
        void restoreUiAttempt(identity).then(value => { if (mounted) pendingAttempt = value; })
            .catch(error => { if (mounted) recoveryFailure = String(error); });
        return () => { mounted = false; };
    });
    async function reconcile() {
        const attempt = pendingAttempt;
        if (readOnly || !attempt || recovering) return;
        recovering = true;
        try {
            const result = await oninteraction(attempt.input, attempt.requestId, attempt.submittedAtMs);
            if (result.status !== 'unknown') {
                await retireUiAttempt(uiIdentityKey(attempt.input.identity), attempt);
                pendingAttempt = null;
            }
            recoveryFailure = result.status === 'committed' ? '' : result.message;
        } catch (error) { recoveryFailure = String(error); }
        finally { recovering = false; }
    }
    let expanded = $state(false);
    let historical = $state<UiSnapshot | null>(null);
    let failure = $state('');
    let loading = $state(false);
    $effect(() => {
        if (!expanded) return;
        const controller = new AbortController();
        historical = null; failure = ''; loading = true;
        void readUiSurfaceRevision(reference, controller.signal).then(value => {
            if (!controller.signal.aborted) {
                historical = value;
                if (!value) failure = t($locale, 'activity.surfaceUnavailable');
            }
        }).catch(error => {
            if (!controller.signal.aborted) failure = error instanceof Error ? error.message : String(error);
        }).finally(() => { if (!controller.signal.aborted) loading = false; });
        return () => controller.abort();
    });
</script>
<div class="surface-reference">
    {#if snapshot}{#key uiIdentityKey(snapshot.identity)}<Surface {readOnly} {snapshot} {oninteraction} {onrefresh} {onpreview} />{/key}
    {:else if summary}<p>{summary}</p>{/if}
    {#if !readOnly && !activeCurrent && pendingAttempt}<button type="button" disabled={recovering} onclick={() => void reconcile()}>{t($locale, 'activity.surfacePending')}</button>{/if}
    {#if recoveryFailure}<p role="alert">{recoveryFailure}</p>{/if}
    <button type="button" aria-expanded={expanded} onclick={() => expanded = !expanded}>{t($locale, 'activity.surfaceVersion', {revision: reference.revision})}</button>
    {#if expanded}
        <section aria-label={t($locale, 'activity.surfaceHistory')} aria-busy={loading}>
            <p class="historical-label">{t($locale, 'activity.surfaceReadOnly')}</p>
            {#if loading}<p role="status">{t($locale, 'activity.loading_detail')}</p>{/if}
            {#if failure}<p role="alert">{failure}</p>{/if}
            {#if historical}{#key uiIdentityKey(historical.identity)}<Surface readOnly snapshot={historical} onrefresh={async () => {}}
                oninteraction={async () => ({status: 'rejected', message: t($locale, 'activity.surfaceNoEdit')})} />{/key}{/if}
        </section>
    {/if}
</div>
<style>
    .surface-reference { min-width: 0; }
    button { min-height: 44px; padding: .5rem .75rem; }
    section { border-inline-start: 2px solid var(--border-color, #8885); padding-inline-start: .75rem; }
    .historical-label { opacity: .65; font-size: .85rem; }
</style>
