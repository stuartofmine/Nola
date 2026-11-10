<script lang="ts">
    import { replyDisclosure } from '../lib/replyDisclosure';
    import { locale, t } from '../lib/i18n';
    import type { ChatReference } from '../lib/chatReferences';
    let { reference }: { reference: ChatReference } = $props();
</script>

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

<style>
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
