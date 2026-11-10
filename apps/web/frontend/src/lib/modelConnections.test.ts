import test from 'node:test';
import assert from 'node:assert/strict';
import { modelSelectionCommand, modelProbeConfirmation, type ConnectionSnapshot, type ConnectionInventory } from './modelConnections.ts';
test('probe confirmation requires explicit charges and exact current account route', () => {
    const model = {model_id:'same-name', display_name:'Fixture', protocol:'responses', context_window:null, modalities:[], reasoning_levels:[]};
    const snapshot: ConnectionSnapshot = {revision:9, probe_credential_bindings:{responses:'opaque-current'},
        document:{format_version:1, connection:{id:'connection', display_name:'Fixture', base_url:'https://fixture.invalid/v1',
            protocols:['responses'], source:'manual', source_id:null, account_id:'account', enabled:true},
            catalog:{observed_at_ms:1, models:[model]}}};
    assert.throws(() => modelProbeConfirmation(snapshot, model, 4, 'request', false));
    assert.throws(() => modelProbeConfirmation(snapshot, model, null, 'request', true));
    assert.throws(() => modelProbeConfirmation(snapshot, {...model,protocol:'anthropic'}, 4, 'request', true));
    const confirmation = modelProbeConfirmation(snapshot, model, 4, 'request', true);
    assert.equal(confirmation.max_requests, 1);
    assert.equal(confirmation.account_revision, 4);
    assert.equal(confirmation.credential_binding, 'opaque-current');
    assert.equal(confirmation.expected_revision, 9);
    assert.equal(confirmation.acknowledge_possible_charges, true);
});
test('model toggle preserves other protocol selections and current model', () => {
    const connection = {revision: 7, document: {connection: {id: 'connection'}}} as ConnectionSnapshot;
    const inventory = {selections: [
        {connection_id: 'connection', catalog_key: 'responses:one', enabled: true, selected: true},
        {connection_id: 'connection', catalog_key: 'chat_completions:manual', enabled: true, selected: false},
        {connection_id: 'other', catalog_key: 'responses:other', enabled: true, selected: true},
    ]} as ConnectionInventory;
    const result = modelSelectionCommand(connection, inventory, 'responses:two', true);
    assert.deepEqual(result.enabled_model_ids, ['responses:one', 'chat_completions:manual', 'responses:two']);
    assert.equal(result.selected_model_id, 'responses:one');
    assert.equal(result.expected_revision, 7);
    const disabled = modelSelectionCommand(connection, inventory, 'responses:one', false);
    assert.equal(disabled.selected_model_id, null);
    assert.deepEqual(disabled.enabled_model_ids, ['chat_completions:manual']);
});
