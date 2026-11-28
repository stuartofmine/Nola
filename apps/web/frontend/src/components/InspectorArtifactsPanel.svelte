<script lang="ts">
    import { onDestroy } from 'svelte';
    import { errorMessage } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import {
        listV2ConversationArtifacts,
        type V2Artifact,
        type V2Message,
    } from '../lib/v2Chat';
    import type { InspectorPreviewItem } from '../lib/inspectorPreview';

    let {
        conversationId,
        messages,
        refreshToken = 0,
        onpreview,
    }: {
        conversationId: string;
        messages: V2Message[];
        refreshToken?: number;
        onpreview?: (item: InspectorPreviewItem) => void;
    } = $props();

    let serverArtifacts = $state<V2Artifact[]>([]);
    let loading = $state(false);
    let error = $state('');
    let copiedId = $state<string | null>(null);
    let disposed = false;
    let controller: AbortController | null = null;

    // Combine persisted server artifacts with attachment/tool outputs observed in the transcript
    const allArtifacts = $derived.by(() => {
        const list: V2Artifact[] = [...serverArtifacts];
        const seen = new Set(list.map(a => `${a.title}:${a.path ?? a.id}`));
        for (const msg of messages) {
            for (const att of msg.attachments ?? []) {
                const key = `${att.name}:att`;
                if (seen.has(key)) continue;
                seen.add(key);
                list.push({
                    id: `att-${msg.id}-${att.name}`,
                    title: att.name,
                    kind: att.mimeType.startsWith('image/') ? 'image' : 'attachment',
                    mimeType: att.mimeType,
                    content: att.text,
                    preview: att.dataUrl || att.text?.slice(0, 200),
                    messageId: msg.id,
                });
            }
            for (const tc of msg.toolCalls ?? []) {
                if (tc.status !== 'completed' || !tc.summary) continue;
                const args = tc.arguments && typeof tc.arguments === 'object' ? (tc.arguments as Record<string, unknown>) : null;
                const filePath = typeof args?.path === 'string'
                    ? args.path
                    : typeof args?.file_path === 'string'
                      ? args.file_path
                      : undefined;
                if (filePath || tc.summary.length > 240) {
                    const title = filePath || tc.name;
                    const key = `${title}:${tc.callId}`;
                    if (seen.has(key)) continue;
                    seen.add(key);
                    list.push({
                        id: `tc-${tc.callId}`,
                        title,
                        kind: filePath ? 'file' : 'output',
                        path: filePath,
                        content: tc.summary,
                        preview: tc.summary.slice(0, 220),
                        messageId: msg.id,
                    });
                }
            }
        }
        return list;
    });

    async function load(id: string) {
        controller?.abort();
        controller = new AbortController();
        const signal = controller.signal;
        loading = true;
        error = '';
        try {
            const items = await listV2ConversationArtifacts(id, signal);
            if (!disposed && !signal.aborted && conversationId === id) {
                serverArtifacts = items;
            }
        } catch (cause) {
            if (!disposed && !signal.aborted && conversationId === id) {
                error = errorMessage(cause);
            }
        } finally {
            if (!disposed && !signal.aborted && conversationId === id) {
                loading = false;
            }
        }
    }

    async function copyArtifact(art: V2Artifact) {
        const text = art.content || art.preview || art.path || art.title;
        try {
            await navigator.clipboard?.writeText(text);
            copiedId = art.id;
            setTimeout(() => {
                if (copiedId === art.id) copiedId = null;
            }, 1600);
        } catch {
            /* Clipboard unavailable */
        }
    }

    function openPreview(art: V2Artifact) {
        const isImage = art.kind === 'image' && Boolean(art.preview?.startsWith('data:image/'));
        onpreview?.({
            title: art.title,
            subtitle: [art.kind, art.path, art.mimeType].filter(Boolean).join(' · '),
            kind: isImage ? 'image' : art.kind === 'file' ? 'file' : 'text',
            mimeType: art.mimeType,
            path: art.path,
            imageUrl: isImage ? art.preview : undefined,
            text: isImage ? undefined : art.content || art.preview || art.path || '',
        });
    }

    function formatSize(bytes?: number): string {
        if (!bytes) return '';
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }

    $effect(() => {
        const id = conversationId;
        refreshToken;
        if (id) void load(id);
    });

    onDestroy(() => {
        disposed = true;
        controller?.abort();
    });
</script>

<section class="artifacts-panel" aria-label={t($locale, 'chatInspector.artifacts')}>
    <h3>{t($locale, 'chatInspector.tabArtifacts')}</h3>
    {#if error}<p class="art-error" role="alert">{error}</p>{/if}
    {#if loading && !allArtifacts.length}
        <p class="art-muted">{t($locale, 'rooms.loading')}</p>
    {:else if allArtifacts.length === 0}
        <p class="art-muted">{t($locale, 'chatInspector.noArtifacts')}</p>
    {:else}
        <ul class="art-list">
            {#each allArtifacts as art (art.id)}
                <li class="art-card">
                    <div class="art-head">
                        <strong class="art-title" title={art.path || art.title}>{art.title}</strong>
                        <span class="art-badge">{art.kind}</span>
                    </div>
                    {#if art.path || art.size || art.createdAt}
                        <div class="art-meta">
                            {#if art.path && art.path !== art.title}<code>{art.path}</code>{/if}
                            {#if art.size}<span>{formatSize(art.size)}</span>{/if}
                            {#if art.createdAt}<span>{art.createdAt}</span>{/if}
                        </div>
                    {/if}
                    {#if art.kind === 'image' && art.preview?.startsWith('data:image/')}
                        <button type="button" class="art-thumb-btn" onclick={() => openPreview(art)} title={art.title}>
                            <img class="art-thumb" src={art.preview} alt={art.title} />
                        </button>
                    {:else if art.preview || art.content}
                        <pre class="art-preview">{(art.preview || art.content || '').slice(0, 240)}</pre>
                    {/if}
                    <div class="art-actions">
                        <button type="button" class="art-btn" onclick={() => void copyArtifact(art)}>
                            {copiedId === art.id ? t($locale, 'chatInspector.copied') : t($locale, 'chatInspector.copy')}
                        </button>
                        {#if art.content || art.preview}
                            <button type="button" class="art-btn" onclick={() => openPreview(art)}>
                                {t($locale, 'chatPage.openTextReader')}
                            </button>
                        {/if}
                    </div>
                </li>
            {/each}
        </ul>
    {/if}
</section>

<style>
    .artifacts-panel {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    h3 {
        margin: 0;
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        color: var(--muted);
    }
    .art-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .art-card {
        display: flex;
        flex-direction: column;
        gap: 6px;
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        font-size: 12px;
    }
    .art-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
    }
    .art-title {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: 12px;
    }
    .art-badge {
        flex: none;
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        font-size: 10px;
        color: var(--muted);
    }
    .art-meta {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        font-size: 11px;
        color: var(--muted);
    }
    .art-meta code {
        overflow-wrap: anywhere;
    }
    .art-thumb-btn {
        display: block;
        width: 100%;
        padding: 0;
        border: 0;
        background: transparent;
        cursor: pointer;
    }
    .art-thumb {
        width: 100%;
        max-height: 150px;
        object-fit: contain;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--panel);
    }
    .art-preview {
        margin: 0;
        padding: 6px 8px;
        border-radius: 6px;
        background: var(--panel);
        border: 1px solid var(--border);
        font-size: 11px;
        max-height: 96px;
        overflow: hidden;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        color: var(--muted);
    }
    .art-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
    }
    .art-btn {
        min-height: 28px;
        min-width: 0;
        padding: 3px 9px;
        font-size: 11.5px;
        border-radius: 6px;
    }
    .art-muted {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .art-error {
        margin: 0;
        padding: 6px 8px;
        border-radius: 6px;
        border: 1px solid var(--danger);
        color: var(--danger);
        font-size: 12px;
    }
</style>
