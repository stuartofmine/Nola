// SPDX-License-Identifier: Apache-2.0
// Exercise the built desktop UI against deterministic repository and transfer responses.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require('playwright-core');
const { browserOptions, artifactPath } = require('./test_environment.cjs');

(async () => {
    const origin = process.env.UI_FIXTURE_URL;
    if (!origin) throw new Error('UI_FIXTURE_URL must point to an isolated Nola Web Host.');
    const evidence = process.env.UI_EVIDENCE_DIR || artifactPath('desktop-model-acquisition');
    await fs.mkdir(evidence, { recursive: true });
    const browser = await chromium.launch({ ...browserOptions(), headless: true });
    try {
        const page = await browser.newPage({ viewport: { width: 1050, height: 840 }, locale: 'en' });
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        const models = [];
        const installs = [];
        const lookups = [];
        let gatewayReads = 0;
        page.on('request', request => {
            if (new URL(request.url()).pathname === '/api/model-gateway/access') ++gatewayReads;
        });
        let job = null;
        let revision = 'a001';
        let setups = 0;
        let denySetup = false;
        const engines = {
            'llama.cpp':{engine:'llama.cpp',ready:false,canInstall:true,reason:'installable',guideUrl:'https://github.com/ggml-org/llama.cpp'},
            vllm:{engine:'vllm',ready:true,canInstall:false,reason:'ready',guideUrl:'https://docs.vllm.ai/en/latest/getting_started/installation/'}
        };
        await page.route('**/api/local-models/engine-status?*', route => {
            const id = new URL(route.request().url()).searchParams.get('engine');
            return route.fulfill({json:{engineStatus:engines[id]}});
        });
        await page.route('**/api/local-models/setup-engine', async route => {
            assert.equal(route.request().postDataJSON().engine,'llama.cpp');
            ++setups;
            if (denySetup) return route.fulfill({status:403,json:{error:'Open Nola on the host.',code:'install_on_host'}});
            await new Promise(resolve=>setTimeout(resolve,300));
            if (setups === 1) return route.fulfill({status:503,json:{error:'Test installer interrupted'}});
            engines['llama.cpp'] = {...engines['llama.cpp'],ready:true,canInstall:false,reason:'ready'};
            return route.fulfill({json:{engineStatus:engines['llama.cpp']}});
        });
        const hardware = {cpu:'Test CPU', cores:8, gpu:'Test GPU', memoryBytes:32*1024**3,
            gpuMemoryBytes:12*1024**3, unifiedMemory:false, freeDiskBytes:80e9};
        await page.route('**/api/local-models/hardware', route => route.fulfill({json:{hardware}}));
        await page.route('**/api/local-models', route => route.fulfill({ json: {
            models, acquisition: job, runtimes: [
                {id: 'vllm', binaryAvailable: true}, {id: 'llama.cpp', binaryAvailable: false}
            ], binaryAvailable: false
        }}));
        await page.route('**/api/local-models/recommend', async route => {
            const body = route.request().postDataJSON();
            lookups.push(body);
            if (!body.query && body.source === 'huggingface') await new Promise(resolve=>setTimeout(resolve,600));
            if (body.query === 'context plans') {
                const candidates = [
                    ['unsloth/Demo-GGUF','Demo-UD-Q4_K_M.gguf',131072,null,'target_met'],
                    ['vendor/Demo-GGUF','Demo-UD-Q4_K_M.gguf',32768,32768,'native_limit'],
                    ['mirror/Small-GGUF','Model-Q4_K_M.gguf',8192,131072,'memory_budget'],
                    ['vendor/Unknown-GGUF','Model-f16.gguf',2048,null,'metadata_unknown']
                ].map(([repository,variant,contextTokens,nativeContextTokens,contextLimitReason]) => ({
                    repository, variant, contextTokens,nativeContextTokens,contextLimitReason,targetContextTokens:131072,
                    downloadBytes:2e9,estimatedMemoryBytes:5e9,revision:'abc',
                    plan:{repository,variant,engine:'llama.cpp',files:[{path:variant}]}
                }));
                return route.fulfill({json:{hardware,candidates,recommendationSource:'mixed'}});
            }
            if (body.query) {
                assert.equal(body.engine, 'vllm');
                assert.equal(body.source, 'modelscope');
            }
            if (body.query === 'timeout') return route.fulfill({status: 502, json: {error: 'ModelScope temporarily unavailable'}});
            const ids = body.query === 'none' ? [] : !body.query ? [body.source === 'modelscope' ? 'best/Fit' : 'old/Result'] : body.query.includes('/') ? [body.query] : ['demo/Tiny', 'other/Tiny'];
            return route.fulfill({json:{hardware, recommendationSource: body.query ? 'search' : 'modelfit',
                candidates:ids.map(repository => ({repository, revision, variant:'F16', downloadBytes:5e6,
                    estimatedMemoryBytes:15e6, contextTokens:2048, plan:{repository,revision,contextTokens:2048}}))}});
        });
        await page.route('**/api/local-models/install-plan', route => {
            const body = route.request().postDataJSON();
            installs.push(body);
            job = {...body, id: String(installs.length), status:'downloading', downloadedBytes: 123456, totalBytes: 5000000};
            return route.fulfill({status: 202, json: {job}});
        });
        await page.route('**/api/local-models/cancel', route => {
            assert.equal(route.request().postDataJSON().id, job.id);
            job = {...job, status:'cancelled'};
            return route.fulfill({json: {job}});
        });
        const openingRecommendation = page.waitForRequest(request=>request.url().endsWith('/api/local-models/recommend'));
        await page.goto(origin + '/models?tab=local');
        const engine = page.getByLabel('Inference engine');
        const source = page.getByRole('group',{name:'Download source'});
        const sourceButton = id => source.getByRole('button',{name:id === 'modelscope' ? 'ModelScope' : 'Hugging Face',exact:true});
        const input = page.getByRole('searchbox', {name:'Model name'});
        const submit = page.getByRole('button', {name: 'Search models'});
        await openingRecommendation;
        await engine.selectOption('vllm');
        await sourceButton('modelscope').click();
        await page.locator('details.hardware > summary').click();
        await page.getByText('Test CPU · 8 cores').waitFor();
        await page.getByText('Test GPU',{exact:true}).waitFor();
        await page.getByText('32 GiB', {exact:true}).waitFor();
        await page.getByText('12 GiB', {exact:true}).waitFor();
        await page.getByRole('link', {name:'ModelFit'}).waitFor();
        await page.getByText('best/Fit').waitFor();
        assert.equal(installs.length, 0);
        const initialLooks = lookups.length;
        assert.equal(gatewayReads, 0, 'opening local models must not load the hidden API model gateway');
        const gatewayRead = page.waitForRequest(request => new URL(request.url()).pathname === '/api/model-gateway/access', {timeout:10000});
        await page.getByRole('tab',{name:'API models'}).click();
        await gatewayRead;
        await page.getByRole('tab',{name:'Local models'}).click();
        await page.getByText('best/Fit').waitFor();
        await new Promise(resolve=>setTimeout(resolve,650));
        assert.equal(gatewayReads, 1, 'the API model panel stays mounted on a tab switch');
        assert.equal(lookups.length, initialLooks, 'returning to the local tab reuses its suggestions');
        assert.equal(await page.getByText('old/Result').count(), 0, 'stale results cannot overwrite the chosen source');
        await engine.selectOption('llama.cpp');
        const setup = page.getByRole('region',{name:'Runtime status'});
        const install = setup.getByRole('button',{name:'Install llama.cpp',exact:true});
        await install.waitFor();
        assert.equal(await setup.getByRole('link',{name:'Official setup guide'}).getAttribute('href'),'https://github.com/ggml-org/llama.cpp');
        await page.context().route('https://github.com/ggml-org/llama.cpp',route => route.fulfill({contentType:'text/html',body:'<title>Official guide fixture</title>'}));
        const popup = page.waitForEvent('popup');
        await setup.getByRole('link',{name:'Official setup guide'}).click();
        const guideWindow = await popup;
        await guideWindow.waitForURL('https://github.com/ggml-org/llama.cpp');
        await guideWindow.close();
        await install.click();
        await setup.getByText('Installing llama.cpp…').waitFor();
        assert.equal(await install.isDisabled(),true);
        await setup.getByText('Test installer interrupted').waitFor();
        await install.click();
        await setup.getByText('llama.cpp is ready',{exact:true}).waitFor();
        assert.equal(setups,2);
        assert.equal(installs.length,0);
        engines['llama.cpp'] = {...engines['llama.cpp'],ready:false,canInstall:false,reason:'homebrew_missing',guideUrl:'https://brew.sh/'};
        const recheck = setup.getByRole('button',{name:'Check again'});
        await recheck.click();
        await setup.getByRole('link',{name:'Get Homebrew'}).waitFor();
        for (const [reason, text] of [['invalid_override','The configured engine path is invalid.'],['install_on_host','Open Nola on the host']]) {
            engines['llama.cpp'] = {...engines['llama.cpp'],reason,guideUrl:'https://github.com/ggml-org/llama.cpp'};
            await recheck.click();
            await setup.getByText(text,{exact:false}).waitFor();
            assert.equal(await install.count(),0);
        }
        await setup.scrollIntoViewIfNeeded();
        await page.screenshot({path:path.join(evidence,'engine-setup-remote.png')});
        await setup.getByRole('button',{name:'Switch to vLLM'}).click();
        await setup.getByText('vLLM is ready',{exact:true}).waitFor();
        engines.vllm = {...engines.vllm,ready:false,reason:'unsupported'};
        await recheck.click();
        await setup.getByText('This host cannot launch vLLM.',{exact:false}).waitFor();
        await setup.getByRole('button',{name:'Connect a vLLM server'}).click();
        assert.equal(await page.getByRole('tab',{name:'API models'}).getAttribute('aria-selected'),'true');
        const serverForm = page.getByRole('dialog');
        await serverForm.waitFor();
        assert.equal(await serverForm.getByLabel('Endpoint name').inputValue(),'vLLM');
        assert.equal(await serverForm.getByLabel('Base URL').inputValue(),'');
        await page.screenshot({path:path.join(evidence,'engine-connect-vllm.png')});
        await serverForm.getByText('Cancel',{exact:true}).click();
        await page.getByRole('tab',{name:'Local models'}).click();
        assert.equal(await install.count(),0);
        await input.fill('timeout'); await submit.click();
        await page.getByRole('alert').filter({hasText:'ModelScope temporarily unavailable'}).waitFor();
        await sourceButton('huggingface').click();
        await input.fill('');
        const automatic = page.waitForRequest(req => req.url().endsWith('/api/local-models/recommend') && req.postDataJSON().source === 'huggingface');
        await automatic;
        await sourceButton('modelscope').click();
        await input.fill('none'); await submit.click();
        await page.getByText('No matching models for vLLM').waitFor();
        await input.fill('Tiny'); await submit.click();
        const row = page.locator('.acquisition li').filter({hasText: 'other/Tiny'});
        await row.waitFor();
        assert.equal(installs.length, 0, 'ambiguous name must leave candidates for the user');
        await row.getByRole('button', {name:'Confirm download'}).click();
        await page.getByRole('progressbar').waitFor();
        await page.reload();
        await page.getByRole('progressbar').waitFor();
        assert.equal(await engine.inputValue(), 'vllm');
        assert.equal(await sourceButton('modelscope').getAttribute('aria-pressed'), 'true');
        assert.deepEqual(installs[0], {engine:'vllm', source:'modelscope', repository:'other/Tiny',plan:{repository:'other/Tiny',revision:'a001',contextTokens:2048}});
        await page.screenshot({path: path.join(evidence, 'windows-downloading.png')});
        await page.locator('.download-banner').getByRole('button', {name:'Cancel'}).click();
        await page.getByText('Download cancelled', {exact:true}).waitFor();
        revision = 'a002';
        const review = page.locator('.download-banner').getByRole('button', {name:'Review download plan'});
        await review.click({timeout:4000});
        const reviewed = page.getByRole('button', {name:'Confirm download · other/Tiny', exact:true});
        await reviewed.waitFor();
        assert.equal(installs.length, 1, 'reviewing a cancelled transfer must not start it');
        assert.deepEqual(lookups.at(-1), {engine:'vllm', source:'modelscope', query:'other/Tiny'});
        await reviewed.click();
        await page.getByRole('progressbar').waitFor();
        assert.equal(installs[1].plan.revision, 'a002');
        job = {...job, status:'failed', error: 'Transfer interrupted'};
        await page.getByText('Transfer interrupted').waitFor();
        await engine.selectOption('llama.cpp');
        await sourceButton('huggingface').click();
        await review.click();
        await reviewed.waitFor();
        assert.equal(await engine.inputValue(), 'vllm');
        assert.equal(await sourceButton('modelscope').getAttribute('aria-pressed'), 'true');
        assert.equal(installs.length, 2);
        await page.screenshot({path:path.join(evidence, 'review-failed-download.png')});
        await input.fill('demo/Tiny'); await submit.click();
        const confirm = page.getByRole('button',{name:'Confirm download · demo/Tiny',exact:true});
        await confirm.waitFor();
        assert.equal(installs.length, 2, 'single exact match still requires confirmation');
        await page.screenshot({path:path.join(evidence, 'windows-install-plan.png')});
        await confirm.click();
        await page.getByRole('progressbar').waitFor();
        assert.equal(installs[2].repository, 'demo/Tiny', 'confirmed exact result starts the download');
        const parameters = { contextTokens: 2048, temperature: .7, topP: .9, topK: 40, minP: .05, gpuLayers: 0, threads: 0, batchSize: 128, flashAttention: false };
        models.push({ id:'demo-tiny', name:'demo/Tiny', runtime:'vllm', format:'safetensors', installed:true,
            binaryAvailable:true, contextWindowTokens:2048, minimumContextWindowTokens:1, configurationRevision:0, parameters });
        job = {...job, status:'installed'};
        const installed = page.locator('.model-row').filter({hasText:'demo/Tiny'});
        await installed.waitFor();
        assert.equal(await installed.getByRole('button', {name:'Switch & use'}).count(), 0, 'an offline model can be installed before its engine');
        await installed.getByRole('button',{name:'Set up engine'}).click();
        assert.equal(await setup.evaluate(node => node === document.activeElement),true);
        engines.vllm = {...engines.vllm,ready:true,reason:'ready'};
        await recheck.click();
        await setup.getByText('vLLM is ready',{exact:true}).waitFor();
        assert.equal(await installed.getByRole('button', {name:'Switch & use'}).isEnabled(), true);
        await installed.getByRole('button', {name:'Configure'}).click();
        const dialog = page.getByRole('dialog');
        await dialog.waitFor();
        assert.equal(await dialog.getByLabel('Context window').getAttribute('min'), '1');
        assert.equal(await dialog.getByText('Batch size', {exact:true}).count(), 0);
        await dialog.getByRole('button', {name:'Cancel'}).click();
        await page.setViewportSize({width:390, height:840});
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
        await installed.scrollIntoViewIfNeeded();
        await page.screenshot({path: path.join(evidence, 'windows-installed.png')});
        await page.setViewportSize({width:1250,height:1040});
        await engine.selectOption('llama.cpp');
        for (const lang of ['en','zh']) {
            if (lang === 'zh') {
                await page.addInitScript(() => localStorage.setItem('nola.web.locale','zh'));
                await page.reload();
            }
            const zh = lang === 'zh';
            await page.getByRole('searchbox',{name:zh?'模型名称':'Model name'}).fill('context plans');
            await page.getByRole('button',{name:zh?'搜索模型':'Search models'}).click();
            const candidates = page.locator('.acquisition li');
            const unsloth = candidates.filter({hasText:'unsloth/Demo-GGUF'});
            await candidates.filter({hasText:'vendor/Unknown-GGUF'}).waitFor();
            for (const summary of await candidates.locator('summary').all()) await summary.click();
            await unsloth.getByText('131,072',{exact:true}).waitFor();
            assert.equal(await unsloth.getByRole('link').getAttribute('href'),'https://modelscope.cn/models/unsloth/Demo-GGUF');
            await unsloth.getByText('Unsloth Dynamic · Demo-UD-Q4_K_M.gguf',{exact:true}).waitFor();
            assert.equal(await page.getByText(/^Unsloth Dynamic/).count(),1);
            await page.getByText(zh ? '上下文目标 128K（131,072 tokens）' : 'Context target 128K (131,072 tokens)',{exact:true}).waitFor();
            await page.getByText(zh?/原生上下文上限为 32,768/:/model supports up to 32,768/).waitFor();
            await page.getByText(zh?/本机可用内存估算需缩短/:/estimated memory budget requires/).waitFor();
            await page.getByText(zh?/元数据不足，按保守值/:/Incomplete model metadata/).waitFor();
            await page.getByRole('link',{name:'ModelFit',exact:true}).waitFor();
            await page.getByRole('link',{name:'ModelScope',exact:true}).waitFor();
            await unsloth.scrollIntoViewIfNeeded();
            await page.screenshot({path:path.join(evidence,'context-plan-'+lang+'.png')});
        }
        engines['llama.cpp'] = {...engines['llama.cpp'],ready:false,canInstall:true,reason:'installable'};
        await page.getByRole('region',{name:'运行状态'}).getByRole('button',{name:'重新检测'}).click();
        await page.getByRole('button',{name:'安装 llama.cpp',exact:true}).waitFor();
        await page.getByRole('region',{name:'运行状态'}).scrollIntoViewIfNeeded();
        await page.screenshot({path:path.join(evidence,'engine-setup-zh.png')});
        denySetup = true;
        await page.getByRole('button',{name:'安装 llama.cpp',exact:true}).click();
        await page.getByText('请在宿主机本机安装引擎，再回到这里重新检测。',{exact:false}).waitFor();
        assert.equal(await page.getByRole('button',{name:'安装 llama.cpp',exact:true}).count(),0);
        engines['llama.cpp'] = {...engines['llama.cpp'],canInstall:false,reason:'manual'};
        engines.vllm = {...engines.vllm,ready:false,canInstall:false,reason:'manual'};
        await page.addInitScript(() => {window.NolaDesktop = {nativeRuntime:async()=>({backend:'vulkan',architecture:'x64'})};});
        await page.reload();
        await page.getByText('Windows 安装包包含 llama.cpp。',{exact:false}).waitFor();
        await page.getByLabel('推理引擎').selectOption('vllm');
        await page.getByText('包括在 WSL 中启动的服务',{exact:false}).waitFor();
        await page.getByRole('button',{name:'连接 vLLM 服务'}).waitFor();
        await page.screenshot({path:path.join(evidence,'engine-windows-vllm.png')});
        assert.deepEqual(errors, []);
        console.log('PASS: engine install failure/retry, official guide, recheck, remote authorization, Windows repair and WSL service, offline model setup, hardware recommendations, plan confirmation, transfer and selection.');
    } finally { await browser.close(); }
})().catch(error => {console.error(error); process.exitCode = 1;});
