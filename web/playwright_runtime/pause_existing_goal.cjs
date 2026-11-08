// Stop only the explicitly selected audit Goal through the real browser UI.
const {chromium}=require('playwright-core');
const fs=require('node:fs');
(async()=>{
 const state=JSON.parse(fs.readFileSync(process.env.NOLA_BUTTON_AUDIT_HOST_STATE));
 const id=process.env.NOLA_GOAL_EXISTING_CONVERSATION;
 if(!id || !state.root.includes('nola-a2ui-real-')) throw Error('An isolated existing audit conversation is required');
 const out=process.env.NOLA_GOAL_AUDIT_OUT; fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:false});
 const page=await browser.newPage();page.setDefaultTimeout(20000);const report={};
 try {
  await page.goto(state.url+'/login');
  await page.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(state.root+'/home/.nola/web/mobile-access-token','utf8').trim());
  await page.getByRole('button',{name:'连接',exact:true}).click();
  await page.goto(state.url+'/chat?conversation='+encodeURIComponent(id));
  await page.getByRole('button',{name:'进度',exact:true}).click();
  const panel=page.getByRole('region',{name:'活动与目标',exact:true});
  await panel.getByRole('button',{name:'暂停',exact:true}).click();
  // Do not resubmit on a missing acknowledgement. Observe the authoritative
  // Goal; retaining the report allows recovery without another control click.
  const deadline=Date.now()+60000;
  while(Date.now()<deadline){
   const response=await page.request.get(state.url+'/api/chat/conversations/'+encodeURIComponent(id)+'/goal');
   const value=await response.json();
   report.status=value.goalProjection?.active?.status;
   if(response.ok() && value.authoritative===true && report.status==='paused'){report.passed=true;break;}
   await page.waitForTimeout(500);
  }
  if(!report.passed) throw Error('Pause was not authoritatively observed');
 } catch(e){report.failure=e.message;process.exitCode=1;}
 finally{fs.writeFileSync(out+'/report.json',JSON.stringify(report,null,2));await page.screenshot({path:out+'/page.png'});await browser.close();console.log(JSON.stringify(report));}
})().catch(e=>{console.error(e);process.exitCode=1});
