<script lang="ts">
    import { untrack, getContext } from 'svelte';
    import { TextReveal, TEXT_REVEAL_MS, STREAMING_TEXT_SELECTION } from '../lib/streamingText';
    let { text, streaming = false, visiblePrefix = text.length }: {
        text: string; streaming?: boolean; visiblePrefix?: number;
    } = $props();
    const preserveSelection = getContext<(() => void) | undefined>(STREAMING_TEXT_SELECTION);
    const reveal = new TextReveal();
    let view = $state.raw(untrack(() => reveal.update(text, visiblePrefix, streaming, performance.now())));
    $effect.pre(() => {
        preserveSelection?.();
        view = reveal.update(text, visiblePrefix, streaming, performance.now());
    });
</script>

{#if streaming}{view.settled}{#each view.pieces as piece (piece.id)}<span class="fresh" style:animation-duration={`${TEXT_REVEAL_MS}ms`} onanimationend={() => { preserveSelection?.(); view = reveal.finish(piece.id); }}>{piece.text}</span>{/each}{:else}{text}{/if}

<style>
    .fresh { animation: surface-text cubic-bezier(.2, .7, .2, 1) both; }
    @keyframes surface-text {
        from { opacity: .08; filter: blur(2px); }
        to { opacity: 1; filter: blur(0); }
    }
    @media (prefers-reduced-motion: reduce) { .fresh { animation: none; } }
</style>
