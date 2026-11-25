<script lang="ts">
    import { onMount } from 'svelte';
    import { fade, fly } from 'svelte/transition';
    import { locale, t } from '../lib/i18n';
    import { toasts, dismissToast } from '../lib/toast';

    let { hasMobileNav = false }: { hasMobileNav?: boolean } = $props();

    let reducedMotion = $state(typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches);
    onMount(() => {
        if (typeof matchMedia !== 'function') return;
        const preference = matchMedia('(prefers-reduced-motion: reduce)');
        const update = () => { reducedMotion = preference.matches; };
        update();
        preference.addEventListener('change', update);
        return () => preference.removeEventListener('change', update);
    });
</script>

{#if $toasts.length}
    <div class="toast-host" class:with-mobile-nav={hasMobileNav} role="region" aria-live="polite" aria-label={t($locale, 'shell.notifications')}>
        {#each $toasts as item (item.id)}
            <div class="toast" data-tone={item.tone} role={item.tone === 'error' ? 'alert' : 'status'}
                in:fly={{ y: 8, duration: reducedMotion ? 0 : 180 }} out:fade={{ duration: reducedMotion ? 0 : 100 }}>
                <span>{item.message}</span>
                <button type="button" class="toast-close" aria-label={t($locale, 'shell.dismiss')} onclick={() => dismissToast(item.id)}>×</button>
            </div>
        {/each}
    </div>
{/if}

<style>
    .toast-host {
        position: fixed;
        inset: auto 16px 20px auto;
        z-index: 120;
        display: grid;
        gap: 8px;
        max-width: min(360px, calc(100vw - 32px));
        pointer-events: none;
    }
    .toast {
        pointer-events: auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        padding: 10px 14px;
        border: 1px solid var(--border);
        border-radius: 12px;
        background: var(--surface-elevated);
        color: var(--content);
        font-size: .875rem;
        box-shadow: var(--dialog-shadow);
    }
    .toast[data-tone='error'] { border-color: var(--danger); color: var(--danger); }
    .toast[data-tone='success'] { border-color: var(--success); }
    .toast-close {
        min-width: 28px;
        min-height: 28px;
        padding: 0;
        border: 0;
        background: transparent;
        color: var(--muted);
        font-size: 16px;
        line-height: 1;
        flex: none;
    }
    @media (pointer: coarse) {
        .toast-close { min-width: 44px; min-height: 44px; margin: -6px -8px -6px 0; }
    }
    @media (max-width: 760px) {
        .toast-host { inset: auto 16px calc(16px + env(safe-area-inset-bottom, 0px)) 16px; max-width: none; }
        .toast-host.with-mobile-nav { bottom: calc(72px + env(safe-area-inset-bottom, 0px)); }
        .toast-close { min-width: 44px; min-height: 44px; margin: -6px -8px -6px 0; }
    }
</style>
