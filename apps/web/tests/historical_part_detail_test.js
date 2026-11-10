// SPDX-License-Identifier: Apache-2.0
const assert = require('node:assert/strict');
const detail = require('../app/static/js/historical-part-detail.js');
const fixture = require('../../../core/tests/fixtures/historical_unloaded_conversation_projection_v1.json');
const part = fixture.messages[0].parts[0];
const request = detail.descriptor(part);
assert(request);
const loaded = structuredClone(part.projection);
loaded.status = { state:'supported' };
loaded.fields = { input:{availability:'complete', value:'<script>untrusted</script>'}, output:{availability:'complete',value:null} };
const pending = [];
const observed = [];
let current = request;
const loader = detail.createLoader({
    read: (value, signal) => new Promise((resolve, reject) => pending.push({resolve,reject,signal,value})),
    isCurrent: value => value === current,
    onState: state => observed.push(state),
});
(async () => {
    const first=loader.open(request);
    assert.equal(observed.at(-1).state,'loading');
    loader.close();
    pending.shift().resolve(loaded);
    await first;
    assert.equal(observed.at(-1).state,'closed');
    const stale=loader.open(request);
    current=null;
    pending.shift().resolve(loaded);
    await stale;
    assert.equal(observed.at(-1).state,'closed');
    current=request;
    const failed=loader.open(request);
    pending.shift().reject(new Error('revision changed'));
    await failed;
    assert.equal(observed.at(-1).state,'error');
    const retry=loader.open(request);
    pending.shift().resolve(loaded);
    await retry;
    assert.equal(observed.at(-1).state,'loaded');
    assert.deepEqual(observed.at(-1).result.fields.output,{availability:'complete',value:null});
    const displayed=detail.format(loaded,{supported:'History',fieldDeferred:'Not loaded'});
    assert(displayed.includes('<script>untrusted</script>')); // UI uses textContent, never HTML.
    assert(displayed.includes('null'));
    const wrong=structuredClone(loaded); wrong.owner.part_id='foreign';
    assert.throws(()=>detail.validate(wrong,request),/owner or revision/);
    const large=structuredClone(loaded); large.canonical_text={text:'x'.repeat(32768),total_utf8_bytes:32768};
    assert.throws(()=>detail.validate(large,request),/byte budget/);
    const unknown=structuredClone(loaded); unknown.status.state='executing';
    assert.throws(()=>detail.validate(unknown,request),/Unknown historical/);
    const deferred=structuredClone(part.projection);
    deferred.status={state:'deferred'};
    deferred.fields={body:{availability:'deferred',locator:{}}};
    assert(detail.format(deferred,{deferred:'Partial',fieldDeferred:'Not loaded'}).includes('Not loaded'));
    // Detail reads never touch the input window/projection or its event sequence.
    assert.equal(part.projection.status.state,'not_loaded');
    assert.deepEqual(part.projection.fields,{});
    current=null; loader.invalidate();
    assert.equal(observed.at(-1).state,'closed');
    // Run the real pagination function, substituting only template localization.
    // A page request may finish after the user opens a detail on the old window.
    const fs = require('node:fs');
    const vm = require('node:vm');
    const path = require('node:path');
    const template = fs.readFileSync(path.join(__dirname, '../app/templates/chat.html'), 'utf8');
    const pagination = template.slice(template.indexOf('async function loadEarlierMessages()'),
        template.indexOf('\nfunction renderMsg(', template.indexOf('async function loadEarlierMessages()')))
        .replace(/{{[\s\S]*?}}/g, 'Localized text');
    let finishPage;
    let finishDetail;
    const pageStates=[];
    const pagingLoader=detail.createLoader({read:()=>new Promise(resolve=>{finishDetail=resolve;}),
        isCurrent:()=>true, onState:state=>pageStates.push(state)});
    const sandbox={ currentConvId:'history', currentMessageWindow:{hasMore:true,loading:false,before:4},
        currentMessages:[{messageId:'old'}], historicalDetailLoader:pagingLoader,
        chatMessages:{scrollHeight:100,scrollTop:3,innerHTML:''},
        renderHistoryWindowControl(){}, apiJson:()=>new Promise(resolve=>{finishPage=resolve;}),
        mapMessage:value=>value, renderMsg:()=>'', hydrateMermaidBlocks(){},
        conversationWindowCache:new Map(), showError:error=>{throw new Error(error);} };
    vm.createContext(sandbox); vm.runInContext(pagination,sandbox);
    const pageWork=sandbox.loadEarlierMessages();
    const detailWork=pagingLoader.open(request);
    finishPage({messages:[{messageId:'older'}],messageWindow:{hasMore:false}});
    await pageWork;
    assert.equal(pageStates.at(-1).state,'closed');
    finishDetail(loaded); await detailWork;
    assert.equal(pageStates.at(-1).state,'closed');
    assert.equal(sandbox.currentMessages.length,2);
    console.log('Historical detail request state, identity, budget, retry and stale rejection: passed');
})().catch(error=>{console.error(error);process.exitCode=1;});
