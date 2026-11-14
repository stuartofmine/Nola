// SPDX-License-Identifier: Apache-2.0
// Actual Svelte reply components. Browser Use performs all admitted input.
const {chromium} = require('playwright-core');
const {BrowserUseExecutor} = require('./browser_use.cjs');
const env = require('./test_environment.cjs');
const fs = require('node:fs/promises'), os = require('node:os'), path = require('node:path'), net = require('node:net'), assert = require('node:assert/strict');
const {pathToFileURL} = require('node:url');
(async () => {
    const frontend = path.resolve(__dirname, '../frontend');
    const fixture = await fs.mkdtemp(path.join(frontend,'.reference-fixture-'));
    const profile = await fs.mkdtemp(path.join(os.tmpdir(),'nola-web-references-'));
    const out = env.artifactPath('cross-reference-polish/web'); await fs.mkdir(out,{recursive:true});
    let server, context, executor;
    const report = {kind:'actual-components-synthetic-data',input:'browser-use',cases:[],errors:[]};
    try {
        await fs.writeFile(path.join(fixture,'index.html'), '<html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><div id="app"></div><script type="module" src="./main.js"></script></html>');
        await fs.writeFile(path.join(fixture,'main.js'), "import {mount} from 'svelte';import App from './App.svelte';import {locale} from '../src/lib/i18n';locale.set('zh-Hans');mount(App,{target:document.querySelector('#app')});");
        await fs.writeFile(path.join(fixture,'App.svelte'), `<script>
            import ChatMessageContent from '../src/components/ChatMessageContent.svelte';import '../src/theme.css';
            const part=(id,value)=>({source:'canonical',id,value:{part_id:id,revision:1,...value}});
            const sources=Array.from({length:31},(_,i)=>part('source-'+i,{kind:'source',source_id:'private-id-'+i,title:'跨平台交互与引用格式参考 '+(i+1),uri:'https://example.com/'+i}));
            const parts=[part('tool',{kind:'tool',execution_source:{kind:'model'},state:'completed',title:'读取来源',input:{},output:'已读取'}),part('answer',{kind:'text',markdown:'正文说明 [来源](https://example.com/paragraph)\\n\\n| 时间 | 事件 | 经过和影响 |\\n| --- | --- | --- |\\n| 阶段一 | 整理资料 | 保留正文阅读层级。[来源](https://example.com/table-one) |\\n| 阶段二 | 比较记录 | 引用紧随所支持的句子。[Source](https://example.com/table-two) |'}),part('skill',{kind:'skill',skill_id:'conduct-deep-research',display_name:'conduct-deep-research',state:'running',model_turns:8,tool_calls:10,elapsed_millis:116945}),...sources];
            let owner=$state('first-owner'),message=$state({id:'reply',role:'assistant',state:'running',canonical:null,parts});
            </script><main><nav><button id="finish" onclick={()=>message={...message,state:'completed'}}>结束本轮</button><button id="owner" onclick={()=>{owner='second-owner';message={...message,id:'second-reply'};}}>切换会话</button></nav><ChatMessageContent {message} ownerKey={owner} /></main>
            <style>:global(body){margin:0;background:var(--canvas);color:var(--content)}main{max-width:680px;margin:auto;padding:20px}nav{display:flex;gap:16px;margin-bottom:20px}nav button{min-height:44px}</style>`);
        const {createServer} = await import(pathToFileURL(path.join(frontend,'node_modules/vite/dist/node/index.js')));
        server = await createServer({root:frontend,logLevel:'error',server:{host:'127.0.0.1',port:0}}); await server.listen();
        const base = `http://127.0.0.1:${server.httpServer.address().port}`;
        const socket=net.createServer();await new Promise(resolve=>socket.listen(0,'127.0.0.1',resolve));const port=socket.address().port;await new Promise(resolve=>socket.close(resolve));
        context = await chromium.launchPersistentContext(profile,{...env.browserOptions(),headless:true,reducedMotion:'reduce',viewport:{width:1000,height:900},args:[`--remote-debugging-port=${port}`]});
        const page=context.pages()[0];page.setDefaultTimeout(10000);page.on('pageerror',error=>report.errors.push(error.message));
        await context.route('**/*',route=>new URL(route.request().url()).origin===base?route.continue():route.abort());
        executor=await BrowserUseExecutor.attach(`http://127.0.0.1:${port}`,context);
        const click=async locator=>{const element=await locator.elementHandle();assert.ok(element);try{await executor.perform(page,'click',{},element);}finally{await element.dispose();}};
        for(const [width,theme] of [[1000,'light'],[390,'dark'],[320,'light']]) {
            await page.setViewportSize({width,height:900});await page.emulateMedia({colorScheme:theme});
            await executor.perform(page,'navigate',{url:base+'/ui/'+path.basename(fixture)+'/index.html'});
            const summary=page.locator('.execution-process>summary');await summary.waitFor();
            assert.equal(await page.locator('.markdown td a.inline-reference').count(),2);
            const inlineStyle=await page.locator('.markdown td a.inline-reference').first().evaluate(element=>({font:parseFloat(getComputedStyle(element).fontSize),color:getComputedStyle(element).color,href:element.getAttribute('href')}));
            assert.ok(inlineStyle.font<=13.1);assert.match(inlineStyle.color,/rgb\((\d+), \1, \1\)/);
            assert.equal(inlineStyle.href,'https://example.com/table-one');
            assert.match(await summary.innerText(),/正在处理中/);
            await click(summary);assert.match(await summary.innerText(),/正在处理中/);
            await click(page.locator('#finish'));assert.match(await summary.innerText(),/执行过程/);
            assert.equal(await page.locator('.skill-summary>small').innerText(),'未完成');
            assert.equal(await page.locator('.citation-row').count(),3);
            await click(page.locator('.bibliography>button'));assert.equal(await page.locator('.citation-row').count(),31);
            await click(page.locator('.bibliography>button'));assert.equal(await page.locator('.citation-row').count(),3);
            const text=await page.locator('main').innerText();assert.doesNotMatch(text,/private-id|116945|conduct-deep-research/);
            assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
            await page.screenshot({path:path.join(out,`${width}-${theme}.png`),fullPage:true});
            await click(page.locator('.bibliography>button'));await click(page.locator('#owner'));
            assert.equal(await page.locator('.citation-row').count(),3,'A different message owner starts compact');
            report.cases.push({width,theme});
        }
        assert.deepEqual(report.errors,[]);await fs.writeFile(path.join(out,'receipt.json'),JSON.stringify(report,null,2));console.log('PASS 3 actual Web reply cases through Browser Use');
    } finally {if(executor)await executor.close();if(context)await context.close();if(server)await server.close();await fs.rm(fixture,{recursive:true,force:true});await fs.rm(profile,{recursive:true,force:true});}
})().catch(error=>{console.error(error);process.exitCode=1;});
