<script lang="ts">
    import { locale, t } from '../lib/i18n';
    let {working, needsResponse, onconversation}: {working: boolean; needsResponse: boolean; onconversation: () => void} = $props();
</script>
<section class="decision-overview" aria-label={t($locale, 'chatInspector.overview')}>
    <p role="status">{t($locale, needsResponse ? 'chatPage.awaiting' : working ? 'chatOverview.running' : 'chatInspector.noCurrentExecution')}</p>
    {#if needsResponse}<button type="button" onclick={onconversation}>{t($locale, 'chatInspector.handleResponse')}</button>{/if}

</section>
<style>
    .decision-overview { display: grid; gap: 16px; } p { margin: 0; font-size: .875rem; line-height:1.6; color: var(--muted); }
    button { display: flex; justify-content: space-between; align-items: center; gap: 16px; width: 100%; min-height: 44px; border-color: transparent; background: transparent; padding: 8px; text-align: start; font-size: .875rem; }
    button:hover { background: var(--panel); }
</style>
