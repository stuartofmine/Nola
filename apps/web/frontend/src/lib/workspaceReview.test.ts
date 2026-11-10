import test from 'node:test';
import assert from 'node:assert/strict';
import {parseWorkspaceReview} from './workspaceReview.ts';
const response = () => ({source_host_id: 'host', conversation_id: 'conversation', workspace_root: '/workspace', review: {
    inventory: {entries: [{path: 'file.txt', index_status: 'A', worktree_status: 'M'}], truncated: false},
    staged: {text: '+staged', error: null}, unstaged: {text: null, error: 'output limit exceeded'},
}});
test('workspace review preserves partial diff failure without claiming a clean worktree', () => {
    const review = parseWorkspaceReview(response(), 'host', 'conversation');
    assert.deepEqual(review.entries, [{path: 'file.txt', index: 'A', worktree: 'M'}]);
    assert.equal(review.staged.text, '+staged');
    assert.equal(review.unstaged.failed, true);
    assert.equal(review.unstaged.text, null);
});
test('workspace review rejects cross-Host/cross-conversation data and ambiguous diff contracts', () => {
    assert.throws(() => parseWorkspaceReview(response(), 'replacement', 'conversation'));
    assert.throws(() => parseWorkspaceReview(response(), 'host', 'other'));
    const malformed = response(); malformed.review.staged.error = 'failed' as unknown as null;
    assert.throws(() => parseWorkspaceReview(malformed, 'host', 'conversation'));
});
