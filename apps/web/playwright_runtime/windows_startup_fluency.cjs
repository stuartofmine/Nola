// The shipped Windows startup renderer, without starting Electron or its Host.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs/promises');
const path=require('node:path');
const {startupPageHtml}=require('../../windows/dist/startupPage.js');
const {stringsFor}=require('../../windows/dist/i18n.js');
(async()=>{
 const output=process.env.UI_EVIDENCE_DIR||require('./test_environment.cjs').artifactPath('nola-windows-startup');await fs.mkdir(output,{recursive:true});
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 try{
  for(const colorScheme of ['light','dark'])for(const width of [320,1440])for(const mode of ['starting','failed']){
   const context=await browser.newContext({colorScheme,viewport:{width,height:900},reducedMotion:'no-preference'});
   await context.route('**/*',route=>route.abort());
   const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
   const detail='<script>window.untrustedMarkup=true</script> '+ 'Long fixture path 中文 '.repeat(30);
   await page.setContent(startupPageHtml(stringsFor('zh-CN'),mode,detail));
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
   assert.equal(await page.evaluate(()=>getComputedStyle(document.body).backgroundColor),colorScheme==='dark'?'rgb(0, 0, 0)':'rgb(255, 255, 255)');
   if(mode==='failed'){
    assert.equal(await page.locator('script').count(),0);assert.equal(await page.locator('.detail').textContent(),detail);
    assert.equal(await page.locator('.actions').evaluate(e=>e.getBoundingClientRect().bottom<=innerHeight),true,'Recovery actions remain visible with long details');
    await page.locator('.detail').focus();await page.keyboard.press('PageDown');
    await page.waitForFunction(()=>document.querySelector('.detail').scrollTop>0);
    assert.equal(await page.locator('.detail').evaluate(e=>e.scrollTop>0),true);
    await page.locator('#nola-retry-host').focus();assert.equal(await page.locator('#nola-retry-host').evaluate(e=>e===document.activeElement),true);
   }else{
    await page.emulateMedia({reducedMotion:'reduce'});
    assert.equal(await page.locator('.pulse').evaluate(e=>getComputedStyle(e,'::after').animationName),'none');
   }
   assert.deepEqual(errors,[]);await page.screenshot({path:path.join(output,`${mode}-${width}-${colorScheme}.png`)});await context.close();
  }
  console.log('PASS: Windows startup/failure HTML, themes, two widths, error escaping, keyboard focus and reduced motion; native Windows frame remains untested');
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
