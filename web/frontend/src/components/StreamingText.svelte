<script lang="ts">
    import { untrack, getContext, onDestroy } from 'svelte';
    import { TextReveal, STREAMING_TEXT_SELECTION, type RevealPiece } from '../lib/streamingText';
    let { text, streaming = false, visiblePrefix = text.length }: {
        text: string; streaming?: boolean; visiblePrefix?: number;
    } = $props();
    const preserveSelection = getContext<(() => void) | undefined>(STREAMING_TEXT_SELECTION);
    const reveal = new TextReveal();
    let reducedMotion = $state(typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches);
    let view = $state.raw(untrack(() => reveal.update(text, visiblePrefix, streaming, performance.now(), reducedMotion)));
    const observeMotion = $derived(streaming || view.pieces.length > 0);
    const nodes = new Map<number, { node: HTMLElement; piece: RevealPiece }>();
    let frame = 0;

    function paint(node: HTMLElement, piece: RevealPiece, now: number) {
        const remaining = reveal.remaining(piece, now);
        node.style.opacity = String(1 - .92 * remaining);
        node.style.top = `${3 * remaining}px`;
        node.style.filter = remaining > .08 ? `blur(${.65 * remaining}px)` : 'none';
    }
    function register(node: HTMLElement, piece: RevealPiece) {
        nodes.set(piece.id, { node, piece });
        paint(node, piece, performance.now());
        return { destroy: () => nodes.delete(piece.id) };
    }
    function animate(now: number) {
        frame = 0;
        const next = reveal.advance(now);
        if (next.pieces.length !== view.pieces.length) {
            preserveSelection?.();
            view = next;
        }
        for (const { node, piece } of nodes.values()) paint(node, piece, now);
        if (next.pieces.length) frame = requestAnimationFrame(animate);
    }
    $effect.pre(() => {
        const nextText = text, prefix = visiblePrefix, active = streaming, reduced = reducedMotion;
        untrack(() => {
            preserveSelection?.();
            view = reveal.update(nextText, prefix, active, performance.now(), reduced);
            if (frame) cancelAnimationFrame(frame);
            frame = view.pieces.length ? requestAnimationFrame(animate) : 0;
        });
    });
    $effect(() => {
        if (!observeMotion) return;
        const preference = matchMedia('(prefers-reduced-motion: reduce)');
        const changed = () => { reducedMotion = preference.matches; };
        preference.addEventListener('change', changed);
        changed();
        return () => preference.removeEventListener('change', changed);
    });
    onDestroy(() => { if (frame) cancelAnimationFrame(frame); });
</script>

{view.settled}{#each view.pieces as piece (piece.id)}<span class="fresh" use:register={piece}>{piece.text}</span>{/each}

<style>
    /* Relative positioning preserves normal wrapping, shaping and selection. */
    .fresh { position: relative; }
    @media (prefers-reduced-motion: reduce) {
        .fresh { opacity: 1 !important; filter: none !important; top: 0 !important; }
    }
</style>
