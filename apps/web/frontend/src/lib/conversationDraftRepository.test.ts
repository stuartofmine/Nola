import test from 'node:test';
import assert from 'node:assert/strict';
import { createWorkspaceDrafts } from './workspaceDrafts.ts';
import { validateConversationDraft, type ConversationDraftRecord, type ConversationDraftRepository } from './conversationDraftRepository.ts';
import { createSourceSubmission } from './sourceSubmission.ts';

test('submission retirement is owner-scoped and preserves newer input rather than matching text', async () => {
    const host = createWorkspaceDrafts().forHost('host');
    await host.load('a'); await host.load('b');
    host.text.write('a', 'original'); host.text.write('b', 'other owner');
    const identity = {source_host_id: 'host', client_id: 'client', conversation_id: 'a', submission_id: 'submission', user_message_id: 'user', assistant_message_id: 'assistant'};
    const pending = createSourceSubmission(identity, {conversationId: 'a', expected_host_id: 'host', client_id: 'client', submission_id: 'submission', user_message_id: 'user', assistant_message_id: 'assistant', messages: [{role: 'user', content: 'original'}]});
    await host.stageSubmission(pending);
    // Even an edit back to identical text is a new draft version.
    host.text.write('a', 'new'); host.text.write('a', 'original');
    await host.retireSubmission('submission');
    assert.equal(host.text.read('a'), 'original');
    assert.equal(host.text.read('b'), 'other owner');
});

test('accepted original draft retires once and late duplicate does not erase the successor', async () => {
    const host = createWorkspaceDrafts().forHost('host'); await host.load('a');
    host.text.write('a', 'original');
    const identity = {source_host_id: 'host', client_id: 'client', conversation_id: 'a', submission_id: 'submission', user_message_id: 'user', assistant_message_id: 'assistant'};
    await host.stageSubmission(createSourceSubmission(identity, {conversationId: 'a', expected_host_id: 'host', client_id: 'client', submission_id: 'submission', user_message_id: 'user', assistant_message_id: 'assistant', messages: [{role: 'user', content: 'original'}]}));
    await host.retireSubmission('submission'); assert.equal(host.text.read('a'), '');
    host.text.write('a', 'successor'); await host.retireSubmission('submission');
    assert.equal(host.text.read('a'), 'successor');
});

class MemoryDraftRepository implements ConversationDraftRepository {
    rows = new Map<string, ConversationDraftRecord>();
    key(host: string, id: string) { return JSON.stringify([host, id]); }
    async read(host: string, id: string) { return structuredClone(this.rows.get(this.key(host, id)) ?? null); }
    async write(record: ConversationDraftRecord) { this.rows.set(this.key(record.hostId, record.conversationId), validateConversationDraft(record)); }
    async delete(host: string, id: string) { this.rows.delete(this.key(host, id)); }
}

test('expert parent acknowledgement clears only its captured text after durable persistence', async () => {
    const repository = new MemoryDraftRepository(), workspace = createWorkspaceDrafts(repository), host = workspace.forHost('host');
    await host.load('expert'); await host.load('other');
    host.text.write('expert', 'submitted'); host.text.write('other', 'other draft'); host.objectives.write('expert', 'keep goal');
    const attachment = {id:'kept', conversationId:'expert', name:'note.txt', mimeType:'text/plain', byteCount:4, text:'kept', clipped:false};
    host.attachments.write('expert', [attachment]); await host.flush();
    host.captureExpertAdmission('expert', 'expert-parent', 'submitted');
    assert.equal(await host.acknowledgeExpertAdmission('expert', 'expert-parent'), true);
    assert.equal(host.text.read('expert'), ''); assert.equal(host.text.read('other'), 'other draft');
    assert.equal(host.objectives.read('expert'), 'keep goal'); assert.deepEqual(host.attachments.read('expert'), [attachment]);
    const cold = createWorkspaceDrafts(repository).forHost('host'); await cold.load('expert');
    assert.equal(cold.text.read('expert'), ''); assert.equal(cold.objectives.read('expert'), 'keep goal');
    assert.deepEqual(cold.attachments.read('expert'), [attachment]);
});

test('expert retry never recaptures the ABA edit back to identical submitted text', async () => {
    const repository = new MemoryDraftRepository(), host = createWorkspaceDrafts(repository).forHost('host'); await host.load('expert');
    host.text.write('expert', 'same text'); host.captureExpertAdmission('expert', 'expert-parent', 'same text');
    host.text.write('expert', ''); host.text.write('expert', 'same text');
    host.captureExpertAdmission('expert', 'expert-parent', 'same text');
    assert.equal(await host.acknowledgeExpertAdmission('expert', 'expert-parent'), false);
    assert.equal(host.text.read('expert'), 'same text'); assert.equal((await repository.read('host', 'expert'))?.text, 'same text');
});

test('edits during the awaited expert clear are queued after it and survive in memory and storage', async () => {
    const repository = new MemoryDraftRepository(), host = createWorkspaceDrafts(repository).forHost('host'); await host.load('expert');
    host.text.write('expert', 'original'); await host.flush(); host.captureExpertAdmission('expert', 'expert-parent', 'original');
    const originalWrite = repository.write.bind(repository);
    let release!: () => void, started!: () => void;
    const began = new Promise<void>(resolve => { started = resolve; });
    const suspended = new Promise<void>(resolve => { release = resolve; });
    repository.write = async record => { if (record.text === '') { started(); await suspended; } await originalWrite(record); };
    const acknowledged = host.acknowledgeExpertAdmission('expert', 'expert-parent');
    await began;
    host.text.write('expert', ''); host.text.write('expert', 'original'); // Same bytes, new edit witness.
    release();
    assert.equal(await acknowledged, false);
    assert.equal(host.text.read('expert'), 'original'); assert.equal((await repository.read('host', 'expert'))?.text, 'original');
});

test('failed durable expert acknowledgement preserves its draft and original retry witness', async () => {
    const repository = new MemoryDraftRepository(), host = createWorkspaceDrafts(repository).forHost('host'); await host.load('expert');
    host.text.write('expert', 'original'); await host.flush(); host.captureExpertAdmission('expert', 'expert-parent', 'original');
    const originalWrite = repository.write.bind(repository);
    repository.write = async () => { throw new Error('storage unavailable'); };
    await assert.rejects(host.acknowledgeExpertAdmission('expert', 'expert-parent'), /storage unavailable/);
    assert.equal(host.text.read('expert'), 'original'); assert.equal(host.persistenceErrors.get('expert'), true);
    host.text.write('expert', ''); host.text.write('expert', 'original');
    repository.write = originalWrite;
    assert.equal(await host.acknowledgeExpertAdmission('expert', 'expert-parent'), false);
    assert.equal(host.text.read('expert'), 'original'); assert.equal((await repository.read('host', 'expert'))?.text, 'original');
});

test('later queued draft failure rejects acknowledgement even after its durable clear succeeded', async () => {
    const repository = new MemoryDraftRepository(), host = createWorkspaceDrafts(repository).forHost('host'); await host.load('expert');
    host.text.write('expert', 'original'); await host.flush(); host.captureExpertAdmission('expert', 'expert-parent', 'original');
    const originalWrite = repository.write.bind(repository);
    let release!: () => void, started!: () => void;
    const began = new Promise<void>(resolve => { started = resolve; });
    const suspended = new Promise<void>(resolve => { release = resolve; });
    repository.write = async record => {
        if (record.text === '') { started(); await suspended; await originalWrite(record); }
        else { throw new Error('new draft storage unavailable'); }
    };
    const acknowledged = host.acknowledgeExpertAdmission('expert', 'expert-parent');
    await began; host.text.write('expert', 'successor'); release();
    // Chat cannot reach clearExpertIntent after this awaited operation rejects.
    await assert.rejects(acknowledged, /persistence is unconfirmed/);
    assert.equal(host.text.read('expert'), 'successor'); assert.equal(host.persistenceErrors.get('expert'), true);
    assert.equal((await repository.read('host', 'expert'))?.text, '', 'The original clear did commit; the successor remains an unsaved local draft');
    repository.write = originalWrite;
    assert.equal(await host.acknowledgeExpertAdmission('expert', 'expert-parent'), false);
    assert.equal((await repository.read('host', 'expert'))?.text, 'successor');
});

test('expert acknowledgement without its original local ticket preserves cold drafts and rejects retired owners', async () => {
    const repository = new MemoryDraftRepository(), workspace = createWorkspaceDrafts(repository), host = workspace.forHost('host'); await host.load('expert');
    host.text.write('expert', 'saved text'); await host.flush(); host.captureExpertAdmission('expert', 'expert-parent', 'saved text');
    const cold = createWorkspaceDrafts(repository).forHost('host'); await cold.load('expert');
    assert.equal(await cold.acknowledgeExpertAdmission('expert', 'expert-parent'), false);
    assert.equal(cold.text.read('expert'), 'saved text');
    const other = workspace.forHost('other-host'); await other.load('expert'); other.text.write('expert', 'new Host draft');
    await assert.rejects(host.acknowledgeExpertAdmission('expert', 'expert-parent'), /owner unavailable/);
    assert.equal(other.text.read('expert'), 'new Host draft');
    await assert.rejects(cold.acknowledgeExpertAdmission('other-conversation', 'expert-parent'), /owner unavailable/);
    cold.forget('expert');
    await assert.rejects(cold.acknowledgeExpertAdmission('expert', 'expert-parent'), /owner unavailable/);
});

test('drafts restore after workspace recreation and isolate the same conversation ID across Hosts', async () => {
    const repository = new MemoryDraftRepository();
    const first = createWorkspaceDrafts(repository).forHost('host-a');
    await first.load('same'); first.text.write('same', 'A'); first.objectives.write('same', 'goal A'); await first.flush();
    const workspace = createWorkspaceDrafts(repository);
    const second = workspace.forHost('host-b'); await second.load('same');
    assert.equal(second.text.read('same'), ''); second.text.write('same', 'B'); await second.flush();
    const restored = workspace.forHost('host-a'); await restored.load('same');
    assert.equal(restored.text.read('same'), 'A'); assert.equal(restored.objectives.read('same'), 'goal A');
});

test('ordered writes and deletion cannot resurrect a forgotten conversation', async () => {
    const repository = new MemoryDraftRepository();
    const workspace = createWorkspaceDrafts(repository), host = workspace.forHost('host');
    await host.load('one'); host.text.write('one', 'first'); host.text.write('one', 'last');
    await host.flush(); assert.equal((await repository.read('host', 'one'))?.text, 'last');
    workspace.forgetConversation('one'); host.text.write('one', 'late'); await host.flush();
    assert.equal(await repository.read('host', 'one'), null);
    assert.equal(host.isDeleted('one'), true);
});

test('load errors are distinct from missing drafts and malformed references are rejected', async () => {
    const repository = new MemoryDraftRepository();
    repository.read = async () => { throw new Error('offline'); };
    const host = createWorkspaceDrafts(repository).forHost('host'); await host.load('one');
    assert.equal(host.isLoaded('one'), false); assert.equal(host.persistenceErrors.has('one'), true);
    assert.throws(() => validateConversationDraft({hostId: 'h', conversationId: 'a', text: '', objective: '',
        attachments: [{id: 'file', conversationId: 'b', name: 'x', mimeType: 'text/plain', text: 'x', byteCount: 1, clipped: false}]}));
});

test('deletion during hydration discards the late record', async () => {
    const repository = new MemoryDraftRepository();
    let resolve!: (value: ConversationDraftRecord | null) => void;
    repository.read = () => new Promise(done => { resolve = done; });
    const workspace = createWorkspaceDrafts(repository), host = workspace.forHost('host');
    const loading = host.load('one'); await Promise.resolve();
    workspace.forgetConversation('one');
    resolve({hostId: 'host', conversationId: 'one', text: 'stale', objective: '', attachments: []});
    await loading; await host.flush(); assert.equal(host.text.read('one'), '');
});

test('editing before hydration preserves untouched stored fields', async () => {
    const repository = new MemoryDraftRepository();
    await repository.write({hostId: 'host', conversationId: 'one', text: 'old', objective: 'saved goal', attachments: []});
    const host = createWorkspaceDrafts(repository).forHost('host');
    host.text.write('one', 'new typing'); await host.flush();
    assert.equal(host.objectives.read('one'), 'saved goal');
    assert.equal((await repository.read('host', 'one'))?.text, 'new typing');
    assert.equal((await repository.read('host', 'one'))?.objective, 'saved goal');
});

test('clearing the workspace creates a new projection and restores durable drafts', async () => {
    const repository = new MemoryDraftRepository();
    const workspace = createWorkspaceDrafts(repository), first = workspace.forHost('host');
    await first.load('one'); first.text.write('one', 'saved'); await first.flush();
    workspace.clear();
    const restored = workspace.forHost('host');
    assert.notEqual(restored, first); await restored.load('one');
    assert.equal(restored.text.read('one'), 'saved');
    first.text.write('one', 'late old projection'); await first.flush();
    assert.equal((await repository.read('host', 'one'))?.text, 'saved');
});

test('cold receipt reconciliation restores a consumed ticket without erasing a successor draft', async () => {
    const identity = {source_host_id:'host',client_id:'client',conversation_id:'a',submission_id:'submitted',user_message_id:'user',assistant_message_id:'assistant'};
    const pending = createSourceSubmission(identity, {conversationId:'a',expected_host_id:'host',client_id:'client',submission_id:'submitted',user_message_id:'user',assistant_message_id:'assistant',messages:[{role:'user',content:'original'}]});
    const ticket = {hostId:'host',clientId:'client',conversationId:'a',submissionId:'submitted',draftRevision:1,pending,committed:true};
    const repository = new MemoryDraftRepository();
    await repository.write({hostId:'host',conversationId:'a',text:'successor',objective:'',attachments:[]});
    let writes = 0;
    const durable: ConversationDraftRepository = Object.assign(repository, {
        pendingSubmissions: async () => [],
        readSubmission: async (hostId: string, clientId: string, submissionId: string) =>
            hostId === ticket.hostId && clientId === ticket.clientId && submissionId === ticket.submissionId ? structuredClone(ticket) : null,
        updateSubmission: async () => { writes++; },
        retireSubmission: async () => false,
    });
    const host = createWorkspaceDrafts(durable).forHost('host');
    await host.load('a');
    assert.deepEqual(await host.restoreSubmissions('client'), []);
    await host.updateSubmission(pending);
    await host.retireSubmission('submitted');
    assert.equal(writes, 1);
    assert.equal(host.text.read('a'), 'successor');
    const foreign = createWorkspaceDrafts(durable).forHost('other');
    await assert.rejects(foreign.updateSubmission(pending), /durable draft submission/);
    const changed = structuredClone(pending); changed.request.messages = [{role:'user',content:'changed'}];
    const cold = createWorkspaceDrafts(durable).forHost('host');
    await assert.rejects(cold.updateSubmission(changed), /durable draft submission/);
    assert.equal(writes, 1);
});
