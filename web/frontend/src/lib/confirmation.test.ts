import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ConfirmationGate } from './confirmation.ts';

test('overlapping confirmations never share an approval', async () => {
    const gate = new ConfirmationGate();
    const first = gate.request();
    assert.equal(gate.pending, true);
    assert.equal(gate.request(), undefined);
    gate.settle(true);
    assert.equal(await first, true);
    assert.equal(gate.pending, false);
    const second = gate.request();
    gate.settle(false);
    assert.equal(await second, false);
});

test('cancellation settles once and duplicate settlement cannot approve the action', async () => {
    const gate = new ConfirmationGate();
    const decision = gate.request();
    gate.settle(false);
    gate.settle(true);
    assert.equal(await decision, false);
    assert.equal(gate.pending, false);
});
