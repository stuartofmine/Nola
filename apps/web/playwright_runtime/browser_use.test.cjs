// SPDX-License-Identifier: Apache-2.0
'use strict';
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {EventEmitter} = require('node:events');
const {PassThrough, Writable} = require('node:stream');
const {BrowserUseExecutor} = require('./browser_use.cjs');

function fixture(reply) {
  const child = new EventEmitter();
  child.stdout = new PassThrough(); child.stderr = new PassThrough();
  child.exitCode = null; child.signalCode = null; child.commands = [];
  child.stdin = new Writable({write(data,encoding,done){
    const request = JSON.parse(String(data)); child.commands.push(request); reply(child,request); done();
  }});
  child.kill = () => { child.signalCode='SIGTERM';child.emit('exit',null,'SIGTERM'); };
  child.stdin.on('finish',()=>{child.exitCode=0;child.emit('exit',0)});
  const executor = new BrowserUseExecutor(child);
  const send = value => child.stdout.write(JSON.stringify(value)+'\n');
  const connection = {send:async()=>({targetInfo:{targetId:'owned-target'}}),detach:async()=>{connection.detached=true}};
  const page = {context:()=>({newCDPSession:async()=>connection}),evaluate:async()=>{}};
  return {child,executor,send,page,connection};
}
const ready = {ready:true,protocol:1,engine:'browser-use',version:'1.0.2'};

test('unknown worker protocol or dependency version cannot become ready',async()=>{
  for(const incompatible of [{...ready,protocol:2},{...ready,version:'2.0.0'}]){
    const f=fixture(()=>{});const rejected=assert.rejects(f.executor.ready,/version|pinned/);
    f.send(incompatible);await rejected;assert.equal(f.child.commands.length,0);await f.executor.close();
  }
});
test('an uncertain input response is surfaced once without retry or transport identity override',async()=>{
  const f=fixture((child,request)=>child.stdout.write(JSON.stringify({id:request.id,ok:false,error:'disconnected after input'})+'\n'));
  f.send(ready);await f.executor.ready;
  await assert.rejects(f.executor.perform(f.page,'text',{text:'once',id:999,operation:'python',target_id:'other'}),/disconnected/);
  assert.equal(f.child.commands.length,1);assert.equal(f.child.commands[0].operation,'text');
  assert.equal(f.child.commands[0].target_id,'owned-target');assert.notEqual(f.child.commands[0].id,999);
  assert.equal(f.connection.detached,true);await f.executor.close();
});
test('worker exit rejects the in-flight action and never starts a replacement worker',async()=>{
  const f=fixture(child=>queueMicrotask(()=>child.kill()));f.send(ready);await f.executor.ready;
  await assert.rejects(f.executor.perform(f.page,'text',{text:'once'}),/exited/);
  assert.equal(f.child.commands.length,1);await f.executor.close();
});

test('a worker stalled in third-party signal cleanup is forcibly stopped',async()=>{
  const f=fixture(()=>{});f.send(ready);await f.executor.ready;
  const signals=[];
  f.child.kill=signal=>{signals.push(signal);if(signal==='SIGKILL'){f.child.signalCode=signal;f.child.emit('exit',null,signal)}};
  const exited=new Promise(resolve=>f.child.once('exit',resolve));f.executor.terminate();await exited;
  assert.deepEqual(signals,['SIGTERM','SIGKILL']);await f.executor.close();
});
