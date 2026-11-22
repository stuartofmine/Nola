// Run once against a direct Host and once against an isolated --tailcat-ingress listener.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {artifactPath, browserOptions, isolatedHostOrigin} = require('./test_environment.cjs');

async function main() {
    const mode = process.argv[2];
    assert.ok(['direct','ingress'].includes(mode), 'Choose direct or ingress.');
    const local = mode === 'direct';
    const base = isolatedHostOrigin();
    const url = pathname => new URL(pathname, base).href;
    const {root} = JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE, 'utf8'));
    const token = fs.readFileSync(path.join(root,'home','.nola','web','mobile-access-token'), 'utf8').trim();
    const out = artifactPath(process.env.NOLA_HOST_ROLE_EVIDENCE || 'web-host-roles');
    fs.mkdirSync(out,{recursive:true});
    const browser = await chromium.launch({...browserOptions(),headless:true});
    async function signIn(page) {
        await page.locator('#host-token').fill(token);
        await page.getByRole('button',{name:'Connect',exact:true}).tap();
        await page.waitForURL(u => u.pathname === '/settings' && u.searchParams.get('section') === 'connections');
    }
    async function session(page) {
        const response = await page.request.get(url('/api/web/session'));
        assert.ok(response.ok());
        return response.json();
    }
    const options = {locale:'en-US',hasTouch:true,isMobile:true,viewport:{width:390,height:844},reducedMotion:'reduce'};
    try {
        const page = await browser.newPage(options);
        const other = await browser.newPage(options);
        const sessions = await session(page);
        assert.equal(sessions.localTrust,local);
        assert.equal(sessions.authenticated,local);
        assert.equal(sessions.sessionAuthenticated,false);
        assert.equal(sessions.transportIngress,local ? 'direct':'tailcat');
        const destination = '/settings?section=connections';
        const login = '/login?next='+encodeURIComponent(destination);
        await page.goto(url(destination));
        if (local) {
            const open = page.getByRole('link',{name:'Sign in with a Host token'});
            await open.tap();
            await page.locator('#host-token').fill('unsent');
            await page.getByRole('button',{name:'Cancel',exact:true}).tap();
            await open.waitFor();
            assert.equal((await session(page)).sessionAuthenticated,false);
            await open.tap();
        } else {
            await page.waitForURL(u => u.pathname === '/login' && u.searchParams.get('next') === destination);
            assert.equal(await page.getByRole('button',{name:'Cancel',exact:true}).count(),0);
            assert.equal((await page.request.get(url('/api/chat/conversations'))).status(),401);
        }
        await signIn(page);
        await other.goto(url(login));
        await signIn(other);
        assert.equal((await session(page)).sessionAuthenticated,true);
        const ownerReads = [];
        page.on('request', request => {
            if (/\/api\/model-connections\/[^/]+-login\/owners$/.test(new URL(request.url()).pathname)) ownerReads.push(request);
        });
        if (local) {
            await Promise.all([
                page.waitForResponse(r => new URL(r.url()).pathname === '/api/model-connections/kimi-login/owners' && r.ok()),
                page.goto(url('/models?tab=api')),
            ]);
            await page.getByText('Codex local sign-in',{exact:true}).waitFor();
            await page.getByText('Codex local sign-in',{exact:true}).scrollIntoViewIfNeeded();
        } else {
            await page.goto(url('/models?tab=api'));
            const instruction = page.getByText('Open the local workspace on the computer running Nola to manage model source accounts.');
            await instruction.scrollIntoViewIfNeeded();
            assert.equal(await page.getByText('Codex local sign-in',{exact:true}).count(),0);
            assert.equal(ownerReads.length,0);
        }
        await page.screenshot({path:path.join(out, mode+'-source-accounts.png'), animations:'disabled'});
        const [inventory] = await Promise.all([
            page.waitForResponse(r => new URL(r.url()).pathname === '/api/harness-plugins' && r.request().method() === 'POST'),
            page.goto(url('/capabilities?tab=plugins')),
        ]);
        assert.ok(inventory.ok());
        assert.ok(Array.isArray((await inventory.json()).profiles));
        await page.getByRole('status').filter({hasText:'Loading'}).waitFor({state:'hidden'});
        assert.equal(await page.locator('[role="alert"]:empty').count(),0);
        await page.goto(url(destination));
        const panel = page.locator('#host-session');
        const logout = panel.getByRole('button',{name:/sign out/i});
        await logout.waitFor();
        if (!local) assert.ok((await panel.innerText()).includes(sessions.hostId));
        await page.screenshot({path:path.join(out, mode+'-host-session.png'),animations:'disabled'});
        await logout.tap();
        await page.getByRole('dialog').getByRole('button',{name:'Cancel'}).tap();
        assert.equal((await session(page)).sessionAuthenticated,true);
        await logout.tap();
        await page.getByRole('dialog').getByRole('button',{name:/sign out/i}).tap();
        await page.waitForURL(u => u.pathname === '/login' && u.searchParams.get('logged_out') === '1');
        assert.equal((await session(page)).sessionAuthenticated,false);
        assert.equal((await session(other)).sessionAuthenticated,true);
        assert.equal((await page.request.get(url('/api/chat/conversations'))).status(), local ? 200 : 401);
        console.log('PASS',mode,'sign-in and return, source-account visibility, plugin inventory, cancel, logout scoped to one browser, access after logout');
    } finally {await browser.close();}
}

main().catch(error => {console.error(error);process.exitCode=1;});
