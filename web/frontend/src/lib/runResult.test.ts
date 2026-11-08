import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRunResult, type RunResultValue } from './runResult.ts';

function fixture(value: RunResultValue) {
    return { result_id: 'result', conversation_id: 'conversation', run_id: 'run',
        originating_submission_id: 'submission', input: { watermark: '1', applications_digest: 'a'.repeat(64) },
        decided_at_revision: '1', decision_event_cursor: '2', requirement_revision: '0', evidence_frontier: '0', value };
}
const reason = { code: 'work.pending', detail: null };

test('all results preserve their distinct evidence shape', () => {
    const values: RunResultValue[] = [
        { kind: 'answered', answer_part_refs: ['answer'] },
        { kind: 'action_completed', requirement_acceptance_refs: ['acceptance'] },
        { kind: 'delegation_accepted', task_id: 'task', parent_call_id: 'call' },
        { kind: 'incomplete', reason, remaining_requirement_refs: [], obligation_refs: ['obligation'] },
        { kind: 'blocked', reason, blocking_fact_refs: ['fact'], resolution_condition: 'Supply the missing resource' },
        { kind: 'cancelled', cancellation_id: 'cancel', observed_effect_refs: ['observed'], unknown_effect_refs: ['unknown'] },
        { kind: 'failed', error_ref: 'error', observed_effect_refs: [], unknown_effect_refs: [] },
    ];
    for (const value of values) {
        const raw = fixture(value);
        assert.deepEqual(parseRunResult(raw, 'conversation', 'run'), raw);
    }
});

test('results reject empty evidence and contradictory effect classifications', () => {
    const invalid: RunResultValue[] = [
        { kind: 'delegation_accepted', task_id: '', parent_call_id: 'call' },
        { kind: 'delegation_accepted', task_id: 'task', parent_call_id: ' ' },
        { kind: 'incomplete', reason, remaining_requirement_refs: [], obligation_refs: [] },
        { kind: 'blocked', reason, blocking_fact_refs: [], resolution_condition: 'Provide resource' },
        { kind: 'failed', error_ref: 'error', observed_effect_refs: ['effect'], unknown_effect_refs: ['effect'] },
        { kind: 'cancelled', cancellation_id: 'cancel', observed_effect_refs: ['effect'], unknown_effect_refs: ['effect'] },
        { kind: 'answered', answer_part_refs: [' answer'] },
        { kind: 'answered', answer_part_refs: ['answer\n'] },
        { kind: 'blocked', reason: { code: 'Needs Approval', detail: null }, blocking_fact_refs: ['fact'], resolution_condition: 'Provide resource' },
    ];
    for (const value of invalid) assert.throws(() => parseRunResult(fixture(value), 'conversation', 'run'));
});

test('result counters preserve bigint precision but stay within durable integer range', () => {
    const raw = fixture({ kind: 'answered', answer_part_refs: ['answer'] });
    raw.decided_at_revision = '9223372036854775807';
    assert.equal(parseRunResult(raw, 'conversation', 'run').decided_at_revision, raw.decided_at_revision);
    raw.decided_at_revision = '9223372036854775808';
    assert.throws(() => parseRunResult(raw, 'conversation', 'run'));
    assert.throws(() => parseRunResult(fixture({ kind: 'answered', answer_part_refs: ['answer'] }), 'other', 'run'));
});

test('empty input commitment is explicit and cannot stand in for a nonempty prefix', () => {
    const raw = fixture({ kind: 'answered', answer_part_refs: ['answer'] });
    raw.input.watermark = '0';
    assert.throws(() => parseRunResult(raw, 'conversation', 'run'));
    raw.input.applications_digest = 'e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd';
    assert.deepEqual(parseRunResult(raw, 'conversation', 'run'), raw);
    raw.input.watermark = '1';
    assert.throws(() => parseRunResult(raw, 'conversation', 'run'));
});
