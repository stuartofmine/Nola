<script lang="ts">
    import { onDestroy } from 'svelte';
    import { errorMessage } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import {
        createV2WorkspaceWorktree,
        integrateV2WorkspaceWorktree,
        listV2WorkspaceWorktrees,
        removeV2WorkspaceWorktree,
        type V2GitWorktree,
    } from '../lib/v2Chat';

    let {
        compact = false,
        refreshToken = 0,
        onchanged,
    }: {
        compact?: boolean;
        refreshToken?: number;
        onchanged?: () => void;
    } = $props();

    let worktrees = $state<V2GitWorktree[]>([]);
    let loading = $state(false);
    let busyId = $state<string | null>(null);
    let creating = $state(false);
    let branchDraft = $state('');
    let pathDraft = $state('');
    let notice = $state('');
    let error = $state('');
    let disposed = false;
    let controller: AbortController | null = null;

    async function load() {
        controller?.abort();
        controller = new AbortController();
        const signal = controller.signal;
        loading = true;
        error = '';
        try {
            const items = await listV2WorkspaceWorktrees(signal);
            if (!disposed && !signal.aborted) worktrees = items;
        } catch (cause) {
            if (!disposed && !signal.aborted) error = errorMessage(cause);
        } finally {
            if (!disposed && !signal.aborted) loading = false;
        }
    }

    async function handleCreate() {
        if (!branchDraft.trim() || creating) return;
        creating = true;
        error = '';
        notice = '';
        try {
            const res = await createV2WorkspaceWorktree({
                branch: branchDraft,
                path: pathDraft || undefined,
            });
            if (disposed) return;
            notice = res.message || `${t($locale, 'chatInspector.worktreeCreate')}: ${branchDraft.trim()}`;
            branchDraft = '';
            pathDraft = '';
            await load();
            onchanged?.();
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) creating = false;
        }
    }

    function isDetachedWorktree(wt: V2GitWorktree): boolean {
        return !wt.branch || wt.detached === true || wt.branch === '(detached)' || wt.branch === 'detached';
    }

    async function handleIntegrate(wt: V2GitWorktree) {
        if (busyId || isDetachedWorktree(wt)) return;
        busyId = wt.id;
        error = '';
        notice = '';
        try {
            const res = await integrateV2WorkspaceWorktree(wt);
            if (disposed) return;
            notice = res.message || `${t($locale, 'chatInspector.worktreeIntegrate')}: ${wt.branch}`;
            await load();
            onchanged?.();
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) busyId = null;
        }
    }

    async function handleRemove(wt: V2GitWorktree) {
        if (busyId || wt.isMain) return;
        busyId = wt.id;
        error = '';
        notice = '';
        try {
            const res = await removeV2WorkspaceWorktree(wt, false);
            if (disposed) return;
            notice = res.message || `${t($locale, 'chatInspector.worktreeRemove')}: ${wt.branch}`;
            await load();
            onchanged?.();
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) busyId = null;
        }
    }

    $effect(() => {
        refreshToken;
        void load();
    });

    onDestroy(() => {
        disposed = true;
        controller?.abort();
    });
</script>

<section class="worktrees-panel" class:compact aria-label={t($locale, 'chatInspector.worktrees')}>
    <div class="wt-head">
        <h3>{t($locale, 'chatInspector.worktrees')}</h3>
        <button type="button" class="wt-btn" onclick={() => void load()} disabled={loading}>
            {t($locale, 'rooms.refresh')}
        </button>
    </div>

    <form
        class="wt-form"
        onsubmit={event => {
            event.preventDefault();
            void handleCreate();
        }}
    >
        <input
            type="text"
            bind:value={branchDraft}
            placeholder={t($locale, 'chatInspector.worktreeBranchPlaceholder')}
            aria-label={t($locale, 'chatInspector.worktreeBranchPlaceholder')}
            disabled={creating}
        />
        <input
            type="text"
            bind:value={pathDraft}
            placeholder={t($locale, 'chatInspector.worktreePathPlaceholder')}
            aria-label={t($locale, 'chatInspector.worktreePathPlaceholder')}
            disabled={creating}
        />
        <button type="submit" class="primary wt-btn" disabled={creating || !branchDraft.trim()}>
            {t($locale, 'chatInspector.worktreeCreate')}
        </button>
    </form>

    {#if notice}<p class="wt-notice" role="status">{notice}</p>{/if}
    {#if error}<p class="wt-error" role="alert">{error}</p>{/if}

    {#if loading && !worktrees.length}
        <p class="wt-muted">{t($locale, 'rooms.loading')}</p>
    {:else if worktrees.length === 0}
        <p class="wt-muted">{t($locale, 'chatInspector.worktreeEmpty')}</p>
    {:else}
        <ul class="wt-list">
            {#each worktrees as wt (wt.id)}
                <li class="wt-item">
                    <div class="wt-meta">
                        <div class="wt-branch-row">
                            <strong class="wt-branch">{wt.branch}</strong>
                            {#if wt.isMain}<span class="wt-badge">main</span>{/if}
                            {#if wt.dirty}<span class="wt-badge warn">dirty</span>{/if}
                            {#if wt.head}<code class="wt-head-sha">{wt.head.slice(0, 7)}</code>{/if}
                        </div>
                        <code class="wt-path">{wt.path}</code>
                    </div>
                    {#if !wt.isMain}
                        <div class="wt-actions">
                            <button
                                type="button"
                                class="wt-btn"
                                disabled={busyId !== null || isDetachedWorktree(wt)}
                                onclick={() => void handleIntegrate(wt)}
                            >
                                {t($locale, 'chatInspector.worktreeIntegrate')}
                            </button>
                            <button
                                type="button"
                                class="wt-btn danger"
                                disabled={busyId !== null}
                                onclick={() => void handleRemove(wt)}
                            >
                                {t($locale, 'chatInspector.worktreeRemove')}
                            </button>
                        </div>
                    {/if}
                </li>
            {/each}
        </ul>
    {/if}
</section>

<style>
    .worktrees-panel {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .wt-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }
    .wt-head h3 {
        margin: 0;
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        color: var(--muted);
    }
    .wt-form {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .wt-form input {
        width: 100%;
        padding: 6px 8px;
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--canvas);
        color: var(--content);
        font-size: 12px;
    }
    .wt-btn {
        min-height: 28px;
        min-width: 0;
        padding: 4px 10px;
        font-size: 12px;
        border-radius: 7px;
        align-self: flex-start;
    }
    .wt-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .wt-item {
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        font-size: 12px;
    }
    .wt-meta {
        display: flex;
        flex-direction: column;
        gap: 3px;
        min-width: 0;
    }
    .wt-branch-row {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 6px;
    }
    .wt-branch {
        font-family: var(--font-code, monospace);
        font-size: 12px;
    }
    .wt-badge {
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        font-size: 10px;
        color: var(--muted);
    }
    .wt-badge.warn {
        border-color: var(--warning);
        color: var(--warning);
    }
    .wt-head-sha,
    .wt-path {
        font-size: 11px;
        color: var(--muted);
        overflow-wrap: anywhere;
    }
    .wt-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .wt-muted {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .wt-notice {
        margin: 0;
        padding: 6px 8px;
        border: 1px solid var(--border);
        border-radius: 6px;
        background: var(--canvas);
        font-size: 12px;
    }
    .wt-error {
        margin: 0;
        padding: 6px 8px;
        border: 1px solid var(--danger);
        border-radius: 6px;
        color: var(--danger);
        font-size: 12px;
    }
</style>
