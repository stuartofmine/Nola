const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
(async()=>{
    const browser=await chromium.launch({executablePath:process.env.CHROME_PATH||'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
    try {
        for(const width of [320,600]) {
            const page=await browser.newPage({viewport:{width,height:900}});const errors=[];
            page.setDefaultTimeout(10000);
            page.on('pageerror',e=>errors.push(e.message));
            await page.route('**/api/model-profiles',async r=>{const response=await r.fetch();const data=await response.json();await r.fulfill({json:{profiles:data.profiles.map(p=>({...p,provider:'openai-compatible'}))}});});
            await page.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5129'}/models?tab=api`);
            await page.locator('.profile-settings > summary').click();
            await page.locator('.endpoint-summary').first().click();
            const menu=page.locator('.endpoint-head .compact-actions').first(), summary=menu.locator('summary');
            await summary.focus();await page.keyboard.press('Enter');
            const first=menu.locator('button').first();await first.focus();
            await page.keyboard.press('Escape');
            assert.equal(await menu.evaluate(e=>e.open),false);
            assert.equal(await summary.evaluate(e=>e===document.activeElement),true);
            await summary.click();
            await menu.locator('button:not(:disabled)').last().focus();
            await page.keyboard.press('Tab');
            assert.equal(await menu.evaluate(e=>e.open),false,'focus departure dismisses');
            await summary.click();
            const popup=await menu.locator('.compact-menu').boundingBox();
            assert.ok(popup.x>=0&&popup.x+popup.width<=width+1,'menu within compact viewport');
            await page.locator('.page-header h1').click();
            assert.equal(await menu.evaluate(e=>e.open),false,'outside pointer dismisses');
            await summary.click();await first.click();
            await page.locator('dialog:modal').waitFor();
            assert.equal(await menu.evaluate(e=>e.open),true,'modal retains opener');
            await page.keyboard.press('Escape');
            assert.equal(await first.evaluate(e=>e===document.activeElement),true);
            await page.keyboard.press('Escape');
            assert.equal(await summary.evaluate(e=>e===document.activeElement),true);
            await summary.click();
            const modelMenu=page.locator('.model .compact-actions').first();
            await modelMenu.locator('summary').focus();
            await page.keyboard.press('Enter');
            assert.equal(await menu.evaluate(e=>e.open),false,'opening another menu dismisses previous');
            await page.keyboard.press('Escape');
            await page.emulateMedia({reducedMotion:'reduce'});
            await summary.click();
            const duration=await menu.locator('.compact-menu').evaluate(e=>parseFloat(getComputedStyle(e).animationDuration));
            assert.ok(duration<.01,'reduced motion respected');
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
            assert.deepEqual(errors,[]);await page.close();
        }
        console.log('PASS: 320/600 menus Escape, Tab/focus departure, outside pointer, bounds, modal opener restoration, single menu and reduced motion');
    }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
