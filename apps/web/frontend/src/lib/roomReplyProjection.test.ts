import test from 'node:test';
import assert from 'node:assert/strict';
import {createRoomReplyProjection} from './roomReplyProjection.ts';
import {terminalFixture} from './assistantTerminal.fixture.ts';
import {createChatTurn} from './chatTurn.ts';
import type {RunObservation} from './runObservation.ts';
import type {TimelineMessage} from './chatTimeline.ts';

const owner={hostId:'host',clientId:'room:room',conversationId:'child',generation:1,runId:'execution',assistantMessageId:'execution:assistant'};
function fixture() {
    const turn=createChatTurn(owner,{submissionId:owner.runId,userMessageId:'execution:user',assistantMessageId:owner.assistantMessageId});
    const shared=terminalFixture(turn,'completed',[],true);
    const requests:{path:string;body:Record<string,unknown>}[]=[];
    const published:TimelineMessage[]=[];
    const request=async<T>(path:string,options?:RequestInit):Promise<T>=>{
        const body=JSON.parse(String(options?.body)); requests.push({path,body});
        if(body.operation==='heads')return {source_host_id:owner.hostId,result:[shared.through]} as T;
        return await shared.transport(body,options?.signal??undefined) as T;
    };
    const projection=createRoomReplyProjection(owner,request,message=>published.push(message));
    const observed=(terminal=false):RunObservation=>({protocol_version:1,conversation_id:owner.conversationId,client_id:owner.clientId,
        run_id:owner.runId,assistant_message_id:owner.assistantMessageId,status:terminal?'sealed':'running',
        worker:{state:terminal?'absent':'running',generation:'1'},recovery:terminal?'terminal':'observe',
        admission:shared.seal.admission,terminal_seal:terminal?shared.seal:null,permissions:[],can_submit:false,
        submission_receipt:null,goal_execution:null,context_usage:null});
    return {projection,requests,published,observed};
}
test('expert readonly projection reuses journal cache and verifies sealed terminal without history polling or dispatch',async()=>{
    const f=fixture();
    assert.equal(await f.projection.accept(f.observed()),true);
    assert.equal(await f.projection.accept(f.observed()),true);
    assert.equal(f.requests.filter(item=>item.body.operation==='parts').length,1);
    assert.equal(await f.projection.accept(f.observed(true)),false);
    assert.equal(f.published.at(-1)?.id,owner.assistantMessageId);
    assert.equal(f.published.at(-1)?.state,'completed');
    assert.ok(f.requests.every(item=>item.path==='/api/chat/agent/v2/journal'));
    assert.ok(!f.requests.some(item=>['submit','cancel','resume','prepare'].includes(String(item.body.operation))));
    f.projection.dispose();
});
test('expert exact Run ownership and disposed owner cannot consume another execution',async()=>{
    const f=fixture();
    await assert.rejects(f.projection.accept({...f.observed(),run_id:'other-run'}),/exact Run/);
    assert.equal(f.requests.length,0);
    f.projection.dispose();
    assert.equal(await f.projection.accept(f.observed()),false);
    assert.equal(f.published.length,0);
});
