// SPDX-License-Identifier: Apache-2.0
// Actual browser -> production Host -> selected provider. No route interception.
const {chromium} = require('playwright-core');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const {query} = require('./real_model_evidence.cjs');
const out = process.env.NOLA_REMOTE_LONG_OUT, base = process.env.NOLA_REMOTE_LONG_URL;
const project = fs.realpathSync(path.join(out, 'store/project')), database = path.join(out, 'store/nola.db');
const setup = JSON.parse(fs.readFileSync(path.join(out, 'browser-input.json')));
const receipt = {passed:false, scenarios:{}, pageErrors:[]};
const runs = id => query(database, `SELECT r.id,r.status,json_extract(v.result_json,'$.value.kind') outcome FROM runs r LEFT JOIN run_results v ON v.run_id=r.id WHERE conversation_id=? ORDER BY r.created_at_ms`, [id]);
const reads = id => query(database, `SELECT count(*) n FROM assistant_journal_parts WHERE json_extract(head_json,'$.identity.conversation_id')=? AND json_extract(head_json,'$.metadata.kind')='tool' AND json_extract(head_json,'$.metadata.data.capability_id')='read_file' AND json_extract(head_json,'$.metadata.data.state')='completed'`, [id])[0].n;
let page, permissionMode = 'chain', permissionWaitVerified = false;
async function permission() {
    const panel = page.locator('.permission-panel');
    if (!await panel.count()) return;
    const once = panel.getByRole('button', {name:'仅允许本次', exact:true});
    if (!await once.isEnabled()) return;
    const tool = (await panel.locator('.tool').innerText()).trim();
    const review = await panel.locator('pre').innerText();
    if (permissionMode === 'chain') {
        assert.equal(tool, 'write_file');
        const match = /^write_file: (".*")$/m.exec(review);
        assert.ok(match, 'a complete concrete write target must be visible');
        const requested = path.resolve(project, JSON.parse(match[1]));
        const target = path.join(fs.realpathSync(path.dirname(requested)), path.basename(requested));
        const relative = path.relative(project, target);
        assert.ok(/^checkpoints\/\d{3}\.json$/.test(relative) || relative === 'report.json', 'approval limited to the owned audit outputs');
        assert.match(review, /Mode: create;/);
    } else {
        assert.ok(['bash','bash_command_review'].includes(tool));
        assert.match(review, /^Command: python3 cancel_job\.py$/m);
        const cwd = /^Working directory: (.+)$/m.exec(review)?.[1];
        assert.ok(cwd && (cwd==='Project directory' || fs.realpathSync(cwd)===project));
        if (!permissionWaitVerified) {
            await page.screenshot({path:path.join(out,'permission-wait.png')});
            await new Promise(r=>setTimeout(r,2500));
            assert.ok(!fs.existsSync(path.join(project,'cancel-starts.txt')), 'work must not start before approval');
            permissionWaitVerified = true;
        }
    }
    await once.click();
    receipt.scenarios.explicitApprovals = (receipt.scenarios.explicitApprovals || 0) + 1;
}
async function until(check, timeout=setup.timeoutMs) {
    const end = Date.now()+timeout;
    while(Date.now()<end) {
        const value = await check(); if(value) return value;
        await permission(); await new Promise(r=>setTimeout(r,1000));
    }
    throw Error('Remote long-task observation deadline');
}
(async()=>{
    const browser = await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    const context = await browser.newContext({viewport:{width:1280,height:900},locale:'zh-CN',colorScheme:'dark'});
    function attach(p) { p.setDefaultTimeout(30000); p.on('pageerror',e=>receipt.pageErrors.push(e.message)); }
    page = await context.newPage(); attach(page);
    try {
        await page.goto(base+'/chat');
        if(new URL(page.url()).pathname==='/login') {
            await page.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(path.join(out,'store/home/.nola/web/mobile-access-token'),'utf8').trim());
            await page.getByRole('button',{name:'连接',exact:true}).click();
        }
        async function create() {
            const button=page.locator('.new-conversation');
            if(!await button.isVisible()) await page.locator('.navigation-toggle').click();
            const response=page.waitForResponse(r=>new URL(r.url()).pathname==='/api/chat/conversations'&&r.request().method()==='POST');
            await button.click(); const body=await(await response).json();
            await page.waitForURL(u=>u.searchParams.get('conversation')===body.conversationId);
            await page.locator('#chat-composer:not([disabled])').waitFor(); return body.conversationId;
        }
        async function send(text) { await page.locator('#chat-composer').fill(text); await page.getByRole('button',{name:'发送',exact:true}).click(); }
        const id=await create(); receipt.chainConversation=id;
        await send(setup.prompt);
        await until(()=>reads(id)>=Math.min(5,Math.max(1,setup.nodes-2)));
        const original=runs(id); assert.equal(original.length,1); const runId=original[0].id;
        receipt.chainRun=runId;
        await page.screenshot({path:path.join(out,'chain-active.png')});
        const before=reads(id);
        await page.goto('about:blank');
        const detachedEnd=Date.now()+45000;
        while(reads(id)<=before && runs(id)[0].status!=='sealed' && Date.now()<detachedEnd) await new Promise(r=>setTimeout(r,1000));
        assert.ok(reads(id)>before,'Host must progress while the browser is detached');
        receipt.scenarios.progressWhileDetached=true;
        await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
        await until(()=>reads(id)>=Math.min(setup.nodes,30) || runs(id)[0].status==='sealed');
        await page.close(); page=await context.newPage(); attach(page);
        await page.goto(base+'/chat?conversation='+encodeURIComponent(id));
        let last=-1, lastPrint=Date.now();
        const terminal=await until(()=>{
            const count=reads(id), rows=runs(id);
            assert.equal(rows.length,1,'reopening must not submit a second Run'); assert.equal(rows[0].id,runId);
            if(count!==last && Date.now()-lastPrint>10000) { console.log(JSON.stringify({stage:'chain-progress',completedReads:count})); last=count;lastPrint=Date.now(); }
            return rows[0].status==='sealed'?rows[0]:null;
        });
        assert.ok(['answered','action_completed'].includes(terminal.outcome), JSON.stringify(terminal));
        await page.locator('article:not(.user)').getByText(/REMOTE_CHAIN_DONE/).last().waitFor({timeout:45000});
        assert.ok(fs.existsSync(path.join(project,'report.json')));
        receipt.scenarios.coldTabSameRun=true;
        await page.screenshot({path:path.join(out,'chain-completed.png')});
        // The independent Python reader validates every node, write and seal afterwards.
        permissionMode='cancel'; const cancel=await create(); receipt.cancelConversation=cancel;
        await send('请实际执行 python3 cancel_job.py，使用前台执行，timeout_sec=240，等待真实完成后再读取 cancel-result.json。只运行这一个命令一次，不要改脚本，不要用其他执行路线，也不要派生助手。');
        await until(()=>fs.existsSync(path.join(project,'cancel-progress.txt')),180000);
        assert.equal(runs(cancel).length,1); const cancelId=runs(cancel)[0].id;
        await page.goto('about:blank'); const size=fs.statSync(path.join(project,'cancel-progress.txt')).size;
        await new Promise(r=>setTimeout(r,3000));
        assert.ok(fs.statSync(path.join(project,'cancel-progress.txt')).size>size);
        await page.goto(base+'/chat?conversation='+encodeURIComponent(cancel));
        await page.locator('.composer').getByRole('button',{name:'停止',exact:true}).click();
        const stopped=await until(()=>{const rows=runs(cancel);assert.equal(rows[0].id,cancelId);return rows[0].status==='sealed'?rows[0]:null;},60000);
        assert.equal(stopped.outcome,'cancelled');
        await new Promise(r=>setTimeout(r,1500)); const bytes=fs.statSync(path.join(project,'cancel-progress.txt')).size;
        await new Promise(r=>setTimeout(r,2500)); assert.equal(fs.statSync(path.join(project,'cancel-progress.txt')).size,bytes,'cancel must stop physical writes');
        assert.equal(fs.readFileSync(path.join(project,'cancel-starts.txt'),'utf8'),'start\n');
        assert.ok(!fs.existsSync(path.join(project,'cancel-result.json')));
        receipt.scenarios.cancelStopsPhysicalProcess=true;
        await send('继续这段对话。仅回复 REMOTE_AFTER_CANCEL，不调用工具。');
        const recovered=await until(()=>{const rows=runs(cancel);return rows.length===2&&rows[1].status==='sealed'?rows[1]:null;},120000);
        assert.equal(recovered.outcome,'answered');
        await page.locator('article:not(.user)').getByText('REMOTE_AFTER_CANCEL',{exact:true}).last().waitFor();
        receipt.scenarios.followupAfterCancel=true;
        assert.equal(receipt.pageErrors.length,0);
        receipt.passed=true;
    } catch(error) {
        receipt.error=String(error.stack||error);process.exitCode=1;
        await page.screenshot({path:path.join(out,'failure.png')}).catch(()=>{});
    } finally {
        fs.writeFileSync(path.join(out,'browser-receipt.json'),JSON.stringify(receipt,null,2));
        await browser.close();
    }
})().catch(e=>{console.error(e);process.exitCode=1;});
