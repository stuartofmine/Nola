/** Lossless business-result transport. Structural checks are not evidence
 * acceptance: authenticated Store reads remain the source of authority. */
import { journalNumber } from './assistantJournal.ts';

type Reason = { code: string; detail: string | null };
export type RunResultValue =
    | { kind: 'answered'; answer_part_refs: string[] }
    | { kind: 'action_completed'; requirement_acceptance_refs: string[] }
    | { kind: 'delegation_accepted'; task_id: string; parent_call_id: string }
    | { kind: 'incomplete'; reason: Reason; remaining_requirement_refs: string[]; obligation_refs: string[] }
    | { kind: 'blocked'; reason: Reason; blocking_fact_refs: string[]; resolution_condition: string }
    | { kind: 'cancelled'; cancellation_id: string; observed_effect_refs: string[]; unknown_effect_refs: string[] }
    | { kind: 'failed'; error_ref: string; observed_effect_refs: string[]; unknown_effect_refs: string[] };
export interface RunResult {
    result_id: string; run_id: string; conversation_id: string; originating_submission_id: string;
    input: { watermark: string; applications_digest: string };
    decided_at_revision: string; decision_event_cursor: string; requirement_revision: string;
    evidence_frontier: string; value: RunResultValue;
}
const object = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const id = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && v.trim() === v && !/[\p{Cc}]/u.test(v);
const hash = (v: unknown): boolean => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
// Domain RunInputCommitment's versioned empty-prefix vector, not a fallback.
const emptyInputDigest = 'e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd';
const refs = (v: unknown): v is string[] => Array.isArray(v) && v.every(id) && new Set(v).size === v.length;
const reason = (v: unknown): boolean => object(v) && typeof v.code === 'string' && /^[a-z0-9_.]+$/.test(v.code) && (v.detail === null || typeof v.detail === 'string');
const effects = (observed: unknown, unknown: unknown): boolean => refs(observed) && refs(unknown)
    && !unknown.some(effect => observed.includes(effect));

export function parseRunResult(raw: unknown, conversationId: string, runId: string): RunResult {
    if (!object(raw) || raw.conversation_id !== conversationId || raw.run_id !== runId ||
        !id(raw.conversation_id) || !id(raw.run_id) || !id(raw.result_id) || !id(raw.originating_submission_id) || !object(raw.input) ||
        !hash(raw.input.applications_digest) || !object(raw.value)) throw new Error('Invalid Run result owner');
    for (const value of [raw.input.watermark, raw.decided_at_revision, raw.decision_event_cursor,
        raw.requirement_revision, raw.evidence_frontier]) {
        if (BigInt(journalNumber(value)) > 9223372036854775807n) throw new Error('Run result counter exceeds durable range');
    }
    if ((raw.input.watermark === '0') !== (raw.input.applications_digest === emptyInputDigest)) {
        throw new Error('Run input watermark and empty-prefix commitment disagree');
    }
    const value = raw.value;
    let valid = false;
    switch (value.kind) {
        case 'delegation_accepted': valid = id(value.task_id) && id(value.parent_call_id); break;
        case 'answered': valid = refs(value.answer_part_refs) && (value.answer_part_refs as string[]).length > 0; break;
        case 'action_completed': valid = refs(value.requirement_acceptance_refs) && (value.requirement_acceptance_refs as string[]).length > 0; break;
        case 'incomplete': valid = reason(value.reason) && refs(value.remaining_requirement_refs) && refs(value.obligation_refs)
            && value.remaining_requirement_refs.length + value.obligation_refs.length > 0; break;
        case 'blocked': valid = reason(value.reason) && refs(value.blocking_fact_refs) && value.blocking_fact_refs.length > 0
            && typeof value.resolution_condition === 'string' && value.resolution_condition.trim().length > 0; break;
        case 'cancelled': valid = id(value.cancellation_id) && effects(value.observed_effect_refs, value.unknown_effect_refs); break;
        case 'failed': valid = id(value.error_ref) && effects(value.observed_effect_refs, value.unknown_effect_refs); break;
    }
    if (!valid) throw new Error('Invalid Run result evidence');
    return structuredClone(raw) as unknown as RunResult;
}

/** Structural equality, independent of wire key ordering. Never substitutes for a digest check. */
export function sameResultJSON(left: unknown, right: unknown): boolean {
    if (left === right) return true;
    if (Array.isArray(left) && Array.isArray(right)) return left.length === right.length && left.every((v, i) => sameResultJSON(v, right[i]));
    if (!object(left) || !object(right)) return false;
    const keys = Object.keys(left);
    return keys.length === Object.keys(right).length && keys.every(key => Object.hasOwn(right, key) && sameResultJSON(left[key], right[key]));
}
