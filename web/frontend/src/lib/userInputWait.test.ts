import test from 'node:test';
import assert from 'node:assert/strict';
import { parseUserInputWaits, freezeWaitResponse, waitIdentity } from './userInputWait.ts';
const record = (conversation_id = 'parent', run_id = 'run') => ({ conversation_id, run_id, wait: {
    wait_id: 'question', revision: 1, response_schema_digest: 'schema', state: { type: 'open' },
    question_set: { questions: [{ id: 'q', prompt: 'Choose', required: true, allow_freeform: true,
        max_text_chars: 100, options: [{ id: 'one', label: 'One', description: null }] }] },
} });
test('parent presentation preserves exact child answer ownership', () => {
    const values = parseUserInputWaits({ conversation_id: 'parent', waits: [record()],
        task_waits: [{ ...record('child', 'child-run'), task_id: 'task' }] }, 'parent');
    assert.equal(values[1].conversation_id, 'child');
    assert.equal(values[1].run_id, 'child-run');
    assert.equal(values[1].task_id, 'task');
    assert.notEqual(waitIdentity(values[0]), waitIdentity(values[1]));
    assert.throws(() => parseUserInputWaits({ conversation_id: 'parent', waits: [record('child')], task_waits: [] }, 'parent'));
    assert.throws(() => parseUserInputWaits({ conversation_id: 'other', waits: [], task_waits: [] }, 'parent'));
});
test('retry freezes answer content, command identity and Core question witness', () => {
    const value = parseUserInputWaits({ conversation_id: 'parent', waits: [record()], task_waits: [] }, 'parent')[0];
    const response = { type: 'answers' as const, answers: [{ question_id: 'q', value: { type: 'text' as const, text: 'Original' } }] };
    const body = freezeWaitResponse(value, response, 'stable-command');
    response.answers[0].value.text = 'Changed'; value.wait.revision = 2;
    const sent = JSON.parse(body);
    assert.equal(sent.response.expected_revision, 1);
    assert.equal(sent.response.response_schema_digest, 'schema');
    assert.equal(sent.response.command_id, 'stable-command');
    assert.equal(sent.response.response.answers[0].value.text, 'Original');
    assert.equal(sent.run_id, 'run');
    assert.equal(sent.permissions, undefined);
});
test('withdrawn and duplicate waits cannot render as actionable questions', () => {
    const withdrawn = record(); withdrawn.wait.state.type = 'withdrawn';
    assert.throws(() => parseUserInputWaits({ conversation_id: 'parent', waits: [withdrawn], task_waits: [] }, 'parent'));
    assert.throws(() => parseUserInputWaits({ conversation_id: 'parent', waits: [record(), record()], task_waits: [] }, 'parent'));
});
