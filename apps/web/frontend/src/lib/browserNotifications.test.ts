import test from 'node:test';
import assert from 'node:assert/strict';
import { createBrowserNotifications, notificationConversationHref } from './browserNotifications.ts';
import type { ChatTurnOwner } from './chatTurn.ts';
import { createChatTurn } from './chatTurn.ts';
import { readTerminal } from './assistantTerminal.ts';
import { terminalFixture } from './assistantTerminal.fixture.ts';
import { supportedLocales, t } from './i18n.ts';

function fixture(persisted = { seen: null as string | null }) {
    const state = { supported: true, permission: 'default' as NotificationPermission, hidden: true, preference: null as string | null, requests: 0, messages: [] as string[], fail: false,
        clicks: [] as (() => void)[], opened: [] as ChatTurnOwner[] };
    const controller = createBrowserNotifications({ supported: () => state.supported, permission: () => state.permission, hidden: () => state.hidden,
        readSeen: () => persisted.seen, writeSeen: value => { persisted.seen = value; },
        read: () => state.preference, write: value => { state.preference = value; }, request: async () => { state.requests++; return state.permission = 'granted'; },
        activate: async owner => { state.opened.push(owner); },
        show: (title, body, activate) => { if (state.fail) throw new Error('OS unavailable'); state.messages.push(`${title}:${body}`); if (activate) state.clicks.push(activate); } });
    return { state, controller };
}
test('notification permission is requested only by explicit enable, never by status or tests', async () => {
    const { state, controller } = fixture();
    assert.equal(controller.snapshot().enabled, false);
    assert.throws(() => controller.test('test', 'body'));
    assert.equal(state.requests, 0);
    assert.equal(await controller.setEnabled(true), true);
    assert.equal(state.requests, 1);
    assert.equal(state.preference, 'true');
    controller.test('test', 'body');
    await controller.setEnabled(false);
    state.permission = 'denied';
    assert.equal(await controller.setEnabled(true), false);
    assert.equal(state.preference, 'false');
    assert.equal(state.requests, 1);
    state.supported = false;
    assert.equal(controller.snapshot().permission, 'unsupported');
    await assert.rejects(controller.setEnabled(true));
});
test('tab-scoped completion identities suppress duplicate alerts after reload without persisting content', async () => {
    const owner = { hostId: 'host', clientId: 'client', conversationId: 'conversation', generation: 1 };
    const turn = createChatTurn(owner, 'submission');
    const f = terminalFixture(turn);
    const evidence = await readTerminal(turn, f.reader, f.transport, () => {}, () => true);
    const persisted = { seen: null as string | null };
    const first = fixture(persisted);
    await first.controller.setEnabled(true);
    assert.equal(first.controller.completed(evidence, owner, 'private-title', 'private-body'), true);
    assert.ok(persisted.seen);
    assert.equal(persisted.seen.includes('private'), false);
    const reloaded = fixture(persisted);
    await reloaded.controller.setEnabled(true);
    assert.equal(reloaded.controller.completed(evidence, owner, 'Nola', 'View outcome'), false);
    assert.deepEqual(reloaded.state.messages, []);
    const corrupt = fixture({ seen: '{broken' });
    await corrupt.controller.setEnabled(true);
    assert.equal(corrupt.controller.completed(evidence, owner, 'Nola', 'View outcome'), true);
    assert.equal(corrupt.controller.completed(evidence, owner, 'Nola', 'View outcome'), false);
});
test('only verified terminal evidence for the exact owner can produce one background alert', async () => {
    const owner = { hostId: 'host', clientId: 'client', conversationId: 'conversation', generation: 1 };
    const turn = createChatTurn(owner, 'submission');
    const f = terminalFixture(turn);
    const evidence = await readTerminal(turn, f.reader, f.transport, () => {}, () => true);
    const { state, controller } = fixture();
    await controller.setEnabled(true);
    assert.equal(controller.completed({}, owner, 'Nola', 'View outcome'), false);
    assert.equal(controller.completed(evidence, { ...owner, hostId: 'other' }, 'Nola', 'View outcome'), false);
    assert.equal(controller.completed(evidence, owner, 'Nola', 'View outcome'), true);
    assert.equal(controller.completed(evidence, owner, 'Nola', 'View outcome'), false);
    assert.deepEqual(state.messages, ['Nola:View outcome']);
    const original = { ...owner };
    owner.conversationId = 'later-selection';
    state.clicks[0](); state.clicks[0]();
    assert.deepEqual(state.opened, [original]);
    owner.conversationId = original.conversationId;
    for (const fault of ['foreground', 'denied', 'disabled', 'throw'] as const) {
        const next = fixture(); await next.controller.setEnabled(true);
        if (fault === 'foreground') next.state.hidden = false;
        if (fault === 'denied') next.state.permission = 'denied';
        if (fault === 'disabled') await next.controller.setEnabled(false);
        if (fault === 'throw') next.state.fail = true;
        assert.equal(next.controller.completed(evidence, owner, 'Nola', 'View outcome'), false);
    }
});
test('notification destinations require an authenticated matching host and encode opaque IDs', () => {
    const owner = { hostId: 'host&fragment=#', clientId: 'client', conversationId: '//external.invalid/?q=#私密', generation: 1 };
    for (const session of [null, {}, { authenticated: false, hostId: owner.hostId }, { authenticated: true, hostId: 'other' }, { authenticated: 'true', hostId: owner.hostId }]) {
        assert.equal(notificationConversationHref(owner, session), null);
    }
    const href = notificationConversationHref(owner, { authenticated: true, hostId: owner.hostId })!;
    const url = new URL(href, 'https://nola.invalid');
    assert.equal(url.origin, 'https://nola.invalid');
    assert.equal(url.pathname, '/chat');
    assert.equal(url.searchParams.get('conversation'), owner.conversationId);
    assert.equal(url.searchParams.get('sourceHost'), owner.hostId);
    assert.equal(url.hash, '');
});
test('notification scope and permission states are localized', () => {
    for (const language of supportedLocales) for (const key of ['scope', 'enable', 'default', 'granted', 'denied', 'unsupported', 'deniedHint', 'failed', 'test', 'testTitle', 'testBody', 'completedTitle', 'completedBody']) assert.notEqual(t(language, `settings.notifications.${key}`), `settings.notifications.${key}`);
});

test('a verified blocked terminal never sends a completion-success notification',async()=>{
    const owner={hostId:'host',clientId:'client',conversationId:'conversation',generation:1};
    const turn=createChatTurn(owner,'submission'),f=terminalFixture(turn);
    Object.assign(f.seal.result,{value:{kind:'blocked',reason:{code:'context.input_budget_exceeded',detail:null},blocking_fact_refs:['budget'],resolution_condition:'Increase context'}});
    const evidence=await readTerminal(turn,f.reader,f.transport,()=>{},()=>true);
    const {controller,state}=fixture();await controller.setEnabled(true);
    assert.equal(controller.completed(evidence,owner,'Complete','Finished'),false);assert.deepEqual(state.messages,[]);
});
