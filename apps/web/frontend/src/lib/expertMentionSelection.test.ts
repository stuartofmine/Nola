import {test} from 'node:test';
import assert from 'node:assert/strict';
import {checkedExpertCandidate, existingExpertParticipant, rejoinedExpertParticipant, selectedExpertMembers} from './expertMentionSelection.ts';
import type {PersistentAgent, RoomSnapshot} from './agentRooms.ts';
const agent:PersistentAgent={id:'opaque-agent',name:'Search current / 编写',description:'',revision:2,availability:'active',defaults:{}};
const old={id:'old',agent_id:agent.id,agent_revision:1,display_name:'Old name',context_conversation_id:'old-context',active:true,description:''};
const before:RoomSnapshot={revision:4,room:{id:'room',title:'',participants:[old]}};
const current={...old,id:'new',agent_revision:2,context_conversation_id:'fresh-context',display_name:agent.name};
const after:RoomSnapshot={revision:5,room:{id:'room',title:'',participants:[current]}};

test('mentions use directory IDs/revisions, including experts not yet in a room; names never resolve a candidate',()=>{
    assert.equal(checkedExpertCandidate({revision:3,agents:[agent]},agent.id,2),agent);
    assert.throws(()=>checkedExpertCandidate({revision:3,agents:[agent]},agent.name,2));
    assert.throws(()=>checkedExpertCandidate({revision:3,agents:[{...agent,availability:'archived'}]},agent.id,2));
    assert.throws(()=>checkedExpertCandidate({revision:3,agents:[agent]},agent.id,1));
    assert.equal(existingExpertParticipant({...before,room:{...before.room,participants:[]}},agent),undefined);
    assert.throws(()=>existingExpertParticipant(before,agent));
});

test('explicit reintroduction receipt must preserve the old child and introduce a new owner',()=>{
    assert.equal(rejoinedExpertParticipant(before,after,agent).id,'new');
    for (const malformed of [before,{...after,room:{...after.room,id:'other'}},
        {...after,room:{...after.room,participants:[]}},
        {...after,room:{...after.room,participants:[old,current]}},
        {...after,room:{...after.room,participants:[{...old,active:false},{...current,context_conversation_id:'old-context'}]}}]) {
        assert.throws(()=>rejoinedExpertParticipant(before,malformed,agent));
    }
});

test('a selected old room snapshot survives directory changes but cannot silently become a new child',()=>{
    assert.deepEqual(selectedExpertMembers(before,[old.id]),[old]);
    assert.throws(()=>existingExpertParticipant(before,agent),'new @ latest still requires explicit rejoin');
    assert.throws(()=>selectedExpertMembers(after,[old.id]));
    assert.throws(()=>selectedExpertMembers(before,[old.id,old.id]));
});
