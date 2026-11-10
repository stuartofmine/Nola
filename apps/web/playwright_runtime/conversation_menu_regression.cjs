const {chromium}=require('playwright-core');const assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});try{
 const page=await browser.newPage({viewport:{width:1440,height:1000}});
 await page.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139'}/chat?conversation=review`);
 const manager=page.locator('.conversation-manager');await manager.locator('.conversation-row').waitFor();
 assert.equal(await manager.locator('summary').count(),1,'one actions disclosure per conversation');
 const summary=manager.locator('summary');assert.equal((await summary.innerText()).trim(),'⋯');
 await page.waitForTimeout(250);await summary.focus();await page.keyboard.press('Enter');
 const history=manager.locator('.history-actions');await history.waitFor();
 assert.equal(await history.locator('button').first().isEnabled(),true,'whole conversation branch retained');
 assert.equal(await history.locator('button').last().isDisabled(),true,'truncate requires selected message');
 assert.equal(await manager.locator('.row-action-buttons > button').count(),5,'pin rename exports delete retained');
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
 const deletion=manager.locator('.row-action-buttons > .danger');
 for(const width of [1440,320]) {
  if(width===320){await page.setViewportSize({width,height:900});await page.locator('.navigation-toggle').click();}
  await page.waitForTimeout(250);await summary.focus();if(!await summary.evaluate(e=>e.parentElement.open))await summary.press('Enter');
  for(let i=0;i<8;i++){await page.keyboard.press('Tab');if(await deletion.evaluate(e=>e===document.activeElement))break;}
  assert.equal(await deletion.evaluate(e=>e===document.activeElement),true,'delete reachable by Tab without activation');
  assert.equal(await deletion.evaluate(e=>{let r=e.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight}),true,'focused action scrolled into viewport');
  assert.equal(await manager.locator('.row-action-buttons > button').first().evaluate(e=>getComputedStyle(e).borderTopWidth),'0px');
  await page.waitForTimeout(250);await page.screenshot({path:require('./test_environment.cjs').artifactPath(`nola-web-full-ui/conversation-menu-${width}.png`)});
 }

 await page.keyboard.press('Escape');assert.equal(await history.isVisible(),false);assert.equal(await summary.evaluate(e=>e===document.activeElement),true);
 await summary.press('Enter');const search=manager.locator('input[type=search]');await search.click();assert.equal(await history.isVisible(),false);assert.equal(await search.evaluate(e=>e===document.activeElement),true);
 console.log('PASS one desktop conversation disclosure; branch/truncate guards and all row actions retained; keyboard toggle');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1});
