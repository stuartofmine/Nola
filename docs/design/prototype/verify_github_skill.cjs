// SPDX-License-Identifier: Apache-2.0
// Browser Use interacts with the local prototype. Playwright observes state and exports screens.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const net = require('node:net');
const path = require('node:path');
const {chromium} = require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const {BrowserUseExecutor} = require('../../../apps/web/playwright_runtime/browser_use.cjs');
const {artifactPath, browserOptions} = require('../../../apps/web/playwright_runtime/test_environment.cjs');
const origin = new URL(process.env.PROTOTYPE_URL || 'http://127.0.0.1:8071');
assert.ok(['localhost','127.0.0.1','[::1]'].includes(origin.hostname));
const out = artifactPath('github-skill-prototype');

async function run() {
    fs.mkdirSync(out, {recursive:true});
    const server = net.createServer();
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const port = server.address().port;
    await new Promise(resolve => server.close(resolve));
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-skill-preview-'));
    const context = await chromium.launchPersistentContext(profile, {...browserOptions(), headless:true, args:[`--remote-debugging-port=${port}`]});
    let driver;
    try {
        const errors = [];
        await context.route('**/*', route => {
            if (new URL(route.request().url()).origin === origin.origin) return route.continue();
            errors.push(route.request().url());
            return route.abort();
        });
        const page = context.pages()[0];
        page.on('pageerror', error => errors.push(error.message));
        driver = await BrowserUseExecutor.attach(`http://127.0.0.1:${port}`, context);
        async function input(locator, operation = 'click', args = {}) {
            await locator.scrollIntoViewIfNeeded();
            const element = await locator.elementHandle();
            try { await driver.perform(page, operation, args, element); }
            finally { await element.dispose(); }
        }
        const device = page.locator('#device');
        const action = label => input(device.getByRole('button', {name:label, exact:true}));
        const screen = name => device.screenshot({path:path.join(out, name + '.png')});
        async function review(platform, theme, host) {
            const mobile = platform === 'ios' || platform === 'android';
            const scene = host ? 'github-skill-host' : 'github-skill';
            await page.setViewportSize({width:mobile ? 390 : 1440, height:900});
            await driver.perform(page, 'navigate', {url:`${origin.origin}/scenarios.html?scene=${scene}&platform=${platform}&theme=${theme}`});
            await device.locator('h3').waitFor();
            const owner = await device.locator('.gs-bar span').innerText();
            assert.equal(owner.includes('配对 Host'), host);
            await action('按名称查找');
            await action('选择 nature-paper-card');
            const source = device.locator('.gs-ref summary');
            await input(source);
            const facts = await device.locator('.gs-facts').innerText();
            assert.ok(facts.includes(owner));
            assert.match(facts, /https:\/\/github\.com\/Yuan1z0825\/nature-skills\/tree\/[0-9a-f]{40}\/skills\/nature-paper-card/);
            const prefix = platform + (host ? '-host-' : '-local-') + theme;
            await input(source);
            await screen(prefix + '-approval');
            await action('拒绝');
            assert.equal(await device.getAttribute('data-state'), 'denied');
            await action('返回对话');
            await action('使用目录链接');
            await input(source);
            assert.match(await device.locator('.gs-facts').innerText(), /tree\/main\/skills/);
            await action('仅允许本次');
            assert.equal(await device.getAttribute('data-state'), 'installed');
            await action('在新会话中使用');
            assert.match(await device.innerText(), /Terminology Ledger/);
            await screen(prefix + '-fresh');
            await action('查看已安装技能');
            await input(device.locator('.gs-source summary'));
            assert.match(await device.locator('.gs-source code').innerText(), /tree\/[0-9a-f]{40}/);
            if (platform !== 'cli') {
                await input(device.locator('.gs-editor summary'));
                const field = device.getByRole('textbox', {name:'技能指令'});
                await input(field, 'type_text', {text:'保留论文出处'});
                await action('保存');
                await input(device.locator('.gs-editor summary'));
                assert.equal(await field.inputValue(), '保留论文出处');
                await input(field, 'type_text', {text:'取消的修改'});
                await action('取消');
                await input(device.locator('.gs-editor summary'));
                assert.equal(await field.inputValue(), '保留论文出处');
                await input(device.locator('.gs-editor summary'));
            }
            await screen(prefix + '-settings');
            await action('停用');
            assert.equal(await device.getByRole('button', {name:'在新会话中使用'}).count(), 0);
            await action('重新启用');
            assert.equal(await device.getAttribute('data-state'), 'settings');
            if (mobile) {
                await page.setViewportSize({width:320,height:720});
                await input(page.getByText('预览与验证边界',{exact:true}));
                await input(page.locator('#large-text'));
                await input(page.locator('button[data-state="review"]'));
                assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1 && document.querySelector('#device').scrollWidth <= document.querySelector('#device').clientWidth + 1));
            }
            console.log(prefix + ' passed');
        }
        async function openFromLibrary(mobile, host) {
            await page.setViewportSize({width:mobile ? 390 : 1440, height:900});
            await driver.perform(page, 'navigate', {url:origin.origin + '/index.html'});
            const navigate = id => input(page.locator(`[data-action="${id}"]:visible`).first());
            if (mobile) {
                await navigate('im:nav:settings');
                await navigate('im:nav:capabilities');
            } else {
                await navigate('sn-section:extensions');
                await navigate('dk-ext-tab:skills');
            }
            await input(page.getByRole('link', {name:host ? '在配对 Host 安装' : '从对话安装 GitHub Skill'}));
            await device.locator('h3').waitFor();
            assert.equal(await device.getAttribute('data-scene'), host ? 'github-skill-host' : 'github-skill');
            assert.equal(new URL(page.url()).searchParams.get('platform'), mobile ? 'ios' : 'mac');
        }
        await openFromLibrary(false, false);
        await openFromLibrary(true, false);
        await openFromLibrary(true, true);
        for (const platform of ['ios','android','mac','web','windows','cli']) {
            for (const theme of ['light','dark']) {
                await review(platform, theme, false);
                if (platform === 'ios' || platform === 'android') await review(platform, theme, true);
            }
        }
        assert.deepEqual(errors, []);
    } finally {
        if (driver) await driver.close();
        await context.close();
        fs.rmSync(profile,{recursive:true,force:true});
    }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
