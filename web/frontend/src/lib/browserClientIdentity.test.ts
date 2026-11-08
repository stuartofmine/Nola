import test from 'node:test';
import assert from 'node:assert/strict';
import { browserIdentityKey, ensureBrowserClientId, readBrowserClientId } from './browserClientIdentity.ts';

test('tabs share durable Host-scoped identity; no session-storage fallback', async t => {
    const data = new Map<string,string>();
    const storage = { getItem: (key:string) => data.get(key) ?? null, setItem: (key:string,value:string) => { data.set(key,value); } };
    let tail = Promise.resolve();
    const locks = { request: (_:string, fn:()=>string) => { const result=tail.then(fn); tail=result.then(()=>{},()=>{}); return result; } };
    Object.defineProperty(globalThis,'localStorage',{configurable:true,value:storage});
    Object.defineProperty(globalThis,'navigator',{configurable:true,value:{locks}});
    t.after(()=>{ Reflect.deleteProperty(globalThis,'localStorage'); Reflect.deleteProperty(globalThis,'navigator'); });
    const [a,b]=await Promise.all([ensureBrowserClientId('host-a'),ensureBrowserClientId('host-a')]);
    assert.equal(a,b); assert.equal(readBrowserClientId('host-a'),a);
    assert.equal(await ensureBrowserClientId('host-a'),a);
    assert.notEqual(await ensureBrowserClientId('host-b'),a);
    data.set(browserIdentityKey('broken'),'invalid owner!');
    await assert.rejects(ensureBrowserClientId('broken'),/browser_recovery_storage_unavailable/);
    storage.setItem=()=>{throw Error('quota');};
    await assert.rejects(ensureBrowserClientId('host-a'),/browser_recovery_storage_unavailable/);
    assert.equal(data.get(browserIdentityKey('host-a')),a);
});
