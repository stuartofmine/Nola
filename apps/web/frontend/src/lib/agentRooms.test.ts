import test from 'node:test';
import assert from 'node:assert/strict';
import { mentionCandidates, mentionQuery, removeMentionQuery, selectedRecipient, mergeRoomSubmissions, pendingRoomRetry, type RoomSubmission, type RoomSnapshot } from './agentRooms.ts';

const room: RoomSnapshot = {revision: 1, room: {id: 'room', title: 'Team', participants: [
    {id: 'one', agent_id: 'agent-one', agent_revision: 1, display_name: '研究 Current', context_conversation_id: 'context-one', active: true, description: 'Research'},
    {id: 'two', agent_id: 'agent-two', agent_revision: 1, display_name: '研究 Current', context_conversation_id: 'context-two', active: true, description: 'Review'},
    {id: 'gone', agent_id: 'agent-three', agent_revision: 1, display_name: 'Search', context_conversation_id: 'context-three', active: false, description: ''},
]}};
test('late polls and send acknowledgements cannot regress or erase room receipts', () => {
    const receipt: RoomSubmission = {input: {room_id: 'room', submission_id: 'one', expected_room_revision: 1, text: 'hello', participant_id: null, at_ms: 1}, state: 'completed', revision: 5, context_conversation_id: 'context-one', selected_participant_id: 'one', failure_code: null, execution_id: 'run'};
    const newer = {...receipt, input: {...receipt.input, submission_id: 'two', at_ms: 2}};
    assert.deepEqual(mergeRoomSubmissions('room', [receipt, newer], [{...receipt, revision: 1, state: 'accepted'}]), [receipt, newer]);
    assert.deepEqual(mergeRoomSubmissions('room', [receipt], [{...receipt, revision: 6, input: {...receipt.input, text: 'different'}}]), [receipt]);
    assert.deepEqual(mergeRoomSubmissions('room', [], [{...receipt, input: {...receipt.input, room_id: 'other'}}]), []);
    assert.equal(mergeRoomSubmissions('room', [{...receipt, revision: 1, state: 'accepted'}], [receipt])[0].state, 'completed');
});
test('uncertain retry preserves the original submission and cannot cross room ownership', () => {
    const pending = {room_id: 'room', submission_id: 'fixed', expected_room_revision: 1, text: 'original', participant_id: 'one', at_ms: 1};
    const draft = {text: 'edited next message', participantId: 'two', pending, error: true};
    assert.strictEqual(pendingRoomRetry('room', draft), pending);
    assert.equal(pendingRoomRetry('other', draft), null);
    assert.equal(pendingRoomRetry('room', {...draft, error: false}), null);
    assert.equal(pendingRoomRetry('room', {text: 'new', participantId: null, error: true}), null);
});
test('mention choices retain stable identities even when display names match', () => {
    assert.deepEqual(mentionCandidates(room, '研究').map(member => member.id), ['one', 'two']);
    assert.deepEqual(mentionCandidates(room, 'CURRENT').map(member => member.id), ['one', 'two']);
    assert.deepEqual(mentionCandidates(room, 'search'), []);
});
test('ordinary text, empty selection and stale members never select a recipient', () => {
    for (const id of [null, '研究 Current', 'search', 'gone', 'unknown']) assert.equal(selectedRecipient(room, id), undefined);
    assert.equal(selectedRecipient(room, 'two')?.context_conversation_id, 'context-two');
});
test('completion syntax does not reinterpret email addresses or natural language', () => {
    assert.equal(mentionQuery('请 @研'), '研');
    assert.equal(mentionQuery('@'), '');
    for (const value of ['mail@example.org', 'search current', '当前搜索', 'please @Agent continue']) assert.equal(mentionQuery(value), null);
    assert.equal(removeMentionQuery('请 @研'), '请 ');
});
