// SPDX-License-Identifier: Apache-2.0
// Isolated, synthetic Host UI review. All requests stay on the prototype origin.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {chromium} = require('../../../apps/web/playwright_runtime/node_modules/playwright-core');
const {artifactPath, browserOptions} = require('../../../apps/web/playwright_runtime/test_environment.cjs');
const base = new URL(process.env.PROTOTYPE_URL || 'http://127.0.0.1:8071');
const out = process.env.PROTOTYPE_OUT || artifactPath('host-prototype');
const sample = 'http://studio-mac.local:5001';

async function main() {
    assert.ok(['localhost','127.0.0.1','[::1]'].includes(base.hostname));
    fs.mkdirSync(out, {recursive:true});
    const browser = await chromium.launch({...browserOptions(), headless:true});
    try {
        for (const platform of ['mac','ios','android']) {
            for (const theme of ['light','dark']) {
                const mobile = platform !== 'mac';
                const context = await browser.newContext({viewport:{width:mobile ? 390 : 1440,height:920}, colorScheme:theme,
                    hasTouch:mobile, reducedMotion:theme === 'dark' ? 'reduce' : 'no-preference'});
                const page = await context.newPage();
                const errors = [];
                page.on('pageerror', err => errors.push(String(err)));
                await context.route('**/*', route => new URL(route.request().url()).origin === base.origin
                    ? route.continue() : (errors.push('Unexpected external request'), route.abort()));
                const device = page.locator('#device');
                const click = label => device.getByRole(['开启共享','关闭共享'].includes(label) ? 'switch' : 'button', {name:label,exact:true}).click();
                const go = scene => page.goto(new URL(`/scenarios.html?scene=${scene}&platform=${platform}&theme=${theme}`,base).href);
                const at = state => page.locator(`#device[data-state="${state}"]`).waitFor();
                const shot = name => device.screenshot({path:path.join(out,`${platform}-${theme}-${name}.png`)});
                if (!mobile) {
                    await go('share-host');
                    await click('开启共享'); await at('starting');
                    await click('关闭共享');
                    await page.waitForTimeout(1250);
                    await at('0');
                    await page.getByLabel('下次演示回执').selectOption('failed');
                    await click('开启共享'); await at('failed');
                    await click('重试'); await at('1');
                    await click('复制连接信息'); await at('copied'); await shot('share');
                    await device.locator('summary').click();
                    await click('复制地址');
                    assert.equal(await device.locator('.hp-copy-status').innerText(),'已复制地址');
                    await click('关闭共享'); await at('2');
                }
                await go('host-pair');
                if (mobile) {
                    const help = device.locator('.hp-setup-help');
                    await help.locator('summary').click();
                    assert.ok(await help.locator('p').isVisible());
                    await help.locator('summary').click();
                }
                await click('手动填写');
                await click('验证并连接');
                assert.match(await device.locator('#field-error').innerText(),/地址/);
                await click('粘贴连接信息'); await at('draft');
                const address = device.getByRole('textbox',{name:'宿主机地址'});
                const token = device.locator('[data-host-field="token"]');
                assert.equal(await address.inputValue(),sample);
                if (platform === 'mac') {
                    await click('验证并连接'); await at('checking');
                    assert.ok(await address.isDisabled());
                    await click('取消验证'); await at('draft');
                    await page.waitForTimeout(1250); await at('draft');
                }
                await token.fill('invalid-demo');
                await click('验证并连接'); await at('rejected');
                assert.equal(await address.inputValue(),sample); await shot('retry');
                await click('粘贴连接信息');
                await address.fill('https://other.example.invalid');
                assert.equal(await token.inputValue(),'');
                if (platform === 'android') await device.locator('[data-host-field="pin"]').waitFor();
                await click('验证并连接'); await at('draft');
                assert.match(await device.locator('#field-error').innerText(),/令牌/);
                await page.locator('#state-list [data-state="0"]').click();
                await click('粘贴连接信息');
                await page.getByLabel('下次演示回执').selectOption('1');
                await click('验证并连接'); await at('1');
                await click('检查连接信息'); await click('验证并连接'); await at('2');
                await shot('paired');
                if (platform === 'android') await click('使用');
                await click('编辑连接');
                await address.fill('https://other.example.invalid');
                await token.fill('demo-pairing-token');
                await click('验证并连接'); await at('offline');
                await click('返回已保存的连接'); await at('2');
                assert.ok((await device.innerText()).includes(sample));
                await click(({mac:'管理消息渠道',ios:'在宿主机上打开插件',android:'管理宿主机模型'})[platform]);
                await at('manage'); await click('返回'); await at('2');
                const forget = ({mac:'移除连接',ios:'忘记配对',android:'断开宿主机'})[platform];
                await click(forget); await page.getByRole('button',{name:'取消',exact:true}).click(); await at('2');
                await click(forget); await page.locator('#confirm-apply').click(); await at('0');
                if (mobile) {
                    await click('粘贴连接信息'); await click('验证并连接'); await at('checking');
                    await click('取消验证');
                    await page.waitForTimeout(1250); await at('draft');
                    assert.equal(await address.inputValue(),sample);
                    assert.ok(await token.inputValue());
                    await page.setViewportSize({width:320,height:800});
                    await page.locator('#state-list [data-state="storage"]').click();
                    assert.ok(await device.evaluate(el => el.scrollWidth <= el.clientWidth + 1));
                }
                assert.deepEqual(errors,[]);
                console.log(`PASS ${platform} ${theme}`);
                await context.close();
            }
        }
        for (const platform of ['web','windows']) {
            const page = await browser.newPage();
            await page.goto(new URL(`/scenarios.html?scene=web-login&platform=${platform}`,base).href);
            const device = page.locator('#device');
            const token = device.locator('input[type=password]');
            const submit = label => device.getByRole('button',{name:label,exact:true}).click();
            await token.fill('invalid-demo'); await submit('登录');
            await device.getByRole('heading',{name:'访问被拒绝'}).waitFor();
            await submit('修改令牌');
            assert.equal(await token.inputValue(),'invalid-demo');
            await token.fill('DEMO_ONLY'); await submit('登录');
            await submit('退出当前会话'); await page.locator('#confirm-cancel').click();
            await device.getByRole('heading',{name:'已连接'}).waitFor();
            await submit('退出当前会话'); await page.locator('#confirm-apply').click();
            await device.getByRole('heading',{name:'浏览器会话已退出'}).waitFor();
            await page.locator('#state-list [data-state=local]').click();
            await submit('用令牌登录'); await submit('取消');
            await device.getByRole('heading',{name:'本机连接'}).waitFor();
            console.log(`PASS ${platform} sign-in, cancel and logout`);
            await page.close();
        }
    } finally { await browser.close(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
