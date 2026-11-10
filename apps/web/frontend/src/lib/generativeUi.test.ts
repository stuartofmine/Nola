import test from 'node:test';
import assert from 'node:assert/strict';
import { uiSurfaceReference, uiIdentityKey, validateUiActionPart } from './generativeUi.ts';
import { parseCanonicalProjection, timelinePartText } from './chatTimeline.ts';

const identity = {
    owner: { authority_id: 'host', conversation_id: 'conversation', participant_id: 'agent',
        context_id: 'context', originating_run_id: 'run', message_id: 'message', part_id: 'tool-part' },
    surface_id: 'form', generation: 1,
};
const part = () => ({kind: 'ui_surface', part_id: 'ui-surface:hash', revision: 90,
    surface: {identity: structuredClone(identity), revision: 2}, summary: 'Form'});

test('UI action shows only the Core summary and keeps original Surface ownership', () => {
    const value = {kind: 'ui_action', part_id: 'new-user-action', revision: 1,
        surface: part().surface, envelope: {private_value: 'never display this envelope'}, summary: 'Submitted form'};
    validateUiActionPart(value, 'conversation');
    assert.throws(() => validateUiActionPart(value, 'foreign-conversation'));
    assert.throws(() => validateUiActionPart({...value, summary: ''}, 'conversation'));
    assert.throws(() => validateUiActionPart({...value, envelope: 'not structured'}, 'conversation'));
    assert.equal(timelinePartText({source: 'canonical', id: 'action', value}), 'Submitted form');
    assert.equal(timelinePartText({source: 'canonical', id: 'action', value}, 'reasoning'), '');
    const historical = {source: 'canonical' as const, id: 'historical-action', value: {
        kind: 'historical', part_id: 'new-user-action', revision: 1, projection: {
            schema: 'message_part_view', canonical_kind: 'ui_action', status: {state: 'supported'},
            fields: {summary: {availability: 'complete', value: 'Submitted form'}},
            canonical_text: {text: JSON.stringify(value.envelope)},
        },
    }};
    assert.equal(timelinePartText(historical), 'Submitted form');
    historical.value.projection.fields.summary.availability = 'not_loaded';
    assert.equal(timelinePartText(historical), '', 'Missing summary never falls back to action JSON');
});

test('canonical action admission preserves envelope and fences conversation ownership', () => {
    const action = {kind: 'ui_action', part_id: 'submitted-action', revision: 1,
        surface: part().surface, envelope: {action: {name: 'submit', context: {name: 'private input'}}},
        summary: 'Submitted form'};
    const projection = {contract_version: 1, conversation_id: 'conversation', revision: 2,
        messages: [{message_id: 'new-user-message', role: 'user', state: 'completed', revision: 1,
            created_at_ms: 1, parts: [action]}]};
    const parsed = parseCanonicalProjection(JSON.parse(JSON.stringify(projection)));
    assert.deepEqual(parsed.messages[0].parts[0].envelope, action.envelope);
    assert.equal(timelinePartText({source: 'canonical', id: action.part_id, value: parsed.messages[0].parts[0]}), action.summary);
    assert.throws(() => parseCanonicalProjection({...projection, conversation_id: 'other'}));
    for (const envelope of [null, undefined, [], 'raw event']) {
        assert.throws(() => parseCanonicalProjection({...projection, messages: [
            {...projection.messages[0], parts: [{...action, envelope}]},
        ]}));
    }
});

test('surface Part retains its independent exact revision and tool provenance', () => {
    const reference = uiSurfaceReference(part(), 'conversation', 'message');
    assert.equal(reference?.revision, 2);
    assert.equal(reference?.identity.owner.part_id, 'tool-part');
    assert.equal(uiIdentityKey(reference!.identity), uiIdentityKey(identity));
});

test('surface Part rejects cross-message or cross-conversation ownership', () => {
    assert.throws(() => uiSurfaceReference(part(), 'other', 'message'));
    assert.throws(() => uiSurfaceReference(part(), 'conversation', 'other'));
    assert.equal(uiSurfaceReference({kind: 'tool'}, 'conversation', 'message'), null);
});

test('surface Part rejects invalid revisions and missing owner fields', () => {
    for (const revision of [0, -1, 0.5, Number.MAX_SAFE_INTEGER + 1]) {
        const value = part(); value.surface.revision = revision;
        assert.throws(() => uiSurfaceReference(value, 'conversation', 'message'));
    }
    const value = part(); value.surface.identity.owner.participant_id = '';
    assert.throws(() => uiSurfaceReference(value, 'conversation', 'message'));
});

test('UI delta requires exact owner, base and catalog and preserves unrelated node identities', async () => {
    const {applyUiSurfaceUpdate} = await import('./generativeUi.ts');
    const node = {instance_id: 'root', component_id: 'root', component: 'Text', props: {text: 'before'}, slots: {}, bindings: {}, actions: {}, validation_errors: []};
    const baseline = {identity, revision: 1, catalog_id: 'catalog', catalog_digest: 'digest', phase: 'ready' as const, root_instance_id: 'root', nodes: {root: node}, summary: 'before'};
    const changes = {identity, base_revision: 1, revision: 2, catalog_digest: 'digest', phase: 'ready' as const,
        root_instance_id: 'root', upserted_nodes: [], removed_instance_ids: [], summary: 'after'};
    const updated = applyUiSurfaceUpdate(identity, baseline, {type: 'changes', changes});
    assert.equal(updated?.nodes.root, node);
    assert.equal(updated?.revision, 2);
    assert.equal(baseline.revision, 1);
    assert.throws(() => applyUiSurfaceUpdate(identity, baseline, {type: 'changes', changes: {...changes, base_revision: 0}}));
    assert.throws(() => applyUiSurfaceUpdate(identity, baseline, {type: 'changes', changes: {...changes, catalog_digest: 'other'}}));
    assert.equal(applyUiSurfaceUpdate(identity, baseline, {type: 'missing', identity}), null);
});

test('historical surface anchors require supported canonical complete fields', () => {
    const projection = {schema: 'message_part_view', canonical_kind: 'ui_surface', binding: 'canonical',
        owner: {conversation_id: 'conversation', message_id: 'message', part_id: 'history'},
        status: {state: 'supported'}, fields: {surface: {availability: 'complete', value: part().surface},
            summary: {availability: 'complete', value: 'History'}}};
    const value = {kind: 'historical', part_id: 'history', projection};
    assert.deepEqual(uiSurfaceReference(value, 'conversation', 'message'), part().surface);
    assert.equal(uiSurfaceReference({...value, projection: {...projection, binding: 'unbound'}}, 'conversation', 'message'), null);
    assert.equal(uiSurfaceReference({...value, projection: {...projection, status: {state: 'deferred'}}}, 'conversation', 'message'), null);
    assert.throws(() => uiSurfaceReference(value, 'other', 'message'));
});

test('draft preview accepts only exact owner revision and draft generation', async () => {
    const {validateUiDraftPreview} = await import('./generativeUi.ts');
    const input = {identity, expected_surface_revision: 4, draft_generation: 7, edits: []};
    const snapshot = {identity, revision:4, phase:'ready' as const, catalog_id:'catalog',catalog_digest:'digest',root_instance_id:null,nodes:{},summary:''};
    const preview = {identity,base_revision:4,draft_generation:7,snapshot};
    assert.equal(validateUiDraftPreview(input,preview),snapshot);
    for (const invalid of [
        {...preview,draft_generation:6}, {...preview,base_revision:3},
        {...preview,snapshot:{...snapshot,revision:5}},
        {...preview,identity:{...identity,generation:2}},
        {...preview,snapshot:{...snapshot,identity:{...identity,generation:2}}},
        {...preview,snapshot:{...snapshot,phase:'read_only' as const}},
    ]) assert.throws(()=>validateUiDraftPreview(input,invalid));
});
