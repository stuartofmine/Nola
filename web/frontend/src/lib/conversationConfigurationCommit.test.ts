import test from 'node:test';
import assert from 'node:assert/strict';
import { committedConversationConfiguration, ConversationConfigurationConflict } from './conversationConfigurationCommit.ts';

test('configuration adapter accepts only typed committed receipt and preserves conflict versions', () => {
    const snapshot = {conversationId: 'owner', configurationRevision: 8};
    assert.deepEqual(committedConversationConfiguration({status: 'committed', snapshot}), snapshot);
    assert.throws(() => committedConversationConfiguration(snapshot));
    assert.throws(() => committedConversationConfiguration({status: 'not_found', conversation_id: 'owner'}));
    assert.throws(() => committedConversationConfiguration({status: 'conflict', expected_revision: 7, actual_revision: 9}),
        error => error instanceof ConversationConfigurationConflict);
});
