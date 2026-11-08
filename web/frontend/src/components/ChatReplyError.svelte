<script lang="ts">
    import { locale, t } from '../lib/i18n';
    let { kind = 'failed', detail = '', oncontinue, continuing = false }: {
        kind?: 'failed' | 'notSent' | 'uncertain'; detail?: string;
        oncontinue?: () => void; continuing?: boolean;
    } = $props();
</script>

<section class="reply-error" data-kind={kind} role="status" aria-atomic="true">
    <strong>{t($locale, `chatPage.replyError_${kind}`)}</strong>
    {#if detail}<p class="detail">{detail}</p>{/if}
    {#if kind === 'notSent'}<p>{t($locale, 'chatPage.replyNotSentHelp')}</p>{/if}
    {#if kind === 'uncertain'}
        <p>{t($locale, 'chatPage.replyUncertainHelp')}</p>
        {#if oncontinue}<button type="button" disabled={continuing} onclick={oncontinue}>{t($locale, 'chatPage.openConversations')}</button>{/if}
    {/if}
</section>

<style>
    .reply-error { margin: 10px 0; padding: 12px 14px; border-inline-start: 3px solid var(--danger); border-radius: 6px; background: color-mix(in srgb, var(--danger) 6%, var(--canvas)); overflow-wrap: anywhere; font-size: 13px; }
    strong { color: var(--danger); }
    p { margin: 6px 0 0; }
    .detail { white-space: pre-wrap; }
    button { margin-top: 10px; min-height: 44px; max-width: 100%; white-space: normal; }
</style>
