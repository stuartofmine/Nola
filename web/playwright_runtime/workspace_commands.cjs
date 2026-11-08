// Actual isolated Rust Host + production UI. No mocked API or model output.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const fs=require('node:fs');
(async()=>{
 const state=JSON.parse(fs.readFileSync('/tmp/nola-a2ui-real-host-state.json','utf8'));
 assert.ok(state.root.includes('nola-a2ui-real-'));assert.equal(new URL(state.url).hostname,'127.0.0.1');
 const browser=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:true});
 const width=Number(process.env.NOLA_TEST_WIDTH||390);
 const page=await browser.newPage({viewport:{width,height:844}});const errors=[],admissions=[];
 page.on('pageerror',e=>errors.push(e.message));
 page.on('request',r=>{if(r.method()==='POST'&&/\/(submit|queued-input|stream|admit-user-message)$/.test(new URL(r.url()).pathname))admissions.push(new URL(r.url()).pathname)});
 try {
 await page.goto(state.url+'/chat',{waitUntil:'domcontentloaded'});
 if(width<=900) await page.getByRole('button',{name:'工作区',exact:true}).first().click({timeout:30000});
 const create=page.getByRole('button',{name:'新建会话',exact:true});await create.waitFor({timeout:30000});
 await create.click();const input=page.locator('#chat-composer');await input.waitFor();
 await input.click();await input.fill('/mo');await page.getByRole('option').filter({hasText:'/model'}).click();assert.equal(await input.inputValue(),'/model ');
 await input.fill('/unknown ');await input.press('Enter');await page.getByRole('alert').filter({hasText:'指令或参数无效'}).waitFor();assert.equal(await input.inputValue(),'/unknown ');
 await input.fill('/search 出师表');await input.press('Enter');assert.equal(await page.locator('#chat-search-input').inputValue(),'出师表');
 await page.locator('.chat-search button').click();
 await input.fill('/stop ');await input.press('Enter');await page.getByRole('alert').filter({hasText:'没有正在执行'}).waitFor();assert.equal(await input.inputValue(),'/stop ');
 await input.fill('/help ');await input.press('Enter');assert.equal(await input.inputValue(),'/');assert.equal(await page.locator('#composer-commands [role=option]').count(),8);
 await input.fill('/plan ');await input.press('Enter');await page.getByRole('tab',{name:'计划',exact:true}).waitFor();assert.equal(await page.getByRole('tab',{name:'计划',exact:true}).getAttribute('aria-selected'),'true');
 for(const label of ['资料','记忆','记录','计划']){await page.getByRole('tab',{name:label,exact:true}).click();assert.equal(await page.getByRole('tab',{name:label,exact:true}).getAttribute('aria-selected'),'true')}
 assert.equal(admissions.length,0,'Local commands must not admit messages');assert.deepEqual(errors,[]);
 fs.mkdirSync('.artifacts/workspace-commands-20261024',{recursive:true});await page.screenshot({path:`.artifacts/workspace-commands-20261024/web-${width}.png`,fullPage:true});
 console.log('PASS: real Host workspace + command completion, invalid input, search, stop, help, plans; zero message admissions');
 } catch(error) { console.log((await page.locator('body').innerText()).slice(0,2000));console.log(await page.locator('#chat-composer').evaluate(el=>({value:el.value,focused:el===document.activeElement,expanded:el.getAttribute('aria-expanded')})).catch(()=>null)); throw error; } finally {await browser.close()}
})().catch(e=>{console.error(e);process.exitCode=1});
