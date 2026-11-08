<script lang="ts">
	import { onDestroy } from 'svelte';
	import Slot from '../../render/Slot.svelte';
	import type { SlotContent } from '../types.js';

	interface Props {
		slots: Record<string, SlotContent>;
		weight?: number;
		ariaLabel?: string;
		disabled?: boolean;
	}

	let { slots, weight, ariaLabel, disabled = false }: Props = $props();

	let open = $state(false);
	let dialog = $state<HTMLDialogElement | null>(null);
    let trigger: HTMLDivElement;
    function reveal() {
        if (disabled || document.querySelector('dialog:modal')) return;
        open = true;
    }
    function closed() {
        open = false;
        if (trigger?.isConnected) trigger.focus({preventScroll: true});
    }
    onDestroy(() => { if (dialog?.open) dialog.close(); });

	// <dialog> owns focus trapping, Escape and the backdrop; keep it authoritative.
	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) {
            if (document.querySelector('dialog:modal')) open = false;
            else dialog.showModal();
        }
		else if (!open && dialog.open) dialog.close();
	});
</script>

<div
	bind:this={trigger}
    class="a2ui-modal-trigger"
	style:flex-grow={weight}
	role="button"
	tabindex={disabled ? -1 : 0}
	aria-disabled={disabled}
	aria-haspopup="dialog"
	aria-expanded={open}
	aria-label={ariaLabel ?? "查看详情"}
	onclick={reveal}
	onkeydown={(e) => {
		if (!disabled && (e.key === 'Enter' || e.key === ' ')) {
			e.preventDefault();
			reveal();
		}
	}}
>
    <!-- Trigger content is visual only; opening the dialog is one action. -->
    <div inert class="trigger-content"><Slot content={slots.trigger} presentationOnly /></div>
</div>

<dialog bind:this={dialog} class="a2ui-modal" aria-label={ariaLabel ?? "详情"} onclose={closed} onclick={(event) => {
        if (event.target !== dialog || !dialog) return;
        const rect = dialog.getBoundingClientRect();
        if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) dialog.close();
    }}>
	<div class="body">
		{#if open}
			<Slot content={slots.content} />
		{/if}
	</div>
	<form method="dialog" class="dismiss">
		<button type="submit" aria-label="关闭">关闭</button>
	</form>
</dialog>

<style>
	.a2ui-modal-trigger {
        display: flex;
        align-items: center;
        min-height: 44px;
        cursor: pointer;
        border-radius: var(--a2ui-radius-small);
    }
    .a2ui-modal-trigger:focus-visible { outline: 2px solid currentColor; outline-offset: 3px; }
    .a2ui-modal-trigger[aria-disabled="true"] { cursor: default; opacity: 0.6; }
    .trigger-content { pointer-events: none; width: 100%;
	}
	.a2ui-modal {
		border: 1px solid var(--a2ui-color-border);
		border-radius: var(--a2ui-radius);
		background: var(--a2ui-color-surface);
		color: var(--a2ui-color-text);
		padding: var(--a2ui-space-inset);
		max-width: min(90vw, 32rem);
        max-height: 85dvh;
        box-sizing: border-box;
        overflow: auto;
        overscroll-behavior: contain;
		box-shadow: var(--a2ui-elevation);
	}
	.a2ui-modal::backdrop {
		background: rgb(0 0 0 / 0.45);
	}
	.body {
		display: flex;
		flex-direction: column;
	}
	.dismiss {
		display: flex;
		justify-content: flex-end;
		margin-top: 0.75rem;
	}
	.dismiss button {
		min-height: 44px;
        font: inherit;
		color: inherit;
		background: none;
		border: 1px solid var(--a2ui-color-border);
		border-radius: var(--a2ui-radius-small);
		padding: 0.35rem 0.75rem;
		cursor: pointer;
	}

	/* On narrow screens present as a bottom sheet. */
	@media (max-width: 40rem) {
		.a2ui-modal {
			margin: auto auto 0;
			max-width: 100vw;
			width: 100%;
			border-radius: var(--a2ui-radius) var(--a2ui-radius) 0 0;
		}
	}
</style>
