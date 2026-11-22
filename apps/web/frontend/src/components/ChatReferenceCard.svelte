<script lang="ts">
    import { replyDisclosure } from '../lib/replyDisclosure';
    import { locale, t } from '../lib/i18n';
    import type { ChatReference } from '../lib/chatReferences';
    let { reference, compact = false, number }: { reference: ChatReference; compact?: boolean; number?: number } = $props();
</script>

{#if compact}
<section class="citation-row">
    <span class="citation-number">[{number}]</span>
    <div class="citation-copy">
        <div class="citation-inline">
            {#if reference.href}<a href={reference.href} title={reference.uri} target="_blank" rel="noopener noreferrer">{reference.title || reference.uri}</a>{:else}<span class="citation-title">{reference.title || t($locale, 'chatPage.reference')}</span>{/if}
            {#if reference.section || reference.page || reference.lineStart}<small class="citation-loc">{[reference.section, reference.page && `${t($locale, 'chatPage.referencePage')} ${reference.page}`, reference.lineStart && `${reference.lineStart}–${reference.lineEnd || reference.lineStart}`].filter(Boolean).join(' · ')}</small>{/if}
            {#if reference.uri || reference.snippet || reference.rangeStart || reference.rangeEnd}
                <details class="citation-detail"><summary>{t($locale, 'chatPage.details')}</summary>
                    <div class="citation-detail-body">
                        {#if reference.uri}<p>{reference.uri}</p>{/if}
                        {#if reference.snippet}<p>{reference.snippet}</p>{/if}
                        {#if reference.rangeStart || reference.rangeEnd}<p>[{reference.rangeStart}, {reference.rangeEnd})</p>{/if}
                        {#if reference.uri && !reference.href}<p>{t($locale, 'chatPage.referenceUnavailable')}</p>{/if}
                    </div>
                </details>
            {/if}
        </div>
    </div>
</section>
{:else}
<section class="reference-card" data-kind={reference.kind}>
    <strong>{#if reference.href}<a href={reference.href} target="_blank" rel="noopener noreferrer">{reference.title || reference.uri}</a>
    {:else}{reference.title || t($locale, 'chatPage.reference')}{/if}</strong>
    {#if reference.uri && !reference.href}<p class="hint">{t($locale, 'chatPage.referenceUnavailable')}</p>{/if}
    {#if reference.mimeType || reference.operation}<p class="hint">{[reference.mimeType, reference.operation].filter(Boolean).join(' · ')}</p>{/if}
    {#if reference.snippet}<p class="snippet">{reference.snippet}</p>{/if}
    <dl>
        {#if reference.section}<dt>{t($locale, 'chatPage.referenceSection')}</dt><dd>{reference.section}</dd>{/if}
        {#if reference.page}<dt>{t($locale, 'chatPage.referencePage')}</dt><dd>{reference.page}</dd>{/if}
        {#if reference.lineStart || reference.lineEnd}<dt>{t($locale, 'chatPage.referenceLines')}</dt><dd>{reference.lineStart || '…'}–{reference.lineEnd || '…'}</dd>{/if}
    </dl>
    {#if reference.uri || reference.sourceId || reference.rangeStart || reference.rangeEnd}
        <details use:replyDisclosure class="reference-details">
            <summary>{t($locale, 'chatPage.toolTechnicalDetails')}</summary>
            {#if reference.uri}<p class="uri">{reference.uri}</p>{/if}
            <dl>
                {#if reference.sourceId}<dt>{t($locale, 'chatPage.referenceId')}</dt><dd>{reference.sourceId}</dd>{/if}
                {#if reference.rangeStart || reference.rangeEnd}<dt>{t($locale, 'chatPage.referenceRange')}</dt><dd>[{reference.rangeStart || '…'}, {reference.rangeEnd || '…'})</dd>{/if}
            </dl>
        </details>
    {/if}
</section>
{/if}

<style>
    .citation-row { display: flex; gap: 6px; align-items: baseline; min-height: 28px; padding-block: 2px; color: var(--muted); font-size: .8125rem; overflow-wrap: anywhere; }
    .citation-number { font-variant-numeric: tabular-nums; flex: none; color: var(--accent); font-weight: 500; }
    .citation-copy { min-width: 0; flex: 1; }
    .citation-inline { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 8px; }
    .citation-inline > a { color: inherit; text-decoration: none; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 1; line-clamp: 1; overflow: hidden; }
    .citation-inline > a:hover { color: var(--content); text-decoration: underline; }
    .citation-title { color: var(--content); }
    .citation-loc { font: inherit; color: var(--muted); }
    .citation-detail { display: inline-block; }
    .citation-detail[open] { display: block; flex-basis: 100%; }
    .citation-detail > summary { display: inline-flex; align-items: center; gap: 4px; min-height: 24px; padding: 1px 8px; border: 0; border-radius: 999px; background: var(--panel); color: var(--muted); font-size: 11px; }
    .citation-detail > summary:hover { color: var(--content); background: var(--accent-subtle); }
    .citation-detail > summary::before { content: none; }
    .citation-detail > summary::after { content: '›'; transform: rotate(90deg); font-size: 12px; color: currentColor; }
    .citation-detail[open] > summary::after { transform: rotate(270deg); }
    .citation-detail-body { margin-top: 6px; padding-inline-start: 10px; border-inline-start: 2px solid var(--border); }
    .citation-detail p { font-size: inherit; user-select: text; margin: 4px 0 0; }
    @media (pointer: coarse) { .citation-row { min-height: 36px; } .citation-detail > summary { min-height: 32px; padding: 4px 10px; } }
    .reference-card { margin: 8px 0; padding: 12px; border: 1px solid var(--border); border-radius: var(--radius); min-width: 0; overflow-wrap: anywhere; }
    p { margin: 6px 0 0; }
    .hint, .uri, dl { font-size: 12px; color: var(--muted); }
    .snippet { white-space: pre-wrap; }
    summary { min-height: 44px; display: flex; align-items: center; gap: 8px; cursor: pointer; font-size: 12px; color: var(--muted); list-style: none; }
    summary::before { content: '›'; font-size: 18px; }
    details[open] > summary::before { transform: rotate(90deg); }
    summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 4px; }
    dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 4px 10px; margin: 8px 0 0; }
    dl:empty { display: none; } dd { margin: 0; user-select: text; }
</style>
