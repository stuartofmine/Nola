<script lang="ts">
    import { onDestroy } from 'svelte';
    import { api, errorMessage } from '../lib/api';
    import {
        conversationPdfUrl,
        conversationResourcesUrl,
        parseConversationPdfResources,
        type ConversationPdfResource,
    } from '../lib/chatResources';
    import { locale, t } from '../lib/i18n';
    import {
        getV2BrowserState,
        type V2BrowserState,
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

    let browserState = $state<V2BrowserState>({
        managedAvailable: false,
        managedConnected: false,
        nativeConnected: false,
        pages: [],
    });
    let pdfResources = $state<ConversationPdfResource[]>([]);
    let loading = $state(false);
    let error = $state('');
    let previewExpanded = $state(false);
    let disposed = false;
    let controller: AbortController | null = null;

    // Extract browser URLs & parsed PDF/document attachments observed in the transcript
    const transcriptBrowserPages = $derived.by(() => {
        const pages: Array<{ id: string; title: string; url: string }> = [];
        const seen = new Set<string>();
        for (const msg of messages) {
            for (const tc of msg.toolCalls ?? []) {
                if (!/browser|web_fetch|web_search|navigate|page/i.test(tc.name)) continue;
                const args = tc.arguments && typeof tc.arguments === 'object' ? (tc.arguments as Record<string, unknown>) : null;
                const url = typeof args?.url === 'string' ? args.url : typeof args?.uri === 'string' ? args.uri : '';
                if (url && !seen.has(url)) {
                    seen.add(url);
                    pages.push({ id: tc.callId, title: tc.name, url });
                }
            }
        }
        return pages;
    });

    const parsedAttachments = $derived.by(() => {
        const items: Array<{ id: string; name: string; mimeType: string; text?: string; dataUrl?: string }> = [];
        for (const msg of messages) {
            for (const att of msg.attachments ?? []) {
                if (
                    att.mimeType === 'application/pdf' ||
                    att.name.toLowerCase().endsWith('.pdf') ||
                    Boolean(att.text)
                ) {
                    items.push({
                        id: `${msg.id}:${att.name}`,
                        name: att.name,
                        mimeType: att.mimeType,
                        text: att.text,
                        dataUrl: att.dataUrl,
                    });
                }
            }
        }
        return items;
    });

    const mergedPages = $derived.by(() => {
        const list = [...browserState.pages];
        const seen = new Set(list.map(p => p.url));
        for (const tp of transcriptBrowserPages) {
            if (!seen.has(tp.url)) list.push(tp);
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
            const [browserRes, pdfRes] = await Promise.allSettled([
                getV2BrowserState(signal),
                api(conversationResourcesUrl(id), { signal }).then(raw => parseConversationPdfResources(raw, id)),
            ]);
            if (disposed || signal.aborted || conversationId !== id) return;
            if (browserRes.status === 'fulfilled') browserState = browserRes.value;
            if (pdfRes.status === 'fulfilled') pdfResources = pdfRes.value;
            else pdfResources = [];
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

    function formatSize(bytes: number): string {
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

<div class="browser-pdf-panel">
    <section class="bp-section" aria-label={t($locale, 'chatBrowser.title')}>
        <div class="bp-head">
            <h3>{t($locale, 'chatBrowser.title')}</h3>
            <button type="button" class="bp-btn" onclick={() => void load(conversationId)} disabled={loading}>
                {t($locale, 'chatBrowser.refresh')}
            </button>
        </div>
        {#if error}<p class="bp-error" role="alert">{error}</p>{/if}
        <div class="bp-status-grid">
            <div class="bp-stat">
                <span>{t($locale, 'chatBrowser.managed')}</span>
                <strong class:connected={browserState.managedConnected}>
                    {browserState.managedConnected
                        ? t($locale, 'chatBrowser.active')
                        : browserState.managedAvailable
                          ? t($locale, 'chatBrowser.available')
                          : t($locale, 'chatBrowser.disconnected')}
                </strong>
            </div>
            <div class="bp-stat">
                <span>{t($locale, 'chatBrowser.native')}</span>
                <strong class:connected={browserState.nativeConnected}>
                    {browserState.nativeConnected
                        ? t($locale, 'chatBrowser.connected')
                        : t($locale, 'chatBrowser.disconnected')}
                </strong>
            </div>
        </div>

        {#if browserState.previewUrl}
            <div class="preview-box">
                <div class="bp-head">
                    <span class="bp-muted">{t($locale, 'chatBrowser.previewTitle')}</span>
                    <button type="button" class="bp-btn" onclick={() => (previewExpanded = !previewExpanded)}>
                        {t($locale, previewExpanded ? 'chatBrowser.collapse' : 'chatBrowser.expand')}
                    </button>
                </div>
                <iframe
                    class="browser-iframe"
                    class:expanded={previewExpanded}
                    title={t($locale, 'chatBrowser.previewTitle')}
                    src={browserState.previewUrl}
                    sandbox="allow-scripts allow-same-origin"
                    referrerpolicy="no-referrer"
                ></iframe>
            </div>
        {/if}

        <div class="bp-sub">
            <span class="bp-sub-label">{t($locale, 'chatBrowser.pages')}</span>
            {#if mergedPages.length === 0}
                <p class="bp-muted">{t($locale, 'chatBrowser.noPages')}</p>
            {:else}
                <ul class="bp-list">
                    {#each mergedPages as page (page.id)}
                        <li class="bp-item">
                            <strong>{page.title || t($locale, 'chatBrowser.untitled')}</strong>
                            {#if page.url}
                                <a href={page.url} target="_blank" rel="noopener noreferrer">{page.url}</a>
                            {/if}
                        </li>
                    {/each}
                </ul>
            {/if}
        </div>
    </section>

    <section class="bp-section" aria-label={t($locale, 'chatResource.title')}>
        <h3>{t($locale, 'chatResource.title')}</h3>
        {#if pdfResources.length === 0 && parsedAttachments.length === 0}
            <p class="bp-muted">{t($locale, 'chatResource.empty')}</p>
        {:else}
            {#if pdfResources.length > 0}
                <ul class="bp-list">
                    {#each pdfResources as pdf (pdf.resourceId)}
                        {@const url = conversationPdfUrl(conversationId, pdf.resourceId)}
                        <li class="bp-item">
                            <div class="bp-head">
                                <strong>{pdf.displayName}</strong>
                                <span class="bp-muted">{formatSize(pdf.byteCount)}</span>
                            </div>
                            <div class="bp-actions">
                                <button
                                    type="button"
                                    class="bp-btn"
                                    onclick={() =>
                                        onpreview?.({
                                            title: pdf.displayName,
                                            subtitle: formatSize(pdf.byteCount),
                                            kind: 'pdf',
                                            mimeType: 'application/pdf',
                                            pdfUrl: url,
                                        })}
                                >
                                    {t($locale, 'chatResource.title')}
                                </button>
                                <a class="bp-link-btn" href={url} download={pdf.displayName}>
                                    {t($locale, 'chatResource.download')}
                                </a>
                            </div>
                        </li>
                    {/each}
                </ul>
            {/if}

            {#if parsedAttachments.length > 0}
                <ul class="bp-list">
                    {#each parsedAttachments as att (att.id)}
                        <li class="bp-item">
                            <div class="bp-head">
                                <strong>{att.name}</strong>
                                <span class="bp-badge">{att.mimeType}</span>
                            </div>
                            {#if att.text}
                                <pre class="att-preview">{att.text.slice(0, 220)}</pre>
                                <div class="bp-actions">
                                    <button
                                        type="button"
                                        class="bp-btn"
                                        onclick={() =>
                                            onpreview?.({
                                                title: att.name,
                                                subtitle: att.mimeType,
                                                kind: 'text',
                                                mimeType: att.mimeType,
                                                text: att.text ?? '',
                                            })}
                                    >
                                        {t($locale, 'chatPage.openTextReader')}
                                    </button>
                                </div>
                            {/if}
                        </li>
                    {/each}
                </ul>
            {/if}
        {/if}
    </section>
</div>

<style>
    .browser-pdf-panel {
        display: flex;
        flex-direction: column;
        gap: 16px;
    }
    .bp-section {
        display: flex;
        flex-direction: column;
        gap: 8px;
    }
    .bp-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
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
    .bp-status-grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 8px;
    }
    .bp-stat {
        display: flex;
        flex-direction: column;
        gap: 2px;
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        font-size: 12px;
    }
    .bp-stat span {
        font-size: 11px;
        color: var(--muted);
    }
    .bp-stat strong.connected {
        color: var(--success);
    }
    .preview-box {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .browser-iframe {
        width: 100%;
        height: 180px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
    }
    .browser-iframe.expanded {
        height: 360px;
    }
    .bp-sub {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .bp-sub-label {
        font-size: 11px;
        font-weight: 600;
        color: var(--muted);
    }
    .bp-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .bp-item {
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        font-size: 12px;
    }
    .bp-item a {
        font-size: 11.5px;
        overflow-wrap: anywhere;
    }
    .bp-actions {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
        margin-top: 2px;
    }
    .bp-btn,
    .bp-link-btn {
        display: inline-flex;
        align-items: center;
        min-height: 28px;
        padding: 3px 9px;
        font-size: 11.5px;
        border-radius: 6px;
        border: 1px solid var(--border);
        background: var(--panel);
        color: var(--content);
        text-decoration: none;
        cursor: pointer;
    }
    .bp-badge {
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        font-size: 10px;
        color: var(--muted);
    }
    .att-preview {
        margin: 0;
        padding: 6px 8px;
        border-radius: 6px;
        background: var(--panel);
        border: 1px solid var(--border);
        font-size: 11px;
        max-height: 90px;
        overflow: hidden;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        color: var(--muted);
    }
    .bp-muted {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .bp-error {
        margin: 0;
        padding: 6px 8px;
        border: 1px solid var(--danger);
        border-radius: 6px;
        color: var(--danger);
        font-size: 12px;
    }
</style>
