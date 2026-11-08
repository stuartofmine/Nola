const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
(async () => {
    const out = '/tmp/nola-a2ui-ui-evidence'; fs.mkdirSync(out, {recursive:true});
    const browser = await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless:true});
    const context = await browser.newContext({viewport:{width:390,height:844}});
    const page = await context.newPage(); const errors = []; page.on('pageerror', error => errors.push(error.message));
    try {
        await page.route('**/ui-surface-revision', async route => {
            const ref = route.request().postDataJSON();
            await route.fulfill({json:{snapshot:{identity:ref.identity,revision:ref.revision,catalog_id:'fixture',catalog_digest:'fixture-digest',phase:'read_only',root_instance_id:'history',summary:'历史登记表',nodes:{history:{instance_id:'history',component_id:'history',component:'Text',props:{text:'历史登记表'},slots:{},bindings:{},actions:{},validation_errors:[]}}}}});
        });
        const url = 'http://127.0.0.1:5138/review-tests/tests/a2ui.html';
        await page.goto(url);
        const field = page.getByRole('textbox', {name:'姓名'});
        await field.fill('张三');
        await field.evaluate(input => { input.focus(); input.setSelectionRange(1,1); });
        const originalField = await field.elementHandle();
        // Model-driven updates do not move pointer focus to the fixture's trigger.
        await page.locator('#update').evaluate(button => button.click());
        await page.getByText('更新后的登记表', {exact:true}).waitFor();
        assert.equal(await field.inputValue(), '张三');
        assert.equal(await originalField.evaluate(input => input.isConnected && input === document.activeElement), true,
            'unrelated model delta must retain the focused native input node');
        assert.deepEqual(await field.evaluate(input => [input.selectionStart,input.selectionEnd]),[1,1],
            'unrelated model delta must preserve the caret');
        await page.getByRole('button', {name:'提交表单',exact:true}).click();
        assert.equal(await page.locator('#calls').textContent(), '0', 'stale edit must not submit');
        await page.getByRole('button', {name:'放弃编辑，使用当前值'}).click();
        await field.fill('李四'); await page.getByRole('button',{name:'提交表单',exact:true}).click();
        await page.waitForFunction(() => document.querySelector('#calls').textContent === '1');
        const receipt = JSON.parse(await page.locator('#receipt').textContent());
        assert.equal(receipt.input.edits[0].value, '李四');
        await page.getByRole('button',{name:'查看详情',exact:true}).click();
        await page.getByRole('dialog').waitFor();
        assert.equal(await page.locator('dialog').evaluate(d => d.matches(':modal')), true);
        await page.locator('dialog').evaluate(async d => { await Promise.all(d.getAnimations().map(a => a.finished)); });
        const paint = await page.locator('dialog').evaluate(d => ({opacity:getComputedStyle(d).opacity,background:getComputedStyle(d).backgroundColor}));
        assert.equal(paint.opacity, '1'); assert.ok(!paint.background.includes('rgba'), 'dialog must paint opaque');
        const bounds = await page.getByRole('dialog').boundingBox(); assert.ok(bounds.width <= 390 && bounds.height <= 844);
        await page.screenshot({path:out+'/modal.png'}); await page.keyboard.press('Escape');
        assert.equal(await page.getByRole('button',{name:'查看详情',exact:true}).evaluate(e=>e===document.activeElement),true);
        await page.getByRole('button',{name:'查看此版本 · 1'}).click(); await page.getByText('历史登记表',{exact:true}).waitFor();
        await page.screenshot({path:out+'/history.png'});
        await page.locator('#unknown').click(); await field.fill('恢复原请求');
        await page.getByRole('button',{name:'提交表单',exact:true}).click();
        await page.waitForFunction(() => document.querySelector('#calls').textContent === '2');
        await page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent === '查询提交结果' && !button.disabled));
        const pending = JSON.parse(await page.locator('#receipt').textContent());
        await page.reload();
        await page.getByRole('button',{name:'查询提交结果'}).click();
        await page.waitForFunction(() => document.querySelector('#calls').textContent === '1');
        assert.deepEqual(JSON.parse(await page.locator('#receipt').textContent()),pending,'reload retries identical durable attempt');
        await page.locator('#delete').click();
        await field.waitFor({state:'detached'});
        await page.screenshot({path:out+'/deleted.png'});
        assert.deepEqual(errors, []);
        fs.writeFileSync(out+'/result.json',JSON.stringify({kind:'renderer-fixture',browser:'Google Chrome',passed:['edit','delta-preserves-edit','delta-preserves-focus-caret','revision-conflict','submit','modal-focus-viewport','historical-read-only','durable-unknown-reload','delete'],errors},null,2));
        console.log('PASS A2UI Chrome renderer fixture');
    } catch (error) {
        fs.writeFileSync(out+'/failure.txt', await page.locator('body').innerText());
        await page.screenshot({path:out+'/failure.png'});
        throw error;
    } finally {await browser.close();}
})().catch(error => { console.error(error); process.exitCode=1; });
