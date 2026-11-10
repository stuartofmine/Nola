import test from 'node:test';
import assert from 'node:assert/strict';
import {parseTodayFacts} from './todayFacts.ts';
const snapshot = () => ({source_host_id: 'host-A', snapshot: {
    needs_attention: false, unrepresented_expert_contexts: [],
    conversations: [{conversation_id: 'parent', title: 'Original parent', updated_at_ms: 1000, needs_response: true,
        active_run: null, owned_task_permissions: [{child_run_id: 'child-run'}]}],
    recent_conversations: [],
    queued_instances: [{occurrence_id: 'occurrence', automation_id: 'plan', definition_revision: 2, title: null, scheduled_for_ms: 2000}],
    has_more_conversations: true, next_conversation_offset: 50, has_more_queued_instances: false,
}});
test('Today preserves Core needs_response for sealed parents and revision-bound queue facts', () => {
    const result = parseTodayFacts(snapshot(), 'host-A');
    assert.equal(result.conversations[0].active_run, null);
    assert.equal(result.conversations[0].needs_response, true);
    assert.equal(result.queued_instances[0].title, null);
    assert.equal(result.queued_instances[0].definition_revision, 2);
    assert.equal(result.has_more_conversations, true);
});
test('Today rejects Host replacement, duplicate identities and incomplete display contracts', () => {
    assert.throws(() => parseTodayFacts(snapshot(), 'host-B'));
    const duplicate = snapshot(); duplicate.snapshot.conversations.push(duplicate.snapshot.conversations[0]);
    assert.throws(() => parseTodayFacts(duplicate, 'host-A'));
    const invalid = snapshot(); (invalid.snapshot.conversations[0] as unknown as Record<string, unknown>).needs_response = undefined;
    assert.throws(() => parseTodayFacts(invalid, 'host-A'));
    const badRun = snapshot(); (badRun.snapshot.conversations[0] as unknown as Record<string, unknown>).active_run = {run_id: 'child-run'};
    assert.throws(() => parseTodayFacts(badRun, 'host-A'));
});
