import test from 'node:test';
import assert from 'node:assert/strict';
import { parseToolExecutionSource } from './toolExecutionSource.ts';

const cell = {
    host_id: 'host', conversation_id: 'conversation', turn_id: 'turn',
    parent_call_id: 'parent', cell_id: 'cell', generation: 'generation',
};

test('execution provenance preserves every owner field without granting authority', () => {
    assert.deepEqual(parseToolExecutionSource({kind: 'model'}), {kind: 'model'});
    const source = {kind: 'code_cell', cell};
    assert.deepEqual(parseToolExecutionSource(source), source);
    assert.notEqual(parseToolExecutionSource(source), source);
});

test('missing, unknown and ambiguous execution provenance is rejected', () => {
    for (const value of [undefined, null, {}, [], {kind: 'unknown'},
        {kind: 'model', cell: null}, {kind: 'model', permission: 'allow'},
        {kind: 'code_cell'}, {kind: 'code_cell', cell, permission: 'allow'},
        {kind: 'code_cell', cell: {...cell, extra: 'alias'}},
    ]) assert.throws(() => parseToolExecutionSource(value));
});

test('all six identity components are required and bounded by UTF-8 bytes', () => {
    for (const key of Object.keys(cell)) {
        const missing: Record<string, unknown> = {...cell};
        delete missing[key];
        assert.throws(() => parseToolExecutionSource({kind: 'code_cell', cell: missing}));
        for (const invalid of ['', '   ', 'owner\n', '\u0085', '界'.repeat(171), 1, null]) {
            assert.throws(() => parseToolExecutionSource({kind: 'code_cell', cell: {...cell, [key]: invalid}}));
        }
        assert.doesNotThrow(() => parseToolExecutionSource({kind: 'code_cell', cell: {...cell, [key]: 'a'.repeat(512)}}));
    }
});
