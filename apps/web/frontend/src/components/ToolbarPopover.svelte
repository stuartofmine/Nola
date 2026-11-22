<script lang="ts">
    import PrototypeIcon from './PrototypeIcon.svelte';
    import type { Snippet } from 'svelte';
    let {label, children}: {label: string; children: Snippet} = $props();
    let panel: HTMLDivElement;
    let trigger: HTMLButtonElement;
    let top = $state(56);
    let right = $state(12);
    const id = $props.id();
    function afterToggle(event: ToggleEvent) {
        if (event.newState === 'open') {
            queueMicrotask(() => { if (panel.matches(':popover-open')) panel.querySelector<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled)')?.focus(); });
        } else if (panel.contains(document.activeElement)) { trigger.focus(); }
    }
</script>
<svelte:window onresize={() => { if (panel?.matches(':popover-open')) panel.hidePopover(); }} />
<button class="toolbar-more" type="button" bind:this={trigger} onclick={() => {
    const bounds = trigger.getBoundingClientRect();
    top = bounds.bottom + 8; right = Math.max(12, window.innerWidth - bounds.right);
}} popovertarget={id} aria-label={label} title={label} aria-haspopup="dialog"><PrototypeIcon name="more" /></button>
<div id={id} popover="auto" role="dialog" aria-label={label} class="toolbar-popover" tabindex="-1" style:top={`${top}px`} style:right={`${right}px`} style:--popover-top={`${top}px`} bind:this={panel} ontoggle={afterToggle} onkeydown={event => { if (event.key === 'Escape') { panel.hidePopover(); trigger.focus(); } }} onclick={event => {
    const target = event.target instanceof Element ? event.target : null;
    if (!target || target.closest('details, [data-keep-popover]')) return;
    const button = target.closest('button');
    if (button && !button.disabled && !button.hasAttribute('aria-haspopup')) panel.hidePopover();
}}>{@render children()}</div>
<style>
    .toolbar-more { flex: none; display: inline-flex; align-items: center; justify-content: center; padding: 0; color: var(--muted); width: var(--control-hit-size); min-height: var(--control-hit-size); border-color: transparent; background: transparent; font-size: 24px; }
    .toolbar-popover { position: fixed; inset: 56px 12px auto auto; margin: 0; width: min(320px, calc(100vw - 24px)); max-height: calc(100dvh - var(--popover-top) - 12px); overflow: auto; padding: 12px; border: 1px solid var(--border); border-radius: 14px; background: var(--surface-elevated, var(--surface)); color: var(--content); box-shadow: 0 12px 48px rgb(0 0 0 / .22); }
    .toolbar-popover:popover-open { display: grid; gap: 8px; }
    .toolbar-popover :global(button) { min-height: 44px; text-align: start; }
    .toolbar-popover :global(.model-menu) { max-width: 100%; width: 100%; }
    .toolbar-popover :global(.model-menu .choices) { position: static; width: 100%; max-width: 100%; max-height: min(40dvh, 280px); box-shadow: none; margin-top: 6px; translate: none !important; }
</style>
