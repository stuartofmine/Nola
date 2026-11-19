// SPDX-License-Identifier: Apache-2.0
// Full Svelte 5 Web visual, contrast, overflow, accessibility, A2UI, and HTTP round-trip regression against an isolated real `nola web` Host.
const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const {spawn, spawnSync} = require('node:child_process');
const fs = require('node:fs');
const net = require('node:net');
const os = require('node:os');
const path = require('node:path');
const {artifactPath, browserOptions} = require('./test_environment.cjs');

const repoRoot = path.resolve(__dirname, '../../..');
const webRoot = path.join(repoRoot, 'apps/web');
const frontendRoot = path.join(webRoot, 'frontend');
const uiIndex = path.join(webRoot, 'app/static/ui/index.html');

const EXPECTED_ROUTES = [
    'today', 'conversations', 'chat', 'activity', 'results', 'dashboard',
    'data-sources', 'scheduled-actions', 'models', 'model-test',
    'capabilities', 'workspace-review', 'settings', 'login',
];

const SETTINGS_CATEGORIES = ['general', 'providers', 'agent', 'connections', 'data', 'diagnostics'];

function verifyRouteParity() {
    const routesSource = fs.readFileSync(path.join(frontendRoot, 'src/lib/pageRoutes.ts'), 'utf8');
    const actualRoutes = [...routesSource.matchAll(/'\/([^']+)':/g)].map(m => m[1]).sort();
    assert.deepEqual(actualRoutes, [...EXPECTED_ROUTES].sort(), 'Route matrix must cover every route in pageRoutes.ts');
}

function ensureUiBuilt() {
    if (!fs.existsSync(uiIndex)) {
        const res = spawnSync('npm', ['--prefix', frontendRoot, 'run', 'build'], {stdio: 'inherit'});
        assert.equal(res.status, 0, 'Frontend build failed');
    }
}

function resolveNolaBinary() {
    const candidates = [
        process.env.NOLA_CLI_PATH,
        path.join(repoRoot, 'target/debug/nola'),
        path.join(repoRoot, 'target/release/nola'),
    ].filter(Boolean);
    const found = candidates.find(candidate => fs.existsSync(candidate));
    if (!found) {
        throw new Error(`No nola binary found in ${candidates.join(', ')}; build with: cargo build -p nola_cli --bin nola`);
    }
    return found;
}

function allocatePort() {
    return new Promise((resolve, reject) => {
        const server = net.createServer();
        server.unref();
        server.once('error', reject);
        server.listen(0, '127.0.0.1', () => {
            const addr = server.address();
            if (!addr || typeof addr === 'string') {
                server.close();
                reject(new Error('Could not allocate ephemeral loopback port'));
                return;
            }
            const port = addr.port;
            server.close(err => err ? reject(err) : resolve(port));
        });
    });
}

async function waitForHealth(origin, child, logs) {
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
        if (child.exitCode !== null) {
            throw new Error(`nola web exited early (code=${child.exitCode}):\n${logs.join('')}`);
        }
        try {
            const res = await fetch(`${origin}/api/health`, {signal: AbortSignal.timeout(1000)});
            if (res.ok) return;
        } catch {
            // Host is still starting.
        }
        await new Promise(resolve => setTimeout(resolve, 200));
    }
    throw new Error(`Timed out waiting for ${origin}/api/health:\n${logs.join('')}`);
}

async function stopProcess(child) {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    child.kill('SIGTERM');
    await Promise.race([
        new Promise(resolve => child.once('exit', resolve)),
        new Promise(resolve => setTimeout(() => {
            if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
            resolve();
        }, 3000)),
    ]);
}

async function waitForUiSettled(page) {
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

async function inspectView(page, label, scheme, width) {
    await waitForUiSettled(page);
    const expectedBg = scheme === 'dark' ? 'rgb(19, 19, 18)' : 'rgb(247, 246, 243)';
    const metrics = await page.evaluate(() => {
        const parseColor = (raw) => {
            if (!raw || raw === 'transparent') return null;
            const match = raw.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)/);
            if (!match) return null;
            const a = match[4] === undefined ? 1 : Number(match[4]);
            return [Number(match[1]), Number(match[2]), Number(match[3]), a];
        };
        const composite = (fg, bg) => {
            const a = fg[3];
            return [
                fg[0] * a + bg[0] * (1 - a),
                fg[1] * a + bg[1] * (1 - a),
                fg[2] * a + bg[2] * (1 - a),
                1,
            ];
        };
        const luminance = ([r, g, b]) => {
            const [lr, lg, lb] = [r, g, b].map(c => {
                const s = c / 255;
                return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
            });
            return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
        };
        const contrastRatio = (c1, c2) => {
            const l1 = luminance(c1);
            const l2 = luminance(c2);
            return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
        };
        const effectiveBg = (el) => {
            const layers = [];
            let cur = el;
            while (cur && cur instanceof Element) {
                const parsed = parseColor(getComputedStyle(cur).backgroundColor);
                if (parsed && parsed[3] > 0) layers.push(parsed);
                if (parsed && parsed[3] >= 0.999) break;
                cur = cur.parentElement;
            }
            const bodyParsed = parseColor(getComputedStyle(document.body).backgroundColor) || [247, 246, 243, 1];
            let bg = [bodyParsed[0], bodyParsed[1], bodyParsed[2], 1];
            for (let i = layers.length - 1; i >= 0; i--) {
                bg = composite(layers[i], bg);
            }
            return bg;
        };
        const isVisible = (el) => {
            if (!(el instanceof HTMLElement)) return false;
            if (el.closest('[hidden], [inert], .sr-only')) return false;
            const style = getComputedStyle(el);
            if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
            const rect = el.getBoundingClientRect();
            return rect.width > 0.5 && rect.height > 0.5 && el.getClientRects().length > 0;
        };

        const bodyBg = getComputedStyle(document.body).backgroundColor;
        const docOverflow = document.documentElement.scrollWidth > window.innerWidth + 1;

        const clippedElements = [];
        for (const el of document.querySelectorAll('section.card, div.card, dialog[open], button')) {
            if (!isVisible(el)) continue;
            if (el.closest('.tabs, .data-tabs, pre, code, table, .mobile-primary-navigation')) continue;
            if (el.tagName === 'BUTTON' && el.scrollWidth > el.clientWidth + 2) {
                clippedElements.push({tag: el.tagName, text: el.textContent.trim().slice(0, 60), scrollWidth: el.scrollWidth, clientWidth: el.clientWidth});
            } else if (el.tagName === 'DIALOG' && el.scrollWidth > el.clientWidth + 2) {
                clippedElements.push({tag: 'DIALOG', text: el.getAttribute('aria-label') || '', scrollWidth: el.scrollWidth, clientWidth: el.clientWidth});
            }
            const rect = el.getBoundingClientRect();
            if (el.tagName !== 'BUTTON' && (rect.right > window.innerWidth + 2 || rect.left < -2)) {
                clippedElements.push({tag: el.tagName, className: el.className, left: rect.left, right: rect.right, innerWidth: window.innerWidth});
            }
        }

        const controls = [...document.querySelectorAll('button, a, input, select, textarea, summary')].filter(isVisible);
        const unnamed = controls.filter(el => {
            const labelledBy = el.getAttribute('aria-labelledby');
            const labelText = labelledBy
                ? labelledBy.split(/\s+/).map(id => document.getElementById(id)?.textContent?.trim() || '').join('').trim()
                : '';
            const hasNativeLabel = 'labels' in el && el.labels && el.labels.length > 0 && [...el.labels].some(l => l.textContent.trim());
            return !el.textContent.trim()
                && !el.getAttribute('aria-label')?.trim()
                && !labelText
                && !el.getAttribute('title')?.trim()
                && !hasNativeLabel
                && !el.getAttribute('placeholder')?.trim()
                && !el.querySelector('img[alt]:not([alt=""])');
        }).map(el => el.outerHTML.slice(0, 160));

        const lowContrast = [];
        const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        let node;
        while ((node = walker.nextNode())) {
            if (!node.nodeValue || !node.nodeValue.trim()) continue;
            const parent = node.parentElement;
            if (!parent || !isVisible(parent)) continue;
            const style = getComputedStyle(parent);
            if (parseFloat(style.fontSize) < 1) continue;
            const fgRaw = parseColor(style.color);
            if (!fgRaw || fgRaw[3] === 0) continue;
            const bg = effectiveBg(parent);
            const fg = composite([fgRaw[0], fgRaw[1], fgRaw[2], fgRaw[3] * Number(style.opacity)], bg);
            const ratio = contrastRatio(fg, bg);
            if (ratio < 4.5) {
                lowContrast.push({
                    tag: parent.tagName,
                    className: String(parent.className || ''),
                    text: node.nodeValue.trim().slice(0, 50),
                    ratio: Number(ratio.toFixed(2)),
                    fg: style.color,
                    bg: `rgb(${Math.round(bg[0])}, ${Math.round(bg[1])}, ${Math.round(bg[2])})`,
                });
            }
        }

        return {bodyBg, docOverflow, clippedElements, unnamed, lowContrast, controlCount: controls.length};
    });

    assert.equal(metrics.bodyBg, expectedBg, `${label} (${scheme}, ${width}px): unexpected body bg ${metrics.bodyBg}`);
    assert.equal(metrics.docOverflow, false, `${label} (${scheme}, ${width}px): document horizontal overflow`);
    assert.deepEqual(metrics.clippedElements, [], `${label} (${scheme}, ${width}px): clipped/overflowing elements`);
    assert.deepEqual(metrics.unnamed, [], `${label} (${scheme}, ${width}px): unnamed controls`);
    assert.deepEqual(metrics.lowContrast, [], `${label} (${scheme}, ${width}px): WCAG AA contrast < 4.5:1`);
    return metrics;
}

async function verifyFirstConnection(page, origin, tokenPath) {
    await page.goto(`${origin}/login?next=/chat`);
    const token = page.locator('#host-token');
    const connect = page.getByRole('button', {name: 'Connect', exact: true});
    await page.locator('.login-card summary').click();
    assert.equal(await page.locator('.login-card details code').innerText(), '~/.nola/web/mobile-access-token');
    await token.fill('short');
    assert.equal(await token.getAttribute('aria-invalid'), 'false');
    await connect.click();
    assert.equal(await token.getAttribute('aria-invalid'), 'true');
    await token.fill('x'.repeat(32));
    const failed = page.waitForResponse(r => r.url().endsWith('/api/web/login'));
    await connect.click();
    assert.equal((await failed).status(), 401);
    await page.locator('[role="alert"]').waitFor();
    await token.fill(fs.readFileSync(tokenPath, 'utf8').trim());
    const accepted = page.waitForResponse(r => r.url().endsWith('/api/web/login'));
    await connect.click();
    assert.equal((await accepted).status(), 200);
    await page.locator('#chat-composer').waitFor();
}

async function verifyModelSetupNavigation(page, origin) {
    const name = 'First run test model';
    const result = await page.request.post(`${origin}/api/model-profiles`, {data: {
        expectedRevisions: {}, endpointName: name, displayName: name, modelName: 'first-run',
        baseURL: 'https://example.invalid/v1', wireProtocol: 'responses', priority: 50,
        capabilities: ['chat'], executionCapabilities: {
            toolCalls: false, reasoning: false, imageInput: false, samplingParameters: false,
        },
    }});
    assert.equal(result.status(), 201);
    await page.goto(`${origin}/settings?section=providers`);
    await page.getByRole('link', {name, exact: true}).click();
    await page.locator('dialog:modal').waitFor();
    assert.ok((await page.locator('dialog:modal').getAttribute('aria-label')).includes(name));
    await page.keyboard.press('Escape');
    await page.locator('[role="tab"][id$="tab-local"]:visible').click();
    await page.reload();
    await page.locator('[role="tab"][id$="tab-local"][aria-selected="true"]').waitFor();
}

async function runWorkflowRoundTrips(page, origin) {
    // 1. Create a real conversation via UI -> verify persisted in backend & /conversations
    await page.goto(`${origin}/chat`, {waitUntil: 'networkidle'});
    const newConvBtn = page.locator('button.new-conversation:visible').first();
    await newConvBtn.waitFor();
    const [convResp] = await Promise.all([
        page.waitForResponse(r => r.url().endsWith('/api/chat/conversations') && r.request().method() === 'POST'),
        newConvBtn.click(),
    ]);
    assert.ok(convResp.ok(), `Expected conversation creation 2xx, got ${convResp.status()}`);
    const created = await convResp.json();
    const convId = created.conversationId || created.conversation?.conversationId;
    assert.ok(convId, 'Created conversation must return conversationId');
    await page.waitForFunction(id => new URL(window.location.href).searchParams.get('conversation') === id, convId);
    await page.locator('#chat-composer').waitFor();
    await page.locator('#chat-composer').fill('Roundtrip draft text');
    assert.equal(await page.locator('#chat-composer').inputValue(), 'Roundtrip draft text');

    const listRes = await page.request.get(`${origin}/api/chat/conversations`);
    assert.ok(listRes.ok());
    const listData = await listRes.json();
    assert.ok(listData.conversations.some(c => c.conversationId === convId), 'Created conversation must be listed by backend');

    // 2. Capability editor real HTTP round-trip: create -> reload -> verify -> delete
    await page.goto(`${origin}/capabilities`, {waitUntil: 'networkidle'});
    await page.locator('header.page-header .actions button.primary:visible').click();
    const capDialog = page.locator('dialog:modal');
    await capDialog.waitFor();
    const skillName = 'Workflow Audit Skill';
    const inputs = capDialog.locator('fieldset input:not([type="checkbox"])');
    await inputs.first().fill(skillName);
    await capDialog.locator('fieldset textarea').nth(0).fill('Verifies real capability persistence over HTTP.');
    await capDialog.locator('fieldset textarea').nth(1).fill('# Workflow Audit\nCheck skill persistence.');
    const [saveSkillResp] = await Promise.all([
        page.waitForResponse(r => r.url().endsWith('/api/agent/settings') && r.request().method() === 'POST'),
        capDialog.locator('form .actions button.primary').click(),
    ]);
    assert.ok(saveSkillResp.ok(), `Expected capability save 2xx, got ${saveSkillResp.status()}`);
    await capDialog.waitFor({state: 'hidden'});
    await page.reload({waitUntil: 'networkidle'});
    const skillCard = page.locator('button.capability:visible', {hasText: skillName});
    await skillCard.waitFor();

    // Clean up created skill via UI delete + confirmation dialog
    await skillCard.click();
    await capDialog.waitFor();
    await capDialog.locator('button.danger').click();
    const confirmDialog = page.locator('dialog:modal').last();
    await Promise.all([
        page.waitForResponse(r => r.url().endsWith('/api/agent/settings') && r.request().method() === 'POST'),
        confirmDialog.locator('button.danger, button.primary').last().click(),
    ]);
    await page.waitForFunction(name => !document.body.innerText.includes(name), skillName);

    // 3. Settings persistence round-trip: appearance + backend memory default
    await page.goto(`${origin}/settings?section=general`, {waitUntil: 'networkidle'});
    const appearanceSelect = page.locator('[id$="settings-panel-general"] .preferences-card select').nth(1);
    await appearanceSelect.waitFor();
    await appearanceSelect.selectOption('dark');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.appearance), 'dark');
    await page.reload({waitUntil: 'networkidle'});
    assert.equal(await page.evaluate(() => document.documentElement.dataset.appearance), 'dark');
    // Reset to system so media colorScheme controls Light/Dark in visual matrix.
    await page.locator('[id$="settings-panel-general"] .preferences-card select').nth(1).selectOption('system');

    await page.locator('[role="tab"][id$="settings-tab-agent"]:visible').click();
    const memorySwitch = page.locator('[id$="settings-panel-agent"] input[role="switch"]').first();
    await memorySwitch.waitFor();
    const initialChecked = await memorySwitch.isChecked();
    const [memResp] = await Promise.all([
        page.waitForResponse(r => r.url().includes('/api/assistant/memory-default') && r.request().method() === 'PUT'),
        memorySwitch.click(),
    ]);
    assert.ok(memResp.ok(), `Expected memory default save 2xx, got ${memResp.status()}`);
    await page.reload({waitUntil: 'networkidle'});
    assert.equal(await page.locator('[id$="settings-panel-agent"] input[role="switch"]').first().isChecked(), !initialChecked);
    // Restore initial state
    await Promise.all([
        page.waitForResponse(r => r.url().includes('/api/assistant/memory-default') && r.request().method() === 'PUT'),
        page.locator('[id$="settings-panel-agent"] input[role="switch"]').first().click(),
    ]);

    return convId;
}

async function verifyA2uiSurface(context, origin, convId, scheme, width, hostId) {
    const fixture = JSON.parse(
        fs.readFileSync(path.join(repoRoot, 'tests/fixtures/a2ui/form_interaction_v0_9_1.json'), 'utf8'),
    );
    const snapshot = structuredClone(fixture.initial_snapshot);
    snapshot.identity.owner.authority_id = hostId;
    snapshot.identity.owner.conversation_id = convId;
    snapshot.identity.owner.context_id = convId;
    snapshot.identity.owner.message_id = 'a2ui-msg-1';
    snapshot.identity.owner.part_id = 'a2ui-part-1';

    const msgRoute = `**/api/chat/conversations/${encodeURIComponent(convId)}/messages*`;
    const surfaceRoute = `**/api/chat/conversations/${encodeURIComponent(convId)}/ui-surface-changes`;
    const handleMessages = async route => route.fulfill({
        json: {
            sourceHostId: hostId,
            conversationProjection: {
                contract_version: 1,
                conversation_id: convId,
                revision: 2,
                messages: [{
                    message_id: 'a2ui-msg-1',
                    role: 'assistant',
                    state: 'completed',
                    revision: 1,
                    created_at_ms: Date.now(),
                    completed_at_ms: Date.now(),
                    parts: [{
                        kind: 'ui_surface',
                        part_id: 'a2ui-part-1',
                        revision: 1,
                        surface: {identity: snapshot.identity, revision: snapshot.revision},
                    }],
                }],
            },
            messageWindow: {projectionRevision: 2, eventSequence: 2, before: null, hasMore: false, totalCount: 1},
        },
    });
    const handleSurface = async route => route.fulfill({
        json: {update: {type: 'snapshot', snapshot}},
    });

    await context.route(msgRoute, handleMessages);
    await context.route(surfaceRoute, handleSurface);
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    try {
        await page.goto(`${origin}/chat?conversation=${encodeURIComponent(convId)}`, {waitUntil: 'networkidle'});
        const surface = page.locator('.nola-generative-surface');
        await surface.waitFor();
        await surface.getByText('Shared A2UI form').waitFor();
        const tokens = await page.evaluate(() => {
            const rootStyle = getComputedStyle(document.documentElement);
            const surfaceEl = document.querySelector('.nola-generative-surface');
            const surfStyle = getComputedStyle(surfaceEl);
            return {
                rootSurface: rootStyle.getPropertyValue('--a2ui-color-surface').trim(),
                rootText: rootStyle.getPropertyValue('--a2ui-color-text').trim(),
                rootPrimary: rootStyle.getPropertyValue('--a2ui-color-primary').trim(),
                rootDanger: rootStyle.getPropertyValue('--a2ui-color-danger').trim(),
                surfBg: surfStyle.getPropertyValue('--a2ui-color-surface').trim(),
            };
        });
        const expandHex = (value) => /^#[0-9a-f]{3}$/i.test(value)
            ? '#' + [...value.slice(1)].map(c => c + c).join('').toLowerCase()
            : value.toLowerCase();
        assert.equal(expandHex(tokens.rootSurface), scheme === 'dark' ? '#1b1b19' : '#ffffff');
        assert.equal(expandHex(tokens.rootText), scheme === 'dark' ? '#f2f0eb' : '#181817');
        assert.equal(expandHex(tokens.rootPrimary), scheme === 'dark' ? '#f2f0eb' : '#181817');
        assert.equal(expandHex(tokens.rootDanger), scheme === 'dark' ? '#efaaa2' : '#ad3d34');
        assert.equal(expandHex(tokens.surfBg), expandHex(tokens.rootSurface));
        await inspectView(page, 'a2ui-surface', scheme, width);
        assert.deepEqual(errors, []);
    } finally {
        await page.close();
        await context.unroute(msgRoute, handleMessages);
        await context.unroute(surfaceRoute, handleSurface);
    }
}

(async () => {
    verifyRouteParity();
    const bundle = process.env.NOLA_CLI_BUNDLE && path.resolve(process.env.NOLA_CLI_BUNDLE);
    if (!bundle) ensureUiBuilt();
    const cliBinary = bundle ? path.join(bundle, 'nola') : resolveNolaBinary();
    fs.accessSync(cliBinary, fs.constants.X_OK);
    const port = await allocatePort();
    const origin = `http://127.0.0.1:${port}`;
    const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-web-ui-'));
    const tmpHome = path.join(tmpRoot, 'home');
    const tmpDb = path.join(tmpRoot, 'nola.db');
    fs.mkdirSync(tmpHome, {recursive: true});

    const childEnv = {
        ...process.env,
        HOME: tmpHome,
        NOLA_HOME: tmpHome,
        NOLA_DB_PATH: tmpDb,
    };
    childEnv.NOLA_MAC_APP_SUPPORT = path.join(tmpRoot, 'support');

    const logs = [];
    const args = ['web', '--host', '127.0.0.1', '--port', String(port)];
    if (bundle) delete childEnv.NOLA_WEB_ROOT;
    else args.push('--web-root', webRoot);
    const host = spawn(cliBinary, args, {
        cwd: bundle ? tmpRoot : repoRoot,
        env: childEnv,
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    host.stdout.on('data', d => logs.push(d.toString('utf8')));
    host.stderr.on('data', d => logs.push(d.toString('utf8')));

    const out = process.env.UI_EVIDENCE_DIR || artifactPath('nola-web-visual-workflow');
    fs.mkdirSync(out, {recursive: true});
    const browser = await chromium.launch({...browserOptions(), headless: true});
    const errors = [];
    let viewCount = 0;

    try {
        await waitForHealth(origin, host, logs);

        // First run real HTTP round-trips against the isolated nola web Host
        const wfContext = await browser.newContext({viewport: {width: 1440, height: 900}, colorScheme: 'light', locale: 'en-US'});
        const wfPage = await wfContext.newPage();
        wfPage.on('pageerror', e => errors.push({context: 'workflow', url: wfPage.url(), message: e.message}));
        const convId = await runWorkflowRoundTrips(wfPage, origin);
        const sessionInfo = await (await wfPage.request.get(`${origin}/api/web/session`)).json();
        await wfContext.close();

        // Next run full visual/contrast/overflow/accessibility matrix across Light & Dark x Desktop & Mobile
        for (const scheme of ['light', 'dark']) {
            for (const viewport of [
                {width: 1440, height: 900},
                {width: 390, height: 844},
                {width: 320, height: 800},
            ]) {
                const context = await browser.newContext({
                    viewport,
                    colorScheme: scheme,
                    locale: 'zh-CN',
                    reducedMotion: 'reduce',
                });
                const page = await context.newPage();
                page.setDefaultTimeout(12_000);
                page.on('pageerror', e => errors.push({scheme, width: viewport.width, url: page.url(), message: e.message}));

                try {
                    for (const route of EXPECTED_ROUTES) {
                        await page.goto(`${origin}/${route}`, {waitUntil: 'networkidle'});
                        await inspectView(page, `route-${route}`, scheme, viewport.width);
                        if (route === 'login' && viewport.width === 390) {
                            await page.screenshot({path: path.join(out, `login-${scheme}-390.png`)});
                        }
                        viewCount += 1;
                    }

                    // Active Chat conversation on both desktop and mobile
                    await page.goto(`${origin}/chat?conversation=${encodeURIComponent(convId)}`, {waitUntil: 'networkidle'});
                    await inspectView(page, 'chat-active', scheme, viewport.width);
                    viewCount += 1;

                    if (viewport.width !== 320) {
                        await page.screenshot({path: path.join(out, `chat-${scheme}-${viewport.width}.png`)});
                    }

                    // Settings 6 categories
                    for (const category of SETTINGS_CATEGORIES) {
                        await page.goto(`${origin}/settings?section=${category}`, {waitUntil: 'networkidle'});
                        await inspectView(page, `settings-${category}`, scheme, viewport.width);
                        viewCount += 1;
                    }

                    // DataSources tabs + Add Source modal
                    const dsTabs = viewport.width <= 700
                        ? ['knowledge', 'sources', 'memory', 'plans', 'activity']
                        : ['sources', 'memory', 'plans', 'usage'];
                    for (const tab of dsTabs) {
                        await page.goto(`${origin}/data-sources?tab=${tab}`, {waitUntil: 'networkidle'});
                        await inspectView(page, `data-sources-${tab}`, scheme, viewport.width);
                        viewCount += 1;
                        if (tab === 'sources') {
                            await page.locator('.page-actions .primary:visible').click();
                            await page.locator('dialog:modal').waitFor();
                            await inspectView(page, 'dialog-data-source', scheme, viewport.width);
                            viewCount += 1;
                            await page.keyboard.press('Escape');
                            await page.locator('dialog:modal').waitFor({state: 'hidden'});
                        }
                    }

                    // Capabilities Skills + MCP + Add Skill modal
                    for (const tab of ['skills', 'mcp']) {
                        await page.goto(`${origin}/capabilities?tab=${tab}`, {waitUntil: 'networkidle'});
                        await inspectView(page, `capabilities-${tab}`, scheme, viewport.width);
                        viewCount += 1;
                        if (tab === 'skills') {
                            await page.locator('header.page-header .actions button.primary:visible').click();
                            await page.locator('dialog:modal').waitFor();
                            await inspectView(page, 'dialog-capability', scheme, viewport.width);
                            viewCount += 1;
                            await page.keyboard.press('Escape');
                            await page.locator('dialog:modal').waitFor({state: 'hidden'});
                        }
                    }

                    // ModelCenter API + Local + Add Endpoint modal
                    await page.goto(`${origin}/models`, {waitUntil: 'networkidle'});
                    await inspectView(page, 'models-api', scheme, viewport.width);
                    viewCount += 1;
                    await page.locator('.catalog-actions button.primary:visible').click();
                    await page.locator('dialog:modal').waitFor();
                    await inspectView(page, 'dialog-remote-model', scheme, viewport.width);
                    viewCount += 1;
                    await page.keyboard.press('Escape');
                    await page.locator('dialog:modal').waitFor({state: 'hidden'});

                    await page.goto(`${origin}/models?tab=local`, {waitUntil: 'networkidle'});
                    await inspectView(page, 'models-local', scheme, viewport.width);
                    viewCount += 1;

                    // A2UI surface render & token cascade check (Desktop 1440 & Mobile 390)
                    if (viewport.width !== 320) {
                        await verifyA2uiSurface(context, origin, convId, scheme, viewport.width, sessionInfo.hostId);
                        viewCount += 1;
                    }


                } finally {
                    await context.close();
                }
            }
        }

        const connectionPage = await browser.newPage({locale: 'en-US'});
        try {
            await verifyFirstConnection(connectionPage, origin, path.join(tmpHome, '.nola/web/mobile-access-token'));
        } finally {
            await connectionPage.close();
        }
        const modelPage = await browser.newPage({locale: 'en-US'});
        try {
            await verifyModelSetupNavigation(modelPage, origin);
        } finally {
            await modelPage.close();
        }
        assert.deepEqual(errors, [], 'Uncaught browser page errors detected');
        console.log(`PASS: Web visual, contrast, overflow, accessibility, A2UI, and real HTTP round-trip suite (${viewCount} views across 14 routes, 6 settings categories, tabs, and modals)`);
    } finally {
        await browser.close();
        await stopProcess(host);
        fs.rmSync(tmpRoot, {recursive: true, force: true});
    }
})().catch(error => {
    console.error(error);
    process.exitCode = 1;
});
