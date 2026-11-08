// SPDX-License-Identifier: Apache-2.0
// Isolated component interaction checks; no Host, credentials or model requests.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const path=require('node:path');
const fs=require('node:fs');
const {pathToFileURL}=require('node:url');
(async()=>{
    const frontend=path.resolve(__dirname,'../frontend');
    const {createServer}=await import(pathToFileURL(path.join(frontend,'node_modules/vite/dist/node/index.js')).href);
    const fixture='\0reasoning-keyboard-fixture';
    const server=await createServer({root:frontend,server:{host:'127.0.0.1',port:0},plugins:[{
        name:'owned-keyboard-fixture',
        resolveId(id){if(id.endsWith('/__keyboard-entry.js'))return fixture;},
        load(id){if(id===fixture)return `import {mount,unmount} from 'svelte';
            import Menu from '/src/components/ChatReasoningMenu.svelte';
            import {locale} from '/src/lib/i18n.ts';
            locale.set('en'); window.selections=[];
            const state={preference:'medium',options:['none','low','medium','high','xhigh'],ready:true,busy:false,errorKey:''};
            let app; window.resetMenu=async(busy=false)=>{if(app)await unmount(app);app=mount(Menu,{target:document.querySelector('#menu'),props:{state:{...state,busy},onselect:value=>window.selections.push(value)}})};
            await window.resetMenu(); window.ready=true;`;},
        configureServer(vite){vite.middlewares.use(async(req,res,next)=>{
            if(!req.url?.split('?')[0].endsWith('/__keyboard'))return next();
            const html=`<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>
                :root{--muted:#b0bdb3;--content:#e0e8e2;--canvas:#101713;--surface-gradient:#1b2820;--border:#4b5d50;--accent-subtle:#304a39;--surface-shadow:none}
                body{margin:0;background:var(--canvas);color:var(--content);font:16px system-ui} #dock{display:flex;justify-content:flex-end;position:fixed;bottom:16px;inset-inline:16px;gap:8px}
                #menu{display:flex;justify-content:flex-end;width:240px} button,summary{outline-offset:-2px} :focus-visible{outline:2px solid #9cc9ab}
                </style></head><body><div id="dock"><div id="menu"></div><button id="outside">Outside</button></div><script type="module" src="/ui/__keyboard-entry.js"></script></body></html>`;
            res.setHeader('Content-Type','text/html');res.end(await vite.transformIndexHtml(req.url,html));
        });}
    }]});
    let browser;const cases=[];
    const out=process.env.NOLA_KEYBOARD_OUT||'/tmp/nola-reasoning-keyboard';fs.mkdirSync(out,{recursive:true});
    try {
        await server.listen();const base='http://127.0.0.1:'+server.httpServer.address().port;
        browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
        for(const [width,height,dir]of[[1000,700,'ltr'],[320,480,'ltr'],[320,480,'rtl']]){
            const page=await browser.newPage({viewport:{width,height},reducedMotion:'reduce'});
            const errors=[];page.on('pageerror',e=>errors.push(e.message));
            await page.goto(base+'/ui/__keyboard');await page.waitForFunction(()=>window.ready);
            await page.evaluate(d=>document.documentElement.dir=d,dir);
            const trigger=page.locator('summary'),choices=page.locator('.choices button');
            await trigger.focus();await page.keyboard.press('ArrowDown');
            await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-pressed')==='true');
            assert.equal(await page.evaluate(()=>window.selections.length),0,'navigation must not select');
            await page.keyboard.press('End');assert.equal(await choices.last().evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('ArrowDown');assert.equal(await choices.first().evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('ArrowUp');assert.equal(await choices.last().evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('Home');assert.equal(await choices.first().evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('Escape');assert.equal(await trigger.evaluate(e=>e===document.activeElement),true);
            assert.equal(await page.locator('details').getAttribute('open'),null);
            await page.keyboard.press('ArrowUp');await page.waitForFunction(()=>document.activeElement?.getAttribute('aria-pressed')==='true');
            await page.keyboard.press('ArrowDown');await page.keyboard.press('Enter');
            assert.deepEqual(await page.evaluate(()=>window.selections),['high']);
            assert.equal(await trigger.evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('ArrowDown');await page.keyboard.press('Control+Home');
            assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('aria-pressed')), 'true','modified navigation remains native');
            await page.keyboard.press('Escape');await trigger.click();await page.locator('#outside').click();
            assert.equal(await page.locator('details').getAttribute('open'),null);
            await trigger.focus();await page.keyboard.press('ArrowDown');await page.keyboard.press('End');
            const bounds=await page.locator('.choices').boundingBox();assert.ok(bounds.x>=0&&bounds.x+bounds.width<=width+.5);
            await page.screenshot({path:path.join(out,`menu-${width}-${dir}.png`)});
            await page.keyboard.press('Escape');await page.evaluate(()=>window.resetMenu(true));
            await trigger.focus();await page.keyboard.press('ArrowDown');assert.equal(await page.locator('details').getAttribute('open'),null);
            assert.deepEqual(errors,[]);cases.push({width,height,dir,passed:true});await page.close();
        }
        fs.writeFileSync(path.join(out,'receipt.json'),JSON.stringify({passed:true,cases},null,2));console.log(JSON.stringify({passed:true,cases,out}));
    } finally {if(browser)await browser.close();await server.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
