const { chromium } = require('playwright-core');
const fs = require('node:fs');
const { execFileSync } = require('node:child_process');
(async () => {
    const state = JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE));
    const out = process.env.NOLA_GOAL_DIAGNOSTIC_OUT || '/tmp/nola-goal-navigation-network'; fs.mkdirSync(out, {recursive:true});
    const browser = await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:false});
    const page = await browser.newPage(); const cdp = await page.context().newCDPSession(page); await cdp.send('Network.enable');
    const requests = new Map();
    cdp.on('Network.requestWillBeSent', e => requests.set(e.requestId,{path:new URL(e.request.url).pathname,type:e.type,started:e.timestamp}));
    cdp.on('Network.responseReceived', e => { const r=requests.get(e.requestId); if(r) Object.assign(r,{status:e.response.status,protocol:e.response.protocol,connection:e.response.connectionId,fromCache:e.response.fromDiskCache,headersAt:e.timestamp}); });
    cdp.on('Network.loadingFinished',e=>{const r=requests.get(e.requestId);if(r)Object.assign(r,{finished:e.timestamp,bytes:e.encodedDataLength});});
    cdp.on('Network.loadingFailed',e=>{const r=requests.get(e.requestId);if(r)Object.assign(r,{failed:e.errorText,cancelled:e.canceled});});
    let failure;
    try {
        await page.goto(state.url+'/login');
        await page.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(state.root+'/home/.nola/web/mobile-access-token','utf8').trim());
        await page.getByRole('button',{name:'连接',exact:true}).click();
        await page.getByRole('button',{name:'新建会话',exact:true}).waitFor();
        await page.goto(state.url+'/chat?conversation='+encodeURIComponent(process.env.NOLA_GOAL_EXISTING_CONVERSATION),{waitUntil:'domcontentloaded'});
        await page.getByRole('button',{name:'进度',exact:true}).waitFor({timeout:20000});
        // The toolbar renders before the conversation has loaded. Its presence
        // alone is not readiness and previously hid a blocked Store read.
        await page.waitForFunction(() => {
            const row = document.querySelector('button.conversation');
            return row && !row.disabled && !document.body.innerText.includes('正在加载会话…');
        }, null, {timeout:20000});
    } catch(e) { failure=e.message; process.exitCode = 1; }
    finally {
        if (failure && Number.isSafeInteger(state.pid)) {
            try { execFileSync('/usr/bin/sample', [String(state.pid), '2', '-file', out+'/host.sample'], {stdio:'ignore',timeout:10000}); } catch {}
        }
        fs.writeFileSync(out+'/network.json',JSON.stringify({failure,requests:[...requests.values()]},null,2));
        fs.writeFileSync(out+'/page.txt',await page.locator('body').ariaSnapshot());
        await page.screenshot({path:out+'/page.png'}); await browser.close();
        console.log(JSON.stringify({failure,out}));
    }
})();
