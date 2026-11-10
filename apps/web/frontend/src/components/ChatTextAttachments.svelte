<script lang="ts">
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onDestroy, tick, untrack } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { MAX_TEXT_ATTACHMENTS, TEXT_ATTACHMENT_ACCEPT, createTextAttachmentDraftStore, readTextAttachment } from '../lib/chatTextAttachments';
    import type { ChatTextAttachment } from '../lib/chatTextAttachments';

    interface Props {
        conversationId: string;
        generation: number;
        showHelp?: boolean;
        busy?: boolean;
        draftStore?: ReturnType<typeof createTextAttachmentDraftStore>;
        onattachments: (attachments: readonly ChatTextAttachment[], reading: boolean) => void;
    }
    let { conversationId, generation, showHelp = true, busy = false, draftStore, onattachments }: Props = $props();
    const localDrafts = createTextAttachmentDraftStore();
    const drafts = $derived(draftStore ?? localDrafts);
    let attachments = $state<ChatTextAttachment[]>([]);
    let reading = $state(false);
    let error = $state('');
    let preview = $state<string | null>(null);
    let request = 0;
    let picker: HTMLInputElement;
    let list = $state<HTMLUListElement>();
    onDestroy(() => { ++request; });
    function publish() { onattachments(attachments.map(item => ({ ...item })), reading); }
    $effect(() => {
        conversationId; generation;
        untrack(() => { ++request; attachments = drafts.read(conversationId); reading = false; error = ''; preview = null; publish(); });
    });
    async function select(event: Event) {
        const input = event.currentTarget as HTMLInputElement;
        const files = [...(input.files ?? [])]; input.value = '';
        if (!files.length) return;
        await addFiles(files);
    }
    /** Picker, drop and clipboard use the same validation and exact owner. */
    export async function addFiles(files: readonly File[]) {
        if (busy || reading) { error = 'chatTextAttachment.busy'; return; }
        if (!files.length) { error = 'chatTextAttachment.invalidFile'; return; }
        const id = conversationId, epoch = generation, sequence = ++request;
        reading = true; error = ''; publish();
        const imported: ChatTextAttachment[] = [];
        let rejected = false;
        let encodingError = false;
        try {
            for (const file of files) {
                if (attachments.length + imported.length >= MAX_TEXT_ATTACHMENTS) { rejected = true; break; }
                try { imported.push(await readTextAttachment(file, id, crypto.randomUUID())); }
                catch (cause) { rejected = true; if (cause instanceof Error && cause.message === 'chatTextAttachment.invalidEncoding') encodingError = true; }
                if (conversationId !== id || generation !== epoch || request !== sequence) return;
            }
            if (conversationId === id && generation === epoch && request === sequence) {
                attachments = [...attachments, ...imported];
                drafts.write(id, attachments);
                if (rejected) error = encodingError ? 'chatTextAttachment.invalidEncoding' : 'chatTextAttachment.invalidFile';
            }
        } finally {
            if (conversationId === id && generation === epoch && request === sequence) { reading = false; publish(); }
        }
    }
    async function remove(id: string, button: HTMLButtonElement) {
        if (busy || reading) return;
        const restoreFocus = document.activeElement === button;
        const index = attachments.findIndex(item => item.id === id);
        const owner = conversationId, epoch = generation;
        attachments = attachments.filter(item => item.id !== id);
        drafts.write(conversationId, attachments);
        if (preview === id) preview = null;
        publish();
        await tick();
        if (restoreFocus && conversationId === owner && generation === epoch) {
            const remaining = list?.querySelectorAll<HTMLButtonElement>('li > button:first-child');
            const next = remaining?.[Math.min(index, remaining.length - 1)] ?? picker;
            next?.focus();
        }
    }
    /** Remove only the files captured by a confirmed submission. Later-added
     * files or a different conversation are never consumed by that result. */
    export function clearSubmitted(ownerConversationId: string, ids: readonly string[]): void {
        const remaining = drafts.clearSubmitted(ownerConversationId, ids);
        if (conversationId === ownerConversationId) { attachments = remaining; if (preview && ids.includes(preview)) preview = null; publish(); }
    }
</script>

<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (reading) { event.preventDefault(); event.returnValue = ''; } }} />
<section class="text-attachments" aria-label={t($locale, 'chatTextAttachment.title')}>
    <div class="attachment-toolbar">
    <label class="picker" class:disabled={busy || reading || attachments.length >= MAX_TEXT_ATTACHMENTS}><span class="mobile-symbol" aria-hidden="true">＋</span><span class="toolbar-label">{t($locale, 'chatTextAttachment.select')}</span><input bind:this={picker} type="file" multiple accept={TEXT_ATTACHMENT_ACCEPT} onchange={select} disabled={busy || reading || attachments.length >= MAX_TEXT_ATTACHMENTS} /></label>
    {#if showHelp}<details class="help"><summary><span class="mobile-symbol" aria-hidden="true">ⓘ</span><span class="toolbar-label">{t($locale, 'chatTextAttachment.helpTitle')}</span></summary><p>{t($locale, 'chatTextAttachment.help')}</p><p>{t($locale, 'chatTextAttachment.dropPaste')}</p></details>{/if}
    </div>
    {#if reading}<p role="status">{t($locale, 'chatTextAttachment.reading')}</p>{/if}
    {#if error}<p class="error" role="alert">{t($locale, error)}</p>{/if}
    {#if attachments.length}<ul bind:this={list}>{#each attachments as item (item.id)}<li>
        <button type="button" title={item.name} onclick={() => preview = preview === item.id ? null : item.id} aria-expanded={preview === item.id}>{item.name}</button>
        <small>{Math.ceil(item.byteCount / 1024).toLocaleString($locale)} KiB{item.clipped ? ` · ${t($locale, 'chatTextAttachment.clipped')}` : ''}</small>
        <button type="button" onclick={event => remove(item.id, event.currentTarget)} disabled={busy || reading} aria-label={t($locale, 'chatTextAttachment.remove', { name: item.name })}>×</button>
        {#if preview === item.id}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable preview must support keyboard reading.) -->
            <pre role="region" tabindex="0" aria-label={t($locale, 'chatTextAttachment.preview', { name: item.name })}>{item.text}</pre>
        {/if}
    </li>{/each}</ul>{/if}
</section>

<style>
    .mobile-symbol { display: none; }
    .text-attachments { display: grid; gap: 6px; min-width: 0; }
    .attachment-toolbar { display: flex; flex-wrap: wrap; align-items: start; gap: 6px 10px; }
    .picker { position: relative; display: inline-flex; align-items: center; min-height: 44px; width: fit-content; max-width: 100%; padding: 5px 8px; border: 1px solid var(--border); border-radius: 7px; font-size: 12px; font-weight: 600; cursor: pointer; }
    .picker.disabled { color: var(--muted); background: var(--panel); cursor: not-allowed; }
    .picker input { position: absolute; width: 1px; height: 1px; overflow: hidden; opacity: 0; }
    .picker:focus-within { outline: 2px solid var(--accent); outline-offset: 2px; }
    .help, small { margin: 0; color: var(--muted); font-size: 11px; line-height: 1.4; }
    .help { flex: 1 1 140px; min-width: 0; }
    .help[open] { flex-basis: 100%; }
    .help summary { cursor: pointer; min-height: 44px; box-sizing: border-box; padding: 14px 0; }
    .help p { margin: 0 0 6px; }
    .error { margin: 0; color: var(--danger); font-size: 12px; }
    ul { display: grid; gap: 5px; padding: 2px; margin: 0; list-style: none; max-height: max(48px, min(20dvh, 160px)); overflow: auto; overscroll-behavior: contain; scrollbar-gutter: stable; }
    li { display: flex; align-items: center; flex-wrap: wrap; gap: 5px; min-width: 0; }
    li > button:first-child { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: start; }
    pre { flex: 1 0 100%; max-height: 180px; overflow: auto; margin: 0; padding: 8px; border: 1px solid var(--border); border-radius: 8px; font-size: 11px; white-space: pre-wrap; overflow-wrap: anywhere; }
    @media (min-width: 0px) {
        .toolbar-label { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
        .mobile-symbol { display: inline; font-size: 22px; font-weight: 400; }
        .picker { width: 44px; height: 44px; padding: 0; justify-content: center; border: 0; border-radius: 12px; }
        .help { flex: 0 1 auto; }
        .help summary { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0; }
        .help summary::-webkit-details-marker { display: none; }
        .help[open] { flex-basis: 100%; }
        .attachment-toolbar { gap: 4px; }
    }
</style>
