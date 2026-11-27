import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { api, ApiError, csrfToken } from './api.ts';
import { locale } from './i18n.ts';
import {
    agentAccessSnapshot,
    agentAccessReceipt,
    agentAccessDraft,
    agentAccessDirty,
    agentAccessPatch,
    knownPermissionProfile,
    requiresAllowAllConfirmation,
    permissionProfiles,
} from './agentAccessSettings.ts';
import {
    credentialSnapshot,
    credentialRequest,
    credentialPath,
    isValidCredentialToken,
} from './endpointCredentials.ts';
import {
    StreamingMarkdownCache,
    highlightCode,
    markdownHref,
    parseMarkdown,
    renderMathExpression,
} from './markdown.ts';
import type { Locale } from './i18n.ts';

const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
after(async () => { await server.close(); });
const { default: Markdown } = await server.ssrLoadModule('/src/components/Markdown.svelte');
const { render } = await server.ssrLoadModule('svelte/server');
const { locale: ssrLocale } = await server.ssrLoadModule('/src/lib/i18n.ts');
const renderMarkdownHtml = (source: string, language: Locale = 'zh-Hans') => {
    ssrLocale.set(language);
    return render(Markdown, { props: { source } }).body.replace(/<!--.*?-->/gs, '');
};

test('CSRF extraction uses the exact cookie, never the session credential', () => {
    assert.equal(csrfToken('nola_web_session=secret; nola_web_csrf=csrf-value'), 'csrf-value');
    assert.equal(csrfToken('other_nola_web_csrf=wrong'), undefined);
});

test('API client enforces same-origin paths before reading credentials', async () => {
    await assert.rejects(api('https://example.invalid/api/models'), /Invalid API path/);
    await assert.rejects(api('/api/\\example.invalid'), /Invalid API path/);
});

test('API mutations send same-origin credentials and CSRF while reads omit CSRF', async (t) => {
    Object.defineProperty(globalThis, 'document', {
        configurable: true,
        value: { cookie: 'nola_web_session=private; nola_web_csrf=csrf' },
    });
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

test('API client propagates Accept-Language, Content-Type rules, and ApiError codes', async (t) => {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: { cookie: '' } });
    t.after(() => { Reflect.deleteProperty(globalThis, 'document'); locale.set('en'); });

    const seen: Array<{ lang: string | null; ct: string | null }> = [];
    let fail = false;
    t.mock.method(globalThis, 'fetch', async (_path: string, options: RequestInit) => {
        if (fail) {
            return new Response(JSON.stringify({ error: 'changed', errorCode: 'automation_revision_conflict' }), { status: 409 });
        }
        const headers = new Headers(options.headers);
        seen.push({ lang: headers.get('Accept-Language'), ct: headers.get('Content-Type') });
        return new Response('{}');
    });

    locale.set('ja');
    const form = new FormData();
    form.set('file', 'content');
    await api('/api/upload', { method: 'POST', body: form });
    locale.set('de');
    await api('/api/text', { method: 'POST', body: 'hello', headers: { 'Content-Type': 'text/plain' } });
    await api('/api/json', { method: 'POST', body: '{}' });

    assert.deepEqual(seen, [
        { lang: 'ja', ct: null },
        { lang: 'de', ct: 'text/plain' },
        { lang: 'de', ct: 'application/json' },
    ]);

    fail = true;
    await assert.rejects(api('/api/scheduled-actions/1/run-now', { method: 'POST', body: '{}' }), (err: unknown) => {
        assert.ok(err instanceof ApiError);
        assert.equal(err.status, 409);
        assert.equal(err.code, 'automation_revision_conflict');
        return true;
    });
});

test('XSS and link safety: dangerous HTML, links, code, and math are escaped or rejected', () => {
    for (const unsafeUrl of [
        'javascript:alert(1)',
        'data:text/html,evil',
        'file:///tmp/main.pdf',
        '/api/private',
        '//evil.test',
        'https://user:secret@example.com',
        'java&#10;script:alert(1)',
        'https://example.com/\npath',
    ]) {
        assert.equal(markdownHref(unsafeUrl), undefined, unsafeUrl);
    }
    assert.equal(markdownHref('https://example.com/?a=1&amp;b=2'), 'https://example.com/?a=1&b=2');

    const rendered = renderMarkdownHtml(
        '<script>alert(1)</script>\n\n[bad](javascript:alert%281%29) [encoded](javascript&#58;alert%281%29) '
        + '![unsafe](javascript:alert%281%29) ![private](/api/private/secret.png) '
        + '![track](https://example.com/pixel) ![local](/api/chat/conversations/c1/resources/r1)\n\n'
        + '<iframe src="https://example.com"></iframe>\n\n'
        + '| Col | Val |\n| --- | --- |\n| `k` | 1 |',
    );
    assert.ok(!/<script|<iframe|href="javascript:|src="javascript:|\/api\/private/i.test(rendered));
    assert.ok(rendered.includes('&lt;script>'));
    assert.ok(rendered.includes('src="/api/chat/conversations/c1/resources/r1"'));
    assert.ok(rendered.includes('rel="noopener noreferrer"'));
    assert.match(rendered, /<table\b/);

    const payload = '<img src=x onerror="alert(1)"> & \'<script>evil()</script>\'';
    for (const lang of ['rust', 'ts', 'python', 'json', 'bash', 'html', 'diff', '']) {
        const out = highlightCode(payload, lang);
        assert.ok(!out.includes('<img') && !out.includes('<script'), `unescaped HTML in ${lang}`);
    }

    const mathXss = renderMathExpression('\\frac{<script>alert(1)</script>}{<img src=x onerror=evil()>}^{"a&b"}');
    assert.ok(!mathXss.includes('<script>') && !mathXss.includes('<img'));
    assert.ok(mathXss.includes('&lt;script&gt;') && mathXss.includes('&amp;'));

    const streamingCache = new StreamingMarkdownCache();
    const sample = '# Title\n\nParagraph **bold** and $E = mc^2$.\n\n```ts\nconst x = 1;\n```';
    for (let i = 1; i <= sample.length; i += 7) {
        assert.deepEqual(streamingCache.update(sample.slice(0, i)).blocks, parseMarkdown(sample.slice(0, i)));
    }
    assert.deepEqual(streamingCache.update(sample).blocks, parseMarkdown(sample));
});

test('endpoint credentials validate bounded ASCII tokens, strip secrets, and stay prototype-safe', () => {
    const status = () => ({
        endpointID: 'provider/中文',
        baseURL: 'https://example.com/v1',
        wireProtocol: 'responses',
        expectedRevisions: { model: 3 },
        credentialRevision: 2,
        hasCredential: true,
    });

    for (const token of ['', '   ', 'a b', 'a\nb', 'a\0b', '密钥', 'a'.repeat(8193)]) {
        assert.equal(isValidCredentialToken(token), false);
        assert.throws(() => credentialRequest(status(), 'replace', token), /credential.invalidToken/);
    }
    assert.equal(isValidCredentialToken('sk-fixture_123.+/=:'), true);
    assert.equal(credentialRequest(status(), 'replace', '  key-fixture  ').secret, 'key-fixture');

    const source = { ...status(), secret: 'must-not-retain', unrelated: { secret: 'hidden' } };
    const snapshot = credentialSnapshot(source, source.endpointID);
    source.expectedRevisions.model = 99;
    assert.equal(snapshot.expectedRevisions.model, 3);
    assert.equal('secret' in snapshot, false);
    assert.equal(credentialPath(snapshot.endpointID), '/api/model-endpoints/provider%2F%E4%B8%AD%E6%96%87/credential');

    const cleared = credentialRequest(snapshot, 'clear');
    assert.equal('secret' in cleared, false);
    assert.throws(() => credentialRequest(snapshot, 'clear', 'new-key'));
    assert.throws(() => credentialSnapshot(status(), 'other'));

    const protoSource = { ...status(), expectedRevisions: JSON.parse('{"__proto__":4,"constructor":5}') };
    const protoPayload = credentialRequest(protoSource, 'clear');
    assert.deepEqual(Object.keys(protoPayload.expectedRevisions), ['__proto__', 'constructor']);
    assert.equal(protoPayload.expectedRevisions.__proto__, 4);
});

test('agent access settings enforce canonical fields, sparse patches, and allow_all confirmation', () => {
    const response = () => ({
        permission_profile: 'cautious',
        custom_system_prompt: ' original\n',
        enable_local_dev_tools: false,
        allowed_dirs: ['/tmp/work'],
        bash_prefixes: ['git status'],
        temperature: 0.3,
        skills: [{ name: 'private' }],
        mcp_servers: [],
    });

    const source = response();
    const snapshot = agentAccessSnapshot(source);
    source.allowed_dirs.push('/new');
    assert.deepEqual(snapshot.allowed_dirs, ['/tmp/work']);
    assert.equal('temperature' in snapshot, false);
    assert.equal('skills' in snapshot, false);

    const { bash_prefixes, ...legacy } = response();
    assert.throws(() => agentAccessSnapshot({ ...legacy, bash_allowed_prefixes: bash_prefixes }));
    assert.throws(() => agentAccessSnapshot({ ...legacy, allowed_bash_prefixes: bash_prefixes }));

    const draft = agentAccessDraft(snapshot);
    assert.equal(agentAccessDirty(snapshot, draft), false);
    assert.deepEqual(agentAccessPatch(snapshot, draft), {});

    draft.customSystemPrompt = ' new instructions\n';
    draft.allowedDirs = '/tmp/new\n\n /tmp/second ';
    draft.bashPrefixes = '';
    draft.enableLocalDevTools = true;
    const patch = agentAccessPatch(snapshot, draft);
    assert.deepEqual(patch, {
        custom_system_prompt: ' new instructions\n',
        allowed_dirs: ['/tmp/new', '/tmp/second'],
        bash_prefixes: [],
        enable_local_dev_tools: true,
    });

    const receipt = agentAccessReceipt({ ...snapshot, ...patch }, patch);
    assert.deepEqual(receipt, { ...snapshot, ...patch });
    assert.throws(() => agentAccessReceipt({ ...snapshot, ...patch, enable_local_dev_tools: false }, patch));

    for (const unknown of ['', 'moderate', 'ALLOW_ALL', 'future_mode']) {
        const unknownSnap = agentAccessSnapshot({ ...response(), permission_profile: unknown });
        assert.equal(knownPermissionProfile(unknown), false);
        assert.throws(() => agentAccessPatch(unknownSnap, agentAccessDraft(unknownSnap)), /agentAccess.unknownProfile/);
    }

    for (const profile of permissionProfiles) {
        const nextDraft = { ...agentAccessDraft(snapshot), permissionProfile: profile };
        assert.equal(requiresAllowAllConfirmation(snapshot, nextDraft), profile === 'allow_all');
    }
    const alreadyAllowed = { ...snapshot, permission_profile: 'allow_all' };
    assert.equal(requiresAllowAllConfirmation(alreadyAllowed, agentAccessDraft(alreadyAllowed)), false);

    for (const invalid of [null, {}, { ...response(), allowed_dirs: [1] }, { ...response(), enable_local_dev_tools: 'false' }]) {
        assert.throws(() => agentAccessSnapshot(invalid), /agentAccess.invalidSnapshot/);
    }
});
