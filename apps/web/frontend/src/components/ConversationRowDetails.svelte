<script lang="ts">
    import { onMount } from 'svelte';
    import { loadV2Messages } from '../lib/v2Chat';
    import { markdownPlainText } from '../lib/markdown';

    let { conversationId, version }: { conversationId: string; version: string } = $props();
    let element: HTMLSpanElement;
    let visible = $state(false);
    let preview = $state('');
    let completed = '';
    const identity = $derived(JSON.stringify([conversationId, version]));

    onMount(() => {
        const observer = new IntersectionObserver(entries => {
            visible = entries.some(entry => entry.isIntersecting);
        });
        observer.observe(element);
        return () => observer.disconnect();
    });

    $effect(() => {
        identity;
        preview = '';
        completed = '';
    });

    $effect(() => {
        const key = identity;
        const id = conversationId;
        if (!visible || completed === key) return;
        const controller = new AbortController();
        const current = () => !controller.signal.aborted && identity === key && conversationId === id;
        void loadV2Messages(id, controller.signal)
            .then(messages => {
                if (!current()) return;
                const latest = [...messages]
                    .reverse()
                    .filter(m => m.role !== 'system')
                    .map(m => (m.role === 'assistant' ? markdownPlainText(m.content) : m.content))
                    .find(text => text.trim());
                preview = latest ? latest.replace(/\s+/g, ' ').trim().slice(0, 240) : '';
            })
            .catch(() => {})
            .finally(() => {
                if (current()) completed = key;
            });
        return () => controller.abort();
    });
</script>

<span class="row-details" bind:this={element}>
    {#if preview}<span class="preview">{preview}</span>{/if}
</span>

<style>
    .row-details { display: block; min-width: 0; min-height: 1px; }
    .preview { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; color: var(--muted); font-size: 13px; line-height: 20px; white-space: normal; }
    @media (max-width: 700px) { .preview { font-size: 15px; line-height: 22px; } }
</style>
