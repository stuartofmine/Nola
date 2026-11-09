<script lang="ts">
    import { onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    type LoginItem = { available: boolean; enabled: boolean; labels: { title: string; body: string; enabled: string; disabled: string; unavailable: string } };
    type Desktop = { platform: string; loginItem?: () => Promise<LoginItem | null>; setLoginItem?: (enabled: boolean) => Promise<{ available: boolean; enabled: boolean } | null> };
    let bridge: Desktop | undefined;
    let loginState = $state<LoginItem | null>(null), busy = $state(false), failed = $state(false);
    let alive = true;
    let supported = $state(false);
    async function load() {
        if (!bridge?.loginItem || busy) return;
        busy = true; failed = false;
        try { const value = await bridge.loginItem(); if (alive) { loginState = value; failed = value === null; } }
        catch { if (alive) failed = true; }
        finally { if (alive) busy = false; }
    }
    onMount(() => {
        bridge = (window as Window & { NolaDesktop?: Desktop }).NolaDesktop;
        if (bridge?.platform === 'windows' && bridge.loginItem) {
            supported = true;
            void load();
        }
        return () => { alive = false; };
    });
    async function toggle() {
        if (!loginState?.available || busy || !bridge?.setLoginItem) return;
        const requested = !loginState.enabled;
        busy = true; failed = false;
        try {
            const next = await bridge.setLoginItem(requested);
            if (!next) throw new Error('unconfirmed');
            if (alive && loginState) loginState = { ...loginState, ...next };
        } catch {
            if (alive) failed = true;
            // Query the saved preference after an uncertain write. A rejected
            // IPC must never leave the old button permanently disabled.
            try { const actual = await bridge.loginItem?.(); if (alive && actual) { loginState = actual; failed = actual.enabled !== requested; } } catch { /* keep the last confirmed value */ }
        } finally { if (alive) busy = false; }
    }
</script>
{#if supported}
<section class="card" id="desktopLoginItemCard" aria-busy={busy}>
    <h2>{loginState?.labels.title ?? t($locale, 'settings.desktop.title')}</h2>
    {#if loginState}
    <p class="muted">{loginState.labels.body}</p>
    <button type="button" disabled={!loginState.available || busy || failed} aria-pressed={loginState.enabled} onclick={toggle}>
        {loginState.available ? (loginState.enabled ? loginState.labels.enabled : loginState.labels.disabled) : loginState.labels.unavailable}
    </button>
    {/if}
    {#if busy}<p role="status">{t($locale, 'rooms.loading')}</p>{/if}
    {#if failed}<p role="alert">{t($locale, 'settings.desktop.failed')}</p><button type="button" disabled={busy} onclick={load}>{t($locale, 'models.reload')}</button>{/if}
</section>
{/if}
