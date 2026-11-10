import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pageLoadFailure, observedPageLoad } from './pageLoad.ts';
test('module load failures report the exact bundled path without query secrets or arbitrary text', () => {
    assert.deepEqual(pageLoadFailure('/chat', new TypeError('Failed to fetch dynamically imported module: http://127.0.0.1:5001/ui/assets/Chat-abc.js?token=secret')), { route:'/chat',kind:'page-module-load-failed',resource:'/ui/assets/Chat-abc.js' });
    for (const cause of [new Error('secret response'), new Error('https://host.invalid/private/data?secret=1'), {message:'https://host.invalid/ui/assets/a.js'}]) {
        assert.equal(pageLoadFailure('/chat',cause).resource,null);
    }
});
test('page loader preserves rejection and never automatically reloads or replays work', async () => {
    const failure = new TypeError('network'); let calls = 0; let reports = 0;
    await assert.rejects(observedPageLoad('/chat',async () => { calls++; throw failure; },() => reports++), error => error === failure);
    assert.equal(calls,1); assert.equal(reports,1);
});
test('successful loading is untouched and diagnostic failures cannot mask original error', async () => {
    const module = {default:'component'};
    assert.equal(await observedPageLoad('/chat',async () => module,() => assert.fail('must not report success')), module);
    const failure = new Error('original');
    await assert.rejects(observedPageLoad('/chat',async () => {throw failure;},() => {throw new Error('logger');}),error => error === failure);
});

test('a stalled module presents retry instead of an endless spinner and ignores late completion', async t => {
    t.mock.timers.enable({apis:['setTimeout']});
    let finish!: (module: {default:string}) => void;
    let calls = 0; const reports: unknown[] = [];
    const loading = observedPageLoad('/chat', () => { calls++; return new Promise(resolve => {finish=resolve;}); }, value => reports.push(value));
    const rejected = assert.rejects(loading, /timed out/);
    t.mock.timers.tick(30_000); await rejected;
    finish({default:'late component'}); await Promise.resolve();
    assert.equal(calls,1);
    assert.deepEqual(reports,[{route:'/chat',kind:'page-module-load-timeout',resource:null}]);
});
