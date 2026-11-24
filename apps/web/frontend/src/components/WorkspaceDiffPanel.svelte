<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { errorMessage } from '../lib/api';
    import { highlightCode, languageFromPath } from '../lib/markdown';
    import { pushToast } from '../lib/toast';
    import PrototypeIcon from './PrototypeIcon.svelte';
    import {
        createWorkspaceWorktree,
        fetchWorkspaceCheckpoints,
        fetchWorkspaceDiff,
        fetchWorkspaceWorktrees,
        fileStatusTone,
        isValidWorktreeBranchName,
        parseUnifiedDiffLines,
        redoWorkspaceTurn,
        undoWorkspaceTurn,
        type WorkspaceCheckpoint,
        type WorkspaceDiffData,
        type WorkspaceWorktree,
    } from '../lib/workspaceWorkflow';

    let {
        open = false,
        conversationId,
        onclose,
        onsummary,
    }: {
        open?: boolean;
        conversationId?: string;
        onclose?: () => void;
        onsummary?: (summary: { changedCount: number; branch: string; isGitRepo: boolean }) => void;
    } = $props();

    type TabId = 'diff' | 'checkpoints' | 'worktrees';
    type DiffViewMode = 'unified' | 'split';

    let activeTab = $state<TabId>('diff');
    let diffViewMode = $state<DiffViewMode>('unified');
    let loading = $state(false);
    let diffLoading = $state(false);
    let mutating = $state(false);
    let loadError = $state('');
    let selectedPath = $state<string | null>(null);
    let diffData = $state<WorkspaceDiffData | null>(null);
    let checkpoints = $state<WorkspaceCheckpoint[]>([]);
    let redoCount = $state(0);
    let worktrees = $state<WorkspaceWorktree[]>([]);
    let worktreesIsGitRepo = $state(true);

    let newBranch = $state('');
    let branchError = $state('');
    let creatingWorktree = $state(false);

    let requestController: AbortController | null = null;
    let requestEpoch = 0;
    let diffController: AbortController | null = null;
    let diffEpoch = 0;

    type RenderedDiffLine = {
        kind: 'file' | 'hunk' | 'add' | 'del' | 'context';
        oldNum: string;
        newNum: string;
        sign: string;
        text: string;
        html: string | null;
    };

    const diffLines = $derived(parseUnifiedDiffLines(diffData?.diff_text ?? ''));
    const renderedDiffLines = $derived.by<RenderedDiffLine[]>(() => {
        let currentLang = languageFromPath(selectedPath);
        let oldLine = 0;
        let newLine = 0;
        const out: RenderedDiffLine[] = [];
        for (const line of diffLines) {
            if (line.kind === 'file') {
                const gitMatch = /^diff --git a\/.+ b\/(.+)$/.exec(line.text);
                if (gitMatch) {
                    currentLang = languageFromPath(gitMatch[1]);
                } else if (line.text.startsWith('+++ b/')) {
                    currentLang = languageFromPath(line.text.slice(6));
                }
                out.push({ kind: 'file', oldNum: '', newNum: '', sign: '', text: line.text, html: null });
                continue;
            }
            if (line.kind === 'hunk') {
                const hunkMatch = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line.text);
                if (hunkMatch) {
                    oldLine = Number.parseInt(hunkMatch[1], 10);
                    newLine = Number.parseInt(hunkMatch[2], 10);
                }
                out.push({ kind: 'hunk', oldNum: '', newNum: '', sign: '', text: line.text, html: null });
                continue;
            }
            if (line.kind === 'add') {
                const codeText = line.text.slice(1);
                out.push({
                    kind: 'add',
                    oldNum: '',
                    newNum: newLine > 0 ? String(newLine++) : '',
                    sign: '+',
                    text: line.text,
                    html: codeText ? highlightCode(codeText, currentLang) : ' ',
                });
                continue;
            }
            if (line.kind === 'del') {
                const codeText = line.text.slice(1);
                out.push({
                    kind: 'del',
                    oldNum: oldLine > 0 ? String(oldLine++) : '',
                    newNum: '',
                    sign: '-',
                    text: line.text,
                    html: codeText ? highlightCode(codeText, currentLang) : ' ',
                });
                continue;
            }
            const hasLeadingSpace = line.text.startsWith(' ');
            const codeText = hasLeadingSpace ? line.text.slice(1) : line.text;
            out.push({
                kind: 'context',
                oldNum: oldLine > 0 ? String(oldLine++) : '',
                newNum: newLine > 0 ? String(newLine++) : '',
                sign: hasLeadingSpace ? ' ' : '',
                text: line.text,
                html: codeText ? highlightCode(codeText, currentLang) : ' ',
            });
        }
        return out;
    });

    type SplitDiffCell = {
        kind: 'add' | 'del' | 'context' | 'empty';
        num: string;
        sign: string;
        html: string;
    };
    type SplitDiffRow =
        | { kind: 'meta'; lineKind: 'file' | 'hunk'; text: string }
        | { kind: 'pair'; left: SplitDiffCell; right: SplitDiffCell };

    const EMPTY_SPLIT_CELL: SplitDiffCell = { kind: 'empty', num: '', sign: '', html: '' };

    const splitDiffRows = $derived.by<SplitDiffRow[]>(() => {
        const rows: SplitDiffRow[] = [];
        const lines = renderedDiffLines;
        let i = 0;
        while (i < lines.length) {
            const line = lines[i];
            if (line.kind === 'file' || line.kind === 'hunk') {
                rows.push({ kind: 'meta', lineKind: line.kind, text: line.text });
                i++;
                continue;
            }
            if (line.kind === 'context') {
                const html = line.html ?? ' ';
                rows.push({
                    kind: 'pair',
                    left: { kind: 'context', num: line.oldNum, sign: line.sign, html },
                    right: { kind: 'context', num: line.newNum, sign: line.sign, html },
                });
                i++;
                continue;
            }
            const dels: RenderedDiffLine[] = [];
            while (i < lines.length && lines[i].kind === 'del') {
                dels.push(lines[i++]);
            }
            const adds: RenderedDiffLine[] = [];
            while (i < lines.length && lines[i].kind === 'add') {
                adds.push(lines[i++]);
            }
            const pairCount = Math.max(dels.length, adds.length);
            for (let j = 0; j < pairCount; j++) {
                const delLine = dels[j];
                const addLine = adds[j];
                rows.push({
                    kind: 'pair',
                    left: delLine
                        ? { kind: 'del', num: delLine.oldNum, sign: '-', html: delLine.html ?? ' ' }
                        : EMPTY_SPLIT_CELL,
                    right: addLine
                        ? { kind: 'add', num: addLine.newNum, sign: '+', html: addLine.html ?? ' ' }
                        : EMPTY_SPLIT_CELL,
                });
            }
        }
        return rows;
    });
    const diffModeLabel = $derived(
        diffViewMode === 'split'
            ? ($locale === 'zh-Hans' ? '单栏视图' : 'Unified')
            : ($locale === 'zh-Hans' ? '双栏对比' : 'Split'),
    );
    const diffModeTitle = $derived(
        diffViewMode === 'split'
            ? ($locale === 'zh-Hans' ? '切换为单栏统一差异视图' : 'Switch to unified diff view')
            : ($locale === 'zh-Hans' ? '切换为双栏并排差异视图' : 'Switch to side-by-side split diff view'),
    );
    const reversedCheckpoints = $derived([...checkpoints].reverse());
    const canUndo = $derived(checkpoints.length > 0 && !mutating);
    const canRedo = $derived(redoCount > 0 && !mutating);

    async function refreshAll(pathOverride?: string | null) {
        requestController?.abort();
        diffController?.abort();
        diffController = null;
        const controller = new AbortController();
        requestController = controller;
        const epoch = ++requestEpoch;
        const currentDiffEpoch = ++diffEpoch;
        const targetPath = pathOverride === undefined ? selectedPath : pathOverride;

        loading = true;
        diffLoading = false;
        loadError = '';
        try {
            const [diff, cp, wt] = await Promise.all([
                fetchWorkspaceDiff({
                    conversationId,
                    path: targetPath ?? undefined,
                    signal: controller.signal,
                }),
                fetchWorkspaceCheckpoints({
                    conversationId,
                    signal: controller.signal,
                }),
                fetchWorkspaceWorktrees({
                    conversationId,
                    signal: controller.signal,
                }),
            ]);
            if (controller.signal.aborted || epoch !== requestEpoch) return;
            checkpoints = cp.checkpoints ?? [];
            redoCount = cp.redo_count ?? 0;
            worktrees = wt.worktrees ?? [];
            worktreesIsGitRepo = wt.is_git_repo;
            if (currentDiffEpoch === diffEpoch) {
                diffData = diff;
                if (targetPath === null) {
                    onsummary?.({
                        changedCount: diff.files.length,
                        branch: diff.branch,
                        isGitRepo: diff.is_git_repo,
                    });
                }
            }
        } catch (cause) {
            if (controller.signal.aborted || epoch !== requestEpoch) return;
            loadError = errorMessage(cause);
        } finally {
            if (!controller.signal.aborted && epoch === requestEpoch) {
                loading = false;
            }
        }
    }

    async function loadDiffOnly(targetPath: string | null) {
        diffController?.abort();
        const controller = new AbortController();
        diffController = controller;
        const currentDiffEpoch = ++diffEpoch;

        diffLoading = true;
        loadError = '';
        try {
            const diff = await fetchWorkspaceDiff({
                conversationId,
                path: targetPath ?? undefined,
                signal: controller.signal,
            });
            if (controller.signal.aborted || currentDiffEpoch !== diffEpoch) return;
            diffData = diff;
            if (targetPath === null) {
                onsummary?.({
                    changedCount: diff.files.length,
                    branch: diff.branch,
                    isGitRepo: diff.is_git_repo,
                });
            }
        } catch (cause) {
            if (controller.signal.aborted || currentDiffEpoch !== diffEpoch) return;
            loadError = errorMessage(cause);
        } finally {
            if (!controller.signal.aborted && currentDiffEpoch === diffEpoch) {
                diffLoading = false;
            }
        }
    }

    async function selectFile(path: string | null) {
        selectedPath = path;
        await loadDiffOnly(path);
    }

    async function handleUndo() {
        if (!canUndo) return;
        mutating = true;
        try {
            const result = await undoWorkspaceTurn(conversationId);
            redoCount = result.remaining_redo;
            pushToast(
                t($locale, 'workspaceWorkflow.undoSuccess', {
                    turn: result.checkpoint_turn,
                    restored: result.restored_files.length,
                    removed: result.removed_files.length,
                }),
                'success',
                3600,
            );
            selectedPath = null;
            await refreshAll(null);
        } catch (cause) {
            pushToast(errorMessage(cause), 'error', 4000);
        } finally {
            mutating = false;
        }
    }

    async function handleRedo() {
        if (!canRedo) return;
        mutating = true;
        try {
            const result = await redoWorkspaceTurn(conversationId);
            redoCount = result.remaining_redo;
            pushToast(
                t($locale, 'workspaceWorkflow.redoSuccess', {
                    turn: result.checkpoint_turn,
                    restored: result.restored_files.length,
                    removed: result.removed_files.length,
                }),
                'success',
                3600,
            );
            selectedPath = null;
            await refreshAll(null);
        } catch (cause) {
            pushToast(errorMessage(cause), 'error', 4000);
        } finally {
            mutating = false;
        }
    }

    async function handleCreateWorktree(event: SubmitEvent) {
        event.preventDefault();
        const trimmed = newBranch.trim();
        if (!isValidWorktreeBranchName(trimmed)) {
            branchError = t($locale, 'workspaceWorkflow.invalidBranch');
            return;
        }
        branchError = '';
        creatingWorktree = true;
        try {
            const result = await createWorkspaceWorktree(trimmed, { conversationId });
            newBranch = '';
            pushToast(
                t($locale, 'workspaceWorkflow.worktreeCreated', { branch: result.branch }),
                'success',
                3600,
            );
            await refreshAll();
        } catch (cause) {
            const message = errorMessage(cause);
            branchError = message;
            pushToast(message, 'error', 4000);
        } finally {
            creatingWorktree = false;
        }
    }

    function handleKeydown(event: KeyboardEvent) {
        if (open && event.key === 'Escape') {
            event.preventDefault();
            onclose?.();
        }
    }

    $effect(() => {
        if (open) {
            conversationId;
            void refreshAll();
        } else {
            requestController?.abort();
            requestController = null;
            diffController?.abort();
            diffController = null;
        }
    });
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
    <div
        class="workflow-backdrop"
        role="presentation"
        onclick={event => {
            if (event.target === event.currentTarget) onclose?.();
        }}
    >
        <div
            class="workflow-drawer"
            class:split-mode={activeTab === 'diff' && diffViewMode === 'split'}
            role="dialog"
            aria-modal="true"
            aria-label={t($locale, 'workspaceWorkflow.panelTitle')}
        >
            <header class="drawer-header">
                <div class="drawer-title-group">
                    <h2>{t($locale, 'workspaceWorkflow.panelTitle')}</h2>
                    {#if diffData?.is_git_repo && diffData.branch}
                        <span class="branch-pill" title={diffData.branch}>
                            <svg aria-hidden="true" viewBox="0 0 24 24" class="mini-icon">
                                <circle cx="6" cy="6" r="2.5" />
                                <circle cx="18" cy="6" r="2.5" />
                                <circle cx="6" cy="18" r="2.5" />
                                <path d="M6 8.5v7M18 8.5c0 4-4 5-12 5" />
                            </svg>
                            <span>{diffData.branch}</span>
                        </span>
                    {/if}
                </div>
                <div class="drawer-header-actions">
                    {#if activeTab === 'diff'}
                        <button
                            type="button"
                            class="action-btn diff-mode-toggle"
                            class:active={diffViewMode === 'split'}
                            aria-pressed={diffViewMode === 'split'}
                            onclick={() => (diffViewMode = diffViewMode === 'unified' ? 'split' : 'unified')}
                            title={diffModeTitle}
                        >
                            <svg aria-hidden="true" viewBox="0 0 24 24" class="mini-icon">
                                {#if diffViewMode === 'split'}
                                    <rect x="3" y="4" width="18" height="16" rx="2" />
                                    <path d="M7 9h10M7 13h10M7 17h6" />
                                {:else}
                                    <rect x="3" y="4" width="18" height="16" rx="2" />
                                    <path d="M12 4v16" />
                                {/if}
                            </svg>
                            <span>{diffModeLabel}</span>
                        </button>
                    {/if}
                    <button
                        type="button"
                        class="action-btn"
                        onclick={handleUndo}
                        disabled={!canUndo}
                        title={t($locale, 'workspaceWorkflow.undoTurn')}
                    >
                        <svg aria-hidden="true" viewBox="0 0 24 24" class="mini-icon">
                            <path d="M9 14 4 9l5-5" />
                            <path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
                        </svg>
                        <span>{t($locale, 'workspaceWorkflow.undoTurn')}</span>
                        {#if checkpoints.length > 0}
                            <span class="count-chip">{checkpoints.length}</span>
                        {/if}
                    </button>
                    <button
                        type="button"
                        class="action-btn"
                        onclick={handleRedo}
                        disabled={!canRedo}
                        title={t($locale, 'workspaceWorkflow.redoTurn')}
                    >
                        <svg aria-hidden="true" viewBox="0 0 24 24" class="mini-icon">
                            <path d="m15 14 5-5-5-5" />
                            <path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13" />
                        </svg>
                        <span>{t($locale, 'workspaceWorkflow.redoTurn')}</span>
                        {#if redoCount > 0}
                            <span class="count-chip">{redoCount}</span>
                        {/if}
                    </button>
                    <button
                        type="button"
                        class="icon-btn"
                        onclick={() => void refreshAll()}
                        disabled={loading}
                        aria-label={t($locale, 'workspaceWorkflow.refresh')}
                        title={t($locale, 'workspaceWorkflow.refresh')}
                    >
                        <svg aria-hidden="true" viewBox="0 0 24 24" class="mini-icon">
                            <path d="M21 12a9 9 0 1 1-2.64-6.36" />
                            <path d="M21 3v6h-6" />
                        </svg>
                    </button>
                    <button
                        type="button"
                        class="icon-btn"
                        onclick={() => onclose?.()}
                        aria-label={t($locale, 'workspaceWorkflow.close')}
                        title={t($locale, 'workspaceWorkflow.close')}
                    >
                        <PrototypeIcon name="close" size={16} />
                    </button>
                </div>
            </header>

            <div class="drawer-tabs" role="tablist">
                <button
                    type="button"
                    role="tab"
                    aria-selected={activeTab === 'diff'}
                    onclick={() => (activeTab = 'diff')}
                >
                    <span>{t($locale, 'workspaceWorkflow.tabDiff')}</span>
                    {#if diffData?.files.length}
                        <span class="tab-badge">{diffData.files.length}</span>
                    {/if}
                </button>
                <button
                    type="button"
                    role="tab"
                    aria-selected={activeTab === 'checkpoints'}
                    onclick={() => (activeTab = 'checkpoints')}
                >
                    <span>{t($locale, 'workspaceWorkflow.tabCheckpoints')}</span>
                    {#if checkpoints.length}
                        <span class="tab-badge">{checkpoints.length}</span>
                    {/if}
                </button>
                <button
                    type="button"
                    role="tab"
                    aria-selected={activeTab === 'worktrees'}
                    onclick={() => (activeTab = 'worktrees')}
                >
                    <span>{t($locale, 'workspaceWorkflow.tabWorktrees')}</span>
                    {#if worktrees.length}
                        <span class="tab-badge">{worktrees.length}</span>
                    {/if}
                </button>
            </div>

            <div class="drawer-body">
                {#if loadError}
                    <p class="error-banner" role="alert">{loadError}</p>
                {/if}
                {#if loading && !diffData}
                    <p class="empty-state" role="status">{t($locale, 'workspaceWorkflow.loading')}</p>
                {:else if activeTab === 'diff'}
                    {#if diffData && !diffData.is_git_repo}
                        <p class="empty-state">{t($locale, 'workspaceWorkflow.notGitRepo')}</p>
                    {:else if diffData && !diffData.files.length && !selectedPath}
                        <p class="empty-state">{t($locale, 'workspaceWorkflow.cleanTree')}</p>
                    {:else if diffData}
                        <div class="diff-layout">
                            <div class="file-list" role="list">
                                <button
                                    type="button"
                                    class="file-item"
                                    class:active={selectedPath === null}
                                    onclick={() => void selectFile(null)}
                                >
                                    <span class="status-badge tone-all">*</span>
                                    <span class="file-path">{t($locale, 'workspaceWorkflow.allFiles')}</span>
                                </button>
                                {#each diffData.files as file (file.path)}
                                    {@const tone = fileStatusTone(file.status)}
                                    <button
                                        type="button"
                                        class="file-item"
                                        class:active={selectedPath === file.path}
                                        onclick={() => void selectFile(file.path)}
                                        title={file.path}
                                    >
                                        <span class={`status-badge tone-${tone}`}>{file.status || 'M'}</span>
                                        <span class="file-path">{file.path}</span>
                                    </button>
                                {/each}
                            </div>

                            <div class="diff-viewer">
                                {#if diffData.truncated}
                                    <p class="truncation-note" role="status">
                                        {t($locale, 'workspaceWorkflow.diffTruncated')}
                                    </p>
                                {/if}
                                {#if renderedDiffLines.length}
                                    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                                    <div class="diff-code" class:loading={diffLoading} class:split={diffViewMode === 'split'} tabindex="0" role="region" aria-label={t($locale, 'workspaceWorkflow.tabDiff')}>
                                        {#if diffViewMode === 'split'}
                                            <table class="diff-split-table">
                                                <colgroup>
                                                    <col class="diff-split-col-gutter" />
                                                    <col class="diff-split-col-code" />
                                                    <col class="diff-split-col-gutter" />
                                                    <col class="diff-split-col-code" />
                                                </colgroup>
                                                <tbody>
                                                    {#each splitDiffRows as row, idx (idx)}
                                                        {#if row.kind === 'meta'}
                                                            <tr class={`diff-split-meta diff-${row.lineKind}`}>
                                                                <td colspan="4">{row.text || ' '}</td>
                                                            </tr>
                                                        {:else}
                                                            <tr class="diff-split-row">
                                                                <td class={`diff-split-gutter diff-${row.left.kind}`} aria-hidden="true">{row.left.num}</td>
                                                                <td class={`diff-split-cell diff-split-left diff-${row.left.kind}`}>
                                                                    {#if row.left.kind !== 'empty'}
                                                                        <span class="diff-sign" aria-hidden="true">{row.left.sign}</span><span class="diff-content">{@html row.left.html}</span>
                                                                    {/if}
                                                                </td>
                                                                <td class={`diff-split-gutter diff-${row.right.kind}`} aria-hidden="true">{row.right.num}</td>
                                                                <td class={`diff-split-cell diff-split-right diff-${row.right.kind}`}>
                                                                    {#if row.right.kind !== 'empty'}
                                                                        <span class="diff-sign" aria-hidden="true">{row.right.sign}</span><span class="diff-content">{@html row.right.html}</span>
                                                                    {/if}
                                                                </td>
                                                            </tr>
                                                        {/if}
                                                    {/each}
                                                </tbody>
                                            </table>
                                        {:else}
                                            {#each renderedDiffLines as line, idx (idx)}
                                                <div class={`diff-line diff-${line.kind}`}>
                                                    {#if line.html !== null}
                                                        <span class="diff-gutter" aria-hidden="true">{line.oldNum}</span>
                                                        <span class="diff-gutter" aria-hidden="true">{line.newNum}</span>
                                                        <span class="diff-sign" aria-hidden="true">{line.sign}</span>
                                                        <span class="diff-content">{@html line.html}</span>
                                                    {:else}
                                                        <span class="diff-meta-line">{line.text || ' '}</span>
                                                    {/if}
                                                </div>
                                            {/each}
                                        {/if}
                                    </div>
                                {:else}
                                    <p class="empty-state">{t($locale, 'workspaceWorkflow.noDiffText')}</p>
                                {/if}
                            </div>
                        </div>
                    {/if}
                {:else if activeTab === 'checkpoints'}
                    {#if reversedCheckpoints.length}
                        <ul class="checkpoint-list">
                            {#each reversedCheckpoints as cp (cp.turn + ':' + cp.tree_sha)}
                                <li class="checkpoint-item">
                                    <div class="checkpoint-meta">
                                        <span class="turn-badge">{t($locale, 'workspaceWorkflow.turnLabel', { turn: cp.turn })}</span>
                                        <code class="sha-code">{cp.tree_sha.slice(0, 7)}</code>
                                    </div>
                                    <p class="checkpoint-label">{cp.label}</p>
                                </li>
                            {/each}
                        </ul>
                    {:else}
                        <p class="empty-state">{t($locale, 'workspaceWorkflow.noCheckpoints')}</p>
                    {/if}
                {:else if activeTab === 'worktrees'}
                    {#if !worktreesIsGitRepo}
                        <p class="empty-state">{t($locale, 'workspaceWorkflow.notGitRepo')}</p>
                    {:else}
                        <form class="worktree-form" onsubmit={handleCreateWorktree}>
                            <label for="new-worktree-branch">{t($locale, 'workspaceWorkflow.newWorktree')}</label>
                            <div class="worktree-form-row">
                                <input
                                    id="new-worktree-branch"
                                    type="text"
                                    bind:value={newBranch}
                                    placeholder={t($locale, 'workspaceWorkflow.branchPlaceholder')}
                                    disabled={creatingWorktree}
                                    autocomplete="off"
                                />
                                <button
                                    type="submit"
                                    class="primary"
                                    disabled={creatingWorktree || !newBranch.trim()}
                                >
                                    {t($locale, creatingWorktree ? 'workspaceWorkflow.creatingWorktree' : 'workspaceWorkflow.createWorktree')}
                                </button>
                            </div>
                            {#if branchError}
                                <p class="form-error" role="alert">{branchError}</p>
                            {/if}
                        </form>

                        {#if worktrees.length}
                            <ul class="worktree-list">
                                {#each worktrees as wt (wt.path)}
                                    <li class="worktree-item" class:current={wt.is_current}>
                                        <div class="worktree-heading">
                                            <strong class="worktree-branch">{wt.branch || '(detached)'}</strong>
                                            <div class="worktree-badges">
                                                {#if wt.is_main}
                                                    <span class="wt-badge">{t($locale, 'workspaceWorkflow.badgeMain')}</span>
                                                {/if}
                                                {#if wt.is_current}
                                                    <span class="wt-badge current">{t($locale, 'workspaceWorkflow.badgeCurrent')}</span>
                                                {/if}
                                                {#if wt.head}
                                                    <code class="sha-code">{wt.head.slice(0, 7)}</code>
                                                {/if}
                                            </div>
                                        </div>
                                        <code class="worktree-path">{wt.path}</code>
                                    </li>
                                {/each}
                            </ul>
                        {:else}
                            <p class="empty-state">{t($locale, 'workspaceWorkflow.noWorktrees')}</p>
                        {/if}
                    {/if}
                {/if}
            </div>
        </div>
    </div>
{/if}

<style>
    .workflow-backdrop {
        position: fixed;
        inset: 0;
        z-index: 60;
        display: flex;
        justify-content: flex-end;
        background: rgb(0 0 0 / 0.36);
        backdrop-filter: blur(2px);
    }
    .workflow-drawer {
        width: min(760px, 100vw);
        height: 100dvh;
        display: flex;
        flex-direction: column;
        background: var(--surface);
        color: var(--content);
        border-left: 1px solid var(--border);
        box-shadow: var(--dialog-shadow);
    }
    .workflow-drawer.split-mode {
        width: min(1080px, 100vw);
    }
    .drawer-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
        padding: 14px 16px;
        border-bottom: 1px solid var(--border);
    }
    .drawer-title-group {
        display: flex;
        align-items: center;
        gap: 8px;
        min-width: 0;
    }
    .drawer-title-group h2 {
        margin: 0;
        font-size: 0.95rem;
        font-weight: 650;
    }
    .branch-pill {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 2px 8px;
        border-radius: 999px;
        border: 1px solid var(--border);
        background: var(--panel);
        color: var(--muted);
        font-family: var(--font-code);
        font-size: 0.75rem;
        max-width: 200px;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
    .drawer-header-actions {
        display: flex;
        align-items: center;
        gap: 6px;
        flex-wrap: wrap;
    }
    .action-btn {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        min-height: 32px;
        padding: 4px 10px;
        font-size: 0.78rem;
        border-radius: 8px;
    }
    .diff-mode-toggle.active {
        background: var(--accent-subtle);
        border-color: var(--outline);
        color: var(--content);
        font-weight: 600;
    }
    .icon-btn {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        width: 32px;
        height: 32px;
        min-width: 32px;
        min-height: 32px;
        padding: 0;
        border-radius: 8px;
    }
    .mini-icon {
        width: 14px;
        height: 14px;
        flex: none;
        fill: none;
        stroke: currentColor;
        stroke-width: 1.8;
        stroke-linecap: round;
        stroke-linejoin: round;
    }
    .count-chip,
    .tab-badge {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-width: 18px;
        height: 18px;
        padding: 0 5px;
        border-radius: 999px;
        background: var(--accent-subtle);
        color: var(--content);
        font-size: 0.7rem;
        font-weight: 600;
    }
    .drawer-tabs {
        display: flex;
        gap: 4px;
        padding: 8px 16px;
        border-bottom: 1px solid var(--border);
        background: var(--canvas);
    }
    .drawer-tabs button {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        min-height: 32px;
        padding: 4px 12px;
        border-radius: 8px;
        border: 1px solid transparent;
        background: transparent;
        color: var(--muted);
        font-size: 0.82rem;
        font-weight: 550;
    }
    .drawer-tabs button[aria-selected='true'] {
        background: var(--surface);
        border-color: var(--border);
        color: var(--content);
    }
    .drawer-body {
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding: 16px;
        display: flex;
        flex-direction: column;
        gap: 14px;
    }
    .error-banner {
        margin: 0;
        padding: 10px 12px;
        border-radius: 10px;
        border: 1px solid color-mix(in srgb, var(--danger) 40%, var(--border));
        background: color-mix(in srgb, var(--danger) 10%, transparent);
        color: var(--danger);
        font-size: 0.85rem;
    }
    .empty-state {
        margin: 24px 0;
        text-align: center;
        color: var(--muted);
        font-size: 0.875rem;
    }
    .diff-layout {
        display: grid;
        grid-template-columns: minmax(180px, 230px) minmax(0, 1fr);
        gap: 12px;
        flex: 1;
        min-height: 0;
    }
    .file-list {
        display: flex;
        flex-direction: column;
        gap: 3px;
        overflow: auto;
        padding-right: 4px;
    }
    .file-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        min-height: 32px;
        padding: 5px 8px;
        border-radius: 8px;
        border: 1px solid transparent;
        background: transparent;
        text-align: left;
        font-size: 0.8rem;
    }
    .file-item.active {
        background: var(--accent-subtle);
        border-color: var(--border);
        font-weight: 600;
    }
    .file-path {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-family: var(--font-code);
        font-size: 0.76rem;
    }
    .status-badge {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-width: 22px;
        padding: 1px 4px;
        border-radius: 5px;
        font-family: var(--font-code);
        font-size: 0.68rem;
        font-weight: 700;
        flex: none;
    }
    .tone-all {
        background: var(--panel);
        color: var(--muted);
    }
    .tone-mod {
        background: color-mix(in srgb, var(--warning) 18%, transparent);
        color: var(--warning);
    }
    .tone-add {
        background: color-mix(in srgb, var(--success) 18%, transparent);
        color: var(--success);
    }
    .tone-del {
        background: color-mix(in srgb, var(--danger) 18%, transparent);
        color: var(--danger);
    }
    .tone-untracked {
        background: color-mix(in srgb, var(--info) 18%, transparent);
        color: var(--info);
    }
    .tone-rename {
        background: var(--accent-subtle);
        color: var(--content);
    }
    .diff-viewer {
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        gap: 8px;
    }
    .truncation-note {
        margin: 0;
        padding: 6px 10px;
        border-radius: 8px;
        background: color-mix(in srgb, var(--warning) 14%, transparent);
        color: var(--warning);
        font-size: 0.78rem;
    }
    .diff-code {
        flex: 1;
        min-height: 220px;
        overflow: auto;
        border: 1px solid var(--border);
        border-radius: 10px;
        background: var(--canvas);
        padding: 8px 0;
        font-family: var(--font-code);
        font-size: 12px;
        line-height: 1.55;
        --tok-kw: #9a2b7c;
        --tok-str: #146c37;
        --tok-num: #9a4d00;
        --tok-cmt: #63636e;
        --tok-type: #0e5a8a;
        --tok-attr: #6f42c1;
        --tok-tag: #1d4ed8;
        --tok-meta: #8a4b08;
        --tok-key: #0f5a78;
        --tok-var: #b43a1c;
    }
    .diff-code.loading {
        opacity: 0.72;
    }
    @media (prefers-color-scheme: dark) {
        :global(:root:not([data-appearance="light"])) .diff-code {
            --tok-kw: #d896ff;
            --tok-str: #7ee787;
            --tok-num: #f0b85a;
            --tok-cmt: #8b8b96;
            --tok-type: #79c0ff;
            --tok-attr: #d2a8ff;
            --tok-tag: #7ee787;
            --tok-meta: #ffa657;
            --tok-key: #79c0ff;
            --tok-var: #ffa198;
        }
    }
    :global(:root[data-appearance="dark"]) .diff-code {
        --tok-kw: #d896ff;
        --tok-str: #7ee787;
        --tok-num: #f0b85a;
        --tok-cmt: #8b8b96;
        --tok-type: #79c0ff;
        --tok-attr: #d2a8ff;
        --tok-tag: #7ee787;
        --tok-meta: #ffa657;
        --tok-key: #79c0ff;
        --tok-var: #ffa198;
    }
    .diff-line {
        display: flex;
        align-items: baseline;
        padding: 0 12px 0 0;
        white-space: pre;
        min-width: max-content;
    }
    .diff-gutter {
        display: inline-block;
        width: 3.4em;
        padding-right: 0.65em;
        text-align: right;
        color: var(--muted);
        opacity: 0.72;
        user-select: none;
        flex: none;
    }
    .diff-sign {
        display: inline-block;
        width: 1.4em;
        text-align: center;
        user-select: none;
        flex: none;
        font-weight: 600;
    }
    .diff-content {
        flex: 1;
        min-width: 0;
    }
    .diff-meta-line {
        padding-left: 12px;
    }
    .diff-add {
        background: color-mix(in srgb, var(--success) 14%, transparent);
        color: var(--content);
    }
    .diff-add .diff-sign,
    .diff-add .diff-gutter {
        color: var(--success);
        opacity: 0.9;
    }
    .diff-del {
        background: color-mix(in srgb, var(--danger) 14%, transparent);
        color: var(--content);
    }
    .diff-del .diff-sign,
    .diff-del .diff-gutter {
        color: var(--danger);
        opacity: 0.9;
    }
    .diff-hunk {
        background: color-mix(in srgb, var(--info) 14%, transparent);
        color: var(--info);
        font-weight: 600;
    }
    .diff-file {
        color: var(--muted);
        font-weight: 600;
    }
    .diff-code.split {
        padding: 0;
    }
    .diff-split-table {
        width: 100%;
        min-width: 620px;
        border-collapse: collapse;
        table-layout: fixed;
    }
    .diff-split-col-gutter {
        width: 3.6em;
    }
    .diff-split-col-code {
        width: calc(50% - 3.6em);
    }
    .diff-split-meta td {
        padding: 3px 12px;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        border-bottom: 1px solid var(--border);
    }
    .diff-split-gutter {
        padding: 0 0.6em 0 0.4em;
        text-align: right;
        vertical-align: top;
        color: var(--muted);
        opacity: 0.72;
        user-select: none;
        white-space: nowrap;
        border-right: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    }
    .diff-split-cell {
        padding: 0 8px 0 4px;
        vertical-align: top;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        tab-size: 4;
    }
    .diff-split-left {
        border-right: 1px solid var(--border);
    }
    .diff-split-gutter.diff-add,
    .diff-split-cell.diff-add .diff-sign {
        color: var(--success);
        opacity: 0.9;
    }
    .diff-split-gutter.diff-del,
    .diff-split-cell.diff-del .diff-sign {
        color: var(--danger);
        opacity: 0.9;
    }
    .diff-empty {
        background: color-mix(in srgb, var(--panel) 55%, transparent);
    }
    .diff-code :global(.tok-kw) { color: var(--tok-kw); }
    .diff-code :global(.tok-str) { color: var(--tok-str); }
    .diff-code :global(.tok-num) { color: var(--tok-num); }
    .diff-code :global(.tok-cmt) { color: var(--tok-cmt); font-style: italic; }
    .diff-code :global(.tok-type) { color: var(--tok-type); }
    .diff-code :global(.tok-attr) { color: var(--tok-attr); }
    .diff-code :global(.tok-tag) { color: var(--tok-tag); }
    .diff-code :global(.tok-meta) { color: var(--tok-meta); }
    .diff-code :global(.tok-key) { color: var(--tok-key); }
    .diff-code :global(.tok-var) { color: var(--tok-var); }
    .checkpoint-list,
    .worktree-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: grid;
        gap: 8px;
    }
    .checkpoint-item,
    .worktree-item {
        padding: 10px 12px;
        border-radius: 10px;
        border: 1px solid var(--border);
        background: var(--canvas);
        display: grid;
        gap: 4px;
    }
    .worktree-item.current {
        border-color: color-mix(in srgb, var(--info) 45%, var(--border));
    }
    .checkpoint-meta,
    .worktree-heading {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        flex-wrap: wrap;
    }
    .turn-badge,
    .wt-badge {
        display: inline-flex;
        align-items: center;
        padding: 2px 7px;
        border-radius: 999px;
        background: var(--panel);
        border: 1px solid var(--border);
        font-size: 0.72rem;
        font-weight: 600;
    }
    .wt-badge.current {
        background: color-mix(in srgb, var(--info) 16%, transparent);
        border-color: color-mix(in srgb, var(--info) 40%, transparent);
        color: var(--info);
    }
    .sha-code {
        font-size: 0.74rem;
        color: var(--muted);
    }
    .checkpoint-label {
        margin: 0;
        font-size: 0.85rem;
        overflow-wrap: anywhere;
    }
    .worktree-badges {
        display: flex;
        align-items: center;
        gap: 6px;
    }
    .worktree-branch {
        font-size: 0.88rem;
        font-family: var(--font-code);
    }
    .worktree-path {
        font-size: 0.76rem;
        color: var(--muted);
        overflow-wrap: anywhere;
    }
    .worktree-form {
        display: grid;
        gap: 8px;
        padding: 12px;
        border-radius: 10px;
        border: 1px solid var(--border);
        background: var(--canvas);
    }
    .worktree-form label {
        font-weight: 600;
        font-size: 0.82rem;
    }
    .worktree-form-row {
        display: flex;
        gap: 8px;
    }
    .worktree-form-row input {
        flex: 1;
        min-width: 0;
    }
    .worktree-form-row button {
        flex: none;
    }
    .form-error {
        margin: 0;
        color: var(--danger);
        font-size: 0.8rem;
    }
    @media (max-width: 700px) {
        .diff-layout {
            grid-template-columns: minmax(0, 1fr);
        }
        .file-list {
            max-height: 180px;
        }
    }
</style>
