<script lang="ts">
    import {locale, t} from '../lib/i18n';
    import type {TimelinePart} from '../lib/chatTimeline';
    import {bibliography} from '../lib/chatReferences';
    import ChatReferenceCard from './ChatReferenceCard.svelte';
    let {parts}: {parts: TimelinePart[]} = $props();
    const references = $derived(bibliography(parts));
    let expanded = $state(false);
</script>
{#if references.length}
<section class="bibliography" aria-label={t($locale, 'chatPage.sourcesTitle')}>
    <p>{t($locale, 'chatPage.sourcesTitle')} <span>{references.length}</span></p>
    {#each references.slice(0, expanded ? references.length : 3) as item, index (item.part.id)}
        <ChatReferenceCard reference={item.reference} compact number={index + 1} />
    {/each}
    {#if references.length > 3}<button type="button" aria-expanded={expanded} onclick={() => expanded = !expanded}>{t($locale, expanded ? 'chatPage.fewerSources' : 'chatPage.allSources')}</button>{/if}
</section>
{/if}
<style>
    .bibliography { margin-block: 12px; color: var(--muted); font-size: .8125rem; min-width: 0; }
    p { margin: 0 0 4px; } p span { margin-inline-start: 4px; }
    button { background: transparent; border: 0; border-radius: 6px; padding: 4px 0; min-height: 32px; color: inherit; font: inherit; cursor: pointer; }
    button:hover { color: var(--content); text-decoration: underline; }
    button:focus-visible { outline: 2px solid var(--muted); outline-offset: 2px; }
    @media (pointer: coarse) { button { min-height: 44px; } }
</style>
