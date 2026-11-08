// Self-owned static server and synthetic API fixtures. Never contacts port 5001.
const { chromium } = require('playwright-core');
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');

async function main() {
    const root = path.resolve(__dirname, '../app/static/ui');
    const output = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-inspector-ui.'));
    const server = http.createServer((request, response) => {
        const pathname = new URL(request.url, 'http://fixture').pathname;
        if (pathname.startsWith('/api/')) { response.writeHead(503); response.end(); return; }
        const file = pathname.startsWith('/ui/assets/') ? path.resolve(root, pathname.slice(4)) : path.join(root, 'index.html');
        if (!file.startsWith(root + '/') || !fs.existsSync(file)) { response.writeHead(404); response.end(); return; }
        response.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
        response.end(fs.readFileSync(file));
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const origin = `http://127.0.0.1:${server.address().port}`;
    const browser = await chromium.launchPersistentContext(fs.mkdtempSync(path.join(os.tmpdir(), 'nola-inspector-chrome.')), {
        executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true,
        viewport: { width: 1440, height: 1000 }, locale: 'en-US', serviceWorkers: 'block',
        args: ['--disable-background-networking', '--disable-component-update', '--disable-sync', '--disable-gpu'],
    });
    const report = { checks: [], errors: [], blocked: [] };
    const conversations = ['alpha', 'beta'].map(id => ({ conversationId: id, title: `Fixture ${id}`, isPinned: false, createdAt: '', updatedAt: '', selectedModelSource: 'auto', selectedModelId: '', executionBackend: 'nola', revision: 1, configurationRevision: 1 }));
    let delayedGoal, holdGoal = false;
    function goal(id) { return { conversationId: id, authoritative: true, goalProjection: { revision: 1, active: { objective: `Canonical ${id} objective`, status: 'running', requirements: { task: { id: 'task', description: `Canonical ${id} task`, status: 'pending' } } } } }; }
    try {
        await browser.route('**/*', async route => {
            const url = new URL(route.request().url());
            if (url.origin !== origin) { report.blocked.push(url.origin); return route.abort(); }
            if (!url.pathname.startsWith('/api/')) return route.continue();
            const reply = json => route.fulfill({ json });
            if (url.pathname === '/api/web/session') return reply({ authenticated: true, hostId: 'fixture-host' });
            if (url.pathname === '/api/chat/conversations') return reply({ conversations });
            if (url.pathname === '/api/chat/agent/v2/observe') return reply({ source_host_id: 'fixture-host', result: null });
            if (url.pathname === '/api/model-profiles') return reply({ profiles: [] });
            if (url.pathname === '/api/local-models') return reply({ models: [] });
            const match = url.pathname.match(/^\/api\/chat\/conversations\/(alpha|beta)\/([^/]+)$/);
            if (match?.[2] === 'configuration') return reply({ configurationRevision: 1, permissionProfile: null, memoryEnabledOverride: null, retrievalEnabledOverride: null, modelOverrides: {}, activeRunID: null, activePermissionPolicy: null });
            if (match?.[2] === 'context-usage') return reply({ conversation_id: match[1], usage: { conversation_cache_usage: null } });
            if (match?.[2] === 'messages') return reply({
                conversationProjection: { contract_version: 1, conversation_id: match[1], revision: 1,
                    messages: [{ message_id: `${match[1]}-reply`, role: 'assistant', state: 'completed', revision: 1, created_at_ms: 1,
                        parts: [{ kind: 'text', part_id: `${match[1]}-text`, revision: 1, markdown: `A preserved **canonical ${match[1]} answer**.`, is_final: true }] }],
                    surface: { revision: 1, committed_message_ids: [`${match[1]}-reply`], live_message_id: null } },
                messageWindow: { projectionRevision: 1, eventSequence: 1, before: null, hasMore: false, totalCount: 1 },
            });
            if (match?.[2] === 'goal') {
                if (holdGoal && match[1] === 'alpha') { delayedGoal = route; return; }
                return reply(goal(match[1]));
            }
            if (match?.[2] === 'traces') return reply({ conversationId: match[1], traces: [] });
            if (match?.[2] === 'plan') return reply({ conversationId: match[1], authoritative: true, plan: { conversation_id: match[1], revision: 2, todos: [{ id: 'todo', content: `Plan ${match[1]} task`, status: 'in_progress' }] } });
            // Nonessential settings/history APIs explicitly fail in this fixture;
            // no request is forwarded to a real Host, including observe POSTs.
            return route.fulfill({ status: 503, json: { code: 'isolated_fixture_unavailable' } });
        });
        await browser.routeWebSocket('**/*', socket => socket.close());
        const page = await browser.newPage();
        page.on('pageerror', error => report.errors.push(error.message));
        page.setDefaultTimeout(10000);
        await page.goto(origin + '/chat?conversation=alpha');
        const progress = page.getByRole('button', { name: 'Progress', exact: true });
        await progress.waitFor();
        assert.equal(await progress.getAttribute('aria-expanded'), 'false');
        const transcript = await page.locator('.transcript').elementHandle();
        await page.locator('[data-message-id="alpha-reply"]').waitFor();
        const answer = await page.locator('[data-message-id="alpha-reply"]').elementHandle();
        await page.locator('#chat-composer').fill('Unsent fixture draft');
        await progress.click();
        await page.locator('.checklist li').filter({ hasText: 'Canonical alpha task' }).waitFor();
        await page.getByRole('button', { name: 'Resources', exact: true }).click();
        assert.equal(await page.locator('#chat-details').isVisible(), true);
        await page.getByRole('button', { name: 'Conversation settings', exact: true }).click();
        assert.equal(await page.locator('#chat-composer').inputValue(), 'Unsent fixture draft');
        assert.equal(await transcript.evaluate(node => node === document.querySelector('.transcript')), true);
        assert.equal(await answer.evaluate(node => node === document.querySelector('[data-message-id="alpha-reply"]')), true);
        assert.equal(await page.getByText('Could not load the conversation. Try again.', { exact: true }).count(), 0);
        assert.equal(await page.locator('.session-settings [role=alert]').count(), 0);
        await progress.click();
        await page.screenshot({ path: path.join(output, 'progress-wide.png'), animations: 'disabled' });
        await progress.click();
        assert.equal(await page.locator('#chat-details').isVisible(), false);
        await page.setViewportSize({ width: 390, height: 844 });
        await progress.click();
        assert.equal(await page.locator('dialog:modal').count(), 1);
        await page.locator('dialog:modal').getByRole('button', { name: 'Resources', exact: true }).click();
        await page.locator('dialog:modal').getByRole('button', { name: 'Conversation settings', exact: true }).click();
        await page.locator('dialog:modal').getByRole('button', { name: 'Progress', exact: true }).click();
        assert.equal(await page.locator('dialog:modal').count(), 1);
        await page.screenshot({ path: path.join(output, 'progress-narrow.png'), animations: 'disabled' });
        await page.locator('dialog:modal').getByRole('button', { name: 'Progress', exact: true }).click();
        assert.equal(await page.locator('dialog:modal').count(), 0);
        await progress.click();
        await page.keyboard.press('Escape');
        assert.equal(await page.locator('dialog:modal').count(), 0);
        assert.equal(await progress.getAttribute('aria-expanded'), 'false');
        assert.equal(await progress.evaluate(node => node === document.activeElement), true);
        await page.setViewportSize({ width: 1440, height: 1000 });
        await progress.click();
        holdGoal = true;
        await page.locator('.activity-goal').getByRole('button', { name: 'Refresh', exact: true }).click();
        await page.waitForFunction(() => document.querySelector('.activity-goal [role=status]'));
        await page.locator('.conversation-list').getByRole('button', { name: /Fixture beta/ }).click();
        await page.waitForURL(url => url.searchParams.get('conversation') === 'beta');
        assert.equal(await progress.getAttribute('aria-expanded'), 'false');
        await delayedGoal.fulfill({ json: goal('alpha') }).catch(() => {});
        await progress.click();
        await page.locator('.checklist li').filter({ hasText: 'Canonical beta task' }).waitFor();
        assert.equal(await page.locator('.checklist li').filter({ hasText: 'Canonical alpha task' }).count(), 0);
        report.checks.push('Three toggles/default closed/same panel', 'Transcript node and draft preserved', 'Single narrow modal/Escape/focus restored', 'Conversation switch clears old snapshot and rejects late owner result');
        assert.deepEqual(report.errors, []);
        assert.deepEqual(report.blocked, []);
    } finally {
        fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify(report, null, 2));
        console.log(output);
        await browser.close();
        await new Promise(resolve => server.close(resolve));
    }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
