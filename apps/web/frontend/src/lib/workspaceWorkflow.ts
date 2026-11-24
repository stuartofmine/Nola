// SPDX-License-Identifier: Apache-2.0
import { api } from './api.ts';

export interface WorkspaceDiffFile {
    status: string;
    path: string;
}

export interface WorkspaceDiffData {
    is_git_repo: boolean;
    branch: string;
    files: WorkspaceDiffFile[];
    diff_text: string;
    truncated: boolean;
}

export interface WorkspaceCheckpoint {
    turn: number;
    label: string;
    tree_sha: string;
}

export interface WorkspaceCheckpointsData {
    checkpoints: WorkspaceCheckpoint[];
    redo_count?: number;
}

export interface WorkspaceRestoreResult {
    ok: boolean;
    checkpoint_turn: number;
    checkpoint_label: string;
    restored_files: string[];
    removed_files: string[];
    remaining_undo: number;
    remaining_redo: number;
}

export interface WorkspaceWorktree {
    path: string;
    head: string;
    branch: string;
    is_main: boolean;
    is_current: boolean;
}

export interface WorkspaceWorktreesData {
    is_git_repo: boolean;
    worktrees: WorkspaceWorktree[];
}

export interface WorkspaceCreateWorktreeResult {
    ok: boolean;
    path: string;
    branch: string;
}

export type UnifiedDiffLineKind = 'file' | 'hunk' | 'add' | 'del' | 'context';

export interface UnifiedDiffLine {
    kind: UnifiedDiffLineKind;
    text: string;
}

export type FileStatusTone = 'add' | 'del' | 'mod' | 'untracked' | 'rename';

export function classifyDiffLine(line: string): UnifiedDiffLineKind {
    if (
        line.startsWith('diff --git ')
        || line.startsWith('--- ')
        || line.startsWith('+++ ')
        || line.startsWith('index ')
        || line.startsWith('new file')
        || line.startsWith('deleted file')
        || line.startsWith('rename from ')
        || line.startsWith('rename to ')
    ) {
        return 'file';
    }
    if (line.startsWith('@@')) return 'hunk';
    if (line.startsWith('+')) return 'add';
    if (line.startsWith('-')) return 'del';
    return 'context';
}

export function parseUnifiedDiffLines(diffText: string): UnifiedDiffLine[] {
    if (!diffText) return [];
    const lines = diffText.replace(/\r\n/g, '\n').split('\n');
    if (lines.length && lines[lines.length - 1] === '') lines.pop();
    return lines.map(text => ({ kind: classifyDiffLine(text), text }));
}

export function fileStatusTone(status: string): FileStatusTone {
    const code = status.trim();
    if (code === '??') return 'untracked';
    if (code.includes('D')) return 'del';
    if (code.includes('A')) return 'add';
    if (code.includes('R') || code.includes('C')) return 'rename';
    return 'mod';
}

export function isValidWorktreeBranchName(raw: string): boolean {
    const branch = raw.trim();
    if (!branch || branch.length > 128) return false;
    if (!/^[A-Za-z0-9._/-]+$/.test(branch)) return false;
    if (
        branch.startsWith('-')
        || branch.startsWith('/')
        || branch.startsWith('.')
        || branch.endsWith('/')
        || branch.endsWith('.')
        || branch.endsWith('.lock')
        || branch.includes('..')
        || branch.includes('//')
        || branch.includes('/.')
        || branch.includes('./')
    ) {
        return false;
    }
    return true;
}

function withQuery(base: string, params: Record<string, string | undefined>): string {
    const search = new URLSearchParams();
    for (const [key, value] of Object.entries(params)) {
        if (value && value.trim()) search.set(key, value.trim());
    }
    const qs = search.toString();
    return qs ? `${base}?${qs}` : base;
}

export function fetchWorkspaceDiff(options: {
    conversationId?: string;
    path?: string;
    signal?: AbortSignal;
} = {}): Promise<WorkspaceDiffData> {
    const url = withQuery('/api/workspace/diff', {
        conversation_id: options.conversationId,
        path: options.path,
    });
    return api<WorkspaceDiffData>(url, { signal: options.signal });
}

export function fetchWorkspaceCheckpoints(options: {
    conversationId?: string;
    signal?: AbortSignal;
} = {}): Promise<WorkspaceCheckpointsData> {
    const url = withQuery('/api/workspace/checkpoints', {
        conversation_id: options.conversationId,
    });
    return api<WorkspaceCheckpointsData>(url, { signal: options.signal });
}

export function undoWorkspaceTurn(conversationId?: string): Promise<WorkspaceRestoreResult> {
    return api<WorkspaceRestoreResult>('/api/workspace/undo', {
        method: 'POST',
        body: JSON.stringify(conversationId ? { conversation_id: conversationId } : {}),
    });
}

export function redoWorkspaceTurn(conversationId?: string): Promise<WorkspaceRestoreResult> {
    return api<WorkspaceRestoreResult>('/api/workspace/redo', {
        method: 'POST',
        body: JSON.stringify(conversationId ? { conversation_id: conversationId } : {}),
    });
}

export function fetchWorkspaceWorktrees(options: {
    conversationId?: string;
    signal?: AbortSignal;
} = {}): Promise<WorkspaceWorktreesData> {
    const url = withQuery('/api/workspace/worktrees', {
        conversation_id: options.conversationId,
    });
    return api<WorkspaceWorktreesData>(url, { signal: options.signal });
}

export function createWorkspaceWorktree(
    branch: string,
    options: { baseRef?: string; conversationId?: string } = {},
): Promise<WorkspaceCreateWorktreeResult> {
    return api<WorkspaceCreateWorktreeResult>('/api/workspace/worktrees', {
        method: 'POST',
        body: JSON.stringify({
            branch: branch.trim(),
            ...(options.baseRef?.trim() ? { base_ref: options.baseRef.trim() } : {}),
            ...(options.conversationId?.trim() ? { conversation_id: options.conversationId.trim() } : {}),
        }),
    });
}
