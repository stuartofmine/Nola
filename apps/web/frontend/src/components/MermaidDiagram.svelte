<script lang="ts">
    import library from '../../../app/static/vendor/mermaid/mermaid.min.js?url';
    import { darkAppearance } from '../lib/appearance';
    import { diagramDocument } from '../lib/diagramDocument';
    import { locale, t } from '../lib/i18n';
    let { source }: { source: string } = $props();
    let frame = $state<HTMLIFrameElement>();
    let height = $state(240);
    let status = $state<'loading' | 'ready' | 'failed'>('loading');
    const document = $derived(typeof location === 'undefined' ? '' : diagramDocument(source, new URL(library, location.href).href, $darkAppearance));
    $effect(() => { document; height = 240; status = 'loading'; });
    function receive(event: MessageEvent) {
        if (event.source !== frame?.contentWindow || event.data?.type !== 'nola-diagram') return;
        if (event.data.status === 'failed') status = 'failed';
        if (event.data.status === 'ready' && Number.isFinite(event.data.height)) {
            height = Math.min(640, Math.max(180, event.data.height));
            status = 'ready';
        }
    }
</script>
<svelte:window onmessage={receive} />
<section class="diagram" aria-label={t($locale, 'a2ui.diagram')} aria-busy={status === 'loading'}>
    {#if status !== 'failed'}<iframe bind:this={frame} title={t($locale, 'a2ui.diagram')} loading="lazy" sandbox="allow-scripts" srcdoc={document} style:height={`${height}px`}></iframe>{/if}
    {#if status === 'loading'}<p role="status">{t($locale, 'a2ui.diagramLoading')}</p>{/if}
    {#if status === 'failed'}<p role="status">{t($locale, 'a2ui.diagramFailed')}</p>{/if}
    <details open={status === 'failed'}><summary>{t($locale, 'a2ui.diagramSource')}</summary><pre><code>{source}</code></pre></details>
</section>
<style>
    .diagram { min-width: 0; border: 1px solid var(--outline); border-radius: 12px; overflow: hidden; }
    iframe { display: block; width: 100%; border: 0; background: var(--surface); }
    summary { min-height: var(--control-hit-size, 44px); padding: 10px 12px; cursor: pointer; }
    p { margin: 0; padding: 10px 12px; color: var(--muted); }
    pre { max-height: 480px; margin: 0; padding: 12px; overflow: auto; white-space: pre; }
</style>
