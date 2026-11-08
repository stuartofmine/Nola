import { test } from 'node:test';
import assert from 'node:assert/strict';
import { latestRequest } from './latestRequest.ts';

function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (error: Error) => void;
    const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
    return { promise, resolve, reject };
}

test('out-of-order catalog responses cannot overwrite the latest read', async () => {
    const reads = latestRequest<string>();
    const old = deferred<string>();
    let signal!: AbortSignal;
    const pending = reads.run(value => { signal = value; return old.promise; });
    assert.equal(await reads.run(async () => 'new'), 'new');
    assert.equal(signal.aborted, true);
    old.resolve('old');
    assert.equal(await pending, undefined, 'Transport ignoring cancellation must still be fenced');
});

test('cancel at a write boundary or teardown suppresses late results and errors', async () => {
    const reads = latestRequest<string>();
    for (const fail of [false, true]) {
        const old = deferred<string>();
        const pending = reads.run(() => old.promise);
        reads.cancel();
        if (fail) old.reject(new Error('obsolete failure'));
        else old.resolve('obsolete snapshot');
        assert.equal(await pending, undefined);
    }
    assert.equal(await reads.run(async () => 'reopened'), 'reopened');
});

test('current failures remain actionable and a retry can succeed', async () => {
    const reads = latestRequest<string>();
    await assert.rejects(reads.run(async () => { throw new Error('offline'); }), /offline/);
    assert.equal(await reads.run(async () => 'online'), 'online');
});
