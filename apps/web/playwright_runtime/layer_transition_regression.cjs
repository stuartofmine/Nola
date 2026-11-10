const{chromium}=require('playwright-core');const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});try{for(const width of [320,390,768,1440]){
const p=await b.newPage({viewport:{width,height:400}});await p.goto(`${process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139'}/chat?conversation=review`);await p.locator('#chat-composer').waitFor();await p.waitForTimeout(250);
await p.locator('.model-menu summary').click();await p.locator('.model-menu .management button').click();await p.waitForTimeout(250);
assert.equal(await p.locator('.model-menu').evaluate(e=>e.open),false);assert.equal(await p.locator('.session-settings').isVisible(),true);
if(width<1251){await p.keyboard.press('Escape');assert.equal(await p.locator('.model-menu summary').evaluate(e=>e===document.activeElement),true);}
console.log('PASS picker → settings and focus return',width);await p.close();}}finally{await b.close();}})().catch(e=>{console.error(e);process.exitCode=1});
