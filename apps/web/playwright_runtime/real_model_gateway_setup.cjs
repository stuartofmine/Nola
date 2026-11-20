// Exercises the public gateway setup flow against a disposable Rust Host.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {browserOptions, isolatedHostOrigin} = require('./test_environment.cjs');

async function main() {
    const origin = isolatedHostOrigin();
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE, 'utf8'));
    const fixture = fs.mkdtempSync(path.join(state.root, 'gateway-client-'));
    const config = path.join(fixture, 'opencode.json');
    fs.writeFileSync(config, '{}\n');
    const browser = await chromium.launch({...browserOptions(), headless: true});
    const page = await browser.newPage({locale: 'zh-CN'});
    const accessURL = new URL('/api/model-gateway/access', origin).href;
    const receipt = pathname => page.waitForResponse(response =>
        new URL(response.url()).pathname === pathname && response.request().method() === 'POST');
    let revoke;
    try {
        await page.goto(new URL('/models?tab=api', origin).href);
        const initial = await (await page.request.get(accessURL)).json();
        assert.equal(initial.enabled, false, 'Use an isolated Host without an active gateway token.');
        const clients = page.locator('details').filter({has: page.locator('summary', {hasText:'连接外部客户端'})}).last();
        await clients.locator('summary').first().click();
        await clients.locator('input').first().fill(config);
        await clients.locator('select').last().selectOption({index: 1});
        const preview = clients.getByRole('button', {name: '读取导入计划', exact: true});
        assert(await preview.isDisabled());

        await clients.getByRole('button', {name: '先设置网关访问令牌', exact: true}).click();
        const access = page.locator('details').filter({has: page.locator('summary', {hasText: '供其他客户端使用'})}).last();
        const issue = access.getByRole('button', {name: '生成或替换令牌', exact: true});
        assert(await issue.evaluate(button => document.activeElement === button));
        const issued = receipt('/api/model-gateway/access');
        await issue.click();
        assert((await issued).ok());
        revoke = access.getByRole('button', {name: /撤销|吊销/});

        const planned = receipt('/api/model-gateway/clients/plan');
        await preview.click();
        const plan = await planned;
        assert(plan.ok());
        assert((await plan.json()).owned_fields.length);
        assert.equal(fs.readFileSync(config, 'utf8'), '{}\n');

        const revoked = receipt('/api/model-gateway/access');
        await revoke.click();
        assert((await revoked).ok());
        revoke = null;
        assert.equal((await (await page.request.get(accessURL)).json()).enabled, false);
        await clients.locator('li').first().waitFor({state: 'detached'});
        assert(await preview.isDisabled());
        console.log('PASS missing access guidance, setup focus, read-only preview, revocation invalidation');
    } finally {
        if (revoke && await revoke.isVisible()) {
            const revoked = receipt('/api/model-gateway/access');
            await revoke.click();
            await revoked;
        }
        await browser.close();
        fs.rmSync(fixture, {recursive: true, force: true});
    }
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
