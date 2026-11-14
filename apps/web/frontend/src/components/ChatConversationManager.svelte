<script lang="ts">
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { dismissibleDisclosure } from '../lib/dismissibleDisclosure';
    import { onMount, tick, untrack } from 'svelte';
    import { api } from '../lib/api';
    import { conversationErrorKey } from '../lib/conversationErrors';
    import { locale, t } from '../lib/i18n';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import ConversationRowDetails from './ConversationRowDetails.svelte';
    import ExpertChildHistory from './ExpertChildHistory.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { parseChatConversation, parseChatConversations, conversationUrl, conversationTitleRequest, conversationBranchRequest, conversationTruncateRequest, parseConversationExport, sameConversationOperation } from '../lib/chatConversations';
    import type { ChatConversation, ConversationOperationOwner } from '../lib/chatConversations';
    import { conversationPinRequest, conversationPinConfirmed } from '../lib/chatConversations';
    import { committedConversationConfiguration } from '../lib/conversationConfigurationCommit';
    interface Change { type: 'created' | 'renamed' | 'deleted' | 'branched' | 'truncated' | 'refreshed'; conversationId: string; conversation?: ChatConversation }
    interface Props { layout?: 'page' | 'sidebar'; highlightSelection?: boolean; navigationDisabled?: boolean; activeId: string | null; generation: number; busy?: boolean; hasMessages?: boolean; showHistoryActions?: boolean; selectedMessageId?: string | null; messageIds?: string[]; writing?: boolean; confirm?: ConfirmationHandle['ask']; onselect: (conversation: ChatConversation) => void; onchange?: (event: Change) => void }
    let { layout = 'sidebar', highlightSelection = true, navigationDisabled = false, activeId, generation, busy = false, hasMessages = false, showHistoryActions = true, selectedMessageId = null, messageIds = [], writing = $bindable(false), confirm, onselect, onchange }: Props = $props();
    let createButton: HTMLButtonElement;
    export function focusCreateEntry() { createButton?.focus(); }
    let confirmation = $state<ConfirmationHandle>();
    let conversations = $state<ChatConversation[]>([]);
    let loading = $state(false);
    let query = $state('');
    let error = $state('');
    let loadError = $state('');
    let title = $state('');
    let renameBaseline = $state('');
    let renameRevision = $state(0);
    let editing = $state<string | null>(null);
    let renameInput = $state<HTMLInputElement>();
    let renameOpener: HTMLElement | undefined;
    let listElement: HTMLDivElement;
    async function rename(conversation: ChatConversation, opener?: HTMLElement) {
        if (locked) return;
        const captured = owner();
        if (editing === conversation.conversationId) { renameInput?.focus(); return; }
        if (!await discardRename() || !current(captured)) return;
        const menu = opener?.closest('details'); if (menu) menu.open = false;
        renameOpener = menu?.querySelector('summary') ?? undefined;
        title = conversation.title; renameBaseline = conversation.title; renameRevision = conversation.revision; editing = conversation.conversationId;
        await tick(); renameInput?.focus(); renameInput?.select();
    }
    async function closeRename() {
        editing = null; await tick();
        if (renameOpener?.isConnected) renameOpener.focus();
        renameOpener = undefined;
    }
    async function deleteRow(conversation: ChatConversation, opener: HTMLButtonElement) {
        const captured = owner();
        await mutate('delete', undefined, conversation);
        await tick();
        if (!current(captured)) return;
        if (opener.isConnected && !opener.disabled) {
            const menu = opener.closest('details');
            if (menu) menu.open = true;
            opener.focus();
        }
        else listElement?.querySelector<HTMLElement>('.conversation[aria-current]')?.focus();
    }
    let disposed = false;
    let mounted = false;
    let loadVersion = $state(0);
    let loadController: AbortController | undefined;
    const locked = $derived(navigationDisabled || busy || writing);
    const renameDirty = $derived(editing !== null && title !== renameBaseline);
    const active = $derived(conversations.find(item => item.conversationId === activeId));
    const visible = $derived(conversations.filter(item => !query.trim() || item.title.toLocaleLowerCase($locale).includes(query.trim().toLocaleLowerCase($locale))).sort((a, b) => Number(b.isPinned) - Number(a.isPinned)));
    const owner = (): ConversationOperationOwner => ({ conversationId: activeId, generation });
    const current = (captured: ConversationOperationOwner) => !disposed && sameConversationOperation(captured, owner());
    function conversationTime(value: string): string {
        const date = new Date(value);
        if (!Number.isFinite(date.getTime())) return '—';
        const today = new Date();
        return new Intl.DateTimeFormat($locale, date.toDateString() === today.toDateString()
            ? { hour: '2-digit', minute: '2-digit' }
            : { month: 'short', day: 'numeric', ...(date.getFullYear() !== today.getFullYear() ? { year: 'numeric' as const } : {}) }).format(date);
    }
    function conversationGroup(conversation: ChatConversation): string {
        if (conversation.isPinned) return t($locale, 'chatConversations.pinned');
        if (layout === 'sidebar') return t($locale, 'chatConversations.recent');
        const date = new Date(conversation.updatedAt), today = new Date(), yesterday = new Date();
        yesterday.setDate(today.getDate() - 1);
        if (date.toDateString() === today.toDateString()) return new Intl.RelativeTimeFormat($locale, { numeric: 'auto' }).format(0, 'day');
        if (date.toDateString() === yesterday.toDateString()) return new Intl.RelativeTimeFormat($locale, { numeric: 'auto' }).format(-1, 'day');
        return Number.isFinite(date.getTime()) ? new Intl.DateTimeFormat($locale, { month: 'long', year: 'numeric' }).format(date) : t($locale, 'chatConversations.recent');
    }
    async function discardRename(): Promise<boolean> {
        if (!renameDirty) return true;
        const captured = owner();
        writing = true;
        try {
            const accepted = await (confirm ?? confirmation!.ask)({ title: t($locale, 'chatConversations.discardRenameTitle'), description: t($locale, 'chatConversations.discardRenameBody'), confirmLabel: t($locale, 'chatConversations.discardRename'), destructive: true });
            if (!current(captured)) return false;
            if (accepted) editing = null;
            return accepted;
        } finally {
            if (!disposed) writing = false;
            await tick();
            if (current(captured) && editing) renameInput?.focus();
        }
    }
    async function selectConversation(conversation: ChatConversation) {
        const captured = owner();
        if (navigationDisabled || writing || !await discardRename() || !current(captured)) return;
        onselect(conversation);
    }
    async function pin(target: ChatConversation, opener: HTMLButtonElement) {
        if (locked || !conversations.some(item => item.conversationId === target.conversationId)) return;
        const captured = owner();
        const expectedPinned = target.isPinned;
        writing = true; error = '';
        try {
            const path = `${conversationUrl(target.conversationId)}/configuration`;
            const configuration = await api(path);
            if (!current(captured)) return;
            const request = conversationPinRequest(configuration, expectedPinned);
            const receipt = committedConversationConfiguration(await api(path, { method: 'PATCH', body: JSON.stringify(request) }));
            if (!conversationPinConfirmed(receipt, request)) throw new Error();
            if (current(captured)) await refresh();
        } catch (cause) { if (current(captured)) error = conversationErrorKey(cause, 'write'); }
        finally {
            if (!disposed) writing = false;
            await tick();
            if (current(captured) && opener.isConnected) opener.focus();
        }
    }
    export async function refresh() {
        const captured = owner();
        const version = ++loadVersion;
        loadController?.abort(); const request = new AbortController(); loadController = request;
        loading = true;
        try { const result = parseChatConversations(await api('/api/chat/conversations', { signal: request.signal })); if (current(captured) && version === loadVersion) {
                conversations = result; loadError = '';
                const selected = result.find(item => item.conversationId === activeId);
                if (selected) onchange?.({type: 'refreshed', conversationId: selected.conversationId, conversation: selected});
            } }
        catch (cause) { if (current(captured) && version === loadVersion && !request.signal.aborted) loadError = conversationErrorKey(cause, 'load'); }
        finally { if (!disposed && version === loadVersion) loading = false; }
    }
    $effect(() => { activeId; generation; if (mounted) untrack(() => { editing = null; error = ''; loadError = ''; void refresh(); }); });
    onMount(() => { mounted = true; void refresh(); return () => { disposed = true; loadController?.abort(); }; });
    export async function mutateMessage(operation: 'branch' | 'truncate', messageId: string, captured: ConversationOperationOwner) {
        if (!current(captured) || !messageIds.includes(messageId)) return;
        return await mutate(operation, messageId);
    }
    export async function createConversation() { return await mutate('create'); }
    async function mutate(operation: 'create' | 'rename' | 'delete' | 'branch' | 'truncate', explicitMessageId?: string, rowTarget?: ChatConversation) {
        if (navigationDisabled || writing || busy && operation !== 'create') return;
        const captured = owner();
        if (operation !== 'rename' && (!await discardRename() || !current(captured))) return;
        const target = rowTarget ?? active;
        if (rowTarget && (!['rename', 'delete'].includes(operation) || !conversations.some(item => item.conversationId === rowTarget.conversationId))) return;
        const messageId = explicitMessageId ?? selectedMessageId;
        if (operation !== 'create' && !target) return;
        if ((operation === 'branch' && !hasMessages) || (operation === 'truncate' && !messageId)) return;
        let body: unknown;
        try { body = operation === 'create' ? {} : operation === 'rename' ? conversationTitleRequest(title, renameRevision) : operation === 'branch' ? conversationBranchRequest(messageId) : operation === 'truncate' ? conversationTruncateRequest(messageId!) : undefined; }
        catch { error = 'chatConversations.invalidTitle'; return; }
        writing = true; error = '';
        try {
            if (operation === 'delete' || operation === 'truncate' || operation === 'branch') {
                const descriptionKey = operation === 'branch' && !messageId ? 'chatConversations.branchWholeBody' : `chatConversations.${operation}Body`;
                const confirmKey = operation === 'branch' && !messageId ? 'chatConversations.branchWhole' : `chatConversations.${operation}`;
                const accepted = await (confirm ?? confirmation!.ask)({ title: t($locale, `chatConversations.${operation}Title`), description: t($locale, descriptionKey, { name: target!.title }), confirmLabel: t($locale, confirmKey), destructive: operation !== 'branch' });
                const boundaryValid = explicitMessageId === undefined ? selectedMessageId === messageId : messageIds.includes(explicitMessageId);
                if (!accepted || !current(captured) || busy || (operation !== 'delete' && !boundaryValid)) return;
            }
            if (!current(captured)) return;
            const path = operation === 'create' ? '/api/chat/conversations' : `${conversationUrl(target!.conversationId)}${operation === 'branch' || operation === 'truncate' ? `/${operation}` : ''}`;
            const result = await api<Record<string, unknown>>(path, { method: operation === 'delete' ? 'DELETE' : operation === 'rename' ? 'PATCH' : 'POST', ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
            let conversation: ChatConversation | undefined;
            if (['create', 'rename', 'branch'].includes(operation)) conversation = parseChatConversation(operation === 'branch' ? result.conversation : result);
            else if (result.ok !== true) throw new Error();
            if (operation === 'rename' && conversation!.conversationId !== target!.conversationId) throw new Error();
            if (disposed) return;
            const type = ({ create: 'created', rename: 'renamed', delete: 'deleted', branch: 'branched', truncate: 'truncated' } as const)[operation];
            onchange?.({ type, conversationId: operation === 'create' ? conversation!.conversationId : target!.conversationId, ...(conversation ? { conversation } : {}) });
            if (current(captured)) {
                if (operation === 'rename') await closeRename(); else editing = null;
                if ((operation === 'create' || operation === 'branch') && conversation) onselect(conversation);
            }
            await refresh();
        } catch (cause) { if (current(captured)) { error = conversationErrorKey(cause, operation === 'create' ? 'create' : 'write'); return error; } }
        finally { if (!disposed) writing = false; }
    }
    async function download(format: 'markdown' | 'json', target = active) {
        if (locked || !target || !conversations.some(item => item.conversationId === target.conversationId)) return;
        const captured = owner();
        writing = true; error = '';
        try {
            const result = parseConversationExport(await api(`${conversationUrl(target.conversationId)}/export?format=${format}`));
            if (!current(captured)) return;
            const href = URL.createObjectURL(new Blob([result.content], { type: format === 'json' ? 'application/json;charset=utf-8' : 'text/markdown;charset=utf-8' }));
            const link = document.createElement('a'); link.href = href; link.download = result.filename; link.click(); setTimeout(() => URL.revokeObjectURL(href), 0);
        } catch { if (current(captured)) error = 'chatConversations.exportFailed'; }
        finally { if (!disposed) writing = false; }
    }
</script>

<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (renameDirty) { event.preventDefault(); event.returnValue = ''; } }} />
{#if !confirm}<ConfirmationDialog bind:this={confirmation} />{/if}
{#snippet createEntry()}
    <button bind:this={createButton} class="new-conversation" disabled={navigationDisabled || writing} onclick={() => mutate('create')}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" /></svg><span>{t($locale, 'shell.conversations.new')}</span></button>
{/snippet}
<section class="conversation-manager" class:page-list={layout === 'page'} class:sidebar-list={layout === 'sidebar'} aria-label={t($locale, 'chatInspector.list.conversations')}>
    <header class="list-heading">
        {#if layout === 'page'}<h1>{t($locale, 'chatInspector.list.conversations')}</h1>{@render createEntry()}
        {:else}<h2>{t($locale, 'chatInspector.list.conversations')}</h2>{/if}
    </header>
    <div class="search-row">
        <label class="conversation-search"><span class="sr-only">{t($locale, 'chatConversations.search')}</span><svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="10.5" cy="10.5" r="7"/><path d="m16 16 5 5"/></svg><input type="search" bind:value={query} placeholder={t($locale, 'chatConversations.search')} title={t($locale, 'chatConversations.localSearch')} /></label>
        <button class="refresh-list" aria-label={t($locale, 'chatConversations.refresh')} title={t($locale, 'chatConversations.refresh')} disabled={loading || locked} onclick={refresh}><svg aria-hidden="true" viewBox="0 0 24 24"><path d="M20 7v5h-5M20 12a8 8 0 1 0-2 5" /></svg></button>
    </div>

    {#if error || loadError}<p class="error" role="alert">{t($locale, error || loadError)}</p>{/if}
    {#if loading}<p role="status" class:sr-only={conversations.length > 0}>{t($locale, 'chatConversations.loading')}</p>{/if}
    <div class="conversation-list" bind:this={listElement}>{#each visible as conversation, index (conversation.conversationId)}
        {#if !query.trim() && (index === 0 || conversationGroup(conversation) !== conversationGroup(visible[index - 1]))}<h3 class="group-title">{conversationGroup(conversation)}</h3>{/if}
        <div class="conversation-row">
            <button class="conversation" aria-current={highlightSelection && activeId === conversation.conversationId ? 'true' : undefined} disabled={navigationDisabled || writing} onclick={() => selectConversation(conversation)}>
                <span class="conversation-heading"><span>{conversation.title || t($locale, 'chatConversations.untitled')}</span>{#if layout === 'page'}<small title={new Date(conversation.updatedAt).toLocaleString($locale)}>{conversationTime(conversation.updatedAt)}</small>{/if}</span>
                {#if layout === 'page'}<ConversationRowDetails conversationId={conversation.conversationId} version={JSON.stringify([generation, loadVersion, conversation.updatedAt, conversation.revision])} />{/if}
            </button>
            <details class="row-actions" use:dismissibleDisclosure>
                <summary aria-label={t($locale, 'chatConversations.actionsFor', {name: conversation.title || t($locale, 'chatConversations.untitled')})} aria-disabled={locked} onclick={event => { if (locked) event.preventDefault(); }}>⋯</summary>
                <div class="row-action-buttons">
                    <button disabled={locked} onclick={event => pin(conversation, event.currentTarget)}>{t($locale, conversation.isPinned ? 'chatConversations.unpin' : 'chatConversations.pin')}</button>
                    <button disabled={locked} onclick={event => rename(conversation, event.currentTarget)}>{t($locale, 'chatConversations.rename')}</button>
                    <button disabled={locked} onclick={() => download('markdown', conversation)}>{t($locale, 'chatConversations.exportMarkdown')}</button>
                    <button disabled={locked} onclick={() => download('json', conversation)}>{t($locale, 'chatConversations.exportJson')}</button>
                    {#if activeId === conversation.conversationId && showHistoryActions}
                        <div class="history-actions">
                            <button disabled={locked || !hasMessages} onclick={() => mutate('branch')}>{t($locale, selectedMessageId ? 'chatConversations.branch' : 'chatConversations.branchWhole')}</button>
                            <button disabled={locked || !selectedMessageId} onclick={() => mutate('truncate')}>{t($locale, 'chatConversations.truncate')}</button>
                            <p class="muted">{t($locale, 'chatConversations.selectionHint')}</p>
                        </div>
                    {/if}
                    <button class="danger" disabled={locked} onclick={event => deleteRow(conversation, event.currentTarget)}>{t($locale, 'chatConversations.delete')}</button>
                </div>
            </details>
            {#if editing === conversation.conversationId}
                <form class="row rename-form" onsubmit={event => { event.preventDefault(); void mutate('rename', undefined, conversation); }}>
                    <label>{t($locale, 'chatConversations.titleField')}<input bind:this={renameInput} bind:value={title} disabled={locked} required onkeydown={event => { if (event.key === 'Escape' && !locked) { event.preventDefault(); void closeRename(); } }} /></label>
                    <button type="submit" disabled={locked}>{t($locale, 'chatConversations.save')}</button>
                    <button type="button" disabled={locked} onclick={closeRename}>{t($locale, 'chatConversations.cancel')}</button>
                </form>
            {/if}
            {#key `${generation}:${conversation.conversationId}`}<ExpertChildHistory parentId={conversation.conversationId} disabled={locked}/>{/key}
        </div>
    {/each}</div>
    {#if !loading && !loadError && !visible.length}<p class="muted">{t($locale, 'chatConversations.empty')}</p>{/if}
    {#if layout === 'sidebar'}{@render createEntry()}{/if}
</section>
<style>
    .conversation-manager { display: grid; gap: 12px; min-width: 0; }
    .list-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-width: 0; }
    .list-heading h1 { font-size: 28px; line-height: 36px; font-weight: 650; margin: 0; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .list-heading h2 { font-size: 12px; line-height: 20px; font-weight: 600; margin: 0; }
    .new-conversation { display: inline-flex; align-items: center; justify-content: flex-start; gap: 6px; min-height: 44px; max-width: 100%; padding: 6px 8px; border: 0; border-radius: 6px; background: transparent; color: var(--muted); text-align: start; }
    .new-conversation span { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    svg { width: 20px; height: 20px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.5; }
    .search-row { position: relative; display: flex; min-width: 0; }
    .conversation-search { display: flex; align-items: center; gap: 10px; min-width: 0; width: 100%; padding-left: 12px; padding-right: 44px; background: var(--panel); border-radius: 8px; color: var(--muted); }
    .conversation-search input { min-height: 44px; padding: 8px 0; border: 0; border-radius: 0; background: transparent; font-size: 15px; }
    .refresh-list { position: absolute; inset: 0 0 0 auto; display: grid; place-items: center; padding: 0; width: 44px; min-height: 44px; border: 0; background: transparent; color: var(--muted); }
    .refresh-list svg { width: 16px; height: 16px; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    .group-title { display: flex; align-items: center; gap: 12px; font-size: 12px; line-height: 20px; color: var(--muted); margin: 12px 0 4px; font-weight: 400; }
    .page-list .group-title::after { content: ''; flex: 1; height: 1px; background: var(--border); }
    .conversation-list { display: grid; gap: 3px; max-height: 340px; overflow: auto; }
    .conversation-row { position: relative; display: grid; grid-template-columns: minmax(0, 1fr); min-width: 0; }
    .conversation { display: grid; align-content: center; gap: 4px; min-width: 0; width: 100%; border: 0; border-radius: 6px; background: transparent; padding: 8px 40px 8px 12px; min-height: 40px; text-align: left; overflow-wrap: anywhere; }
    .conversation[aria-current] { background: var(--panel); color: var(--content); }
    .conversation-heading { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
    .conversation-heading > span { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 400; }
    .conversation-heading small { flex-shrink: 0; max-width: 30%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--muted); font-size: 12px; font-weight: 400; }
    .page-list { gap: 20px; }
    .page-list .list-heading { min-height: 44px; flex-wrap: wrap; }
    .page-list .list-heading h1 { white-space: normal; overflow-wrap: anywhere; }
    .page-list .new-conversation { flex: 0 0 auto; max-width: 100%; padding-inline: 4px; }
    .page-list .conversation-list { max-height: none; overflow: visible; gap: 0; }
    .page-list .conversation-row { border-bottom: 1px solid var(--border); }
    .page-list .conversation { padding: 16px 0; gap: 6px; border-radius: 0; min-height: 80px; align-content: start; }
    .page-list .conversation-heading > span { font-size: 17px; line-height: 24px; font-weight: 550; }
    .page-list .conversation :global(.row-details) { padding-right: 36px; }
    .sidebar-list .list-heading { min-height: 24px; padding-inline: 12px; }
    .sidebar-list .conversation-search { padding-left: 10px; gap: 0; }
    .sidebar-list .conversation-search > svg { display: none; }
    .sidebar-list .conversation-search input { min-height: 36px; font-size: 12px; }
    .sidebar-list .refresh-list { min-height: 36px; width: 36px; }
    .sidebar-list .group-title { margin-left: 12px; margin-top: 4px; }
    .sidebar-list .new-conversation { justify-self: start; color: var(--content); }
    @media (max-width: 700px), (pointer: coarse) { .sidebar-list .conversation, .sidebar-list .conversation-search input, .sidebar-list .refresh-list { min-height: 44px; } }
    form label { flex: 1; min-width: 160px; }
    .muted { font-size: 12px; margin: 0; }
    .row-actions { display: block; grid-column: 1 / -1; min-width: 0; }
    .row-actions > summary { position: absolute; top: 0; right: 0; width: 44px; min-height: 44px; display: grid; place-items: center; cursor: pointer; list-style: none; border: 0; border-radius: var(--radius-control); }
    .page-list .row-actions > summary { top: auto; bottom: 4px; width: 36px; min-height: 44px; color: var(--muted); }
    .page-list .row-actions[open] > summary { top: 40px; bottom: auto; }
    @media (hover: hover) { .row-actions:not([open]) > summary { opacity: 0; } .conversation-row:hover .row-actions > summary, .conversation-row:focus-within .row-actions > summary { opacity: 1; } }
    .row-actions > summary::-webkit-details-marker { display: none; }
    .row-actions > summary[aria-disabled='true'] { color: var(--muted); background: var(--panel); cursor: not-allowed; }
    .row-action-buttons, .rename-form { grid-column: 1 / -1; min-width: 0; }
    .row-action-buttons { display: grid; gap: 2px; padding: 4px; max-height: min(360px, 50dvh); overflow-y: auto; scroll-padding-block: 6px; border-top: 1px solid var(--border); }
    .history-actions { display: grid; gap: 2px; width: 100%; border-top: 1px solid var(--border); padding-top: 4px; }
    .history-actions p { width: 100%; padding: 6px 8px; }
    .row-action-buttons button { width: 100%; min-height: var(--control-hit-size); padding: 6px 8px; text-align: start; border: 0; background: transparent; }
    .row-action-buttons button:focus-visible { outline-offset: -2px; }
    .row-action-buttons > .danger { border-top: 1px solid var(--border); border-radius: 0; }
    .rename-form { flex-wrap: wrap; padding: 6px; }
</style>
