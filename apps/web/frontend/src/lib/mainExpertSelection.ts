import type {RoomSnapshot} from './agentRooms.ts';
export interface MainExpertSelection {
    agent_id:string;agent_revision:number;participant_id:string|null;context_conversation_id:string|null;
}
/** References are typed Main guidance. User prose is never parsed or rewritten for routing. */
export function mainExpertSelections(room:RoomSnapshot|null,participantIds:readonly string[]):MainExpertSelection[] {
    if(!participantIds.length)return [];
    if(!room||new Set(participantIds).size!==participantIds.length)throw Error('Expert selection is unavailable');
    return participantIds.map(id=>{
        const member=room.room.participants.find(item=>item.id===id&&item.active);
        if(!member||!member.agent_id?.trim()||!member.context_conversation_id?.trim()||!Number.isSafeInteger(member.agent_revision)||member.agent_revision<1)throw Error('Expert selection changed');
        return {agent_id:member.agent_id,agent_revision:member.agent_revision,participant_id:member.id,context_conversation_id:member.context_conversation_id};
    });
}

/** Fixed Core protocol IDs only; natural language is never a routing signal. */
export function mainExpertSelectionError(code: unknown): string | null {
    if (code === 'main_expert_selection_requires_tool_capable_main_model' || code === 'main_expert_selection_requires_enabled_expert_dispatch') return 'rooms.mainRequiresTools';
    if (code === 'main_expert_selection_source_changed') return 'rooms.expertCatalogUnavailable';
    return null;
}
