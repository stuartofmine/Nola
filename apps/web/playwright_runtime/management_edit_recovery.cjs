// SPDX-License-Identifier: Apache-2.0
// A disposable Host owns the expert directory. Task revisions, plugin responses and OS permission decisions are fixtures.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {browserOptions,artifactPath} = require('./test_environment.cjs');
const origin = process.env.UI_FIXTURE_URL;
const out = process.env.UI_EVIDENCE_DIR || artifactPath('management-edit-recovery');
const shot = (page, name) => page.screenshot({path:path.join(out, name+'.png')});
const button = (area, name) => area.getByRole('button',{name,exact:true});

async function plans(page) {
    let rows = [], patches = 0;
    await page.route('**/api/scheduled-actions', async route => {
        if (route.request().method() === 'GET') return route.fulfill({json:{actions:rows,executionCapabilities:{scheduledDispatch:true,manualRun:true}}});
        const input = route.request().postDataJSON();
        rows = [{...input,id:'planned',revision:1,enabled:false,updatedAt:'2026-01-01',url:''}];
        return route.fulfill({json:{success:true,action:rows[0]}});
    });
    await page.route('**/api/scheduled-actions/planned', route => {
        patches++;
        const {expectedRevision,definition} = route.request().postDataJSON();
        if (patches === 1) {
            rows[0] = {...rows[0],name:'Updated on host',revision:2};
            return route.fulfill({status:409,json:{code:'automation_revision_conflict',error:'revision changed'}});
        }
        assert.equal(expectedRevision,2);
        rows[0] = {...rows[0],...definition,revision:3};
        return route.fulfill({json:{success:true,action:rows[0]}});
    });
    await page.goto(origin+'/scheduled-actions');
    await button(page,'New task').click();
    const form = page.locator('.automation-dialog:modal');
    const name = form.getByLabel('Name',{exact:true});
    const workspace = form.getByLabel('Execution workspace absolute path');
    await name.fill('Reading plan');
    await form.getByLabel('Agent prompt').fill('Summarize my reading list.');
    await workspace.fill('notes');
    await form.getByLabel('Execution permissions').selectOption('cautious');
    await button(form,'Save').click();
    assert.equal(await workspace.getAttribute('aria-invalid'),'true');
    assert.equal(await workspace.evaluate(n=>n===document.activeElement),true);
    const skills = form.locator('.skill-picker');
    assert.equal(await skills.evaluate(node=>node.open),false);
    await skills.locator('summary').click();
    await skills.getByRole('checkbox').first().check();
    await skills.locator('summary').click();
    assert.match(await skills.locator('summary').innerText(),/1/);
    await workspace.fill('/fixture/notes');
    await button(form,'Save').click();
    await page.getByRole('heading',{name:'Reading plan'}).waitFor();
    await button(page,'Edit task').click();
    await name.fill('Draft changes');
    await page.keyboard.press('Escape');
    await button(page,'Keep editing').click();
    assert.equal(await name.inputValue(),'Draft changes');
    await button(form,'Save').click();
    await form.getByRole('alert').filter({hasText:'changed on the host'}).waitFor();
    await button(form,'Load latest version').click();
    await button(page,'Keep editing').click();
    assert.equal(await name.inputValue(),'Draft changes');
    assert.equal(await button(form,'Save').count(),0);
    assert.ok(await form.locator('h2').evaluate(el=>el.getBoundingClientRect().top >= 0));
    assert.ok(await form.locator('footer').evaluate(el=>el.getBoundingClientRect().bottom <= innerHeight));
    assert.equal(await page.locator('dialog:not([open])').evaluateAll(nodes=>nodes.some(el=>!el.childElementCount && el.offsetHeight > 0)),false);
    await shot(page,'plan-conflict');
    await button(form,'Load latest version').click();
    await button(page.getByRole('dialog',{name:'Load latest version'}),'Load latest version').click();
    await page.waitForFunction(()=>document.querySelector('.automation-dialog input').value === 'Updated on host');
    await name.fill('Final plan');
    await button(form,'Save').click();
    await page.getByRole('heading',{name:'Final plan'}).waitFor();
    assert.equal(patches,2);
    await page.getByRole('searchbox',{name:'Search automations'}).fill('no such plan');
    await button(page,'Clear filters').click();
    await page.getByRole('heading',{name:'Final plan'}).waitFor();
}

async function plugins(page) {
    let failing = false, rejectInstall = true, installs = 0;
    const packages = [{source:'npm:reader@1',pages:[],links:[]}];
    await page.route('**/api/harness-plugins', route => {
        const body = route.request().postDataJSON();
        if (body.action === 'library') return route.fulfill(failing ? {status:503,json:{error:'Catalog temporarily offline'}} :
            {json:{profiles:[{profile:'default',runtime:'pi',available:true,packages}]}});
        assert.equal(body.action,'install');
        installs++;
        if (rejectInstall) {failing = true; return route.fulfill({status:503,json:{error:'Install interrupted'}});}
        packages.push({source:body.source,pages:[],links:[]});
        return route.fulfill({json:{}});
    });
    await page.goto(origin+'/capabilities?tab=plugins');
    const library = page.locator('.library:visible');
    await library.getByText('reader',{exact:true}).waitFor();
    failing = true;
    await button(library,'Refresh').click();
    await library.getByRole('alert').waitFor();
    assert.equal(await library.getByText('reader',{exact:true}).isVisible(),true);
    assert.equal(await library.locator('button.add').isDisabled(),true);
    failing = false;
    await button(library,'Refresh').click();
    await library.locator('button.add').click();
    const input = library.locator('form input');
    await input.fill('npm:reader-tools@2');
    await library.locator('form button.primary').click();
    await page.keyboard.press('Escape');
    assert.equal(installs,0);
    await library.locator('form button.primary').click();
    await page.locator('dialog:modal button.primary').click();
    await library.getByText(/Install interrupted/).waitFor();
    assert.equal(await input.inputValue(),'npm:reader-tools@2');
    assert.ok(await library.locator('form button.primary').isDisabled());
    await shot(page,'plugin-install-recovery');
    failing = false; rejectInstall = false;
    await button(library,'Refresh').click();
    await library.locator('form button.primary').click();
    await page.locator('dialog:modal button.primary').click();
    await library.getByText('reader-tools',{exact:true}).waitFor();
    assert.equal(installs,2);
}

async function notifications(page) {
    await page.addInitScript(()=>{
        let permission = 'default';
        window.permissionReply = 'default';
        window.shown = 0;
        class TestNotification {
            static get permission() {return permission;}
            static async requestPermission() { permission = window.permissionReply; return permission; }
            constructor() {window.shown++;}
        }
        Object.defineProperty(window,'Notification',{value:TestNotification});
    });
    await page.goto(origin+'/settings?section=notifications');
    const area = page.locator('[id$=settings-notifications]:visible');
    const toggle = area.getByRole('switch');
    await toggle.click();
    await area.getByRole('alert').filter({hasText:'still off'}).waitFor();
    assert.equal(await toggle.isChecked(),false);
    await page.evaluate(()=>window.permissionReply='granted');
    await toggle.click();
    await area.getByText('Browser notifications allowed').waitFor();
    assert.equal(await toggle.isChecked(),true);
    await button(area,'Send test notification').click();
    await area.getByText('Test notification sent to the browser.').waitFor();
    assert.equal(await page.evaluate(()=>window.shown),1);
    const other = await page.context().newPage();
    await other.goto(origin);
    await other.evaluate(()=>localStorage.setItem('nola.browserNotifications.enabled','false'));
    await page.waitForFunction(()=>!document.querySelector('[id$=settings-notifications] input').checked);
    await other.close();
    await page.evaluate(()=>window.permissionReply='denied');
    await page.reload();
    await page.evaluate(()=>window.permissionReply='denied');
    await toggle.click();
    await area.getByText('Browser notifications blocked').waitFor();
    assert.equal(await area.getByText(/Allow notifications for this site/).count(),1);
    await shot(page,'notification-permission');
}

async function experts(page) {
    await page.goto(origin+'/chat');
    await button(page,'New conversation').first().click();
    await button(page,'Context').first().click();
    const section = page.locator('details[data-inspector-section=participants]:visible');
    if (!await section.evaluate(n=>n.open)) await section.locator('summary').click();
    const profiles = [['broad',['low','high']], ['short',['low']]].map(([id,efforts]) => ({
        id,displayName:id,enabled:true,endpoint:{enabled:true},capabilities:['chat'],
        executionCapabilities:{reasoning:true},reasoning:{control:'effort',efforts},
    }));
    await page.route('**/api/model-profiles', route => route.fulfill({json:{profiles}}));
    await button(section,'Manage experts').click();
    const editor = page.getByRole('dialog',{name:'Manage experts'});
    const name = editor.getByLabel('Agent name',{exact:true});
    const title = 'Workspace editor '+Date.now();
    await name.fill(title);
    const model = editor.getByLabel('Choose model',{exact:true});
    const effort = editor.locator('select').filter({has:page.locator('option[value="automatic"]')});
    await model.selectOption('configured:broad');
    await effort.selectOption('high');
    await model.selectOption('configured:short');
    assert.equal(await effort.inputValue(),'automatic');
    await effort.selectOption('low');
    await model.selectOption('configured:broad');
    assert.equal(await effort.inputValue(),'low');
    await model.selectOption('');
    await button(editor,'Save expert').click();
    const current = editor.locator('.expert-item').filter({hasText:title});
    await current.waitFor();
    await name.fill(title+' pending');
    await current.click();
    assert.equal(await name.inputValue(),title+' pending');
    assert.equal(await name.evaluate(n=>document.activeElement===n),true);
    await editor.locator('.create-btn').click();
    const confirm = page.locator('.confirmation');
    await button(confirm,'Cancel').click();
    assert.equal(await name.inputValue(),title+' pending');
    await page.setViewportSize({width:390,height:774});
    const picker = editor.locator('.expert-mobile-picker select');
    assert.ok(await picker.isVisible());
    const chosenId = await picker.inputValue();
    await picker.selectOption('');
    await button(page.locator('.confirmation'),'Cancel').click();
    assert.equal(await picker.inputValue(),chosenId);
    assert.equal(await name.inputValue(),title+' pending');
    await button(editor,'Save expert').scrollIntoViewIfNeeded();
    assert.ok(await button(editor,'Close').isVisible());
    assert.ok(await editor.locator('header').evaluate(node=>node.getBoundingClientRect().top>=0));
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await name.scrollIntoViewIfNeeded();
    await shot(page,'expert-draft');
    await button(editor,'Save expert').click();
    await editor.locator('.expert-identity-copy strong').getByText(title+' pending',{exact:true}).waitFor();
}

(async()=>{
    if (!origin) throw Error('UI_FIXTURE_URL must point to a disposable Host');
    await fs.mkdir(out,{recursive:true});
    const browser=await chromium.launch({...browserOptions(),headless:true});
    let failed = false;
    try {
        for(const [name,check] of Object.entries({plans,plugins,notifications,experts})){
            const compact = process.env.NOLA_MANAGEMENT_COMPACT === '1';
            const context=await browser.newContext({locale:'en-US',colorScheme:compact?'dark':'light',viewport:{width:compact && name!=='experts' ? 390 : 1100,height:820}});
            const page=await context.newPage();
            page.setDefaultTimeout(6500);
            try {await check(page);console.log('PASS '+name);}
            catch(error){failed=true; console.error(name,error.message); await shot(page,'failure-'+name);}
            finally {await page.context().close();}
        }
    } finally {await browser.close();}
    if(failed) process.exitCode=1;
})().catch(error=>{console.error(error);process.exitCode=1;});
