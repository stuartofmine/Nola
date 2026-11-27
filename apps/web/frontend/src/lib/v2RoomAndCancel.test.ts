import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';
import { supportedLocales, t } from './i18n.ts';

function stubFetch(handler: (url: string, init?: RequestInit) => Promise<Response>) {
    const originalFetch = globalThis.fetch;
    const originalWindow = (globalThis as Record<string, unknown>).window;
    const originalDocument = (globalThis as Record<string, unknown>).document;
    (globalThis as Record<string, unknown>).window = { location: { origin: 'http://127.0.0.1:4096' } };
    (globalThis as Record<string, unknown>).document = { cookie: '' };
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
        return handler(url, init);
    }) as typeof fetch;
    return () => {
        globalThis.fetch = originalFetch;
        (globalThis as Record<string, unknown>).window = originalWindow;
        (globalThis as Record<string, unknown>).document = originalDocument;
    };
}

test('updateV2AgentRoom and attachV2ConversationToRoom use POST /api/agent-rooms update_room without 404 PUT or swallowed errors', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { attachV2ConversationToRoom, updateV2AgentRoom } = await server.ssrLoadModule('/src/lib/v2Chat.ts');

        const updateCalls: Array<{ method: string; url: string; body?: string }> = [];
        const restoreUpdate = stubFetch(async (url, init) => {
            const method = (init?.method ?? 'GET').toUpperCase();
            updateCalls.push({ method, url, body: typeof init?.body === 'string' ? init.body : undefined });
            if (method === 'POST' && url.endsWith('/api/agent-rooms')) {
                return new Response(JSON.stringify({ ok: true, rooms: [], experts: [] }), {
                    status: 200,
                    headers: { 'content-type': 'application/json' },
                });
            }
            return new Response(JSON.stringify({ host_id: 'local', rooms: [], experts: [], roleTemplates: [] }), {
                status: 200,
                headers: { 'content-type': 'application/json' },
            });
        });
        try {
            await updateV2AgentRoom('room-42', { name: 'Design Squad', participantExpertIds: ['exp-1'] });
            assert.equal(updateCalls.some(c => c.method === 'PUT'), false, 'must never issue PUT /api/agent-rooms/<id>');
            const postCall = updateCalls.find(c => c.method === 'POST' && c.url.endsWith('/api/agent-rooms'));
            assert.ok(postCall);
            const parsed = JSON.parse(postCall.body ?? '{}');
            assert.equal(parsed.command?.type, 'update_room');
            assert.equal(parsed.command?.input?.id, 'room-42');
            assert.equal(parsed.command?.input?.name, 'Design Squad');
            assert.deepEqual(parsed.command?.input?.participant_expert_ids, ['exp-1']);
        } finally {
            restoreUpdate();
        }

        const detachCalls: Array<{ method: string; url: string; body?: string }> = [];
        let failDetach = false;
        const restoreDetach = stubFetch(async (url, init) => {
            const method = (init?.method ?? 'GET').toUpperCase();
            detachCalls.push({ method, url, body: typeof init?.body === 'string' ? init.body : undefined });
            if (failDetach) {
                return new Response(JSON.stringify({ ok: false, error: 'detach failed' }), {
                    status: 500,
                    headers: { 'content-type': 'application/json' },
                });
            }
            return new Response(JSON.stringify({ ok: true, result: { ok: true, room: { id: 'room-7', name: 'Core', session_id: '' } } }), {
                status: 200,
                headers: { 'content-type': 'application/json' },
            });
        });
        try {
            const result = await attachV2ConversationToRoom('conv-1', null, undefined, 'room-7');
            assert.equal(result, null);
            assert.equal(detachCalls.length, 1);
            assert.equal(detachCalls[0].method, 'POST');
            assert.ok(detachCalls[0].url.endsWith('/api/agent-rooms'));
            const payload = JSON.parse(detachCalls[0].body ?? '{}');
            assert.deepEqual(payload, {
                command: {
                    type: 'update_room',
                    input: { id: 'room-7', detach: true, session_id: null },
                },
            });

            failDetach = true;
            await assert.rejects(
                () => attachV2ConversationToRoom('conv-1', null, undefined, 'room-7'),
                /detach failed/,
            );
        } finally {
            restoreDetach();
        }
    } finally {
        await server.close();
    }
});

test('cancel and openLogsFolder failures surface localized UI errors instead of being swallowed', () => {
    const chat = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
    const modelTest = readFileSync(new URL('../pages/ModelTest.svelte', import.meta.url), 'utf8');
    const desktopSettings = readFileSync(new URL('../components/DesktopSettings.svelte', import.meta.url), 'utf8');
    const hostDiagnostics = readFileSync(new URL('../components/HostDiagnostics.svelte', import.meta.url), 'utf8');

    const cancelFn = chat.slice(chat.indexOf('async function cancelActiveRun()'), chat.indexOf('async function refreshModelChoices()'));
    assert.match(cancelFn, /await cancelV2Turn\(targetId\)/);
    assert.match(cancelFn, /pushToast\(/);
    assert.doesNotMatch(cancelFn, /\.catch\(\(\) => \{\}\)/);

    const stopFn = modelTest.slice(modelTest.indexOf('async function stop()'), modelTest.indexOf('function clear()'));
    assert.match(stopFn, /await cancelV2Turn\(convId\)/);
    assert.match(stopFn, /pushToast\(/);
    assert.doesNotMatch(stopFn, /\.catch\(\(\) => \{\}\)/);

    assert.match(desktopSettings, /diagnostics\.openLogsFailed/);
    assert.match(desktopSettings, /pushToast\(/);
    assert.match(hostDiagnostics, /diagnostics\.openLogsFailed/);
    assert.match(hostDiagnostics, /pushToast\(/);

    for (const language of supportedLocales) {
        assert.notEqual(t(language, 'diagnostics.openLogsFailed'), 'diagnostics.openLogsFailed');
        assert.ok(t(language, 'diagnostics.openLogsFailed').trim());
    }
});
