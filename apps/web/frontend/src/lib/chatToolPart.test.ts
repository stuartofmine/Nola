import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';

test('v2 tool cards, permission summary formatter, and text reader keep content escaped in every locale', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: TextReader } = await server.ssrLoadModule('/src/components/ChatTextReaderDialog.svelte');
        const { formatPermissionSummary } = await server.ssrLoadModule('/src/lib/v2Chat.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');

        const unsafeText = '<script>unsafe()</script><img src=x onerror=alert(1)>';
        for (const language of supportedLocales) {
            locale.set(language);
            const html: string = render(TextReader, {
                props: {
                    snapshot: {
                        title: '独立标题 <script>x</script>',
                        subtitle: 'text/plain',
                        text: unsafeText,
                    },
                    onclose() {},
                },
            }).body;
            assert.ok(html.includes('独立标题'));
            assert.ok(html.includes('text/plain'));
            assert.ok(html.includes(t(language, 'chatPage.copyText')));
            assert.ok(html.includes(t(language, 'chatPage.closeReader')));
            assert.ok(html.includes(t(language, 'chatPage.readerSnapshot')));
            assert.ok(!html.includes('<script>unsafe()'), 'Reader body must stay HTML-escaped');
            assert.ok(!html.includes('<img src=x'), 'Reader body must escape inline HTML');
            assert.ok(!html.includes('<dialog open'), 'Reader dialog opens via showModal, never open attribute');
        }

        const chatSource = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
        assert.match(chatSource, /class="tool-card" data-status=\{tc\.status\}/);
        assert.match(chatSource, /chatPage\.toolRunning/);
        assert.match(chatSource, /chatPage\.toolFailed/);
        assert.match(chatSource, /chatPage\.toolCompleted/);
        assert.match(chatSource, /<pre class="tool-summary">\{tc\.summary\}<\/pre>/);
        const toolBlock = chatSource.slice(
            chatSource.indexOf('<div class="tool-calls">'),
            chatSource.indexOf('{#if msg.uiSurfaces?.length}'),
        );
        assert.ok(!toolBlock.includes('<button'), 'Tool status cards must not render inline permission grant buttons');

        const shellSummary = formatPermissionSummary({
            requestId: 'req-1',
            tool: 'shell_exec',
            arguments: { command: 'ls -la /tmp' },
            reason: 'Inspect temp directory',
        });
        assert.equal(shellSummary.target, 'ls -la /tmp');
        const fileSummary = formatPermissionSummary({
            requestId: 'req-2',
            tool: 'write_file',
            arguments: { path: '/tmp/report.txt' },
            reason: 'Write report',
        });
        assert.equal(fileSummary.target, '/tmp/report.txt');
    } finally {
        await server.close();
    }
});
