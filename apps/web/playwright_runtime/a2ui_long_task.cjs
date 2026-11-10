// Explicit real-model acceptance. Run only when the shared provider window is assigned.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { explicitProfile, successfulCalls, sealedOutcome, retryableObservationRace } = require('./a2ui_long_task_oracle.cjs');
const timeoutMs = Number(process.env.NOLA_A2UI_LONG_TIMEOUT_MS || 600000);
assert.ok(Number.isSafeInteger(timeoutMs) && timeoutMs >= 60000 && timeoutMs <= 1800000);
const prompt = '请认真设计一个本地优先跨平台知识工作助手的实施计划，比较数据归属、离线同步冲突、安全审批、长任务恢复、性能与验收取舍。请用 render_ui 建立同一张任务进度卡，surface_id 为 implementation-progress，先显示任务与阶段，再分至少四个实质分析阶段逐次更新同一张卡的进度和阶段结论，最后卡上提供完整结论。每阶段至少分析两种方案及选择理由，不能只修改进度数字；最后给出实施顺序和可执行验收标准。不要读写文件、不要访问外部网络，不要使用其他工具。';
(async () => {
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_A2UI_HOST_STATE || require('./test_environment.cjs').artifactPath('nola-a2ui-real-host-state.json')));
    const origin = new URL(state.url).origin;
    assert.equal(new URL(origin).hostname, '127.0.0.1');
    assert.ok(require('node:path').basename(state.root).startsWith('nola-a2ui-real-'));
    const resumeDirectory = process.env.NOLA_A2UI_LONG_RESUME_DIR;
    if (resumeDirectory) assert.ok(require('node:path').basename(resumeDirectory).startsWith('nola-a2ui-long-task-'));
    const out = resumeDirectory || fs.mkdtempSync(require('./test_environment.cjs').temporaryPrefix('nola-a2ui-long-task-'));
    const prior = resumeDirectory ? JSON.parse(fs.readFileSync(`${out}/reconnect-identity.json`)) : null;
    if (resumeDirectory) for (const name of ['result', 'scenarios']) {
        const existing = `${out}/${name}.json`;
        if (fs.existsSync(existing)) fs.copyFileSync(existing, `${out}/${name}-before-resume-${Date.now()}.json`);
    }
    const load = (name, fallback) => fs.existsSync(`${out}/${name}.json`) ? JSON.parse(fs.readFileSync(`${out}/${name}.json`)) : fallback;
    const save = (name, value) => fs.writeFileSync(`${out}/${name}.json`, JSON.stringify(value, null, 2), { mode: 0o600 });
    const browser = await chromium.launch({ ...require('./test_environment.cjs').browserOptions(), headless: true });
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const page = await context.newPage();
    const clientId = prior?.clientId || `web-long-task-${crypto.randomUUID()}`;
    await page.addInitScript(({id, origin}) => {
        if (location.origin === origin) sessionStorage.setItem('nola.web.chat.client', id);
    }, {id: clientId, origin});
    const errors = [];
    let modelSelection, hostId, currentScenario;
    page.on('pageerror', error => errors.push(error.message));
    const transport = [];
    page.on('response', response => {
        const endpoint = new URL(response.url()).pathname;
        if (!['/api/chat/agent/v2/resume', '/api/chat/agent/v2/recovery'].includes(endpoint)) return;
        void response.text().then(body => {
            // Recovery SSE contains no credentials. Store only structured errors,
            // not model content or arbitrary response bodies.
            const failures = body.split('\n').filter(line => line.startsWith('data:')).flatMap(line => {
                try { const event = JSON.parse(line.slice(5)); return event.type === 'error' ? [event] : []; }
                catch { return []; }
            });
            transport.push({at:Date.now(),path:endpoint,status:response.status(),failures});
            save('recovery-transport',transport);
        }).catch(error => {transport.push({at:Date.now(),path:endpoint,error:error.name});save('recovery-transport',transport);});
    });
    const requestFailures = load('request-failures', []);
    let stage = 'bootstrap';
    const request = async (path, body) => {
        const method = body === undefined ? 'GET' : 'POST';
        for (let attempt = 0; ; attempt++) {
            let result;
            try {
                result = await context.request.fetch(origin + path, body === undefined ? {} : { method, data: body });
            } catch (error) {
                const failure = {at:Date.now(),stage,path,method,attempt,identity:currentScenario ? {...currentScenario} : null,
                    transportError:error.message,retryable:false};
                requestFailures.push(failure);save('request-failures',requestFailures);
                error.evidence=failure;throw error;
            }
            if (result.ok()) return result.json();
            const responseText = await result.text();
            let responseBody;try {responseBody=JSON.parse(responseText);} catch {responseBody=responseText;}
            const retryable = retryableObservationRace(path,method,result.status(),responseBody);
            const failure = {at:Date.now(),stage,path,method,attempt,status:result.status(),
                identity:currentScenario ? {...currentScenario} : null,responseBody,retryable,retried:retryable && attempt < 2};
            requestFailures.push(failure);save('request-failures',requestFailures);
            if (failure.retried) continue; // Exact same read body, at most three attempts.
            const error = new Error(`HTTP ${result.status()} at ${path}; see request-failures.json`);
            error.evidence=failure;throw error;
        }
    };
    async function approveUiOnly() {
        const panel = page.locator('.permission-panel');
        if (!await panel.isVisible()) return;
        const tool = (await panel.locator('.tool').innerText()).trim();
        if (tool !== 'render_ui') throw Error(`Unexpected permission ${tool}; no authorization granted`);
        const allow = panel.locator('.actions button.primary');
        if (await allow.isEnabled()) await allow.click();
    }
    async function run(cancel) {
        const label = cancel ? 'cancel' : 'reconnect';
        stage = label + ':open';
        currentScenario = null;
        const recoveringOriginal = !cancel && prior;
        const conversation = recoveringOriginal ? (await request('/api/chat/conversations')).conversations.find(item => item.conversationId === prior.cid) : await request('/api/chat/conversations', {
            title: `A2UI actual long task ${label}`, ...modelSelection,
        });
        assert.ok(conversation, 'Original conversation must still exist');
        const cid = conversation.conversationId;
        currentScenario = recoveringOriginal ? {...prior} : {cid, clientId, submitted: false, sealed: false, runId: null};
        assert.equal(conversation.selectedModelSource, modelSelection.selectedModelSource);
        assert.equal(conversation.selectedModelId, modelSelection.selectedModelId);
        const read = suffix => request(`/api/chat/conversations/${encodeURIComponent(cid)}/${suffix}`);
        await page.goto(`${origin}/chat?conversation=${encodeURIComponent(cid)}`, { waitUntil: 'domcontentloaded' });
        assert.equal(await page.evaluate(() => sessionStorage.getItem('nola.web.chat.client')), clientId);
        if (!recoveringOriginal) {
            await page.locator('#chat-composer').fill(prompt);
            currentScenario.submitted = true;
            save(`${label}-identity`, currentScenario);
            await page.locator('.composer button[type=submit]').click();
        }
        const samples = recoveringOriginal ? load(`${label}-samples`, []) : [];
        const started = samples[0]?.at || Date.now(), deadline = Date.now() + timeoutMs;
        const calls = new Set(), revisions = new Set(samples.flatMap(sample => sample.revisions));
        const oldSurface = recoveringOriginal ? load(`${label}-surfaces`, []).find(value => value.identity.surface_id === 'implementation-progress') : null;
        let runId = recoveringOriginal ? prior.runId : null, identity = oldSurface ? JSON.stringify(oldSurface.identity) : null;
        let disconnected = !!recoveringOriginal, cancelled = false, terminal = false, reattached = false, outcome = null;
        let checkedRecovery = false, resumed = false;
        async function recoverThroughUi() {
            const recovery = page.locator('.recovery');
            if (!await recovery.isVisible()) return;
            if (await recovery.getByRole('button', {name:/^(授权重试|Authorize retry)$/}).count()) {
                throw Error('Unknown effect requires separate human review; original Run preserved');
            }
            const resumeButton = recovery.getByRole('button', {name:/^(恢复当前运行|Resume current run)$/});
            if (!resumed && await resumeButton.count() && await resumeButton.isEnabled()) {
                resumed = true; await resumeButton.click();
                save(`${label}-recovery-ui`, {checked: checkedRecovery, resumed, runId});
                return;
            }
            const inspect = recovery.getByRole('button', {name:/^(检查记录|Inspect records)$/});
            if (!checkedRecovery && await inspect.count() && await inspect.isEnabled()) {
                checkedRecovery = true; await inspect.click();
            }
        }
        while (Date.now() < deadline) {
            stage = label + ':observe';
            await approveUiOnly();
            const messages = await read('messages');
            const projection = messages.conversationProjection;
            const active = projection.active_run;
            const current = typeof active === 'string' ? active : active?.run_id;
            if (current) { runId ??= current; assert.equal(current, runId, 'same Run after reconnection'); }
            currentScenario.runId = runId;
            let workerState = null;
            if (runId) {
                for (const id of successfulCalls(projection, runId)) calls.add(id);
                const observed = (await request('/api/chat/agent/v2/observe', {
                    operation: 'observe', expected_host_id: hostId, client_id: clientId,
                    conversation_id: cid, expected_run_id: runId, assistant_message_id: `${runId}:assistant`,
                })).result;
                assert.equal(observed.run_id, runId);
                assert.equal(observed.client_id, clientId);
                assert.equal(observed.conversation_id, cid);
                assert.equal(observed.assistant_message_id, `${runId}:assistant`);
                workerState = observed.worker?.state;
                if (disconnected && workerState === 'running') reattached = true;
                save(`${label}-observation`, observed);
                outcome = sealedOutcome(observed, cid, runId);
                currentScenario.sealed = outcome !== null;
                if (observed.recovery === 'resume_required' && !outcome) await recoverThroughUi();
                save(`${label}-identity`, currentScenario);
            }
            const surfaces = (await read('ui-surfaces')).surfaces;
            const surface = surfaces.find(item => item.identity.surface_id === 'implementation-progress');
            if (surface) {
                const key = JSON.stringify(surface.identity);
                identity ??= key;
                assert.equal(key, identity, 'all phase updates use the original Surface identity');
                revisions.add(surface.revision);
            }
            samples.push({ at: Date.now(), run_id: current, revisions: [...revisions], calls: calls.size, worker: workerState });
            save(`${label}-samples`, samples);
            save(`${label}-messages`, messages); save(`${label}-surfaces`, surfaces);
            if (current && surface && !disconnected && !cancel) {
                // Detach the actual stream by leaving, then reopen with the original session identity.
                await page.goto('about:blank');
                await page.goto(`${origin}/chat?conversation=${encodeURIComponent(cid)}`, { waitUntil: 'domcontentloaded' });
                assert.equal(await page.evaluate(() => sessionStorage.getItem('nola.web.chat.client')), clientId);
                disconnected = true;
            }
            if (current && surface && cancel && !cancelled) {
                const stop = page.locator('.composer button[type=button]').filter({ hasText: /停止|Stop/ });
                await stop.click(); cancelled = true;
            }
            if (outcome && !active && projection.messages.at(-1)?.state === (cancel ? 'cancelled' : 'completed')) {
                terminal = true; break;
            }
            await page.waitForTimeout(500); // Polling cadence, not artificial task duration.
        }
        assert.ok(terminal, `${label} did not finish within ${timeoutMs}ms`);
        const elapsed = Date.now() - started;
        const observedActiveMs = samples.slice(1).reduce((sum, sample, index) => {
            const previous = samples[index], gap = sample.at - previous.at;
            return sum + (sample.worker === 'running' && previous.worker === 'running' ? gap : 0);
        }, 0);
        if (cancel) {
            assert.ok(cancelled, 'must cancel while the task is active');
            assert.equal(outcome, 'cancelled', 'authoritative Run result must be cancelled');
            const baseline = JSON.stringify((await read('ui-surfaces')).surfaces);
            const end = Date.now() + 15000;
            while (Date.now() < end) {
                assert.equal(JSON.stringify((await read('ui-surfaces')).surfaces), baseline, 'no Surface updates after terminal cancellation');
                assert.equal((await read('messages')).conversationProjection.active_run, null);
                await page.waitForTimeout(1000);
            }
        } else {
            assert.ok(['answered', 'action_completed'].includes(outcome), 'authoritative Run must succeed');
            assert.ok(disconnected && reattached, 'must observe the same active Run after reopening');
            assert.ok(calls.size >= 3 && revisions.size >= 3, 'requires at least three real render_ui calls and observed revisions');

        }
        const result = { cid, runId, elapsed_ms: elapsed, calls: calls.size, revisions: [...revisions], disconnected, reattached, cancelled, terminal, outcome, observed_active_ms: observedActiveMs, passed: cancel || observedActiveMs >= 60000 };
        save(`${label}-result`, result);
        await page.screenshot({path: `${out}/${label}-terminal.png`});
        return result;
    }
    try {
        await page.goto(state.url, { waitUntil: 'domcontentloaded' });
        hostId = (await request('/api/web/session')).hostId;
        const profileId = process.env.NOLA_A2UI_PROFILE_ID;
        assert.ok(profileId, 'NOLA_A2UI_PROFILE_ID must explicitly select the enabled real model');
        modelSelection = explicitProfile((await request('/api/model-profiles')).profiles, profileId);
        if (prior) assert.ok(prior.submitted && prior.runId && prior.clientId === clientId, 'Resume requires exact recorded submitted Run');
        const results = [];
        // A short successful task is still saved; it does not prevent independent cancel verification.
        for (const cancel of [false, true]) {
            try { results.push(await run(cancel)); }
            catch (error) {
                results.push({ scenario: cancel ? 'cancel' : 'reconnect', passed: false, error: error.message, stage, failure: error.evidence ?? null, identity: currentScenario });
                // An observation failure is not a terminal Run. Preserve its
                // identity and stop; do not launch another model over it.
                if (currentScenario?.submitted && !currentScenario.sealed) break;
            }
        }
        save('scenarios', results);
        assert.ok(results.every(result => result.passed), 'One or more independent acceptance scenarios failed; inspect scenarios.json');
        assert.deepEqual(errors, []);
        save('result', { passed: true });
        console.log(JSON.stringify({ passed: true, evidence: out }));
    } catch (error) {
        save('result', { passed: false, error: error.message, errors });
        console.error(JSON.stringify({ passed: false, evidence: out, error: error.message }));
        process.exitCode = 1;
    } finally { await browser.close(); }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
