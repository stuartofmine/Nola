// SPDX-License-Identifier: Apache-2.0
// Run with: node web/tests/agent_trace_journal_recovery_test.js

'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

global.window = global;
const runtimePath = path.resolve(
    __dirname,
    '../app/static/js/agent-trace-runtime.js',
);
vm.runInThisContext(fs.readFileSync(runtimePath, 'utf8'), { filename: runtimePath });

const store = global.NolaAgentTrace.createTraceStore();
const identity = {
    schema_version: 2,
    trace_id: 'trace-1',
    run_id: 'turn-1',
    conversation_id: 'conversation-1',
    message_id: 'assistant-1',
    turn_id: 'turn-1',
};

function trace(sequence, kind, payload = {}, status = 'running') {
    return {
        type: 'agent_trace',
        trace: {
            ...identity,
            event_id: `event-${sequence}`,
            sequence,
            kind,
            status,
            payload,
        },
    };
}

const earlyActivationStore = global.NolaAgentTrace.createTraceStore();
const earlyActivation = {
    type: 'core_capabilities_activated',
    session_id: identity.conversation_id,
    turn_id: identity.turn_id,
    cursor: 9,
    source: 'skill-runtime',
    capability_ids: ['documents.create', 'files.write'],
};
assert.equal(
    earlyActivationStore.ingest(earlyActivation).ignored,
    'buffered_capability_activation',
);
earlyActivationStore.ingest(trace(1, 'run.started', { phase: 'processing' }));
let earlyProjection = earlyActivationStore.projection(identity.run_id, identity.message_id);
assert.equal(earlyProjection.tools.length, 1);
assert.equal(earlyProjection.tools[0].callId, 'core-capability:turn-1:skill-runtime');
assert.equal(earlyProjection.tools[0].resultText, 'documents.create\nfiles.write');
assert.deepEqual(earlyProjection.tools[0].capabilityIds, ['documents.create', 'files.write']);

earlyActivationStore.ingest({
    ...earlyActivation,
    cursor: 8,
    capability_ids: ['stale.capability'],
});
earlyProjection = earlyActivationStore.projection(identity.run_id, identity.message_id);
assert.equal(earlyProjection.tools.length, 1);
assert.equal(earlyProjection.tools[0].resultText, 'documents.create\nfiles.write');

earlyActivationStore.ingest({
    ...earlyActivation,
    cursor: 10,
    capability_ids: ['documents.create', 'files.write', 'documents.validate'],
});
earlyProjection = earlyActivationStore.projection(identity.run_id, identity.message_id);
assert.equal(earlyProjection.tools.length, 1);
assert.equal(earlyProjection.tools[0].resultText, 'documents.create\nfiles.write\ndocuments.validate');
assert.deepEqual(earlyProjection.tools[0].capabilityIds, [
    'documents.create', 'files.write', 'documents.validate',
]);

store.ingest(trace(1, 'run.started', { phase: 'processing' }));
store.ingest(trace(2, 'assistant.delta', { delta: 'ignored' }));
store.ingest({
    type: 'core_part_updated',
    protocol_version: 2,
    session_id: identity.conversation_id,
    conversation_id: identity.conversation_id,
    turn_id: identity.turn_id,
    run_id: identity.run_id,
    message_id: identity.message_id,
    cursor: 7,
    projection_revision: 7,
    part_id: 'text-1',
    update: { type: 'text_appended', part_id: 'text-1' },
    part: { id: 'text-1', kind: 'text', text: 'Hello wor', is_complete: true },
});
assert.equal(store.projection(identity.run_id, identity.message_id).assistantText, 'Hello wor');

// Legacy body deltas are rejected before entering the canonical trace ledger.
// They must neither consume a sequence nor poison a later canonical event ID.
assert.equal(store.ingest(trace(2, 'assistant.delta', { delta: 'ignored' })).ignored, 'non_canonical_event');
store.ingest(trace(3, 'assistant.delta', { delta: 'world' }));
store.ingest(trace(2, 'tool.started', {
    call_id: 'call-1',
    tool_name: 'web_search',
    arguments: { query: 'Nola' },
}));
assert.equal(store.ingest(trace(2, 'tool.started', {})).ignored, 'duplicate_event_id');
store.ingest(trace(3, 'tool.completed', {
    call_id: 'call-1',
    tool_name: 'web_search',
    result: { text: 'one result' },
}, 'succeeded'));
store.ingest({
    type: 'core_part_updated',
    protocol_version: 2,
    session_id: identity.conversation_id,
    conversation_id: identity.conversation_id,
    turn_id: identity.turn_id,
    run_id: identity.run_id,
    message_id: identity.message_id,
    cursor: 8,
    projection_revision: 8,
    part_id: 'tool-1',
    update: { type: 'created', part_id: 'tool-1' },
    part: {
        id: 'tool-1', kind: 'tool', call_id: 'call-1', capability_id: 'web_search',
        state: 'completed', output_preview: 'one result',
    },
});
store.ingest(trace(4, 'run.completed', {}, 'succeeded'));

// Journal replay can repair progress, but only a later canonical Part may
// advance the visible document.
store.ingest({
    type: 'core_part_updated',
    protocol_version: 2,
    session_id: identity.conversation_id,
    conversation_id: identity.conversation_id,
    turn_id: identity.turn_id,
    run_id: identity.run_id,
    message_id: identity.message_id,
    cursor: 9,
    projection_revision: 9,
    part_id: 'text-1',
    update: { type: 'text_appended', part_id: 'text-1' },
    part: { id: 'text-1', kind: 'text', text: 'Hello world', is_complete: true },
});

const recovered = store.projection(identity.run_id, identity.message_id);
assert.equal(recovered.assistantText, 'Hello world');
assert.equal(recovered.tools.length, 1);
assert.equal(recovered.tools[0].status, 'succeeded');
assert.equal(recovered.terminal, true);
assert.equal(recovered.terminalKind, 'run.completed');
