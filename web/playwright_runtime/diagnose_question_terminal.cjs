const { chromium } = require('playwright-core'); const fs = require('node:fs');
(async () => {
 const s=JSON.parse(fs.readFileSync('/tmp/nola-question-host-state.json'));
 const previous=JSON.parse(fs.readFileSync('/tmp/nola-real-question/report.json'));
 const b=await chromium.launch({executablePath:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',headless:false});
 const p=await b.newPage(); const result={observations:[],errors:[]};
 p.on('pageerror',e=>result.errors.push(e.message));
 p.on('response',async r=>{const path=new URL(r.url()).pathname;
  if(path.endsWith('/observe') || path.endsWith('/terminal')) { const v=await r.json().catch(()=>null); result.observations.push({path,status:r.status(),operation:r.request().postDataJSON()?.operation,result:v?.result?{status:v.result.status,recovery:v.result.recovery,can_submit:v.result.can_submit,has_seal:!!v.result.terminal_seal}:null,code:v?.code}); }
 });
 try {await p.goto(s.url+'/login', {waitUntil:'domcontentloaded'}); await p.getByLabel('Host 访问令牌',{exact:true}).fill(fs.readFileSync(s.root+'/home/.nola/web/mobile-access-token','utf8').trim()); await p.getByRole('button',{name:'连接',exact:true}).click(); await p.getByRole('button',{name:'新建会话',exact:true}).waitFor(); await p.goto(s.url+'/chat?conversation='+encodeURIComponent(previous.conversationId), {waitUntil:'domcontentloaded'}); await p.waitForTimeout(20000); await p.screenshot({path:'/tmp/nola-real-question/reopened.png'}); result.body=await p.locator('body').ariaSnapshot();}
 finally {fs.writeFileSync('/tmp/nola-real-question/terminal-diagnostic.json',JSON.stringify(result,null,2));await b.close();}
})();
