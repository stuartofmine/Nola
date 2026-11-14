// SPDX-License-Identifier: Apache-2.0
'use strict';
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');

async function main() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'nola-browser-use-e2e-'));
  const server = http.createServer((request, response) => {
    response.setHeader('Content-Type', 'text/html; charset=utf-8');
    response.end(`<!doctype html><label>Name <input aria-label="Name" oninput="document.querySelector('#draft').textContent=this.value"></label>
      <button onclick="document.querySelector('#result').textContent='Saved '+document.querySelector('input').value">Save</button>
      <input type="number" aria-label="Quantity" oninput="document.querySelector('#qty').textContent=this.value">
      <div contenteditable="true" role="textbox" aria-label="Notes"></div>
      <p id="qty"></p><p id="draft"></p><p id="result"></p>
      <dialog id="confirmation"><label>Confirmation reason<input></label><button onclick="document.querySelector('#confirmation').close()">Confirm modal</button></dialog>
      <button onclick="document.querySelector('#editor').showModal()">Open labelled editor</button>
      <dialog id="editor"><details><summary>Expandable details</summary><p>Expanded fixture content</p></details><label>Implicit field<input></label><form onsubmit="event.preventDefault();document.querySelector('#select-submit').textContent='Native form submitted'"><label>Native option<select onchange="document.querySelector('#select-result').textContent='Selected '+this.value"><option>First option</option><option value="second-id">Second option</option><option disabled value="disabled-id">Disabled option</option><option value="dup-a">Duplicate</option><option value="dup-b">Duplicate</option></select></label><button type="submit">Submit native form</button></form><p id="select-result"></p><p id="select-submit">Native form not submitted</p>
      <button onclick="document.querySelector('#confirmation').showModal()">Open earlier confirmation</button><button onclick="document.querySelector('#editor').close()">Close labelled editor</button></dialog>
      <div style="position:fixed;left:600px;top:100px;width:200px;height:100px;overflow:auto" onscroll="document.querySelector('#scroll-result').textContent='Nested scrolled'"><div style="height:800px">Nested scroll area</div></div><p id="scroll-result"></p>`);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const bridge = spawn(process.execPath, [path.join(__dirname, 'bridge.cjs')], {
    env: {...process.env, NOLA_PLAYWRIGHT_STATE_ROOT:root}, stdio:['pipe','pipe','inherit'],
  });
  const pending = new Map(); let sequence = 0;
  const lines = readline.createInterface({input:bridge.stdout});
  lines.on('line', line => {
    const reply = JSON.parse(line), item = pending.get(reply.id);
    pending.delete(reply.id); clearTimeout(item.timer);
    if (reply.error) item.reject(new Error(reply.error)); else item.resolve(reply.result);
  });
  const call = (action, args) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error('Test browser response timed out')); }, 60000);
    pending.set(id, {resolve,reject,timer}); bridge.stdin.write(JSON.stringify({id,action,args})+'\n');
  });
  try {
    const url = `http://127.0.0.1:${server.address().port}/`;
    const [a,b] = await Promise.all(['owner-a','owner-b'].map(session_id => call('navigate', {session_id,url})));
    assert.equal(a.pages.length,1); assert.equal(b.pages.length,1);
    assert.notEqual(a.generation,b.generation);
    const aInput = a.elements.find(item => item.name === 'Name');
    const bInput = b.elements.find(item => item.name === 'Name');
    await assert.rejects(call('type_text', {session_id:'owner-b',ref:aInput.ref,revision:a.revision,text:'wrong owner'}), /unknown|stale/);
    let [aa,bb] = await Promise.all([
      call('type_text',{session_id:'owner-a',ref:aInput.ref,revision:a.revision,text:'A 你好'}),
      call('type_text',{session_id:'owner-b',ref:bInput.ref,revision:b.revision,text:'B draft'}),
    ]);
    assert.match(aa.text,/A 你好/); assert.match(bb.text,/B draft/); assert.doesNotMatch(bb.text,/A 你好/);
    // Inspector input is admitted to this exact owner/generation, through Browser Use too.
    const inputURL = new URL(aa.preview_url); inputURL.pathname='/input';
    const stale = new URL(inputURL); stale.searchParams.set('generation','old-generation');
    assert.equal((await fetch(stale,{method:'POST',body:JSON.stringify({kind:'text',text:'WRONG'})})).status,409);
    assert.equal((await fetch(inputURL,{method:'POST',body:JSON.stringify({kind:'text',text:'!'})})).status,204);
    assert.equal((await fetch(inputURL,{method:'POST',body:JSON.stringify({kind:'click',x:650,y:150})})).status,204);
    assert.equal((await fetch(inputURL,{method:'POST',body:JSON.stringify({kind:'wheel',delta_y:200})})).status,204);
    await new Promise(resolve=>setTimeout(resolve,100));
    aa = await call('snapshot',{session_id:'owner-a'});
    assert.match(aa.text,/Nested scrolled/);
    assert.match(aa.text,/A 你好!/); assert.doesNotMatch(aa.text,/WRONG/);
    for(let step=0;step<20;step++) {
      const field = aa.elements.find(item => item.name === 'Name');
      aa = await call('type_text',{session_id:'owner-a',ref:field.ref,revision:aa.revision,text:`A step ${step}`});
      const button = aa.elements.find(item => item.name === 'Save');
      aa = await call('click',{session_id:'owner-a',ref:button.ref,revision:aa.revision,screenshot:false});
      assert.match(aa.text,new RegExp(`Saved A step ${step}`));
      assert.equal(aa.pages.length,1,'Long tasks must not leak daemon startup tabs');
    }
    aa = await call('snapshot',{session_id:'owner-a'});
    let target = aa.elements.find(item => item.name === 'Open labelled editor');
    aa = await call('click',{session_id:'owner-a',ref:target.ref,revision:aa.revision,screenshot:false});
    assert.ok(aa.elements.some(item => item.name === 'Implicit field'), 'Wrapping labels must name form controls');
    assert.ok(aa.elements.some(item => item.name === 'Native option'), 'Select options must not pollute its accessible label');
    assert.ok(!aa.elements.some(item => item.name === 'Name'), 'Blocked background controls must not consume modal references');
    target = aa.elements.find(item => item.name === 'Expandable details');
    aa = await call('click',{session_id:'owner-a',ref:target.ref,revision:aa.revision,screenshot:false});
    assert.match(aa.text,/Expanded fixture content/,'Native disclosure summaries remain actionable');
    target = aa.elements.find(item => item.name === 'Native option');
    aa = await call('type_text',{session_id:'owner-a',ref:target.ref,revision:aa.revision,text:'second-id'});
    assert.match(aa.text,/Selected second-id/,'Native select commits only an exact existing option');
    for (const invalid of ['Unknown option','disabled-id','Duplicate']) {
      target = aa.elements.find(item => item.name === 'Native option');
      const rejected = await call('type_text',{session_id:'owner-a',ref:target.ref,revision:aa.revision,text:invalid});
      assert.equal(rejected.ok,false); assert.match(rejected.error.message,/option|ambiguous|unavailable/i);
      aa = await call('snapshot',{session_id:'owner-a'});
      assert.match(aa.text,/Selected second-id/);
    }
    target = aa.elements.find(item => item.name === 'Native option');
    aa = await call('type_text',{session_id:'owner-a',ref:target.ref,revision:aa.revision,text:'dup-b'});
    assert.match(aa.text,/Selected dup-b/,'Duplicate labels require an exact value');
    target = aa.elements.find(item => item.name === 'Native option');
    aa = await call('type_text',{session_id:'owner-a',ref:target.ref,revision:aa.revision,text:'Second option'});
    assert.match(aa.text,/Selected second-id/,'A unique exact option label is also accepted');
    assert.match(aa.text,/Native form not submitted/,'Filling a native select never presses Enter or submits its form');
    target = aa.elements.find(item => item.name === 'Implicit field');
    aa = await call('type_text',{session_id:'owner-a',ref:target.ref,revision:aa.revision,text:'Modal draft'});
    target = aa.elements.find(item => item.name === 'Open earlier confirmation');
    aa = await call('click',{session_id:'owner-a',ref:target.ref,revision:aa.revision,screenshot:false});
    assert.ok(aa.elements.some(item => item.name === 'Confirmation reason'), 'Top-layer modal can precede its parent in DOM order');
    assert.ok(!aa.elements.some(item => item.name === 'Implicit field'), 'Parent modal is blocked by its confirmation');
    target = aa.elements.find(item => item.name === 'Confirm modal');
    aa = await call('click',{session_id:'owner-a',ref:target.ref,revision:aa.revision,screenshot:false});
    target = aa.elements.find(item => item.name === 'Close labelled editor');
    aa = await call('click',{session_id:'owner-a',ref:target.ref,revision:aa.revision,screenshot:false});
    await call('stop',{session_id:'owner-a'});
    bb = await call('snapshot',{session_id:'owner-b'});
    assert.match(bb.text,/B draft/,'Stopping another owner must not stop this browser');
    const field = bb.elements.find(item => item.name === 'Name');
    bb = await call('type_text',{session_id:'owner-b',ref:field.ref,revision:bb.revision,text:''});
    assert.doesNotMatch(bb.text,/B draft/,'Clearing a field is a real browser input');
    for (const [label,value,addition] of [['Quantity','12','3'],['Notes','备注','更多']]) {
      let field = bb.elements.find(item => item.name === label);
      bb = await call('type_text',{session_id:'owner-b',ref:field.ref,revision:bb.revision,text:value});
      field = bb.elements.find(item => item.name === label);
      bb = await call('type_text',{session_id:'owner-b',ref:field.ref,revision:bb.revision,text:addition,clear:false,delay_ms:0});
      assert.ok(bb.text.includes(value+addition),label+' supports real input and append');
    }
    const notes = bb.elements.find(item => item.name === 'Notes');
    bb = await call('press_key',{session_id:'owner-b',ref:notes.ref,revision:bb.revision,key:'Backspace'});
    assert.ok(bb.text.includes('备注更'),'Referenced key input focuses the exact element');
    assert.ok(!bb.text.includes('备注更多'));
    await call('stop',{session_id:'owner-b'});
    console.log('PASS Browser Use: isolated owners, Unicode/append/clear, preview admission, 20 action rounds and independent shutdown');
  } finally {
    bridge.kill('SIGTERM');
    if(bridge.exitCode===null&&bridge.signalCode===null) await new Promise(resolve=>bridge.once('exit',resolve));
    await new Promise(resolve=>server.close(resolve));
    fs.rmSync(root,{recursive:true,force:true,maxRetries:5,retryDelay:100});
  }
}
main().catch(error=>{console.error(error);process.exitCode=1});
