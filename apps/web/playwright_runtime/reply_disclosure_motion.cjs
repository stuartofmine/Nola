// Actual reply components; isolated fixture, native <details>, no external requests.
const { chromium } = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
(async () => {
    const frontend = path.resolve(__dirname, '../frontend');
    const output = process.env.UI_EVIDENCE_DIR || require('./test_environment.cjs').artifactPath('nola-reply-disclosure');
    await fs.mkdir(output, { recursive: true });
    const fixture = await fs.mkdtemp(path.join(frontend, '.disclosure-fixture-'));
    let server, browser, context;
    try {
        await fs.writeFile(path.join(fixture, 'index.html'), '<html><meta charset="utf-8"><div id="app"></div><script type="module" src="./main.js"></script></html>');
        await fs.writeFile(path.join(fixture, 'main.js'), "import {mount} from 'svelte';import App from './App.svelte';import {locale} from '../src/lib/i18n';locale.set('zh');mount(App,{target:document.querySelector('#app')});");
        await fs.writeFile(path.join(fixture, 'App.svelte'), `<script>
            import {onMount} from 'svelte';
            import ChatExecutionProcess from '../src/components/ChatExecutionProcess.svelte';
            import {replyDisclosure} from '../src/lib/replyDisclosure';
            import '../src/theme.css';
            let expanded=$state(false), choices=$state({}), focusOpen=$state(false);
            let parts=$state([{source:'canonical',id:'reason',value:{kind:'reasoning',part_id:'reason',revision:1,state:'completed',summary:'先核对输入，再检查实际运行结果。\\n\\n已显示的正文保持稳定，详情沿阅读方向展开。'}},
                {source:'canonical',id:'tool',value:{kind:'tool',execution_source:{kind:'model'},part_id:'tool',revision:1,state:'completed',title:'读取文件',summary:'已读取内容',input:{path:'example.txt'},output:'真实结果仅用于本地回放。'}}]);
            onMount(()=>{window.disclosures={closeFocus:()=>focusOpen=false,append:()=>{parts=[{...parts[0],value:{...parts[0].value,summary:parts[0].value.summary+' 新增进度。'}},parts[1]];}};});
            </script><main><p id="before">回复正文保持稳定。</p>
            <ChatExecutionProcess {parts} live={false} {choices} expandedChoice={expanded}
                onprocessexpandedchange={next=>expanded=next} onexpandedchange={(id,next)=>choices={...choices,[id]:next}} />
            <p id="after">后续内容随详情高度自然移动。</p>
            <details use:replyDisclosure id="focus" bind:open={focusOpen}><summary>焦点与键盘验证</summary><div style="height:100px"><button id="inside">复制</button></div></details>
            </main><style>main{max-width:640px;margin:48px auto;padding:16px}#before,#after{line-height:28px} :global(body){min-height:100vh;margin:0}</style>`);
        const {createServer} = await import(pathToFileURL(path.join(frontend,'node_modules/vite/dist/node/index.js')));
        server=await createServer({root:frontend,logLevel:'error',server:{host:'127.0.0.1',port:0}});await server.listen();
        browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
        context=await browser.newContext({reducedMotion:'no-preference',colorScheme:process.env.UI_COLOR_SCHEME==='dark'?'dark':'light',viewport:{width:840,height:720},recordVideo:{dir:output,size:{width:840,height:720}}});
        const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));
        await page.route('**/*',route=>new URL(route.request().url()).hostname==='127.0.0.1'?route.continue():route.abort());
        await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/ui/${path.basename(fixture)}/index.html`);
        await page.waitForFunction(()=>window.disclosures);
        assert.ok(await page.evaluate(()=>CSS.supports('interpolate-size','allow-keywords')));
        const disclosure=page.locator('.execution-process'),summary=disclosure.locator(':scope > summary');
        const height=()=>disclosure.evaluate(node=>node.getBoundingClientRect().height);
        const closed=await height(),prefix=await page.locator('#before').boundingBox();
        // Pause real CSS transitions at a known fraction. Build load must not
        // turn an IPC/wall-clock delay into a false motion regression.
        const holdNextToggle=()=>page.evaluate(()=>{
            const root=document.querySelector('.execution-process');
            window.heldReplyAnimations=[];
            const clicked=event=>{
                if(!root.querySelector(':scope > summary').contains(event.target))return;
                document.removeEventListener('click',clicked);
                queueMicrotask(()=>{
                    root.getBoundingClientRect();
                    window.heldReplyAnimations=root.getAnimations({subtree:true});
                    for(const animation of window.heldReplyAnimations){
                        const duration=animation.effect.getTiming().duration;
                        animation.pause();animation.currentTime=typeof duration==='number'?duration*.35:60;
                    }
                });
            };
            document.addEventListener('click',clicked);
        });
        const resume=()=>page.evaluate(()=>{for(const animation of window.heldReplyAnimations)animation.play();});
        await holdNextToggle();await summary.click();
        await page.waitForFunction(()=>window.heldReplyAnimations.length>0);
        const opening=await height();await resume();
        await page.waitForTimeout(220);const open=await height();
        assert.ok(opening>closed && opening<open,`opening must have an intermediate height: ${closed}, ${opening}, ${open}`);
        assert.deepEqual(await page.locator('#before').boundingBox(),prefix);
        await page.screenshot({path:path.join(output,'reply-open.png')});
        const tool=disclosure.locator('.tool-part');await tool.locator(':scope > summary').click();await page.waitForTimeout(220);
        assert.ok(await tool.evaluate(node=>node.open));
        await holdNextToggle();await summary.click();
        await page.waitForFunction(()=>window.heldReplyAnimations.length>0);
        const closing=await height();await resume();
        await page.waitForTimeout(220);assert.equal(await height(),closed);
        assert.ok(closing>closed,'closing content must remain rendered during height reduction');
        await summary.focus();await page.keyboard.press('Enter');await page.waitForTimeout(45);
        await page.keyboard.press('Enter');await page.waitForTimeout(45);
        await page.keyboard.press('Enter');await page.waitForTimeout(220);
        assert.ok(await disclosure.evaluate(node=>node.open),'rapid reversal must remain interactive');
        // A new body revision while already open must not restart the disclosure.
        await page.evaluate(()=>window.disclosures.append());await page.waitForTimeout(30);
        assert.equal(await disclosure.evaluate(node=>node.getAnimations({subtree:true}).filter(a=>['height','block-size','opacity'].includes(a.transitionProperty)).length),0);
        await page.locator('#focus > summary').click();await page.waitForTimeout(220);await page.locator('#inside').focus();
        await page.evaluate(()=>window.disclosures.closeFocus());
        await page.waitForFunction(()=>document.querySelector('#focus > summary')===document.activeElement);
        await page.keyboard.press('Tab');
        assert.equal(await page.locator('#inside').evaluate(node=>node===document.activeElement),false,
            'closing descendants must leave the keyboard path before the visual transition ends');
        await page.waitForTimeout(220);
        await page.emulateMedia({reducedMotion:'reduce'});await summary.click();await page.waitForTimeout(30);
        assert.equal(await height(),closed);
        assert.equal(await disclosure.evaluate(node=>node.getAnimations({subtree:true}).length),0);
        await page.setViewportSize({width:390,height:844});await summary.click();await page.waitForTimeout(30);
        assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
        assert.deepEqual(errors,[]);
        const video=page.video();await context.close();context=null;await video.saveAs(path.join(output,'reply-disclosure.webm'));
        console.log('PASS: opening/closing intermediate frames, nested tools, stable prefix, keyboard reversal, no stream replay, focus recovery, reduced motion and compact layout');
    } finally {if(context)await context.close();if(browser)await browser.close();if(server)await server.close();await fs.rm(fixture,{recursive:true,force:true});}
})().catch(error=>{console.error(error);process.exitCode=1;});
