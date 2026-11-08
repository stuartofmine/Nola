<script lang="ts">
    import { workspaceDialog } from '../lib/workspacePageLocation';
    const ownDialog = workspaceDialog();
    import { onDestroy, tick } from 'svelte';
    import { ConfirmationGate } from '../lib/confirmation';
    import type { ConfirmationRequest } from '../lib/confirmation';
    import { locale, t } from '../lib/i18n';

    const gate = new ConfirmationGate();
    let request = $state<ConfirmationRequest>();
    let dialog: HTMLDialogElement;
    let cancelButton = $state<HTMLButtonElement>();

    export async function ask(value: ConfirmationRequest): Promise<boolean> {
        const decision = gate.request();
        if (!decision) return false;
        request = value;
        await tick();
        if (gate.pending) {
            try { dialog.showModal(); cancelButton?.focus(); }
            catch { finish(false); }
        }
        return decision;
    }

    // Owners may live inside a collapsed inspector. Modal content must not
    // inherit hidden/inert ancestors while showModal makes the page inert.
    function portal(node: HTMLDialogElement) {
        document.body.appendChild(node);
        return { destroy() { node.close(); node.remove(); } };
    }

    function finish(accepted: boolean) {
        if (dialog?.open) dialog.close();
        request = undefined;
        gate.settle(accepted);
    }

    function cancel(event: Event) { event.preventDefault(); finish(false); }
    onDestroy(() => gate.settle(false));
</script>

<dialog use:ownDialog use:portal bind:this={dialog} aria-label={request?.title ?? t($locale, 'shell.confirm')} oncancel={cancel}>
    {#if request}
        <div class="confirmation">
            <h2>{request.title}</h2>
            <p class="muted">{request.description}</p>
            <div class="actions">
                <button bind:this={cancelButton} onclick={() => finish(false)}>{request.cancelLabel ?? t($locale, 'shell.cancel')}</button>
                <button class:danger={request.destructive} class:primary={!request.destructive} onclick={() => finish(true)}>{request.confirmLabel}</button>
            </div>
        </div>
    {/if}
</dialog>

<style>
    dialog { max-width: 420px; }
    .confirmation { display: grid; gap: 16px; }
    h2, p { margin: 0; overflow-wrap: anywhere; }
</style>
