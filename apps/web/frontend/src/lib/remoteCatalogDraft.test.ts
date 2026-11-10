import test from 'node:test';
import assert from 'node:assert/strict';
import { catalogDraftDirty, catalogDraftValidationError, catalogReplacement, copyRemoteCatalog, discoveredModelsForEndpoint, endpointDiscoveryMatchesSaved, stageCatalogDelete, stageCatalogMove, stageEndpoint, stageModel } from './remoteCatalogDraft.ts';
import { importExecutionCapabilities, metadataChanges, metadataDraft, type RemoteProfile } from './modelProfiles.ts';

const endpoint = { id: 'endpoint', name: 'Provider', baseURL: 'https://example.test/v1', wireProtocol: 'responses', enabled: true, order: 0 };
const model = (id: string, order: number): RemoteProfile => ({ id, provider: 'openai-compatible', displayName: id,
    modelName: id, revision: 3, enabled: true, selected: order === 0, priority: 50, order,
    endpoint, capabilities: ['chat'], reasoning: null, executionCapabilities: importExecutionCapabilities(),
    parameters: { temperature: 0.7, maxOutputTokens: 16384, topK: 40, customSystemPrompt: 'private local setting' } });

test('whole-catalog draft is isolated and serializes only owned fields', () => {
    const saved = [model('first', 0), model('second', 1)];
    const draft = stageModel(stageEndpoint(copyRemoteCatalog(saved), 'endpoint', { enabled: false }), 'first', { priority: 80 });
    assert.equal(saved[0].endpoint.enabled, true);
    assert.equal(saved[0].priority, 50);
    assert.ok(catalogDraftDirty(saved, draft));
    const request = catalogReplacement(saved, draft);
    assert.deepEqual(request.expectedRevisions, { first: 3, second: 3 });
    assert.equal(request.endpoints[0].enabled, false);
    assert.equal(request.endpoints[0].models[0].priority, 80);
    assert.deepEqual(request.endpoints[0].models[0].executionCapabilities,
        { toolCalls: true, imageInput: false, reasoning: false, samplingParameters: false, samplingRequiresNoReasoning: false,
            chatOutputTokenField: 'auto' });
    assert.deepEqual(request.endpoints[0].models[0].parameters, { temperature: 0.7, maxOutputTokens: 16384 });
    assert.ok(!JSON.stringify(request).includes('private local setting'));
    assert.ok(!catalogDraftDirty(saved, copyRemoteCatalog(saved)));
});

test('execution capability edits are isolated and survive catalog replacement', () => {
    const saved = [model('first', 0)];
    const draft = copyRemoteCatalog(saved);
    draft[0].executionCapabilities!.toolCalls = false;
    assert.equal(saved[0].executionCapabilities?.toolCalls, true);
    assert.equal(catalogDraftDirty(saved, draft), true);
    assert.equal(catalogReplacement(saved, draft).endpoints[0].models[0].executionCapabilities?.toolCalls, false);
});

test('unknown declarations remain unknown through catalog copying and require explicit confirmation', () => {
    const unknown = model('unknown', 0);
    delete unknown.executionCapabilities;
    const copied = copyRemoteCatalog([unknown]);
    assert.equal(copied[0].executionCapabilities, undefined);
    assert.equal(catalogDraftDirty([unknown], copied), false);
    assert.equal(catalogReplacement([unknown], copied).endpoints[0].models[0].executionCapabilities, null);
    assert.equal(catalogDraftValidationError(copied)?.cause?.code, 'modelValidation.executionDeclaration');
    const metadata = metadataDraft(copied[0]);
    metadata.executionCapabilities.toolCalls = true;
    assert.equal(metadata.executionCapabilitiesConfirmed, false);
    assert.deepEqual(metadataChanges(unknown, metadata), {});
    metadata.executionCapabilitiesConfirmed = true;
    const confirmed = stageModel(copied, 'unknown', metadataChanges(unknown, metadata));
    assert.equal(catalogDraftValidationError(confirmed), null);
    assert.equal(catalogReplacement([unknown], confirmed).endpoints[0].models[0].executionCapabilities?.toolCalls, true);
    assert.equal(unknown.executionCapabilities, undefined);
});

test('route identity changes retain draft values but invalidate execution authority', () => {
    const saved = [model('first', 0)];
    saved[0].executionCapabilities!.toolCalls = true;
    const renamedEndpoint = stageEndpoint(copyRemoteCatalog(saved), 'endpoint', { name: 'New display name' });
    assert.equal(renamedEndpoint[0].executionCapabilities?.toolCalls, true);
    for (const draft of [
        stageEndpoint(copyRemoteCatalog(saved), 'endpoint', { baseURL: 'https://after.test/v1' }),
        stageEndpoint(copyRemoteCatalog(saved), 'endpoint', { wireProtocol: 'anthropic' }),
        stageModel(copyRemoteCatalog(saved), 'first', { modelName: 'replacement' }),
    ]) {
        assert.equal(draft[0].executionCapabilities, null);
        assert.equal(catalogDraftValidationError(draft)?.cause?.code, 'modelValidation.executionDeclaration');
        const metadata = metadataDraft(draft[0]);
        assert.equal(metadata.executionCapabilities.toolCalls, true);
        assert.equal(metadata.executionCapabilitiesConfirmed, false);
        assert.equal(catalogReplacement(saved, draft).endpoints[0].models[0].executionCapabilities, null);
    }
    assert.equal(saved[0].executionCapabilities?.toolCalls, true);
});

test('staged ordering and deletion change the one replacement document', () => {
    const saved = [model('first', 0), model('second', 1)];
    const moved = stageCatalogMove(copyRemoteCatalog(saved), 'model', 'second', -1);
    assert.deepEqual(catalogReplacement(saved, moved).endpoints[0].models.map(model => model.id), ['second', 'first']);
    const deleted = stageCatalogDelete(moved, 'model', 'first');
    assert.deepEqual(catalogReplacement(saved, deleted).endpoints[0].models.map(model => model.id), ['second']);
    assert.equal(saved.length, 2);
});

test('whole-catalog validation rejects an incompatible staged protocol before save', () => {
    const saved = [model('first', 0)];
    const draft = stageModel(copyRemoteCatalog(saved), 'first', {
        reasoning: { control: 'effort', efforts: ['high'], budgets: [], defaultPreference: 'high' },
        executionCapabilities: { toolCalls: false, imageInput: false, reasoning: true, samplingParameters: false },
    });
    assert.equal(catalogDraftValidationError(draft), null);
    const incompatible = stageEndpoint(draft, 'endpoint', { wireProtocol: 'anthropic' });
    assert.equal(catalogDraftValidationError(incompatible)?.code, 'modelValidation.endpointReasoning');
    assert.equal(catalogDraftValidationError(stageEndpoint(draft, 'endpoint', { baseURL: 'file:///private' }))?.code,
        'modelValidation.url');
});

test('catalog rejects reasoning controls without declared model execution support', () => {
    const draft = stageModel(copyRemoteCatalog([model('first', 0)]), 'first', {
        reasoning: { control: 'effort', efforts: ['high'], budgets: [] },
    });
    assert.equal(catalogDraftValidationError(draft)?.cause?.code, 'modelValidation.executionReasoning');
});

test('discovery is unavailable until a changed route is committed', () => {
    const saved = [model('first', 0)];
    assert.equal(endpointDiscoveryMatchesSaved(saved, copyRemoteCatalog(saved), 'endpoint'), true);
    assert.equal(endpointDiscoveryMatchesSaved(saved, stageEndpoint(copyRemoteCatalog(saved), 'endpoint',
        { baseURL: 'https://different.test/v1' }), 'endpoint'), false);
    assert.equal(endpointDiscoveryMatchesSaved(saved, stageEndpoint(copyRemoteCatalog(saved), 'endpoint',
        { wireProtocol: 'anthropic' }), 'endpoint'), false);
    assert.equal(endpointDiscoveryMatchesSaved(saved, saved, 'new-endpoint'), false);
    const found = { baseURL: endpoint.baseURL, wireProtocol: endpoint.wireProtocol, models: ['discovered'] };
    assert.deepEqual(discoveredModelsForEndpoint(found, endpoint), ['discovered']);
    assert.deepEqual(discoveredModelsForEndpoint(found, { ...endpoint, baseURL: 'https://different.test/v1' }), []);
    assert.deepEqual(discoveredModelsForEndpoint(found, { ...endpoint, wireProtocol: 'anthropic' }), []);
});
