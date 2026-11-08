import { parseSourceReceipt } from './sourceSubmission.ts';
import type { SourceSubmissionReceipt } from './sourceSubmission.ts';
import type { ChatTurnOwner } from './chatTurn.ts';

export interface QueuedUiHead {
    command_id: string; kind: string; user_message_id: string | null; lease_until_ms: number | null;
}
/** A dispatch receipt is immutable source evidence, never a new request template. */
export function queuedUiDispatch(value: unknown, head: QueuedUiHead, owner: ChatTurnOwner): SourceSubmissionReceipt | null {
    if (value === null) return null;
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Missing queued dispatch evidence');
    const receipt = value as SourceSubmissionReceipt;
    if (!receipt.identity || receipt.identity.source_host_id !== owner.hostId
        || receipt.identity.client_id !== owner.clientId || receipt.identity.conversation_id !== owner.conversationId
        || receipt.identity.submission_id !== head.command_id || receipt.identity.user_message_id !== head.user_message_id
        || head.user_message_id !== `ui:${head.command_id}:user`) throw new Error('Queued dispatch owner mismatch');
    return parseSourceReceipt(receipt, receipt.identity, receipt.request_digest);
}
