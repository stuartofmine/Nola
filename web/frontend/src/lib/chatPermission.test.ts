import test from 'node:test';
import assert from 'node:assert/strict';
import { ChatPermissionSubmission, parseChatPermission, permissionDecision, permissionExpired, permissionIdentity } from './chatPermission.ts';
import { chatPermissionMessages } from './locales/chatPermissions.ts';
import { createServer } from 'vite';

const raw = { protocol_version: 1, allow_conversation: true, nonce: 'nonce', request_id: 'request', host_id: 'host', client_id: 'client', session_id: 'session', tool_name: 'any_tool', expires_at: 200 };
const challenge = () => parseChatPermission(raw)!;
const mock = (handler: (body: string) => Promise<Response>) => ((_: unknown, options: RequestInit) => handler(String(options.body))) as typeof fetch;
const receipt = (nonce = raw.nonce) => new Response(JSON.stringify({ accepted: true, nonce }), { status: 200 });

test('parses exact v4 identity and rejects unsupported/malformed challenges', () => {
    for (const field of ['nonce', 'request_id', 'host_id', 'client_id', 'session_id', 'tool_name']) {
        assert.equal(parseChatPermission({ ...raw, [field]: '' }), null);
        assert.notEqual(permissionIdentity(challenge()), permissionIdentity(parseChatPermission({ ...raw, [field]: 'other' })!));
    }
    for (const update of [{ protocol_version: 2 }, { protocol_version: 3 }, { allow_conversation: undefined }, { allow_conversation: 'true' }, { expires_at: '200' }, { expires_at: Infinity }, { permission_instructions: [{}] }]) assert.equal(parseChatPermission({ ...raw, ...update }), null);
    assert.equal(permissionExpired(challenge(), 199999), false);
    assert.equal(permissionExpired(challenge(), 200000), true);
    assert.throws(() => permissionDecision(challenge(), 'allow_once', '', 200000));
});

test('wire response preserves every identity field without client-generated grants', () => {
    const body = permissionDecision(challenge(), 'allow_conversation', '', 1000);
    assert.deepEqual(body, { protocol_version: 1, nonce: 'nonce', request_id: 'request', host_id: 'host', client_id: 'client', session_id: 'session', tool_name: 'any_tool', resolution: { type: 'authorization', decision: 'allow_conversation' } });
    assert.deepEqual(permissionDecision(challenge(), 'return_to_composer', '', 1000).resolution, { type: 'intervention', intervention: { type: 'return_to_composer' } });
    assert.throws(() => permissionDecision(challenge(), 'invented' as never, '', 1000));
});

test('guidance uses UTF8 budgets and is never authorization', () => {
    assert.throws(() => permissionDecision(challenge(), 'provide_instruction', '字'.repeat(1366), 1000));
    assert.throws(() => permissionDecision(challenge(), 'provide_instruction', '  ', 1000));
    const full = challenge(); full.permission_instructions = Array.from({ length: 16 }, () => ({ permission_request_id: 'r', instruction: 'x' }));
    assert.throws(() => permissionDecision(full, 'provide_instruction', 'next', 1000));
    assert.deepEqual(permissionDecision(challenge(), 'provide_instruction', ' note ', 1000).resolution, { type: 'intervention', intervention: { type: 'provide_instruction', instruction: 'note' } });
});

test('duplicate clicks deliver once and acknowledgement is transport accepted only', async () => {
    const gate = new ChatPermissionSubmission(challenge());
    let calls = 0; let finish!: (value: Response) => void;
    const fetcher = mock(() => { calls++; return new Promise(resolve => { finish = resolve; }); });
    const first = gate.submit('allow_once', '', fetcher, 1000);
    await gate.submit('deny', '', fetcher, 1000);
    assert.equal(gate.state, 'submitting'); assert.equal(calls, 1);
    finish(receipt()); await first;
    assert.equal(gate.state, 'accepted');
    await gate.submit('deny', '', fetcher, 1000); assert.equal(calls, 1);
});

test('default permission submission stays same-origin and carries CSRF', async () => {
    const originalFetch = globalThis.fetch;
    const originalDocument = globalThis.document;
    let called = false;
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: 'nola_web_csrf=test-token; unrelated=value' } });
    globalThis.fetch = (async (input, options) => {
        called = true;
        assert.equal(input, '/api/agent/host-permission/decision');
        assert.equal(options?.credentials, 'same-origin');
        assert.equal(options?.method, 'POST');
        const headers = new Headers(options?.headers);
        assert.equal(headers.get('X-Nola-CSRF'), 'test-token');
        assert.ok(headers.get('Accept-Language'));
        return receipt();
    }) as typeof fetch;
    try {
        const gate = new ChatPermissionSubmission(challenge());
        await gate.submit('deny', '', undefined, 1000);
        assert.equal(gate.state, 'accepted');
        assert.ok(called);
    } finally {
        globalThis.fetch = originalFetch;
        if (originalDocument === undefined) delete (globalThis as { document?: Document }).document;
        else Object.defineProperty(globalThis, 'document', { configurable: true, value: originalDocument });
    }
});

test('unknown network result freezes exact decision and retries only by explicit action', async () => {
    const gate = new ChatPermissionSubmission(challenge()); const bodies: string[] = [];
    await gate.submit('allow_once', '', mock(async body => { bodies.push(body); throw new Error('secret upstream'); }), 1000);
    assert.equal(gate.state, 'unknown');
    await gate.submit('deny', '', mock(async () => { throw new Error('must not send'); }), 1000);
    await gate.retry(mock(async body => { bodies.push(body); return receipt(); }), 1000);
    assert.equal(gate.state, 'accepted'); assert.equal(bodies.length, 2); assert.equal(bodies[0], bodies[1]);
});

test('invalid receipts, expiry and stale components never manufacture completion', async () => {
    for (const response of [receipt('wrong'), new Response('{}'), new Response('secret', { status: 500 })]) {
        const gate = new ChatPermissionSubmission(challenge());
        await gate.submit('deny', '', mock(async () => response), 1000);
        assert.equal(gate.state, 'unknown');
        let sent = false; await gate.retry(mock(async () => { sent = true; return receipt(); }), 200000); assert.equal(sent, false);
    }
    const disposed = new ChatPermissionSubmission(challenge());
    let finish!: (value: Response) => void;
    const pending = disposed.submit('deny', '', mock(() => new Promise(resolve => { finish = resolve; })), 1000);
    disposed.dispose(); finish(receipt()); await pending;
    assert.notEqual(disposed.state, 'accepted');
    const rejected = new ChatPermissionSubmission(challenge());
    await rejected.submit('deny', '', mock(async () => new Response('sensitive body', { status: 409 })), 1000);
    assert.equal(rejected.state, 'rejected');
});

test('seven permission catalogs cover all visible submission states without missing values', () => {
    assert.equal(Object.keys(chatPermissionMessages).length, 7);
    for (const catalog of Object.values(chatPermissionMessages)) {
        assert.equal(Object.keys(catalog).length, 21);
        for (const key of ['ready', 'submitting', 'accepted', 'unknown', 'rejected', 'expired', 'invalid']) assert.ok(catalog[`chatPermission.${key}`]);
        for (const value of Object.values(catalog)) assert.equal(typeof value, 'string');
    }
});

test('actual permission component never renders raw arguments or nonce and disables unsafe approvals', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: Component } = await server.ssrLoadModule('/src/components/ChatPermissionPanel.svelte');
        const { render } = await server.ssrLoadModule('svelte/server');
        const input = { ...raw, expires_at: Math.floor(Date.now() / 1000) + 300, nonce: 'secret-nonce', args: { password: 'secret-password' }, reason: 'secret-reason' };
        const html = render(Component, { props: { challenge: input } }).body;
        for (const secret of ['secret-nonce', 'secret-password', 'secret-reason']) assert.ok(!html.includes(secret));
        assert.match(html, /<button\b[^>]*\bdisabled(?:="")?[^>]*>Allow once<\/button>/);
        const inactive = render(Component, { props: { challenge: input, active: false, reviewText: '<script>not HTML</script>' } }).body;
        assert.ok(inactive.includes('&lt;script'));
        assert.ok(!inactive.includes('<script>not HTML</script>'));
        for (const button of inactive.matchAll(/<button\b([^>]*)>/g)) assert.ok(button[1].includes('disabled'));
        const onceOnly = { ...input, allow_conversation: false };
        const onceHtml = render(Component, { props: { challenge: onceOnly, reviewText: 'Review this invocation' } }).body;
        assert.ok(!onceHtml.includes(chatPermissionMessages.en['chatPermission.allowConversation']));
        assert.ok(onceHtml.includes(chatPermissionMessages.en['chatPermission.allowOnce']));
        const parsedOnce = parseChatPermission(onceOnly)!;
        assert.throws(() => permissionDecision(parsedOnce, 'allow_conversation'));
        assert.equal(permissionDecision(parsedOnce, 'allow_once').resolution.decision, 'allow_once');
    } finally { await server.close(); }
});
