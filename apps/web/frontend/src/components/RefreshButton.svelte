<script lang="ts">
    import { locale, t } from '../lib/i18n';
    let {loading, disabled = false, refresh, element = $bindable()}: {
        loading: boolean; disabled?: boolean; refresh: () => void; element?: HTMLButtonElement;
    } = $props();
</script>

<button type="button" bind:this={element} {disabled} aria-disabled={disabled || loading} aria-busy={loading}
    onclick={() => { if (!loading && !disabled) refresh(); }}>
    <span class:pending={loading}>{t($locale, 'data_sources.action.refresh')}</span>
    {#if loading}<span class="spinner" aria-hidden="true"></span>{/if}
</button>

<style>
    button { display: inline-grid; place-items: center; }
    button > span { grid-area: 1 / 1; }
    .pending { opacity: 0; }
    .spinner { width: 1em; height: 1em; border: 2px solid var(--border); border-top-color: currentColor; border-radius: 50%; }
    @media (prefers-reduced-motion: no-preference) { .spinner { animation: rotate .85s linear infinite; } }
    @keyframes rotate { to { transform: rotate(1turn); } }
</style>
