import test from 'node:test';
import assert from 'node:assert/strict';
import { loadTaskConversation, taskDetailsOwner } from './chatTaskDetails.ts';
import { journalPart } from './journalTimeline.fixture.ts';
const identity = {source_store_epoch:'epoch', conversation_id:'parent', run_id:'original-run', assistant_message_id:'message'};
const part = journalPart(identity, {part_id:'part',kind:'task',task_id:'task',result_ref:'nola-journal://not-a-conversation/digest'});
const owner = {conversationId:'parent', runId:'original-run', taskId:'task'};
const task = {id:'task',owner_session_id:'parent',owner_turn_id:'original-run',lineage:{child_session_id:'child /?id'}};
test('task navigation uses exact Core lineage and existing conversation API', async () => {
    assert.deepEqual(taskDetailsOwner(part), owner);
    assert.deepEqual(taskDetailsOwner({...part, value: {...part.value, kind: 'task_control'}}), owner);
    assert.equal(taskDetailsOwner({source: 'canonical', id: part.id, value: {...part.value, revision: 1}}), null, 'a canonical summary has no owned journal Run');
    const href = await loadTaskConversation(owner, async path => {
        assert.equal(path, '/api/chat/conversations/parent/messages?limit=1');
        return {taskProjection:{tasks:{task}}};
    });
    assert.equal(href, '/chat?conversation=child%20%2F%3Fid');
});
test('missing or foreign task ownership never navigates using result pointer', async () => {
    for (const changed of [{...task,owner_turn_id:'new-run'}, {...task,owner_session_id:'foreign'}, {...task,id:'other'}, {...task,lineage:{child_session_id:'parent'}}]) {
        await assert.rejects(loadTaskConversation(owner, async () => ({taskProjection:{tasks:{task:changed}}})));
    }
    await assert.rejects(loadTaskConversation(owner, async () => ({})));
    assert.equal(await loadTaskConversation(owner, async () => ({taskProjection:{tasks:{task:{...task,lineage:{child_session_id:null}}}}})), null);
});

test('task-only ownership is source typed and never inferred from child-like names', async () => {
    const {conversationExecutionOwnership} = await import('./chatTaskDetails.ts');
    assert.deepEqual(conversationExecutionOwnership({kind:'user'}, 'agent-name'), {kind:'user'});
    assert.deepEqual(conversationExecutionOwnership(undefined, 'agent-name'), {kind:'unavailable'});
    const owner = {kind:'task_child',parent_conversation_id:'parent',parent_run_id:'run',task_id:'task',child_conversation_id:'child'};
    assert.deepEqual(conversationExecutionOwnership(owner, 'child'), owner);
    assert.deepEqual(conversationExecutionOwnership(owner, 'foreign'), {kind:'unavailable'});
    assert.deepEqual(conversationExecutionOwnership({...owner,parent_conversation_id:'child'}, 'child'), {kind:'unavailable'});
});
