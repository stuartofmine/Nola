import test from 'node:test';
import assert from 'node:assert/strict';
import { restoreBrowserWorkspaceCompletions } from './browserWorkspaceCompletions.ts';
import { createWorkspaceDrafts } from './workspaceDrafts.ts';
import { SourceSubmissionStorage, createSourceSubmission, bindSourceDigest, acceptSourceReceipt } from './sourceSubmission.ts';
import type { ChatTurn } from './chatTurn.ts';

function fixture() {
    const data = new Map<string, string>([['nola.web.chat.client."host"', 'client-1234']]);
    const storage = { get length() { return data.size; }, key: (index: number) => [...data.keys()][index] ?? null,
        getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => { data.set(key, value); },
        removeItem: (key: string) => { data.delete(key); } };
    const save = (conversation: string, host = 'host', client = 'client-1234', namespace = 'chat', admitted = true, sent = true) => {
        const identity = { source_host_id: host, client_id: client, conversation_id: conversation,
            submission_id: 'run', user_message_id: 'user', assistant_message_id: 'assistant' };
        const request = { conversationId: conversation, expected_host_id: host, client_id: client, submission_id: 'run',
            user_message_id: 'user', assistant_message_id: 'assistant', messages: [{ role: 'user', content: 'private test content' }] };
        const pending = bindSourceDigest(createSourceSubmission(identity, request), {
            source_host_id: host, result: { identity, request_digest: 'a'.repeat(64) },
        });
        const receipt = { protocol_version: 1, identity, request_digest: pending.request_digest, phase: 'admitted',
            admission: { identity: { source_store_epoch: 'epoch', conversation_id: conversation, run_id: 'run', assistant_message_id: 'assistant' },
                sequence: '1', record_digest: 'b'.repeat(64) } };
        new SourceSubmissionStorage(storage, namespace).save({ ...(admitted ? acceptSourceReceipt(pending, receipt) : pending), sent });
    };
    return { data, storage, save };
}

test('stored admission enumeration isolates namespace, host and client, and never deletes uncertain or corrupt data', () => {
    const f = fixture();
    f.save('valid'); f.save('foreign-host', 'other'); f.save('foreign-client', 'host', 'other-client');
    f.save('model-test', 'host', 'client-1234', 'modeltest');
    f.save('uncertain', 'host', 'client-1234', 'chat', false);
    f.save('unsent', 'host', 'client-1234', 'chat', true, false);
    f.data.set('nola.source-submission.chat.["host","client-1234","broken"]', '{broken');
    f.data.set('nola.source-submission.chat.not-json', 'invalid');
    const before = [...f.data];
    const restored = new SourceSubmissionStorage(f.storage).recoverable('host', 'client-1234');
    assert.deepEqual(restored.map(item => item.identity.conversation_id), ['valid', 'uncertain']);
    assert.deepEqual([...f.data], before);
});

test('reload restoration authenticates once, observes exact saved Run, and never dispatches', async t => {
    const f = fixture(); f.save('conversation');
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: f.storage });
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'localStorage'); Reflect.deleteProperty(globalThis, 'document'); });
    const requests: string[] = [];
    t.mock.method(globalThis, 'fetch', async (path: string, options: RequestInit) => {
        requests.push(path); assert.equal(options.method, undefined);
        return new Response(JSON.stringify({ authenticated: true, hostId: 'host' }));
    });
    const workspace = createWorkspaceDrafts(); const watched: ChatTurn[] = [];
    workspace.forHost('host').completions.watch = async turn => { watched.push(turn); };
    const before = [...f.data];
    await restoreBrowserWorkspaceCompletions(workspace, new AbortController().signal);
    assert.deepEqual(requests, ['/api/web/session']);
    assert.equal(watched.length, 1);
    assert.deepEqual(watched[0].owner, { hostId: 'host', clientId: 'client-1234', conversationId: 'conversation', generation: 0 });
    assert.equal(watched[0].runId, 'run');
    assert.equal(watched[0].identity.assistantMessageId, 'assistant');
    assert.deepEqual([...f.data], before);
});

test('late recovery cannot switch the active host or revive an unmounted workspace', async t => {
    const f = fixture(); f.save('conversation');
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: f.storage });
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'localStorage'); Reflect.deleteProperty(globalThis, 'document'); });
    let release!: (response: Response) => void;
    t.mock.method(globalThis, 'fetch', () => new Promise<Response>(resolve => { release = resolve; }));
    for (const abort of [false, true]) {
        const workspace = createWorkspaceDrafts(); const controller = new AbortController();
        const done = restoreBrowserWorkspaceCompletions(workspace, controller.signal);
        if (abort) controller.abort();
        else workspace.forHost('new-host').text.write('new-conversation', 'keep me');
        release(new Response(JSON.stringify({ authenticated: true, hostId: 'host' })));
        await done;
        assert.equal(workspace.authenticatedHostId, abort ? null : 'new-host');
        if (!abort) assert.equal(workspace.forHost('new-host').text.read('new-conversation'), 'keep me');
        workspace.clear();
    }
});
