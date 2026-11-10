import test from 'node:test';
import assert from 'node:assert/strict';
import { createWorkspaceDrafts } from './workspaceDrafts.ts';
import { workspaceDestination, workspaceClick } from './workspaceNavigation.ts';

test('room drafts and recipients remain isolated by room and authenticated host', () => {
    const workspace = createWorkspaceDrafts(null), host = workspace.forHost('host-a');
    host.rooms.set('one', {text: 'first', participantId: 'agent-one'});
    host.rooms.set('two', {text: 'second', participantId: null});
    assert.equal(workspace.hasDrafts, true);
    assert.equal(workspace.forHost('host-a').rooms.get('one')?.participantId, 'agent-one');
    assert.equal(host.rooms.get('two')?.participantId, null);
    assert.equal(workspace.forHost('host-b').rooms.size, 0);
    assert.equal(host.rooms.size, 0);
});

test('workspace drafts survive remounts but never cross authenticated hosts or workspace lifetime', () => {
    const workspace = createWorkspaceDrafts();
    const host = workspace.forHost('host-a');
    host.text.write('conversation', 'unfinished');
    host.attachments.write('conversation', [{id: 'file', conversationId: 'conversation', name: 'data.txt', mimeType: 'text/plain', byteCount: 3, text: 'abc', clipped: false}]);
    host.submissions.set('conversation', {submissionId: 'submission', text: 'unfinished', attachmentIds: ['file']});
    host.replyNotices.set('conversation', [{submissionId: 'submission', userMessageId: 'user', assistantMessageId: 'assistant', kind: 'notSent'}]);
    host.lastConversationId = 'conversation';
    const transport = new AbortController(); host.transports.add(transport);
    let completionCleared = false;
    let sourceCleared = false;
    host.completions.clear = () => { completionCleared = true; };
    host.sources.clear = () => { sourceCleared = true; };
    assert.equal(workspace.forHost('host-a'), host);
    assert.equal(workspace.hasDrafts, true);
    assert.equal(workspace.selectedConversationId, 'conversation');
    assert.equal(workspace.forHost('host-a').attachments.read('conversation')[0].text, 'abc');
    assert.equal(createWorkspaceDrafts().hasDrafts, false);
    const other = workspace.forHost('host-b');
    assert.equal(transport.signal.aborted, true);
    assert.equal(completionCleared, true);
    assert.equal(sourceCleared, true);
    assert.equal(other.text.read('conversation'), '');
    assert.equal(host.text.read('conversation'), '');
    assert.equal(host.attachments.hasDrafts, false);
    assert.equal(host.submissions.size, 0);
    assert.equal(host.replyNotices.size, 0);
    assert.equal(other.replyNotices.size, 0);
    assert.equal(host.lastConversationId, null);
    other.text.write('conversation', 'new');
    workspace.clear();
    assert.equal(workspace.hasDrafts, false);
    assert.throws(() => workspace.forHost(''));
});
test('deleting a conversation from another workspace page forgets only its exact drafts', () => {
    const workspace = createWorkspaceDrafts(); const host = workspace.forHost('host');
    host.text.write('one', 'draft one'); host.text.write('two', 'draft two'); host.lastConversationId = 'one';
    const forgotten: string[] = [];
    host.completions.forget = id => { forgotten.push(id); };
    host.sources.forget = id => { forgotten.push(`source:${id}`); };
    workspace.forgetConversation('one');
    assert.deepEqual(forgotten, ['one', 'source:one']);
    assert.equal(workspace.selectedConversationId, null);
    assert.equal(host.text.read('one'), '');
    assert.equal(host.text.read('two'), 'draft two');
});
test('goal drafts are conversation-scoped, included in unload protection, and cleared on host changes', () => {
    const workspace = createWorkspaceDrafts(); const host = workspace.forHost('host');
    host.objectives.write('one', 'First goal'); host.objectives.write('two', 'Second goal');
    assert.equal(workspace.hasDrafts, true);
    assert.equal(workspace.forHost('host').objectives.read('one'), 'First goal');
    workspace.forgetConversation('one');
    assert.equal(host.objectives.read('one'), '');
    assert.equal(host.objectives.read('two'), 'Second goal');
    workspace.forHost('other-host');
    assert.equal(host.objectives.read('two'), '');
    assert.equal(workspace.hasDrafts, false);
});
test('workspace navigation only intercepts known same-origin pages, not authentication, files or fragments', () => {
    const current = 'https://nola.test/chat?conversation=one';
    assert.equal(workspaceDestination('/models?tab=remote', current)?.pathname, '/models');
    assert.equal(workspaceDestination('/chat?conversation=two', current)?.search, '?conversation=two');
    for (const href of ['/login', '/api/chat/conversations', '#tool', 'https://elsewhere.test/models', 'https://user:secret@nola.test/models', 'javascript:alert(1)', '/unknown']) {
        assert.equal(workspaceDestination(href, current), null, href);
    }
    assert.equal(workspaceDestination('/models', 'https://nola.test/login'), null);
    const anchor = {hasAttribute: () => false, target: '', relList: {contains: () => false}} as unknown as HTMLAnchorElement;
    const event = {button: 0} as MouseEvent;
    assert.equal(workspaceClick(event, anchor), true);
    for (const override of [{button: 1}, {ctrlKey: true}, {metaKey: true}, {shiftKey: true}, {altKey: true}, {defaultPrevented: true}]) {
        assert.equal(workspaceClick({...event, ...override} as MouseEvent, anchor), false);
    }
    assert.equal(workspaceClick(event, {...anchor, target: '_blank'} as HTMLAnchorElement), false);
    assert.equal(workspaceClick(event, {...anchor, hasAttribute: () => true} as HTMLAnchorElement), false);
});
