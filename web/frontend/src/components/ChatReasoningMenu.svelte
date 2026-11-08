<script lang="ts">
    import { tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import type { SessionReasoningQuickState } from '../lib/chatSessionSettings';
    let { state: reasoning, busy = false, onselect }: {
        state: SessionReasoningQuickState | null;
        busy?: boolean;
        onselect: (value: string) => unknown;
    } = $props();
    let open = $state(false);
    let root = $state<HTMLDetailsElement>();
    let choiceShift = $state(0);
    function positionChoices() {
        const choices = root?.querySelector<HTMLElement>('.choices');
        if (!open || !choices) return;
        const bounds = choices.getBoundingClientRect();
        const naturalLeft = bounds.left - choiceShift;
        const edge = window.visualViewport?.offsetLeft ?? 0;
        const width = window.visualViewport?.width ?? window.innerWidth;
        choiceShift = Math.min(Math.max(naturalLeft, edge + 8), Math.max(edge + 8, edge + width - bounds.width - 8)) - naturalLeft;
    }
    $effect(() => { if (open) void tick().then(positionChoices); });
    const disabled = $derived(!reasoning?.ready || reasoning.busy || busy);
    const preference = $derived(reasoning?.preference ?? 'inherit');
    $effect(() => { if (!reasoning?.options.length || disabled) open = false; });
    function label(value: string) { return t($locale, `chatSession.${value}`); }
    function close() { open = false; root?.querySelector('summary')?.focus({ preventScroll: true }); }
    async function navigate(event: KeyboardEvent) {
        if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || disabled) return;
        if (open && event.key === 'Escape') {
            event.preventDefault(); event.stopPropagation(); close(); return;
        }
        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        const fromTrigger = event.target === root?.querySelector('summary');
        open = true;
        await tick();
        positionChoices();
        await tick();
        if (!open || disabled || !root?.isConnected) return;
        const buttons = [...root.querySelectorAll<HTMLButtonElement>('.choices button:not(:disabled)')];
        if (!buttons.length) return;
        const active = buttons.indexOf(document.activeElement as HTMLButtonElement);
        let index: number;
        if (event.key === 'Home') index = 0;
        else if (event.key === 'End') index = buttons.length - 1;
        else if (fromTrigger || active < 0) {
            index = buttons.findIndex(button => button.getAttribute('aria-pressed') === 'true');
            if (index < 0) index = event.key === 'ArrowUp' ? buttons.length - 1 : 0;
        } else index = (active + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
        buttons[index].focus({ preventScroll: true });
        buttons[index].scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' });
    }
    function select(value: string) {
        if (disabled) return;
        close();
        void onselect(value);
    }
</script>
<svelte:window onclick={event => { if (open && root && !event.composedPath().includes(root)) open = false; }}
    onfocusin={event => { if (root && event.target instanceof Node && !root.contains(event.target)) open = false; }}
    onresize={positionChoices} />
{#if reasoning?.options.length}
    <details class="reasoning-menu" bind:this={root} bind:open>
        <summary onkeydown={navigate} aria-label={`${t($locale, 'chatSession.reasoning')}: ${label(preference)}`} aria-disabled={disabled} aria-busy={reasoning.busy}
            onclick={event => { if (disabled) event.preventDefault(); }}>
            <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M6 18v-6m6 6V8m6 10V4" /></svg>
            <span>{label(preference)}</span><small aria-hidden="true">⌄</small>
        </summary>
        <div class="choices" style:translate={`${choiceShift}px 0`} aria-label={t($locale, 'chatSession.reasoning')}>
            {#each ['inherit', 'automatic', ...reasoning.options] as effort (effort)}
                <button type="button" onkeydown={navigate} disabled={disabled} aria-pressed={preference === effort} onclick={() => select(effort)}>
                    <span>{label(effort)}</span><span aria-hidden="true">{preference === effort ? '✓' : ''}</span>
                </button>
            {/each}
            {#if reasoning.errorKey}<p role="status">{t($locale, reasoning.errorKey)}</p>{/if}
        </div>
    </details>
{/if}
<style>
    .reasoning-menu { position: relative; min-width: 0; flex: 0 1 auto; max-width: 35%; }
    summary { display: flex; align-items: center; gap: 4px; min-height: 44px; padding: 0 6px; cursor: pointer; list-style: none; color: var(--muted); font-size: 12px; }
    summary::-webkit-details-marker { display: none; }
    summary[aria-disabled="true"] { opacity: .5; cursor: default; }
    summary span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    svg { width: 14px; height: 14px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; }
    .choices { position: absolute; bottom: calc(100% + 8px); inset-inline-end: 0; width: min(240px, calc(100vw - 80px)); max-height: min(50dvh, 360px); overflow: auto; overscroll-behavior: contain; z-index: 50; padding: 6px; background: var(--surface-gradient); border: 1px solid var(--border); border-radius: 14px; box-shadow: var(--surface-shadow); }
    button { display: flex; width: 100%; align-items: center; justify-content: space-between; gap: 12px; min-height: 44px; padding: 8px 12px; text-align: start; border: 0; background: transparent; box-shadow: none; color: var(--content); }
    button[aria-pressed="true"], button:hover:not(:disabled) { background: var(--accent-subtle); }
    p { padding: 8px; font-size: 12px; }
</style>
