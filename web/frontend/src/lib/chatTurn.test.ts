import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chatTurnIdentity, createChatTurn, reduceChatTurn, canReleaseChatTurn, buildChatResumeRequest } from './chatTurn.ts';
import { createSourceSubmission, sourceIdentity, bindSourceDigest, acceptSourceReceipt } from './sourceSubmission.ts';
import type { ChatTurn } from './chatTurn.ts';
import type { VerifiedTerminal } from './assistantTerminal.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';

const owner = { conversationId: 'conversation', hostId: 'host', clientId: 'browser-test', generation: 3 };
const start = () => createChatTurn(owner, 'submission');
const permission = { protocol_version: 4, allow_conversation: true, nonce: 'nonce', request_id: 'request', host_id: owner.hostId, client_id: owner.clientId, session_id: owner.conversationId, tool_name: 'file.write', expires_at: 200 };
test('journal and Core trace cannot replace an admitted submission receipt', () => {
    const state = start(); const fixture = terminalFixture(state);
    const event = { type: 'assistant_journal_committed_v1', source_host_id: owner.hostId, session_id: owner.conversationId,
        through: { ...fixture.through, identity: { ...fixture.through.identity, run_id: 'canonical-run' } } };
    for (const payload of [event, { session_id: owner.conversationId, canonical_run_id: 'canonical-run' },
        { trace: { conversation_id: owner.conversationId, run_id: 'core-turn' } }])
        assert.equal(reduceChatTurn(state, { type: 'event', owner, payload }).state.runId, null);
    const identity = sourceIdentity(owner, state.identity);
    const request = { conversationId: owner.conversationId, expected_host_id: owner.hostId, client_id: owner.clientId,
        submission_id: state.identity.submissionId, user_message_id: state.identity.userMessageId, assistant_message_id: state.identity.assistantMessageId, messages: [] };
    const bound = bindSourceDigest(createSourceSubmission(identity, request), { source_host_id: owner.hostId, result: { identity, request_digest: 'a'.repeat(64) } });
    const admitted = acceptSourceReceipt(bound, { protocol_version: 1, identity, request_digest: bound.request_digest, phase: 'admitted',
        admission: { ...fixture.through, identity: { ...fixture.through.identity, run_id: state.identity.submissionId } }, rejection_code: null });
    assert.equal(reduceChatTurn(state, { type: 'run_bound', owner, runId: admitted.receipt!.admission!.identity.run_id }).state.runId, 'submission');
});
async function commit(state: ChatTurn, status: 'completed' | 'failed' | 'cancelled' = 'completed') {
    const evidence = (await terminalFixture(state, status).verify())!;
    return reduceChatTurn(state, { type: 'terminal_verified', owner, evidence });
}

test('submission identities remain deterministic through uncertain delivery', () => {
    const state = start();
    assert.deepEqual(state.identity, chatTurnIdentity('submission'));
    for (const reason of ['eof', 'done', 'error', 'abort', 'trace_terminal'] as const) {
        const next = reduceChatTurn(state, { owner, type: 'transport_interrupted', reason }).state;
        assert.deepEqual(next.identity, state.identity);
        assert.equal(next.phase, 'recovering'); assert.equal(next.outcome, 'uncertain');
        assert.equal(canReleaseChatTurn(next), false);
    }
    for (const invalid of ['', ' leading', 'newline\n', '界'.repeat(100), 'a'.repeat(256)]) assert.throws(() => createChatTurn(owner, invalid));
    assert.throws(() => createChatTurn({ ...owner, generation: NaN }, state.identity));
    assert.throws(() => createChatTurn(owner, { ...state.identity, assistantMessageId: state.identity.userMessageId }));
});

test('only fully materialized authenticated seal evidence releases the exact turn', async () => {
    const initial = start(); const result = await commit(initial);
    assert.equal(result.ignored, undefined); assert.equal(result.state.phase, 'committed');
    assert.equal(result.state.outcome, 'answered'); assert.equal(result.state.committedRevision, '9007199254740993');
    assert.equal(initial.phase, 'sending'); assert.equal(canReleaseChatTurn(result.state), true);
    const evidence = (await terminalFixture(initial).verify())!;
    assert.equal(reduceChatTurn(result.state, { owner, type: 'terminal_verified', evidence }).ignored, 'committed');
    assert.equal(reduceChatTurn(initial, { owner, type: 'terminal_verified', evidence: structuredClone(evidence) }).ignored, 'invalid');
    assert.equal(reduceChatTurn(initial, { owner, type: 'terminal_verified', evidence: {} as VerifiedTerminal }).ignored, 'invalid');
});

test('all stream terminals, legacy snapshots and mutable history stay nonauthoritative', () => {
    for (const type of ['[DONE]', 'error', 'core_part_updated', 'core_task_updated', 'accepted', 'assistant_run_terminal_committed_v1', 'conversation_projection_committed_v1']) {
        assert.equal(canReleaseChatTurn(reduceChatTurn(start(), { owner, type: 'event', payload: { type, terminal_status: 'completed' } }).state), false);
    }
});

test('foreign generation, host, session, client, submission and root assistant cannot release composer', async () => {
    const state = start(); const evidence = (await terminalFixture(state).verify())!;
    for (const changed of [{ generation: 4 }, { hostId: 'foreign' }, { clientId: 'foreign' }, { conversationId: 'foreign' }]) {
        assert.equal(reduceChatTurn(state, { owner: { ...owner, ...changed }, type: 'terminal_verified', evidence }).ignored, 'owner');
        assert.equal(reduceChatTurn(createChatTurn({ ...owner, ...changed }, state.identity), { owner: { ...owner, ...changed }, type: 'terminal_verified', evidence }).ignored, 'owner');
    }
    for (const changed of [{ submissionId: 'foreign' }, { assistantMessageId: 'foreign' }]) {
        assert.equal(reduceChatTurn(createChatTurn(owner, { ...state.identity, ...changed }), { owner, type: 'terminal_verified', evidence }).ignored, 'owner');
    }
});

test('cancel and failure require their sealed outcome, not transport termination', async () => {
    let state = reduceChatTurn(start(), { owner, type: 'cancel_requested' }).state;
    state = reduceChatTurn(state, { owner, type: 'transport_interrupted', reason: 'abort' }).state;
    assert.equal(state.phase, 'cancel_requested'); assert.equal(buildChatResumeRequest(state), null);
    state = (await commit(state, 'cancelled')).state;
    assert.equal(state.outcome, 'cancelled'); assert.equal(canReleaseChatTurn(state), true);
    assert.equal(reduceChatTurn(state, { owner, type: 'transport_interrupted', reason: 'eof' }).state, state);
    const failed = (await commit(start(), 'failed')).state;
    assert.equal(failed.phase, 'committed'); assert.equal(failed.outcome, 'failed');
});

test('permission acknowledgement is not resolution or terminal evidence', async () => {
    let state = reduceChatTurn(start(), { owner, type: 'permission_pending', challenge: permission, nowSeconds: 100 }).state;
    assert.equal(state.phase, 'awaiting_permission');
    assert.equal(reduceChatTurn(state, { owner, type: 'permission_pending', challenge: { ...permission, tool_name: 'foreign' }, nowSeconds: 100 }).ignored, 'permission');
    assert.equal(reduceChatTurn(state, { owner, type: 'permission_acknowledged', nonce: 'foreign' }).ignored, 'permission');
    state = reduceChatTurn(state, { owner, type: 'permission_acknowledged', nonce: permission.nonce }).state;
    assert.equal(state.permissionAcknowledged, true); assert.equal(canReleaseChatTurn(state), false);
    state = reduceChatTurn(state, { owner, type: 'permission_resolved', nonce: permission.nonce }).state;
    assert.equal(state.phase, 'streaming'); assert.equal(state.permission, null);
    assert.equal((await commit(state)).state.phase, 'committed');
});

test('permission expiry, protocol and owner are validated without tool-name routing', () => {
    for (const changed of [{ protocol_version: 2 }, { expires_at: 100 }, { nonce: '' }]) assert.equal(reduceChatTurn(start(), { owner, type: 'permission_pending', challenge: { ...permission, ...changed }, nowSeconds: 100 }).ignored, 'invalid');
    for (const changed of [{ host_id: 'foreign' }, { client_id: 'foreign' }, { session_id: 'foreign' }]) assert.equal(reduceChatTurn(start(), { owner, type: 'permission_pending', challenge: { ...permission, ...changed }, nowSeconds: 100 }).ignored, 'owner');
});

test('successive nested permissions advance from the owning Run snapshot without an empty poll', () => {
    let state = reduceChatTurn(start(), { owner, type: 'run_bound', runId: 'original-run' }).state;
    const first = { ...permission, request_id: 'cell:child:1', tool_name: 'write_file' };
    const second = { ...first, nonce: 'nonce-2', request_id: 'cell:child:2' };
    state = reduceChatTurn(state, { owner, type: 'permission_pending', challenge: first, nowSeconds: 100 }).state;
    state = reduceChatTurn(state, { owner, type: 'permission_acknowledged', nonce: first.nonce }).state;
    assert.equal(reduceChatTurn(state, { owner, type: 'permission_pending', challenge: second, nowSeconds: 100 }).ignored, 'permission');
    // A parallel request must not displace the unresolved current request, even if reordered.
    const parallel = reduceChatTurn(state, { owner, type: 'permissions_observed', challenges: [second, first], nowSeconds: 100 }).state;
    assert.equal(parallel.permission?.request_id, first.request_id);
    assert.equal(parallel.permissionAcknowledged, true);
    state = reduceChatTurn(parallel, { owner, type: 'permissions_observed', challenges: [second], nowSeconds: 100 }).state;
    assert.equal(state.permission?.request_id, second.request_id);
    assert.equal(state.permissionAcknowledged, false);
    assert.equal(state.phase, 'awaiting_permission');
    assert.equal(state.runId, 'original-run');
    assert.deepEqual(state.owner, owner);
    assert.equal(canReleaseChatTurn(state), false);
    for (const changed of [{ session_id: 'other' }, { host_id: 'other' }, { client_id: 'other' }, { expires_at: 100 }]) {
        const result = reduceChatTurn(state, { owner, type: 'permissions_observed', challenges: [{ ...second, ...changed }], nowSeconds: 100 });
        assert.ok(result.ignored);
        assert.equal(result.state, state);
    }
    assert.equal(reduceChatTurn(state, { owner: { ...owner, generation: 4 }, type: 'permissions_observed', challenges: [], nowSeconds: 100 }).ignored, 'owner');
    assert.equal(reduceChatTurn(state, { owner, type: 'permissions_observed', challenges: [], nowSeconds: 100 }).state.permission, null);
});

test('resume contains only the exact existing run identity', async () => {
    let state = start(); assert.equal(buildChatResumeRequest(state), null);
    state = reduceChatTurn(state, { owner, type: 'run_bound', runId: 'canonical-run' }).state;
    assert.equal(reduceChatTurn(state, { owner, type: 'run_bound', runId: 'different-run' }).ignored, 'run');
    state = reduceChatTurn(state, { owner, type: 'transport_interrupted', reason: 'error' }).state;
    assert.deepEqual(buildChatResumeRequest(state), { conversation_id: owner.conversationId, expected_run_id: 'canonical-run', expected_host_id: owner.hostId, client_id: owner.clientId });
    assert.equal(buildChatResumeRequest((await commit(state)).state), null);
});

test('observed Goal permission retains its execution client without changing browser ownership', () => {
    const bound = reduceChatTurn(start(), {type:'run_bound', owner, runId:'goal-run', permissionClientId:'web-goal'}).state;
    const challenge = {...permission, client_id:'web-goal'};
    const shown = reduceChatTurn(bound, {type:'permissions_observed', owner, challenges:[challenge], nowSeconds:100});
    assert.equal(shown.ignored, undefined);
    assert.equal(shown.state.owner.clientId, owner.clientId);
    assert.equal(shown.state.permission?.client_id, 'web-goal');
    assert.equal(reduceChatTurn(bound, {type:'run_bound', owner, runId:'goal-run', permissionClientId:'foreign'}).ignored, 'owner');
    assert.equal(reduceChatTurn(bound, {type:'permission_pending', owner, challenge:permission, nowSeconds:100}).ignored, 'owner');
    const ordinary = reduceChatTurn(start(), {type:'run_bound', owner, runId:'normal'}).state;
    assert.equal(reduceChatTurn(ordinary, {type:'permission_pending', owner, challenge, nowSeconds:100}).ignored, 'owner');
});
