<script lang="ts">
    import type { TimelineMessage } from '../lib/chatTimeline';
    import { timelineToolPresentation } from '../lib/chatTimeline';
    import { timelineReference, toolArtifacts } from '../lib/chatReferences';
    import ChatReferenceCard from './ChatReferenceCard.svelte';
    import { locale, t } from '../lib/i18n';
    let {messages}: {messages: TimelineMessage[]} = $props();
    // These are committed transcript references, not invented artifact versions.
    const references = $derived(messages.flatMap(message => message.parts.flatMap(part => {
        const reference = timelineReference(part);
        return [...(reference?.kind === 'attachment' ? [reference] : []), ...toolArtifacts(timelineToolPresentation(part)?.artifacts)];
    })));
</script>
<section aria-label={t($locale, 'chatInspector.artifacts')}>
    {#if references.length}{#each references as reference}<ChatReferenceCard {reference} />{/each}
    {:else}<p class="muted">{t($locale, 'chatInspector.noArtifacts')}</p>{/if}
</section>
<style>.muted { color: var(--muted); font-size: .875rem; line-height: 1.6; }</style>
