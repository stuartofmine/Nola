// SPDX-License-Identifier: Apache-2.0
// Exercise recovery from the public management pages against isolated Host responses.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const {chromium} = require('playwright-core');
const {browserOptions, artifactPath} = require('./test_environment.cjs');

const origin = process.env.UI_FIXTURE_URL;
const evidence = process.env.UI_EVIDENCE_DIR || artifactPath('workspace-recovery');
const screenshot = (page, name) => page.screenshot({path: path.join(evidence, name + '.png')});

async function checkActivity(page) {
    let fail = true;
    const cursors = [];
    await page.route('**/api/activity', route => {
        const {query, source, cursor} = route.request().postDataJSON();
        if (cursor) {
            cursors.push(cursor.run_id);
            if (fail) {
                fail = false;
                return route.fulfill({status: 503, json: {error: 'Unavailable'}});
            }
        }
        const id = cursor ? 'older' : (source || 'all');
        const row = {run_id: id, source: 'conversation', title: 'Run ' + id, created_at_ms: 10, status: 'answered'};
        return route.fulfill({json: {entries: query ? [] : [row],
            next_cursor: !query && !cursor ? {created_at_ms: 10, run_id: id} : null}});
    });
    await page.goto(origin + '/activity');
    const list = page.locator('.activity-list.full');
    await list.getByRole('combobox').selectOption('conversation');
    await list.getByText('Run conversation').waitFor();
    await list.getByRole('button', {name: 'Load more'}).click();
    await list.getByRole('alert').waitFor();
    await list.getByRole('button', {name: 'Retry'}).click();
    await list.getByText('Run older').waitFor();
    assert.deepEqual(cursors, ['conversation', 'conversation']);
    assert.ok(await list.getByText('Run conversation').isVisible());
    await list.getByRole('searchbox').fill('missing');
    await list.getByText('No matching history').waitFor();
    await screenshot(page, 'activity-no-matches');
    await list.getByRole('button', {name: 'Clear filters'}).click();
    await list.getByText('Run all').waitFor();
    assert.equal(await list.getByRole('searchbox').evaluate(node => node === document.activeElement), true);
}

async function checkInformation(page) {
    let sourceReads = 0, usageReads = 0, conversations = [], content = 'Saved finding';
    const row = {conversationId: 'notes', title: 'Field notes', isPinned: false, createdAt: '', updatedAt: '',
        selectedModelSource: '', selectedModelId: '', executionBackend: '', revision: 1, configurationRevision: 1};
    await page.route('**/api/data-sources', r => { sourceReads++; return r.fulfill({json:{sources:[]}}); });
    await page.route('**/api/data-sources/usage', r => { usageReads++; return r.fulfill({json:{usageRecords:[]}}); });
    await page.route('**/api/chat/conversations', r => r.fulfill({json:{conversations}}));
    await page.route('**/api/chat/conversations/notes/configuration', r => r.fulfill({json:{
        conversationId:'notes', configurationRevision:1, knowledge:{keyPointsMarkdown:content, wikiMarkdown:''}}}));
    await page.goto(origin + '/data-sources?tab=knowledge');
    const panel = page.locator('.knowledge-overview');
    const refresh = panel.getByRole('button', {name: 'Refresh'});
    await panel.getByText('No saved knowledge points yet.').waitFor();
    assert.equal(await panel.getByRole('combobox').count(), 0);
    assert.deepEqual([sourceReads, usageReads], [0, 0]);
    conversations = [row];
    await refresh.click();
    await panel.getByRole('combobox').selectOption('notes');
    await panel.getByText(content).waitFor();
    content = 'Updated finding';
    await refresh.click();
    await panel.getByText(content).waitFor();
    await screenshot(page, 'knowledge-refreshed');
    conversations = [];
    await refresh.click();
    await panel.getByText('No saved knowledge points yet.').waitFor();
    assert.equal(await panel.getByRole('link').count(), 0);
    for (const item of ['sources','usage','sources','usage']) {
        await page.locator('[role=tab][id$=data-tab-' + item + ']').click();
        await page.locator('[id$=data-panel-' + item + '][aria-busy=false]:visible').waitFor();
    }
    assert.deepEqual([sourceReads, usageReads], [1, 1]);
}

async function checkModels(page) {
    let reads = 0, selections = 0, starts = 0;
    const models = [
        {id:'ready-engine',name:'Available model',runtime:'llama.cpp',installed:true,selected:false,running:false},
        {id:'missing-engine',name:'Offline model',runtime:'vllm',installed:true},
        {id:'transfer',name:'Pending size',runtime:'llama.cpp',installed:false,
            download:{status:'downloading',downloadedBytes:2048,totalBytes:0}}
    ];
    await page.route('**/api/local-models', route => {
        reads++;
        if (reads === 2) return route.fulfill({status:503,json:{error:'Temporary read failure'}});
        return route.fulfill({json:{models,binaryAvailable:true,runtimes:[
            {id:'llama.cpp',binaryAvailable:true},{id:'vllm',binaryAvailable:false}]}});
    });
    const ready = models[0];
    await page.route('**/api/local-models/ready-engine/select', route => {
        selections++; ready.selected = true;
        return route.fulfill({json:{}});
    });
    await page.route('**/api/local-models/ready-engine/start', route => {
        if (++starts === 1) return route.fulfill({status:503,json:{error:'Port already in use'}});
        ready.running = true;
        return route.fulfill({json:{}});
    });
    await page.route('**/api/local-models/engine-status?*', route => {
        const engine = new URL(route.request().url()).searchParams.get('engine');
        return route.fulfill({json:{engineStatus:{engine,ready:engine === 'llama.cpp',canInstall:false,reason:'ready'}}});
    });
    // Keep automatic hardware recommendations within the fixture.
    await page.route('**/api/local-models/recommend', r => r.fulfill({json:{candidates:[],hardware:{}}}));
    await page.route('**/api/local-models/hardware', r => r.fulfill({json:{hardware:{}}}));
    await page.goto(origin + '/models?tab=local');
    const transfer = page.locator('.model-row').filter({hasText:'Pending size'});
    await transfer.getByText('Downloading',{exact:true}).waitFor();
    assert.equal(await transfer.getByRole('progressbar').evaluate(node => node.position), -1);
    const installed = page.locator('.model-row').filter({hasText:'Offline model'});
    const nextRead = page.waitForResponse(response => new URL(response.url()).pathname === '/api/local-models' && reads >= 3);
    await installed.getByRole('button',{name:'Set up engine'}).click();
    assert.equal(await page.getByLabel('Inference engine').inputValue(), 'vllm');
    assert.ok(await page.getByRole('region',{name:'Runtime status'}).evaluate(node => node === document.activeElement));
    await nextRead;
    await page.locator('#panel-local > [role=alert]').waitFor({state:'hidden'});
    await installed.scrollIntoViewIfNeeded();
    await screenshot(page, 'models-engine-and-transfer');
    const available = page.locator('.model-row').filter({hasText:'Available model'});
    await available.getByRole('button',{name:'Switch & use'}).click();
    await page.getByRole('alert').filter({hasText:'Port already in use'}).waitFor();
    await available.getByText('Selected',{exact:true}).waitFor();
    await available.getByRole('button',{name:'Switch & use'}).click();
    await available.getByRole('button',{name:'Stop'}).waitFor();
    assert.equal(starts, 2);
    assert.equal(selections, 1);
}

(async () => {
    if (!origin) throw new Error('UI_FIXTURE_URL must point to an isolated Web Host');
    await fs.mkdir(evidence, {recursive:true});
    const browser = await chromium.launch({...browserOptions(),headless:true});
    try {
        for (const [name, check] of Object.entries({activity:checkActivity, knowledge:checkInformation, models:checkModels})) {
            const page = await browser.newPage({viewport:{width:1040,height:800}, locale:'en-US'});
            page.setDefaultTimeout(10000);
            try { await check(page); console.log('PASS ' + name); }
            catch (error) { await screenshot(page, 'failure-' + name); throw error; }
            finally { await page.close(); }
        }
    } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
