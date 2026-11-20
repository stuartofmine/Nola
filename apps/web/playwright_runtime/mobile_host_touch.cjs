// Trusted touch input against a disposable Rust Host. No device keyboard is emulated.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {artifactPath, browserOptions, isolatedHostOrigin} = require('./test_environment.cjs');

async function verifyEditorGestures(page, origin) {
    await page.goto(new URL('/settings?section=agent', origin).href);
    const editor = page.getByRole('textbox', {name:'Custom system prompt (optional)'});
    await editor.tap();
    await editor.fill('Retain this draft while scrolling');
    const cdp = await page.context().newCDPSession(page);
    const touch = (type, x, y) => cdp.send('Input.dispatchTouchEvent', {
        type, touchPoints:type === 'touchEnd' ? [] : [{x, y}],
    });
    try {
        const before = await page.evaluate(() => scrollY);
        await touch('touchStart', 7, 600);
        for (const y of [575, 525, 475, 425, 375, 350]) await touch('touchMove', 7, y);
        await touch('touchEnd');
        await page.waitForFunction(offset => window.scrollY > offset + 50, before);
        assert.ok(await editor.evaluate(el => el === document.activeElement));
        assert.equal(await editor.inputValue(), 'Retain this draft while scrolling');
        await editor.scrollIntoViewIfNeeded();
        await editor.evaluate(el => {
            el.setSelectionRange(0, 6);
            document.addEventListener('copy', event => {
                event.preventDefault(); // Observe the native command without writing the machine clipboard.
                window.__copiedSelection = el.value.slice(el.selectionStart, el.selectionEnd);
            }, {once:true});
        });
        await editor.press(process.platform === 'darwin' ? 'Meta+c' : 'Control+c');
        assert.equal(await page.evaluate(() => window.__copiedSelection), 'Retain');
        // Check focus at touch release, before the browser generates mouse events.
        await page.evaluate(() => window.addEventListener('pointerup', () => {
            window.__longPressActive = document.activeElement?.tagName;
        }, {once:true}));
        await touch('touchStart', 7, 300);
        await new Promise(resolve => setTimeout(resolve, 600));
        await touch('touchEnd');
        assert.equal(await page.evaluate(() => window.__longPressActive), 'TEXTAREA');
        await editor.fill('');
        console.log('PASS settings scroll retained focus, copy preserved selected text, long press did not dismiss editor');
    } finally { await cdp.detach(); }
}

async function verify(context, origin, state, out, motion) {
    const page = await context.newPage();
    page.setDefaultTimeout(10000);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    const button = name => page.getByRole('button', {name, exact: true});
    const link = name => page.getByRole('link', {name, exact: true});
    const hostPanel = page.locator('#host-session');
    const url = route => new URL(route, origin).href;
    const shot = name => page.screenshot({path:path.join(out, motion + '-' + name + '.png'), animations:'disabled'});
    const isFocused = field => field.evaluate(el => el === document.activeElement);
    const tapped = async (field, value) => {
        await field.tap();
        await field.fill(value);
        assert.ok(await isFocused(field));
    };
    async function verifiedBlankTap(field, x, y, value) {
        // Observe the end of the trusted touch before browser-specific mouse focus defaults.
        await page.evaluate(() => {
            window.addEventListener('pointerup', event => {
                window.__blankTap = {trusted:event.isTrusted, type:event.pointerType, active:document.activeElement?.id};
            }, {once:true});
        });
        await page.touchscreen.tap(x, y);
        const observed = await page.evaluate(() => window.__blankTap);
        assert.ok(observed.trusted && observed.type === 'touch');
        assert.notEqual(observed.active, await field.getAttribute('id'));
        assert.equal(await field.inputValue(), value);
        assert.equal(await isFocused(field), false);
    }
    try {
        await page.goto(url('/conversations'));
        await button('New conversation').tap();
        await page.waitForURL(u => u.pathname === '/chat' && !!u.searchParams.get('conversation'));
        const composer = page.locator('#chat-composer');
        const draft = 'Unsent touch draft';
        await tapped(composer, draft);
        await verifiedBlankTap(composer, 8, 190, draft);
        await composer.tap();
        await button('More tools').tap();
        await button('Find in conversation').tap();
        const search = page.locator('#chat-search-input');
        await tapped(search, 'notes');
        await verifiedBlankTap(search, 8, 260, 'notes');
        assert.equal(await composer.inputValue(), draft);
        await shot('chat-search');
        await button('Workspace').tap();
        await page.waitForURL(u => u.pathname === '/conversations');
        await link('Activity').tap();
        await link('Settings').tap();
        await link('Connections').tap();
        await hostPanel.getByRole('heading').waitFor();
        await page.goBack();
        await link('Connections').waitFor();
        assert.equal(new URL(page.url()).pathname,'/settings');
        await page.goForward();
        await hostPanel.getByRole('heading').waitFor();
        await page.locator('.settings-back button').tap();
        await link('Connections').waitFor();
        await link('Connections').tap();
        await link('Sign in with a Host token').tap();
        const tokenField = page.locator('#host-token');
        await tokenField.waitFor();
        assert.equal(new URL(page.url()).searchParams.get('next'), '/settings?section=connections');
        await tokenField.fill('unsent-test-token');
        const loginWrites = [];
        const observeLogin = request => {
            if (request.method() === 'POST' && new URL(request.url()).pathname === '/api/web/login') loginWrites.push(request);
        };
        page.on('request', observeLogin);
        await button('Cancel').tap();
        await link('Sign in with a Host token').waitFor();
        assert.equal(loginWrites.length, 0);
        page.off('request', observeLogin);
        await link('Sign in with a Host token').tap();
        await tokenField.waitFor();
        assert.equal(await tokenField.inputValue(), '');
        await tapped(tokenField, 'short-input');
        await verifiedBlankTap(tokenField, 4, 250, 'short-input');
        await button('Show').tap();
        assert.equal(await tokenField.getAttribute('type'), 'text');
        await button('Hide').tap();
        await button('Connect').tap();
        await page.locator('#token-validation').waitFor();
        assert.ok(await isFocused(tokenField));
        await shot('token-validation');
        const token = fs.readFileSync(path.join(state.root, 'home', '.nola', 'web', 'mobile-access-token'), 'utf8').trim();
        await tokenField.fill(token);
        const [login] = await Promise.all([page.waitForResponse(r => new URL(r.url()).pathname === '/api/web/login'), button('Connect').tap()]);
        assert.ok(login.ok());
        await page.waitForURL(u => u.pathname === '/settings' && u.searchParams.get('section') === 'connections');
        const signOut = hostPanel.getByRole('button', {name:/sign out/i});
        await signOut.waitFor();
        const session = async () => {
            const response = await page.request.get(url('/api/web/session'));
            assert.ok(response.ok());
            return response.json();
        };
        assert.equal((await session()).sessionAuthenticated, true);
        await shot('host-connected');
        await signOut.tap();
        await page.getByRole('dialog').getByRole('button',{name:'Cancel'}).tap();
        await signOut.tap();
        const [logout] = await Promise.all([
            page.waitForResponse(r => new URL(r.url()).pathname === '/api/web/logout'),
            page.getByRole('dialog').getByRole('button',{name:/sign out/i}).tap(),
        ]);
        assert.ok(logout.ok());
        await page.waitForURL(u => u.pathname === '/login' && u.searchParams.get('logged_out') === '1');
        assert.equal(new URL(page.url()).searchParams.get('next'), '/settings?section=connections');
        assert.equal((await session()).sessionAuthenticated, false);
        await verifyEditorGestures(page, origin);
        assert.deepEqual(errors, []);
        console.log('PASS', motion, 'chat draft, chat search, OS-style back/forward, token validation, Host login, cancel, logout');
    } finally {
        await page.close();
    }
}

async function main() {
    const origin = isolatedHostOrigin();
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE, 'utf8'));
    const out = artifactPath(process.env.NOLA_MOBILE_TOUCH_EVIDENCE || 'mobile-touch');
    fs.mkdirSync(out, {recursive:true});
    const browser = await chromium.launch({...browserOptions(), headless:true});
    try {
        for (const reducedMotion of ['no-preference', 'reduce']) {
            const context = await browser.newContext({viewport:{width:390,height:844},hasTouch:true,isMobile:true,locale:'en-US',reducedMotion});
            try { await verify(context, origin, state, out, reducedMotion); } finally { await context.close(); }
        }
    } finally { await browser.close(); }
}

main().catch(error => {console.error(error);process.exitCode=1;});
