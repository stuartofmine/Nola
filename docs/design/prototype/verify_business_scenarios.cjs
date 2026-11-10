// SPDX-License-Identifier: Apache-2.0
// Exercises only the standalone prototype. All external requests are blocked.
const {chromium} = require('../../../web/playwright_runtime/node_modules/playwright-core');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const base = process.env.PROTOTYPE_URL || 'http://127.0.0.1:8071';
const out = process.env.PROTOTYPE_OUT || '/tmp/nola-prototype-business-20261110/qa';
const catalog = JSON.parse(fs.readFileSync(path.join(__dirname,'BUSINESS-SCENARIOS.json')));
const report = {kind:'prototype-only',scenarios:catalog.scenarios.length,states:0,platformStates:0,transitions:0,unavailable:0,screens:[],errors:[],externalRequests:[],checks:[]};
function contrast(a,b) {
    const linear = text => text.match(/[\d.]+/g).slice(0,3).map(v=>{v=Number(v)/255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;}).reduce((v,n,i)=>v+n*[.2126,.7152,.0722][i],0);
    const x=linear(a),y=linear(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);
}
(async () => {
    fs.mkdirSync(out,{recursive:true});
    const browser = await chromium.launch({executablePath:process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
    let page;
    try {
        for (const platform of Object.keys(catalog.platforms)) for (const theme of ['light','dark']) {
            const mobile=['ios','android'].includes(platform);
            const context=await browser.newContext({viewport:{width:mobile?390:1440,height:1000},colorScheme:theme,reducedMotion:'reduce'});
            await context.route('**/*',route=>new URL(route.request().url()).origin===new URL(base).origin?route.continue():(report.externalRequests.push(route.request().url()),route.abort()));
            page=await context.newPage();page.setDefaultTimeout(8000);
            page.on('pageerror',error=>report.errors.push({platform,theme,error:error.message}));
            await page.goto(`${base}/scenarios.html?platform=${platform}&theme=${theme}`);
            await page.locator('#device[data-scene]').waitFor();
            const colors=await page.evaluate(()=>{const style=getComputedStyle(document.documentElement);const color=key=>{const e=document.createElement('span');e.style.color=style.getPropertyValue(key);document.body.append(e);const value=getComputedStyle(e).color;e.remove();return value;};return Object.fromEntries(['--ink','--muted','--canvas','--accent','--on-accent','--warn','--warn-bg','--error','--error-bg'].map(key=>[key,color(key)]));});
            for(const [a,b] of [['--ink','--canvas'],['--muted','--canvas'],['--accent','--on-accent'],['--warn','--warn-bg'],['--error','--error-bg']])assert.ok(contrast(colors[a],colors[b])>=4.5,`${platform}/${theme}: contrast ${a} ${b}`);
            for (const scene of catalog.scenarios) {
                // Navigate through real links, reopening the mobile directory when needed.
                if (mobile && !await page.locator('#catalog').isVisible()) await page.locator('#catalog-toggle').click();
                await page.locator(`a[data-scene="${scene.id}"]`).click();
                assert.equal(await page.locator('#device').getAttribute('data-scene'),scene.id);
                if (!scene.platforms.includes(platform)) {
                    assert.equal(await page.locator('#device').getAttribute('data-available'),'false');
                    assert.equal(await page.locator('#device [data-transition]').count(),0);
                    report.unavailable++;continue;
                }
                for (const state of scene.states) {
                    await page.locator(`button[data-state="${state.id}"]`).click();
                    assert.equal(await page.locator('#device').getAttribute('data-state'),state.id);
                    assert.equal(await page.locator('#device h3').first().innerText(),state.title);
                    const overflow=await page.evaluate(()=>({document:document.documentElement.scrollWidth>innerWidth+1,device:document.querySelector('#device').scrollWidth>document.querySelector('#device').clientWidth+1}));
                    assert.deepEqual(overflow,{document:false,device:false},`${platform}/${theme}/${scene.id}/${state.id} overflow`);
                    const targets=await page.locator('#device .actions button').evaluateAll(elements=>elements.map(e=>({height:e.getBoundingClientRect().height,padding:parseFloat(getComputedStyle(e).paddingLeft)})));
                    assert.ok(targets.every(t=>t.height>=(mobile?47:35)&&t.padding>=10),'Action hit areas and insets');
                    if (scene.id==='permission-missing' && state.id==='0') {
                        const disabled=await page.locator('#device .permission-disabled button').evaluateAll(elements=>elements.every(e=>e.disabled));assert.ok(disabled);
                    }
                    if (platform===scene.platforms[0] && (theme==='light' || state.id==='0')) {
                        const file=`${platform}-${theme}-${scene.id}-${state.id}.png`;
                        await page.locator('#device').screenshot({path:path.join(out,file)});
                        report.screens.push({file,scene:scene.id,state:state.id,platform,theme});
                    }
                    report.platformStates++;
                }
                // Exercise every edge including confirmation/cancel, not only state selectors.
                if (platform===scene.platforms[0] && theme==='light') {
                    for (const state of scene.states) for (let index=0;index<state.actions.length;index++) {
                        await page.locator(`button[data-state="${state.id}"]`).click();
                        await page.locator(`[data-transition="${index}"]`).click();
                        if (state.actions[index].confirm) {
                            await page.locator('#confirm[open]').waitFor();
                            await page.locator('#confirm-cancel').click();
                            assert.equal(await page.locator('#device').getAttribute('data-state'),state.id);
                            await page.locator(`[data-transition="${index}"]`).click();
                            await page.locator('#confirm-apply').click();
                        }
                        assert.equal(await page.locator('#device').getAttribute('data-state'),state.actions[index].to,`${scene.id}: ${state.title} → ${state.actions[index].label}`);
                        report.transitions++;
                    }
                    report.states+=scene.states.length;
                }
            }
            console.log(JSON.stringify({stage:'platform-reviewed',platform,theme,platformStates:report.platformStates}));
            await context.close();
        }
        const context=await browser.newContext({viewport:{width:320,height:844},reducedMotion:'reduce'});
        await context.route('**/*',route=>new URL(route.request().url()).origin===new URL(base).origin?route.continue():(report.externalRequests.push(route.request().url()),route.abort()));
        page=await context.newPage();page.setDefaultTimeout(8000);page.on('pageerror',error=>report.errors.push({error:error.message}));
        // Small phone + large text: every applicable state, both themes, no horizontal page overflow.
        for(const theme of ['light','dark']) {
            await page.goto(`${base}/scenarios.html?scene=host-pair&platform=android&theme=${theme}`);
            await page.locator('#device[data-scene]').waitFor();
            await page.getByText('预览与验证边界',{exact:true}).click();
            await page.locator('#large-text').check();
            for(const scene of catalog.scenarios.filter(s=>s.platforms.includes('android'))) {
                if(!await page.locator('#catalog').isVisible())await page.locator('#catalog-toggle').click();
                await page.locator(`a[data-scene="${scene.id}"]`).click();
                for(const state of scene.states) {
                    await page.locator(`button[data-state="${state.id}"]`).click();
                    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`320px large text ${scene.id}/${state.id}`);
                }
            }
        }
        report.checks.push('320px Android large text: all applicable states in both themes');
        // Required field, failed save draft, scenario isolation, cancel baseline and history.
        await page.goto(`${base}/scenarios.html?scene=api-config&platform=android&theme=dark`);await page.locator('[data-field="0"]').waitFor();
        await page.locator('[data-field="0"]').fill('');await page.locator('[data-transition="0"]').click();
        assert.equal(await page.locator('#device').getAttribute('data-state'),'0');assert.match(await page.locator('#field-error').innerText(),/请填写/);
        await page.locator('[data-field="0"]').fill('保留的服务草稿');await page.locator('[data-transition="0"]').click();
        await page.locator('button[data-state="0"]').click();assert.equal(await page.locator('[data-field="0"]').inputValue(),'保留的服务草稿');
        await page.locator('#catalog-toggle').click();await page.locator('a[data-scene="goal-input"]').click();
        await page.locator('[data-field="0"]').fill('另一个场景的目标');
        await page.locator('#catalog-toggle').click();await page.locator('a[data-scene="api-config"]').click();
        assert.equal(await page.locator('[data-field="0"]').inputValue(),'保留的服务草稿');
        await page.getByRole('button',{name:'取消并保留原状',exact:true}).click();
        assert.equal(await page.locator('[data-field="0"]').inputValue(),'演示服务');
        await page.locator('button[data-state="1"]').click();await page.goBack();assert.equal(await page.locator('#device').getAttribute('data-state'),'0');
        report.checks.push('required input, retained error draft, isolated scenarios, cancel baseline, browser history');
        await page.locator('#catalog-toggle').click();await page.locator('#search').fill('不存在的业务词');assert.equal(await page.locator('a[data-scene]').count(),0);
        await page.locator('#search').fill('宿主机');assert.ok(await page.locator('a[data-scene]').count()>0);await page.locator('#search').fill('');
        report.checks.push('search no results and recovery');
        // Multiple independent inspector sections retain an edited Wiki draft.
        await page.goto(`${base}/scenarios.html?scene=inspector&platform=mac&theme=dark`);await page.locator('[data-wiki]').waitFor();
        await page.locator('[data-wiki]').fill('保留的 Wiki 草稿');
        const details=page.locator('#device details').filter({has:page.locator('[data-wiki]')});await details.locator('summary').click();await details.locator('summary').click();
        assert.equal(await page.locator('[data-wiki]').inputValue(),'保留的 Wiki 草稿');
        await page.locator('button[data-state="1"]').click();assert.equal(await page.locator('[data-wiki]').inputValue(),'保留的 Wiki 草稿');
        report.checks.push('independent inspector sections and retained draft');
        await page.goto(`${base}/scenarios.html?scene=permission&platform=android&theme=dark`);
        await page.getByText('完整操作与授权范围',{exact:true}).click();
        assert.match(await page.locator('.operation-review pre').innerText(),/Command: python3 remote_job.py/);
        await page.goto(`${base}/scenarios.html?scene=mcp-config&platform=mac&theme=light`);
        await page.locator('#mcp-transport').selectOption('stdio');
        assert.match(await page.locator('.transport-note').innerText(),/不会创建进程/);
        await page.goto(`${base}/scenarios.html?scene=general-fonts&platform=ios&theme=light`);
        await page.locator('#font-preview').selectOption('serif');
        assert.equal(await page.locator('.font-specimen.serif').count(),1);
        report.checks.push('permission operation details, MCP transport and font specimen');
        assert.equal(report.states,catalog.scenarios.reduce((n,s)=>n+s.states.length,0));
        assert.equal(report.externalRequests.length,0);assert.equal(report.errors.length,0);
        report.passed=true;
        await context.close();
    } catch(error) {
        report.passed=false;report.failure=String(error.stack||error);
        if(page&&!page.isClosed())await page.screenshot({path:path.join(out,'failure.png')}).catch(()=>{});
        process.exitCode=1;
    } finally {
        fs.writeFileSync(path.join(out,'receipt.json'),JSON.stringify(report,null,2));
        fs.writeFileSync(path.join(out,'index.html'),`<!doctype html><meta charset="utf-8"><title>Nola 业务原型图册</title><style>body{font:14px system-ui;margin:24px;background:#f2f4f0;color:#202620}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:24px}img{width:100%;height:420px;object-fit:contain;object-position:top;background:white}a{color:inherit}h2{font-size:16px}</style><h1>Nola 业务场景图册</h1><p>合成原型，不是生产截图。${report.scenarios} 个场景，状态及验证结果见 receipt.json。</p><main>${report.screens.map(s=>`<section><h2>${catalog.scenarios.find(scene=>scene.id===s.scene)?.title || s.scene} · ${s.state} · ${s.platform} · ${s.theme}</h2><a href="${s.file}"><img loading="lazy" src="${s.file}" alt="${s.scene} ${s.state}"></a></section>`).join('')}</main>`);
        await browser.close();
    }
    console.log(JSON.stringify({passed:report.passed,scenarios:report.scenarios,states:report.states,platformStates:report.platformStates,transitions:report.transitions,screens:report.screens.length,failure:report.failure,output:out}));
})().catch(error=>{console.error(error);process.exitCode=1;});
