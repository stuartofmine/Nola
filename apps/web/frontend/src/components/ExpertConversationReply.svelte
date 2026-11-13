<script lang="ts">
    import ExpertReplyFrame from './ExpertReplyFrame.svelte';
    import RoomReply from './RoomReply.svelte';
    import ChatMessageActions from './ChatMessageActions.svelte';
    import type { TimelineMessage } from '../lib/chatTimeline';
    let {hostId, agentId, name, status, roomId, contextId, executionId, running, active, compact = false, permissionInteractive = true}: {
        hostId: string; agentId: string; name: string; status: string; roomId: string;
        contextId: string; executionId: string; running: boolean; active: boolean; compact?: boolean; permissionInteractive?: boolean;
    } = $props();
    let message = $state<TimelineMessage | null>(null);
    const ownerKey = $derived(JSON.stringify([hostId, roomId, contextId, executionId]));
</script>
<ExpertReplyFrame {hostId} {agentId} {name} {status}>
    <div class="message-content"><RoomReply {active} {hostId} {roomId} {contextId} {executionId} {running} {compact} {permissionInteractive} hideContext bind:message /></div>
    {#snippet actions()}
        {#if message}<ChatMessageActions {message} {ownerKey} />{/if}
    {/snippet}
</ExpertReplyFrame>
