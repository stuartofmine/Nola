export interface WorkspaceReview {root: string; truncated: boolean; entries: {path: string; index: string; worktree: string}[]; staged: {text: string | null; failed: boolean}; unstaged: {text: string | null; failed: boolean}}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
export function parseWorkspaceReview(value: unknown, host: string, conversation: string): WorkspaceReview {
    if (!object(value) || value.source_host_id !== host || value.conversation_id !== conversation || typeof value.workspace_root !== 'string' || !object(value.review)) throw new Error('review.owner');
    const review = value.review;
    if (!object(review.inventory) || !Array.isArray(review.inventory.entries) || typeof review.inventory.truncated !== 'boolean') throw new Error('review.invalid');
    const entries = review.inventory.entries.map(entry => {
        if (!object(entry) || typeof entry.path !== 'string' || !entry.path || typeof entry.index_status !== 'string' || typeof entry.worktree_status !== 'string') throw new Error('review.invalid');
        return {path: entry.path, index: entry.index_status, worktree: entry.worktree_status};
    });
    const diff = (value: unknown) => {
        if (!object(value) || (value.text !== null && typeof value.text !== 'string') || (value.error !== null && typeof value.error !== 'string')
            || (value.text === null) === (value.error === null)) throw new Error('review.invalid');
        return {text: value.text as string | null, failed: value.error !== null};
    };
    return {root: value.workspace_root, truncated: review.inventory.truncated, entries, staged: diff(review.staged), unstaged: diff(review.unstaged)};
}
