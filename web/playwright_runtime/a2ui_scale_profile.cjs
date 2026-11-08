// Deterministic production renderer baseline, not a model/Core/FFI measurement.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
(async () => {
    const root = path.resolve(__dirname, '../app/static/ui-tests');
    const out = fs.mkdtempSync('/tmp/nola-a2ui-chrome-scale-');
    const server = http.createServer((request, response) => {
        const file = path.resolve(root, '.' + new URL(request.url, 'http://localhost').pathname.replace(/^\/review-tests/, ''));
        if (!file.startsWith(root + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { response.writeHead(404).end(); return; }
        response.setHeader('Content-Type', file.endsWith('.js') ? 'application/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
        fs.createReadStream(file).pipe(response);
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const browser = await chromium.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true,
        args: ['--enable-precise-memory-info'] });
    try {
        if (process.env.NOLA_A2UI_HTTP_STATE) {
            const assert = require('node:assert/strict');
            const crypto = require('node:crypto');
            const {execFileSync} = require('node:child_process');
            const state = JSON.parse(fs.readFileSync(process.env.NOLA_A2UI_HTTP_STATE));
            assert.equal(new URL(state.url).hostname,'127.0.0.1');
            assert.ok(path.basename(state.root).startsWith('nola-a2ui-real-'));
            const cid = process.env.NOLA_A2UI_HTTP_CONVERSATION;
            assert.ok(cid);
            const context = await browser.newContext();
            const page = await context.newPage();
            const errors=[], denied=[];
            page.on('pageerror',e=>errors.push(e.message));
            await page.route('**/api/**', async route => {
                const request=route.request(), pathname=new URL(request.url()).pathname;
                const read = request.method()==='GET' || request.method()==='POST' && /\/ui-surface-(revision|changes)$/.test(pathname);
                if (!read) {denied.push({path:pathname,method:request.method()});await route.abort();}
                else await route.continue();
            });
            // Serve only the existing renderer measurement entry/assets under
            // the Host origin. API queries below reach the real isolated Host.
            await page.route('**/review-tests/**', async route => {
                const file=path.resolve(root,'.'+new URL(route.request().url()).pathname.replace(/^\/review-tests/,''));
                assert.ok(file.startsWith(root+path.sep));
                await route.fulfill({path:file,contentType:file.endsWith('.js')?'application/javascript':file.endsWith('.css')?'text/css':'text/html'});
            });
            const messages=await context.request.get(`${state.url}/api/chat/conversations/${encodeURIComponent(cid)}/messages`);
            assert.equal(messages.status(),200);
            const projection=(await messages.json()).conversationProjection;
            assert.equal(projection.active_run,null,'Only a sealed conversation may be measured');
            assert.equal(projection.messages.at(-1).state,'completed');
            const response=await context.request.get(`${state.url}/api/chat/conversations/${encodeURIComponent(cid)}/ui-surfaces`);
            assert.equal(response.status(),200);
            const snapshot=(await response.json()).surfaces.find(s=>s.identity.surface_id==='implementation-progress');
            assert.ok(snapshot?.revision>1);
            assert.ok(Object.values(snapshot.nodes).every(n=>!Object.keys(n.actions).length && !Object.keys(n.bindings).length),
                'Historical read-only baseline is suitable only for this noninteractive completed card');
            const session=await context.newCDPSession(page), system=await browser.newBrowserCDPSession();
            await session.send('Performance.enable');
            await page.goto(`${state.url}/review-tests/tests/a2ui-scale.html`);
            await page.waitForFunction(()=>window.a2uiScale);
            const rows=[];
            for (const width of [1280,390]) {
                await page.setViewportSize({width,height:900});
                for(let sample=0;sample<5;sample++) {
                    const measurement=await page.evaluate(({identity,base})=>window.a2uiScale.remoteCycle(identity,base),{identity:snapshot.identity,base:snapshot.revision-1});
                    const metrics=await session.send('Performance.getMetrics');
                    const info=await system.send('SystemInfo.getProcessInfo');
                    const ids=info.processInfo.map(p=>p.id).filter(Number.isSafeInteger);
                    const rss=execFileSync('ps',['-p',ids.join(','),'-o','pid=,rss='],{encoding:'utf8'}).trim().split('\n').filter(Boolean).map(line=>{const [pid,kib]=line.trim().split(/\s+/).map(Number);return {pid,rss_bytes:kib*1024};});
                    rows.push({width,sample,...measurement,heap_used_bytes:metrics.metrics.find(m=>m.name==='JSHeapUsedSize')?.value,
                        chrome_process_rss_bytes:rss.reduce((sum,p)=>sum+p.rss_bytes,0),process_rss:rss,dom:await session.send('Memory.getDOMCounters')});
                    fs.writeFileSync(`${out}/samples.json`,JSON.stringify(rows,null,2));
                }
                await page.screenshot({path:`${out}/${width}.png`,fullPage:true});
            }
            const sources=['web/frontend/tests/A2uiScale.svelte','web/playwright_runtime/a2ui_scale_profile.cjs','third_party/a2ui/production-adaptation.json',
                ...fs.readdirSync(path.join(root,'assets')).map(f=>`web/app/static/ui-tests/assets/${f}`)];
            const fingerprints=Object.fromEntries(sources.map(file=>[file,crypto.createHash('sha256').update(fs.readFileSync(path.resolve(__dirname,'../..',file))).digest('hex')]));
            const stats=values=>({median:[...values].sort((a,b)=>a-b)[Math.floor(values.length/2)],min:Math.min(...values),max:Math.max(...values)});
            const summary={kind:'readonly-real-http-to-production-renderer-baseline',browser:await browser.version(),identity:snapshot.identity,
                revision:snapshot.revision,base_revision:snapshot.revision-1,errors,denied,fingerprints,
                summaries:[1280,390].map(width=>{const r=rows.filter(r=>r.width===width);return {width,samples:r.length,node_count:r[0].node_count,
                    historical_body_bytes:r[0].historical.body_bytes,changes_body_bytes:r[0].changes.body_bytes,changes_type:r[0].changes.type,
                    mount_http_to_frame_ms:stats(r.map(x=>x.mount_http_to_frame_ms)),delta_http_to_frame_ms:stats(r.map(x=>x.delta_http_to_frame_ms)),
                    historical_network_ms:stats(r.map(x=>x.historical.network_ms)),changes_network_ms:stats(r.map(x=>x.changes.network_ms)),
                    historical_transfer_bytes:r[0].historical.transfer_size,changes_transfer_bytes:r[0].changes.transfer_size,
                    frame_interval_ms:stats(r.flatMap(x=>[...x.mount.frame_intervals_ms,...x.delta.frame_intervals_ms])),
                    mount_dom_ms:stats(r.map(x=>x.mount.commit_ms)),delta_dom_ms:stats(r.map(x=>x.delta.commit_ms)),
                    heap_used_bytes:stats(r.map(x=>x.heap_used_bytes)),chrome_process_rss_bytes:stats(r.map(x=>x.chrome_process_rss_bytes)),overflow:r.some(x=>x.overflow)};}),
                limitations:['Only historical revision and changes queries of a sealed real card; no submission, admission, write transaction, model, or FFI timing measured.',
                    'Existing measurement fixture mounts production Surface/theme; it does not include Chat history hydration or app navigation.',
                    'HTTP-to-frame includes fetch/body parse, Svelte flush/layout and two rAF callbacks; rAF is not proof of GPU presentation.',
                    'Headless Chrome, five sequential samples per viewport, concurrent native work, no threshold pass asserted.',
                    'RSS sums sampled Chrome-reported process PIDs; shared pages can be counted more than once, not unique physical footprint or peak. Heap is sampled and GC uncontrolled.',
                    'Original historical snapshot strips actions/bindings; this completed text-only card has neither, verified before measurement.']};
            fs.writeFileSync(`${out}/summary.json`,JSON.stringify(summary,null,2));
            assert.deepEqual(errors,[]);assert.deepEqual(denied,[]);
            assert.ok(rows.every(r=>!r.overflow),'real long-card horizontal viewport overflow');
            console.log(JSON.stringify({evidence:out,...summary}));
            return;
        }
        const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
        const session = await page.context().newCDPSession(page);
        await session.send('Performance.enable');
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.goto(`http://127.0.0.1:${server.address().port}/review-tests/tests/a2ui-scale.html`);
        await page.waitForFunction(() => window.a2uiScale);
        const rows = [];
        for (const count of [10, 100, 1000]) {
            for (let sample = 0; sample < 10; sample++) {
                const mount = await page.evaluate(count => window.a2uiScale.mount(count), count);
                const input = page.locator('input');
                await input.waitFor();
                await page.waitForFunction(() => !document.querySelector('input').disabled);
                await input.focus();
                const delta = await page.evaluate(() => window.a2uiScale.delta());
                await page.evaluate(() => {
                    window.inputSamples = [];
                    const input = document.querySelector('input');
                    input.addEventListener('input', () => {
                        const at = performance.now(), value = input.value;
                        requestAnimationFrame(() => window.inputSamples.push({event_to_frame_ms: performance.now() - at,
                            value, focused: document.activeElement === input, identity: document.querySelector('input') === input}));
                    });
                });
                const started = performance.now();
                await page.keyboard.type('baseline');
                await page.waitForFunction(() => window.inputSamples.length === 8);
                const keyboardRoundtrip = performance.now() - started;
                const inputSamples = await page.evaluate(() => window.inputSamples);
                const memory = await session.send('Performance.getMetrics');
                const dom = await session.send('Memory.getDOMCounters');
                const row = {count, sample, mount, delta, keyboard_roundtrip_ms: keyboardRoundtrip, input_samples: inputSamples,
                    heap_used_bytes: memory.metrics.find(metric => metric.name === 'JSHeapUsedSize')?.value, dom};
                rows.push(row);
                fs.writeFileSync(`${out}/samples.json`, JSON.stringify(rows, null, 2));
            }
        }
        const quantile = (values, p) => [...values].sort((a,b) => a-b)[Math.min(values.length-1, Math.floor(values.length*p))];
        const summaries = [10,100,1000].map(count => {
            const rowsAtScale = rows.filter(row => row.count === count);
            const stat = values => ({median: quantile(values,.5),p95:quantile(values,.95),max:Math.max(...values)});
            return {count, samples: rowsAtScale.length, snapshot_bytes: rowsAtScale[0].mount.snapshot_bytes,
                mount_commit_ms: stat(rowsAtScale.map(row => row.mount.commit_ms)),
                delta_commit_ms: stat(rowsAtScale.map(row => row.delta.commit_ms)),
                frame_interval_ms: stat(rowsAtScale.flatMap(row => [...row.mount.frame_intervals_ms,...row.delta.frame_intervals_ms])),
                input_event_to_frame_ms: stat(rowsAtScale.flatMap(row => row.input_samples.map(value => value.event_to_frame_ms))),
                sampled_heap_used_bytes: stat(rowsAtScale.map(row => row.heap_used_bytes)),
                identity_or_focus_errors: rowsAtScale.filter(row => !row.delta.input_identity_preserved || !row.delta.focus_preserved || row.input_samples.some(value => !value.identity || !value.focused)).length};
        });
        const report = {browser:await browser.version(),viewport:{width:1280,height:900},kind:'baseline-not-threshold-pass',summaries,errors,
            limitations:['Headless Chrome on this machine; concurrent native tests may contend for CPU.',
                'Projection fixtures bypass Rust admission, persistence, transport and FFI.',
                'rAF gaps measure callback cadence, not GPU presentation or every main-thread task.',
                'Input interval begins at DOM input event; excludes hardware/OS dispatch latency. Keyboard roundtrip includes CDP automation.',
                'CDP JS heap is sampled, includes fixture and framework, is not peak RSS; GC is uncontrolled and DOM counters can include detached nodes.',
                'Ten sequential samples per scale; first sample includes cold component caches. No performance acceptance threshold asserted.']};
        fs.writeFileSync(`${out}/summary.json`,JSON.stringify(report,null,2));
        console.log(JSON.stringify({evidence:out,...report}));
    } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error.message);process.exitCode=1; });
