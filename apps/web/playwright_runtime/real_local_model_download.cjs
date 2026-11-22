// SPDX-License-Identifier: Apache-2.0
// Live ModelScope smoke test. Uses a temporary real Host, database and model library.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { setTimeout: sleep } = require('node:timers/promises');
const { chromium } = require('playwright-core');
const { artifactPath, browserOptions } = require('./test_environment.cjs');

async function freePort() {
    const socket = http.createServer();
    await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
    const port = socket.address().port;
    await new Promise(resolve => socket.close(resolve));
    return port;
}

(async () => {
    const chinese = process.env.NOLA_TEST_LOCALE === 'zh';
    const label = (en, zh) => chinese ? zh : en;
    const screenshot = name => artifactPath('desktop-model-acquisition/' + name + (chinese ? '-zh' : '') + '.png');
    const root = path.resolve(__dirname, '../../..');
    const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'nola-live-model-'));
    const models = path.join(temp, 'models');
    const repo = 'ggml-org/test-model-stories260K';
    const query = repo.split('/')[1];
    const port = await freePort();
    const origin = 'http://127.0.0.1:' + port;
    const host = spawn(process.env.NOLA_TEST_CLI || path.join(root, 'target/debug/nola'), [
        'web', '--host', '127.0.0.1', '--port', String(port), '--web-root', path.join(root, 'apps/web')
    ], {
        cwd: temp, stdio: 'ignore',
        env: {PATH: process.env.PATH, TMPDIR: os.tmpdir(), HOME: temp, NOLA_HOME: temp,
            NOLA_DB_PATH: path.join(temp, 'test.db'), NOLA_MODELS_ROOT: models}
    });
    let browser;
    try {
        for (let retries = 0; ; retries++) {
            if (host.exitCode !== null) throw Error('Isolated Host exited');
            if ((await fetch(origin + '/api/web/session').catch(() => null))?.ok) break;
            if (retries === 100) throw Error('Isolated Host did not start');
            await sleep(200);
        }
        browser = await chromium.launch({...browserOptions(), headless: true});
        const page = await browser.newPage({viewport: {width: 1150, height: 920}, locale:label('en','zh-CN')});
        const errors = [];
        page.on('pageerror', error => errors.push(error.message));
        await page.addInitScript(() => localStorage.setItem('nola.localModel.source','modelscope'));
        if (process.env.NOLA_ENGINE_STATUS_ONLY === '1') {
            const statusPath = '/api/local-models/engine-status?engine=';
            let installs = 0;
            page.on('request',request=>{if (request.url().endsWith('/api/local-models/setup-engine')) installs++;});
            const panel = page.getByRole('region',{name:label('Runtime status','运行状态')});
            const results = [];
            for (const id of ['llama.cpp','vllm']) {
                const checking = page.waitForResponse(response=>response.url().endsWith(statusPath + id));
                if (id === 'llama.cpp') {
                    await page.goto(origin + '/models?tab=local');
                    await page.getByRole('searchbox',{name:label('Model name','模型名称')}).fill('engine setup');
                } else {
                    await page.getByLabel(label('Inference engine','推理引擎')).selectOption(id);
                }
                const response = await checking;
                assert.equal(response.status(),200);
                const {engineStatus:state} = await response.json();
                assert.equal(state.engine,id);
                const recheck = page.waitForResponse(response=>response.url().endsWith(statusPath + id));
                await panel.getByRole('button',{name:label('Check again','重新检测')}).click();
                assert.deepEqual((await (await recheck).json()).engineStatus,state);
                const title = id === 'vllm' ? 'vLLM' : id;
                const install = panel.getByRole('button',{name:label('Install ','安装 ') + title,exact:true});
                if (state.canInstall && !state.ready) await install.waitFor();
                else assert.equal(await install.count(),0);
                if (!state.ready && state.guideUrl) {
                    const guide = panel.getByRole('link');
                    assert.equal(await guide.getAttribute('href'),new URL(state.guideUrl).href);
                }
                assert.equal(await page.getByRole('searchbox',{name:label('Model name','模型名称')}).isEditable(),true);
                await panel.scrollIntoViewIfNeeded();
                await page.screenshot({path:screenshot('engine-status-' + id.replace('.','-'))});
                results.push({engine:id,ready:state.ready,canInstall:state.canInstall,reason:state.reason});
            }
            assert.equal(installs,0);
            assert.deepEqual(errors,[]);
            console.log(JSON.stringify({engineStatus:results,setupRequests:installs}));
            return;
        }
        const openedAt = Date.now();
        const recommendation = page.waitForResponse(res => res.url().endsWith('/api/local-models/recommend'), {timeout:300000});
        await page.goto(origin + '/models?tab=local');
        assert.equal(await page.getByLabel(label('Inference engine','推理引擎')).inputValue(), 'llama.cpp');
        assert.equal(await page.getByRole('group',{name:label('Download source','下载来源')}).getByRole('button',{name:'ModelScope'}).getAttribute('aria-pressed'), 'true');
        const hardware = await (await page.request.get(origin + '/api/local-models/hardware')).json();
        assert.ok(hardware.hardware.cpu && hardware.hardware.cores && hardware.hardware.memoryBytes);
        await page.locator('details.hardware > summary').click();
        await page.locator('details.hardware').getByText(hardware.hardware.cpu, {exact:false}).first().waitFor();
        const recommended = await (await recommendation).json();
        assert.ok(recommended.candidates?.length, JSON.stringify(recommended));
        assert.ok(['modelfit','hub','mixed'].includes(recommended.recommendationSource));
        await page.getByText(label('Context target 128K (131,072 tokens)','上下文目标 128K（131,072 tokens）'), {exact:true}).waitFor();
        if (recommended.recommendationSource === 'mixed') {
            await page.getByRole('link',{name:'ModelFit',exact:true}).waitFor();
            await page.getByRole('link',{name:'ModelScope',exact:true}).waitFor();
        }
        const first = recommended.candidates[0];
        await page.locator('.acquisition li').getByText(first.repository, {exact:true}).waitFor();
        console.log(JSON.stringify({stage:'recommendations',elapsedMs:Date.now()-openedAt,source:recommended.recommendationSource,
            candidates:recommended.candidates.map(({repository,variant,contextTokens,contextLimitReason}) =>
                ({repository,variant,contextTokens,contextLimitReason}))}));
        assert.ok(first.downloadBytes < hardware.hardware.freeDiskBytes);
        assert.ok(first.estimatedMemoryBytes <= hardware.hardware.memoryBytes);
        assert.equal((await (await page.request.get(origin + '/api/local-models')).json()).acquisition, null);
        await page.screenshot({path:screenshot('modelscope-hardware-recommendations')});
        let repeats = 0;
        page.on('request', request => {
            if (request.url().endsWith('/api/local-models/recommend') && !request.postDataJSON().query) repeats++;
        });
        await page.getByRole('tab', {name:label('API models','API 模型')}).click();
        await page.getByRole('tab', {name:label('Local models','本地模型')}).click();
        await page.getByText(first.repository, {exact:true}).waitFor();
        await sleep(300);
        assert.equal(repeats, 0);
        await page.getByRole('searchbox', {name:label('Model name','模型名称')}).fill(query);
        const search = page.waitForResponse(res => res.url().endsWith('/api/local-models/recommend'), {timeout:180000});
        await page.getByRole('button', {name:label('Search models','搜索模型')}).click();
        const found = await search;
        const result = await found.json();
        assert.equal(found.status(), 200, JSON.stringify(result));
        const choice = result.candidates.find(candidate => candidate.repository.toLowerCase() === repo.toLowerCase());
        assert.ok(choice?.plan?.revision, JSON.stringify(result));
        assert.equal(choice.targetContextTokens,131072);
        assert.ok(['native_limit','memory_budget','metadata_unknown'].includes(choice.contextLimitReason));
        const confirm = page.getByRole('button', {name:label('Confirm download · ','确认下载 · ') + choice.repository, exact:true});
        await confirm.waitFor();
        const selectedPlan = page.locator('.acquisition li').filter({has:confirm});
        assert.ok(choice.contextTokens < 131072);
        await selectedPlan.locator('summary').click();
        await selectedPlan.getByText(choice.contextTokens.toLocaleString('en'),{exact:true}).waitFor();
        await selectedPlan.getByText(chinese ? /备选方案/ : /Alternative/).waitFor();
        assert.equal((await (await page.request.get(origin + '/api/local-models')).json()).acquisition, null);
        const missingPlan = await page.request.post(origin + '/api/local-models/install-plan', {
            headers:{Origin:origin}, data:{engine:'llama.cpp',source:'modelscope',repository:choice.repository}
        });
        assert.equal(missingPlan.status(), 400);
        await page.screenshot({path:screenshot('modelscope-hardware-plan')});
        await confirm.click();
        const row = page.locator('.model-row').filter({hasText:query});
        for (let retries = 0; ; retries++) {
            if (await row.isVisible()) break;
            const failure = page.locator('.download-banner').getByRole('button', {name: label('Review download plan','重新查看下载方案')});
            if (await failure.isVisible()) throw Error(await page.locator('.download-banner').innerText());
            if (retries === 120) throw Error('Timed out waiting for the model download');
            await sleep(1000);
        }
        const catalog = async () => (await page.request.get(origin + '/api/local-models')).json();
        const before = await catalog();
        const installed = before.models.find(model => model.name === query);
        assert.ok(installed?.installed && installed.runtime === 'llama.cpp');
        assert.ok(before.acquisition.totalBytes === choice.downloadBytes && before.acquisition.status === 'installed');
        const diskCatalog = JSON.parse(await fs.readFile(path.join(models, 'catalog.json'), 'utf8'));
        const record = diskCatalog.models.find(model => model.repository === repo);
        assert.ok(record.snapshotPath.startsWith((await fs.realpath(models)) + path.sep));
        const blob = await fs.open(record.artifactPath, 'r');
        try { const signature = Buffer.alloc(4); await blob.read(signature, 0, 4, 0); assert.equal(signature.toString(), 'GGUF'); }
        finally { await blob.close(); }
        assert.equal(record.revision, choice.plan.revision);
        assert.equal(installed.parameters.contextTokens, choice.contextTokens);
        await row.getByRole('button', {name:label('Configure','配置')}).click();
        const context = page.getByRole('dialog').getByLabel(label('Context window','上下文窗口'));
        const initial = Number(await context.inputValue());
        await context.fill(String(initial - 1));
        await page.getByRole('dialog').getByRole('button', {name:label('Save','保存')}).click();
        await page.getByRole('dialog').waitFor({state:'hidden'});
        await page.reload();
        await row.waitFor();
        assert.equal(await page.getByRole('group',{name:label('Download source','下载来源')}).getByRole('button',{name:'ModelScope'}).getAttribute('aria-pressed'), 'true');
        const restored = (await catalog()).models.find(model => model.id === installed.id);
        assert.equal(restored.parameters.contextTokens, initial - 1);
        await row.getByText(label('Context','上下文') + ' · ' + (initial - 1).toLocaleString('en'),{exact:true}).waitFor();
        if (restored.binaryAvailable) {
            await row.getByRole('button',{name:label('Switch & use','切换并使用'),exact:true}).click();
            const stop = row.getByRole('button',{name:label('Stop','停止'),exact:true});
            await stop.waitFor({timeout:90000});
            const running = (await catalog()).models.find(model => model.id === installed.id);
            assert.ok(running.selected && running.running);
            await stop.click();
            await row.getByText(label('Selected','已选择'),{exact:true}).waitFor();
        } else {
            await row.getByRole('button',{name:label('Set up engine','设置引擎')}).click();
            await page.getByRole('region',{name:label('Runtime status','运行状态')}).waitFor();
        }
        await row.scrollIntoViewIfNeeded();
        await page.screenshot({path:screenshot('modelscope-configured')});
        assert.deepEqual(errors, []);
        console.log(JSON.stringify({repository:repo, planReason:choice.contextLimitReason, planContext:choice.contextTokens, candidates:result.candidates.length, recommendationSource:recommended.recommendationSource, recommendedModels:recommended.candidates.map(value => value.repository),
            downloadedBytes:before.acquisition.totalBytes, contextTokens:restored.parameters.contextTokens,
            selected:restored.selected, runtimeAvailable:restored.binaryAvailable}));
    } finally {
        await browser?.close();
        if (host.exitCode === null) {
            const stopped = new Promise(resolve => host.once('exit', resolve));
            host.kill(); await stopped;
        }
        await fs.rm(temp, {recursive:true, force:true});
    }
})().catch(error => {console.error(error); process.exitCode = 1;});
