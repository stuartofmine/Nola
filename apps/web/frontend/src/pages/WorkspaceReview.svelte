<script lang="ts">
    import { onMount } from 'svelte';
    import { api, errorMessage } from '../lib/api';
    import { parseInitializationSession } from '../lib/chatInitialization';
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    import { parseWorkspaceReview, type WorkspaceReview } from '../lib/workspaceReview';
    import { listV2WorkspaceCheckpoints, redoV2Workspace, undoV2Workspace, type V2WorkspaceCheckpoint } from '../lib/v2Chat';
    import { locale, t } from '../lib/i18n';
    let {conversationId, hostId, onopen}: {conversationId?: string; hostId?: string; onopen?: () => void} = $props();
    const location = workspacePageLocation();
    const selected = $derived(conversationId ?? location.read().searchParams.get('conversation') ?? '');
    let review = $state<WorkspaceReview | null>(null), loading = $state(false), failed = $state(false);
    let checkpoints = $state<V2WorkspaceCheckpoint[]>([]);
    let checkpointBusy = $state(false), checkpointNotice = $state(''), checkpointError = $state('');
    let mounted = $state(false);
    let request: AbortController | null = null;
    let sequence = 0;
    async function load() {
        request?.abort(); const controller = new AbortController(); request = controller;
        const id = selected, host = hostId, epoch = ++sequence;
        const current = () => mounted && !controller.signal.aborted && epoch === sequence && id === selected && host === hostId;
        review = null; loading = true; failed = false; checkpointNotice = ''; checkpointError = '';
        try {
            if (!id) { checkpoints = []; loading = false; return; }
            const ownerHost = host || parseInitializationSession(await api('/api/web/session', {signal: controller.signal})).hostId;
            const [reviewResult, cpResult] = await Promise.allSettled([
                api(`/api/chat/conversations/${encodeURIComponent(id)}/workspace-review`, {signal: controller.signal}),
                listV2WorkspaceCheckpoints(id, controller.signal),
            ]);
            if (!current()) return;
            if (cpResult.status === 'fulfilled') checkpoints = cpResult.value;
            else checkpoints = [];
            if (reviewResult.status === 'fulfilled') review = parseWorkspaceReview(reviewResult.value, ownerHost, id);
            else failed = true;
        } catch { if (current()) failed = true; }
        finally { if (current()) loading = false; }
    }
    async function runCheckpointAction(action: 'undo' | 'redo') {
        const id = selected;
        if (!id || checkpointBusy || loading) return;
        checkpointBusy = true; checkpointNotice = ''; checkpointError = '';
        try {
            const res = action === 'undo' ? await undoV2Workspace(id) : await redoV2Workspace(id);
            if (!mounted || id !== selected) return;
            if (res.ok) {
                checkpointNotice = res.message || t($locale, action === 'undo' ? 'chatInspector.undo' : 'chatInspector.redo');
                await load();
            } else {
                checkpointError = res.message || t($locale, 'chatInspector.reviewUnavailable');
            }
        } catch (cause) {
            if (mounted && id === selected) checkpointError = errorMessage(cause);
        } finally {
            if (mounted) checkpointBusy = false;
        }
    }
    onMount(() => { mounted = true; return () => { mounted = false; ++sequence; request?.abort(); }; });
    $effect(() => { selected; hostId; if (mounted) void load(); });
    function entryStatusKey(index: string, worktree: string): string {
        const code = `${index}${worktree}`.trim();
        if (code === '??') return 'chatInspector.gitUntracked';
        if (code.includes('U') || code === 'AA' || code === 'DD') return 'chatInspector.gitConflicted';
        if (code.includes('R')) return 'chatInspector.gitRenamed';
        if (code.includes('C')) return 'chatInspector.gitCopied';
        if (code.includes('A')) return 'chatInspector.gitAdded';
        if (code.includes('D')) return 'chatInspector.gitDeleted';
        if (code.includes('M')) return 'chatInspector.gitModified';
        return 'chatInspector.gitChanged';
    }
    function formatFileSize(bytes: number): string {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }
</script>
{#snippet reviewBody()}
    {#if !selected}
        {#if onopen}<p>{t($locale, 'chatPage.choose')}</p>{/if}
        <div class="actions start"><a class="button" href="/conversations">{t($locale, 'shell.conversations')}</a></div>
    {:else}
        <section class="checkpoints-toolbar" aria-label={t($locale, 'chatInspector.checkpoints')}>
            <div class="checkpoints-head">
                <div class="checkpoints-title">
                    <h3>{t($locale, 'chatInspector.checkpoints')}</h3>
                    {#if checkpoints.length}<span class="badge">{checkpoints.length}</span>{/if}
                </div>
                <div class="actions">
                    <button type="button" onclick={() => void runCheckpointAction('undo')} disabled={loading || checkpointBusy || !selected}>{t($locale, 'chatInspector.undo')}</button>
                    <button type="button" onclick={() => void runCheckpointAction('redo')} disabled={loading || checkpointBusy || !selected}>{t($locale, 'chatInspector.redo')}</button>
                </div>
            </div>
            {#if checkpointNotice}<p role="status" class="muted">{checkpointNotice}</p>{/if}
            {#if checkpointError}<p role="alert" class="error">{checkpointError}</p>{/if}
            {#if checkpoints.length}
                <details class="checkpoints-list">
                    <summary>{t($locale, 'chatInspector.checkpoints')} ({checkpoints.length})</summary>
                    <ul>
                        {#each checkpoints as cp (cp.id)}
                            <li class="checkpoint-item">
                                <span class="checkpoint-label">{cp.label}</span>
                                {#if cp.summary}<code class="checkpoint-summary">{cp.summary}</code>{/if}
                                {#if cp.createdAt}<span class="file-meta">{cp.createdAt}</span>{/if}
                            </li>
                        {/each}
                    </ul>
                </details>
            {:else if !loading}
                <p class="muted">{t($locale, 'chatInspector.noCheckpoints')}</p>
            {/if}
        </section>
    {/if}
    {#if loading}<p role="status">{t($locale, 'chatPage.loading')}</p>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, 'chatInspector.reviewUnavailable')}</p>{/if}
    {#if review}
        {#if review.isGitRepository === false}
            <p class="workspace-root">
                <span class="badge workspace-badge">{t($locale, review.isSessionSandbox ? 'chatInspector.temporaryWorkspace' : 'chatInspector.workspace')}</span>
                <code>{review.root}</code>
            </p>
            {#if review.files.length}
                <ul class="workspace-files">
                    {#each review.files as file}
                        <li>
                            <span class="file-path">{file.path}</span>
                            <span class="file-meta">{file.isDir ? t($locale, 'chatInspector.directory') : formatFileSize(file.size)}</span>
                        </li>
                    {/each}
                </ul>
            {:else}
                <p>{t($locale, 'chatInspector.emptyWorkspace')}</p>
            {/if}
            {#if review.truncated}<p role="status">{t($locale, 'chatInspector.filesTruncated')}</p>{/if}
        {:else}
            <p class="workspace-root">{t($locale, 'chatInspector.hostWorkspace')}<code>{review.root}</code></p>
            {#if review.entries.length}<ul>{#each review.entries as entry}<li><span class="badge">{t($locale, entryStatusKey(entry.index, entry.worktree))}</span> <span>{entry.path}</span></li>{/each}</ul>
            {:else}<p>{t($locale, 'chatInspector.noChanges')}</p>{/if}
            {#if review.truncated}<p role="status">{t($locale, 'chatInspector.reviewTruncated')}</p>{/if}
            {#each [{label: 'unstaged', value: review.unstaged}, {label: 'staged', value: review.staged}] as section}
                <details open={!!section.value.text}><summary>{t($locale, `chatInspector.${section.label}`)}</summary>
                    {#if section.value.failed}<p role="alert" class="error">{t($locale, 'chatInspector.diffUnavailable')}</p>
                    {:else if section.value.text}
                        <!-- svelte-ignore a11y_no_noninteractive_tabindex (Scrollable diff supports keyboard reading.) -->
                        <pre tabindex="0" role="region" aria-label={t($locale, `chatInspector.${section.label}`)}>{section.value.text}</pre>
                    {:else}<p>{t($locale, 'chatInspector.noChanges')}</p>{/if}
                </details>
            {/each}
        {/if}
    {/if}
{/snippet}
<section class="workspace-review" class:embedded-review={!!onopen} aria-label={t($locale, 'chatInspector.review')}>
    {#if onopen}
        <header class="review-header"><h2>{t($locale, 'chatInspector.review')}</h2><button type="button" onclick={load} disabled={loading || !selected}>{t($locale, 'chatPage.refresh')}</button></header>
        <div class="actions start"><button type="button" onclick={onopen}>{t($locale, 'chatResource.openTab')}</button></div>
        {@render reviewBody()}
    {:else}
        <header class="page-header">
            <div><h1>{t($locale, 'chatInspector.review')}</h1>{#if !selected}<p class="muted">{t($locale, 'chatPage.choose')}</p>{/if}</div>
            <div class="actions"><button type="button" onclick={load} disabled={loading || !selected}>{t($locale, 'chatPage.refresh')}</button></div>
        </header>
        <div class="content management-content">{@render reviewBody()}</div>
    {/if}
</section>
<style>
    .workspace-review { min-width: 0; }
    .embedded-review { display: grid; gap: 12px; }
    .review-header { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
    .review-header h2 { margin: 0; font-size: 1rem; }
    .embedded-review p { margin: 0; color: var(--muted); font-size: .875rem; line-height: 1.6; }
    .workspace-root code { display: block; overflow-wrap: anywhere; }
    .badge { display: inline-flex; align-items: center; flex: none; padding: 2px 8px; border: 1px solid var(--border); border-radius: 999px; background: var(--surface-subtle, var(--panel)); color: var(--muted); font-size: 12px; font-weight: 500; line-height: 1.4; white-space: nowrap; }
    .workspace-badge { margin-bottom: 4px; }
    .workspace-files li { justify-content: space-between; align-items: baseline; }
    .workspace-files .file-path { flex: 1 1 auto; }
    .workspace-files .file-meta { flex: 0 0 auto; color: var(--muted); font-size: .75rem; white-space: nowrap; }
    .actions.start { justify-content: flex-start; }
    ul { list-style: none; margin: 0; padding: 0; } li { display: flex; align-items: center; gap: 8px; border-bottom: 1px solid var(--border); padding-block: 10px; font-size: .875rem; } li span { overflow-wrap: anywhere; min-width: 0; }
    summary { min-height: 44px; padding-block: 10px; cursor: pointer; font-size: .875rem; }
    pre { max-height: min(60dvh, 600px); overflow: auto; padding: 12px; border: 1px solid var(--border); border-radius: 10px; font: 12px/1.65 var(--font-code, monospace); white-space: pre; }
    .embedded-review button { min-height: var(--control-hit-size); }
    .checkpoints-toolbar { display: grid; gap: 8px; padding: 12px 14px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface); }
    .checkpoints-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 10px; }
    .checkpoints-title { display: flex; align-items: center; gap: 8px; }
    .checkpoints-title h3 { margin: 0; font-size: .9375rem; font-weight: 600; }
    .checkpoint-item { justify-content: space-between; flex-wrap: wrap; }
    .checkpoint-label { flex: 1 1 auto; }
    .checkpoint-summary { font-size: .75rem; color: var(--muted); }
</style>
