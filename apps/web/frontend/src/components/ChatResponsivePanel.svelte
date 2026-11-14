<script lang="ts">
    import { onMount, tick, type Snippet } from 'svelte';
    import { isDialogBackdrop } from '../lib/dialogBackdrop';
    import { completesPanelSwipe, panelOutwardSign, type PanelSide, type SwipePoint } from '../lib/panelSwipe';

    let { label, closeLabel, breakpoint, children, expanded = null, onclose = () => {}, toolbar, side = 'end' }: {
        label: string; closeLabel: string; breakpoint: number; children: Snippet;
        expanded?: boolean | null; onclose?: () => void;
        toolbar?: Snippet; side?: PanelSide;
    } = $props();
    let dialog: HTMLDialogElement;
    let closeButton = $state<HTMLButtonElement>();
    let compact = $state(false);
    let opener: HTMLElement | null = null;
    let dragX = $state(0);
    let dragging = $state(false);
    let dragStart: SwipePoint | undefined;
    let dragPointer: number | undefined;
    let backdropPointer: number | undefined;
    function backdropStart(event: PointerEvent) {
        backdropPointer = compact && isDialogBackdrop(event.target === dialog, event.clientX, event.clientY,
            dialog.getBoundingClientRect()) ? event.pointerId : undefined;
    }
    function backdropEnd(event: PointerEvent) {
        const dismiss = backdropPointer === event.pointerId && compact && isDialogBackdrop(
            event.target === dialog, event.clientX, event.clientY, dialog.getBoundingClientRect());
        backdropPointer = undefined;
        if (dismiss) closePanel();
    }
    function startDrag(event: PointerEvent) {
        if (!compact || event.pointerType !== 'touch' || (event.target as HTMLElement).closest('button, a, input, select, textarea')) return;
        dragStart = {x: event.clientX, y: event.clientY, time: event.timeStamp};
        dragPointer = event.pointerId;
        (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    }
    function moveDrag(event: PointerEvent) {
        if (!dragStart || event.pointerId !== dragPointer) return;
        const dx = event.clientX - dragStart.x, dy = event.clientY - dragStart.y;
        if (Math.abs(dy) > Math.abs(dx) && !dragging) { dragStart = undefined; return; }
        const sign = panelOutwardSign(side, getComputedStyle(dialog).direction === 'rtl');
        dragging = Math.abs(dx) > 8;
        dragX = sign * Math.max(0, Math.min(dialog.clientWidth, dx * sign));
    }
    function finishDrag(event: PointerEvent) {
        if (dragStart && event.pointerId === dragPointer && completesPanelSwipe(dragStart,
            {x:event.clientX, y:event.clientY, time:event.timeStamp}, dialog.clientWidth,
            panelOutwardSign(side, getComputedStyle(dialog).direction === 'rtl'))) closePanel();
        cancelDrag();
    }
    function cancelDrag() { dragStart = undefined; dragPointer = undefined; dragging = false; dragX = 0; }


    function showInline() {
        // Inline complementary content must not run the dialog focusing steps.
        // show() focuses its first control even when the document body had focus.
        dialog.open = true;
    }
    function focusModalClose() {
        // The compact toolbar is created by Svelte after a breakpoint change.
        void tick().then(() => {
            if (dialog?.isConnected && dialog.matches(':modal') && closeButton?.getClientRects().length) closeButton.focus();
        });
    }
    // Keep the same mounted children across breakpoints: settings and conversation
    // observers must not be duplicated or restarted by a presentation change.
    export function openPanel() {
        if (!compact || dialog.open) return;
        opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        dialog.showModal();
        focusModalClose();
    }
    export function closePanel() {
        cancelDrag();
        onclose();
        if (!compact && expanded === null) return;
        dialog.close();
        if (opener?.isConnected && opener.getClientRects().length) opener.focus();
        opener = null;
    }
    $effect(() => {
        if (!dialog || expanded === null) return;
        if (expanded) {
            if (compact) openPanel();
            else if (!dialog.open) { opener = document.activeElement instanceof HTMLElement ? document.activeElement : null; showInline(); }
        } else if (dialog.open) closePanel();
    });
    onMount(() => {
        const media = window.matchMedia(`(max-width: ${breakpoint}px)`);
        let deferred = false;
        const update = () => {
            const otherModalOpen = [...document.querySelectorAll<HTMLDialogElement>('dialog:modal')].some(modal => modal !== dialog);
            if (otherModalOpen && dialog.open) { deferred = true; return; }
            deferred = false;
            if (!compact && media.matches && expanded === true) {
                dialog.close(); compact = true; onclose(); return;
            }
            const wasModal = compact && dialog.open;
            if (dialog.open) dialog.close();
            compact = media.matches;
            if (!compact && expanded !== false) showInline();
            else if (compact && expanded === true) { dialog.showModal(); focusModalClose(); }
            if (wasModal && !compact) {
                if (opener?.isConnected && opener.getClientRects().length) opener.focus();
            }
        };
        update();
        media.addEventListener('change', update);
        const afterNestedClose = () => {
            if (!deferred) return;
            // Closing a parent while its child is in the top layer breaks native
            // Escape/focus ownership. Apply the pending breakpoint after it exits.
            void tick().then(() => {
                const focused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
                update();
                if (!deferred && focused && dialog.contains(focused) && focused.getClientRects().length) focused.focus({ preventScroll: true });
            });
        };
        document.addEventListener('close', afterNestedClose, true);
        return () => { media.removeEventListener('change', update); document.removeEventListener('close', afterNestedClose, true); };
    });
</script>

<dialog bind:this={dialog} class:compact class:dragging class:start={side === 'start'} style:--panel-drag-x={`${dragX}px`} role={compact ? 'dialog' : 'complementary'}
    onpointerdown={backdropStart} onpointerup={backdropEnd} onpointercancel={() => { backdropPointer = undefined; }}
    aria-label={label} oncancel={event => { event.preventDefault(); closePanel(); }}>
    {#if compact}<div class="panel-toolbar" onpointerdown={startDrag} onpointermove={moveDrag} onpointerup={finishDrag} onpointercancel={cancelDrag} role="presentation">{#if toolbar}{@render toolbar()}{:else}<strong>{label}</strong>{/if}<button bind:this={closeButton} onclick={closePanel} aria-label={closeLabel}>×</button></div>{/if}
    {@render children()}
</dialog>

<style>
    dialog:not(.compact)[open] { display: contents; }
    dialog:not([open]) { display: none; }
    dialog.compact { --panel-entry-x:24px; padding: 0; width: min(440px, calc(100vw - 28px)); max-width: none; height:100dvh; max-height:100dvh; inset-block:0; margin-block:0; margin-inline: auto 0; overflow: auto; color: inherit; background: var(--canvas); border: 1px solid var(--border); border-radius: 0;
        opacity: 0; transform: translateX(var(--panel-entry-x));
        transition: opacity var(--motion-slow) var(--motion-disclosure), transform var(--motion-slow) var(--motion-disclosure), display var(--motion-slow) allow-discrete, overlay var(--motion-slow) allow-discrete;
        transition-behavior: allow-discrete;
    }
    dialog.compact.start { --panel-entry-x:-24px; margin-inline:0 auto; }
    dialog.compact:dir(rtl) { --panel-entry-x:-24px; }
    dialog.compact.start:dir(rtl) { --panel-entry-x:24px; }
    dialog.compact.dragging { transition:none; }
    dialog.compact:not([open]) { display: none; }
    dialog.compact[open] { opacity: 1; transform: translateX(var(--panel-drag-x, 0px)); }
    dialog.compact::backdrop { background: rgb(0 0 0 / 0); transition: background-color var(--motion-slow) ease; }
    dialog.compact[open]::backdrop { background: rgb(0 0 0 / .4); }
    @starting-style { dialog.compact[open] { opacity: 0; transform: translateX(var(--panel-entry-x)); } dialog.compact[open]::backdrop { background: rgb(0 0 0 / 0); } }
    @media (prefers-reduced-motion: reduce) { dialog.compact, dialog.compact::backdrop { transition-duration: 0ms; } }
    .panel-toolbar { touch-action:pan-y; user-select:none; display: flex; align-items: center; justify-content: space-between; gap: 16px; position: sticky; top: 0; padding: max(8px, env(safe-area-inset-top)) 12px 8px; background: var(--canvas); z-index: 1; border-bottom: 1px solid var(--border); }
    .panel-toolbar button { min-width: 44px; min-height: 44px; font-size: 20px; }
</style>
