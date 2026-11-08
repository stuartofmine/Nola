import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { applyUiSurfaceUpdate, uiIdentityKey, uiSurfaceReference } from './generativeUi.ts';

// Generated and independently revalidated by Rust Core. Clients consume only
// resolved snapshots and changes; wire_input is never parsed by this client.
const fixture = JSON.parse(readFileSync(new URL('../../../../tests/fixtures/a2ui/form_interaction_v0_9_1.json', import.meta.url), 'utf8'));

test('shared Core golden projection applies the exact interaction delta', () => {
    assert.equal(fixture.fixture_version, 1);
    const initial = structuredClone(fixture.initial_snapshot);
    const before = JSON.stringify(initial);
    const updated = applyUiSurfaceUpdate(fixture.identity, initial, {type: 'changes', changes: fixture.changes});
    assert.deepEqual(updated, fixture.edited_snapshot);
    assert.equal(JSON.stringify(initial), before, 'applying a delta cannot mutate the previous projection');
    const changed = new Set(fixture.changes.upserted_nodes.map((node: {instance_id: string}) => node.instance_id));
    for (const id of Object.keys(initial.nodes)) {
        if (!changed.has(id) && updated?.nodes[id]) assert.equal(updated.nodes[id], initial.nodes[id]);
    }
    assert.equal(fixture.interaction.expected_surface_revision, initial.revision);
    assert.equal(uiIdentityKey(fixture.interaction.identity), uiIdentityKey(fixture.identity));
});

test('shared golden owner fences every identity dimension and stale delta', () => {
    for (const key of Object.keys(fixture.identity.owner)) {
        const foreign = structuredClone(fixture.identity);
        foreign.owner[key] += '-foreign';
        assert.throws(() => applyUiSurfaceUpdate(foreign, fixture.initial_snapshot, {type: 'changes', changes: fixture.changes}));
    }
    for (const key of ['surface_id', 'generation']) {
        const foreign = structuredClone(fixture.identity);
        foreign[key] = key === 'generation' ? foreign[key] + 1 : `${foreign[key]}-foreign`;
        assert.throws(() => applyUiSurfaceUpdate(foreign, fixture.initial_snapshot, {type: 'changes', changes: fixture.changes}));
    }
    assert.throws(() => applyUiSurfaceUpdate(fixture.identity, fixture.edited_snapshot, {type: 'changes', changes: fixture.changes}));
});

test('shared golden history is read only and message reference preserves provenance', () => {
    const historical = fixture.historical_snapshot;
    assert.equal(historical.phase, 'read_only');
    assert.equal(uiIdentityKey(historical.identity), uiIdentityKey(fixture.identity));
    for (const node of Object.values(historical.nodes) as Array<{actions: object; bindings: object}>) {
        assert.deepEqual(node.actions, {});
        assert.deepEqual(node.bindings, {});
    }
    const surface = {identity: fixture.identity, revision: fixture.initial_snapshot.revision};
    assert.deepEqual(uiSurfaceReference({kind: 'ui_surface', surface},
        fixture.identity.owner.conversation_id, fixture.identity.owner.message_id), surface);
});
