import { test } from 'node:test';
import assert from 'node:assert/strict';
import { executionParameterValidationError, groupedProfiles, parameterChanges, reasoningDraft, reasoningChanges, metadataDraft, metadataChanges, reasoningOptions, reasoningValidationError, canSelectProfile } from './modelProfiles.ts';
import type { ModelExecutionCapabilities, RemoteProfile } from './modelProfiles.ts';

test('default selection is available only for enabled chat models on enabled endpoints', () => {
    const profile = { enabled: true, capabilities: ['chat'], endpoint: { enabled: true } } as RemoteProfile;
    assert.equal(canSelectProfile(profile), true);
    assert.equal(canSelectProfile({ ...profile, enabled: false }), false);
    assert.equal(canSelectProfile({ ...profile, capabilities: ['retrieval'] }), false);
    assert.equal(canSelectProfile({ ...profile, endpoint: { ...profile.endpoint, enabled: false } }), false);
});

test('budget controls only expose valid tiers under the edited output limit', () => {
    const draft = reasoningDraft({ control: 'budget', budgets: [1024, 2048, 4096], efforts: [], defaultPreference: 'detailed' });
    assert.equal(reasoningValidationError(draft, 'anthropic', 8192), null);
    assert.deepEqual(reasoningOptions(draft, 'anthropic', 4096), []);
    assert.ok(reasoningValidationError(draft, 'anthropic', 4096));
    for (const budgets of [[1024, 2048], [1024, 2048, 2048], [2048, 1024, 4096], [1024, 2048.5, 4096], [1024, NaN, 4096]]) {
        assert.ok(reasoningValidationError({ ...draft, budgets }, 'anthropic', 8192));
    }
});

test('editor rejects protocol mismatches and missing declared efforts before save', () => {
    const draft = reasoningDraft({ control: 'effort', efforts: ['low', 'high'], budgets: [] });
    assert.equal(reasoningValidationError(draft, 'responses', 512), null);
    assert.equal(reasoningValidationError(draft, 'chat_completions', 512), null);
    assert.equal(reasoningValidationError(draft, 'anthropic', 16384), null);
    assert.ok(reasoningValidationError(draft, 'unknown', 16384));
    assert.deepEqual(reasoningOptions(draft, 'unknown'), []);
    assert.ok(reasoningValidationError({ ...draft, efforts: [] }, 'responses', 16384));
    assert.ok(reasoningValidationError({ ...draft, efforts: ['unsupported'] }, 'responses', 16384));
    assert.ok(reasoningValidationError({ ...draft, control: 'automatic', budgets: [-1] }, 'responses', 16384), 'Changing control must not persist invalid inactive budgets');
});

test('stale defaults are retained for correction, not silently replaced', () => {
    const draft = reasoningDraft({ control: 'effort', efforts: ['low'], budgets: [], defaultPreference: 'high' });
    assert.ok(reasoningValidationError(draft, 'responses', 16384));
    assert.equal(draft.defaultPreference, 'high');
    assert.equal(reasoningValidationError({ ...draft, defaultPreference: 'automatic' }, 'responses', 16384), null);
    assert.equal(reasoningValidationError({ ...draft, defaultPreference: null }, 'responses', 16384), null);
});

test('Mac thinking defaults survive Web edits and choices follow the protocol', () => {
    const stored = { control: 'effort' as const, efforts: ['low', 'high'], budgets: [1024, 4096, 8192], defaultPreference: 'high' };
    const draft = reasoningDraft(stored);
    assert.equal(draft.defaultPreference, 'high');
    assert.deepEqual(reasoningChanges(stored, draft), {});
    assert.deepEqual(reasoningOptions(draft, 'responses'), ['low', 'high']);
    assert.deepEqual(reasoningOptions(draft, 'chat_completions'), ['low', 'high']);
    draft.efforts.push('medium');
    assert.deepEqual(reasoningOptions(draft, 'chat_completions'), ['low', 'medium', 'high']);
    draft.defaultPreference = 'automatic';
    assert.equal(reasoningChanges(stored, draft).reasoning?.defaultPreference, 'automatic');
    assert.equal(stored.defaultPreference, 'high');
    assert.deepEqual(reasoningOptions({ ...draft, control: 'budget' }, 'anthropic'), ['concise', 'balanced', 'detailed']);
});

test('model metadata drafts isolate arrays and send sparse edits including false and zero', () => {
    const profile = { displayName: 'Original', enabled: true, priority: 50, capabilities: ['chat', 'retrieval'] } as RemoteProfile;
    const draft = metadataDraft(profile);
    draft.capabilities.reverse();
    assert.deepEqual(metadataChanges(profile, draft), {});
    draft.enabled = false;
    draft.priority = 0;
    draft.displayName = 'Renamed';
    draft.capabilities.push('issue_repair');
    assert.deepEqual(profile.capabilities, ['chat', 'retrieval']);
    assert.deepEqual(metadataChanges(profile, draft), { displayName: 'Renamed', enabled: false, priority: 0, capabilities: ['retrieval', 'chat', 'issue_repair'] });
});

test('execution capability edits are not confused with product uses', () => {
    const profile = { displayName: 'Fixture', enabled: true, priority: 50, capabilities: ['chat'] } as RemoteProfile;
    const draft = metadataDraft(profile);
    draft.executionCapabilities.reasoning = true;
    assert.deepEqual(metadataChanges(profile, draft), {}, 'Changing an unconfirmed draft is not execution authority.');
    draft.executionCapabilitiesConfirmed = true;
    assert.deepEqual(metadataChanges(profile, draft), { executionCapabilities: {
        toolCalls: true, imageInput: false, reasoning: true, samplingParameters: false, samplingRequiresNoReasoning: false,
        chatOutputTokenField: 'auto',
    } });
    assert.deepEqual(profile.capabilities, ['chat']);
});

test('Chat output-limit field is saved with the exact model execution contract', () => {
    const profile = { displayName: 'Fixture', enabled: true, priority: 50, capabilities: ['chat'] } as RemoteProfile;
    const draft = metadataDraft(profile);
    draft.executionCapabilities.chatOutputTokenField = 'max_completion_tokens';
    draft.executionCapabilitiesConfirmed = true;
    assert.equal((metadataChanges(profile, draft).executionCapabilities as ModelExecutionCapabilities).chatOutputTokenField,
        'max_completion_tokens');
});

test('missing declaration and explicit all-false declaration are different admission facts', () => {
    const missing = { displayName: 'Unknown', capabilities: ['chat'], parameters: {},
        endpoint: { wireProtocol: 'chat_completions' } } as RemoteProfile;
    assert.equal(executionParameterValidationError(missing)?.code, 'modelValidation.executionDeclaration');
    assert.equal(metadataDraft(missing).executionCapabilitiesConfirmed, false);
    const declared = { ...missing, executionCapabilities: {
        toolCalls: false, imageInput: false, reasoning: false, samplingParameters: false,
    } };
    assert.equal(executionParameterValidationError(declared), null);
    assert.equal(metadataDraft(declared).executionCapabilitiesConfirmed, true);
    assert.equal(metadataDraft(declared).executionCapabilities.toolCalls, false, 'Explicit provider false overrides import default');
    const confirmed = metadataDraft(missing);
    confirmed.executionCapabilitiesConfirmed = true;
    const change = metadataChanges(missing, confirmed);
    assert.equal((change.executionCapabilities as ModelExecutionCapabilities).toolCalls, true);
    assert.equal(Object.hasOwn(change, 'executionCapabilitiesConfirmed'), false);
});

test('unsupported sampling and protocol parameters fail before saving a model', () => {
    const profile: RemoteProfile = { id: 'fixture', displayName: 'Fixture', modelName: 'fixture', revision: 1,
        enabled: true, selected: true, priority: 50, order: 0, capabilities: ['chat'],
        endpoint: { id: 'endpoint', name: 'Fixture', baseURL: 'https://fixture.invalid/v1', wireProtocol: 'responses', enabled: true, order: 0 },
        parameters: { temperature: 0.2 },
        executionCapabilities: { toolCalls: false, imageInput: false, reasoning: false, samplingParameters: false } };
    assert.equal(executionParameterValidationError(profile)?.code, 'modelValidation.executionSampling');
    profile.executionCapabilities!.samplingParameters = true;
    assert.equal(executionParameterValidationError(profile), null);
    profile.executionCapabilities!.samplingRequiresNoReasoning = true;
    assert.equal(executionParameterValidationError(profile)?.code, 'modelValidation.executionSamplingReasoning');
    profile.executionCapabilities!.reasoning = true;
    profile.reasoning = { control: 'effort', efforts: ['none', 'low'], budgets: [] };
    assert.equal(executionParameterValidationError(profile), null);
    profile.parameters.seed = 4;
    assert.equal(executionParameterValidationError(profile)?.code, 'modelValidation.protocolParameters');
});

test('reasoning drafts do not mutate stored arrays and only submit actual edits', () => {
    const original = { control: 'effort' as const, efforts: ['high'], budgets: [1024, 4096, 8192], responses: true };
    const draft = reasoningDraft(original);
    assert.deepEqual(reasoningChanges(original, draft), {});
    draft.efforts.push('xhigh');
    draft.budgets[0] = 2048;
    assert.deepEqual(original.efforts, ['high']);
    assert.equal(original.budgets[0], 1024);
    assert.deepEqual(reasoningChanges(original, draft), { reasoning: draft });
});

test('unconfigured reasoning stays inherited until explicitly edited', () => {
    assert.deepEqual(reasoningChanges(null, reasoningDraft(null)), {});
    assert.deepEqual(reasoningChanges(undefined, reasoningDraft(undefined)), {});
    const draft = reasoningDraft({ control: 'budget', budgets: [1024, 2048, 4096], efforts: [] });
    draft.control = 'automatic';
    assert.equal(reasoningChanges({ control: 'budget', budgets: [1024, 2048, 4096], efforts: [] }, draft).reasoning?.control, 'automatic');
});

test('parameter patches preserve zero and explicit seed reset without rewriting unrelated settings', () => {
    assert.deepEqual(parameterChanges({ temperature: .7, seed: 123, showThinking: true }, { temperature: 0, seed: null, showThinking: false }), { temperature: 0, seed: null });
    assert.deepEqual(parameterChanges({ topP: .9 }, { topP: .9 }), {});
});

test('endpoint/model ordering matches stored order without mutating the input', () => {
    const model = (id: string, endpoint: string, endpointOrder: number, order: number) => ({ id, order, endpoint: { id: endpoint, order: endpointOrder } } as RemoteProfile);
    const input = [model('b', 'first', 0, 2), model('c', 'last', 1, 0), model('a', 'first', 0, 1)];
    const groups = groupedProfiles(input);
    assert.deepEqual(groups.map(group => group.models.map(value => value.id)), [['a', 'b'], ['c']]);
    assert.deepEqual(input.map(value => value.id), ['b', 'c', 'a']);
});


test('native efforts retain exact defaults and do not become token budgets', () => {
    const draft = reasoningDraft({control:'effort', efforts:['low','high','max'], defaultPreference:'max'});
    assert.deepEqual(reasoningOptions(draft, 'anthropic'), ['low','high','max']);
    assert.equal(reasoningValidationError(draft, 'anthropic', 8192), null);
    assert.ok(reasoningValidationError({...draft, defaultPreference:'none'}, 'anthropic', 8192));
    assert.equal(reasoningValidationError({...draft, defaultPreference:'balanced'}, 'chat_completions', 8192), null);
});
