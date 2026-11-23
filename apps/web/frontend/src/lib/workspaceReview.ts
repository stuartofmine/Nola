export interface WorkspaceFileEntry {
    path: string;
    size: number;
    isDir: boolean;
    modifiedAtMs: number | null;
}
export interface WorkspaceReview {
    root: string;
    isGitRepository: boolean;
    isSessionSandbox: boolean;
    truncated: boolean;
    entries: {path: string; index: string; worktree: string}[];
    files: WorkspaceFileEntry[];
    staged: {text: string | null; failed: boolean};
    unstaged: {text: string | null; failed: boolean};
}
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
export function parseWorkspaceReview(value: unknown, host: string, conversation: string): WorkspaceReview {
    if (!object(value) || value.source_host_id !== host || value.conversation_id !== conversation || typeof value.workspace_root !== 'string') throw new Error('review.owner');
    const isGitRepository = value.is_git_repository !== false && value.isGitRepository !== false;
    const isSessionSandbox = value.is_session_sandbox === true || value.isSessionSandbox === true;
    if (!isGitRepository) {
        if (!Array.isArray(value.files) || typeof value.truncated !== 'boolean') throw new Error('review.invalid');
        const files = value.files.map(entry => {
            if (!object(entry) || typeof entry.path !== 'string' || !entry.path || typeof entry.size !== 'number' || typeof entry.is_dir !== 'boolean') throw new Error('review.invalid');
            const modifiedAtMs = typeof entry.modified_at_ms === 'number' ? entry.modified_at_ms : null;
            return {path: entry.path, size: entry.size, isDir: entry.is_dir, modifiedAtMs};
        });
        return {
            root: value.workspace_root,
            isGitRepository: false,
            isSessionSandbox,
            truncated: value.truncated,
            entries: [],
            files,
            staged: {text: null, failed: false},
            unstaged: {text: null, failed: false},
        };
    }
    if (!object(value.review)) throw new Error('review.owner');
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
    return {
        root: value.workspace_root,
        isGitRepository: true,
        isSessionSandbox,
        truncated: review.inventory.truncated,
        entries,
        files: [],
        staged: diff(review.staged),
        unstaged: diff(review.unstaged),
    };
}
