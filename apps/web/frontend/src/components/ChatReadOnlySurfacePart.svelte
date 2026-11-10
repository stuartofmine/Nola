<script lang="ts">
    import { untrack } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { readUiSurfaceChanges, uiIdentityKey, uiSurfaceReference } from '../lib/generativeUi';
    import type { UiSnapshot } from '../lib/generativeUi';
    import type { TimelinePart } from '../lib/chatTimeline';
    import ChatSurfaceReference from './ChatSurfaceReference.svelte';

    // An observation-only context has no submission controller. It must never
    // borrow the parent conversation's model, queue, or interaction authority.
    let { part, hostId, conversationId, messageId, active }: {part: TimelinePart; hostId: string;
        conversationId: string; messageId: string; active: boolean} = $props();
    const reference = $derived(uiSurfaceReference(part.value, conversationId, messageId));
    const ownerKey = $derived(reference ? JSON.stringify([hostId, uiIdentityKey(reference.identity), reference.revision, part.value.kind]) : '');
    let snapshot = $state<UiSnapshot | undefined>();
    let failure = $state('');
    let sequence = 0;
    async function refresh(signal?: AbortSignal) {
        const selected = reference, key = ownerKey, request = ++sequence;
        if (!active || !hostId || !selected || part.value.kind !== 'ui_surface') { snapshot = undefined; return; }
        const previous = snapshot && uiIdentityKey(snapshot.identity) === uiIdentityKey(selected.identity) ? snapshot : undefined;
        try {
            const value = await readUiSurfaceChanges(selected.identity, previous, signal);
            if (!signal?.aborted && active && request === sequence && ownerKey === key) {
                snapshot = value ?? undefined; failure = '';
            }
        } catch (cause) {
            if (!signal?.aborted && active && request === sequence && ownerKey === key)
                failure = cause instanceof Error ? cause.message : String(cause);
        }
    }
    $effect(() => {
        ownerKey; active;
        snapshot = undefined; failure = '';
        const controller = new AbortController();
        void untrack(() => refresh(controller.signal));
        return () => { ++sequence; controller.abort(); };
    });
</script>

{#if reference}
    <p class="read-only-status">{t($locale, 'chatPage.surfaceReadOnly')}</p>
    <ChatSurfaceReference {reference} {snapshot} readOnly recoverable={false}
        summary={typeof part.value.summary === 'string' ? part.value.summary : ''}
        onrefresh={async () => { await refresh(); }}
        oninteraction={async () => ({status: 'rejected', message: t($locale, 'chatPage.surfaceReadOnly')})} />
    {#if failure}<p role="alert">{failure}</p>{/if}
{/if}

<style>.read-only-status { color: var(--muted); font-size: .8125rem; margin-block: 4px; }</style>
