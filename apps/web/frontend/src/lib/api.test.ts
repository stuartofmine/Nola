import { test } from 'node:test';
import assert from 'node:assert/strict';
import { api, csrfToken } from './api.ts';
import { locale } from './i18n.ts';

test('CSRF extraction uses the exact cookie, never the session credential', () => {
    assert.equal(csrfToken('nola_web_session=secret; nola_web_csrf=csrf-value'), 'csrf-value');
    assert.equal(csrfToken('other_nola_web_csrf=wrong'), undefined);
});
test('API client rejects external URLs before reading credentials', async () => {
    await assert.rejects(api('https://example.invalid/api/models'), /Invalid API path/);
    await assert.rejects(api('/api/\\example.invalid'), /Invalid API path/);
});

test('mutations use same-origin credentials and CSRF; read requests do not add CSRF', async (t) => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: 'nola_web_session=private; nola_web_csrf=csrf' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'document'); });
    const requests: RequestInit[] = [];
    t.mock.method(globalThis, 'fetch', async (_path: string, options: RequestInit) => {
        requests.push(options);
        return new Response(JSON.stringify({ ok: true }), { headers: { 'Content-Type': 'application/json' } });
    });
    await api('/api/agent/settings', { method: 'POST', body: '{}' });
    await api('/api/local-models');
    assert.equal(requests[0].credentials, 'same-origin');
    assert.equal(new Headers(requests[0].headers).get('X-Nola-CSRF'), 'csrf');
    assert.equal(new Headers(requests[0].headers).get('Content-Type'), 'application/json');
    assert.equal(new Headers(requests[1].headers).get('X-Nola-CSRF'), null);
});

test('API requests carry the selected UI language rather than only browser defaults', async (t) => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'document'); locale.set('en'); });
    const languages: (string | null)[] = [];
    t.mock.method(globalThis, 'fetch', async (_path: string, options: RequestInit) => {
        languages.push(new Headers(options.headers).get('Accept-Language'));
        return new Response('{}');
    });
    locale.set('ja'); await api('/api/model-profiles');
    locale.set('de'); await api('/api/model-profiles');
    assert.deepEqual(languages, ['ja', 'de']);
});

test('server validation failures are not mistaken for saved settings', async (t) => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'document'); });
    t.mock.method(globalThis, 'fetch', async () => new Response(JSON.stringify({ error: 'revision conflict', code: 'conflict' }), { status: 409 }));
    await assert.rejects(api('/api/local-models/fixture/configuration', { method: 'POST', body: '{}' }), /revision conflict/);
});
