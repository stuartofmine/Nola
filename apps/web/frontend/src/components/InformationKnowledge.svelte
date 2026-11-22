<script lang="ts">
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    import { locale, t } from '../lib/i18n';
    import { parseChatConversations, type ChatConversation } from '../lib/chatConversations';
    import { knowledgeDocument, knowledgeUrl } from '../lib/chatKnowledge';
    import { conversationHref } from '../lib/conversations';
    import RefreshButton from './RefreshButton.svelte';
    import Markdown from './Markdown.svelte';

    const api = workspacePageApi(unscopedApi);
    let conversations = $state<ChatConversation[]>([]);
    let selected = $state('');
    let points = $state<string | null>(null);
    let loading = $state(true), failed = $state<'' | 'list' | 'points'>(''), loaded = $state(false);
    let disposed = false, revision = 0;
    let controller: AbortController | undefined;
    async function loadList() {
        controller?.abort(); const request = new AbortController(); controller = request;
        const token = ++revision; loading = true; failed = '';
        try {
            const rows = parseChatConversations(await api('/api/chat/conversations', {signal: request.signal}));
            if (disposed || token !== revision) return;
            conversations = rows; loaded = true;
            if (selected && rows.some(row => row.conversationId === selected)) {
                await loadPoints(true);
            } else {
                selected = ''; points = null;
            }
        } catch { if (!disposed && token === revision) failed = 'list'; }
        finally { if (!disposed && token === revision) loading = false; }
    }
    async function loadPoints(keepSnapshot = false) {
        controller?.abort(); const request = new AbortController(); controller = request;
        const token = ++revision, id = selected;
        if (!keepSnapshot) points = null;
        failed = ''; loading = !!id;
        if (!id) return;
        try {
            const snapshot = knowledgeDocument(await api(knowledgeUrl(id), {signal: request.signal}), id);
            if (disposed || token !== revision || id !== selected) return;
            points = snapshot.document.knowledgePoints;
        } catch { if (!disposed && token === revision) failed = 'points'; }
        finally { if (!disposed && token === revision) loading = false; }
    }
    onMount(() => { void loadList(); return () => { disposed = true; revision++; controller?.abort(); }; });
</script>

<section class="knowledge-overview card" aria-busy={loading}>
    <div class="knowledge-controls">
        {#if conversations.length}
            <label>{t($locale, 'chatConversations.title')}
                <select bind:value={selected} onchange={() => loadPoints()}>
                    <option value="">{t($locale, 'chatPage.choose')}</option>
                    {#each conversations as conversation (conversation.conversationId)}
                        <option value={conversation.conversationId}>{conversation.title || t($locale, 'chatConversations.untitled')}</option>
                    {/each}
                </select>
            </label>
        {/if}
        <RefreshButton {loading} refresh={loadList} />
    </div>
    {#if loading && (!loaded || selected && points === null)}<p role="status">{t($locale, 'chatKnowledge.loading')}</p>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, 'shell.pageLoadFailed')}</p><button onclick={() => failed === 'points' ? loadPoints(true) : loadList()}>{t($locale, 'shell.retry')}</button>{/if}
    {#if points?.trim()}<div class="knowledge-prose"><Markdown source={points} /></div>{:else if !loading && !failed && (points !== null || loaded && !conversations.length)}<p class="muted">{t($locale, 'shell.informationKnowledgeEmpty')}</p>{/if}
    {#if selected}<a class="button" href={conversationHref(selected)}>{t($locale, 'data_sources.usage.conversation')}</a>{/if}
</section>

<style>
    section { display: grid; gap: 16px; min-width: 0; }
    .knowledge-controls { display: flex; flex-wrap: wrap; align-items: end; gap: 12px; }
    .knowledge-controls label { flex: 1 1 16rem; min-width: 0; }
    .knowledge-controls select { width: 100%; }
    .knowledge-prose { min-width: 0; overflow-wrap: anywhere; }
    .button, button { justify-self: start; min-height: 44px; }
</style>
