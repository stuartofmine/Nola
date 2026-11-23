// SPDX-License-Identifier: Apache-2.0
// Runs against a disposable Web Host and real configured model.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {browserOptions} = require('./test_environment.cjs');

(async () => {
    const [origin, fixture, evidence] = process.argv.slice(2);
    const browser = await chromium.launch({...browserOptions(), headless: true});
    const context = await browser.newContext({viewport: {width: 1280, height: 900}});
    const page = await context.newPage();
    async function start(prompt) {
        await page.goto(origin + '/chat');
        const create = page.locator('.new-conversation');
        if (!await create.isVisible()) await page.locator('.navigation-toggle').click();
        const [response] = await Promise.all([
            page.waitForResponse(r => r.url() === origin + '/api/chat/conversations' && r.request().method() === 'POST'),
            create.click(),
        ]);
        const record = await response.json();
        const id = record.conversationId ?? record.conversation?.conversationId;
        assert.ok(id);
        await page.waitForURL(url => url.searchParams.get('conversation') === id);
        await page.locator('#chat-composer:not([disabled])').fill(prompt);
        await page.locator('.composer-actions button[type=submit]').click();
        return id;
    }
    async function waitForCompletion(id) {
        const deadline = Date.now() + 120000;
        while (Date.now() < deadline) {
            const response = await context.request.get(origin + '/api/chat/conversations/' + encodeURIComponent(id) + '/messages');
            const state = (await response.json()).conversationProjection;
            if (!state?.active_run && state?.messages?.at(-1)?.state === 'completed') return;
            await page.waitForTimeout(500);
        }
        throw Error('Conversation did not reach its completed state: ' + id);
    }
    try {
        const token = fs.readFileSync(path.join(fixture, 'home/.nola/web/mobile-access-token'), 'utf8').trim();
        const login = await context.request.post(origin + '/api/web/login', {data: {token}, headers: {Origin: origin}});
        assert.ok(login.ok());
        const install = await start('请把这个 GitHub Skill 安装到当前 Nola，启用供新会话使用：https://github.com/Yuan1z0825/nature-skills/tree/main/skills/nature-paper-card');
        const readSettings = async () => (await context.request.get(origin + '/api/agent/settings')).json();
        const deadline = Date.now() + 180000;
        for (;;) {
            const panel = page.locator('.permission-panel').last();
            if (await panel.isVisible()) {
                assert.equal(await panel.locator('.tool').getAttribute('data-tool-name'), 'github_skill_install');
                assert.match(await panel.locator('pre').innerText(), /Yuan1z0825\/nature-skills/i);
                const approve = panel.locator('button.primary');
                if (await approve.isEnabled()) await approve.click();
            }
            const settings = await readSettings();
            if ((settings.skills || settings.uploadedSkills || []).some(s => s.id === 'nature-paper-card' && s.enabled)) break;
            if (Date.now() > deadline) throw Error('The submitted Web chat did not register the Skill');
            await page.waitForTimeout(800);
        }
        await waitForCompletion(install);
        await page.screenshot({path: path.join(evidence, 'web-install.png')});
        const fresh = await start('请调用 nature-paper-card，实际打开 manifest.yaml 指定的术语表，原样引用它第一句英文正文。');
        assert.notEqual(install, fresh);
        await page.getByText(/A manuscript must use one name for one thing/i).last().waitFor({timeout: 180000});
        await waitForCompletion(fresh);
        await page.screenshot({path: path.join(evidence, 'web-fresh.png')});
        fs.writeFileSync(path.join(evidence, 'web-conversations.json'), JSON.stringify({install,fresh}));
        console.log('Passed Web chat installation and fresh conversation resource read');
    } catch (error) {
        await page.screenshot({path:path.join(evidence, 'web-failure.png')});
        console.error((await page.locator('body').innerText()).slice(-900));
        throw error;
    } finally {
        await browser.close();
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
