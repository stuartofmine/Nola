import test from 'node:test';
import assert from 'node:assert/strict';
import { createWorkspaceCompletions } from './workspaceCompletions.ts';
import { createChatTurn, reduceChatTurn } from './chatTurn.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';
import { isVerifiedTerminal } from './assistantTerminal.ts';

const owner = { hostId: 'host', clientId: 'client', conversationId: 'conversation', generation: 1 };
function fixture() {
    const initial = createChatTurn(owner, 'submission');
    const turn = reduceChatTurn(initial, { type: 'run_bound', owner, runId: 'run' }).state;
    return { initial, turn, ...terminalFixture(turn) };
}

test('workspace completion requires an admitted Run and a complete verified terminal, deduplicated across page generations', async () => {
    const f = fixture(); const notifications: unknown[] = [];
    const observer = createWorkspaceCompletions({ journal: f.transport, completed: proof => notifications.push(proof) });
    await observer.watch(f.initial);
    assert.equal(f.requests.length, 0);
    const first = observer.watch(f.turn);
    const second = observer.watch({ ...f.turn, owner: { ...owner, generation: 2 } });
    assert.equal(first, second);
    await first;
    assert.equal(notifications.length, 1);
    assert.equal(isVerifiedTerminal(notifications[0]), true);
    assert.ok(f.requests.some(request => request.operation === 'terminal_manifest'));
    const count = f.requests.length;
    await observer.watch(f.turn);
    assert.equal(f.requests.length, count);
});

test('missing, failed or foreign evidence cannot notify; temporary failures recover without dispatching', async () => {
    const f = fixture(); let attempts = 0; let notifications = 0;
    const delays: number[] = [];
    const observer = createWorkspaceCompletions({
        journal: async (body, signal) => {
            assert.equal(signal.aborted, false);
            if (body.operation === 'terminal_seal') {
                attempts++;
                if (attempts === 1) return { source_host_id: 'host', result: null };
                if (attempts === 2) throw new Error('offline');
                if (attempts === 3) return { source_host_id: 'other-host', result: null };
            }
            return f.transport(body);
        },
        completed: () => { notifications++; },
        wait: async delay => { assert.equal(notifications, 0); delays.push(delay); },
    });
    await observer.watch(f.turn);
    assert.equal(attempts, 4);
    assert.deepEqual(delays, [5000, 5000, 10000]);
    assert.equal(notifications, 1);
});

test('clearing the workspace fences late reads and forget aborts only the selected conversation', async () => {
    const f = fixture(); const releases: (() => void)[] = []; const signals: AbortSignal[] = [];
    let notifications = 0;
    const observer = createWorkspaceCompletions({
        journal: (body, signal) => new Promise(resolve => {
            signals.push(signal); releases.push(() => resolve({ source_host_id: 'host', result: null }));
        }), completed: () => { notifications++; },
        wait: async () => assert.fail('aborted reader must not schedule another poll'),
    });
    const first = observer.watch(f.turn);
    const second = observer.watch({ ...f.turn, owner: { ...owner, conversationId: 'other' } });
    observer.forget('conversation');
    assert.equal(signals[0].aborted, true);
    assert.equal(signals[1].aborted, false);
    observer.clear();
    assert.equal(signals[1].aborted, true);
    releases.forEach(release => release());
    await Promise.all([first, second]);
    assert.equal(notifications, 0);
});

test('an invalid manifest never becomes a notification and cancellation releases its wait', async () => {
    const f = fixture(); let waited = false;
    const observer = createWorkspaceCompletions({
        journal: async body => body.operation === 'terminal_manifest' ? { source_host_id: 'host', result: {} } : f.transport(body),
        completed: () => assert.fail('partial evidence notified'),
        wait: async () => { waited = true; observer.clear(); },
    });
    await observer.watch(f.turn);
    assert.equal(waited, true);
});
