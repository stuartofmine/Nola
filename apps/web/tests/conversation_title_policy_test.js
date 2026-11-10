// SPDX-License-Identifier: Apache-2.0
const assert = require('assert');

global.window = global;
require('../app/static/js/conversation-title-policy.js');

const id = '01DAF2BB-1906-4993-9B5A-9AC74E05B5D8';
for (const title of [id, '00000000-0000-0000-0000-000000000000', 'Session 01DAF2BB', 'Conversation 01DAF2BB']) {
    assert.equal(NolaConversationTitle.displayTitle(title, id, 'New conversation'), 'New conversation');
}
assert.equal(NolaConversationTitle.displayTitle('Session planning', id, 'New conversation'), 'Session planning');
assert.equal(NolaConversationTitle.displayTitle('Conversation design review', id, 'New conversation'), 'Conversation design review');
console.log('Conversation title presentation policy: passed');
