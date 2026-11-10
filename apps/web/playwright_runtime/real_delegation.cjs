// Real UI/model delegation followed by read-only child navigation.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const { execFileSync } = require('node:child_process');
const assert = require('node:assert/strict');
(async () => {
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE));
    assert.ok(state.root.includes('nola-a2ui-real-'));
    const out = process.env.NOLA_DELEGATION_OUT || require('./test_environment.cjs').artifactPath('nola-real-delegation');
    fs.mkdirSync(out, {recursive: true});
    const browser = await chromium.launch({...require('./test_environment.cjs').browserOptions(), headless: false});
    const resume = process.env.NOLA_DELEGATION_RESUME;
    const page = await browser.newPage(resume ? {storageState: resume + '/browser-owner.json'} : {});
    page.setDefaultTimeout(20000);
    const report = {errors: [], responses: []};
    page.on('response', async response => {
        const path = new URL(response.url()).pathname;
        if (path.startsWith('/api/chat/conversations') || response.status() >= 400) {
            const body = await response.json().catch(() => null);
            report.responses.push({path, status: response.status(),
                ...(response.status() >= 400 ? {body} : {conversationId: body?.conversationId})});
        }
    });
    page.on('pageerror', error => report.errors.push(error.message));
    try {
        if (resume) {
            const previous = JSON.parse(fs.readFileSync(resume + '/report.json'));
            report.parent = previous.parent; report.childHref = previous.childHref; report.resumedFrom = resume;
            await page.goto(new URL(report.childHref, state.url).href);
        } else {
        await page.goto(state.url + '/login');
        await page.getByLabel('Host 访问令牌', {exact: true}).fill(fs.readFileSync(state.root + '/home/.nola/web/mobile-access-token', 'utf8').trim());
        await page.getByRole('button', {name: '连接', exact: true}).click();
        const created = page.waitForResponse(r => new URL(r.url()).pathname === '/api/chat/conversations' && r.request().method() === 'POST');
        await page.getByRole('button', {name: '新建会话', exact: true}).click();
        report.parent = (await (await created).json()).conversationId;
        await page.waitForURL(u => u.searchParams.get('conversation') === report.parent);
        await page.locator('#chat-composer').fill('Delegate exactly one task using spawn_agent with the explore profile: write a detailed tutorial explaining multiplication of 17 × 23 using five different methods, with worked steps and common mistakes for each method, then end with exactly CHILD_RESULT=<computed product>. Do not perform the child task yourself. No files or network searches are needed.');
        await page.getByRole('button', {name: '发送', exact: true}).click();
        const process = page.locator('details > summary').filter({hasText: /执行过程/}).first();
        await process.waitFor({timeout: 120000});
        if (await process.locator('..').getAttribute('open') === null) await process.click();
        const task = page.locator('.journal-detail > summary').filter({hasText: /后台任务/}).first();
        await task.waitFor({timeout: 120000});
        if (await task.locator('..').getAttribute('open') === null) await task.click();
        const child = page.getByRole('link', {name: '打开子会话', exact: true}).first();
        report.childHref = await child.getAttribute('href');
        await child.click();
        }
        const childID = new URL(report.childHref, state.url).searchParams.get('conversation');
        const detailPath = state.url + '/api/chat/conversations/' + encodeURIComponent(childID);
        const detail = await page.request.get(detailPath);
        assert.equal(detail.status(), 200, 'child detail must be readable independently of the ordinary conversation list');
        assert.equal((await detail.json()).conversationId, childID);
        assert.equal((await page.request.head(detailPath)).status(), 200);
        assert.equal((await page.request.get(state.url + '/api/chat/conversations/missing-' + childID)).status(), 404);
        const back = page.getByRole('link', {name: '返回父会话', exact: true});
        await back.waitFor();
        const readLiveChild = () => JSON.parse(execFileSync('python3', ['-c', `
import json,sqlite3,sys
c=sqlite3.connect('file:'+sys.argv[1]+'/state/nola-v5.db?mode=ro',uri=True)
rows=c.execute('SELECT b.run_id,r.status FROM runtime_task_run_bindings b JOIN runs r ON r.id=b.run_id WHERE b.owner_session_id=? AND b.child_session_id=?',(sys.argv[2],sys.argv[3])).fetchall()
assert len(rows)<=1
print(json.dumps({'run_id':rows[0][0],'status':rows[0][1]} if rows else None))
`, state.root, report.parent, new URL(report.childHref, state.url).searchParams.get('conversation')], {encoding:'utf8'}));
        const bindDeadline = Date.now() + 60000;
        do {
            report.liveChild = readLiveChild();
            if (report.liveChild) break;
            await page.waitForTimeout(250);
        } while (Date.now() < bindDeadline);
        assert.ok(report.liveChild, 'queued child must acquire its exact Run binding');
        assert.notEqual(report.liveChild.status, 'sealed', 'child finished before live observation; this does not prove active read-only access');
        assert.equal(await page.locator('#chat-composer').count(), 0);
        await page.waitForFunction(() => [...document.querySelectorAll('article:not(.user) .message-content')].some(e => e.textContent.includes('CHILD_RESULT=391')), undefined, {timeout: 120000});
        report.assistantResult = await page.locator('article:not(.user) .message-content').allTextContents();
        await page.reload();
        await back.waitFor();
        await page.waitForFunction(() => [...document.querySelectorAll('article:not(.user) .message-content')].some(e => e.textContent.includes('CHILD_RESULT=391')));
        assert.equal(await page.locator('#chat-composer').count(), 0);
        await page.screenshot({path: out + '/child.png'});
        await back.click();
        await page.waitForURL(u => u.searchParams.get('conversation') === report.parent);
        await page.locator('#chat-composer').waitFor();
        const readExecution = () => JSON.parse(execFileSync('python3', ['-c', `
import json, sqlite3, sys
c=sqlite3.connect('file:'+sys.argv[1]+'/state/nola-v5.db?mode=ro',uri=True)
rows=c.execute("SELECT b.task_id,b.run_id,r.status,a.phase FROM runtime_task_run_bindings b JOIN runs r ON r.id=b.run_id AND r.conversation_id=b.child_session_id JOIN runtime_agent_task_authorities a ON a.task_id=b.task_id WHERE b.owner_session_id=? AND b.child_session_id=?",(sys.argv[2],sys.argv[3])).fetchall()
assert len(rows)==1, 'expected one exact child Run binding'
task,run,status,phase=rows[0]
receipts=c.execute('SELECT count(*) FROM runtime_task_terminal_receipts WHERE task_id=?',(task,)).fetchone()[0]
print(json.dumps({'task_id':task,'run_id':run,'status':status,'admission':phase,'terminal_receipts':receipts}))
`, state.root, report.parent, childID], {encoding: 'utf8'}));
        const sealDeadline = Date.now() + 30000;
        do {
            report.execution = readExecution();
            if (report.execution.status === 'sealed' && report.execution.terminal_receipts === 1) break;
            await page.waitForTimeout(250);
        } while (Date.now() < sealDeadline);
        assert.equal(report.execution.status, 'sealed');
        assert.equal(report.execution.admission, 'admitted');
        assert.equal(report.execution.terminal_receipts, 1);
        const stopParent = page.getByRole('button', {name: '停止', exact: true});
        if (await stopParent.isVisible()) {
            await stopParent.click();
            report.parentStopClicked = true;
        }
        const readParentRuns = () => JSON.parse(execFileSync('python3', ['-c', `
import json,sqlite3,sys
c=sqlite3.connect('file:'+sys.argv[1]+'/state/nola-v5.db?mode=ro',uri=True)
print(json.dumps(c.execute('SELECT id,status FROM runs WHERE conversation_id=?',(sys.argv[2],)).fetchall()))
`, state.root, report.parent], {encoding: 'utf8'}));
        const parentDeadline = Date.now() + 30000;
        do {
            report.parentRuns = readParentRuns();
            if (report.parentRuns.length && report.parentRuns.every(run => run[1] === 'sealed')) break;
            await page.waitForTimeout(250);
        } while (Date.now() < parentDeadline);
        assert.ok(report.parentRuns.length && report.parentRuns.every(run => run[1] === 'sealed'), 'parent must reach a durable terminal after returning from the child');
        assert.deepEqual(report.errors, []);
        report.passed = true;
    } catch (error) {
        report.failure = String(error.stack || error);
        await page.screenshot({path: out + '/failure.png'}).catch(() => {});
        fs.writeFileSync(out + '/failure.txt', await page.locator('body').ariaSnapshot().catch(() => ''));
        process.exitCode = 1;
    } finally {
        // Keep the actual browser owner for recovery after a failed UI assertion.
        // This local credential-bearing artifact is never part of the report/repo.
        const storagePath = out + '/browser-owner.json';
        fs.writeFileSync(storagePath, JSON.stringify(await page.context().storageState()), {mode: 0o600});
        fs.writeFileSync(out + '/report.json', JSON.stringify(report, null, 2));
        await browser.close();
    }
})();
