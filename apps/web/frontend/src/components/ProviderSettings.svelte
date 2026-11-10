<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { providerSummary, type ProviderSummary } from '../lib/providerSummary';
    let summary = $state<ProviderSummary>();
    let loading = $state(false), failed = $state(false);
    let disposed = false;
    const controller = new AbortController();
    async function refresh() {
        if (loading) return;
        loading = true; failed = false; summary = undefined;
        try {
            const next = providerSummary(await api('/api/model-profiles', { signal: controller.signal }));
            if (!disposed) summary = next;
        } catch { if (!disposed) failed = true; }
        finally { if (!disposed) loading = false; }
    }
    onMount(() => { void refresh(); return () => { disposed = true; controller.abort(); }; });
</script>
<section class="card" aria-busy={loading}>
    <header><h2>{t($locale, 'settings.category.providers')}</h2><button disabled={loading} onclick={refresh}>{t($locale, 'models.reload')}</button></header>
    <p class="muted">{t($locale, 'settings.providers.detail')}</p>
    {#if loading}<p role="status">{t($locale, 'settings.providers.loading')}</p>{/if}
    {#if failed}<p role="alert">{t($locale, 'settings.providers.failed')}</p>{/if}
    {#if summary}
        <dl>
            <dt>{t($locale, 'settings.providers.endpoints')}</dt><dd>{summary.endpoints}</dd>
            <dt>{t($locale, 'settings.providers.models')}</dt><dd>{summary.models}</dd>
            <dt>{t($locale, 'settings.providers.selected')}</dt><dd>{#if summary.selected}<a href={`/models?tab=api&configure=${encodeURIComponent(summary.selected.id)}`}>{summary.selected.name}</a>{:else}{t($locale, 'settings.providers.none')}{/if}</dd>
        </dl>
    {/if}
    <a class="button" href="/models?tab=api">{t($locale, 'settings.providers.manage')}</a>
</section>
<style>
    dd a{color:var(--accent);text-decoration:underline;text-underline-offset:3px}
    header{display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}h2{margin:0}dl{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:12px}dt{color:var(--muted)}dd{margin:0;overflow-wrap:anywhere}@media(max-width:480px){dl{grid-template-columns:minmax(0,1fr)}dd{margin-bottom:8px}}
</style>
