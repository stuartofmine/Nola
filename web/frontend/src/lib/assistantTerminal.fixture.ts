import { AssistantJournalReader } from './assistantJournal.ts';
import type { JournalTransport, JournalUpdate } from './assistantJournal.ts';
import { readTerminal } from './assistantTerminal.ts';
import type { ChatTurn } from './chatTurn.ts';
import type { RunResultValue } from './runResult.ts';

/** In-memory authenticated Store transport for terminal consumer tests. */
export function terminalFixture(turn: ChatTurn, status: 'completed' | 'cancelled' | 'failed' = 'completed', additionalAssistantIds: string[] = [], includeUser = false, userText: string | null = null) {
    const identity = { source_store_epoch: 'epoch', conversation_id: turn.owner.conversationId,
        run_id: turn.runId ?? turn.identity.submissionId, assistant_message_id: turn.identity.assistantMessageId };
    const through = { identity, sequence: '9007199254740993', record_digest: 'a'.repeat(64) };
    const assistantIds = [identity.assistant_message_id, ...additionalAssistantIds];
    const declarations = [...(includeUser ? [{id: turn.identity.userMessageId, role: 'user'}] : []), ...assistantIds.map(id => ({id, role: 'assistant'}))];
    const admission = { identity, sequence: String(declarations.length), record_digest: declarations.length.toString(16).repeat(64) };
    const value: RunResultValue = status === 'completed' ? { kind: 'answered', answer_part_refs: ['answer'] }
        : status === 'cancelled' ? { kind: 'cancelled', cancellation_id: 'cancel', observed_effect_refs: [], unknown_effect_refs: [] }
        : { kind: 'failed', error_ref: 'failure', observed_effect_refs: [], unknown_effect_refs: [] };
    const result = { result_id: 'result', run_id: identity.run_id, conversation_id: identity.conversation_id,
        originating_submission_id: turn.identity.submissionId,
        input: { watermark: '1', applications_digest: 'e'.repeat(64) },
        decided_at_revision: '9007199254740993', decision_event_cursor: '9007199254740994',
        requirement_revision: '1', evidence_frontier: '9007199254740992', value };
    const seal = { protocol_version: 2, identity, core_session_id: 'core', transition_id: 'transition',
        checkpoint_revision: '9007199254740993', core_revision: '9007199254740993', event_cursor: '9007199254740994',
        result, result_digest: 'f'.repeat(64),
        completed_at_ms: 1234, entry_count: '1', heads_count: '1', child_receipt_count: '0',
        manifest_digest: 'b'.repeat(64), seal_digest: 'c'.repeat(64), admission };
    const requests: Record<string, unknown>[] = [];
    const transport: JournalTransport = async request => {
        requests.push(request);
        let result: unknown;
        switch (request.operation) {
            case 'admission_head': result = admission; break;
            case 'admissions': {
                const records = [];
                for (const [index, {id, role}] of declarations.entries()) {
                    const text = JSON.stringify({ ordinal: index + 1, input: { id, run_id: identity.run_id,
                        role, status: role === 'user' ? 'completed' : 'streaming', created_at_ms: 1000, completed_at_ms: null,
                        initial_parts: role === 'user' && userText !== null ? [{id: `${id}:body`, kind: 'text', content_text: userText}] : [] } });
                    const declaration_digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))), byte => byte.toString(16).padStart(2, '0')).join('');
                    records.push({ cursor: { identity, sequence: String(index + 1), record_digest: (index + 1).toString(16).repeat(64) },
                        previous_digest: index.toString(16).repeat(64), message_id: id, offset: '0',
                        total_utf8_bytes: String(new TextEncoder().encode(text).length), declaration_digest, text });
                }
                result = { after: { identity, sequence: '0', record_digest: '0'.repeat(64) }, through: admission, next: admission, complete: true,
                    records };
                break;
            }
            case 'terminal_seal': result = seal; break;
            case 'terminal_manifest': result = { seal, after_entry: '0', next_entry: '1', complete: true,
                entries: [{ kind: 'assistant', through, ordinal: 1, created_at_ms: 1000, state: status, completed_at_ms: 1234 }] }; break;
            case 'open_snapshot': result = { snapshot_id: 'snapshot', at: request.at }; break;
            case 'parts': result = { at: through, parts: [], next_part_id: null }; break;
            case 'release_snapshot': result = null; break;
            default: throw new Error(`Unexpected request ${request.operation}`);
        }
        return { source_host_id: turn.owner.hostId, result };
    };
    const reader = new AssistantJournalReader(turn.owner.hostId, turn.owner.conversationId, turn.owner.generation, transport);
    return { reader, transport, requests, through, seal,
        verify: (apply: (update: JournalUpdate | null) => void = () => {}) => readTerminal(turn, reader, transport, apply, () => true) };
}
