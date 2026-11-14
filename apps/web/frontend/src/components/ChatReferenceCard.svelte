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
        {#if reference.href}<a href={reference.href} title={reference.uri} target="_blank" rel="noopener noreferrer">{reference.title || reference.uri}</a>{:else}<span>{reference.title || t($locale, 'chatPage.reference')}</span>{/if}
        {#if reference.section || reference.page || reference.lineStart}<small>{[reference.section, reference.page && `${t($locale, 'chatPage.referencePage')} ${reference.page}`, reference.lineStart && `${reference.lineStart}–${reference.lineEnd || reference.lineStart}`].filter(Boolean).join(' · ')}</small>{/if}
        <details class="citation-detail"><summary aria-label={t($locale, 'chatPage.details')}>ⓘ</summary>
            <p>{reference.uri}</p>{#if reference.snippet}<p>{reference.snippet}</p>{/if}
            {#if reference.rangeStart || reference.rangeEnd}<p>[{reference.rangeStart}, {reference.rangeEnd})</p>{/if}
            {#if reference.uri && !reference.href}<p>{t($locale, 'chatPage.referenceUnavailable')}</p>{/if}
        </details>
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
    .citation-row { display: flex; gap: 6px; align-items: baseline; min-height: 32px; padding-block: 3px; color: var(--muted); font-size: .8125rem; overflow-wrap: anywhere; }
    .citation-number { font-variant-numeric: tabular-nums; flex: none; }
    .citation-copy { min-width: 0; flex: 1; position: relative; padding-inline-end: 32px; }
    .citation-copy > a { color: inherit; text-decoration: none; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; overflow: hidden; }
    .citation-copy > a:hover { color: var(--content); text-decoration: underline; }
    .citation-copy > small { display: block; font: inherit; }
    .citation-detail > summary { position: absolute; inset-inline-end: 0; top: -8px; min-height: 32px; min-width: 28px; justify-content: center; padding: 0; }
    .citation-detail > summary::before { content: none; }
    .citation-detail p { font-size: inherit; user-select: text; }
    @media (pointer: coarse) { .citation-row, .citation-detail > summary { min-height: 44px; } }
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
