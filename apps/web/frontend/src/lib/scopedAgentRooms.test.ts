import test from 'node:test';
import assert from 'node:assert/strict';
import {roomCommandAtSource} from './agentRooms.ts';

test('scoped Room transport pins the epoch from its own response and refuses replacement/unknown envelopes',async()=>{
    const original=globalThis.fetch;
    const documentDescriptor=Object.getOwnPropertyDescriptor(globalThis,'document');
    Object.defineProperty(globalThis,'document',{configurable:true,value:{cookie:''}});
    const body={type:'directory'};
    try {
        globalThis.fetch=async (_path,options)=>{
            assert.deepEqual(JSON.parse(String(options?.body)),{command:body,expected_source_store_epoch:'original'});
            return new Response(JSON.stringify({source_store_epoch:'original',source_matches:true,result:{agents:[]}}),{headers:{'content-type':'application/json'}});
        };
        assert.deepEqual(await roomCommandAtSource(body,'original'),{sourceEpoch:'original',result:{agents:[]}});
        for(const payload of [{source_store_epoch:'replaced',source_matches:true,result:{}},{source_store_epoch:'original',source_matches:false,result:null},{result:{agents:[]}}]){
            globalThis.fetch=async()=>new Response(JSON.stringify(payload),{headers:{'content-type':'application/json'}});
            await assert.rejects(roomCommandAtSource(body,'original'));
        }
    } finally {globalThis.fetch=original;if(documentDescriptor)Object.defineProperty(globalThis,'document',documentDescriptor);else Reflect.deleteProperty(globalThis,'document');}
});
