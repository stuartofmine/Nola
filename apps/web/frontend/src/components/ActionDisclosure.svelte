<script lang="ts">
    import type { Snippet } from 'svelte';
    let { label, children, iconOnly = false }: { label: string; children: Snippet; iconOnly?: boolean } = $props();
    let root: HTMLDetailsElement;
    let open = $state(false);
    function close(restore = false) {
        open = false;
        if (root) root.open = false;
        if (restore) root?.querySelector('summary')?.focus();
    }
</script>

<svelte:window
    onclick={event => { if (root?.open && !event.composedPath().includes(root)) close(); }}
    onfocusin={event => { if (root?.open && event.target instanceof Node && !root.contains(event.target)) close(); }}
    onkeydown={event => { if (root?.open && event.key === 'Escape' && event.composedPath().includes(root)) { event.preventDefault(); close(true); } }} />
<details class="action-disclosure" bind:this={root} bind:open>
    <summary aria-label={label} title={label}>
        {#if iconOnly}<span aria-hidden="true">⋯</span>{:else}<span>{label}</span><span class="chevron" aria-hidden="true">⌄</span>{/if}
    </summary>
    <div class="disclosed-actions">{@render children()}</div>
</details>

<style>
    .action-disclosure { position: relative; min-width: 0; }
    summary { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-width: var(--control-hit-size); min-height: var(--control-hit-size); padding: 6px 8px; border-radius: var(--radius-control); color: var(--muted); list-style: none; cursor: pointer; font-size: .8125rem; }
    summary::-webkit-details-marker { display: none; }
    summary:hover, details[open] > summary { background: var(--panel); color: var(--content); }
    .chevron { transition: transform var(--motion-base) ease; }
    details[open] .chevron { transform: rotate(180deg); }
    .disclosed-actions { display: flex; flex-wrap: wrap; gap: 4px; padding: 8px 0; }
    details[open] .disclosed-actions { animation: enter var(--motion-base) ease-out; }
    .disclosed-actions :global(button) { text-align: start; }
    @keyframes enter { from { opacity: 0; } to { opacity: 1; } }
</style>
