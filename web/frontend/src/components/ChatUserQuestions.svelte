<script lang="ts">
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { conversationUrl } from '../lib/chatConversations';
    import { parseUserInputWaits, waitIdentity, type PresentedWait } from '../lib/userInputWait';
    import { locale, t } from '../lib/i18n';
    import ChatUserQuestion from './ChatUserQuestion.svelte';
    let { conversationId, busy, onresolved }: { conversationId: string; busy: boolean; onresolved: () => void } = $props();
    let records = $state<PresentedWait[]>([]);
    let stale = $state(false);
    let loading = false;
    let disposed = false;
    let controller: AbortController | null = null;
    async function load() {
        if (loading || disposed) return;
        const owner = conversationId;
        loading = true; controller = new AbortController();
        try {
            const value = await api(`${conversationUrl(owner)}/user-input-waits`, { signal: controller.signal });
            if (!disposed && owner === conversationId) { records = parseUserInputWaits(value, owner); stale = false; }
        } catch { if (!disposed && owner === conversationId) stale = true; }
        finally { loading = false; }
    }
    function resolved() { onresolved(); void load(); }
    onMount(() => {
        void load();
        const timer = setInterval(() => { if (document.visibilityState === 'visible' && (busy || records.length || stale)) void load(); }, 3000);
        return () => { disposed = true; clearInterval(timer); controller?.abort(); };
    });
</script>
{#each records as record (waitIdentity(record))}
    <ChatUserQuestion {record} {stale} onresolved={resolved} onrefresh={() => { records = []; void load(); }} />
{/each}
{#if stale}<p role="status">{t($locale, 'chatActivity.questionsUnavailable')} <button onclick={() => load()}>{t($locale, 'chatActivity.refresh')}</button></p>{/if}
