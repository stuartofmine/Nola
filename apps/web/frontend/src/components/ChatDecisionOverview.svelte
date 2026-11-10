<script lang="ts">
    import type { Snippet } from 'svelte';
    import { locale, t } from '../lib/i18n';
    let {working, needsResponse, onconversation, ontool, details}: {details?: Snippet; working: boolean; needsResponse: boolean; onconversation: () => void; ontool: (tool: 'review' | 'artifacts' | 'progress' | 'goal') => void} = $props();
</script>
<section class="decision-overview" aria-label={t($locale, 'chatInspector.overview')}>
    <h3>{t($locale, 'chatInspector.currentWork')}</h3>
    <p role="status">{t($locale, needsResponse ? 'chatPage.awaiting' : working ? 'chatOverview.running' : 'chatInspector.noCurrentExecution')}</p>
    {#if needsResponse}<button type="button" onclick={onconversation}>{t($locale, 'chatInspector.handleResponse')}</button>{/if}
    {#if details}<section class="deliverables"><h3>{t($locale, 'chatInspector.artifacts')}</h3>{@render details()}</section>{/if}
    <nav aria-label={t($locale, 'chatInspector.more')}>
        {#each ['review', 'artifacts', 'progress', 'goal'] as tool}
            <button type="button" onclick={() => ontool(tool as 'review' | 'artifacts' | 'progress' | 'goal')}><span>{t($locale, `chatInspector.${tool}`)}</span><span aria-hidden="true">›</span></button>
        {/each}
    </nav>
</section>
<style>
    h3 { margin:0; font-size:13px; font-weight:550; }
    .deliverables { display:grid; gap:8px; padding-top:16px; border-top:1px solid var(--border); }
    .decision-overview { display: grid; gap: 16px; } p { margin: 0; font-size: .875rem; line-height:1.6; color: var(--muted); }
    nav { display: grid; gap: 4px; } button { display: flex; justify-content: space-between; align-items: center; gap: 16px; width: 100%; min-height: 44px; border-color: transparent; background: transparent; padding: 8px; text-align: start; font-size: .875rem; }
    button:hover { background: var(--panel); }
</style>
