import test from 'node:test';
import assert from 'node:assert/strict';
import { MessageAdmissionReader } from './messageAdmissions.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';
import { createChatTurn } from './chatTurn.ts';

const owner = { hostId: 'host', conversationId: 'conversation', clientId: 'client', generation: 1 };
test('fixed admission frontier caches exact declarations and does not replay pages', async () => {
    const fixture = terminalFixture(createChatTurn(owner, 'submission'));
    const reader = new MessageAdmissionReader(owner.hostId, owner.conversationId, fixture.transport, () => true);
    const messages = await reader.read(fixture.seal.identity, fixture.seal.admission);
    assert.equal(messages[0].input.id, 'submission:assistant');
    assert.equal(messages[0].input.status, 'streaming', 'shell is not completion authority');
    assert.equal(await reader.read(fixture.seal.identity, fixture.seal.admission), messages);
    assert.equal(fixture.requests.filter(request => request.operation === 'admissions').length, 1);
});

test('foreign root, corrupt declaration and stale generation cannot admit shells', async () => {
    for (const scenario of ['root', 'digest', 'stale']) {
        const fixture = terminalFixture(createChatTurn(owner, 'submission'));
        const reader = new MessageAdmissionReader(owner.hostId, owner.conversationId, async request => {
            const response = await fixture.transport(request) as { result: { records?: { text: string }[] }; source_host_id: string };
            if (scenario === 'digest' && response.result.records) response.result.records[0].text = response.result.records[0].text.replace('streaming', 'completed');
            return response;
        }, () => scenario !== 'stale');
        const root = { ...fixture.seal.identity, assistant_message_id: scenario === 'root' ? 'foreign' : 'submission:assistant' };
        await assert.rejects(reader.read(root, fixture.seal.admission));
    }
});
