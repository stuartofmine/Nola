import type { ActivityOwner } from './activity';

export function sameActivityOwner(left: ActivityOwner, right: ActivityOwner): boolean {
    return left.conversation_id === right.conversation_id && left.run_id === right.run_id
        && left.assistant_message_id === right.assistant_message_id;
}

/** A cursor belongs to the exact normalized filter that produced it. */
export function activityQueryKey(source: string, needsResponse: boolean, query: string): string {
    return JSON.stringify([source || null, needsResponse, query.trim()]);
}
