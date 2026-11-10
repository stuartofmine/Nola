<script lang="ts">
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    import { locale, t } from '../lib/i18n';
    import { parseChatConversations, type ChatConversation } from '../lib/chatConversations';
    import { knowledgeDocument, knowledgeUrl } from '../lib/chatKnowledge';
    import { conversationHref } from '../lib/conversations';
    import Markdown from './Markdown.svelte';

    const api = workspacePageApi(unscopedApi);
    let conversations = $state<ChatConversation[]>([]);
    let selected = $state('');
    let points = $state<string | null>(null);
    let loading = $state(false), failed = $state(false), loaded = $state(false);
    let disposed = false, revision = 0;
    let controller: AbortController | undefined;
    async function loadList() {
        controller?.abort(); const request = new AbortController(); controller = request;
        const token = ++revision; loading = true; failed = false;
        try {
            const rows = parseChatConversations(await api('/api/chat/conversations', {signal: request.signal}));
            if (disposed || token !== revision) return;
            conversations = rows; loaded = true;
        } catch { if (!disposed && token === revision) failed = true; }
        finally { if (!disposed && token === revision) loading = false; }
    }
    async function loadPoints() {
        controller?.abort(); const request = new AbortController(); controller = request;
        const token = ++revision, id = selected;
        points = null; failed = false; loading = !!id;
        if (!id) return;
        try {
            const snapshot = knowledgeDocument(await api(knowledgeUrl(id), {signal: request.signal}), id);
            if (disposed || token !== revision || id !== selected) return;
            points = snapshot.document.knowledgePoints;
        } catch { if (!disposed && token === revision) failed = true; }
        finally { if (!disposed && token === revision) loading = false; }
    }
    onMount(() => { void loadList(); return () => { disposed = true; revision++; controller?.abort(); }; });
</script>

<section class="knowledge-overview card" aria-busy={loading}>
    <h2>{t($locale, 'chatKnowledge.knowledgePoints')}</h2>
    <p class="muted">{t($locale, 'shell.informationKnowledgeHint')}</p>
    <label>{t($locale, 'chatConversations.title')}
        <select bind:value={selected} onchange={loadPoints} disabled={!loaded}>
            <option value="">{t($locale, 'chatPage.choose')}</option>
            {#each conversations as conversation (conversation.conversationId)}
                <option value={conversation.conversationId}>{conversation.title || t($locale, 'chatConversations.untitled')}</option>
            {/each}
        </select>
    </label>
    {#if loading}<p role="status">{t($locale, 'chatKnowledge.loading')}</p>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, 'shell.pageLoadFailed')}</p><button onclick={() => selected ? loadPoints() : loadList()}>{t($locale, 'shell.retry')}</button>{/if}
    {#if points?.trim()}<div class="knowledge-prose"><Markdown source={points} /></div>{:else if points !== null || loaded && !conversations.length}<p class="muted">{t($locale, 'shell.informationKnowledgeEmpty')}</p>{/if}
    {#if selected}<a class="button" href={conversationHref(selected)}>{t($locale, 'data_sources.usage.conversation')}</a>{/if}
</section>

<style>
    section { display: grid; gap: 16px; min-width: 0; }
    .knowledge-prose { min-width: 0; overflow-wrap: anywhere; }
    .button, button { justify-self: start; min-height: 44px; }
</style>
