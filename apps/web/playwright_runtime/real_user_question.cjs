// Real Chrome and production Host/model. No intercepted requests or injected waits.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const assert = require('node:assert/strict');
const { execFileSync, spawn } = require('node:child_process');
(async () => {
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_QUESTION_HOST_STATE || require('./test_environment.cjs').artifactPath('nola-question-host-state.json')));
    assert.ok(state.root.includes('nola-a2ui-real-'));
    const out = process.env.NOLA_QUESTION_AUDIT_OUT || require('./test_environment.cjs').artifactPath('nola-real-question');
    fs.mkdirSync(out, { recursive: true });
    // Keep this owned test awake without changing system power preferences.
    const awake = spawn('/usr/bin/caffeinate', ['-i', '-w', String(process.pid)], { stdio: 'ignore' });
    const browser = await chromium.launch({ ...require('./test_environment.cjs').browserOptions(), headless: false });
    const page = await browser.newPage({ viewport: { width: 1050, height: 850 } });
    if (process.env.NOLA_QUESTION_SESSION_STATE) {
        const saved = JSON.parse(fs.readFileSync(process.env.NOLA_QUESTION_SESSION_STATE, 'utf8'));
        await page.addInitScript(entries => {
            if (!sessionStorage.getItem('nola.web.chat.client')) {
                for (const [key, value] of entries) sessionStorage.setItem(key, value);
            }
        }, saved);
    }
    page.setDefaultTimeout(20000);
    const report = { clicks: [], errors: [], receipts: [], visibility: [] };
    await page.exposeFunction('recordQuestionVisibility', value => report.visibility.push(value));
    await page.addInitScript(() => {
        document.addEventListener('visibilitychange', () => window.recordQuestionVisibility({state: document.visibilityState, at: Date.now()}));
    });
    page.on('pageerror', e => report.errors.push(e.message));
    const reads = [];
    page.on('response', async response => {
        const path = new URL(response.url()).pathname;
        if (!['/api/chat/agent/v2/observe', '/api/chat/agent/v2/journal'].includes(path) && !path.endsWith('/messages')) return;
        const request = response.request().postDataJSON();
        const value = await response.json().catch(() => null);
        reads.push({ path, operation: request?.operation, status: response.status(), value });
    });
    async function click(name, scope = page) { await scope.getByRole('button', { name, exact: true }).click(); report.clicks.push(name); }
    try {
        await page.goto(state.url + '/login');
        await page.getByLabel('Host 访问令牌', { exact: true }).fill(fs.readFileSync(state.root + '/home/.nola/web/mobile-access-token', 'utf8').trim());
        await click('连接');
        if (process.env.NOLA_QUESTION_CONVERSATION) {
            report.conversationId = process.env.NOLA_QUESTION_CONVERSATION;
            await page.goto(state.url + '/chat?conversation=' + encodeURIComponent(report.conversationId));
        } else {
        const created = page.waitForResponse(r => new URL(r.url()).pathname === '/api/chat/conversations' && r.request().method() === 'POST');
        await click('新建会话');
        report.conversationId = (await (await created).json()).conversationId;
        await page.locator('#chat-composer').fill('This is an isolated user-question integration task. Use request_user_input to ask me to choose exactly one display color, Blue or Amber. Do not guess my choice. Wait for my submitted answer, then reply exactly QUESTION_CHOICE=<selected color>. No files, network searches or other tasks are needed.');
        await click('发送');
        }
        const card = page.getByRole('article', { name: '需要你回答', exact: true });
        await card.waitFor({ timeout: 180000 });
        await page.screenshot({ path: out + '/question.png' });
        await page.reload();
        await card.waitFor({ timeout: 30000 });
        report.restoredQuestion = true;
        await card.getByRole('radio', { name: /Amber/i }).check();
        const answered = page.waitForResponse(r => new URL(r.url()).pathname.endsWith('/user-input-waits/respond') && r.request().method() === 'POST');
        await click('提交回答', card);
        const response = await answered;
        const receipt = await response.json();
        report.receipts.push({ status: response.status(), type: receipt.type, state: receipt.wait?.wait?.state?.type, wake_pending: receipt.wake_pending });
        assert.equal(response.status(), 200); assert.equal(receipt.type, 'resolved');
        await page.locator('.transcript').getByText('QUESTION_CHOICE=Amber', { exact: true }).waitFor({ timeout: 180000 });
        await page.bringToFront();
        report.visibilityAtAnswer = await page.evaluate(() => document.visibilityState);
        await page.getByRole('button', { name: '发送', exact: true }).waitFor({ timeout: 30000 });
        const readProof = () => JSON.parse(execFileSync('python3', ['-c', `import sqlite3,json,sys
c=sqlite3.connect('file:'+sys.argv[1]+'/state/nola-v5.db?mode=ro',uri=True)
r=c.execute("select r.id,r.status,json_extract(v.result_json,'$.value.kind') from runs r left join run_results v on v.run_id=r.id where r.conversation_id=? order by r.created_at_ms desc limit 1",(sys.argv[2],)).fetchone()
print(json.dumps({'run_id':r[0],'status':r[1],'result':r[2]} if r else {}))`, state.root, report.conversationId], { encoding: 'utf8' }));
        const proof = readProof();
        report.terminal = proof;
        assert.equal(proof.status, 'sealed'); assert.equal(proof.result, 'answered');
        await page.screenshot({ path: out + '/answer.png' });
        await page.locator('#chat-composer').fill('Reply exactly QUESTION_FOLLOWUP=842.');
        await click('发送');
        await page.locator('.transcript').getByText('QUESTION_FOLLOWUP=842', { exact: true }).waitFor({ timeout: 180000 });
        await page.bringToFront();
        await page.getByRole('button', { name: '发送', exact: true }).waitFor({ timeout: 30000 });
        report.followupTerminal = readProof();
        assert.notEqual(report.followupTerminal.run_id, proof.run_id);
        assert.equal(report.followupTerminal.status, 'sealed');
        assert.equal(report.followupTerminal.result, 'answered');
        report.followupAnswered = true;
        await page.screenshot({ path: out + '/followup.png' });
        assert.deepEqual(report.errors, []);
        report.passed = true;
    } catch (e) {
        report.error = e.message;
        await page.screenshot({ path: out + '/failure.png' }).catch(() => {});
        fs.writeFileSync(out + '/failure.txt', await page.locator('body').ariaSnapshot().catch(() => 'unavailable'));
        process.exitCode = 1;
    } finally {
        const saved = await page.evaluate(() => Object.entries(sessionStorage).filter(([key]) =>
            key === 'nola.web.chat.client' || key.startsWith('nola.source-submission.'))).catch(() => []);
        // Private test-owned browser recovery state; never printed or committed.
        fs.writeFileSync(out + '/session-state.json', JSON.stringify(saved), { mode: 0o600 });
        fs.writeFileSync(out + '/protocol-reads.json', JSON.stringify(reads));
        fs.writeFileSync(out + '/report.json', JSON.stringify(report, null, 2));
        await browser.close();
        awake.kill();
    }
})();
