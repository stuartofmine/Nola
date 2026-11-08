import test from 'node:test';
import assert from 'node:assert/strict';
import { emptyContentTabs, openPdfContentTab, closeContentTab, selectContentTab, conversationTabKey, pdfTabKey, openPageContentTab, managementContentPath, updatePageContentLocation } from './conversationContentTabs.ts';
import type { ConversationPdfResource } from './chatResources.ts';
import { createWorkspaceDrafts } from './workspaceDrafts.ts';
const pdf = (id: string, conversationId = 'A'): ConversationPdfResource => ({resourceId: id, conversationId,
    displayName: `${id}.pdf`, mimeType: 'application/pdf', byteCount: 20, sha256: 'a'.repeat(64)});

test('content objects deduplicate by type and ID, preserve order and cannot close conversation', () => {
    const initial = emptyContentTabs();
    let state = openPdfContentTab(initial, 'A', pdf('one'));
    state = openPdfContentTab(state, 'A', pdf('two'));
    state = openPdfContentTab(state, 'A', {...pdf('one'), displayName: 'renamed.pdf'});
    assert.equal(state.items.length, 2);
    assert.equal(state.selected, pdfTabKey('one'));
    assert.deepEqual(state.items.map(item => item.kind === 'pdf' ? item.resource.resourceId : null), ['one', 'two']);
    assert.equal(state.items[0].kind === 'pdf' && state.items[0].resource.displayName, 'renamed.pdf');
    assert.equal(closeContentTab(state, conversationTabKey), state);
    assert.equal(selectContentTab(state, 'foreign'), state);
    assert.deepEqual(initial, emptyContentTabs());
});
test('closing selected objects returns left, background close keeps selected content', () => {
    let state = openPdfContentTab(openPdfContentTab(emptyContentTabs(), 'A', pdf('one')), 'A', pdf('two'));
    assert.equal(closeContentTab(state, pdfTabKey('two')).selected, pdfTabKey('one'));
    assert.equal(closeContentTab(state, pdfTabKey('one')).selected, pdfTabKey('two'));
    state = closeContentTab(closeContentTab(state, pdfTabKey('two')), pdfTabKey('one'));
    assert.equal(state.selected, conversationTabKey);
    assert.deepEqual(state.items, []);
});
test('Host changes, conversation removal and foreign resources cannot transfer content or drafts', () => {
    const workspace = createWorkspaceDrafts(null);
    const host = workspace.forHost('host-1');
    const state = openPdfContentTab(emptyContentTabs(), 'A', pdf('one'));
    host.contentTabs.set('A', state);
    host.contentTabs.set('B', emptyContentTabs());
    host.text.write('A', 'draft-A'); host.text.write('B', 'draft-B');
    assert.throws(() => openPdfContentTab(state, 'A', pdf('foreign', 'B')));
    assert.equal(selectContentTab(host.contentTabs.get('A')!, conversationTabKey).items.length, 1);
    assert.equal(host.contentTabs.get('B')!.items.length, 0);
    assert.equal(host.text.read('A'), 'draft-A');
    host.forget('A');
    assert.equal(host.contentTabs.has('A'), false);
    assert.equal(host.text.read('B'), 'draft-B');
    const successor = workspace.forHost('host-2');
    assert.equal(successor.contentTabs.size, 0);
    assert.equal(host.contentTabs.size, 0);
    assert.equal(successor.text.read('B'), '');
});


test('management tabs retain their own location and reject external or unsupported navigation', () => {
    const origin = 'http://127.0.0.1:5001';
    for (const href of ['/chat', '/login', 'https://other.example/models', 'file:///models', 'http://[bad']) {
        assert.equal(managementContentPath(href, origin), null);
    }
    const target = managementContentPath('/settings?section=agent', origin)!;
    let state = openPageContentTab(emptyContentTabs(), target.path, target.titleKey);
    const key = state.selected;
    assert.equal(openPageContentTab(state, target.path, target.titleKey).items.length, 1);
    state = updatePageContentLocation(state, key, '/settings?section=diagnostics');
    assert.equal(state.selected, key);
    assert.equal(state.items[0].kind === 'page' && state.items[0].path, '/settings?section=diagnostics');
    assert.equal(openPageContentTab(state, '/settings?section=diagnostics', target.titleKey).items.length, 1);
    assert.equal(closeContentTab(state, key).selected, conversationTabKey);
});

test('reopening an editor original address after section changes preserves one stable object and its reading position', () => {
    let state = openPageContentTab(emptyContentTabs(), '/settings?section=agent', 'shell.settings');
    const originalKey = state.selected;
    const tab = state.items[0];
    assert.equal(tab.kind, 'page');
    if (tab.kind !== 'page') throw new Error('page expected');
    tab.viewport.top = 640;
    state = updatePageContentLocation(state, originalKey, '/settings?section=diagnostics');
    state = openPageContentTab(state, '/settings?section=agent', 'shell.settings');
    assert.equal(state.items.length, 1);
    assert.equal(state.selected, originalKey);
    assert.equal(state.items[0].kind === 'page' && state.items[0].path, '/settings?section=diagnostics');
    assert.equal(state.items[0].kind === 'page' && state.items[0].viewport.top, 640);
});
