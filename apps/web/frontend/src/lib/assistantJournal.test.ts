import test from 'node:test';
import assert from 'node:assert/strict';
import { AssistantJournalReader, journalNumber, parseJournalStructured } from './assistantJournal.ts';
import type { JournalCursor, JournalTransport, JournalUpdate } from './assistantJournal.ts';
import { createChatTimeline, reduceChatTimeline, timelineMessageText } from './chatTimeline.ts';
import { createChatTurn, reduceChatTurn, canReleaseChatTurn } from './chatTurn.ts';

const identity = { source_store_epoch: 'epoch', conversation_id: 'conversation', run_id: 'run', assistant_message_id: 'run:assistant' };
const cursor = (sequence: string, hash = 'a'): JournalCursor => ({ identity, sequence, record_digest: hash.repeat(64) });
const notification = (through: JournalCursor) => ({ type: 'assistant_journal_committed_v1', session_id: identity.conversation_id, source_host_id: 'host', through });
const body = (text: string, generation = '0', hash = 'b') => ({ field: 'markdown', content_generation: generation,
    utf8_bytes: String(new TextEncoder().encode(text).length), chunk_count: text ? '1' : '0', content_root: hash.repeat(64) });
const head = (at: JournalCursor, text: string, generation = '0', hash = 'b') => ({ identity, part_id: 'text', generation, revision: at.sequence,
    metadata_revision: at.sequence, order_key: { section: 'transcript', position: '9007199254740993' },
    metadata: { kind: 'text', data: { is_final: false, visibility: 'public', review: null } }, bodies: [body(text, generation, hash)] });
function fixture() {
    const requests: Record<string, unknown>[] = [];
    let current = cursor('9007199254740993');
    let text = '你'; let part = head(current, text);
    const transport: JournalTransport = async request => {
        requests.push(request);
        let result: unknown;
        switch (request.operation) {
            case 'open_snapshot': result = { snapshot_id: 'snapshot', at: request.at }; break;
            case 'release_snapshot': result = null; break;
            case 'parts': result = { at: current, parts: [part], next_part_id: null }; break;
            case 'changes': result = { base: request.base, at: current, changes: [{ part_id: 'text', head: part }], next_part_id: null }; break;
            case 'range': {
                assert.ok(Number(request.limit_bytes) >= 4 && Number(request.limit_bytes) <= 32768, 'Core raw-byte range contract');
                const bytes = new TextEncoder().encode(text);
                const offset = Number(request.offset_bytes);
                const end = Math.min(bytes.length, offset + Number(request.limit_bytes));
                result = { offset_bytes: request.offset_bytes, next_offset_bytes: String(end), total_utf8_bytes: String(bytes.length),
                    text: new TextDecoder('utf-8', { fatal: true }).decode(bytes.slice(offset, end)), end_of_content: end === bytes.length };
                break;
            }
            default: throw new Error(`Unexpected operation ${request.operation}`);
        }
        return { source_host_id: 'host', result };
    };
    const reader = new AssistantJournalReader('host', 'conversation', 4, transport);
    return { reader, requests, transport, get current() { return current; },
        advance(value: string, generation = '0') {
            text = value; current = cursor((BigInt(current.sequence) + 1n).toString(), 'c'); part = head(current, text, generation, 'd');
        } };
}

test('journal counters reject JS numbers and preserve the complete u64 domain', () => {
    assert.equal(journalNumber('18446744073709551615'), '18446744073709551615');
    for (const value of [1, 9007199254740992, '01', '-1', '18446744073709551616']) assert.throws(() => journalNumber(value));
});

test('structured fields unwrap their typed payload and preserve large numeric tokens', () => {
    assert.deepEqual(parseJournalStructured('{"protocol_version":1,"value":{"id":18446744073709551615,"small":2,"fraction":0.12345678901234567890,"quoted":"12345678901234567890"}}', 'value'),
        { id: '18446744073709551615', small: 2, fraction: '0.12345678901234567890', quoted: '12345678901234567890' });
    assert.deepEqual(parseJournalStructured('{"protocol_version":1,"artifacts":[]}', 'artifacts'), []);
    assert.deepEqual(parseJournalStructured('{"protocol_version":1,"resources":[]}', 'resources'), []);
    assert.deepEqual(parseJournalStructured('{"protocol_version":1,"evidence":{"title":"source"}}', 'evidence'), { title: 'source' });
    assert.throws(() => parseJournalStructured('{"protocol_version":2,"value":null}', 'value'));
    assert.throws(() => parseJournalStructured('{"protocol_version":1,"artifacts":[]}', 'value'));
});

test('actual reader → timeline appends UTF-8 ranges without rescanning heads or unlocking the turn', async () => {
    const f = fixture();
    const owner = { conversationId: 'conversation', generation: 4, hostId: 'host', clientId: 'browser' };
    let timeline = reduceChatTimeline(createChatTimeline(owner), { type: 'beginSubmission', owner, submissionId: 'run' }).state;
    let turn = createChatTurn(owner, 'run');
    const apply = (update: JournalUpdate) => {
        const result = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: 'host', runId: 'run', update });
        assert.equal(result.ignored, undefined); timeline = result.state;
    };
    apply((await f.reader.notification(notification(f.current), 'run'))!);
    assert.equal(timelineMessageText(timeline.messages[0]), '你');
    assert.equal(timeline.messages[0].parts[0].value.revision, '9007199254740993');
    f.advance('你好🙂');
    apply((await f.reader.notification(notification(f.current), 'run'))!);
    assert.equal(timelineMessageText(timeline.messages[0]), '你好🙂');
    assert.equal(f.requests.filter(request => request.operation === 'parts').length, 1);
    assert.equal(f.requests.filter(request => request.operation === 'changes').length, 1);
    assert.deepEqual(f.requests.filter(request => request.operation === 'range').map(request => request.offset_bytes), ['0', '3']);
    assert.equal(f.requests.filter(request => request.operation === 'release_snapshot').length, 2);
    turn = reduceChatTurn(turn, { type: 'event', owner, payload: notification(f.current) }).state;
    assert.equal(canReleaseChatTurn(turn), false);
    assert.equal(timeline.durable, false);
    const count = f.requests.length;
    assert.equal(await f.reader.notification(notification(f.current), 'run'), null);
    assert.equal(f.requests.length, count, 'exact duplicate performs no reads');
    await assert.rejects(f.reader.notification(notification(cursor(f.current.sequence, 'f')), 'run'), /Conflicting/);
    const stale = reduceChatTimeline(timeline, { type: 'journal', owner: { ...owner, generation: 3 }, sourceHostId: 'host', runId: 'run', update: (await (fixture().reader).notification(notification(cursor('9007199254740993')), 'run'))! });
    assert.equal(stale.ignored, 'owner');
});

test('replacement generation reads from zero and epoch/Run/transport ownership cannot be borrowed', async () => {
    const f = fixture();
    await f.reader.notification(notification(f.current), 'run');
    f.advance('替换', '1');
    const update = await f.reader.notification(notification(f.current), 'run');
    assert.equal(update!.changes[0].part!.value.markdown, '替换');
    assert.deepEqual(f.requests.filter(request => request.operation === 'range').map(request => request.offset_bytes), ['0', '0']);
    await assert.rejects(f.reader.notification({ ...notification(f.current), source_host_id: 'local' }, 'run'), /owner/);
    await assert.rejects(f.reader.notification(notification(f.current), 'another-run'), /Run/);
    await assert.rejects(f.reader.read({ ...f.current, identity: { ...identity, source_store_epoch: 'other-epoch' } }, 'run'), /epoch/);
    const foreign = new AssistantJournalReader('host', 'conversation', 4, async () => ({ source_host_id: 'other-host', result: {} }));
    await assert.rejects(foreign.read(f.current, 'run'), /source/);
});

test('empty directory pages continue; withheld content is never fetched; retirement is explicit', async () => {
    const first = cursor('1'); const second = cursor('2', 'd');
    const requests: Record<string, unknown>[] = [];
    const reader = new AssistantJournalReader('host', 'conversation', 1, async request => {
        requests.push(request);
        const at = request.operation === 'open_snapshot' ? request.at : (request.snapshot as { at: JournalCursor })?.at;
        let result: unknown;
        if (request.operation === 'open_snapshot') result = { snapshot_id: 'snapshot', at };
        else if (request.operation === 'release_snapshot') result = null;
        else if (request.operation === 'parts') {
            const withheld = head(first, 'never fetch'); withheld.metadata.data.visibility = 'withheld';
            result = request.after_part_id === null ? { at, parts: [], next_part_id: 'retired-directory-entry' }
                : { at, parts: [withheld], next_part_id: null };
        } else if (request.operation === 'changes') result = { base: first, at: second, changes: [{ part_id: 'text', head: null }], next_part_id: null };
        else throw new Error('Withheld body was fetched');
        return { source_host_id: 'host', result };
    });
    const shown = await reader.read(first, 'run');
    assert.equal(shown!.changes[0].part!.value.markdown, '');
    assert.equal(requests.filter(request => request.operation === 'parts').length, 2);
    const removed = await reader.read(second, 'run');
    assert.deepEqual(removed!.changes, [{ partId: 'text', part: null }]);
});

test('malformed pages release pinned snapshots and aborted generations cannot publish', async () => {
    let released = 0;
    const at = cursor('1');
    const reader = new AssistantJournalReader('host', 'conversation', 1, async request => {
        if (request.operation === 'open_snapshot') return { source_host_id: 'host', result: { snapshot_id: 'snapshot', at } };
        if (request.operation === 'release_snapshot') { released++; return { source_host_id: 'host', result: null }; }
        return { source_host_id: 'host', result: { at: { ...at, identity: { ...identity, run_id: 'other' } }, parts: [], next_part_id: null } };
    });
    await assert.rejects(reader.read(at, 'run'), /page/);
    assert.equal(released, 1);
    reader.dispose();
    await assert.rejects(reader.read(at, 'run'), /disposed/);
});

test('legacy terminal aggregate supplies neither terminal authority nor replacement body', async () => {
    const f = fixture(); const owner = { conversationId: 'conversation', generation: 4 };
    const update = (await f.reader.read(f.current, 'run'))!;
    let state = reduceChatTimeline(createChatTimeline(owner), { type: 'journal', owner, sourceHostId: 'host', runId: 'run', update }).state;
    // The timeline has no wire-event action: only verified reader output enters it.
    const unchanged = reduceChatTimeline(state, { type: 'conversation_projection_committed_v1', owner } as unknown as import('./chatTimeline.ts').TimelineAction);
    assert.equal(unchanged.ignored, 'unsupported');
    assert.equal(unchanged.state, state);
    assert.equal(state.messages[0].state, 'running');
    assert.equal(timelineMessageText(state.messages[0]), '你');
});

test('malformed snapshot watermark still releases the acquired handle', async () => {
    const requests: Record<string, unknown>[] = [];
    const at = cursor('1');
    const reader = new AssistantJournalReader('host', 'conversation', 1, async request => {
        requests.push(request);
        return { source_host_id: 'host', result: request.operation === 'open_snapshot'
            ? { snapshot_id: 'snapshot', at: { sequence: 1 } } : null };
    });
    await assert.rejects(reader.read(at, 'run'), /cursor/);
    assert.deepEqual(requests.map(request => request.operation), ['open_snapshot', 'release_snapshot']);
    assert.deepEqual(requests[1].snapshot, { snapshot_id: 'snapshot', at });
});

test('tool presentation decodes typed bodies and nested display metadata, not envelopes', async () => {
    const at = cursor('1');
    const fields: Record<string, string> = {
        tool_input: '{"protocol_version":1,"value":{"command":"pwd"}}',
        tool_output: '{"protocol_version":1,"value":{"status":"ok"}}',
        tool_artifacts: '{"protocol_version":1,"artifacts":[{"uri":"file:///result"}]}',
        tool_resources: '{"protocol_version":1,"resources":[]}',
        tool_output_text: 'output', title: 'Shell', summary: 'Read directory',
    };
    const reader = new AssistantJournalReader('host', 'conversation', 1, async request => {
        let result: unknown = null;
        if (request.operation === 'open_snapshot') result = { snapshot_id: 'snapshot', at };
        if (request.operation === 'parts') result = { at, next_part_id: null, parts: [{ ...head(at, ''), metadata: { kind: 'tool', data: {
            title: { kind: 'field', value: 'title' }, summary: { kind: 'field', value: 'summary' },
            affected_resources: { kind: 'field', value: 'tool_resources' },
            permission: { request_id: 'permission', permission_domain: { kind: 'inline', value: 'filesystem' }, rationale: { kind: 'inline', value: 'Read only' }, decision: null },
            origin: { kind: 'builtin', provider_id: null, provider_title: null, namespace: null },
        } }, bodies: Object.entries(fields).map(([field, text]) => ({ ...body(text), field })) }] };
        if (request.operation === 'range') {
            const content = request.content as { field: string; utf8_bytes: string };
            result = { offset_bytes: '0', next_offset_bytes: content.utf8_bytes, total_utf8_bytes: content.utf8_bytes, text: fields[content.field], end_of_content: true };
        }
        return { source_host_id: 'host', result };
    });
    const value = (await reader.read(at, 'run'))!.changes[0].part!.value;
    assert.deepEqual(value.input, { command: 'pwd' });
    assert.deepEqual(value.output, { status: 'ok' });
    assert.deepEqual(value.artifacts, [{ uri: 'file:///result' }]);
    assert.deepEqual(value.affected_resources, []);
    assert.equal(value.title, 'Shell');
    assert.equal(value.summary, 'Read directory');
    assert.equal(value.output_preview, 'output');
    assert.deepEqual(value.permission, { request_id: 'permission', permission_domain: 'filesystem', rationale: 'Read only', decision: null });
});

test('large real journal content is requested in Core-compatible bounded ranges', async () => {
    const f = fixture();
    f.advance('a'.repeat(70000));
    await f.reader.notification(notification(f.current), 'run');
    const ranges = f.requests.filter(request => request.operation === 'range');
    assert.deepEqual(ranges.map(request => request.offset_bytes), ['0', '32768', '65536']);
    assert.ok(ranges.every(request => request.limit_bytes === 32768));
});

test('UI surface journal preserves plain summary and independent surface revision', async () => {
    const at = cursor('90');
    const surface = { identity: { owner: { authority_id: 'host', conversation_id: 'conversation',
        participant_id: 'agent', context_id: 'context', originating_run_id: 'run',
        message_id: 'run:assistant', part_id: 'render-tool' }, surface_id: 'form', generation: 1 }, revision: 2 };
    const reader = new AssistantJournalReader('host', 'conversation', 1, async request => {
        const result = request.operation === 'open_snapshot' ? {snapshot_id: 'snapshot', at}
            : request.operation === 'release_snapshot' ? null
            : request.operation === 'parts' ? {at, next_part_id: null, parts: [{
                ...head(at, ''), part_id: 'ui-surface:hash', bodies: [],
                metadata: {kind: 'ui_surface', data: {surface, summary: '填写姓名'}},
            }]} : (() => { throw new Error(`Unexpected ${request.operation}`); })();
        return {source_host_id: 'host', result};
    });
    const update = (await reader.notification(notification(at), 'run'))!;
    const owner = {conversationId: 'conversation', generation: 1};
    const initial = reduceChatTimeline(createChatTimeline(owner), {type: 'beginSubmission', owner, submissionId: 'run'}).state;
    const timeline = reduceChatTimeline(initial, {type: 'journal', owner, sourceHostId: 'host', runId: 'run', update}).state;
    const part = timeline.messages[0].parts.find(part => part.value.kind === 'ui_surface');
    assert.equal(part?.value.summary, '填写姓名');
    assert.deepEqual(part?.value.surface, surface);
    assert.equal(part?.value.revision, '90');
});


test('TaskControl keeps its distinct Core tag through the journal reader', async () => {
    const at = cursor('1');
    const task = {task_id: 'task', display_key: 'task.background', state: 'running', progress: null, cancel_requested: false};
    const parts = ['task', 'task_control'].map((kind, index) => ({identity, part_id: `part-${kind}`, generation: '1', revision: '1', metadata_revision: '1',
        order_key: {section: 'tasks', position: String(index + 1)}, bodies: [],
        metadata: {kind, data: kind === 'task_control' ? {task, supports_guidance: true, supports_cancellation: true} : task}}));
    const reader = new AssistantJournalReader('host', 'conversation', 1, async request => {
        const result = request.operation === 'open_snapshot' ? {snapshot_id: 'task-snapshot', at}
            : request.operation === 'release_snapshot' ? null
            : request.operation === 'parts' ? {at, parts, next_part_id: null}
            : (() => { throw new Error(`Unexpected operation ${request.operation}`); })();
        return {source_host_id: 'host', result};
    });
    try {
        const update = await reader.read(at, 'run');
        assert.deepEqual(update!.changes.map(change => change.part!.value.kind), ['task', 'task_control']);
        const {taskControl} = await import('./chatTaskActions.ts');
        const owner = {hostId: 'host', conversationId: 'conversation', runId: 'run'};
        assert.equal(taskControl(update!.changes[0].part!, owner), null);
        assert.equal(taskControl(update!.changes[1].part!, owner)?.guidance, true);
    } finally { reader.dispose(); }
});

test('cold parts accept four pages with descending IDs across sections', async () => {
    const at = cursor('42'); let page = 0; const ids = ['z', 'm', 'b', 'a'];
    const reader = new AssistantJournalReader('host', 'conversation', 4, async request => {
        let result: unknown;
        if (request.operation === 'open_snapshot') result = {snapshot_id:'ordered', at};
        else if (request.operation === 'release_snapshot') result = null;
        else if (request.operation === 'parts') {
            const index = page++;
            result = {at, parts:[{...head(at, ''), part_id:ids[index], order_key:{section:index < 2 ? 'transcript' : 'tasks', position:String(index + 1)}}], next_part_id:index === 3 ? null : ids[index]};
        } else throw new Error('unexpected operation');
        return {source_host_id:'host', result};
    });
    const update = await reader.read(at, 'run');
    assert.equal(page, 4); assert.equal(update?.changes.length, 4);
});
