// SPDX-License-Identifier: Apache-2.0
// This fixture writes only to an explicitly isolated Host. Its worker stays disabled.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('playwright-core');
const { artifactPath, browserOptions, isolatedHostOrigin } = require('./test_environment.cjs');

(async () => {
    const origin = isolatedHostOrigin(), out = process.env.UI_EVIDENCE_DIR || artifactPath('messaging-channel-reload');
    await fs.mkdir(out, {recursive:true});
    let channelReads = 0;
    const backend = http.createServer((req, res) => {
        if (req.method !== 'GET' || req.url !== '/v1/channels' || req.headers.authorization !== 'Bearer local-audit-fixture') {
            res.writeHead(404).end();
            return;
        }
        channelReads++;
        res.writeHead(200, {'Content-Type':'application/json'}).end(JSON.stringify({channels:[]}));
    });
    await new Promise(resolve => backend.listen(0, '127.0.0.1', resolve));
    const browser = await chromium.launch({...browserOptions(), headless:true});
    try {
        const page = await browser.newPage({locale:'zh-CN', viewport:{width:1280,height:900}});
        page.setDefaultTimeout(8000);
        const routes = [['dashboard',1280], ...['general','agent','providers','connections','data','diagnostics'].map(id=>['settings?section='+id,1280]), ['settings',390]];
        const enter = async (entry,width) => {
            await page.setViewportSize({width,height:900});
            await page.goto(new URL('/'+entry,origin).href);
            if (entry === 'dashboard') await page.getByText('查看配置',{exact:true}).filter({visible:true}).first().click();
            if (entry !== 'settings?section=connections') {
                const panel=page.locator('[id$="settings-panel-connections"]:visible');
                if (!await panel.count()) await page.getByText('连接',{exact:true}).filter({visible:true}).first().click();
            }
            const messaging = page.locator('[id$="messaging-settings"]:visible');
            await messaging.getByLabel('Nola 服务地址',{exact:true}).waitFor();
            return messaging;
        };
        const setup = await enter('settings?section=connections',1280);
        const reload = setup.getByRole('button',{name:'重新载入',exact:true}).last();
        const address = setup.getByLabel('Nola 服务地址',{exact:true}), token = setup.locator('form input[type=password]');
        assert.equal(await address.inputValue(),'','a fresh isolated messaging configuration is required');
        await setup.getByText('先保存服务地址和账户访问令牌，再管理消息渠道。',{exact:true}).waitFor();
        assert.ok(await reload.isDisabled());
        await page.screenshot({path:path.join(out,'unconfigured.png')});
        await address.fill('http://127.0.0.1:'+backend.address().port);
        await token.fill('local-audit-fixture');
        const listReceipt = () => page.waitForResponse(r => new URL(r.url()).pathname==='/api/messaging-channels'&&r.request().method()==='GET',{timeout:8000});
        const [initial]=await Promise.all([listReceipt(),setup.locator('form').getByRole('button',{name:'保存',exact:true}).click()]);
        assert.ok(initial.ok());
        const results=[];
        for (const [entry,width] of routes) {
            const panel=await enter(entry,width);
            const button=panel.getByRole('button',{name:'重新载入',exact:true}).last();
            await page.waitForFunction(()=>Array.from(document.querySelectorAll('[id$="messaging-settings"] button')).filter(b=>b.textContent.trim()==='重新载入').at(-1)?.disabled===false);
            const before=channelReads;
            const [receipt]=await Promise.all([listReceipt(),button.click()]);
            assert.ok(receipt.ok());assert.deepEqual((await receipt.json()).channels,[]);assert.ok(channelReads>before);
            const shot='reload-'+width+'-'+entry.replace('?section=','-')+'.png';
            await page.screenshot({path:path.join(out,shot)});
            results.push({route:entry,width:String(width),role:'button',label:'重新载入',ordinal:2,passed:true,shot,status:receipt.status()});
            await fs.writeFile(path.join(out,'results.json'),JSON.stringify(results,null,2));
        }
        console.log('PASS: unavailable channel reload is disabled; all eight entry points contacted the isolated channel service through the Rust Host.');
    } finally { await browser.close(); await new Promise(resolve=>backend.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1;});
