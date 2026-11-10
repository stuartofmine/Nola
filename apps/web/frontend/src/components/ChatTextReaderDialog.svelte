<script lang="ts">
    import { onDestroy, onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import type {MessageTextSnapshot} from '../lib/messagePartReading';
    let {snapshot,onclose}:{snapshot:MessageTextSnapshot;onclose:()=>void}=$props();
    let dialog:HTMLDialogElement;
    let back:HTMLButtonElement;
    let copyState=$state<'idle'|'copying'|'copied'|'failed'>('idle');
    let alive=true;
    onMount(()=>{dialog.showModal();back.focus();});
    function close(){onclose();}
    async function copy(){
        if(copyState==='copying')return;
        copyState='copying';
        try{await navigator.clipboard.writeText(snapshot.text);if(alive&&dialog.open)copyState='copied';}
        catch{if(alive&&dialog.open)copyState='failed';}
    }
    onDestroy(()=>{alive=false;if(dialog?.open)dialog.close();});
</script>

<dialog class="text-reader" bind:this={dialog} aria-label={snapshot.title} oncancel={event => { event.preventDefault(); close(); }}>
    <header>
        <h2 title={snapshot.title}>{snapshot.title}</h2>
        <button type="button" disabled={copyState === 'copying'} aria-busy={copyState === 'copying'} onclick={copy}>{t($locale, 'chatPage.copyText')}</button>
        <button type="button" bind:this={back} onclick={close}>{t($locale, 'chatPage.closeReader')}</button>
    </header>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Notices can scroll on short, zoomed viewports.) -->
    <div class="reader-notices" role="region" aria-label={t($locale, 'chatPage.readerSnapshot')} tabindex="0">
        <p class="snapshot-note">{t($locale, 'chatPage.readerSnapshot')}</p>
        <p class="copy-status" role="status" aria-atomic="true">{copyState === 'failed' ? t($locale, 'chatPage.copyFailed') : copyState === 'copied' ? t($locale, 'chatPage.textCopied') : ''}</p>
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable output must be keyboard reachable.) -->
    <pre role="region" aria-label={snapshot.title} tabindex="0">{snapshot.text}</pre>
</dialog>

<style>
    :global(html:has(dialog.text-reader[open])) { overflow: hidden; }
    .text-reader { box-sizing: border-box; width: min(960px, calc(100vw - 48px)); max-width: none; height: min(85dvh, 900px); max-height: none; padding: 20px; background: var(--panel); color: inherit; border: 1px solid var(--border); border-radius: var(--radius); overflow: hidden; }
    .text-reader[open] { display: flex; flex-direction: column; gap: 12px; }
    .text-reader::backdrop { background: rgb(0 0 0 / .4); }
    header { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; flex: 0 0 auto; min-width: 0; }
    h2 { flex: 1 0 100%; min-width: 0; margin: 0; font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    header button:last-child { margin-inline-start: auto; }
    header button { min-height: 44px; }
    pre { min-height: 0; flex: 1; margin: 0; padding: 12px; overflow: auto; overscroll-behavior: contain; white-space: pre-wrap; overflow-wrap: anywhere; background: var(--canvas); border-radius: 6px; font-size: 13px; line-height: 1.6; }
    button:focus-visible, pre:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    p { margin: 0; }
    .reader-notices { flex: 0 1 auto; min-height: 0; max-height: 25%; overflow: auto; overscroll-behavior: contain; }
    .copy-status { font-size: 12px; }
    .copy-status:not(:empty) { margin-top: 6px; }
    .snapshot-note { color: var(--muted); font-size: 12px; }
    @media (min-width: 601px) and (min-height: 501px) { h2 { flex: 1 1 160px; } }
    @media (max-width: 600px), (max-height: 500px) { .text-reader { width: 100%; height: 100dvh; margin: 0; border-radius: 0; padding: max(12px, env(safe-area-inset-top)) max(12px, env(safe-area-inset-right)) max(12px, env(safe-area-inset-bottom)) max(12px, env(safe-area-inset-left)); } }
</style>
