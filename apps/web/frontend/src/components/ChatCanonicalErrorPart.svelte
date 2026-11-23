<script lang="ts">
    import { replyDisclosure } from '../lib/replyDisclosure';
    import { locale, t } from '../lib/i18n';
    import type { CanonicalPart } from '../lib/chatTimeline';
    let { part }: {part: CanonicalPart} = $props();
    const displayKey = $derived(typeof part.display_key === 'string' ? part.display_key : '');
    const translated = $derived(displayKey ? t($locale, displayKey) : '');
    const title = $derived(translated && translated !== displayKey ? translated : t($locale, 'chatPage.partFailed'));
    const message = $derived(typeof part.message === 'string' && part.message.trim() ? part.message.trim()
        : typeof part.detail === 'string' && part.detail.trim() ? part.detail.trim() : '');
    const code = $derived(typeof part.code === 'string' && part.code.trim() ? part.code.trim() : '');
    // Typed diagnostics grant no authority to retry the failed operation.
    const diagnostics = $derived(JSON.stringify({part_id: part.part_id,
        code: part.code, display_key: part.display_key, recoverable: part.recoverable}, null, 2));
</script>

<section class="part-error" data-kind="failed" data-part-id={part.part_id} role="status">
    <strong>{title}</strong>
    {#if message}<p class="error-message">{message}</p>{/if}
    {#if code && code !== title}<p class="error-code"><code>{code}</code></p>{/if}
    <details use:replyDisclosure>
        <summary>{t($locale, 'chatPage.toolTechnicalDetails')}</summary>
        <pre>{diagnostics}</pre>
    </details>
</section>

<style>
    .part-error { min-width: 0; overflow-wrap: anywhere; }
    strong { color: var(--danger); font-size: .875rem; }
    .error-message { margin: 6px 0 0; font-size: .875rem; white-space: pre-wrap; }
    .error-code { margin: 4px 0 0; font-size: .75rem; color: var(--muted); }
    summary { min-height: 44px; box-sizing: border-box; padding-block: 12px; cursor: pointer; color: var(--muted); font-size: .8125rem; }
    summary:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 6px; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: .8125rem; margin-block: 4px; }
</style>
