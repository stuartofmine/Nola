<script lang="ts">
    import { confirmInPlaceCompletion, type InPlaceResendCompletion } from '../lib/inPlaceResend.ts';
    import { onDestroy, tick } from 'svelte';
    import ActionDisclosure from './ActionDisclosure.svelte';
    import ChatMessageEditDialog from './ChatMessageEditDialog.svelte';
    import { createEditedResendDraft } from '../lib/editedResend.ts';
    import type { EditedExpertResendCompletion } from '../lib/expertMessage.ts';
    import type { EditedResendSourceSnapshot, EditedResendDraft, EditedResendCompletion } from '../lib/editedResend.ts';
    import { editedResendLabel } from '../lib/editedResendLocale.ts';
    import { locale, t } from '../lib/i18n';
    import type { TimelineMessage } from '../lib/chatTimeline';
    import { messageCopyContent, messageMutationId } from '../lib/chatMessageActions';
    let { mentionDirectory=[], mentionMembers=[], loadMentions, mentionEpoch, message, ownerKey, busy = false, selected = false, onselect, onoperation, editSource, oneditresend, ownerCurrent }: {
        mentionDirectory?:import('../lib/agentRooms').PersistentAgent[]; mentionMembers?:import('../lib/agentRooms').RoomParticipant[];
        loadMentions?:()=>Promise<void>; mentionEpoch?:()=>string|null;
        message: TimelineMessage; ownerKey: string; busy?: boolean; selected?: boolean; onselect?: () => void;
        onoperation?: (operation: 'branch' | 'truncate', messageId: string) => void | Promise<string | void>;
        editSource?: EditedResendSourceSnapshot;
        oneditresend?: (draft: EditedResendDraft) => Promise<EditedResendCompletion | EditedExpertResendCompletion | InPlaceResendCompletion>;
        ownerCurrent?: () => boolean;
    } = $props();
    let editing = $state<{ownerKey:string; sourceKey:string; draft:EditedResendDraft} | null>(null);
    const editSourceKey = $derived(editSource ? JSON.stringify([editSource.hostId,editSource.clientId,editSource.conversationId,
        editSource.conversationRevision,editSource.messageId,editSource.messageRevision]) : '');
    const editOwnerChanged = $derived(!!editing && (editing.ownerKey!==ownerKey || editing.sourceKey!==editSourceKey || !!ownerCurrent && !ownerCurrent()));
    const editable = $derived(!!oneditresend && !!editSource && message.role==='user' && message.state==='completed'
        && message.canonical?.message_id===editSource.messageId && message.canonical.revision===editSource.messageRevision);
    function openEditor() {
        if (!editable || busy || !editSource || ownerCurrent && !ownerCurrent()) return;
        try { editing={ownerKey,sourceKey:editSourceKey,draft:createEditedResendDraft(editSource)}; }
        catch(cause) { notice=cause instanceof Error?cause.message:String(cause); noticeError=true; }
    }
    async function submitEditor(draft:EditedResendDraft) {
        if (!oneditresend || busy || editOwnerChanged || ownerCurrent && !ownerCurrent()) throw new Error('Edit owner changed or is running');
        return oneditresend(draft);
    }
    let notice = $state('');
    let noticeError = $state(false);
    let copying = $state(false);
    let overflow = $state<ActionDisclosure>();
    let manualCopy = $state<string | null>(null);
    let copyField = $state<HTMLTextAreaElement>();
    const source = $derived(messageCopyContent(message, 'markdown'));
    const unavailable = $derived(!source || message.role === 'assistant' && ['running', 'pending'].includes(message.state));
    const mutationId = $derived(messageMutationId(message));
    const contentKey = $derived(JSON.stringify([ownerKey, message.id, message.role, message.state, source]));
    let generation = 0;
    let copyFeedback: ReturnType<typeof setTimeout> | null = null;
    function clearCopyFeedback() {
        if (copyFeedback !== null) clearTimeout(copyFeedback);
        copyFeedback = null;
    }
    $effect.pre(() => {
        contentKey;
        generation++; clearCopyFeedback();
        notice = ''; noticeError = false; manualCopy = null; copying = false;
    });
    onDestroy(() => { generation++; clearCopyFeedback(); });
    async function mutate(operation: 'branch' | 'truncate', button: HTMLButtonElement) {
        if (busy || !mutationId || !onoperation) return;
        const capturedId = mutationId;
        const disclosure = button.closest('details');
        const fallback = disclosure?.querySelector('summary');
        const epoch = generation, key = contentKey;
        notice = ''; noticeError = false;
        const failure = await onoperation(operation, capturedId);
        if (epoch === generation && key === contentKey && capturedId === mutationId && failure) { notice = failure; noticeError = true; }
        await tick();
        // Native dialog restoration happens while the operation lock still disables
        // the opener. Restore only a surviving boundary after that lock releases.
        if (epoch === generation && key === contentKey && capturedId === mutationId) {
            if ((!disclosure || disclosure.open) && button.isConnected && !button.disabled && button.getClientRects().length) button.focus();
            else if (fallback?.isConnected && fallback.getClientRects().length) fallback.focus();
        }
    }
    async function copy(format: 'plain' | 'markdown') {
        if (unavailable || copying) return;
        clearCopyFeedback(); copying = true; notice = ''; noticeError = false; manualCopy = null;
        const epoch = generation, key = contentKey;
        const current = () => epoch === generation && key === contentKey;
        const content = messageCopyContent(message, format);
        try {
            await navigator.clipboard.writeText(content);
            if (current()) {
                notice = 'chatPage.textCopied'; noticeError = false;
                copyFeedback = setTimeout(() => { copyFeedback = null; if (current()) notice = ''; }, 1800);
            }
        }
        catch {
            if (current()) {
                notice = 'chatPage.copyFailed'; noticeError = true; manualCopy = content;
                await tick();
                if (current() && copyField?.isConnected) { copyField.focus(); copyField.select(); }
            }
        }
        finally { if (current()) copying = false; }
    }
</script>

{#if source || onoperation && mutationId || onselect || editable}
    <div class="message-actions">
        {#if source}
            <button class="copy-action" type="button" aria-busy={copying} disabled={unavailable || copying} onclick={() => copy('plain')}><svg aria-hidden="true" viewBox="0 0 24 24">{#if notice === 'chatPage.textCopied'}<path d="m5 12 4 4L19 6"/>{:else}<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V4H4v12h4"/>{/if}</svg>{t($locale, 'shell.markdown.copy')}</button>
        {/if}
        {#if editable}<button type="button" disabled={busy || !!ownerCurrent && !ownerCurrent()} onclick={openEditor}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="m4 16 12-12 4 4L8 20H4zM14 6l4 4"/></svg>{editedResendLabel($locale,'edit')}</button>{/if}
        {#if source && message.role === 'assistant' || onoperation && mutationId || onselect}
            <ActionDisclosure bind:this={overflow} iconOnly label={t($locale, 'chatPage.messageActions')}>
                {#if onselect}<button type="button" aria-pressed={selected} onclick={() => { overflow?.close(true); onselect?.(); }}>{t($locale, 'chatPage.selectMessage')}</button>{/if}
                {#if source && message.role === 'assistant'}<button class="copy-action" type="button" disabled={unavailable || copying} onclick={() => { overflow?.close(true); void copy('markdown'); }}>{t($locale, 'chatPage.copyMarkdown')}</button>{/if}
                {#if onoperation && mutationId}
                    <button type="button" disabled={busy} onclick={event => mutate('branch', event.currentTarget)}>{t($locale, 'chatConversations.branchHere')}</button>
                    <button type="button" disabled={busy} onclick={event => mutate('truncate', event.currentTarget)}>{t($locale, 'chatConversations.truncateHere')}</button>
                {/if}
            </ActionDisclosure>
        {/if}
        {#if notice}<span role={noticeError ? 'alert' : 'status'} class:error={noticeError}>{t($locale, notice)}</span>{/if}
        {#if manualCopy !== null}<textarea bind:this={copyField} readonly value={manualCopy} aria-label={t($locale, 'shell.markdown.copy')} rows="3"></textarea>{/if}
    </div>
{/if}

{#if editing}
    <ChatMessageEditDialog initial={editing.draft} {mentionDirectory} {mentionMembers} {loadMentions} {mentionEpoch} {busy} ownerChanged={editOwnerChanged} onsubmit={submitEditor} onclose={()=>editing=null} />
{/if}

<style>
    .message-actions { display: flex; flex-wrap: wrap; gap: 2px; align-items: center; margin-top: 5px; font-size: 12px; color: var(--muted); }
    button { display:inline-flex; align-items:center; gap:5px; min-height: var(--control-hit-size); padding: 6px 8px; font-size: 12px; border: 0; box-shadow: none; background: transparent; color: var(--muted); }
    button:hover:not(:disabled) { background: var(--panel); color: var(--content); }
    button:active:not(:disabled) { background: var(--accent-subtle); color: var(--content); }
    button.copy-action:hover:not(:disabled) { background: var(--panel); color: var(--content); }
    button.copy-action:active:not(:disabled) { background: var(--accent-subtle); color: var(--content); }
    svg { flex-shrink:0; width:14px; height:14px; fill:none; stroke:currentColor; stroke-width:1.6; }
    textarea { flex: 1 0 100%; min-width: 0; max-width: 100%; max-height: 180px; box-sizing: border-box; background: var(--canvas); color: inherit; border: 1px solid var(--border); border-radius: 7px; padding: 8px; }
    @media (max-width: 760px) {
        .message-actions { gap: 2px; margin-top: 4px; }
        button { border-color: transparent; background: transparent; color: var(--muted); padding: 6px; }
        button:active:not(:disabled) { background: var(--panel); color: inherit; }
    }
</style>
