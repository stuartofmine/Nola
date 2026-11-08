<script lang="ts">
    import { onMount, type Snippet } from 'svelte';
    import ExpertBubbleColorPicker from './ExpertBubbleColorPicker.svelte';
    import { expertBubbleColor, expertBubblePalette, observeExpertBubbleColors } from '../lib/expertBubbleColors';
    let {hostId, agentId, name, status = '', children, actions}: {hostId: string; agentId: string; name: string;
        status?: string; children: Snippet; actions?: Snippet} = $props();
    let revision = $state(0);
    const color = $derived.by(() => { revision; return expertBubbleColor(hostId, agentId); });
    onMount(() => observeExpertBubbleColors(() => revision++));
</script>
<div class="expert-frame" style:--expert-light={expertBubblePalette[color][0]} style:--expert-dark={expertBubblePalette[color][1]}>
    <div class="identity">
        <ExpertBubbleColorPicker {hostId} {agentId} {name} avatar />
        <strong class="expert-name" title={name}>{name}</strong>
    </div>
    <div class="reply-column">
        <div class="bubble">{@render children()}</div>
        {#if status}<small class="status">{status}</small>{/if}
        {#if actions}<div class="message-actions">{@render actions()}</div>{/if}
    </div>
</div>

<style>
    .expert-frame { display: flex; align-items: flex-start; gap: 12px; max-width: 100%; min-width: 0; }
    .identity { display: flex; align-items: center; flex-direction: column; gap: 6px; padding-top: 3px; justify-content: center; flex: 0 0 48px; min-width: 0; }
    .reply-column { display: flex; flex-direction: column; align-items: flex-start; min-width: 0; max-width: calc(100% - 60px); }
    .expert-name { font-size: 12px; line-height: 18px; font-weight: 500; text-align: center; overflow-wrap: anywhere; max-width: 100%; margin: 0; }
    .bubble { max-width: 100%; min-width: 0; padding: 14px 18px; border-radius: 5px 16px 16px; background: var(--expert-light); color: var(--content); }
    .bubble :global(.message-content) { min-width: 0; }
    .status { color: var(--muted); font-size: 12px; line-height: 18px; margin-top: 5px; }
    .message-actions { margin-top: 5px; }
    @media (prefers-color-scheme: dark) { :global(:root:not([data-appearance="light"])) .bubble { background: var(--expert-dark); } }
    :global(:root[data-appearance="dark"]) .bubble { background:var(--expert-dark); }
    @media (max-width: 700px) { .expert-frame { gap: 10px; } .identity { flex-basis: 38px; } .reply-column { max-width: calc(100% - 48px); } .bubble { padding: 12px 14px; border-radius: 5px 14px 14px; } }
    @media (max-width: 340px) { .expert-frame { gap: 8px; } .identity { flex-basis: 34px; } .reply-column { max-width: calc(100% - 42px); } .bubble { padding-inline: 12px; } }
</style>
