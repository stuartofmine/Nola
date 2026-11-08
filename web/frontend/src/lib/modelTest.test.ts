import test from 'node:test';
import assert from 'node:assert/strict';
import { activeApiTestModel, activeLocalTestModel, modelTestConversationRequest, modelTestParametersUrl, modelTestRunRequest } from './modelTest.ts';
import type { RemoteProfile } from './modelProfiles.ts';
import { createServer } from 'vite';
import { modelTestMessages } from './locales/modelTest.ts';
import { supportedLocales } from './i18n.ts';

const profile = { id: 'profile', enabled: true, selected: true, endpoint: { enabled: true }, capabilities: ['chat'] } as RemoteProfile;

test('model test only uses a selected eligible model', () => {
    assert.equal(activeApiTestModel([{ ...profile, enabled: false }, profile])?.id, 'profile');
    assert.equal(activeApiTestModel([{ ...profile, capabilities: ['retrieval'] }]), undefined);
    assert.equal(activeLocalTestModel([{ id: 'a', selected: true, installed: false, running: false }, { id: 'b', selected: true, installed: true, running: false }])?.id, 'b');
});

test('model test creates an explicitly bound safe chat-only run', () => {
    const create = modelTestConversationRequest('cloud', 'profile', 'Model Test');
    assert.deepEqual(create, { title: 'Model Test', selectedModelSource: 'cloud', selectedModelId: 'profile', executionBackend: 'nola' });
    const request = modelTestRunRequest({ conversationId: 'test-1' } as never, 'client', 'submission', 'Hello', 'en', 'UTC');
    assert.equal(request.permission_profile, 'cautious');
    assert.equal(request.agentMode, false);
    assert.equal(request.model_test_mode, true);
    assert.equal(request.assistant_message_id, 'submission:assistant');
    assert.deepEqual(request.messages, [{ role: 'user', content: 'Hello' }]);
    assert.throws(() => modelTestConversationRequest('cloud', '', 'Model Test'));
});

test('model test parameters open the selected model editor, including opaque ids', () => {
    assert.equal(modelTestParametersUrl('local', 'mlx/model 1'), '/models?tab=local&configure=mlx%2Fmodel%201');
    assert.equal(modelTestParametersUrl('cloud', 'profile:a&b'), '/models?tab=api&configure=profile%3Aa%26b');
    assert.throws(() => modelTestParametersUrl('cloud', ''));
});

test('the Svelte model test has a routed, localized setup and result surface', async () => {
    const keys = Object.keys(modelTestMessages.en).sort();
    for (const language of supportedLocales) {
        assert.deepEqual(Object.keys(modelTestMessages[language]).sort(), keys);
        assert.ok(modelTestMessages[language]['modelTest.defaultPrompt']);
    }
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        const { default: ModelTest } = await server.ssrLoadModule('/src/pages/ModelTest.svelte');
        const { render } = await server.ssrLoadModule('svelte/server');
        const html: string = render(ModelTest).body;
        assert.match(html, /model-test-prompt/);
        assert.match(html, /\/models/);
        assert.match(html, /role="tablist"/);
        assert.match(html, /data-active-pane="setup"/);
        assert.match(html, /id="model-test-setup"/);
        assert.match(html, /id="model-test-output"/);
    } finally { await server.close(); }
});
