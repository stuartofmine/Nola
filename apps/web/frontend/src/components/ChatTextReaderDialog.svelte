<script lang="ts">
    import { onDestroy, onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';

    export interface TextReaderSnapshot {
        title: string;
        text: string;
        subtitle?: string;
    }

    let {
        snapshot,
        onclose,
    }: {
        snapshot: TextReaderSnapshot;
        onclose: () => void;
    } = $props();

    let dialog: HTMLDialogElement;
    let back: HTMLButtonElement;
    let copyState = $state<'idle' | 'copying' | 'copied' | 'failed'>('idle');
    let opener: HTMLElement | null = null;
    let alive = true;

    onMount(() => {
        opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        dialog?.showModal();
        back?.focus();
    });

    function close() {
        onclose();
    }

    async function copy() {
        if (copyState === 'copying') return;
        copyState = 'copying';
        try {
            await navigator.clipboard.writeText(snapshot.text);
            if (alive && dialog?.open) copyState = 'copied';
        } catch {
            if (alive && dialog?.open) copyState = 'failed';
        }
    }

    onDestroy(() => {
        alive = false;
        if (dialog?.open) dialog.close();
        const target = opener;
        opener = null;
        if (target?.isConnected) target.focus({ preventScroll: true });
    });
</script>

<dialog
    class="text-reader"
    bind:this={dialog}
    aria-label={snapshot.title}
    oncancel={event => {
        event.preventDefault();
        close();
    }}
>
    <header>
        <div class="reader-title-group">
            <h2 title={snapshot.title}>{snapshot.title}</h2>
            {#if snapshot.subtitle}
                <small class="reader-subtitle">{snapshot.subtitle}</small>
            {/if}
        </div>
        <button type="button" disabled={copyState === 'copying'} aria-busy={copyState === 'copying'} onclick={copy}>
            {copyState === 'copied' ? t($locale, 'chatInspector.copied') : t($locale, 'chatPage.copyText')}
        </button>
        <button type="button" bind:this={back} onclick={close}>
            {t($locale, 'chatPage.closeReader')}
        </button>
    </header>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Notices can scroll on short viewports.) -->
    <div class="reader-notices" role="region" aria-label={t($locale, 'chatPage.readerSnapshot')} tabindex="0">
        <p class="snapshot-note">{t($locale, 'chatPage.readerSnapshot')}</p>
        {#if copyState === 'failed' || copyState === 'copied'}
            <p class="copy-status" role="status" aria-atomic="true">
                {copyState === 'failed' ? t($locale, 'chatPage.copyFailed') : t($locale, 'chatPage.textCopied')}
            </p>
        {/if}
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable output must be keyboard reachable.) -->
    <pre role="region" aria-label={snapshot.title} tabindex="0">{snapshot.text}</pre>
</dialog>

<style>
    :global(html:has(dialog.text-reader[open])) {
        overflow: hidden;
    }
    .text-reader {
        box-sizing: border-box;
        width: min(960px, calc(100vw - 48px));
        max-width: none;
        height: min(85dvh, 900px);
        max-height: none;
        padding: 20px;
        background: var(--panel);
        color: var(--content);
        border: 1px solid var(--border);
        border-radius: var(--radius, 12px);
        overflow: hidden;
    }
    .text-reader[open] {
        display: flex;
        flex-direction: column;
        gap: 12px;
    }
    .text-reader::backdrop {
        background: rgb(0 0 0 / 0.45);
    }
    header {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 8px;
        flex: 0 0 auto;
        min-width: 0;
    }
    .reader-title-group {
        flex: 1 1 200px;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 2px;
    }
    h2 {
        min-width: 0;
        margin: 0;
        font-size: 16px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .reader-subtitle {
        font-size: 12px;
        color: var(--muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    header button {
        min-height: 36px;
        padding: 4px 12px;
        border-radius: 8px;
    }
    pre {
        min-height: 0;
        flex: 1;
        margin: 0;
        padding: 14px;
        overflow: auto;
        overscroll-behavior: contain;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        background: var(--canvas);
        border: 1px solid var(--border);
        border-radius: 8px;
        font-family: var(--font-code, monospace);
        font-size: 13px;
        line-height: 1.6;
    }
    .reader-notices {
        flex: 0 1 auto;
        min-height: 0;
    }
    .snapshot-note,
    .copy-status {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .copy-status {
        margin-top: 4px;
        color: var(--content);
    }
    @media (max-width: 600px), (max-height: 500px) {
        .text-reader {
            width: 100%;
            height: 100dvh;
            margin: 0;
            border-radius: 0;
        }
    }
</style>
