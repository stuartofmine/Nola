import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';
import { createServer as createNetServer } from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '../../windows/node_modules/playwright/test.mjs';
import { startMockOpenAIServer } from './mock-openai-server.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '../../..');
const WEB_ROOT = path.join(REPO_ROOT, 'apps/web');
const NOLA_BIN = process.env.NOLA_BIN || path.join(REPO_ROOT, 'target/debug/nola');

const USE_REAL_LLM = process.env.NOLA_E2E_USE_REAL_LLM === '1';
const REAL_LLM_ENDPOINT = (process.env.NOLA_REAL_LLM_ENDPOINT || '').trim();
const REAL_LLM_MODEL = (process.env.NOLA_REAL_LLM_MODEL || '').trim();
const REAL_LLM_API_KEY = (process.env.NOLA_REAL_LLM_API_KEY || '').trim();

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

async function waitForServer(url, timeoutMs = 25_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const res = await fetch(url);
            if (res.ok) return;
        } catch {
            // Still booting.
        }
        await new Promise(r => setTimeout(r, 200));
    }
    throw new Error(`Timed out waiting for nola web server at ${url}`);
}

async function fileExists(filePath) {
    try {
        await fs.access(filePath);
        return true;
    } catch {
        return false;
    }
}

async function waitForTurnIdle(page, { timeoutMs = 45_000 } = {}) {
    const stopBtn = page.locator('.composer-actions button.danger');
    const permCard = page.locator('.permission-card');

    // Briefly observe if streaming starts (fast responses may finish immediately).
    const startDeadline = Date.now() + 4_000;
    while (Date.now() < startDeadline) {
        if (await stopBtn.isVisible().catch(() => false)) break;
        if (await permCard.isVisible().catch(() => false)) break;
        await page.waitForTimeout(80);
    }

    const deadline = Date.now() + timeoutMs;
    let idleTicks = 0;
    while (Date.now() < deadline) {
        const streamingNow = await stopBtn.isVisible().catch(() => false);
        const permNow = await permCard.isVisible().catch(() => false);
        if (!streamingNow && !permNow) {
            idleTicks += 1;
            if (idleTicks >= 4) return;
        } else {
            idleTicks = 0;
        }
        await page.waitForTimeout(150);
    }
    throw new Error('Timed out waiting for streaming turn to complete');
}

async function ensureDetailsOpen(detailsLocator) {
    await expect(detailsLocator).toBeVisible();
    const isOpen = await detailsLocator.evaluate(el => el.hasAttribute('open'));
    if (!isOpen) {
        await detailsLocator.locator(':scope > summary').click();
    }
}

async function saveCatalogIfDirty(page) {
    const saveBtn = page.locator('.catalog-actions button.primary');
    await expect(saveBtn).toBeVisible();
    await expect(saveBtn).toBeEnabled();
    await saveBtn.click();
    await expect(saveBtn).toBeHidden();
}

async function createNewConversation(page) {
    const createRespPromise = page.waitForResponse(
        res =>
            res.request().method() === 'POST' &&
            new URL(res.url()).pathname === '/api/chat/conversations',
    );
    await page.locator('.conversation-manager button.new-conversation').click();
    const res = await createRespPromise;
    expect(res.ok()).toBe(true);
    await expect(page.locator('article.message')).toHaveCount(0);
    await expect(page.locator('#chat-composer')).toBeEnabled();
}

test.describe.serial('Nola Web Client Real-Browser E2E Workflows', () => {
    let tempRoot;
    let tempWorkspace;
    let tempDbPath;
    let localDataSourcePath;
    let mockServer;
    let serverProc;
    let serverLogs = [];
    let baseUrl;
    let llmEndpoint;
    let primaryModelId;
    let discoveredModelId;

    let browserContext;
    let page;
    let consoleErrors = [];
    let pageErrors = [];
    let server5xx = [];
    let unexpected4xx = [];

    const resetErrorTrackers = () => {
        consoleErrors = [];
        pageErrors = [];
        server5xx = [];
        unexpected4xx = [];
    };

    const assertZeroErrors = contextLabel => {
        expect(
            pageErrors,
            `[${contextLabel}] Uncaught pageerror events:\n${pageErrors.join('\n')}`,
        ).toEqual([]);
        expect(
            consoleErrors,
            `[${contextLabel}] Console error events:\n${JSON.stringify(consoleErrors, null, 2)}`,
        ).toEqual([]);
        expect(
            server5xx,
            `[${contextLabel}] Unexpected HTTP 5xx responses:\n${JSON.stringify(server5xx, null, 2)}`,
        ).toEqual([]);
        expect(
            unexpected4xx,
            `[${contextLabel}] Unexpected HTTP 4xx responses:\n${JSON.stringify(unexpected4xx, null, 2)}`,
        ).toEqual([]);
    };

    test.beforeAll(async ({ browser }) => {
        tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'nola-web-e2e-suite-'));
        const tempHome = path.join(tempRoot, 'home');
        const tempDataDir = path.join(tempRoot, 'data');
        tempDbPath = path.join(tempDataDir, 'nola.db');
        tempWorkspace = path.join(tempRoot, 'workspace');
        await fs.mkdir(tempHome, { recursive: true });
        await fs.mkdir(tempDataDir, { recursive: true });
        await fs.mkdir(tempWorkspace, { recursive: true });

        localDataSourcePath = path.join(tempWorkspace, 'workflow_dataset.json');
        await fs.writeFile(
            localDataSourcePath,
            `${JSON.stringify([{ id: 1, title: 'Nola Workflow Fixture', status: 'ok' }], null, 2)}\n`,
            'utf8',
        );

        if (USE_REAL_LLM && REAL_LLM_ENDPOINT && REAL_LLM_MODEL) {
            llmEndpoint = REAL_LLM_ENDPOINT;
            primaryModelId = REAL_LLM_MODEL;
            discoveredModelId = REAL_LLM_MODEL;
        } else {
            mockServer = await startMockOpenAIServer({
                models: ['mock-nola-alpha', 'mock-nola-beta', 'mock-nola-gamma'],
            });
            llmEndpoint = mockServer.v1Url;
            primaryModelId = 'mock-nola-alpha';
            discoveredModelId = 'mock-nola-beta';

            // Verify mock /props endpoint is healthy and llama.cpp compatible.
            const propsRes = await fetch(`${mockServer.baseUrl}/props`);
            expect(propsRes.status).toBe(200);
            const propsJson = await propsRes.json();
            expect(propsJson?.default_generation_settings?.n_ctx).toBeGreaterThan(0);
        }

        const port = await findFreePort();
        baseUrl = `http://127.0.0.1:${port}`;
        serverLogs = [];

        serverProc = spawn(
            NOLA_BIN,
            [
                'web',
                '--host',
                '127.0.0.1',
                '--port',
                String(port),
                '--web-root',
                WEB_ROOT,
                '--db',
                tempDbPath,
            ],
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

        await waitForServer(`${baseUrl}/api/web/session`, 25_000);

        browserContext = await browser.newContext({
            viewport: { width: 1440, height: 900 },
            locale: 'en-US',
        });
        await browserContext.addCookies([
            { name: 'nola_ui_locale', value: 'en', url: baseUrl },
        ]);

        page = await browserContext.newPage();
        page.setDefaultTimeout(20_000);

        page.on('console', msg => {
            if (msg.type() === 'error') {
                consoleErrors.push({ text: msg.text(), location: msg.location() });
            }
        });
        page.on('pageerror', err => {
            pageErrors.push(String(err));
        });
        page.on('response', async res => {
            const status = res.status();
            if (status >= 500) {
                let body = '';
                try {
                    body = (await res.text()).slice(0, 400);
                } catch {
                    // Ignore unreadable body.
                }
                server5xx.push({
                    status,
                    method: res.request().method(),
                    url: res.url(),
                    body,
                });
            } else if (status >= 400) {
                let body = '';
                try {
                    body = (await res.text()).slice(0, 400);
                } catch {
                    // Ignore unreadable body.
                }
                unexpected4xx.push({
                    status,
                    method: res.request().method(),
                    url: res.url(),
                    body,
                });
            }
        });
    });

    test.afterAll(async () => {
        if (browserContext) await browserContext.close().catch(() => {});
        if (serverProc && !serverProc.killed) {
            serverProc.kill('SIGTERM');
            await new Promise(resolve => {
                const timer = setTimeout(() => {
                    serverProc.kill('SIGKILL');
                    resolve();
                }, 3_000);
                serverProc.once('exit', () => {
                    clearTimeout(timer);
                    resolve();
                });
            });
        }
        if (mockServer) await mockServer.close().catch(() => {});
        if (tempRoot) await fs.rm(tempRoot, { recursive: true, force: true }).catch(() => {});
    });

    test.beforeEach(() => {
        resetErrorTrackers();
    });

    // -------------------------------------------------------------------------
    // Workflow 1: Model Center Full Lifecycle
    // -------------------------------------------------------------------------
    test('1. Model Center full lifecycle: add, discover, enable, set default, edit, delete & reload persistence', async () => {
        await page.goto(`${baseUrl}/models`, { waitUntil: 'networkidle' });
        await expect(page.locator('.models-toolbar button.primary')).toBeVisible();

        // 1a. Add primary API connection pointing to local mock server (or real LAN override).
        await page.locator('.models-toolbar button.primary').click();
        const createDialog = page.locator('dialog[open]').last();
        await expect(createDialog).toBeVisible();

        const inputs = createDialog.locator('.fields input:not([type="checkbox"]):not([type="number"])');
        await inputs.nth(0).fill('Local Mock Primary');
        await inputs.nth(1).fill(llmEndpoint);
        await inputs.nth(2).fill(primaryModelId);
        await inputs.nth(3).fill('Mock Alpha');
        // Enable reasoning capability so reasoning output is declared.
        await createDialog.locator('.uses').nth(1).locator('input[type="checkbox"]').nth(2).check();

        await createDialog.locator('button.primary').click();
        await expect(createDialog).toBeHidden();
        await saveCatalogIfDirty(page);

        if (USE_REAL_LLM && REAL_LLM_API_KEY) {
            const firstCard = page.locator('details.endpoint-card').first();
            await ensureDetailsOpen(firstCard);
            await firstCard.locator('.wide-actions button').filter({ hasText: /Credential|API Key|凭据/i }).first().click();
            const credDialog = page.locator('dialog[open]').last();
            await credDialog.locator('input[type="password"]').fill(REAL_LLM_API_KEY);
            await credDialog.locator('button.primary').click();
            await expect(credDialog.locator('[role="status"]')).toBeVisible();
            await credDialog.locator('.actions button').filter({ hasText: /Close|关闭/i }).first().click();
            await expect(credDialog).toBeHidden();
        }

        // 1b. Click Discover -> verify discovered models appear -> add discovered model.
        const primaryCard = page.locator('details.endpoint-card').filter({ hasText: 'Local Mock Primary' }).first();
        await ensureDetailsOpen(primaryCard);

        const discoverBtn = primaryCard.locator('.model-toolbar .actions button').filter({ hasText: /Discover|发现/i }).first();
        await expect(discoverBtn).toBeEnabled();

        const discoverRespPromise = page.waitForResponse(
            res =>
                res.request().method() === 'POST' &&
                /\/api\/model-endpoints\/[^/]+\/discover$/.test(new URL(res.url()).pathname),
        );
        await discoverBtn.click();
        const discoverRes = await discoverRespPromise;
        expect(discoverRes.status()).toBe(200);
        const discoverBody = await discoverRes.json();
        expect(Array.isArray(discoverBody.models)).toBe(true);
        expect(discoverBody.models).toContain(discoveredModelId);

        if (!USE_REAL_LLM) {
            // Open discovered dropdown and add mock-nola-beta.
            const discoveredDetails = primaryCard.locator('details.discovered');
            await expect(discoveredDetails).toBeVisible();
            await discoveredDetails.locator('summary').click();
            const discoveredBtn = discoveredDetails.locator('.discovered-list button', { hasText: discoveredModelId });
            await expect(discoveredBtn).toBeVisible();
            await discoveredBtn.click();

            const addDiscoveredDialog = page.locator('dialog[open]').last();
            await expect(addDiscoveredDialog).toBeVisible();
            await addDiscoveredDialog.locator('.fields input:not([type="checkbox"]):not([type="number"])').nth(1).fill('Mock Beta');
            await addDiscoveredDialog.locator('.uses').nth(1).locator('input[type="checkbox"]').nth(2).check();
            await addDiscoveredDialog.locator('button.primary').click();
            await expect(addDiscoveredDialog).toBeHidden();
            await saveCatalogIfDirty(page);

            // 1c. Toggle Enable switch on Mock Beta (disable -> save -> re-enable -> save).
            await ensureDetailsOpen(primaryCard);
            const betaArticle = primaryCard.locator('article.model').filter({ hasText: 'Mock Beta' }).first();
            await expect(betaArticle).toBeVisible();
            const betaSwitch = betaArticle.locator('input[type="checkbox"][role="switch"]');
            await expect(betaSwitch).toBeChecked();
            await betaSwitch.uncheck();
            await saveCatalogIfDirty(page);
            await expect(betaSwitch).not.toBeChecked();

            await betaSwitch.check();
            await saveCatalogIfDirty(page);
            await expect(betaSwitch).toBeChecked();

            // 1d. Set Mock Beta as Default model by raising its priority above Mock Alpha (90 > 50).
            await betaArticle.locator('.wide-actions button').filter({ hasText: /Configure|配置/i }).first().click();
            const configDialog = page.locator('dialog[open]').last();
            await expect(configDialog).toBeVisible();
            await configDialog.locator('details.disclosure > summary').click();
            const priorityInput = configDialog.locator('.advanced-fields input[type="number"]').first();
            await priorityInput.fill('90');
            const configSaveBtn = configDialog.locator('form > .actions button.primary');
            await expect(configSaveBtn).toBeEnabled();
            await configSaveBtn.click();
            await expect(configDialog).toBeHidden();
            await saveCatalogIfDirty(page);

            await ensureDetailsOpen(primaryCard);
            await expect(betaArticle).toContainText(/Default|默认/i);
        }

        // 1e. Edit primary connection display name.
        await ensureDetailsOpen(primaryCard);
        await primaryCard.locator('.endpoint-head .wide-actions button').filter({ hasText: /Edit|编辑/i }).first().click();
        const editEndpointDialog = page.locator('dialog[open]').last();
        await expect(editEndpointDialog).toBeVisible();
        await editEndpointDialog.locator('fieldset input').first().fill('Local Mock Primary Edited');
        await editEndpointDialog.locator('.actions button.primary').click();
        await expect(editEndpointDialog).toBeHidden();
        await saveCatalogIfDirty(page);

        // 1f. Add a secondary API connection, save, then delete it cleanly.
        await page.locator('.models-toolbar button.primary').click();
        const secondaryDialog = page.locator('dialog[open]').last();
        await expect(secondaryDialog).toBeVisible();
        const secInputs = secondaryDialog.locator('.fields input:not([type="checkbox"]):not([type="number"])');
        await secInputs.nth(0).fill('Secondary Disposable Endpoint');
        await secInputs.nth(1).fill(llmEndpoint);
        await secInputs.nth(2).fill('mock-nola-gamma');
        await secInputs.nth(3).fill('Mock Gamma');
        await secondaryDialog.locator('button.primary').click();
        await expect(secondaryDialog).toBeHidden();
        await saveCatalogIfDirty(page);

        const secondaryCard = page.locator('details.endpoint-card').filter({ hasText: 'Secondary Disposable Endpoint' }).first();
        await ensureDetailsOpen(secondaryCard);
        await secondaryCard.locator('.endpoint-head .wide-actions button.danger').click();
        const confirmDialog = page.locator('dialog[open]').last();
        await expect(confirmDialog).toBeVisible();
        await confirmDialog.locator('.actions button').last().click();
        await expect(confirmDialog).toBeHidden();
        await saveCatalogIfDirty(page);
        await expect(page.locator('details.endpoint-card').filter({ hasText: 'Secondary Disposable Endpoint' })).toHaveCount(0);

        // 1g. Reload page and assert persisted state.
        await page.reload({ waitUntil: 'networkidle' });
        const persistedCard = page.locator('details.endpoint-card').filter({ hasText: 'Local Mock Primary Edited' }).first();
        await ensureDetailsOpen(persistedCard);
        await expect(page.locator('details.endpoint-card').filter({ hasText: 'Secondary Disposable Endpoint' })).toHaveCount(0);
        await expect(persistedCard.locator('article.model').filter({ hasText: 'Mock Alpha' })).toBeVisible();
        if (!USE_REAL_LLM) {
            const persistedBeta = persistedCard.locator('article.model').filter({ hasText: 'Mock Beta' }).first();
            await expect(persistedBeta).toBeVisible();
            await expect(persistedBeta.locator('input[type="checkbox"][role="switch"]')).toBeChecked();
            await expect(persistedBeta).toContainText(/Default|默认/i);
        }

        assertZeroErrors('Workflow 1: Model Center');
    });

    // -------------------------------------------------------------------------
    // Workflow 2: Rich Chat Streaming, Math/Markdown, Reasoning & Multi-Turn Persistence
    // -------------------------------------------------------------------------
    test('2. Rich chat streaming, reasoning, math/markdown, multi-turn & conversation CRUD persistence', async () => {
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        await expect(page.locator('#chat-composer')).toBeVisible();

        // Ensure a configured model is selected in the composer toolbar.
        const modelSelect = page.locator('.composer-toolbar select.toolbar-select').first();
        await expect(modelSelect).toBeVisible();
        const selectedVal = await modelSelect.inputValue();
        expect(selectedVal.length).toBeGreaterThan(0);

        // 2a. Send rich prompt and verify streaming reasoning + markdown + math.
        const richPrompt = USE_REAL_LLM
            ? 'Reply with a markdown table, a TypeScript code block, inline math $p = n_s + n_t$, display math $$S_s = \\lambda S_t + UV^\\top$$, and E2E_RICH_STREAM_OK.'
            : '[E2E_RICH_MATH] Show parameter equation $p = n_s + n_t$, matrix decomposition, table, and TypeScript code.';

        await page.locator('#chat-composer').fill(richPrompt);
        await page.locator('.composer-actions button[type="submit"]').click();
        await waitForTurnIdle(page);

        // Verify reasoning disclosure rendered and contains reasoning text.
        if (!USE_REAL_LLM) {
            const reasoningBox = page.locator('details.reasoning-box').first();
            await expect(reasoningBox).toBeVisible();
            await reasoningBox.locator('summary').click();
            await expect(reasoningBox.locator('pre')).toContainText('p = n_s + n_t');
        }

        // Verify rich Markdown elements: heading, table, code block, inline math, display math.
        const assistantBody = page.locator('article.message.assistant .message-body').last();
        await expect(assistantBody.locator('table tbody tr')).toHaveCount(2);
        await expect(assistantBody.locator('.code-block pre code')).toContainText('totalParams');

        const inlineMath = assistantBody.locator('.math-inline').first();
        await expect(inlineMath).toBeVisible();
        await expect(inlineMath).toContainText('p');
        await expect(inlineMath.locator('sub')).toHaveCount(2);

        const displayMath = assistantBody.locator('.math-block').first();
        await expect(displayMath).toBeVisible();
        await expect(displayMath).toHaveAttribute('aria-label', /S_s\s*=\s*\\lambda\s*S_t\s*\+\s*UV\^\\top/);
        await expect(displayMath).toContainText('λ');
        await expect(displayMath).toContainText('⊤');

        await expect(assistantBody).toContainText('E2E_RICH_STREAM_OK');

        // 2b. Reload page and verify full rich history persists.
        await page.reload({ waitUntil: 'networkidle' });
        const reloadedAssistant = page.locator('article.message.assistant .message-body').last();
        await expect(reloadedAssistant).toContainText('E2E_RICH_STREAM_OK');
        await expect(reloadedAssistant.locator('table tbody tr')).toHaveCount(2);
        await expect(reloadedAssistant.locator('.code-block pre code')).toContainText('totalParams');
        await expect(reloadedAssistant.locator('.math-inline').first()).toBeVisible();
        await expect(reloadedAssistant.locator('.math-block').first()).toBeVisible();

        // 2c. Multi-turn conversation (Turn 2) + persistence.
        await page.locator('#chat-composer').fill('[E2E_TURN_2] Confirm second turn continuity.');
        await page.locator('.composer-actions button[type="submit"]').click();
        await waitForTurnIdle(page);
        await expect(page.locator('article.message.assistant')).toHaveCount(2);
        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_TURN_2_OK');

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('article.message.assistant')).toHaveCount(2);
        await expect(page.locator('article.message.assistant').first()).toContainText('E2E_RICH_STREAM_OK');
        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_TURN_2_OK');

        // Capture first conversation ID before creating second conversation.
        const firstConvRow = page.locator('.conversation-list .conversation-row').filter({
            has: page.locator('button.conversation[aria-current="true"]'),
        }).first();
        await expect(firstConvRow).toBeVisible();
        const firstConvId = await firstConvRow.getAttribute('data-conversation-id');
        expect(firstConvId).toBeTruthy();

        // 2d. Create 2nd conversation via New Conversation button & send prompt.
        await createNewConversation(page);

        await page.locator('#chat-composer').fill('[E2E_CONV_2] Start isolated second conversation.');
        await page.locator('.composer-actions button[type="submit"]').click();
        await waitForTurnIdle(page);
        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_CONV_2_OK');

        const secondConvRow = page.locator('.conversation-list .conversation-row').filter({
            has: page.locator('button.conversation[aria-current="true"]'),
        }).first();
        const secondConvId = await secondConvRow.getAttribute('data-conversation-id');
        expect(secondConvId).toBeTruthy();
        expect(secondConvId).not.toBe(firstConvId);

        // 2e. Switch between conversations and verify isolated transcript loading.
        await page.locator(`.conversation-row[data-conversation-id="${firstConvId}"] button.conversation`).click();
        await expect(page.locator('article.message.assistant').first()).toContainText('E2E_RICH_STREAM_OK');
        await expect(page.locator('article.message.assistant')).toHaveCount(2);

        await page.locator(`.conversation-row[data-conversation-id="${secondConvId}"] button.conversation`).click();
        await expect(page.locator('article.message.assistant').first()).toContainText('E2E_CONV_2_OK');
        await expect(page.locator('article.message.assistant')).toHaveCount(1);

        // 2f. Rename conversation 1 & conversation 2 so search is unambiguous.
        const renameRow = async (convId, newTitle) => {
            const row = page.locator(`.conversation-row[data-conversation-id="${convId}"]`);
            await row.hover();
            await row.locator('details.row-actions > summary').click();
            await row.locator('.row-action-buttons button').filter({ hasText: /Rename|重命名/i }).first().click();
            const renameForm = row.locator('form.rename-form');
            await expect(renameForm).toBeVisible();
            await renameForm.locator('input').fill(newTitle);
            await renameForm.locator('button[type="submit"].primary').click();
            await expect(renameForm).toBeHidden();
            await expect(row).toContainText(newTitle);
        };

        await renameRow(firstConvId, 'Alpha Primary Thread');
        await renameRow(secondConvId, 'Beta Secondary Thread');

        // 2g. Search conversations in sidebar.
        const searchInput = page.locator('.conversation-manager .conversation-search input[type="search"]');
        await searchInput.fill('Alpha Primary');
        await expect(page.locator('.conversation-list .conversation-row')).toHaveCount(1);
        await expect(page.locator('.conversation-list .conversation-row').first()).toContainText('Alpha Primary Thread');
        await searchInput.fill('');
        await expect(page.locator('.conversation-list .conversation-row')).toHaveCount(2);

        // 2h. Pin conversation 1 and reload to verify renamed + pinned state persist.
        const row1 = page.locator(`.conversation-row[data-conversation-id="${firstConvId}"]`);
        await row1.hover();
        await row1.locator('details.row-actions > summary').click();
        await row1.locator('.row-action-buttons button').filter({ hasText: /Pin|置顶/i }).first().click();
        await expect(page.locator('.conversation-list h3.group-title').filter({ hasText: /Pinned|置顶/i })).toBeVisible();

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('.conversation-list h3.group-title').filter({ hasText: /Pinned|置顶/i })).toBeVisible();
        await expect(page.locator(`.conversation-row[data-conversation-id="${firstConvId}"]`)).toContainText('Alpha Primary Thread');
        await expect(page.locator(`.conversation-row[data-conversation-id="${secondConvId}"]`)).toContainText('Beta Secondary Thread');

        // 2i. Delete conversation 2 and verify deletion persists after reload.
        const row2 = page.locator(`.conversation-row[data-conversation-id="${secondConvId}"]`);
        await row2.hover();
        await row2.locator('details.row-actions > summary').click();
        await row2.locator('.row-action-buttons button.danger').click();
        const delConfirm = page.locator('dialog[open]').last();
        await expect(delConfirm).toBeVisible();
        await delConfirm.locator('.actions button').last().click();
        await expect(delConfirm).toBeHidden();
        await expect(page.locator(`.conversation-row[data-conversation-id="${secondConvId}"]`)).toHaveCount(0);

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator(`.conversation-row[data-conversation-id="${secondConvId}"]`)).toHaveCount(0);
        await expect(page.locator(`.conversation-row[data-conversation-id="${firstConvId}"]`)).toBeVisible();

        assertZeroErrors('Workflow 2: Rich Chat & Conversation CRUD');
    });

    // -------------------------------------------------------------------------
    // Workflow 3: Permission Modes (Strictly 2 Modes) & Live Approval
    // -------------------------------------------------------------------------
    test('3. Permission modes strictly expose cautious & allow_all, persist across reload, and handle live tool approval/denial', async () => {
        // 3a. Verify Settings -> Agent Access also exposes strictly 2 permission modes (no accept_edits).
        await page.goto(`${baseUrl}/settings?section=agent`, { waitUntil: 'networkidle' });
        const settingsPermSelect = page.locator('section.agent-access select').first();
        await expect(settingsPermSelect).toBeVisible();
        const settingsPermValues = await settingsPermSelect.locator('option').evaluateAll(opts => opts.map(o => o.value));
        expect(settingsPermValues).toEqual(['cautious', 'allow_all']);
        expect(settingsPermValues).not.toContain('accept_edits');

        // 3b. Verify Chat composer permission selector exposes strictly 2 modes.
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        const activeConvBtn = page.locator('.conversation-list .conversation-row button.conversation').first();
        await activeConvBtn.click();
        await expect(page.locator('#chat-composer')).toBeVisible();

        const chatPermSelect = page.locator('.composer-toolbar select.toolbar-select').nth(1);
        await expect(chatPermSelect).toBeVisible();
        const chatPermValues = await chatPermSelect.locator('option').evaluateAll(opts => opts.map(o => o.value));
        expect(chatPermValues).toEqual(['cautious', 'allow_all']);
        expect(chatPermValues).not.toContain('accept_edits');

        // 3c. Switch cautious -> allow_all -> reload -> verify persisted -> switch back -> reload -> verify persisted.
        const patchAllowPromise = page.waitForResponse(
            res =>
                res.request().method() === 'PATCH' &&
                /\/api\/chat\/conversations\/[^/]+\/configuration$/.test(new URL(res.url()).pathname),
        );
        await chatPermSelect.selectOption('allow_all');
        expect((await patchAllowPromise).status()).toBe(200);

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('.composer-toolbar select.toolbar-select').nth(1)).toHaveValue('allow_all');

        const patchCautiousPromise = page.waitForResponse(
            res =>
                res.request().method() === 'PATCH' &&
                /\/api\/chat\/conversations\/[^/]+\/configuration$/.test(new URL(res.url()).pathname),
        );
        await page.locator('.composer-toolbar select.toolbar-select').nth(1).selectOption('cautious');
        expect((await patchCautiousPromise).status()).toBe(200);

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('.composer-toolbar select.toolbar-select').nth(1)).toHaveValue('cautious');

        // 3d. Trigger a tool call requiring approval under cautious mode -> Deny -> verify file NOT written.
        const deniedFile = path.join(tempWorkspace, 'approval_denied_probe.txt');
        await page.locator('#chat-composer').fill(
            `[E2E_APPROVAL_TOOL] Write APPROVED_WRITE_OK to TARGET_PATH=${deniedFile} using write_file.`,
        );
        await page.locator('.composer-actions button[type="submit"]').click();

        const permCard = page.locator('.permission-card');
        await expect(permCard).toBeVisible({ timeout: 25_000 });
        await expect(permCard.locator('.permission-tool-badge')).toContainText('write_file');
        await permCard.locator('.interactive-actions button.danger').click();
        await waitForTurnIdle(page);

        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_APPROVAL_DENIED_OK');
        expect(await fileExists(deniedFile)).toBe(false);

        // 3e. Trigger a second tool call requiring approval under cautious mode -> Approve once -> verify file IS written.
        const allowedFile = path.join(tempWorkspace, 'approval_allowed_probe.txt');
        await page.locator('#chat-composer').fill(
            `[E2E_APPROVAL_TOOL] Write APPROVED_WRITE_OK to TARGET_PATH=${allowedFile} using write_file.`,
        );
        await page.locator('.composer-actions button[type="submit"]').click();

        await expect(permCard).toBeVisible({ timeout: 25_000 });
        await expect(permCard.locator('.permission-tool-badge')).toContainText('write_file');
        await permCard.locator('.interactive-actions button.primary').click();
        await waitForTurnIdle(page);

        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_APPROVAL_ALLOWED_OK');
        expect(await fileExists(allowedFile)).toBe(true);
        const writtenContent = (await fs.readFile(allowedFile, 'utf8')).trim();
        expect(writtenContent).toBe('APPROVED_WRITE_OK');

        assertZeroErrors('Workflow 3: Permission Modes & Live Approval');
    });

    // -------------------------------------------------------------------------
    // Workflow 4: Mid-Stream Stop / Cancel & Follow-Up
    // -------------------------------------------------------------------------
    test('4. Mid-stream Stop / Cancel cleanly exits running state and follow-up succeeds', async () => {
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        await createNewConversation(page);

        // Start a slow streaming turn.
        await page.locator('#chat-composer').fill('[E2E_SLOW_STREAM] Emit slow incremental chunks for cancellation test.');
        await page.locator('.composer-actions button[type="submit"]').click();

        const stopBtn = page.locator('.composer-actions button.danger');
        await expect(stopBtn).toBeVisible({ timeout: 15_000 });

        if (!USE_REAL_LLM) {
            await expect(page.locator('article.message.assistant').last()).toContainText('SLOW_STREAM_STARTED', {
                timeout: 15_000,
            });
        }

        // Click Stop button mid-stream.
        await stopBtn.click();

        // Verify UI exits running state cleanly and Send button returns.
        await expect(stopBtn).toBeHidden({ timeout: 15_000 });
        await expect(page.locator('.composer-actions button[type="submit"]')).toBeVisible();
        await expect(page.locator('.composer-wrap .notice.error')).toHaveCount(0);

        // Immediately send a follow-up message in the same conversation and verify it succeeds cleanly.
        await page.locator('#chat-composer').fill('[E2E_FOLLOWUP] Verify clean follow-up after stop.');
        await page.locator('.composer-actions button[type="submit"]').click();
        await waitForTurnIdle(page);

        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_FOLLOWUP_OK');

        // Reload and verify follow-up persisted.
        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_FOLLOWUP_OK');

        assertZeroErrors('Workflow 4: Mid-Stream Cancel & Follow-Up');
    });

    // -------------------------------------------------------------------------
    // Workflow 5: All Navigation Pages & Core Interactive Controls
    // -------------------------------------------------------------------------
    test('5. All navigation pages & interactive controls: Sidebar, Experts/Rooms, Memory CRUD, Data Sources, Skills, Automations, Settings', async () => {
        // 5a. Chat Sidebar resource navigation (verify all sidebar nav entries render and open cleanly).
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        const listCategories = page.locator('.list-categories button');
        await expect(listCategories).toHaveCount(2);
        await listCategories.nth(1).click();
        const resourceNav = page.locator('.resource-navigation');
        await expect(resourceNav).toBeVisible();
        for (const href of ['/today', '/activity', '/automations', '/data-sources', '/models', '/capabilities']) {
            await expect(resourceNav.locator(`a[href="${href}"]`)).toBeVisible();
        }
        // Switch back to conversations list category.
        await listCategories.nth(0).click();
        await expect(page.locator('.conversation-manager')).toBeVisible();

        // 5b. Models page (API + Local tabs).
        await page.goto(`${baseUrl}/models`, { waitUntil: 'networkidle' });
        const tabApi = page.locator('#tab-api, [data-workspace-id="tab-api"]').first();
        const tabLocal = page.locator('#tab-local, [data-workspace-id="tab-local"]').first();
        const panelApi = page.locator('#panel-api, [data-workspace-id="panel-api"]').first();
        const panelLocal = page.locator('#panel-local, [data-workspace-id="panel-local"]').first();
        await expect(tabApi).toBeVisible();
        await tabLocal.click();
        await expect(panelLocal).toBeVisible();
        await tabApi.click();
        await expect(panelApi).toBeVisible();

        // 5c. Experts / Rooms page: create Expert, create Room, reload & verify, then mention Expert in Chat.
        await page.goto(`${baseUrl}/experts`, { waitUntil: 'networkidle' });
        const expertForm = page.locator('.mgr-standalone form.mgr-form-col');
        await expect(expertForm).toBeVisible();
        await expertForm.locator('input[type="text"]').nth(0).fill('E2E_Specialist');
        await expertForm.locator('input[type="text"]').nth(1).fill('Automated verification expert');
        await expertForm.locator('textarea').fill('Provide concise verification summaries.');
        await expertForm.locator('button[type="submit"].primary').click();
        await expect(page.locator('.mgr-standalone .entity-list')).toContainText('@E2E_Specialist');

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('.mgr-standalone .entity-list')).toContainText('@E2E_Specialist');

        // Test selecting the created Expert from Chat's @ Experts picker.
        await page.goto(`${baseUrl}/chat`, { waitUntil: 'networkidle' });
        await createNewConversation(page);
        await page.locator('.composer-toolbar button[aria-label]').filter({ hasText: /@/ }).first().click();
        const expertPopup = page.locator('.expert-popup');
        await expect(expertPopup).toBeVisible();
        await expertPopup.locator('button.expert-option', { hasText: '@E2E_Specialist' }).click();
        await expect(page.locator('.attachment-chips .expert-chip')).toContainText('@E2E_Specialist');
        await page.locator('#chat-composer').fill('[E2E_EXPERT_CHECK] Verify expert mention turn.');
        await page.locator('.composer-actions button[type="submit"]').click();
        await waitForTurnIdle(page);
        await expect(page.locator('article.message.assistant').last()).toContainText('E2E_EXPERT_OK');

        // 5d. Memory CRUD: Create -> Reload -> Edit/Revise -> Reload -> Delete -> Reload.
        await page.goto(`${baseUrl}/data-sources?tab=memory`, { waitUntil: 'networkidle' });
        const createMemBtn = page.locator('.memory-center .memory-actions button.primary');
        await expect(createMemBtn).toBeEnabled();
        await createMemBtn.click();

        const memCreateDialog = page.locator('dialog[open]').last();
        await expect(memCreateDialog).toBeVisible();
        const initialMemoryText = 'E2E Memory Record: Prefer deterministic browser verification.';
        await memCreateDialog.locator('textarea').nth(0).fill(initialMemoryText);
        await memCreateDialog.locator('textarea').nth(1).fill('Deterministic browser E2E preference');
        await memCreateDialog.locator('button[type="submit"].primary').click();
        await expect(memCreateDialog).toBeHidden();

        await expect(page.locator('.memory-center article').filter({ hasText: initialMemoryText })).toBeVisible();
        await page.reload({ waitUntil: 'networkidle' });
        const memArticle = page.locator('.memory-center article').filter({ hasText: initialMemoryText }).first();
        await expect(memArticle).toBeVisible();

        // Revise memory record.
        await memArticle.locator('.actions button:not(.danger)').filter({ hasText: /Revise|Edit|修订|编辑/i }).first().click();
        const memEditDialog = page.locator('dialog[open]').last();
        await expect(memEditDialog).toBeVisible();
        const revisedMemoryText = 'E2E Memory Record Revised: Always assert zero console and 5xx errors.';
        await memEditDialog.locator('textarea').nth(0).fill(revisedMemoryText);
        await memEditDialog.locator('button[type="submit"].primary').click();
        await expect(memEditDialog).toBeHidden();

        await page.reload({ waitUntil: 'networkidle' });
        const revisedArticle = page.locator('.memory-center article').filter({ hasText: revisedMemoryText }).first();
        await expect(revisedArticle).toBeVisible();

        // Delete memory record.
        await revisedArticle.locator('.actions button.danger').click();
        const memDelConfirm = page.locator('dialog[open]').last();
        await expect(memDelConfirm).toBeVisible();
        await memDelConfirm.locator('.actions button').last().click();
        await expect(memDelConfirm).toBeHidden();
        await expect(page.locator('.memory-center article').filter({ hasText: revisedMemoryText })).toHaveCount(0);

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('.memory-center article').filter({ hasText: revisedMemoryText })).toHaveCount(0);

        // 5e. Data Sources: create local JSON file Data Source & verify reload persistence.
        await page.goto(`${baseUrl}/data-sources?tab=sources`, { waitUntil: 'networkidle' });
        const addSourceBtn = page.locator('header.page-header .page-actions button.primary');
        await expect(addSourceBtn).toBeEnabled();
        await addSourceBtn.click();
        const sourceDialog = page.locator('dialog[open]').last();
        await expect(sourceDialog).toBeVisible();
        await sourceDialog.locator('fieldset input').nth(0).fill('Workflow Local JSON');
        await sourceDialog.locator('fieldset select').nth(0).selectOption('localFile');
        await sourceDialog.locator('fieldset input').nth(2).fill(localDataSourcePath);
        await sourceDialog.locator('button.primary').click();
        await expect(sourceDialog).toBeHidden();

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('article.source').filter({ hasText: 'Workflow Local JSON' })).toBeVisible();

        // 5f. Skills & MCP (/capabilities): create a custom Skill, reload & verify, then inspect MCP tab.
        await page.goto(`${baseUrl}/capabilities`, { waitUntil: 'networkidle' });
        const addCapBtn = page.locator('header.page-header .actions button.primary');
        await expect(addCapBtn).toBeEnabled();
        await addCapBtn.click();
        const skillDialog = page.locator('dialog[open]').last();
        await expect(skillDialog).toBeVisible();
        await skillDialog.locator('fieldset input:not([type="checkbox"])').first().fill('E2E Workflow Skill');
        await skillDialog.locator('fieldset textarea').nth(0).fill('Skill created by Playwright E2E suite.');
        await skillDialog.locator('fieldset textarea').nth(1).fill('# E2E Workflow Skill\nValidate all UI routes.');
        await skillDialog.locator('button[type="submit"].primary').click();
        await expect(skillDialog).toBeHidden();

        await page.reload({ waitUntil: 'networkidle' });
        await expect(page.locator('#capability-panel, [data-workspace-id="capability-panel"]').first()).toContainText(
            'E2E Workflow Skill',
        );

        await page.locator('#capability-mcp, [data-workspace-id="capability-mcp"]').first().click();
        await expect(
            page.locator('#capability-mcp[aria-selected="true"], [data-workspace-id="capability-mcp"][aria-selected="true"]').first(),
        ).toBeVisible();

        // 5g. Automations (/automations): create automation, reload, verify pause/resume toggle.
        await page.goto(`${baseUrl}/automations`, { waitUntil: 'networkidle' });
        await page.locator('header.page-header button.primary').click();
        const autoDialog = page.locator('dialog.automation-dialog[open]');
        await expect(autoDialog).toBeVisible();
        const autoInputs = autoDialog.locator('.editor-body > fieldset > label > input');
        await autoInputs.nth(0).fill('E2E Nightly Check');
        await autoDialog.locator('.editor-body > fieldset > label > textarea').first().fill('Summarize test status.');
        await autoInputs.nth(1).fill(tempWorkspace);
        await autoDialog.locator('.editor-body > fieldset > label > select').nth(1).selectOption('cautious');
        await autoDialog.locator('footer button[type="submit"].primary').click();
        await expect(autoDialog).toBeHidden();

        await page.reload({ waitUntil: 'networkidle' });
        const autoRow = page.locator('.automation-row').filter({ hasText: 'E2E Nightly Check' }).first();
        await expect(autoRow).toBeVisible();
        const autoToggleBtn = autoRow.locator('.buttons > button').nth(1);
        await autoToggleBtn.click();
        await expect(autoRow.locator('.badge[data-status="disabled"]')).toBeVisible();
        await autoToggleBtn.click();
        await expect(autoRow.locator('.badge[data-status="enabled"]')).toBeVisible();

        // 5h. Settings (/settings): inspect every category tab and toggle appearance cleanly.
        await page.goto(`${baseUrl}/settings`, { waitUntil: 'networkidle' });
        const appearanceSelect = page.locator('.preferences-card select').last();
        await expect(appearanceSelect).toBeVisible();
        await appearanceSelect.selectOption('dark');
        await expect.poll(() => page.evaluate(() => document.documentElement.dataset.appearance)).toBe('dark');
        await appearanceSelect.selectOption('light');
        await expect.poll(() => page.evaluate(() => document.documentElement.dataset.appearance)).toBe('light');

        for (const section of ['agent', 'providers', 'data', 'general']) {
            const tab = page.locator(`#settings-tab-${section}, [data-workspace-id="settings-tab-${section}"]`).first();
            if (await tab.isVisible().catch(() => false)) {
                await tab.click();
                await expect(
                    page.locator(`#settings-panel-${section}, [data-workspace-id="settings-panel-${section}"]`).first(),
                ).toBeVisible();
            }
        }

        // Also smoke-check Today and Activity routes for zero blank crashes.
        await page.goto(`${baseUrl}/today`, { waitUntil: 'networkidle' });
        await expect(page.locator('body')).not.toBeEmpty();
        await page.goto(`${baseUrl}/activity`, { waitUntil: 'networkidle' });
        await expect(page.locator('body')).not.toBeEmpty();

        assertZeroErrors('Workflow 5: All Navigation Pages & Interactive Controls');
    });
});
