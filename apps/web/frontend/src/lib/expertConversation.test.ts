import test from 'node:test';
import assert from 'node:assert/strict';
import { expertDirectoryMatches, groupExpertSessions, projectExpertSubmissions, toggleExpertParticipant } from './expertConversation.ts';
import type { PersistentAgent, RoomParticipant, RoomSubmission } from './agentRooms.ts';

const ada: RoomParticipant = { id: 'participant-ada', agent_id: 'ada', agent_revision: 1, display_name: 'Ada', context_conversation_id: 'ctx-ada', active: true, description: '' };
const lin: RoomParticipant = { id: 'participant-lin', agent_id: 'lin', agent_revision: 1, display_name: 'Lin', context_conversation_id: 'ctx-lin', active: true, description: '' };
function reply(id: string, who: RoomParticipant, at: number, parent = 'user-1'): RoomSubmission {
    return { input: { room_id: 'room-1', submission_id: id, expected_room_revision: 1, text: `Reply ${id}`, participant_id: who.id, at_ms: at, parent_message_id: parent, conversation_id: 'conversation-1' }, state: 'completed', context_conversation_id: who.context_conversation_id, selected_participant_id: who.id, revision: 1, failure_code: null, execution_id: `run-${id}` };
}

test('projects only host-attributed replies in canonical submission order', () => {
    const projected = projectExpertSubmissions('room-1', 'conversation-1', [ada, lin], [reply('a1', ada, 3), reply('l1', lin, 2), reply('a2', ada, 4), reply('a3', ada, 5)], new Set(['user-1']));
    assert.deepEqual(projected.map(item => item.submission.input.submission_id), ['l1', 'a1', 'a2', 'a3']);
});

test('fails closed for hidden parents, foreign rooms and mismatched participant contexts', () => {
    const inactive = { ...lin, active: false };
    const invalid = [
        reply('hidden', ada, 1, 'deleted-user'),
        {...reply('foreign-conversation', ada, 1), input: {...reply('foreign-conversation', ada, 1).input, conversation_id:'other'}},
        { ...reply('foreign', ada, 2), input: { ...reply('foreign', ada, 2).input, room_id: 'room-2' } },
        { ...reply('mismatch', ada, 4), context_conversation_id: 'ctx-other' },
        { ...reply('legacy', ada, 5), input: { ...reply('legacy', ada, 5).input, parent_message_id: null } },
    ];
    assert.deepEqual(projectExpertSubmissions('room-1', 'conversation-1', [ada, inactive], invalid, new Set(['user-1'])), []);
    const historical = projectExpertSubmissions('room-1', 'conversation-1', [inactive], [reply('inactive-history', inactive, 6)], new Set(['user-1']));
    assert.equal(historical.length, 1, 'deactivation must not erase an already admitted reply from history');
});

test('expert selection toggles persisted participant IDs without duplicating or routing by agent ID', () => {
    assert.deepEqual(toggleExpertParticipant(['participant-ada'], 'participant-ada'), []);
    assert.deepEqual(toggleExpertParticipant([], 'participant-ada'), ['participant-ada']);
    assert.deepEqual(toggleExpertParticipant(['participant-lin'], 'participant-ada'), ['participant-lin', 'participant-ada']);
});

test('missing or malformed participant identity fails closed without losing valid history', () => {
    const malformed = { ...reply('bad-id', ada, 1), selected_participant_id: null };
    const legacy = { ...reply('legacy-no-parent', ada, 2), input: { ...reply('legacy-no-parent', ada, 2).input } };
    delete legacy.input.parent_message_id;
    const projected = projectExpertSubmissions('room-1', 'conversation-1', [ada], [malformed, legacy, reply('good', ada, 3)], new Set(['user-1']));
    assert.deepEqual(projected.map(item => item.submission.input.submission_id), ['good']);
});

test('the sidebar groups ordered turns by exact context, not display name', () => {
    const duplicateName = {...lin, display_name: ada.display_name};
    const items = projectExpertSubmissions('room-1', 'conversation-1', [ada, duplicateName],
        [reply('a2', ada, 3), reply('l1', duplicateName, 2), reply('a1', ada, 1)], new Set(['user-1']));
    assert.deepEqual(groupExpertSessions(items).map(session => [
        session.participant.context_conversation_id, session.replies.map(item => item.submission.input.submission_id),
    ]), [['ctx-ada', ['a1', 'a2']], ['ctx-lin', ['l1']]]);
});

test('mention directory matches mixed-width names while excluding selected or inactive experts', () => {
    const expert = (id: string, availability = 'active'): PersistentAgent =>
        ({id, name: 'Ada 研究', availability, revision: 1, defaults: {}, description: ''});
    assert.deepEqual(expertDirectoryMatches([expert('match'), expert('chosen'), expert('disabled', 'inactive')],
        'ＡＤＡ', new Set(['chosen'])).map(agent => agent.id), ['match']);
});
