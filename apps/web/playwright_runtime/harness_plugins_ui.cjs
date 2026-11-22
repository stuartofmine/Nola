// SPDX-License-Identifier: Apache-2.0
// Real package-manager and browser acceptance, using an owned Host and temporary profiles.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const {spawn} = require('node:child_process');
const {chromium} = require('playwright-core');
const {desktopPluginRegistry} = require('./desktop_plugin_fixture.cjs');
const {BrowserUseExecutor} = require('./browser_use.cjs');
const {artifactPath, browserOptions} = require('./test_environment.cjs');
const listen = server => new Promise(resolve => server.listen(0,'127.0.0.1',()=>resolve(server.address().port)));
async function freePort() {
    const s = http.createServer();
    const port = await listen(s);
    await new Promise(resolve=>s.close(resolve));
    return port;
}

(async () => {
    const root = path.resolve(__dirname,'../../..');
    const temp = fs.mkdtempSync(path.join(os.tmpdir(),'nola-plugin-ui-'));
    const db = path.join(temp,'qa.db');
    const packageName = '@deepseek-ai/dsh-experimental-badge-skill-bundle';
    const out = artifactPath('harness-ui');
    fs.mkdirSync(out,{recursive:true});
    let host, context, driver, registry;
    let headers;
    const board = http.createServer((req,res) => {
        if (req.url !== '/board') {
            res.writeHead(404);
            return res.end();
        }
        headers = req.headers;
        res.writeHead(200,{'Content-Type':'text/html; charset=utf-8'});
        res.end(`<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
            <title>Task board</title><style>body{margin:2rem auto;padding:0 1rem;max-width:42rem;font:1rem/1.5 system-ui}
            section{margin:1rem 0;border-top:1px solid #ccc} h2{font-size:1rem;color:#565656}</style>
            <main><h1>Task board</h1><section><h2>To do</h2>Organize interview notes</section>
            <section><h2>In progress</h2>Review research report</section><section><h2>Done</h2>Archive sources</section></main>`);
    });
    try {
        registry = await desktopPluginRegistry(temp);
        const pagePort = await listen(board);
        const dir = path.join(temp,'qa.plugins/harness/dsh');
        fs.mkdirSync(dir,{recursive:true});
        fs.writeFileSync(path.join(dir,'web-pages.json'), JSON.stringify({version:1,pages:[
            {package:packageName,title:'Task board',url:'http://localhost:'+pagePort+'/board'}
        ]}));
        const port = await freePort();
        const base = 'http://127.0.0.1:'+port;
        host = spawn(process.env.NOLA_TEST_CLI || path.join(root,'target/debug/nola'),['web','--host','127.0.0.1','--port',String(port),'--web-root',path.join(root,'apps/web')], {
            cwd:temp,env:{...process.env,HOME:temp,NOLA_HOME:temp,NOLA_DB_PATH:db},stdio:'ignore'
        });
        const deadline = Date.now() + 30000;
        while (true) {
            if (host.exitCode !== null) throw Error('Test Host exited with code '+host.exitCode);
            const ready = await fetch(base+'/api/web/session').catch(()=>undefined);
            if (ready?.ok) break;
            if (Date.now() >= deadline) throw Error('Test Host did not start');
            await new Promise(resolve=>setTimeout(resolve,150));
        }
        const cdp = await freePort();
        context = await chromium.launchPersistentContext(path.join(temp,'browser'),{...browserOptions(),headless:true,args:['--remote-debugging-port='+cdp]});
        driver = await BrowserUseExecutor.attach('http://127.0.0.1:'+cdp,context);
        const p = context.pages()[0];
        const errors = [];
        p.on('pageerror',e=>errors.push(e.message));
        const navigate = url => driver.perform(p,'navigate',{url});
        const input = async (locator,op='click',args={}) => {
            await locator.scrollIntoViewIfNeeded();
            const element = await locator.elementHandle();
            try {await driver.perform(p,op,args,element);} finally {await element.dispose();}
        };
        const button = name => p.getByRole('button',{name,exact:true});
        const shot = name => p.screenshot({path:path.join(out,name+'.png'),animations:'disabled'});
        async function collectionShot(name) {
            const add = button('添加插件');
            await add.waitFor();
            await p.waitForFunction(()=>document.querySelector('.library .add')?.matches(':enabled'));
            await p.mouse.move(0,0);
            await shot(name);
        }
        await context.addCookies([{name:'host_only_test', value:'qa', url:base}]);
        await navigate(base+'/capabilities');
        // Exercise one failed inventory read and a retry against the real Host.
        await p.route('**/api/harness-plugins',route=>route.fulfill({status:503,json:{error:'Host temporarily unavailable'}}),{times:1});
        await input(p.getByRole('link',{name:'插件',exact:true}));
        await p.getByText('Host temporarily unavailable',{exact:true}).waitFor();
        await input(p.getByRole('region',{name:'插件',exact:true}).getByRole('button',{name:'刷新'}));
        const empty = p.getByText('还没有插件。',{exact:true});
        const collection = p.getByRole('list',{name:'插件',exact:true});
        const field = p.getByRole('textbox',{name:'npm 包名'});
        const dialog = p.getByRole('dialog',{name:/^(安装|移除) · /});
        const profileField = p.getByRole('combobox',{name:'安装位置'});
        async function form(profile) {
            await input(button('添加插件'));
            await field.waitFor();
            await profileField.selectOption(profile);
        }
        async function install(source,profile) {
            await form(profile);
            await input(field,'type_text',{text:source});
            await input(button('安装'));
            await dialog.waitFor();
            assert.ok((await dialog.innerText()).includes(source));
            await input(dialog.getByRole('button',{name:'取消'}));
            assert.equal(await field.inputValue(),source);
            await input(button('安装'));
            await input(dialog.getByRole('button',{name:'安装'}));
            await field.waitFor({state:'detached',timeout:150000});
        }
        async function remove(row) {
            const more = row.locator('summary');
            assert.match(await more.getAttribute('aria-label'), /更多操作/);
            await input(more);
            await input(row.getByRole('button',{name:'移除',exact:true}));
            await input(dialog.getByRole('button',{name:'移除'}));
            await row.waitFor({state:'detached',timeout:150000});
        }
        const row = text => collection.getByRole('listitem').filter({hasText:text});
        await empty.waitFor();
        assert.equal(await profileField.count(),0);
        await collectionShot('desktop-plugin-library-empty');
        await form('pi');
        await input(button('取消'));
        await empty.waitFor();
        await install('pi-statusline@0.0.2','pi');
        await row('pi-statusline').waitFor();
        assert.equal(await profileField.count(),0);
        await collectionShot('desktop-pi');
        await p.setViewportSize({width:390,height:840});
        await navigate(base+'/capabilities?tab=plugins');
        await form('web');
        assert.equal(await profileField.isEnabled(),true);
        await p.mouse.move(0,0);
        await shot('mobile-plugin-add');
        await input(button('取消'));
        await install(packageName+'@0.2.2-rc.5','web');
        await row(packageName).waitFor();
        assert.equal(await collection.getByRole('listitem').count(),2);
        assert.match(await row(packageName).innerText(),/DeepSeek Harness · Web/);
        await collectionShot('mobile-dsh-web');
        const popup = context.waitForEvent('page');
        await input(row(packageName).getByRole('link',{name:/Task board/}));
        const boardPage = await popup;
        await boardPage.getByRole('heading',{name:'Task board'}).waitFor();
        assert.equal(await boardPage.evaluate(()=>window.opener),null);
        assert.ok(headers);
        assert.equal(headers.referer,undefined);
        assert.equal(headers.cookie,undefined);
        await boardPage.screenshot({path:path.join(out,'plugin-workspace.png')});
        await boardPage.close();
        const dshProfiles = await p.evaluate(async () => {
            const response = await fetch('/api/harness-plugins',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:'library'})});
            return (await response.json()).profiles.filter(profile=>profile.runtime==='dsh');
        });
        assert.equal(dshProfiles.find(profile=>profile.profile==='tui').packages.length,0);
        assert.equal(dshProfiles.find(profile=>profile.profile==='web').packages.length,1);
        await remove(row(packageName));
        assert.equal(await collection.getByRole('listitem').count(),1);
        await p.setViewportSize({width:1440,height:920});
        await navigate(base+'/capabilities?tab=plugins');
        const nav = p.getByRole('navigation',{name:'插件页面'});
        const boardLink = nav.getByRole('link',{name:'工作看板', exact:true});
        assert.equal(await boardLink.count(),0);
        await install(registry.name+'@1.0.0','pi');
        await boardLink.waitFor({timeout:90000});
        const managerLink = collection.getByRole('link',{name:'工作看板',exact:true});
        await managerLink.waitFor();
        await collectionShot('desktop-plugin-library');
        await input(managerLink);
        const frame = p.frameLocator('iframe[title="工作看板"]').frameLocator('iframe[title="工作看板"]');
        await frame.getByRole('heading',{name:'本周工作'}).waitFor();
        const pageURL = p.url();
        await p.goBack();
        await managerLink.waitFor();
        await input(boardLink);
        await frame.getByRole('button',{name:'完成校对'}).click();
        await frame.getByText('研究报告已校对').waitFor();
        assert.equal(context.pages().length,1);
        assert.equal(new URL(p.url()).origin,base);
        const exposed = await frame.locator('body').evaluate(()=> {
            try {return window.parent.document.title;} catch {return null;}
        });
        assert.equal(exposed,null);
        await shot('desktop-installed-workspace');
        await p.reload();
        await frame.getByRole('heading',{name:'本周工作'}).waitFor();
        await p.setViewportSize({width:390,height:840});
        await navigate(base+'/capabilities?tab=plugins');
        await input(collection.getByRole('link',{name:'工作看板',exact:true}));
        await frame.getByRole('heading',{name:'本周工作'}).waitFor();
        await frame.getByRole('button',{name:'完成校对'}).click();
        await frame.getByText('研究报告已校对').waitFor();
        await shot('mobile-installed-workspace');
        await p.goBack();
        await collection.getByRole('link',{name:'工作看板',exact:true}).waitFor();
        await p.setViewportSize({width:1440,height:920});
        await navigate(base+'/capabilities?tab=plugins');
        await remove(row(registry.name));
        await boardLink.waitFor({state:'detached'});
        await remove(row('pi-statusline'));
        await empty.waitFor();
        await navigate(pageURL);
        await p.getByText('此插件页面已不可用。',{exact:true}).waitFor();
        assert.equal(await p.locator('iframe[title="工作看板"]').count(),0);
        const forbidden = await fetch(base+'/api/harness-plugins',{method:'POST',body:'{"runtime":"pi","action":"list"}'});
        assert.equal(forbidden.status,403);
        assert.deepEqual(errors,[]);
        console.log('Real Pi/DSH install, profile isolation, external page, bundled desktop page navigation, interaction, reload and removal passed.');
    } finally {
        if(registry) await registry.close();
        if(driver) await driver.close();
        if(context) await context.close();
        if(host && host.exitCode===null) host.kill();
        await new Promise(resolve=>board.close(resolve));
        fs.rmSync(temp,{recursive:true,force:true});
    }
})().catch(error=>{console.error(error);process.exitCode=1;});
