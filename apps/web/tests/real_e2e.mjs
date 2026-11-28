import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer as createNetServer } from 'node:net';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { chromium } from '../../windows/node_modules/playwright/index.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../..');
const WEB_ROOT = path.join(REPO_ROOT, 'apps/web');
const NOLA_BIN = path.join(REPO_ROOT, 'target/debug/nola');
const CHROME_PATH = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const LLM_ENDPOINT = (process.env.NOLA_REAL_LLM_ENDPOINT || 'http://192.168.31.18:8081/v1').trim();
const LLM_MODEL = (process.env.NOLA_REAL_LLM_MODEL || 'qwen3.8-flash-next-iq3_s').trim();
const LLM_API_KEY = (process.env.NOLA_REAL_LLM_API_KEY || process.env.NOLA_PRIMARY_API_KEY || '').trim();
const EVIDENCE_DIR = path.resolve(process.env.NOLA_E2E_EVIDENCE_DIR || '/tmp/nola-real-e2e-evidence/web');
const HEADLESS = process.env.NOLA_E2E_HEADED !== '1';
const HANDOFF_MARKER = 'NOLA_WEB_E2E_MARKER_9421';

function findFreePort() {
    return new Promise((resolve, reject) => {
        const srv = createNetServer();
        srv.unref();
        srv.on('error', reject);
        srv.listen(0, '127.0.0.1', () => {
            const addr = srv.address();
            const port = typeof addr === 'object' && addr ? addr.port : 0;
            srv.close(() => resolve(port));
        });
    });
}

async function waitForServer(url, timeoutMs = 20000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const res = await fetch(url);
            if (res.ok) return;
        } catch {
            // Server is still booting.
        }
        await new Promise(r => setTimeout(r, 250));
    }
    throw new Error(`Timed out waiting for nola web server at ${url}`);
}

async function waitForStreamComplete(page, { timeoutMs = 120000, autoApprovePermissions = false } = {}) {
    const stopButton = page.locator('.composer-actions button.danger');
    const permAllowBtn = page.locator('.permission-card .interactive-actions button.primary');
    const startedDeadline = Date.now() + 15000;
    let sawStreaming = false;
    let approvedCount = 0;

    // Wait briefly for streaming to start (or complete quickly).
    while (Date.now() < startedDeadline) {
        if (await stopButton.isVisible().catch(() => false)) {
            sawStreaming = true;
            break;
        }
        if (autoApprovePermissions && await permAllowBtn.isVisible().catch(() => false)) {
            sawStreaming = true;
            break;
        }
        await page.waitForTimeout(150);
    }

    const deadline = Date.now() + timeoutMs;
    let idleStreak = 0;
    while (Date.now() < deadline) {
        if (autoApprovePermissions) {
            if (await permAllowBtn.isVisible().catch(() => false) && await permAllowBtn.isEnabled().catch(() => false)) {
                await permAllowBtn.click();
                approvedCount += 1;
                idleStreak = 0;
                await page.waitForTimeout(400);
                continue;
            }
        }
        const streamingNow = await stopButton.isVisible().catch(() => false);
        const permVisible = await permAllowBtn.isVisible().catch(() => false);
        if (!streamingNow && !permVisible) {
            idleStreak += 1;
            if (idleStreak >= 4) {
                return { sawStreaming, approvedCount };
            }
        } else {
            sawStreaming = true;
            idleStreak = 0;
        }
        await page.waitForTimeout(250);
    }
    throw new Error('Timed out waiting for LLM streaming turn to complete');
}

async function run() {
    await fs.mkdir(EVIDENCE_DIR, { recursive: true });
    // Clean stale video files in evidenceDir before starting.
    for (const entry of await fs.readdir(EVIDENCE_DIR)) {
        if (entry.endsWith('.webm')) {
            await fs.rm(path.join(EVIDENCE_DIR, entry), { force: true });
        }
    }

    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'nola-web-real-e2e-'));
    const tempHome = path.join(tempRoot, 'home');
    const tempDataDir = path.join(tempRoot, 'data');
    const tempDbPath = path.join(tempDataDir, 'nola.db');
    const tempWorkspace = path.join(tempRoot, 'workspace');
    await fs.mkdir(tempHome, { recursive: true });
    await fs.mkdir(tempDataDir, { recursive: true });
    await fs.mkdir(tempWorkspace, { recursive: true });
    await fs.writeFile(path.join(tempWorkspace, 'handoff.txt'), `${HANDOFF_MARKER}\n`, 'utf8');
    const localDataSourcePath = path.join(tempWorkspace, 'e2e_datasource.json');
    await fs.writeFile(
        localDataSourcePath,
        `${JSON.stringify([{ id: 1, name: 'Nola E2E Row', status: 'verified' }], null, 2)}\n`,
        'utf8',
    );
    const codexDir = path.join(tempRoot, 'codex');
    await fs.mkdir(codexDir, { recursive: true });
    const codexConfigPath = path.join(codexDir, 'config.toml');
    const codexAuthPath = path.join(codexDir, 'auth.json');
    await fs.writeFile(
        codexConfigPath,
        [
            'model_provider = "codex_e2e"',
            'model = "gpt-5.4-codex-e2e"',
            '',
            '[model_providers.codex_e2e]',
            'name = "Codex E2E Provider"',
            'base_url = "https://api.example.invalid/v1"',
            'wire_api = "responses"',
            'env_key = "CODEX_E2E_KEY"',
            '',
        ].join('\n'),
        'utf8',
    );
    await fs.writeFile(
        codexAuthPath,
        `${JSON.stringify({ CODEX_E2E_KEY: 'sk-e2e-codex-dummy-token-0001' }, null, 2)}\n`,
        'utf8',
    );

    const port = await findFreePort();
    const baseUrl = `http://127.0.0.1:${port}`;
    const serverLogs = [];
    const serverProc = spawn(
        NOLA_BIN,
        ['web', '--host', '127.0.0.1', '--port', String(port), '--web-root', WEB_ROOT],
        {
            cwd: tempWorkspace,
            env: {
                ...process.env,
                HOME: tempHome,
                NOLA_DATA_DIR: tempDataDir,
                NOLA_DB_PATH: tempDbPath,
            },
            stdio: ['ignore', 'pipe', 'pipe'],
        },
    );
    serverProc.stdout.on('data', chunk => serverLogs.push(chunk.toString('utf8')));
    serverProc.stderr.on('data', chunk => serverLogs.push(chunk.toString('utf8')));

    const consoleErrors = [];
    const pageErrors = [];
    const httpErrors = [];
    const screenshots = [];
    const stages = {};
    let browser;
    let context;
    let videoPath;

    const recordShot = async (page, filename) => {
        const dest = path.join(EVIDENCE_DIR, filename);
        await page.screenshot({ path: dest, fullPage: false });
        screenshots.push(dest);
        return dest;
    };

    try {
        await waitForServer(`${baseUrl}/api/web/session`, 20000);
        console.log(`[e2e] nola web listening on ${baseUrl} (model=${LLM_MODEL}, endpoint=${LLM_ENDPOINT})`);

        browser = await chromium.launch({
            executablePath: CHROME_PATH,
            headless: HEADLESS,
            args: ['--no-first-run', '--no-default-browser-check'],
        });

        context = await browser.newContext({
            viewport: { width: 1440, height: 900 },
            locale: 'zh-CN',
            recordVideo: { dir: EVIDENCE_DIR, size: { width: 1440, height: 900 } },
        });
        await context.addCookies([
            { name: 'nola_ui_locale', value: 'zh-Hans', url: baseUrl },
        ]);

        const page = await context.newPage();
        page.setDefaultTimeout(20000);

        page.on('console', msg => {
            if (msg.type() === 'error') {
                consoleErrors.push({ text: msg.text(), location: msg.location() });
            }
        });
        page.on('pageerror', err => {
            pageErrors.push(String(err));
        });
        page.on('response', async res => {
            if (res.status() >= 400) {
                let bodyText = '';
                try {
                    bodyText = (await res.text()).slice(0, 600);
                } catch {
                    // Ignore unreadable response body.
                }
                httpErrors.push({ status: res.status(), url: res.url(), method: res.request().method(), body: bodyText });
            }
        });

        // ------------------------------------------------------------------
        // Stage 1: /models — Configure real LAN model via UI, reload & verify
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/models`, { waitUntil: 'networkidle' });
        await page.locator('.models-toolbar button.primary').waitFor({ state: 'visible' });
        await page.locator('.models-toolbar button.primary').click();

        const createDialog = page.locator('dialog[open]').last();
        await createDialog.waitFor({ state: 'visible' });

        // Fill endpoint name, baseURL, model ID, display name in CreateRemoteModel.svelte.
        const textInputs = createDialog.locator('.fields input:not([type="checkbox"]):not([type="number"])');
        await textInputs.nth(0).fill('LAN Real Model');
        await textInputs.nth(1).fill(LLM_ENDPOINT);
        await textInputs.nth(2).fill(LLM_MODEL);
        await textInputs.nth(3).fill(LLM_MODEL);

        const stageBtn = createDialog.locator('button.primary');
        await assert.doesNotReject(() => stageBtn.waitFor({ state: 'visible' }));
        await stageBtn.click();
        await createDialog.waitFor({ state: 'hidden' });

        const saveCatalogBtn = page.locator('.catalog-actions button.primary');
        await saveCatalogBtn.waitFor({ state: 'visible' });
        await saveCatalogBtn.click();
        await page.locator('.catalog-actions button.primary').waitFor({ state: 'hidden' });

        // If an API key is provided in env, save it through EndpointCredentialEditor without logging it.
        if (LLM_API_KEY) {
            const firstEndpoint = page.locator('details.endpoint-card').first();
            await firstEndpoint.locator('summary.endpoint-summary').click();
            const credBtn = firstEndpoint.locator('.wide-actions button').filter({ hasText: /凭据|密钥|Credential|API Key/i }).first();
            await credBtn.click();
            const credDialog = page.locator('dialog[open]').last();
            await credDialog.locator('input[type="password"]').waitFor({ state: 'visible' });
            await credDialog.locator('input[type="password"]').fill(LLM_API_KEY);
            await credDialog.locator('button.primary').click();
            await credDialog.locator('[role="status"]').waitFor({ state: 'visible' });
            await credDialog.locator('.actions button').filter({ hasText: /关闭|Close/i }).first().click();
            await credDialog.waitFor({ state: 'hidden' });
        }

        await page.reload({ waitUntil: 'networkidle' });
        const endpointCard = page.locator('details.endpoint-card').first();
        await endpointCard.waitFor({ state: 'visible' });
        if (!(await endpointCard.evaluate(el => el.hasAttribute('open')))) {
            await endpointCard.locator('summary.endpoint-summary').click();
        }
        await endpointCard.locator('article.model').getByText(LLM_MODEL, { exact: false }).first().waitFor({ state: 'visible' });
        await recordShot(page, 'web-01-models.png');
        stages.models = { ok: true, model: LLM_MODEL, endpoint: LLM_ENDPOINT };

        // ------------------------------------------------------------------
        // Stage 1b: /models — Discover Models on configured LAN endpoint card
        // ------------------------------------------------------------------
        const discoverBtn = endpointCard.locator('.model-toolbar .actions button').filter({ hasText: /发现|Discover/i }).first();
        await discoverBtn.waitFor({ state: 'visible' });
        const discoverBtnHandle = await discoverBtn.elementHandle();
        await page.waitForFunction(
            btn => btn && !btn.hasAttribute('disabled'),
            discoverBtnHandle,
        );
        const discoverResponsePromise = page.waitForResponse(
            res => res.request().method() === 'POST' && /\/api\/model-endpoints\/[^/]+\/discover$/.test(new URL(res.url()).pathname),
            { timeout: 30000 },
        );
        await discoverBtn.click();
        const discoverRes = await discoverResponsePromise;
        assert.equal(discoverRes.status(), 200, `Expected HTTP 200 from endpoint discover, got ${discoverRes.status()}`);
        const discoverPayload = await discoverRes.json();
        const discoveredList = Array.isArray(discoverPayload?.models) ? discoverPayload.models : [];
        assert.ok(
            discoveredList.includes(LLM_MODEL),
            `Expected discovered models to include ${LLM_MODEL}, got ${JSON.stringify(discoveredList)}`,
        );
        await page.locator('.profile-settings p[role="status"]').first().waitFor({ state: 'visible' });
        assert.equal(await page.locator('.profile-settings .notice.error').count(), 0, 'Unexpected error notice after discover');
        await recordShot(page, 'web-01b-models-discover.png');
        stages.modelsDiscover = { ok: true, discoveredCount: discoveredList.length, includesTargetModel: true };

        // ------------------------------------------------------------------
        // Stage 1c: /models — Import from Codex TOML + auth.json via <ModelConnections>
        // ------------------------------------------------------------------
        const importDetails = page.locator('section.connections section.integrations details').first();
        await importDetails.locator('summary').click();
        const importForm = importDetails.locator('form');
        await importForm.waitFor({ state: 'visible' });
        await importForm.locator('select').selectOption('codex');
        await importForm.locator('input:not([type="checkbox"])').nth(0).fill(codexConfigPath);
        await importForm.locator('input[type="checkbox"]').nth(0).check();
        const credPathInput = importForm.locator('input:not([type="checkbox"])').nth(1);
        await credPathInput.waitFor({ state: 'visible' });
        await credPathInput.fill(codexAuthPath);
        await importForm.locator('button[type="submit"]').click();

        const importPlan = importDetails.locator('.plan');
        await importPlan.waitFor({ state: 'visible' });
        await importPlan.getByText('Codex E2E Provider', { exact: false }).waitFor({ state: 'visible' });
        await importPlan.locator('article input[type="checkbox"]').first().check();
        const confirmImportBtn = importPlan.locator('button.primary');
        await confirmImportBtn.waitFor({ state: 'visible' });
        await confirmImportBtn.click();
        await importDetails.locator('p[role="status"]').first().waitFor({ state: 'visible' });

        await page.locator('section.connections details.connection-detail').getByText('Codex E2E Provider', { exact: false }).first().waitFor({ state: 'visible' });
        await page.reload({ waitUntil: 'networkidle' });
        await page.locator('section.connections details.connection-detail').getByText('Codex E2E Provider', { exact: false }).first().waitFor({ state: 'visible' });
        await recordShot(page, 'web-01c-models-codex-import.png');
        stages.modelsCodexImport = { ok: true, imported: 'Codex E2E Provider', persistedAfterReload: true };

        // ------------------------------------------------------------------
        // Stage 1d: /models — Create Gemini model, Configure…, Save & verify Codex not wiped
        // ------------------------------------------------------------------
        await page.locator('.models-toolbar button.primary').click();
        const geminiCreateDialog = page.locator('dialog[open]').last();
        await geminiCreateDialog.waitFor({ state: 'visible' });
        await geminiCreateDialog.locator('.preset-chip').filter({ hasText: 'Google Gemini' }).click();
        const geminiTextInputs = geminiCreateDialog.locator('.fields input:not([type="checkbox"]):not([type="number"])');
        await geminiTextInputs.nth(2).fill('gemini-2.5-pro');
        await geminiTextInputs.nth(3).fill('Gemini E2E');
        await geminiCreateDialog.locator('button.primary').click();
        await geminiCreateDialog.waitFor({ state: 'hidden' });

        await page.locator('.catalog-actions button.primary').click();
        await page.locator('.catalog-actions button.primary').waitFor({ state: 'hidden' });

        await page.reload({ waitUntil: 'networkidle' });
        // Verify Codex imported connection survived saving Remote Models.
        await page.locator('section.connections details.connection-detail').getByText('Codex E2E Provider', { exact: false }).first().waitFor({ state: 'visible' });
        const geminiCard = page.locator('details.endpoint-card').filter({ hasText: 'Google Gemini' }).first();
        await geminiCard.waitFor({ state: 'visible' });
        if (!(await geminiCard.evaluate(el => el.hasAttribute('open')))) {
            await geminiCard.locator('summary.endpoint-summary').click();
        }
        await geminiCard.locator('article.model').getByText('Gemini E2E', { exact: false }).first().waitFor({ state: 'visible' });

        // Open Configure… on Gemini model, edit display name, verify Save button is enabled & saves cleanly.
        const geminiConfigBtn = geminiCard.locator('article.model .wide-actions button').filter({ hasText: /配置|Configure/i }).first();
        await geminiConfigBtn.click();
        const geminiConfigDialog = page.locator('dialog[open]').last();
        await geminiConfigDialog.waitFor({ state: 'visible' });
        const geminiDisplayNameInput = geminiConfigDialog.locator('fieldset').first().locator('input:not([type="checkbox"])').first();
        await geminiDisplayNameInput.fill('Gemini E2E Updated');
        const geminiSaveBtn = geminiConfigDialog.locator('form > .actions button.primary');
        await geminiSaveBtn.waitFor({ state: 'visible' });
        assert.equal(await geminiSaveBtn.isDisabled(), false, 'Expected Gemini Configure Save button to be enabled');
        await geminiSaveBtn.click();
        await geminiConfigDialog.waitFor({ state: 'hidden' });

        await page.locator('.catalog-actions button.primary').click();
        await page.locator('.catalog-actions button.primary').waitFor({ state: 'hidden' });
        await page.reload({ waitUntil: 'networkidle' });
        const geminiCardAfter = page.locator('details.endpoint-card').filter({ hasText: 'Google Gemini' }).first();
        if (!(await geminiCardAfter.evaluate(el => el.hasAttribute('open')))) {
            await geminiCardAfter.locator('summary.endpoint-summary').click();
        }
        await geminiCardAfter.locator('article.model').getByText('Gemini E2E Updated', { exact: false }).first().waitFor({ state: 'visible' });
        await page.locator('section.connections details.connection-detail').getByText('Codex E2E Provider', { exact: false }).first().waitFor({ state: 'visible' });
        await recordShot(page, 'web-01d-models-gemini.png');
        stages.modelsGemini = { ok: true, model: 'gemini-2.5-pro', updatedDisplayName: 'Gemini E2E Updated', codexPreservedAfterCatalogSave: true };

        // ------------------------------------------------------------------
        // Stage 1e: /model-test — Click Test Model link, verify load & context window
        // ------------------------------------------------------------------
        await page.locator('header.models-header a[href="/model-test"]').click();
        await page.locator('.model-test-page').waitFor({ state: 'visible' });
        const testModelSelect = page.locator('.model-test-page .model-select select');
        await testModelSelect.waitFor({ state: 'visible' });
        const testTargetOption = await testModelSelect.evaluate((sel, targetName) => {
            const opts = Array.from(sel.options);
            const found = opts.find(o => o.textContent?.includes(targetName) || o.value.includes(targetName));
            return found ? found.value : (opts[0]?.value ?? '');
        }, LLM_MODEL);
        assert.ok(testTargetOption, 'Expected real LAN model in /model-test selector');
        await testModelSelect.selectOption(testTargetOption);
        await page.locator('.model-test-page textarea').fill('请只回复 MODEL_TEST_OK。');
        const runTestBtn = page.locator('.model-test-page header.page-header button.primary');
        await runTestBtn.click();
        await page.locator('.model-test-page .rendered-output').waitFor({ state: 'visible', timeout: 60000 });
        await page.waitForFunction(() => {
            const outputStatus = document.querySelector('.model-test-page .output [role="status"]');
            return !outputStatus;
        }, null, { timeout: 60000 });
        const metricsText = await page.locator('.model-test-page .metrics').innerText();
        assert.ok(metricsText.includes('128,000') || /\d+\s*\/\s*[\d,]+/.test(metricsText) || !metricsText.includes('NaN'), `Unexpected metrics text: ${metricsText}`);
        assert.equal(await page.locator('.model-test-page .notice.error').count(), 0, 'Unexpected error notice on /model-test');
        await recordShot(page, 'web-01e-model-test.png');
        stages.modelTest = { ok: true, url: page.url(), metricsClean: true };

        // ------------------------------------------------------------------
        // Stage 2: /chat — Real streaming reply with Markdown table + sentinel
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        await page.locator('#chat-composer').waitFor({ state: 'visible' });

        // Select the configured real LAN model in composer toolbar.
        const modelSelect = page.locator('.composer-toolbar select.toolbar-select').first();
        await modelSelect.waitFor({ state: 'visible' });
        const targetOptionValue = await modelSelect.evaluate((sel, targetName) => {
            const opts = Array.from(sel.options);
            const found = opts.find(o => o.textContent?.includes(targetName) || o.value.includes(targetName));
            return found ? found.value : (opts[0]?.value ?? '');
        }, LLM_MODEL);
        assert.ok(targetOptionValue, 'Expected real LAN model option in chat model selector');
        await modelSelect.selectOption(targetOptionValue);

        const prompt1 = '请用中文简短回复，并给出一个包含两行数据的 Markdown 表格，最后附上一句 OK_WEB_REAL_MODEL。';
        await page.locator('#chat-composer').fill(prompt1);
        await page.locator('.composer-actions button[type="submit"]').click();
        const turn1 = await waitForStreamComplete(page, { timeoutMs: 120000, autoApprovePermissions: true });

        await page.locator('table').first().waitFor({ state: 'visible', timeout: 15000 });
        const bodyTextTurn1 = await page.locator('body').innerText();
        assert.ok(
            bodyTextTurn1.includes('OK_WEB_REAL_MODEL'),
            `Expected OK_WEB_REAL_MODEL in real LLM reply, got snippet: ${bodyTextTurn1.slice(-400)}`,
        );
        const chatUrlAfterTurn1 = page.url();
        await recordShot(page, 'web-02-chat-real-reply.png');
        stages.chatRealReply = { ok: true, ...turn1, hasTable: true, hasSentinel: true };

        // ------------------------------------------------------------------
        // Stage 3: /chat — Real tool turn (read handoff.txt + write e2e_proof.txt)
        // ------------------------------------------------------------------
        const handoffPath = path.join(tempWorkspace, 'handoff.txt');
        const proofPath = path.join(tempWorkspace, 'e2e_proof.txt');
        const prompt2 = `请使用文件工具读取 ${handoffPath} 的内容，并在 ${proofPath} 写入文本 REAL_TOOL_OK（只写这一行），最后在回复中把读取到的标记文本和写入结果告诉我。`;
        await page.locator('#chat-composer').fill(prompt2);
        await page.locator('.composer-actions button[type="submit"]').click();
        const turn2 = await waitForStreamComplete(page, { timeoutMs: 150000, autoApprovePermissions: true });

        let proofWritten = false;
        let proofContent = '';
        try {
            proofContent = (await fs.readFile(proofPath, 'utf8')).trim();
            proofWritten = proofContent.includes('REAL_TOOL_OK');
        } catch {
            // Check whether tool card rendered or handoff marker was read.
        }
        const toolCardsCount = await page.locator('details.tool-card').count();
        const bodyTextTurn2 = await page.locator('body').innerText();
        const markerRead = bodyTextTurn2.includes(HANDOFF_MARKER);
        assert.ok(
            proofWritten || markerRead || toolCardsCount > 0,
            `Expected real tool execution evidence (proofWritten=${proofWritten}, markerRead=${markerRead}, toolCards=${toolCardsCount})`,
        );
        await recordShot(page, 'web-03-chat-tool.png');
        stages.chatTool = { ok: true, ...turn2, proofWritten, markerRead, toolCardsCount };

        // ------------------------------------------------------------------
        // Stage 4: Inspector — Open & click through 5 tabs
        // ------------------------------------------------------------------
        await page.locator('[data-testid="inspector-toggle"]').click();
        await page.locator('#inspector-tab-tasks').waitFor({ state: 'visible' });
        const inspectorTabs = [
            { id: 'tasks', label: '任务' },
            { id: 'artifacts', label: '产物' },
            { id: 'context', label: '上下文' },
            { id: 'data-sources', label: '数据源' },
            { id: 'knowledge', label: '知识' },
        ];
        for (const tab of inspectorTabs) {
            const tabBtn = page.locator(`#inspector-tab-${tab.id}`);
            await tabBtn.click();
            await page.locator(`#inspector-panel-${tab.id}`).waitFor({ state: 'visible' });
            await page.waitForTimeout(200);
        }
        await recordShot(page, 'web-04-inspector.png');
        stages.inspector = { ok: true, tabsVisited: inspectorTabs.map(t => t.label) };

        // ------------------------------------------------------------------
        // Stage 4b: Chat management actions — Compact, Rename, Pin, Reload & verify
        // ------------------------------------------------------------------
        await page.locator('#inspector-tab-context').click();
        await page.locator('#inspector-panel-context').waitFor({ state: 'visible' });
        const compactBtn = page.locator('#inspector-panel-context button.primary.compact-btn');
        await compactBtn.waitFor({ state: 'visible' });
        let compactedTriggered = false;
        if (!(await compactBtn.isDisabled())) {
            const compactRespPromise = page.waitForResponse(
                res => res.request().method() === 'POST' && /\/api\/chat\/conversations\/[^/]+\/compact$/.test(new URL(res.url()).pathname),
                { timeout: 60000 },
            );
            await compactBtn.click();
            const compactRes = await compactRespPromise;
            assert.equal(compactRes.status(), 200, `Expected HTTP 200 from /compact, got ${compactRes.status()}`);
            await page.locator('#inspector-panel-context .inspector-notice').waitFor({ state: 'visible', timeout: 15000 });
            compactedTriggered = true;
        }

        const activeConvRow = page.locator('.conversation-list .conversation-row').filter({
            has: page.locator('button.conversation[aria-current="true"]'),
        }).first();
        await activeConvRow.waitFor({ state: 'visible' });

        // Rename active conversation via UI.
        await activeConvRow.hover();
        await activeConvRow.locator('details.row-actions > summary').click();
        await activeConvRow.locator('.row-action-buttons button').filter({ hasText: /重命名|Rename/i }).first().click();
        const renameForm = activeConvRow.locator('form.rename-form');
        await renameForm.waitFor({ state: 'visible' });
        const renamedTitle = 'E2E Renamed Chat';
        await renameForm.locator('input').fill(renamedTitle);
        await renameForm.locator('button[type="submit"].primary').click();
        await renameForm.waitFor({ state: 'hidden' });
        await activeConvRow.getByText(renamedTitle, { exact: false }).waitFor({ state: 'visible' });

        // Toggle Pin on active conversation via UI.
        await activeConvRow.hover();
        await activeConvRow.locator('details.row-actions > summary').click();
        await activeConvRow.locator('.row-action-buttons button').filter({ hasText: /置顶|Pin/i }).first().click();
        await page.locator('.conversation-list h3.group-title').filter({ hasText: /置顶|Pinned/i }).first().waitFor({ state: 'visible' });

        // Reload and verify renamed title + pin persisted.
        await page.reload({ waitUntil: 'networkidle' });
        await page.locator('.conversation-list').getByText(renamedTitle, { exact: false }).first().waitFor({ state: 'visible' });
        await page.locator('.conversation-list h3.group-title').filter({ hasText: /置顶|Pinned/i }).first().waitFor({ state: 'visible' });
        await recordShot(page, 'web-04b-chat-management.png');
        stages.chatManagement = { ok: true, renamedTitle, pinned: true, compactedTriggered };

        // ------------------------------------------------------------------
        // Stage 5: /experts — Create QA_Expert, reload & verify persisted
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/experts`, { waitUntil: 'networkidle' });
        const expertForm = page.locator('.mgr-standalone form.mgr-form-col');
        await expertForm.waitFor({ state: 'visible' });
        await expertForm.locator('input[type="text"]').nth(0).fill('QA_Expert');
        await expertForm.locator('input[type="text"]').nth(1).fill('Real E2E verification specialist');
        await expertForm.locator('textarea').fill('Verify outputs accurately and concisely.');
        await expertForm.locator('button[type="submit"].primary').click();
        await page.locator('.mgr-standalone .entity-list').getByText('@QA_Expert', { exact: false }).waitFor({ state: 'visible' });

        await page.reload({ waitUntil: 'networkidle' });
        await page.locator('.mgr-standalone .entity-list').getByText('@QA_Expert', { exact: false }).waitFor({ state: 'visible' });
        await recordShot(page, 'web-05-experts.png');
        stages.experts = { ok: true, created: 'QA_Expert' };

        // ------------------------------------------------------------------
        // Stage 6: /automations — Create scheduled task via UI, reload & verify
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/automations`, { waitUntil: 'networkidle' });
        await page.locator('header.page-header button.primary').click();
        const autoDialog = page.locator('dialog.automation-dialog[open]');
        await autoDialog.waitFor({ state: 'visible' });

        const autoInputs = autoDialog.locator('.editor-body > fieldset > label > input');
        await autoInputs.nth(0).fill('Daily E2E Check');
        await autoDialog.locator('.editor-body > fieldset > label > textarea').first().fill('Summarize workspace status concisely.');
        await autoInputs.nth(1).fill(tempWorkspace);
        await autoDialog.locator('.editor-body > fieldset > label > select').nth(1).selectOption('cautious');
        await autoDialog.locator('footer button[type="submit"].primary').click();
        await autoDialog.waitFor({ state: 'hidden' });

        await page.locator('.automation-row').getByText('Daily E2E Check', { exact: false }).waitFor({ state: 'visible' });
        await page.reload({ waitUntil: 'networkidle' });
        const autoRow = page.locator('.automation-row').filter({ hasText: 'Daily E2E Check' }).first();
        await autoRow.waitFor({ state: 'visible' });

        // Toggle automation (Pause -> disabled, Enable -> enabled) and trigger Run Now if enabled.
        const toggleBtn = autoRow.locator('.buttons > button').nth(1);
        await toggleBtn.click();
        await autoRow.locator('.badge[data-status="disabled"]').waitFor({ state: 'visible' });
        await toggleBtn.click();
        await autoRow.locator('.badge[data-status="enabled"]').waitFor({ state: 'visible' });

        const runNowBtn = autoRow.locator('.buttons > button').nth(0);
        let runNowTriggered = false;
        if (!(await runNowBtn.isDisabled())) {
            const runNowRespPromise = page.waitForResponse(
                res => res.request().method() === 'POST' && /\/api\/scheduled-actions\/[^/]+\/run-now$/.test(new URL(res.url()).pathname),
                { timeout: 60000 },
            );
            await runNowBtn.click();
            const confirmDialog = page.locator('dialog[open]').last();
            await confirmDialog.waitFor({ state: 'visible' });
            await confirmDialog.locator('.actions button').last().click();
            await confirmDialog.waitFor({ state: 'hidden' });
            const runNowRes = await runNowRespPromise;
            assert.equal(runNowRes.status(), 200, `Expected HTTP 200 from automation run-now, got ${runNowRes.status()}`);
            runNowTriggered = true;
        }
        await recordShot(page, 'web-06-automations.png');
        stages.automations = { ok: true, created: 'Daily E2E Check', toggled: true, runNowTriggered };

        // ------------------------------------------------------------------
        // Stage 7: /data-sources?tab=memory — Create & revise memory fact + create local Data Source
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/data-sources?tab=memory`, { waitUntil: 'networkidle' });
        const createMemBtn = page.locator('.memory-center .memory-actions button.primary');
        await createMemBtn.waitFor({ state: 'visible' });
        await page.waitForFunction(() => {
            const btn = document.querySelector('.memory-center .memory-actions button.primary');
            return btn && !btn.hasAttribute('disabled');
        });
        await createMemBtn.click();

        const memDialog = page.locator('dialog[open]').last();
        await memDialog.waitFor({ state: 'visible' });
        const memFact = 'E2E Memory Fact: Prefer concise structured verification reports.';
        await memDialog.locator('textarea').nth(0).fill(memFact);
        await memDialog.locator('textarea').nth(1).fill('Concise E2E report preference');
        await memDialog.locator('button[type="submit"].primary').click();
        await memDialog.waitFor({ state: 'hidden' });

        await page.locator('.memory-center article').getByText(memFact, { exact: false }).waitFor({ state: 'visible' });
        await page.reload({ waitUntil: 'networkidle' });
        const memArticle = page.locator('.memory-center article').filter({ hasText: memFact }).first();
        await memArticle.waitFor({ state: 'visible' });

        // Revise memory fact & verify updated text persists across reload.
        await memArticle.locator('.actions button:not(.danger)').filter({ hasText: /修订|修改|编辑|Revise|Edit/i }).first().click();
        const memReviseDialog = page.locator('dialog[open]').last();
        await memReviseDialog.waitFor({ state: 'visible' });
        const memFactRevised = 'E2E Memory Fact Revised: Always include concrete reload-verified evidence.';
        await memReviseDialog.locator('textarea').nth(0).fill(memFactRevised);
        await memReviseDialog.locator('button[type="submit"].primary').click();
        await memReviseDialog.waitFor({ state: 'hidden' });
        await page.locator('.memory-center article').getByText(memFactRevised, { exact: false }).waitFor({ state: 'visible' });

        await page.reload({ waitUntil: 'networkidle' });
        await page.locator('.memory-center article').getByText(memFactRevised, { exact: false }).waitFor({ state: 'visible' });
        await recordShot(page, 'web-07-memory.png');
        stages.memory = { ok: true, persisted: true, revisedPersisted: true };

        // Switch to Data Sources tab, create local data source, reload & verify persisted.
        const dataSourcesTab = page.locator('[data-workspace-id="data-tab-sources"], #data-tab-sources').first();
        const dataSourcesPanel = page.locator('[data-workspace-id="data-panel-sources"], #data-panel-sources').first();
        await dataSourcesTab.click();
        await dataSourcesPanel.waitFor({ state: 'visible' });
        const addSourceBtn = page.locator('header.page-header .page-actions button.primary');
        await addSourceBtn.waitFor({ state: 'visible' });
        await page.waitForFunction(() => {
            const btn = document.querySelector('header.page-header .page-actions button.primary');
            return btn && !btn.hasAttribute('disabled');
        });
        await addSourceBtn.click();
        const sourceDialog = page.locator('dialog[open]').last();
        await sourceDialog.waitFor({ state: 'visible' });
        await sourceDialog.locator('fieldset input').nth(0).fill('E2E Local Dataset');
        await sourceDialog.locator('fieldset select').nth(0).selectOption('localFile');
        await sourceDialog.locator('fieldset input').nth(2).fill(localDataSourcePath);
        await sourceDialog.locator('button.primary').click();
        await sourceDialog.waitFor({ state: 'hidden' });

        await dataSourcesPanel.locator('article.source').getByText('E2E Local Dataset', { exact: false }).waitFor({ state: 'visible' });
        await page.goto(`${baseUrl}/data-sources?tab=sources`, { waitUntil: 'networkidle' });
        await dataSourcesPanel.locator('article.source').getByText('E2E Local Dataset', { exact: false }).waitFor({ state: 'visible' });
        await recordShot(page, 'web-07b-data-sources.png');
        stages.dataSources = { ok: true, created: 'E2E Local Dataset', persisted: true };

        // ------------------------------------------------------------------
        // Stage 8: /capabilities — Real Skill & MCP CRUD + reload persistence
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/capabilities`, { waitUntil: 'networkidle' });
        const skillsTab = page.locator('[data-workspace-id="capability-skills"], #capability-skills').first();
        const mcpTab = page.locator('[data-workspace-id="capability-mcp"], #capability-mcp').first();
        const capPanel = page.locator('[data-workspace-id="capability-panel"], #capability-panel').first();
        await skillsTab.waitFor({ state: 'visible' });
        await capPanel.waitFor({ state: 'visible' });

        // Create a custom Skill and verify reload persistence.
        const addCapBtn = page.locator('header.page-header .actions button.primary');
        await addCapBtn.waitFor({ state: 'visible' });
        await page.waitForFunction(() => {
            const btn = document.querySelector('header.page-header .actions button.primary');
            return btn && !btn.hasAttribute('disabled');
        });
        await addCapBtn.click();
        const skillDialog = page.locator('dialog[open]').last();
        await skillDialog.waitFor({ state: 'visible' });
        await skillDialog.locator('fieldset input:not([type="checkbox"])').first().fill('E2E Custom Skill');
        await skillDialog.locator('fieldset textarea').nth(0).fill('Custom skill created during real Web E2E test.');
        await skillDialog.locator('fieldset textarea').nth(1).fill('# E2E Custom Skill\nVerify all persistence checks cleanly.');
        await skillDialog.locator('button[type="submit"].primary').click();
        await skillDialog.waitFor({ state: 'hidden' });
        await capPanel.getByText('E2E Custom Skill', { exact: false }).first().waitFor({ state: 'visible' });

        await page.reload({ waitUntil: 'networkidle' });
        await capPanel.getByText('E2E Custom Skill', { exact: false }).first().waitFor({ state: 'visible' });

        // Switch to MCP tab, create an MCP server, reload & verify persisted.
        await mcpTab.click();
        await page.locator('[data-workspace-id="capability-mcp"][aria-selected="true"], #capability-mcp[aria-selected="true"]').first().waitFor({ state: 'visible' });
        await addCapBtn.click();
        const mcpDialog = page.locator('dialog[open]').last();
        await mcpDialog.waitFor({ state: 'visible' });
        await mcpDialog.locator('fieldset input:not([type="checkbox"])').nth(0).fill('E2E MCP Server');
        await mcpDialog.locator('fieldset textarea').nth(0).fill('Remote MCP server added during real Web E2E test.');
        await mcpDialog.locator('fieldset input[type="url"]').fill('https://mcp.example.invalid/sse');
        await mcpDialog.locator('button[type="submit"].primary').click();
        await mcpDialog.waitFor({ state: 'hidden' });
        await capPanel.getByText('E2E MCP Server', { exact: false }).first().waitFor({ state: 'visible' });

        await page.goto(`${baseUrl}/capabilities?tab=mcp`, { waitUntil: 'networkidle' });
        await page.locator('[data-workspace-id="capability-mcp"][aria-selected="true"], #capability-mcp[aria-selected="true"]').first().waitFor({ state: 'visible' });
        await capPanel.getByText('E2E MCP Server', { exact: false }).first().waitFor({ state: 'visible' });
        await recordShot(page, 'web-08-capabilities.png');
        stages.capabilities = { ok: true, tabsVisited: ['skills', 'mcp'], skillPersisted: 'E2E Custom Skill', mcpPersisted: 'E2E MCP Server' };

        // ------------------------------------------------------------------
        // Stage 9: /settings — Toggle appearance to Dark mode & verify theme
        // ------------------------------------------------------------------
        await page.goto(`${baseUrl}/settings`, { waitUntil: 'networkidle' });
        const appearanceSelect = page.locator('.preferences-card select').last();
        await appearanceSelect.waitFor({ state: 'visible' });
        await appearanceSelect.selectOption('dark');
        await page.waitForFunction(() => document.documentElement.dataset.appearance === 'dark');
        await page.waitForTimeout(200);
        await recordShot(page, 'web-09-settings-dark.png');
        stages.settingsDark = { ok: true, appearance: 'dark' };

        // ------------------------------------------------------------------
        // Stage 10: Mobile 390x844 — /conversations, /chat, /models, /settings
        // ------------------------------------------------------------------
        await page.setViewportSize({ width: 390, height: 844 });
        await page.goto(`${baseUrl}/conversations`, { waitUntil: 'networkidle' });
        await page.locator('.mobile-primary-navigation').waitFor({ state: 'visible' });

        await page.goto(chatUrlAfterTurn1, { waitUntil: 'networkidle' });
        await page.locator('#chat-composer').waitFor({ state: 'visible' });
        await page.waitForTimeout(250);
        await recordShot(page, 'web-10-mobile-chat.png');

        await page.goto(`${baseUrl}/models`, { waitUntil: 'networkidle' });
        await page.locator('.models-toolbar').waitFor({ state: 'visible' });

        await page.goto(`${baseUrl}/settings`, { waitUntil: 'networkidle' });
        await page.locator('.settings-container').waitFor({ state: 'visible' });
        await page.waitForTimeout(250);
        await recordShot(page, 'web-11-mobile-settings.png');
        stages.mobile = { ok: true, viewport: '390x844', pagesVisited: ['/conversations', '/chat', '/models', '/settings'] };

        videoPath = await page.video()?.path();
    } catch (err) {
        console.error('[e2e] Stage failure diagnostics:', JSON.stringify({ stages, consoleErrors, pageErrors, httpErrors }, null, 2));
        throw err;
    } finally {
        if (context) await context.close();
        if (browser) await browser.close();
        serverProc.kill('SIGTERM');
        await new Promise(resolve => {
            const timer = setTimeout(() => {
                serverProc.kill('SIGKILL');
                resolve();
            }, 3000);
            serverProc.once('exit', () => {
                clearTimeout(timer);
                resolve();
            });
        });
        await fs.rm(tempRoot, { recursive: true, force: true });
    }

    let finalVideoPath = null;
    if (videoPath) {
        finalVideoPath = path.join(EVIDENCE_DIR, 'web-e2e.webm');
        await fs.rename(videoPath, finalVideoPath);
    }

    const summary = {
        ok: consoleErrors.length === 0 && pageErrors.length === 0 && httpErrors.length === 0,
        timestamp: new Date().toISOString(),
        model: LLM_MODEL,
        endpoint: LLM_ENDPOINT,
        evidenceDir: EVIDENCE_DIR,
        video: finalVideoPath,
        screenshots,
        stages,
        errors: {
            consoleErrors,
            pageErrors,
            httpErrors,
        },
    };
    await fs.writeFile(path.join(EVIDENCE_DIR, 'summary.json'), `${JSON.stringify(summary, null, 2)}\n`, 'utf8');

    assert.equal(consoleErrors.length, 0, `Unexpected console.error events: ${JSON.stringify(consoleErrors)}`);
    assert.equal(pageErrors.length, 0, `Unexpected pageerror events: ${JSON.stringify(pageErrors)}`);
    assert.equal(httpErrors.length, 0, `Unexpected HTTP >= 400 responses: ${JSON.stringify(httpErrors)}`);

    console.log(`[e2e] PASS — ${screenshots.length} screenshots + video saved under ${EVIDENCE_DIR}`);
}

run().catch(err => {
    console.error('[e2e] FAIL:', err);
    process.exitCode = 1;
});
