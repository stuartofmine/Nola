import test from 'node:test';
import assert from 'node:assert/strict';
import {initialWorkspaceLocation} from './workspaceNavigation.ts';

test('initial layout preserves exact conversation and Host links and adapts management locations', () => {
    for (const path of ['/today', '/conversations']) {
        const desktop = initialWorkspaceLocation(`http://local${path}`, 1440);
        assert.equal(desktop.pathname, '/chat');
        assert.equal(desktop.searchParams.get('content'), path);
        assert.equal(initialWorkspaceLocation(desktop.href, 390).pathname, path);
        assert.equal(initialWorkspaceLocation(`http://local${path}`, 700).pathname, path);
    }
    assert.equal(initialWorkspaceLocation('http://local/chat', 700).pathname, '/today');
    assert.equal(initialWorkspaceLocation('http://local/chat', 701).pathname, '/chat');
    const exact = 'http://local/chat?conversation=original&sourceHost=host';
    assert.equal(initialWorkspaceLocation(exact, 390).href, exact);
    assert.equal(initialWorkspaceLocation('http://local/chat?sourceHost=host', 390).pathname, '/chat');
    const desktop = initialWorkspaceLocation('http://local/settings?section=privacy#access', 1000);
    assert.equal(desktop.pathname, '/chat');
    assert.equal(desktop.searchParams.get('content'), '/settings?section=privacy#access');
    const mobile = initialWorkspaceLocation(desktop.href, 390);
    assert.equal(mobile.pathname, '/settings');
    assert.equal(mobile.search, '?section=privacy');
    assert.equal(mobile.hash, '#access');
    assert.equal(initialWorkspaceLocation('http://local/login', 1000).pathname, '/login');
});


test('activity source focus keeps exact child ownership and a bounded return destination', async () => {
    const {activitySourcePath, activityFocusOwner, activityReturnPath} = await import('./activity.ts');
    const entry = {conversation_id:'child', run_id:'run-child', assistant_message_id:'answer', source:'agent' as const,
        title:'Review', created_at_ms:1, completed_at_ms:null, status:'waiting_for_input', needs_response:true, is_current:true};
    const detail = {entry, source_conversation_id:'parent', review_in_source:true, explanation:null,
        result_text:null, result_has_more:false, execution_host_id:'host', context_scope:'task' as const};
    const target = new URL(activitySourcePath(detail)!, 'http://local');
    assert.equal(target.searchParams.get('conversation'), 'parent');
    assert.deepEqual(activityFocusOwner(target), {conversationId:'child',runId:'run-child'});
    assert.equal(activityReturnPath(target), '/activity?conversation_id=child&run_id=run-child&assistant_message_id=answer');
    target.searchParams.set('from_activity','https://elsewhere.example/activity?conversation_id=child&run_id=r&assistant_message_id=a');
    assert.equal(activityReturnPath(target), null);
    const completed = new URL(activitySourcePath({...detail,review_in_source:false,entry:{...entry,needs_response:false}})!, 'http://local');
    assert.equal(activityFocusOwner(completed), null);
    assert.equal(activitySourcePath({...detail,source_conversation_id:null}), null);
});
