import test from 'node:test';
import assert from 'node:assert/strict';
import { AssistantJournalReader } from './assistantJournal.ts';
import type { JournalTransport } from './assistantJournal.ts';
import { readTerminal, isVerifiedTerminal } from './assistantTerminal.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';
import { createChatTurn, reduceChatTurn, canReleaseChatTurn } from './chatTurn.ts';
import { createChatTimeline, reduceChatTimeline } from './chatTimeline.ts';

const owner = { conversationId: 'conversation', hostId: 'host', clientId: 'client', generation: 1 };
const turn = () => createChatTurn(owner, 'submission');

test('V2 child receipts require sealed exact child identity and one receipt per task', async () => {
    for (const fault of ['legacy_completed', 'missing_child', 'missing_result_digest', 'duplicate_task']) {
        const state = turn(); const f = terminalFixture(state);
        const child: Record<string, unknown> = { kind: 'child_receipt', task_id: 'task', receipt_id: 'receipt',
            parent_turn_id: 'parent', child_session_id: 'child-session', child_run_id: 'child-run',
            status: 'sealed', input_watermark: null, completion_watermark: null,
            receipt_digest: 'd'.repeat(64), result_digest: 'e'.repeat(64) };
        if (fault === 'legacy_completed') child.status = 'completed';
        if (fault === 'missing_child') delete child.child_run_id;
        if (fault === 'missing_result_digest') delete child.result_digest;
        const children = fault === 'duplicate_task' ? [child, { ...child, receipt_id: 'other-receipt' }] : [child];
        const seal = { ...f.seal, entry_count: String(1 + children.length), child_receipt_count: String(children.length) };
        const transport: JournalTransport = async request => {
            const response = await f.transport(request) as { source_host_id: string; result: unknown };
            if (request.operation === 'terminal_seal') return { ...response, result: seal };
            if (request.operation === 'terminal_manifest') {
                const page = response.result as { entries: unknown[] };
                return { ...response, result: { ...page, seal, after_entry: '0', next_entry: seal.entry_count,
                    complete: true, entries: [...page.entries, ...children] } };
            }
            return response;
        };
        await assert.rejects(readTerminal(state, f.reader, transport, () => {}, () => true), /terminal receipt/);
        assert.equal(canReleaseChatTurn(state), false);
    }
});

test('verified result cannot be mutated into a different business outcome', async () => {
    const fixture = terminalFixture(turn());
    const proof = (await fixture.verify())!;
    assert.equal(proof.seal.result.value.kind, 'answered');
    assert.throws(() => Object.assign(proof.seal.result.value, { kind: 'action_completed' }), TypeError);
    assert.throws(() => Object.assign(proof.seal.result.input, { watermark: '999' }), TypeError);
    assert.equal(reduceChatTurn(turn(), { owner, type: 'terminal_verified', evidence: proof }).state.outcome, 'answered');
});

test('first terminal hint is only a Run query candidate until exact root seal is verified', async () => {
    const state = turn();
    const fixture = terminalFixture({ ...state, runId: 'canonical-run' });
    assert.equal(state.runId, null);
    const evidence = await readTerminal(state, fixture.reader, fixture.transport, () => {}, () => true,
        fixture.seal.seal_digest, 'canonical-run');
    assert.equal(state.runId, null, 'query must not mutate submission ownership');
    assert.ok(evidence);
    const accepted = reduceChatTurn(state, { type: 'terminal_verified', owner, evidence });
    assert.equal(accepted.state.runId, 'canonical-run');
    assert.equal(canReleaseChatTurn(accepted.state), true);
    const foreign = terminalFixture({ ...state, identity: { ...state.identity, assistantMessageId: 'foreign-root' }, runId: 'canonical-run' });
    await assert.rejects(readTerminal(state, foreign.reader, foreign.transport, () => {}, () => true,
        foreign.seal.seal_digest, 'canonical-run'), /owner mismatch/);
});

test('all sealed heads are materialized before terminal evidence, retaining user display order', async () => {
    const state = turn(); const fixture = terminalFixture(state);
    let timeline = reduceChatTimeline(createChatTimeline(owner), { type: 'beginSubmission', owner,
        submissionId: state.identity.submissionId, userMessageId: state.identity.userMessageId, userText: '用户输入' }).state;
    const evidence = (await fixture.verify(update => {
        assert.equal(canReleaseChatTurn(state), false);
        if (update) timeline = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId, runId: update.through.identity.run_id, update }).state;
    }))!;
    assert.equal(isVerifiedTerminal(evidence), true);
    assert.deepEqual(fixture.requests.map(request => request.operation), ['terminal_seal', 'admissions', 'terminal_manifest', 'open_snapshot', 'parts', 'release_snapshot']);
    const result = reduceChatTimeline(timeline, { type: 'terminal', owner, evidence });
    assert.equal(result.state.submissionParentMessageId, [...evidence.declarations].sort((a, b) => a.ordinal - b.ordinal).at(-1)?.input.id);
    const next = reduceChatTimeline(result.state, {type: 'beginSubmission', owner, submissionId: 'next-turn', userMessageId: 'optimistic-next', userText: 'follow-up'});
    assert.equal(next.state.submissionParentMessageId, result.state.submissionParentMessageId, 'optimistic successor cannot replace the committed parent');
    assert.equal(result.ignored, undefined); assert.equal(result.state.durable, true);
    assert.deepEqual(result.state.messages.map(message => message.id), [state.identity.userMessageId, state.identity.assistantMessageId]);
    assert.equal(result.state.messages[1].state, 'completed');
    assert.equal(reduceChatTimeline(createChatTimeline(owner), { type: 'terminal', owner, evidence }).ignored, 'invalid', 'proof cannot bypass this timeline catchup');
    assert.equal(canReleaseChatTurn(reduceChatTurn(state, { type: 'terminal_verified', owner, evidence }).state), true);
});

test('missing seal, malformed counts, wrong root, source and fixed watermark cannot release', async () => {
    const state = turn();
    for (const scenario of ['missing', 'count', 'root', 'source', 'page', 'watermark', 'missing_message_state', 'missing_message_completion']) {
        const f = terminalFixture(state);
        const transport: JournalTransport = async request => {
            const response = await f.transport(request) as { source_host_id: string; result: unknown };
            if (scenario === 'source') return { ...response, source_host_id: 'foreign' };
            if (request.operation === 'terminal_seal') {
                if (scenario === 'missing') return { ...response, result: null };
                if (scenario === 'count') return { ...response, result: { ...f.seal, heads_count: '2' } };
                if (scenario === 'root') return { ...response, result: { ...f.seal, identity: { ...f.seal.identity, assistant_message_id: 'foreign' } } };
            }
            if (scenario === 'page' && request.operation === 'terminal_manifest') return { ...response, result: { ...(response.result as object), seal: { ...f.seal, seal_digest: 'd'.repeat(64) } } };
            if (scenario.startsWith('missing_message_') && request.operation === 'terminal_manifest') {
                const page = structuredClone(response.result) as { entries: Record<string, unknown>[] };
                delete page.entries[0][scenario === 'missing_message_state' ? 'state' : 'completed_at_ms'];
                return { ...response, result: page };
            }
            return response;
        };
        const read = readTerminal(state, f.reader, transport, () => {}, () => true, scenario === 'watermark' ? 'd'.repeat(64) : undefined);
        if (scenario === 'missing') assert.equal(await read, null); else await assert.rejects(read);
        assert.equal(canReleaseChatTurn(state), false);
    }
});

test('pagination checks every assistant and child receipt without rescanning snapshots', async () => {
    const state = turn(); const f = terminalFixture(state, 'completed', ['follow-up']);
    const seal = { ...f.seal, result: { ...f.seal.result, value: {
        kind: 'failed', error_ref: 'failure', observed_effect_refs: [], unknown_effect_refs: [],
    } }, entry_count: '3', heads_count: '2', child_receipt_count: '1' };
    const second = { ...f.through, identity: { ...f.through.identity, assistant_message_id: 'follow-up' } };
    const entries = [
        { kind: 'assistant', through: f.through, ordinal: 1, created_at_ms: 1000, state: 'completed', completed_at_ms: 1200 },
        { kind: 'assistant', through: second, ordinal: 2, created_at_ms: 1001, state: 'failed', completed_at_ms: 1234 },
        { kind: 'child_receipt', task_id: 'task', receipt_id: 'receipt', parent_turn_id: 'parent',
            child_session_id: 'child-session', child_run_id: 'child-run', status: 'sealed',
            input_watermark: null, completion_watermark: null, receipt_digest: 'd'.repeat(64), result_digest: 'e'.repeat(64) },
    ];
    const offsets: unknown[] = []; let currentSnapshot = f.through;
    const transport: JournalTransport = async request => {
        let result: unknown;
        if (request.operation === 'admissions' || request.operation === 'admission_head') return f.transport(request);
        if (request.operation === 'terminal_seal') result = seal;
        else if (request.operation === 'terminal_manifest') {
            offsets.push(request.after_entry);
            const index = Number(request.after_entry);
            result = { seal, after_entry: String(index), next_entry: String(index + 1), complete: index === 2, entries: [entries[index]] };
        } else if (request.operation === 'open_snapshot') { currentSnapshot = request.at as typeof f.through; result = { snapshot_id: 'snapshot', at: currentSnapshot }; }
        else if (request.operation === 'parts') result = { at: currentSnapshot, parts: [], next_part_id: null };
        else if (request.operation === 'release_snapshot') result = null;
        else throw new Error('Unexpected operation');
        return { source_host_id: owner.hostId, result };
    };
    const reader = new AssistantJournalReader(owner.hostId, owner.conversationId, owner.generation, transport);
    const applied: string[] = [];
    let timeline = createChatTimeline(owner);
    const evidence = (await readTerminal(state, reader, transport, update => {
        if (update) {
            applied.push(update.through.identity.assistant_message_id);
            timeline = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId, runId: update.through.identity.run_id, update }).state;
        }
    }, () => true))!;
    assert.deepEqual(offsets, ['0', '1', '2']);
    assert.deepEqual(applied, [state.identity.assistantMessageId, 'follow-up']);
    assert.equal(evidence.heads.length, 2);
    timeline = reduceChatTimeline(timeline, { type: 'terminal', owner, evidence }).state;
    assert.deepEqual(timeline.messages.map(message => message.state), ['completed', 'failed'], 'later Run failure cannot rewrite an earlier successful owner');
});

test('cancelled UI generation, unapplied head and incomplete pagination never mint evidence', async () => {
    const state = turn();
    const f = terminalFixture(state);
    await assert.rejects(readTerminal(state, f.reader, f.transport, () => { throw new Error('timeline rejected'); }, () => true), /timeline rejected/);
    await assert.rejects(readTerminal(state, f.reader, f.transport, () => {}, () => false), /Stale/);
    const broken: JournalTransport = async request => request.operation === 'terminal_manifest'
        ? { source_host_id: owner.hostId, result: { seal: f.seal, after_entry: '0', next_entry: '0', complete: false, entries: [] } }
        : f.transport(request);
    await assert.rejects(readTerminal(state, f.reader, broken, () => {}, () => true), /advance/);
});

test('immutable completion frontier accepts a later authenticated background journal head', async () => {
    const state = turn(); const f = terminalFixture(state);
    const later = { ...f.through, sequence: '9007199254740994', record_digest: 'd'.repeat(64) };
    let snapshots = 0;
    const transport: JournalTransport = async request => {
        if (request.operation === 'open_snapshot') { snapshots++; return { source_host_id: owner.hostId, result: { snapshot_id: 'later', at: request.at } }; }
        if (request.operation === 'parts') return { source_host_id: owner.hostId, result: { at: later, parts: [], next_part_id: null } };
        return f.transport(request);
    };
    const reader = new AssistantJournalReader(owner.hostId, owner.conversationId, owner.generation, transport);
    let timeline = createChatTimeline(owner);
    const update = (await reader.read(later, state.identity.submissionId))!;
    timeline = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId, runId: state.identity.submissionId, update }).state;
    const evidence = (await readTerminal(state, reader, transport, result => assert.equal(result, null), () => true))!;
    assert.equal(snapshots, 1, 'seal catchup must not roll back or reload a newer verified head');
    assert.equal(reduceChatTimeline(timeline, { type: 'terminal', owner, evidence }).ignored, undefined);
});

test('cancelled and failed committed runs advance only their own submission frontier', async () => {
    for (const status of ['cancelled', 'failed'] as const) {
        const current = turn(); const fixture = terminalFixture(current, status);
        let timeline = createChatTimeline(owner);
        const evidence = (await fixture.verify(update => {
            if (update) timeline = reduceChatTimeline(timeline, {type: 'journal', owner, sourceHostId: owner.hostId, runId: update.through.identity.run_id, update}).state;
        }))!;
        timeline = reduceChatTimeline(timeline, {type: 'terminal', owner, evidence}).state;
        assert.equal(timeline.submissionParentMessageId, current.identity.assistantMessageId);
        const foreign = createChatTimeline({...owner, conversationId: 'different'});
        assert.equal(reduceChatTimeline(foreign, {type: 'terminal', owner, evidence}).state.submissionParentMessageId, null);
    }
});
