import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parseExpertChildPage,currentExpertChildMember,expertChildContinuationUrl,expertChildHistoryPayload,expertChildExchangeSenders} from './expertChildConversations.ts';
const child={conversation_id:'child-old',revision:2,title:'Frozen name',participant_id:'member-old',agent_id:'agent',agent_revision:1,is_current:true,active:true,created_at_ms:1,updated_at_ms:2,message_count:3};
const page={version:1,conversation_id:'parent',room_id:'room',room_revision:1,children:[child],has_more:false,next_offset:null};
const member={id:'member-old',agent_id:'agent',agent_revision:1,display_name:'Frozen',context_conversation_id:'child-old',active:true,description:''};
const room={revision:1,room:{id:'room',title:'',participants:[member]}};
test('expert child pages preserve unknown historical revision and validate exact parent and advancing cursor',()=>{
    assert.equal(parseExpertChildPage({...page,children:[{...child,agent_revision:null,is_current:false}]},'parent',0).children[0].agent_revision,null);
    for(const invalid of [{...page,version:2},{...page,conversation_id:'other'},{...page,children:[child,child]},
        {...page,children:[{...child,conversation_id:'parent'}]},{...page,has_more:true,next_offset:0},{...page,next_offset:1}])assert.throws(()=>parseExpertChildPage(invalid,'parent',0));
    assert.equal(parseExpertChildPage({...page,has_more:true,next_offset:41},'parent',40).next_offset,41);
});
test('continue binds the existing pinned member, while old/rejoined histories remain read-only',()=>{
    assert.equal(currentExpertChildMember(room,child),member);
    for(const changed of [{...child,is_current:false},{...child,active:false},{...child,agent_revision:null},{...child,participant_id:'new'},{...child,conversation_id:'new-child'}])assert.equal(currentExpertChildMember(room,changed),undefined);
    assert.throws(()=>expertChildContinuationUrl('parent','Host',{...child,is_current:false}));
    const url=new URL(expertChildContinuationUrl('parent','Host',child),'http://local');
    assert.equal(url.searchParams.get('conversation'),'parent');assert.equal(url.searchParams.get('expert_context'),'child-old');
});
test('bounded child history rejects another parent, context, room, and unavailable source',()=>{
    const value={version:1,conversation_id:'parent',child_conversation_id:'child-old',room_id:'room',source_owner:{kind:'local'},window:{projection:{revision:2},event_sequence:3,before_ordinal:1,has_more:true,total_message_count:10}};
    assert.equal(expertChildHistoryPayload(value,'parent',child,'room','Host').payload.messageWindow.eventSequence,3);
    for(const invalid of [{...value,conversation_id:'other'},{...value,child_conversation_id:'other'},{...value,room_id:'other'},{...value,source_owner:{kind:'unavailable'}}])assert.throws(()=>expertChildHistoryPayload(invalid,'parent',child,'room','Host'));
});
test('peer author derives only from explicit exchange identity and frozen source display name',()=>{
    const peer={exchange_id:'exchange',parent_conversation_id:'parent',target:{context_conversation_id:child.conversation_id,participant_id:child.participant_id,agent_id:child.agent_id},source:{agent_id:'source-agent'},source_display_name:'审查专家'};
    assert.deepEqual(expertChildExchangeSenders([peer],child,'parent'),{'exchange:user':{name:'审查专家',agentId:'source-agent'}});
    assert.deepEqual(expertChildExchangeSenders(undefined,child,'parent'),{});
    assert.throws(()=>expertChildExchangeSenders([{...peer,target:{...peer.target,context_conversation_id:'other'}}],child,'parent'));
    assert.throws(()=>expertChildExchangeSenders([peer,peer],child,'parent'));
});


test('Main task provenance is bound to the actual parent and old expert authors remain readable',()=>{
    const child={conversation_id:'child',participant_id:'member',agent_id:'target'} as import('./expertChildConversations.ts').ExpertChildConversation;
    const peer={exchange_id:'exchange-main',parent_conversation_id:'parent',target:{context_conversation_id:'child',participant_id:'member',agent_id:'target'},source:{kind:'main_agent',context_conversation_id:'parent'}};
    assert.deepEqual(expertChildExchangeSenders([peer],child,'parent'),{'exchange-main:user':{name:'Nola',agentId:'parent'}});
    assert.throws(()=>expertChildExchangeSenders([{...peer,source:{...peer.source,context_conversation_id:'foreign'}}],child,'parent'));
    assert.throws(()=>expertChildExchangeSenders([{...peer,source:{kind:'unknown',agent_id:'guessed'}}],child,'parent'));
});
