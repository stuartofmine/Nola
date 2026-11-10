<script lang="ts">
    import { onMount } from 'svelte';
    import { api } from '../lib/api';
    import { roomCommand, type RoomSnapshot } from '../lib/agentRooms';
    import { messageWindowUrl } from '../lib/chatConversations';
    import { parseCanonicalProjection, timelinePartText } from '../lib/chatTimeline';
    import { markdownPlainText } from '../lib/markdown';

    let { conversationId, version }: { conversationId: string; version: string } = $props();
    let element: HTMLSpanElement;
    let visible = $state(false);
    let preview = $state('');
    let experts = $state<{ id: string; name: string }[]>([]);
    let completed = '';
    const identity = $derived(JSON.stringify([conversationId, version]));

    onMount(() => {
        // The list can contain thousands of conversations. Hydrate only rows in
        // its visible scrollport; leaving the viewport cancels unfinished reads.
        const observer = new IntersectionObserver(entries => {
            visible = entries.some(entry => entry.isIntersecting);
        });
        observer.observe(element);
        return () => observer.disconnect();
    });
    $effect(() => {
        identity;
        preview = ''; experts = []; completed = '';
    });
    $effect(() => {
        const key = identity, id = conversationId;
        if (!visible || completed === key) return;
        const controller = new AbortController();
        const current = () => !controller.signal.aborted && identity === key && conversationId === id;
        void Promise.allSettled([
            api<{ conversationProjection: unknown }>(messageWindowUrl(id, null, 8), { signal: controller.signal }).then(value => {
                const projection = parseCanonicalProjection(value.conversationProjection);
                if (projection.conversation_id !== id || !current()) return;
                preview = [...projection.messages].reverse().filter(message => message.role !== 'system')
                    .map(message => {
                        const text = message.parts.map(part => timelinePartText({ source: 'canonical', id: part.part_id, value: part })).join('');
                        return message.role === 'assistant' ? markdownPlainText(text) : text;
                    })
                    .find(text => text.trim())?.replace(/\s+/g, ' ').trim().slice(0, 240) ?? '';
            }),
            roomCommand<{ conversation_id: string; room: RoomSnapshot } | null>(
                { type: 'for_conversation', conversation_id: id }, controller.signal).then(value => {
                if (!current() || !value || value.conversation_id !== id || !Array.isArray(value.room?.room?.participants)) return;
                const unique = new Map<string, string>();
                for (const participant of value.room.room.participants) {
                    if (participant.active && typeof participant.agent_id === 'string' && typeof participant.display_name === 'string'
                        && participant.display_name.trim()) unique.set(participant.agent_id, participant.display_name);
                }
                experts = [...unique].map(([id, name]) => ({ id, name }));
            }),
        ]).then(() => { if (current()) completed = key; });
        // Missing optional metadata never prevents opening the conversation.
        return () => controller.abort();
    });
</script>

<span class="row-details" bind:this={element}>
    {#if preview}<span class="preview">{preview}</span>{/if}
    {#if experts.length}
        <span class="experts">
            <span class="avatars" aria-hidden="true">{#each experts.slice(0, 3) as expert (expert.id)}<span>{Array.from(expert.name)[0]}</span>{/each}</span>
            <span class="names">{experts.map(expert => expert.name).join(' · ')}</span>
        </span>
    {/if}
</span>

<style>
    .row-details { display: block; min-width: 0; min-height: 1px; }
    .preview { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; color: var(--muted); font-size: 13px; line-height: 20px; white-space: normal; }
    .experts { display: flex; align-items: center; gap: 6px; margin-top: 6px; min-width: 0; color: var(--muted); font-size: 12px; line-height: 18px; }
    .avatars { display: flex; flex-shrink: 0; gap: 3px; }
    .avatars > span { display: grid; place-items: center; width: 14px; height: 14px; border: 1px solid var(--muted); border-radius: 50%; background: transparent; font-size: 9px; line-height: 12px; }
    .names { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    @media (max-width: 700px) { .preview { font-size: 15px; line-height: 22px; } }
</style>
