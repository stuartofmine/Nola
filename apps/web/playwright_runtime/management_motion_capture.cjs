// Isolated actual-route motion evidence. CDP screencast timestamps preserve frame timing.
const {chromium}=require('playwright-core');
const fs=require('node:fs'); const {execFileSync}=require('node:child_process');
const base=process.env.UI_FIXTURE_URL||'http://127.0.0.1:5139';
const out=process.env.UI_EVIDENCE_DIR||require('./test_environment.cjs').artifactPath('nola-web-full-ui/motion');
(async()=>{
 fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
 try {
 const page=await browser.newPage({viewport:{width:390,height:900},colorScheme:'dark',reducedMotion:'no-preference'});
 await page.route('**/api/web/session',r=>r.fulfill({json:{authenticated:true,sessionAuthenticated:false,localTrust:true,hostId:'ui-review-fixture'}}));
 const client=await page.context().newCDPSession(page);
 for(const kind of ['settings-tabs','inspector']) {
  await page.goto(`${base}/${kind==='inspector'?'chat?conversation=review':'settings'}`);
  await page.locator(kind==='inspector'?'#chat-composer':'#settings-tab-general').waitFor();
  const dir=`${out}/${kind}`;fs.mkdirSync(dir,{recursive:true}); const frames=[];const events=[];
  const handler=async e=>{const file=`frame-${String(frames.length).padStart(5,'0')}.jpg`;fs.writeFileSync(`${dir}/${file}`,Buffer.from(e.data,'base64'));frames.push({file,timestamp:e.metadata.timestamp});await client.send('Page.screencastFrameAck',{sessionId:e.sessionId});};
  client.on('Page.screencastFrame',handler);
  await client.send('Page.startScreencast',{format:'jpeg',quality:90,everyNthFrame:1});
  const mark=name=>events.push({name,wallTime:Date.now()});
  mark('stable-start');await page.waitForTimeout(1000);
  mark('open-or-select');
  if(kind==='inspector')await page.locator('.chat-header [aria-controls="chat-details"]').last().click();
  else {await page.locator('#settings-tab-general').focus();await page.keyboard.press('ArrowRight');}
  await page.waitForTimeout(1200); mark('stable-open');
  mark('close-or-return');
  if(kind==='inspector')await page.keyboard.press('Escape');
  else await page.keyboard.press('Home');
  await page.waitForTimeout(1200);mark('stable-end');
  await client.send('Page.stopScreencast');client.off('Page.screencastFrame',handler);
  fs.writeFileSync(`${dir}/timing.json`,JSON.stringify({events,frames},null,2));
  const lines=[];frames.forEach((f,i)=>{lines.push(`file '${f.file}'`);lines.push(`duration ${Math.max(.001,(frames[i+1]?.timestamp??(events.at(-1).wallTime/1000))-f.timestamp)}`);});lines.push(`file '${frames.at(-1).file}'`);
  fs.writeFileSync(`${dir}/frames.txt`,lines.join('\n'));
  execFileSync(process.env.FFMPEG_PATH || 'ffmpeg',['-y','-f','concat','-safe','0','-i',`${dir}/frames.txt`,'-fps_mode','vfr','-pix_fmt','yuv420p',`${out}/${kind}.mp4`],{stdio:'ignore'});
  console.log(`${kind}: ${frames.length} actual captured frames; ${out}/${kind}.mp4`);
 }
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
