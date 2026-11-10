import assert from 'node:assert/strict';
import test from 'node:test';
import { localSessionProfile, parseSessionConfiguration, parseSessionLocalModels, parseSessionProfiles, permissionPatchConfirmed, sessionConfigurationPatch, sessionDraft, sessionDraftError, sessionModelKey, sessionModelSelectionConfirmed, sessionPatchHasChanges, sessionPermissionPatch, sessionReasoningOptions, selectableSessionProfile } from './chatSessionSettings.ts';
import { createServer } from 'vite';
import type { ChatConversation } from './chatConversations.ts';
import type { RemoteProfile } from './modelProfiles.ts';
import { sessionConfigurationPatchConfirmed, sessionPermissionApplicationStatus } from './chatSessionSettings.ts';
import { supportedLocales, t } from './i18n.ts';

test('retrieval override stays independent of memory and validates exact sparse receipts', () => {
    const state = parseSessionConfiguration({ configurationRevision: 1, permissionProfile: null, memoryEnabledOverride: false, retrievalEnabledOverride: null, modelOverrides: {} });
    for (const [retrieval, value] of [['on', true], ['off', false]] as const) {
        const patch = sessionConfigurationPatch(state, null, { ...sessionDraft(state, null), retrieval }, []);
        assert.deepEqual(patch, { expectedRevision: 1, retrievalEnabledOverride: value });
        assert.equal(sessionConfigurationPatchConfirmed({ ...state, configurationRevision: 2, retrievalEnabledOverride: value }, patch), true);
        assert.equal(sessionConfigurationPatchConfirmed({ ...state, configurationRevision: 2 }, patch), false);
        const overridden = { ...state, retrievalEnabledOverride: value };
        assert.deepEqual(sessionConfigurationPatch(overridden, null, { ...sessionDraft(overridden, null), retrieval: 'inherit' }, []), { expectedRevision: 1, retrievalEnabledOverride: null });
    }
    for (const retrievalEnabledOverride of [undefined, 'false', 0]) assert.throws(() => parseSessionConfiguration({ ...state, retrievalEnabledOverride }));
    for (const language of supportedLocales) for (const key of ['retrieval', 'retrievalScope']) assert.notEqual(t(language, `chatSession.${key}`), `chatSession.${key}`);
});

test('permission status consumes the authoritative application state after its configuration commit', () => {
    const saved = parseSessionConfiguration({ configurationRevision: 12, memoryEnabledOverride: null,
        retrievalEnabledOverride: null, permissionProfile: 'allow_all', modelOverrides: {},
        activeRunID: 'run', activePermissionPolicy: null, permissionState: 'pending' });
    assert.equal(sessionPermissionApplicationStatus(saved, 'allow_all', true, false, true, 12), 'saving');
    assert.equal(sessionPermissionApplicationStatus(saved, 'allow_all', false, false, true, 12), 'pending');
    assert.equal(sessionPermissionApplicationStatus({ ...saved, activePermissionPolicy: {
        run_id: 'run', configuration_revision: 11, profile: 'allow_all',
    } }, 'allow_all', false, false, true, 12), 'pending');
    assert.equal(sessionPermissionApplicationStatus({ ...saved, permissionState: 'active', activePermissionPolicy: {
        run_id: 'run', configuration_revision: 12, profile: 'allow_all',
    } }, 'allow_all', false, false, true, 12), 'active');
    assert.equal(sessionPermissionApplicationStatus(saved, 'cautious', false, true, true, 12), 'failed');
    assert.throws(() => parseSessionConfiguration({ ...saved, activePermissionPolicy: {
        run_id: 'other-run', configuration_revision: 12, profile: 'allow_all',
    } }));
});

test('memory override uses canonical tri-state sparse patches and exact revision receipts', () => {
    const state = parseSessionConfiguration({ retrievalEnabledOverride: null, configurationRevision: 1, permissionProfile: null, memoryEnabledOverride: null, modelOverrides: {} });
    for (const [memory, value] of [['on', true], ['off', false]] as const) {
        const draft = { ...sessionDraft(state, null), memory };
        const patch = sessionConfigurationPatch(state, null, draft, []);
        assert.deepEqual(patch, { expectedRevision: 1, memoryEnabledOverride: value });
        assert.equal(sessionConfigurationPatchConfirmed({ ...state, configurationRevision: 2, memoryEnabledOverride: value }, patch), true);
        assert.equal(sessionConfigurationPatchConfirmed({ ...state, configurationRevision: 2 }, patch), false);
        assert.equal(sessionConfigurationPatchConfirmed({ ...state, memoryEnabledOverride: value }, patch), false);
        const overridden = { ...state, memoryEnabledOverride: value };
        assert.deepEqual(sessionConfigurationPatch(overridden, null, { ...sessionDraft(overridden, null), memory: 'inherit' }, []), { expectedRevision: 1, memoryEnabledOverride: null });
    }
    for (const memoryEnabledOverride of [undefined, 'true', 0]) assert.throws(() => parseSessionConfiguration({ ...state, memoryEnabledOverride }));
});

const conversation: ChatConversation = { revision: 3, configurationRevision: 7, isPinned: false, conversationId: 'c', title: 'Chat', createdAt: '', updatedAt: '', selectedModelSource: 'cloud', selectedModelId: 'p', executionBackend: 'nola' };
const profile: RemoteProfile = { id: 'p', displayName: 'P', modelName: 'p', revision: 1, enabled: true, selected: false, priority: 50, order: 0,
    endpoint: { id: 'e', name: 'E', baseURL: '', wireProtocol: 'responses', enabled: true, order: 0 }, capabilities: ['chat'],
    parameters: { maxOutputTokens: 8192, temperature: 0.7 }, reasoning: { control: 'effort', efforts: ['low', 'high'], budgets: [] } };

test('only a real enabled chat-capable profile may edit a selected cloud model', () => {
    assert.deepEqual(parseSessionProfiles({ profiles: [profile] }), [profile]);
    assert.throws(() => parseSessionProfiles({ profiles: [{ ...profile, endpoint: null }] }));
    assert.throws(() => parseSessionProfiles({ profiles: [profile, profile] }));
    assert.equal(sessionModelKey(conversation), 'configured:p');
    assert.equal(selectableSessionProfile(conversation, [profile]), profile);
    assert.equal(selectableSessionProfile(conversation, [{ ...profile, capabilities: [] }]), null);
    assert.equal(selectableSessionProfile(conversation, [{ ...profile, endpoint: { ...profile.endpoint, enabled: false } }]), null);
    assert.equal(selectableSessionProfile({ ...conversation, selectedModelId: 'unknown' }, [profile]), null);
    assert.equal(sessionModelKey({ ...conversation, selectedModelSource: 'auto', selectedModelId: '' }), null);
});

test('installed local models can be selected and receive conversation-scoped defaults', () => {
    const models = parseSessionLocalModels({ models: [
        { id: 'local-a', name: 'Local A', installed: true, running: false, parameters: { temperature: 0.4, topP: 0.8, contextTokens: 65536 } },
        { id: 'local-b', name: 'Local B', installed: false, running: false, parameters: {} },
    ] });
    const selected = { ...conversation, selectedModelSource: 'local', selectedModelId: 'local-a' };
    assert.equal(sessionModelKey(selected), 'local:local-a');
    const local = localSessionProfile(selected, models);
    assert.equal(local?.displayName, 'Local A');
    assert.equal(local?.parameters.temperature, 0.4);
    assert.equal(local?.parameters.maxContextLength, 65536);
    assert.equal(localSessionProfile({ ...selected, selectedModelId: 'local-b' }, models), null);
    assert.equal(sessionModelSelectionConfirmed(selected, { selectedModelSource: 'local', selectedModelId: 'local-a' }), true);
    assert.equal(sessionModelSelectionConfirmed(selected, { selectedModelSource: 'local', selectedModelId: 'local-b' }), false);
    assert.equal(sessionModelSelectionConfirmed(conversation, { selectedModelSource: 'local', selectedModelId: 'local-a' }), false);
    assert.throws(() => parseSessionLocalModels({ models: [{ id: 'same', installed: true, running: false, parameters: {} }, { id: 'same', installed: true, running: false, parameters: {} }] }));
    assert.throws(() => parseSessionLocalModels({ models: [{ id: 'bad', installed: true, running: false, parameters: [] }] }));
});

test('effort controls expose their complete declared values while budget controls retain their supported tiers', () => {
    assert.deepEqual(sessionReasoningOptions(profile), ['low', 'high']);
    assert.deepEqual(sessionReasoningOptions({ ...profile, endpoint: { ...profile.endpoint, wireProtocol: 'chat_completions' } }), []);
    assert.deepEqual(sessionReasoningOptions({ ...profile, endpoint: { ...profile.endpoint, wireProtocol: 'chat_completions' }, reasoning: { control: 'effort', efforts: ['low', 'medium', 'high'], budgets: [] } }), ['low', 'medium', 'high']);
    assert.deepEqual(sessionReasoningOptions({ ...profile, endpoint: { ...profile.endpoint, wireProtocol: 'anthropic' }, reasoning: { control: 'budget', efforts: [], budgets: [1024, 2048, 4096] } }), ['concise', 'balanced', 'detailed']);
    assert.deepEqual(sessionReasoningOptions(profile, { wireProtocol: 'anthropic', reasoning: { control: 'budget', efforts: [], budgets: [1024, 2048, 9999] } }), []);
});

test('session patch is CAS scoped, sparse, and separates inherited from explicit provider default', () => {
    const state = parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 12, permissionProfile: 'cautious', modelOverrides: { 'configured:p': { temperature: 0, seed: { kind: 'value', value: '0' }, reasoningPreference: 'high', customField: 'keep' } } });
    const draft = sessionDraft(state, 'configured:p');
    assert.equal(draft.numbers.temperature, '0'); assert.equal(draft.seedValue, '0');
    assert.equal(sessionPatchHasChanges(sessionConfigurationPatch(state, 'configured:p', draft, ['low', 'high'])), false);
    draft.permission = 'allow_all'; draft.numbers.temperature = ''; draft.reasoningPreference = 'automatic';
    const patch = sessionConfigurationPatch(state, 'configured:p', draft, ['low', 'high']);
    assert.deepEqual(patch, { expectedRevision: 12, permissionProfile: 'allow_all', modelOverrides: { 'configured:p': { temperature: null, reasoningPreference: 'automatic' } } });
    draft.reasoningPreference = 'inherit';
    assert.deepEqual(sessionConfigurationPatch(state, 'configured:p', draft, ['low', 'high']), { expectedRevision: 12, permissionProfile: 'allow_all', modelOverrides: { 'configured:p': { temperature: null, reasoningPreference: null } } });
});

test('invalid and unknown values fail closed instead of being coerced', () => {
    assert.throws(() => parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 2, permissionProfile: 'moderate', modelOverrides: {} }));
    assert.throws(() => parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 2, permissionProfile: null, modelOverrides: { x: [] } }));
    const state = parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 2, permissionProfile: null, modelOverrides: {} });
    const draft = sessionDraft(state, 'configured:p');
    draft.reasoningPreference = 'xhigh';
    assert.equal(sessionDraftError(draft, ['low']), 'chatSession.invalidReasoning');
    assert.throws(() => sessionConfigurationPatch(state, 'configured:p', draft, ['low']));
    draft.reasoningPreference = 'inherit'; draft.numbers.maxOutputTokens = '10';
    assert.equal(sessionDraftError(draft, ['low']), 'chatSession.invalidNumber');
});

test('seed intent preserves exact i64 text and separates inherit, clear, and zero through save/reopen', () => {
    const read = (fields: Record<string, unknown>) => parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 7, permissionProfile: null, modelOverrides: { 'configured:p': fields } });
    for (const value of ['0', '-1', '9007199254740993', '-9223372036854775808', '9223372036854775807']) {
        const state = read({});
        const draft = sessionDraft(state, 'configured:p');
        assert.equal(draft.seedMode, 'inherit');
        draft.seedMode = 'value'; draft.seedValue = value;
        const patch = sessionConfigurationPatch(state, 'configured:p', draft, []);
        assert.deepEqual(patch, { expectedRevision: 7, modelOverrides: { 'configured:p': { seed: { kind: 'value', value } } } });
        const reopened = read(JSON.parse(JSON.stringify(patch.modelOverrides))['configured:p']);
        const restored = sessionDraft(reopened, 'configured:p');
        assert.equal(restored.seedValue, value);
        restored.seedMode = 'clear';
        assert.deepEqual(sessionConfigurationPatch(reopened, 'configured:p', restored, []).modelOverrides, { 'configured:p': { seed: { kind: 'clear' } } });
        const cleared = read({ seed: { kind: 'clear' } });
        const reset = sessionDraft(cleared, 'configured:p');
        assert.equal(reset.seedMode, 'clear'); reset.seedMode = 'inherit';
        assert.deepEqual(sessionConfigurationPatch(cleared, 'configured:p', reset, []).modelOverrides, { 'configured:p': { seed: null } });
    }
    for (const value of ['', '+1', '-0', '01', '-01', ' 1', '1 ', '1\n', '1\r\n', '1.0', '1e3', '9223372036854775808', '-9223372036854775809']) {
        const state = read({}); const draft = sessionDraft(state, 'configured:p');
        draft.seedMode = 'value'; draft.seedValue = value;
        assert.equal(sessionDraftError(draft, []), 'chatSession.invalidSeed');
        assert.throws(() => sessionConfigurationPatch(state, 'configured:p', draft, []));
    }
    for (const seed of [0, 42, null, '0', {}, { kind: 'clear', value: '1' }, { kind: 'value', value: 0 }, { kind: 'value', value: '0', extra: true }]) {
        assert.throws(() => read({ seed }));
    }
});

test('compact permission CAS is sparse and cannot submit unsaved model parameters', () => {
    const state = parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 12, permissionProfile: 'cautious', modelOverrides: { 'configured:p': { temperature: 0.7 } } });
    const draft = sessionDraft(state, 'configured:p');
    draft.numbers.temperature = '1.2'; draft.permission = 'allow_all';
    const patch = sessionPermissionPatch(state, draft.permission)!;
    assert.deepEqual(patch, { expectedRevision: 12, permissionProfile: 'allow_all' });
    assert.equal(permissionPatchConfirmed(parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 13, permissionProfile: 'allow_all', modelOverrides: state.modelOverrides }), patch), true);
    assert.equal(permissionPatchConfirmed(parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 12, permissionProfile: 'allow_all', modelOverrides: state.modelOverrides }), patch), false);
    assert.equal(permissionPatchConfirmed(parseSessionConfiguration({ retrievalEnabledOverride: null, memoryEnabledOverride: null, configurationRevision: 13, permissionProfile: 'cautious', modelOverrides: state.modelOverrides }), patch), false);
    assert.throws(() => sessionPermissionPatch(state, 'inherit' as never));
    assert.equal(sessionPermissionPatch(state, 'cautious'), null);
    assert.throws(() => sessionPermissionPatch(state, 'unsafe' as never));
});

test('stateless composer permission widget renders seven-language scope and does not own an API client', async () => {
    const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
    try {
        // Compile the actual editor as well as the stateless permission widget.
        await server.ssrLoadModule('/src/components/ChatSessionSettings.svelte');
        const { default: Widget } = await server.ssrLoadModule('/src/components/ChatPermissionMode.svelte');
        const { locale, supportedLocales, t } = await server.ssrLoadModule('/src/lib/i18n.ts');
        const { render } = await server.ssrLoadModule('svelte/server');
        const state = { choice: 'cautious', savedChoice: 'cautious', ready: true, busy: false, saving: false,
            conflict: false, canSave: true, errorKey: '', noticeKey: '' };
        for (const language of supportedLocales) {
            locale.set(language);
            for (const key of ['seedClear', 'seedValue', 'invalidSeed']) {
                const translated = t(language, `chatSession.${key}`);
                assert.ok(translated && translated !== `chatSession.${key}`);
            }
            const html: string = render(Widget, { props: { state, onselect: () => {}, onsave: () => {}, onreload: () => {} } }).body;
            assert.ok(html.includes(t(language, 'chatSession.permission')));
            assert.ok(html.includes(t(language, 'chatPermissionMode.scope')));
            assert.match(html, /id="chat-permission-mode"/);
        }
    } finally { await server.close(); }
});

test('current Host automatic reasoning declaration does not disable unrelated session settings', () => {
    const automatic = { ...profile, reasoning: { control: 'automatic' as const } };
    const catalog = parseSessionProfiles({ profiles: [profile, automatic.id === profile.id ? { ...automatic, id: 'automatic' } : automatic] });
    assert.equal(catalog.length, 2);
    assert.deepEqual(sessionReasoningOptions(catalog[1]), []);
    assert.deepEqual(parseSessionProfiles({ profiles: [{ ...profile, reasoning: { control: 'effort', efforts: ['low', 'high'] } }] })[0].reasoning, { control: 'effort', efforts: ['low', 'high'] });
    for (const reasoning of [{ control: 'unknown' }, { control: 'effort' }, { control: 'budget' }, { control: 'automatic', efforts: 'high' }, { control: 'automatic', budgets: ['1024'] }]) {
        assert.throws(() => parseSessionProfiles({ profiles: [{ ...profile, reasoning }] }));
    }
});

test('five declared efforts remain independently selectable and do not collapse to three tiers', () => {
    const efforts = ['none', 'low', 'medium', 'high', 'xhigh'];
    const five = { ...profile, reasoning: { control: 'effort' as const, efforts, budgets: [] } };
    assert.deepEqual(sessionReasoningOptions(five), efforts);
});
