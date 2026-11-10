import test from 'node:test';
import assert from 'node:assert/strict';
import {knowledgeDocument, knowledgeDirty, knowledgePending, knowledgePatch, knowledgeUrl, reconcileKnowledge, resolveKnowledge} from './chatKnowledge.ts';
import {createWorkspaceDrafts} from './workspaceDrafts.ts';
import {chatKnowledgeMessages} from './locales/chatKnowledge.ts';
import {createServer} from 'vite';

test('knowledge reads require the exact conversation and both document fields', () => {
    assert.deepEqual(knowledgeDocument({conversationId: 'one', configurationRevision: 2, knowledge: {keyPointsMarkdown: '', wikiMarkdown: '<script>source</script>', updatedAt: null}}, 'one'), {revision: 2, document: {knowledgePoints: '', wiki: '<script>source</script>'}});
    for (const value of [null, {}, {conversationId: 'other', knowledgePoints: '', wiki: ''}, {conversationId: 'one', knowledgePoints: ''}]) {
        assert.throws(() => knowledgeDocument(value, 'one'));
    }
    assert.equal(knowledgeUrl('a/b?'), '/api/chat/conversations/a%2Fb%3F/configuration');
    assert.deepEqual(knowledgeDocument({conversationId: 'one', configurationRevision: 1,
        knowledge: {keyPointsMarkdown: '', wikiMarkdown: ''}}, 'one').document, {knowledgePoints: '', wiki: ''});
    for (const updatedAt of ['2026-01-01', true, {}, NaN, Infinity]) {
        assert.throws(() => knowledgeDocument({conversationId: 'one', configurationRevision: 1,
            knowledge: {keyPointsMarkdown: '', wikiMarkdown: '', updatedAt}}, 'one'));
    }
    for (const configurationRevision of [0, -1, 1.5, '1', Number.MAX_SAFE_INTEGER + 1]) {
        assert.throws(() => knowledgeDocument({conversationId: 'one', configurationRevision, knowledge: null}, 'one'));
    }
});
test('knowledge saves only changed fields, including explicit clearing; drafts never cross hosts', () => {
    const draft = {revision: 1, baseline: {knowledgePoints: 'old', wiki: 'keep'}, value: {knowledgePoints: '', wiki: 'keep'}};
    assert.equal(knowledgeDirty(draft), true);
    assert.deepEqual(knowledgePatch(draft), {expectedRevision: 1, knowledge: {keyPointsMarkdown: ''}});
    const workspace = createWorkspaceDrafts(); const host = workspace.forHost('host');
    host.knowledge.set('one', draft);
    assert.equal(workspace.hasDrafts, true);
    assert.equal(workspace.forHost('host').knowledge.get('one'), draft);
    workspace.forgetConversation('one');
    assert.equal(workspace.hasDrafts, false);
    host.knowledge.set('one', draft);
    workspace.forHost('other');
    assert.equal(host.knowledge.size, 0);
    assert.equal(workspace.hasDrafts, false);
});
test('reload reconciles lost receipts and unrelated edits but never silently resolves conflicting edits', () => {
    const draft = {revision: 1, baseline: {knowledgePoints: 'old', wiki: 'wiki'}, value: {knowledgePoints: '  local\n', wiki: 'wiki'}, uncertain: true};
    const merged = reconcileKnowledge(draft, {revision: 2, document: {knowledgePoints: 'old', wiki: 'remote wiki'}});
    assert.throws(() => reconcileKnowledge(merged, {revision: 1, document: {...draft.baseline}}));
    assert.equal(merged.uncertain, undefined);
    assert.deepEqual(knowledgePatch(merged), {expectedRevision: 2, knowledge: {keyPointsMarkdown: '  local\n'}});
    const committed = reconcileKnowledge(draft, {revision: 2, document: {...draft.value}});
    assert.equal(knowledgeDirty(committed), false);
    const remote = {revision: 3, document: {knowledgePoints: 'remote', wiki: 'remote wiki'}};
    const conflict = reconcileKnowledge(draft, remote);
    assert.equal(conflict.conflict, true);
    assert.equal(reconcileKnowledge(conflict, remote).conflict, true);
    assert.deepEqual(conflict.value, draft.value);
    assert.equal(knowledgeDirty(resolveKnowledge(conflict, 'remote')), false);
    assert.deepEqual(knowledgePatch(resolveKnowledge(conflict, 'local')), {expectedRevision: 3, knowledge: {keyPointsMarkdown: '  local\n'}});
});
test('knowledge labels cover every supported locale', () => {
    const keys = Object.keys(chatKnowledgeMessages.en).sort();
    assert.equal(Object.keys(chatKnowledgeMessages).length, 7);
    for (const catalog of Object.values(chatKnowledgeMessages)) {
        assert.deepEqual(Object.keys(catalog).sort(), keys);
        assert.ok(Object.values(catalog).every(value => value.trim()));
    }
});
test('unsent edits and unconfirmed writes keep the send fence and workspace guard', () => {
    const clean = {revision: 1, value: {knowledgePoints: '', wiki: ''}, baseline: {knowledgePoints: '', wiki: ''}};
    assert.equal(knowledgePending(undefined), false);
    assert.equal(knowledgePending(clean), false);
    const workspace = createWorkspaceDrafts();
    const host = workspace.forHost('one');
    for (const pending of [{...clean, uncertain: true}, {...clean, conflict: true}, {...clean, value: {...clean.value, wiki: 'edit'}}]) {
        assert.equal(knowledgePending(pending), true);
        host.knowledge.set('conversation', pending);
        assert.equal(workspace.hasDrafts, true);
        assert.equal(knowledgePending(host.knowledge.get('other')), false);
    }
    host.knowledge.delete('conversation');
    assert.equal(workspace.hasDrafts, false);
});
test('knowledge panes expose keyboard tabs with exact mounted panel ownership in every locale', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatKnowledge.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Component, {props: {conversationId: 'one', generation: 1}}).body;
            assert.equal((html.match(/role="tab"/g) ?? []).length, 3);
            assert.equal((html.match(/role="tabpanel"/g) ?? []).length, 3);
            assert.equal((html.match(/aria-selected="true"/g) ?? []).length, 1);
            for (const pane of ['knowledgePoints', 'wiki', 'recall']) {
                assert.ok(html.includes(`aria-controls="knowledge-panel-${pane}"`));
                assert.ok(html.includes(`aria-labelledby="knowledge-tab-${pane}"`));
                assert.ok(html.includes(t(language, `chatKnowledge.${pane}`)));
            }
        }
    } finally { await server.close(); }
});
