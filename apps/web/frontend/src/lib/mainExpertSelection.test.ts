import test from 'node:test';
import assert from 'node:assert/strict';
import {mainExpertSelections} from './mainExpertSelection.ts';
import type {RoomSnapshot} from './agentRooms.ts';
const member={id:'member',agent_id:'expert',agent_revision:2,context_conversation_id:'child',active:true,display_name:'研究',description:''};
const room={revision:3,room:{id:'room',title:'',participants:[member]}} as RoomSnapshot;
test('Main guidance pins the actual active member identity and rejects names or duplicates',()=>{
    assert.deepEqual(mainExpertSelections(room,['member']),[{agent_id:'expert',agent_revision:2,participant_id:'member',context_conversation_id:'child'}]);
    assert.deepEqual(mainExpertSelections(null,[]),[]);
    assert.throws(()=>mainExpertSelections(null,['member']));
    assert.throws(()=>mainExpertSelections(room,['expert']));
    assert.throws(()=>mainExpertSelections(room,['member','member']));
    assert.throws(()=>mainExpertSelections({...room,room:{...room.room,participants:[{...member,active:false}]}},['member']));
});

test('Main delegation capability is additive and an old catalog never silently consumes selections',async()=>{
    const {supportsMainExpertDelegation}=await import('./expertRoles.ts');
    assert.equal(supportsMainExpertDelegation({version:1,features:{main_agent_delegation:true}}),true);
    for(const value of [{version:1,features:{reasoning_effort:true}},{version:2,features:{main_agent_delegation:true}},null])assert.equal(supportsMainExpertDelegation(value),false);
});

test('Main capability guidance accepts exact protocol IDs without matching user prose',async()=>{
    const {mainExpertSelectionError}=await import('./mainExpertSelection.ts');
    for(const id of ['main_expert_selection_requires_tool_capable_main_model','main_expert_selection_requires_enabled_expert_dispatch'])assert.equal(mainExpertSelectionError(id),'rooms.mainRequiresTools');
    assert.equal(mainExpertSelectionError('main_expert_selection_source_changed'),'rooms.expertCatalogUnavailable');
    for(const value of [null,'unknown','please '+ 'main_expert_selection_requires_tool_capable_main_model'])assert.equal(mainExpertSelectionError(value),null);
});
