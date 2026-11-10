import { test } from 'node:test';
import assert from 'node:assert/strict';
import { commitSessionModel, recoverSessionModel } from './sessionModelCommit.ts';
import type { ChatConversation } from './chatConversations.ts';
import type { SessionConfiguration, SessionModelChoiceState } from './chatSessionSettings.ts';
const conversation: ChatConversation = { conversationId: 'one', title: 'One', revision: 1, configurationRevision: 7, createdAt: '', updatedAt: '', isPinned: false, selectedModelSource: 'cloud', selectedModelId: 'old', executionBackend: 'nola' };
const snapshot: SessionConfiguration = { configurationRevision: 9, memoryEnabledOverride: null, retrievalEnabledOverride: null, permissionProfile: 'cautious', modelOverrides: {} };
const state: SessionModelChoiceState = { choice: 'cloud:old', disabled: false, errorKey: '', options: [ { value: 'cloud:new', name: 'Same name', detail: 'Host A', local: false }, { value: 'local:new', name: 'Same name', detail: '', local: true } ] };
const result = { ...conversation, configurationRevision: 10, selectedModelId: 'new' };
const input = { conversation, snapshot, state, choice: 'cloud:new', isCurrent: () => true };

test('saving parameters then selecting a model uses the saved configuration revision', async () => {
    let calls = 0;
    const saved = await commitSessionModel({ ...input, request: async (url, options) => {
        calls++; assert.equal(url, '/api/chat/conversations/one');
        assert.deepEqual(JSON.parse(options.body), { selectedModelSource: 'cloud', selectedModelId: 'new', expectedConfigurationRevision: 9 });
        return result;
    } });
    assert.equal(calls, 1); assert.equal(saved?.selectedModelId, 'new');
});
test('local selection and default inheritance preserve exact source and omit stale ids', async () => {
    for (const choice of ['local:new', 'auto']) {
        const saved = await commitSessionModel({ ...input, choice, request: async (_url, options) => {
            const body = JSON.parse(options.body);
            assert.equal(body.selectedModelSource, choice === 'auto' ? 'auto' : 'local');
            assert.equal(body.selectedModelId, choice === 'auto' ? undefined : 'new');
            return { ...result, selectedModelSource: body.selectedModelSource, selectedModelId: body.selectedModelId ?? '' };
        } });
        assert.equal(saved?.selectedModelSource, choice === 'auto' ? 'auto' : 'local');
    }
});
test('loading, pending drafts/conflicts, absent configuration and duplicate selection never dispatch', async () => {
    for (const patch of [{ state: { ...state, disabled: true } }, { snapshot: null }, { choice: state.choice }, { isCurrent: () => false }]) {
        assert.equal(await commitSessionModel({ ...input, ...patch, request: async () => { assert.fail('must not dispatch'); } }), null);
    }
});
test('unknown models are rejected before any mutation', async () => {
    await assert.rejects(commitSessionModel({ ...input, choice: 'cloud:unknown', request: async () => { assert.fail('must not dispatch'); } }), /invalidModel/);
});
test('wrong owner, unchanged revision and mismatched source/model receipts are rejected', async () => {
    for (const patch of [{ conversationId: 'other' }, { configurationRevision: 9 }, { configurationRevision: 8 }, { selectedModelId: 'wrong' }, { selectedModelSource: 'local' }]) {
        await assert.rejects(commitSessionModel({ ...input, request: async () => ({ ...result, ...patch }) }), /invalidResponse/);
    }
});
test('network errors and conflicts never automatically repeat a model mutation', async () => {
    for (const failure of [new Error('network'), new Error('409 conflict')]) {
        let calls = 0;
        await assert.rejects(commitSessionModel({ ...input, request: async () => { calls++; throw failure; } }), failure);
        assert.equal(calls, 1);
    }
});
test('late response after conversation switch cannot update the new conversation', async () => {
    let current = true;
    let resolve!: (value: unknown) => void;
    const pending = commitSessionModel({ ...input, isCurrent: () => current, request: () => new Promise(done => resolve = done) });
    current = false; resolve(result);
    assert.equal(await pending, null);
});
test('configuration older than conversation fails closed without dispatch', async () => {
    await assert.rejects(commitSessionModel({ ...input, snapshot: { ...snapshot, configurationRevision: 6 }, request: async () => { assert.fail('must not dispatch'); } }), /invalidResponse/);
});

test('lost model receipt recovery reads authoritative conversation without replaying PATCH', async () => {
    const recovered = await recoverSessionModel('one', async url => {
        assert.equal(url, '/api/chat/conversations');
        return { conversations: [result] };
    }, () => true);
    assert.equal(recovered?.selectedModelId, 'new');
    assert.equal(recovered?.configurationRevision, 10);
});
test('recovery rejects deleted/malformed conversations and discards reads after navigation', async () => {
    await assert.rejects(recoverSessionModel('one', async () => ({ conversations: [] }), () => true), /invalidResponse/);
    await assert.rejects(recoverSessionModel('one', async () => ({ conversations: [{}] }), () => true), /invalidResponse/);
    assert.equal(await recoverSessionModel('one', async () => ({ conversations: [result] }), () => false), null);
});


test('default selection accepts Core resolved identity, including an initially unselected conversation', async () => {
    const saved = await commitSessionModel({ ...input, conversation: { ...conversation, selectedModelSource: 'auto', selectedModelId: '' },
        state: { ...state, choice: 'auto' }, choice: 'auto', request: async (_url, options) => {
            assert.equal(JSON.parse(options.body).selectedModelSource, 'auto');
            return result;
        } });
    assert.equal(saved?.selectedModelSource, 'cloud');
    assert.equal(saved?.selectedModelId, 'new');
});
