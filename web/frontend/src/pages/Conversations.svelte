<script lang="ts">
    import ChatConversationManager from '../components/ChatConversationManager.svelte';
    import { navigateWorkspace } from '../lib/workspaceNavigation';
    import { locale, t } from '../lib/i18n';
</script>
<svelte:head><title>{t($locale, 'chatConversations.title')} — Nola</title></svelte:head>
<main id="page-main" tabindex="-1" class="conversation-list-page">
    <ChatConversationManager layout="page" activeId={null} generation={0} showHistoryActions={false}
        onselect={conversation => navigateWorkspace(`/chat?conversation=${encodeURIComponent(conversation.conversationId)}`)} />
</main>
<style>
    .conversation-list-page { min-height: 100dvh; max-width: 760px; margin-inline: auto; padding: max(16px, env(safe-area-inset-top)) 16px max(16px, env(safe-area-inset-bottom)); background: var(--canvas); }
    @media (min-width: 360px) and (max-width: 700px) { .conversation-list-page { padding-inline: 20px; } }
</style>
