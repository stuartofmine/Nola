// Real browser clicks against an isolated production Host. Optional explicit
// lost-ack fault forwards the real mutation, then drops its HTTP response.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const assert = require('node:assert/strict');
const { execFileSync, spawn } = require('node:child_process');

(async () => {
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE));
    assert.ok(state.root.includes('nola-a2ui-real-'));
    const out = process.env.NOLA_GOAL_AUDIT_OUT || require('./test_environment.cjs').artifactPath('nola-real-goal-controls');
    fs.mkdirSync(out, { recursive: true });
    const browser = await chromium.launch({ ...require('./test_environment.cjs').browserOptions(), headless: false });
    const page = await browser.newPage();
    page.setDefaultTimeout(Number(process.env.NOLA_GOAL_BROWSER_TIMEOUT_MS || 20000));
    const report = { clicks: [], errors: [], receipts: [], network: [] };
    // Scoped to this test process; does not change system power settings.
    const awake = spawn('/usr/bin/caffeinate', ['-i', '-w', String(process.pid)], {stdio: 'ignore'});
    awake.on('error', error => { report.powerAssertionError = error.message; });
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    const transport = new Map();
    cdp.on('Network.requestWillBeSent', event => transport.set(event.requestId, { path: new URL(event.request.url).pathname }));
    cdp.on('Network.requestWillBeSentExtraInfo', event => { const item = transport.get(event.requestId); if (item) item.headersSent = true; });
    cdp.on('Network.responseReceived', event => { const item = transport.get(event.requestId); if (item) Object.assign(item, { status: event.response.status, protocol: event.response.protocol, timing: event.response.timing }); });
    cdp.on('Network.loadingFinished', event => { const item = transport.get(event.requestId); if (item) item.finishedBytes = event.encodedDataLength; });
    cdp.on('Network.loadingFailed', event => { const item = transport.get(event.requestId); if (item) item.failed = event.errorText; });
    const pendingRequests = new Map();
    page.on('request', request => pendingRequests.set(request, {path: new URL(request.url()).pathname, type: request.resourceType(), startedAt: Date.now()}));
    page.on('requestfinished', request => pendingRequests.delete(request));
    page.on('requestfailed', request => { report.network.push({path:new URL(request.url()).pathname,error:request.failure()?.errorText}); pendingRequests.delete(request); });
    page.on('pageerror', error => report.errors.push(error.message));
    const click = async (name, scope = page) => {
        await scope.getByRole('button', { name, exact: true }).click();
        report.clicks.push(name);
    };
    try {
        await page.goto(state.url + '/login');
        await page.getByLabel('Host 访问令牌', { exact: true }).fill(fs.readFileSync(state.root + '/home/.nola/web/mobile-access-token', 'utf8').trim());
        await click('连接');
        await page.getByRole('button', { name: '新建会话', exact: true }).waitFor();
        let identity;
        if (process.env.NOLA_GOAL_EXISTING_CONVERSATION) {
            identity = {conversationId: process.env.NOLA_GOAL_EXISTING_CONVERSATION};
            await page.goto(state.url + '/chat?conversation=' + encodeURIComponent(identity.conversationId));
            await page.waitForURL(u => u.searchParams.get('conversation') === identity.conversationId);
            report.restored = true;
        } else {
            const created = page.waitForResponse(r => new URL(r.url()).pathname === '/api/chat/conversations' && r.request().method() === 'POST');
            await click('新建会话');
            identity = await (await created).json();
            await page.waitForURL(u => u.searchParams.get('conversation') === identity.conversationId);
        }
        report.conversationId = identity.conversationId;
        // An explicit UI selection is a conversation preference; a model used
        // by a previous per-turn remote intent is not silently made permanent.
        if (process.env.NOLA_GOAL_MODEL_LABEL) {
            await page.locator('details.model-menu > summary').click();
            const escapedLabel = process.env.NOLA_GOAL_MODEL_LABEL.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
            const choice = page.locator('details.model-menu .choices > button')
                .filter({has: page.locator('span').filter({hasText: new RegExp('^(?:✓ )?' + escapedLabel + '$')})});
            if (await choice.getAttribute('aria-pressed') === 'true') {
                await page.locator('details.model-menu > summary').click();
                report.conversationModelAlreadySelected = true;
            } else {
                const saved = page.waitForResponse(r => new URL(r.url()).pathname === '/api/chat/conversations/' + encodeURIComponent(identity.conversationId)
                    && r.request().method() === 'PATCH');
                await choice.click();
                const response = await saved;
                assert.equal(response.ok(), true, await response.text());
            }
            report.selectedConversationModel = process.env.NOLA_GOAL_MODEL_LABEL;
        }
        await click('进度');
        const panel = page.getByRole('region', { name: '活动与目标', exact: true });
        const startNewGoal = !report.restored || process.env.NOLA_GOAL_RESTART_EXISTING === '1';
        if (startNewGoal) await panel.getByLabel('目标内容', { exact: true }).fill(process.env.NOLA_GOAL_OBJECTIVE || 'Isolated UI goal control audit: wait for my next instruction before performing any work.');
        const executionEvidence = () => JSON.parse(execFileSync('python3', ['-c', `import json,sqlite3,sys
c=sqlite3.connect('file:'+sys.argv[1]+'/state/nola-v5.db?mode=ro',uri=True)
r=c.execute('select id,status from runs where conversation_id=? order by created_at_ms desc limit 1',(sys.argv[2],)).fetchone()
print(json.dumps({'run_id':r[0] if r else None,'status':r[1] if r else None,'model_deltas':c.execute("select count(*) from host_event_inbox where run_id=? and payload_json like '%model_delta%'",(r[0],)).fetchone()[0] if r else 0,'model_completions':c.execute("select count(*) from host_event_inbox where run_id=? and payload_json like '%model_completed%'",(r[0],)).fetchone()[0] if r else 0}))`, state.root, identity.conversationId], {encoding:'utf8'}));
        let lastRun;
        const beforeControl = executionEvidence();
        const waitModel = async () => {
            if (process.env.NOLA_GOAL_REQUIRE_MODEL !== '1') return;
            const deadline = Date.now() + 120_000;
            while (Date.now() < deadline) {
                const evidence = executionEvidence();
                const freshOutput = evidence.run_id !== beforeControl.run_id
                    ? evidence.model_deltas > 0 || evidence.model_completions > 0
                    : evidence.model_deltas > beforeControl.model_deltas || evidence.model_completions > beforeControl.model_completions;
                if (evidence.run_id && evidence.run_id !== lastRun && freshOutput && evidence.status !== 'sealed') {
                    lastRun = evidence.run_id; report.receipts.push({realModel: evidence}); return;
                }
                await page.waitForTimeout(250);
            }
            report.lastExecution = executionEvidence();
            throw new Error('No new active Run with actual provider output within 120 seconds');
        };
        const waitStopped = async () => {
            if (process.env.NOLA_GOAL_REQUIRE_MODEL !== '1') return;
            const deadline = Date.now() + 60_000;
            while (Date.now() < deadline) {
                const evidence = executionEvidence();
                if (evidence.run_id === lastRun && evidence.status === 'sealed') { report.receipts.push({stopped: evidence}); return; }
                await page.waitForTimeout(250);
            }
            throw new Error('Goal control committed but exact Run did not stop');
        };
        const control = async (name, expected, scope = panel) => {
            const controlStartedAt = Date.now();
            const [response] = await Promise.all([
                page.waitForResponse(r => new URL(r.url()).pathname.endsWith('/goal') && r.request().method() === 'POST'),
                click(name, scope),
            ]);
            const submitted = response.request().postDataJSON();
            if (submitted.action !== 'start') {
                assert.equal(typeof submitted.expectedControlToken, 'string');
                assert.match(submitted.expectedControlToken, /^[a-f0-9]{64}$/);
                assert.equal(Object.hasOwn(submitted, 'expectedRevision'), false);
            }
            const body = await response.json();
            if (body.goalProjection?.active) {
                assert.match(body.goalProjection.control_token, /^[a-f0-9]{64}$/);
            }
            const status = body.goalProjection?.active?.status;
            report.receipts.push({ action: name, httpStatus: response.status(), elapsedMs: Date.now() - controlStartedAt, status, error: body.error,
                expectedControlToken: submitted.expectedControlToken, observedControlToken: body.goalProjection?.control_token,
                projectionRevision: body.goalProjection?.revision });
            assert.ok([200, 202].includes(response.status()), body.error || `HTTP ${response.status()}`);
            if (response.status() === 202) {
                assert.equal(body.controlPending, true);
                assert.equal(body.authoritative, false);
                assert.equal(typeof body.operationId, 'string');
                const deadline = Date.now() + 60_000;
                let settled = false;
                while (Date.now() < deadline) {
                    const read = await page.request.get(state.url + '/api/chat/conversations/' + encodeURIComponent(identity.conversationId) + '/goal');
                    assert.ok(read.ok(), `Goal read HTTP ${read.status()}`);
                    const snapshot = await read.json();
                    assert.equal(snapshot.authoritative, true);
                    const operation = snapshot.goalProjection?.applied_operations?.[body.operationId];
                    if (operation) {
                        assert.equal(operation.rejection, null, operation.rejection);
                        assert.equal(snapshot.goalProjection.active.status, expected);
                        report.receipts.push({operationId: body.operationId, status: expected, committed: true});
                        settled = true;
                        break;
                    }
                    await page.waitForTimeout(250);
                }
                assert.ok(settled, 'Goal control was queued but never committed within 60 seconds');
            } else {
                assert.equal(body.authoritative, true);
                assert.equal(status, expected);
            }
        };
        const pauseWithLostAcknowledgement = async () => {
            const path = '/api/chat/conversations/' + encodeURIComponent(identity.conversationId) + '/goal';
            let submitted;
            let delivered;
            let resolveProcessed;
            const processed = new Promise(resolve => { resolveProcessed = resolve; });
            const matcher = url => new URL(url).pathname === path;
            await page.route(matcher, async route => {
                if (route.request().method() === 'POST') {
                    assert.equal(submitted, undefined, 'UI must not silently send a second control');
                    submitted = route.request().postDataJSON();
                    const actual = await route.fetch(); // Production Host really executes.
                    delivered = { status: actual.status(), body: await actual.json() };
                    await route.abort('connectionreset');
                    resolveProcessed();
                } else {
                    // Keep the confirmation temporarily unavailable so reload
                    // must recover local intent rather than a lucky GET.
                    await route.abort('internetdisconnected');
                }
            });
            try {
                await click('暂停', panel);
                await Promise.race([processed, new Promise((_, reject) => setTimeout(() => reject(new Error('Real Host did not process pause')), 30000))]);
                assert.ok([200, 202].includes(delivered.status), JSON.stringify(delivered.body));
                assert.equal(typeof submitted.operationId, 'string');
                const saved = () => page.evaluate(({ operationId }) => Object.keys(localStorage)
                    .filter(key => key.startsWith('nola.goal-intent:'))
                    .map(key => JSON.parse(localStorage.getItem(key)))
                    .find(value => value.request.operationId === operationId), submitted);
                assert.deepEqual((await saved()).request.operationId, submitted.operationId);
                await page.reload();
                await click('进度');
                await panel.getByRole('button', { name: '重试原操作', exact: true }).waitFor();
                assert.equal((await saved()).request.operationId, submitted.operationId);
                report.lostAcknowledgement = { operationId: submitted.operationId,
                    realHostStatus: delivered.status, survivedReload: true };
            } finally {
                await page.unroute(matcher);
            }
            // Keep receipt reads unavailable until the user explicitly retries;
            // verify the click replays the original payload against the real Host.
            await page.route(matcher, async route => {
                if (route.request().method() === 'GET') await route.abort('internetdisconnected');
                else await route.continue();
            });
            try {
                const [retryResponse] = await Promise.all([
                    page.waitForResponse(response => new URL(response.url()).pathname === path && response.request().method() === 'POST'),
                    click('重试原操作', panel),
                ]);
                assert.deepEqual(retryResponse.request().postDataJSON(), submitted);
                assert.ok([200, 202].includes(retryResponse.status()));
                report.lostAcknowledgement.retriedOriginalPayload = true;
            } finally {
                await page.unroute(matcher);
            }
            await click('刷新', panel);
            await panel.locator('p.muted').filter({hasText:/^已暂停/}).waitFor();
            await panel.getByRole('button', { name: '重试原操作', exact: true }).waitFor({state:'hidden'});
            const response = await page.request.get(state.url + path);
            const canonical = await response.json();
            assert.equal(canonical.goalProjection.applied_operations[submitted.operationId].rejection, null);
            assert.equal(canonical.goalProjection.active.status, 'paused');
            report.lostAcknowledgement.canonicalConfirmed = true;
        };
        if (process.env.NOLA_GOAL_CANCEL_ONLY === '1') {
            assert.equal(report.restored, true, 'Cancel-only must target an explicit existing conversation');
            await click('取消目标', panel);
            await control('取消目标', 'cancelled', page.getByRole('dialog', { name: '确认取消长期目标？', exact: true }));
            await panel.locator('p.muted').filter({hasText:/^已取消/}).waitFor();
            await page.screenshot({path:out + '/cancelled.png'});
            assert.deepEqual(report.errors, []);
            report.passed = true;
            return;
        }
        if (startNewGoal) await control('开始目标', 'running');
        if (report.restored) await panel.locator('.actions button').first().waitFor();
        if (report.restored && await panel.getByRole('button', {name:'继续', exact:true}).isVisible()) await control('继续', 'running');
        await waitModel();
        if (process.env.NOLA_GOAL_DROP_PAUSE_ACK === '1') await pauseWithLostAcknowledgement();
        else await control('暂停', 'paused');
        await waitStopped();
        await page.reload();
        await click('进度');
        await panel.locator('p.muted').filter({hasText:/^已暂停/}).waitFor();
        if (process.env.NOLA_GOAL_PAUSE_ONLY === '1') {
            await page.screenshot({path:out + '/paused.png'});
            assert.deepEqual(report.errors, []);
            report.passed = true;
            report.pausedForNativeObservation = true;
            return;
        }
        await control('继续', 'running');
        await waitModel();
        await click('取消目标', panel);
        await control('取消目标', 'cancelled', page.getByRole('dialog', { name: '确认取消长期目标？', exact: true }));
        await waitStopped();
        await panel.locator('p.muted').filter({hasText:/^已取消/}).waitFor();
        // The Goal receipt can precede the final trace projection. Observe the
        // rendered history settle without a manual refresh or page reload.
        const history = panel.locator('.section').filter({ has: page.getByRole('heading', { name: '运行记录', exact: true }) });
        await history.locator('ol li').first().waitFor();
        await page.waitForFunction(() => {
            const section = [...document.querySelectorAll('.section')]
                .find(element => element.querySelector('h3')?.textContent === '运行记录');
            const rows = section?.querySelectorAll('ol li');
            return rows?.length && [...rows].every(row => !/· (运行中|待运行|等待中)/.test(row.textContent));
        }, undefined, { timeout: 20000 });
        report.renderedRunHistory = await history.locator('ol li').allTextContents();
        await page.screenshot({ path: out + '/cancelled.png' });
        assert.deepEqual(report.errors, []);
        report.passed = true;
    } catch (error) {
        report.failure = error.message;
        report.pendingRequests = [...pendingRequests.values()].map(request => ({...request, elapsedMs:Date.now()-request.startedAt}));
        await page.screenshot({ path: out + '/failure.png' }).catch(() => {});
        fs.writeFileSync(out + '/failure.txt', await page.locator('body').ariaSnapshot());
        process.exitCode = 1;
    } finally {
        report.transport = [...transport.values()].filter(item => !item.finishedBytes || item.failed);
        fs.writeFileSync(out + '/report.json', JSON.stringify(report, null, 2));
        console.log(JSON.stringify({ passed: report.passed, failure: report.failure, clicks: report.clicks.length, out }));
        awake.kill();
        await browser.close();
    }
})();
