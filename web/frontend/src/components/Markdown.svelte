<script lang="ts">
    import { untrack, setContext, tick, onDestroy } from 'svelte';
    import { STREAMING_TEXT_SELECTION, captureTextSelection } from '../lib/streamingText';
    import StreamingText from './StreamingText.svelte';
    import { parseMarkdown } from '../lib/markdown';
    import type { InlineNode, MarkdownBlock } from '../lib/markdown';
    import { locale, t } from '../lib/i18n';
    let { source, streaming = false, initialText = '' }: { source: string; streaming?: boolean; initialText?: string } = $props();
    let container: HTMLDivElement;
    let selectionQueued = false;
    function preserveSelection() {
        if (selectionQueued || !container) return;
        const restore = captureTextSelection(container);
        if (!restore) return;
        selectionQueued = true;
        void tick().then(() => { selectionQueued = false; restore(); });
    }
    setContext(STREAMING_TEXT_SELECTION, preserveSelection);
    let wasStreaming = untrack(() => streaming);
    $effect.pre(() => {
        source;
        if (streaming || wasStreaming) preserveSelection();
        wasStreaming = streaming;
    });
    const content = $derived(parseMarkdown(source));
    function indexText(blocks: MarkdownBlock[]) {
        const offsets = new Map<object, number>();
        let text = '';
        function inlines(nodes: InlineNode[]) {
            for (const node of nodes) {
                offsets.set(node, text.length);
                if (node.kind === 'text' || node.kind === 'code') text += node.text;
                else if ('children' in node) inlines(node.children);
            }
        }
        function visit(nodes: MarkdownBlock[]) {
            for (const node of nodes) {
                offsets.set(node, text.length);
                if (node.kind === 'code') text += node.text;
                else if (node.kind === 'paragraph' || node.kind === 'heading') inlines(node.children);
                else if (node.kind === 'quote') visit(node.children);
                else if (node.kind === 'list') for (const item of node.items) visit(item.children);
                else if (node.kind === 'table') {
                    for (const cell of node.header) inlines(cell);
                    for (const row of node.rows) for (const cell of row) inlines(cell);
                }
            }
        }
        visit(blocks);
        return { text, offsets };
    }
    const indexed = $derived(streaming ? indexText(content) : null);
    let previous = untrack(() => streaming ? indexText(parseMarkdown(initialText)).text : '');
    let visiblePrefix = $state(untrack(() => previous.length));
    $effect.pre(() => {
        const next = indexed?.text ?? '';
        visiblePrefix = streaming && next.startsWith(previous) ? previous.length : next.length;
        previous = next;
    });
    function prefix(node: object) { return visiblePrefix - (indexed?.offsets.get(node) ?? 0); }
    let copyNotice = $state('');
    let copyGeneration = 0;
    let copyTimer: ReturnType<typeof setTimeout> | undefined;
    $effect.pre(() => {
        source;
        copyGeneration++; clearTimeout(copyTimer); copyNotice = '';
    });
    onDestroy(() => { copyGeneration++; clearTimeout(copyTimer); });
    async function copy(text: string) {
        const generation = ++copyGeneration, captured = source;
        clearTimeout(copyTimer); copyNotice = '';
        const current = () => generation === copyGeneration && captured === source;
        try {
            await navigator.clipboard.writeText(text);
            if (current()) {
                copyNotice = 'shell.markdown.copied';
                copyTimer = setTimeout(() => { if (current()) copyNotice = ''; }, 1800);
            }
        }
        catch { if (current()) copyNotice = 'shell.markdown.copy_failed'; }
    }
</script>

{#snippet inlines(nodes: InlineNode[])}
    {#each nodes as node}
        {#if node.kind === 'text'}{#if streaming}<StreamingText text={node.text} {streaming} visiblePrefix={prefix(node)} />{:else}{node.text}{/if}
        {:else if node.kind === 'code'}<code>{#if streaming}<StreamingText text={node.text} {streaming} visiblePrefix={prefix(node)} />{:else}{node.text}{/if}</code>
        {:else if node.kind === 'strong'}<strong>{@render inlines(node.children)}</strong>
        {:else if node.kind === 'em'}<em>{@render inlines(node.children)}</em>
        {:else if node.kind === 'del'}<del>{@render inlines(node.children)}</del>
        {:else if node.kind === 'br'}<br />
        {:else if node.kind === 'link'}
            {#if node.href}<a href={node.href} title={node.title} target="_blank" rel="noopener noreferrer">{@render inlines(node.children)}</a>
            {:else}{@render inlines(node.children)}{/if}
        {:else if node.kind === 'image'}
            {#if node.href}<a href={node.href} target="_blank" rel="noopener noreferrer">{t($locale, 'shell.markdown.image', { label: node.text || t($locale, 'shell.markdown.view_image') })}</a>
            {:else}{node.text}{/if}
        {/if}
    {/each}
{/snippet}

{#snippet renderBlocks(nodes: MarkdownBlock[])}
    {#each nodes as node}
        {#if node.kind === 'paragraph'}<p>{@render inlines(node.children)}</p>
        {:else if node.kind === 'heading'}<svelte:element this={`h${Math.min(6, Math.max(1, node.depth))}`}>{@render inlines(node.children)}</svelte:element>
        {:else if node.kind === 'hr'}<hr />
        {:else if node.kind === 'quote'}<blockquote>{@render renderBlocks(node.children)}</blockquote>
        {:else if node.kind === 'code'}
            <div class="code-block"><div class="code-header"><span>{node.language || t($locale, 'shell.markdown.code')}</span><button type="button" onclick={() => copy(node.text)}>{t($locale, 'shell.markdown.copy')}</button></div>
                <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard users need to focus this overflow region to scroll code.) -->
                <pre tabindex="0" role="region" aria-label={t($locale, 'shell.markdown.code')}><code>{#if streaming}<StreamingText text={node.text} {streaming} visiblePrefix={prefix(node)} />{:else}{node.text}{/if}</code></pre>
            </div>
        {:else if node.kind === 'list'}
            <svelte:element this={node.ordered ? 'ol' : 'ul'} start={node.ordered ? node.start : undefined}>
                {#each node.items as item}<li class:task={item.checked !== undefined}>
                    {#if item.checked !== undefined}<input type="checkbox" checked={item.checked} disabled aria-label={t($locale, item.checked ? 'shell.markdown.completed' : 'shell.markdown.incomplete')} />{/if}
                    <div>{@render renderBlocks(item.children)}</div>
                </li>{/each}
            </svelte:element>
        {:else if node.kind === 'table'}
            <!-- svelte-ignore a11y_no_noninteractive_tabindex (Keyboard users need to focus this overflow region to scroll wide tables.) -->
            <div class="table-scroll" tabindex="0" role="region" aria-label={t($locale, 'shell.markdown.table')}><table>
                <thead><tr>{#each node.header as cell, index}<th scope="col" style:text-align={node.align[index] ?? 'left'}>{@render inlines(cell)}</th>{/each}</tr></thead>
                <tbody>{#each node.rows as row}<tr>{#each row as cell, index}<td style:text-align={node.align[index] ?? 'left'}>{@render inlines(cell)}</td>{/each}</tr>{/each}</tbody>
            </table></div>
        {/if}
    {/each}
{/snippet}

<div class="markdown" bind:this={container}>{@render renderBlocks(content)}{#if copyNotice}<p class="copy-notice" role="status">{t($locale, copyNotice)}</p>{/if}</div>

<style>
    .markdown { min-width: 0; line-height: 1.5625; overflow-wrap: anywhere; }
    .markdown > :global(:first-child) { margin-top: 0; }
    .markdown > :global(:last-child) { margin-bottom: 0; }
    .markdown :global(p) { margin: 12px 0; }
    .markdown :global(h1), .markdown :global(h2), .markdown :global(h3), .markdown :global(h4), .markdown :global(h5), .markdown :global(h6) { font-weight: 650; line-height: 1.35; margin: 1.2em 0 .5em; }
    .markdown :global(h1) { font-size: 1.5em; } .markdown :global(h2) { font-size: 1.3em; } .markdown :global(h3) { font-size: 1.15em; }
    .markdown :global(a) { color: var(--accent); text-decoration: underline; text-underline-offset: 3px; }
    .markdown :global(ul), .markdown :global(ol) { padding-inline-start: 1.65em; margin: .65em 0; }
    .markdown :global(li) { margin: .3em 0; padding-inline-start: .15em; }
    .markdown :global(li > div > p:first-child) { margin-top: 0; } .markdown :global(li > div > p:last-child) { margin-bottom: 0; }
    li.task { list-style: none; display: flex; align-items: baseline; gap: .55em; } .task input { width: auto; margin: 0; }
    blockquote { margin: 12px 0; padding-inline-start: 12px; border-inline-start: 2px solid var(--outline); color: var(--muted); }
    .markdown :global(code) { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .9em; background: var(--panel); border-radius: 4px; padding: .12em .3em; }
    .code-block { margin: 1em 0; border: 1px solid var(--border); border-radius: 10px; overflow: hidden; }
    .code-header { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 6px 12px; background: var(--panel); font-size: .85em; color: var(--muted); }
    .code-header button { padding: 3px 8px; min-height: max(32px, var(--control-hit-size)); min-width: 44px; }
    pre { margin: 0; padding: 12px; overflow: auto; max-height: 480px; white-space: pre; line-height: 1.5; tab-size: 4; }
    .markdown pre code { padding: 0; background: none; border-radius: 0; }
    .table-scroll { overflow-x: auto; margin: 1em 0; border: 1px solid var(--border); border-radius: 8px; }
    table { border-collapse: collapse; min-width: 100%; font-size: .95em; }
    th, td { padding: 8px 12px; border-bottom: 1px solid var(--border); min-width: 100px; vertical-align: top; }
    th { background: var(--panel); font-weight: 600; } tbody tr:last-child td { border-bottom: 0; }
    hr { border: 0; border-top: 1px solid var(--border); margin: 1.3em 0; }
    .copy-notice { font-size: .85em; color: var(--muted); }
    @media (pointer: coarse) { .code-header button { min-height: 44px; } }
    @media (max-width: 700px) { .markdown { line-height: 1.6470588235; } }
</style>
