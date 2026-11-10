// Production renderer regression; no model, credentials or external network.
const { chromium } = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
(async () => {
    const frontend = path.resolve(__dirname, '../frontend');
    const output = process.env.UI_EVIDENCE_DIR || '/tmp/nola-water-surface';
    await fs.mkdir(output, { recursive: true });
    const fixture = await fs.mkdtemp(path.join(frontend, '.water-fixture-'));
    let server, browser, context;
    try {
        await fs.writeFile(path.join(fixture, 'index.html'), '<html><meta charset="utf-8"><div id="app"></div><script type="module" src="./main.js"></script></html>');
        await fs.writeFile(path.join(fixture, 'main.js'), `import {mount} from 'svelte'; import App from './App.svelte'; mount(App,{target:document.querySelector('#app')});`);
        await fs.writeFile(path.join(fixture, 'App.svelte'), `<script>
            import {onMount} from 'svelte';
            import Markdown from '../src/components/Markdown.svelte';
            import '../src/theme.css';
            let text=$state('已显示的文字保持稳定。 '), streaming=$state(true);
            const initial=text;
            onMount(()=>{window.water={append:(delta)=>text+=delta, finish:()=>streaming=false,
                replace:(next)=>{text=next; streaming=true;}};});
            </script><main><p class="label">Nola · 流式浮现</p><div id="ink"><Markdown source={text} {streaming} initialText={initial}/></div></main>
            <style>:global(html){background:#191919 !important}:global(body){margin:0;min-height:100vh;background:#191919;color:#e8e8e8}main{max-width:660px;margin:72px auto;padding:0 24px} .label{font-size:13px;color:#aaa;margin-bottom:28px}#ink{font-size:20px;line-height:1.8}</style>`);
        const { createServer } = await import(pathToFileURL(path.join(frontend, 'node_modules/vite/dist/node/index.js')));
        server = await createServer({ root: frontend, logLevel: 'error', server: { host: '127.0.0.1', port: 0 } });
        await server.listen();
        const port = server.httpServer.address().port;
        browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
        context = await browser.newContext({ viewport: { width: 920, height: 580 }, recordVideo: { dir: output, size: { width: 920, height: 580 } } });
        const page = await context.newPage(), errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.route('**/*', route => new URL(route.request().url()).hostname === '127.0.0.1' ? route.continue() : route.abort());
        await page.goto(`http://127.0.0.1:${port}/ui/${path.basename(fixture)}/index.html`);
        await page.waitForFunction(() => window.water);
        const prefix = await page.locator('#ink').textContent();
        const shape = () => page.locator('#ink').evaluate(root => {
            const walker = document.createTreeWalker(root.querySelector('p'), NodeFilter.SHOW_TEXT);
            let node; while ((node = walker.nextNode()) && node.textContent.length < 8) {}
            const range = document.createRange(); range.setStart(node, 0); range.setEnd(node, 8);
            const r = range.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
        });
        const before = await shape();
        const active = await page.evaluate(async () => {
            window.water.append('新的文字，像从水面缓缓浮现，逐渐清晰，轻轻落在阅读的位置。');
            await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
            return Array.from(document.querySelectorAll('#ink .fresh'), el => {
                const style = getComputedStyle(el); return { opacity: +style.opacity, top: parseFloat(style.top), filter: style.filter };
            });
        });
        assert.ok(active.some(style => style.opacity < .9 && style.top > 0 && style.filter !== 'none'), 'real DOM must rise from soft ink');
        assert.deepEqual(await shape(), before, 'settled prefix must not move');
        assert.ok((await page.locator('#ink').textContent()).startsWith(prefix));
        await page.screenshot({ path: path.join(output, 'web-water-active.png') });
        await page.waitForTimeout(620);
        assert.equal(await page.locator('#ink .fresh').count(), 0);
        await page.screenshot({ path: path.join(output, 'web-water-settled.png') });
        // Record a paced stream so the demo contains a continuous moving front.
        for (const delta of ['\n\n', '像', '水面', '一样，', '新文字', '轻轻', '浮起，', '逐渐', '变得', '清晰。', ' ', '已显示', '的文字', '保持', '稳定，', '阅读', '不被', '打断。']) {
            await page.evaluate(delta => window.water.append(delta), delta);
            await page.waitForTimeout(65);
        }
        await page.waitForTimeout(620);
        // Select across settled and active nodes; settling must preserve selected text.
        await page.evaluate(async () => {
            window.water.append(' Selection remains stable.');
            await new Promise(resolve => requestAnimationFrame(resolve));
            const root = document.querySelector('#ink p'), range = document.createRange(); range.selectNodeContents(root);
            getSelection().removeAllRanges(); getSelection().addRange(range);
        });
        const selected = await page.evaluate(() => getSelection().toString());
        await page.waitForTimeout(620);
        assert.equal(await page.evaluate(() => getSelection().toString()), selected);
        await page.evaluate(() => getSelection().removeAllRanges());
        const finishing = await page.evaluate(async () => {
            window.water.append(' 最后一行也自然收住。');
            await new Promise(resolve => requestAnimationFrame(resolve));
            window.water.finish();
            await new Promise(resolve => requestAnimationFrame(resolve));
            return document.querySelectorAll('#ink .fresh').length;
        });
        assert.ok(finishing > 0, 'parent Markdown must retain the finishing renderer');
        await page.waitForTimeout(200);
        assert.equal(await page.locator('#ink .fresh').count(), 0);
        await page.evaluate(() => window.water.replace('替换后的内容立即可读。'));
        await page.waitForTimeout(30);
        assert.equal(await page.locator('#ink .fresh').count(), 0);
        await page.evaluate(() => window.water.append(' reduced motion is immediate'));
        await page.emulateMedia({ reducedMotion: 'reduce' });
        await page.waitForFunction(() => document.querySelectorAll('#ink .fresh').length === 0);
        await page.emulateMedia({ reducedMotion: 'no-preference' });
        await page.waitForTimeout(30);
        assert.equal(await page.locator('#ink .fresh').count(), 0, 'changing preference must not replay old ink');
        await page.setViewportSize({ width: 390, height: 844 });
        await page.evaluate(() => window.water.append(' 中文、👨‍👩‍👧‍👦、é 和很长的连续文字也保持正常换行。'.repeat(20)));
        await page.waitForTimeout(30);
        assert.ok(await page.locator('#ink .fresh').count() <= 32);
        assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
        const activeHeight = await page.locator('#ink').evaluate(root => root.getBoundingClientRect().height);
        await page.waitForTimeout(620);
        assert.equal(await page.locator('#ink .fresh').count(), 0);
        assert.equal(await page.locator('#ink').evaluate(root => root.getBoundingClientRect().height), activeHeight,
            'merging settled spans must preserve the laid-out text height');
        assert.deepEqual(errors, []);
        const video = page.video();
        await context.close(); context = null;
        await video.saveAs(path.join(output, 'water-surface.webm'));
        console.log('PASS: actual water frames, stable prefix, completion, selection, replacement, reduced motion, compact wrapping and bounded burst');
    } finally {
        if (context) await context.close();
        if (browser) await browser.close();
        if (server) await server.close();
        await fs.rm(fixture, { recursive: true, force: true });
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
