<script lang="ts">
    import type { Snippet } from 'svelte';
    import { anchoredPopover } from '../lib/anchoredPopover';
    let { label, children, iconOnly = false }: { label: string; children: Snippet; iconOnly?: boolean } = $props();
    let root: HTMLDetailsElement;
    let open = $state(false);
    export function close(restore = false) {
        open = false;
        if (root) root.open = false;
        if (restore) root?.querySelector('summary')?.focus();
    }
</script>

<svelte:window
    onclick={event => { if (root?.open && !event.composedPath().includes(root)) close(); }}
    onfocusin={event => { if (root?.open && event.target instanceof Node && !root.contains(event.target)) close(); }}
    onkeydown={event => { if (root?.open && event.key === 'Escape' && event.composedPath().includes(root)) { event.preventDefault(); close(true); } }} />
<details class="action-disclosure" class:compact-actions={iconOnly} bind:this={root} bind:open>
    <summary aria-label={label} title={label}>
        {#if iconOnly}<span aria-hidden="true">⋯</span>{:else}<span>{label}</span><span class="chevron" aria-hidden="true">⌄</span>{/if}
    </summary>
    {#if iconOnly}
        <div class="disclosed-actions" popover="manual" use:anchoredPopover={216}>{@render children()}</div>
    {:else}
        <div class="disclosed-actions">{@render children()}</div>
    {/if}
</details>

<style>
    .action-disclosure { position: relative; min-width: 0; }
    summary { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-width: var(--control-hit-size); min-height: var(--control-hit-size); padding: 6px 8px; border-radius: var(--radius-control); color: var(--muted); list-style: none; cursor: pointer; font-size: .8125rem; }
    summary::-webkit-details-marker { display: none; }
    summary:hover, details[open] > summary { background: var(--panel); color: var(--content); }
    .compact-actions { flex: none; }
    .compact-actions > summary {
        box-sizing: border-box; width: var(--control-hit-size); height: var(--control-hit-size);
        justify-content: center; padding: 0; font-size: 18px; line-height: 1;
    }
    .disclosed-actions[popover] {
        position: fixed; inset: auto; margin: 0; box-sizing: border-box; overflow: auto;
        padding: 4px; border: 1px solid var(--border); border-radius: var(--radius-control);
        background: var(--canvas); color: var(--content); box-shadow: 0 6px 18px rgb(0 0 0 / .15);
    }
    .disclosed-actions[popover]:not(:popover-open) { display: none; }
    .disclosed-actions:popover-open { display: grid; gap: 0; }
    .disclosed-actions[popover] :global(button:focus-visible) { outline-offset: -2px; }
    .chevron { transition: transform var(--motion-base) ease; }
    details[open] .chevron { transform: rotate(180deg); }
    .disclosed-actions { display: flex; flex-wrap: wrap; gap: 4px; padding: 8px 0; }
    details[open] .disclosed-actions { animation: enter var(--motion-base) ease-out; }
    .disclosed-actions :global(button) { text-align: start; }
    @keyframes enter { from { opacity: 0; } to { opacity: 1; } }
</style>
