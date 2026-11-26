<script lang="ts">
    import { onDestroy } from 'svelte';
    import { errorMessage } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import {
        searchV2ConversationRecall,
        searchV2Documents,
        type V2DocumentSearchHit,
        type V2RecallHit,
    } from '../lib/v2Chat';
    import ChatTextReaderDialog, { type TextReaderSnapshot } from './ChatTextReaderDialog.svelte';

    let { conversationId }: { conversationId: string } = $props();

    // Section 1: Conversation Recall
    let recallQuery = $state('');
    let recallHits = $state<V2RecallHit[]>([]);
    let recallLoading = $state(false);
    let recallSearched = $state(false);
    let recallError = $state('');

    // Section 2: Indexed Documents & Memory Search
    let docsQuery = $state('');
    let docHits = $state<V2DocumentSearchHit[]>([]);
    let docsLoading = $state(false);
    let docsSearched = $state(false);
    let docsError = $state('');

    let readerSnapshot = $state<TextReaderSnapshot | null>(null);
    let disposed = false;
    let recallController: AbortController | null = null;
    let docsController: AbortController | null = null;

    $effect(() => {
        conversationId;
        recallController?.abort();
        recallHits = [];
        recallSearched = false;
        recallError = '';
    });

    onDestroy(() => {
        disposed = true;
        recallController?.abort();
        docsController?.abort();
    });

    async function runRecallSearch() {
        const q = recallQuery.trim();
        if (!q || !conversationId) return;
        recallController?.abort();
        recallController = new AbortController();
        const signal = recallController.signal;
        const id = conversationId;
        recallLoading = true;
        recallError = '';
        recallSearched = false;
        try {
            const results = await searchV2ConversationRecall(id, q, signal);
            if (!disposed && !signal.aborted && conversationId === id) {
                recallHits = results;
                recallSearched = true;
            }
        } catch (cause) {
            if (!disposed && !signal.aborted && conversationId === id) {
                recallError = errorMessage(cause);
            }
        } finally {
            if (!disposed && !signal.aborted && conversationId === id) {
                recallLoading = false;
            }
        }
    }

    async function runDocsSearch() {
        const q = docsQuery.trim();
        if (!q) return;
        docsController?.abort();
        docsController = new AbortController();
        const signal = docsController.signal;
        docsLoading = true;
        docsError = '';
        docsSearched = false;
        try {
            const results = await searchV2Documents(q, signal);
            if (!disposed && !signal.aborted) {
                docHits = results;
                docsSearched = true;
            }
        } catch (cause) {
            if (!disposed && !signal.aborted) {
                docsError = errorMessage(cause);
            }
        } finally {
            if (!disposed && !signal.aborted) {
                docsLoading = false;
            }
        }
    }

    function jumpToMessage(messageId: string) {
        if (typeof document === 'undefined' || !messageId) return;
        const escaped =
            typeof CSS !== 'undefined' && typeof CSS.escape === 'function'
                ? CSS.escape(messageId)
                : messageId.replace(/"/g, '\\"');
        const target =
            document.querySelector<HTMLElement>(`[data-message-id="${escaped}"]`) ??
            document.getElementById(`message-${messageId}`);
        if (!target) return;
        target.scrollIntoView({
            behavior: window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth',
            block: 'center',
        });
        target.animate?.(
            [{ backgroundColor: 'var(--accent-subtle)' }, { backgroundColor: 'transparent' }],
            { duration: 1500, easing: 'ease-out' },
        );
    }
</script>

<div class="recall-panel">
    <section class="search-section" aria-label={t($locale, 'chatKnowledge.recall')}>
        <h3>{t($locale, 'chatKnowledge.recall')}</h3>
        <p class="search-muted">{t($locale, 'chatKnowledge.recallHint')}</p>
        <form
            class="search-form"
            onsubmit={e => {
                e.preventDefault();
                void runRecallSearch();
            }}
        >
            <input
                type="search"
                bind:value={recallQuery}
                placeholder={t($locale, 'chatKnowledge.search')}
                aria-label={t($locale, 'chatKnowledge.search')}
                maxlength="500"
            />
            <button type="submit" class="primary search-btn" disabled={recallLoading || !recallQuery.trim()}>
                {t($locale, 'chatKnowledge.search')}
            </button>
        </form>
        {#if recallLoading}<p class="search-muted" role="status">{t($locale, 'chatKnowledge.searching')}</p>{/if}
        {#if recallError}<p class="search-error" role="alert">{recallError}</p>{/if}
        {#if recallSearched && !recallHits.length}
            <p class="search-muted" role="status">{t($locale, 'chatKnowledge.noMatches')}</p>
        {/if}
        {#if recallHits.length}
            <ol class="hit-list">
                {#each recallHits as hit (hit.messageId + ':' + (hit.ordinal ?? 0))}
                    <li>
                        <button type="button" class="hit-card" onclick={() => jumpToMessage(hit.messageId)}>
                            <small>#{hit.ordinal ?? '—'} · {hit.role}{hit.timestamp ? ` · ${hit.timestamp}` : ''}</small>
                            <span class="hit-snippet">{hit.snippet}</span>
                        </button>
                    </li>
                {/each}
            </ol>
        {/if}
    </section>

    <section class="search-section" aria-label={t($locale, 'chatInspector.docsSearch')}>
        <h3>{t($locale, 'chatInspector.docsSearch')}</h3>
        <form
            class="search-form"
            onsubmit={e => {
                e.preventDefault();
                void runDocsSearch();
            }}
        >
            <input
                type="search"
                bind:value={docsQuery}
                placeholder={t($locale, 'chatInspector.docsSearchPlaceholder')}
                aria-label={t($locale, 'chatInspector.docsSearchPlaceholder')}
                maxlength="500"
            />
            <button type="submit" class="primary search-btn" disabled={docsLoading || !docsQuery.trim()}>
                {t($locale, 'chatKnowledge.search')}
            </button>
        </form>
        {#if docsLoading}<p class="search-muted" role="status">{t($locale, 'chatKnowledge.searching')}</p>{/if}
        {#if docsError}<p class="search-error" role="alert">{docsError}</p>{/if}
        {#if docsSearched && !docHits.length}
            <p class="search-muted" role="status">{t($locale, 'chatKnowledge.noMatches')}</p>
        {/if}
        {#if docHits.length}
            <ul class="hit-list">
                {#each docHits as hit (hit.kind + ':' + hit.id)}
                    <li class="doc-hit-card">
                        <div class="doc-hit-head">
                            <strong>{hit.title}</strong>
                            <span class="doc-badge">{hit.kind}</span>
                        </div>
                        {#if hit.path && hit.path !== hit.title}
                            <code class="doc-path">{hit.path}</code>
                        {/if}
                        {#if hit.snippet}
                            <p class="hit-snippet">{hit.snippet.slice(0, 260)}</p>
                            {#if hit.snippet.length > 200}
                                <button
                                    type="button"
                                    class="reader-btn"
                                    onclick={() =>
                                        (readerSnapshot = {
                                            title: hit.title,
                                            subtitle: hit.path || hit.kind,
                                            text: hit.snippet ?? '',
                                        })}
                                >
                                    {t($locale, 'chatPage.openTextReader')}
                                </button>
                            {/if}
                        {/if}
                    </li>
                {/each}
            </ul>
        {/if}
    </section>
</div>

{#if readerSnapshot}
    <ChatTextReaderDialog snapshot={readerSnapshot} onclose={() => (readerSnapshot = null)} />
{/if}

<style>
    .recall-panel {
        display: flex;
        flex-direction: column;
        gap: 16px;
    }
    .search-section {
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
    .search-form {
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .search-form input {
        width: 100%;
        padding: 7px 9px;
        border: 1px solid var(--border);
        border-radius: 7px;
        background: var(--canvas);
        color: var(--content);
        font-size: 12px;
    }
    .search-btn {
        min-height: 30px;
        padding: 4px 10px;
        font-size: 12px;
        border-radius: 7px;
        align-self: flex-start;
    }
    .hit-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-direction: column;
        gap: 6px;
    }
    .hit-card {
        display: flex;
        flex-direction: column;
        gap: 4px;
        width: 100%;
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        color: var(--content);
        text-align: start;
        cursor: pointer;
    }
    .hit-card:hover {
        border-color: var(--primary);
    }
    .hit-card small {
        font-size: 11px;
        color: var(--muted);
    }
    .doc-hit-card {
        display: flex;
        flex-direction: column;
        gap: 4px;
        padding: 8px 10px;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--canvas);
        font-size: 12px;
    }
    .doc-hit-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 6px;
    }
    .doc-badge {
        padding: 1px 6px;
        border-radius: 999px;
        border: 1px solid var(--border);
        font-size: 10px;
        color: var(--muted);
    }
    .doc-path {
        font-size: 11px;
        color: var(--muted);
        overflow-wrap: anywhere;
    }
    .hit-snippet {
        margin: 0;
        font-size: 12px;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
    }
    .reader-btn {
        align-self: flex-start;
        min-height: 26px;
        padding: 2px 8px;
        font-size: 11px;
        border-radius: 6px;
    }
    .search-muted {
        margin: 0;
        font-size: 12px;
        color: var(--muted);
    }
    .search-error {
        margin: 0;
        padding: 6px 8px;
        border-radius: 6px;
        border: 1px solid var(--danger);
        color: var(--danger);
        font-size: 12px;
    }
</style>
