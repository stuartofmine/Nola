import test from 'node:test';
import assert from 'node:assert/strict';
import {inlineQuery,mentionSegments,mentionTargets,reconcileMentionEdit,replaceMentionRange,storedMentions,partMentions} from './inlineExpertMentions.ts';
const target={agent_id:'quality',agent_revision:3,participant_id:'member',context_conversation_id:'child'};
test('a typed query belongs to the caret; email and accepted mentions stay literal',()=>{
    assert.deepEqual(inlineQuery('🙂请@测后文',5,[]),{start:3,end:5,query:'测'});
    assert.equal(inlineQuery('first.last@example',18,[]),null);
    const first=replaceMentionRange('🙂请@测后文',[],3,5,'@测试',target);
    assert.equal(first.text,'🙂请@测试后文');assert.equal(first.cursor,6);
    assert.equal(inlineQuery(first.text,6,first.mentions),null);
});
test('ordered occurrences preserve duplicate names and Unicode; only exact targets deduplicate',()=>{
    const second={...target,agent_id:'quality-2',participant_id:'member-2',context_conversation_id:'child-2'};
    const entries=[{start:0,text:'@测试',selection:target},{start:4,text:'@测试',selection:second},{start:8,text:'@测试',selection:target}];
    const text='@测试，@测试；@测试。'; const wire=mentionSegments(text,entries);
    assert.deepEqual(mentionTargets(entries),[target,second]);
    assert.deepEqual(storedMentions({...wire,selected_experts:[target,second]},text),entries);
    assert.deepEqual(storedMentions({...wire,selected_experts:[target,second]},text+'?'),[]);
    assert.deepEqual(reconcileMentionEdit(text,text.slice(0,4)+text.slice(7),4,entries).map(m=>[m.start,m.selection.agent_id]),[[0,'quality'],[5,'quality']]);
    assert.deepEqual(replaceMentionRange(text,entries,0,3,'@测试').mentions.map(m=>m.start),[4,8]);
    assert.throws(()=>mentionSegments(text.slice(1),entries));
    assert.throws(()=>mentionTargets([entries[0],{...entries[0],selection:{...target,agent_revision:4}}]));
});

test('reads only a complete frozen occurrence projection and leaves legacy text alone',()=>{
    const text='Reply @测试', occurrences=[{start:6,text:'@测试',selection:target}];
    const value={...mentionSegments(text,occurrences),selected_experts:[target]};
    const projection=(availability:string)=>({projection:{fields:{expert_mentions:{availability,value}}}});
    assert.deepEqual(partMentions(projection('complete'),text),occurrences);
    assert.deepEqual(partMentions(projection('deferred'),text),[]);
    assert.deepEqual(partMentions({markdown:text},text),[]);
});
