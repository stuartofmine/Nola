<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { locale, t } from '../lib/i18n';
    type LoginItem = { available: boolean; enabled: boolean; labels: { title: string; body: string; enabled: string; disabled: string; unavailable: string } };
    type Desktop = { platform: string; loginItem?: () => Promise<LoginItem | null>; setLoginItem?: (enabled: boolean) => Promise<{ available: boolean; enabled: boolean } | null>; openLogsFolder?: () => Promise<unknown> };
    let bridge: Desktop | undefined;
    let loginState = $state<LoginItem | null>(null), busy = $state(false), failed = $state(false);
    let alive = true;
    let supported = $state(false);
    let canOpenLogs = $state(false);
    let loadedLocale = '';
    async function load() {
        if (!bridge?.loginItem || busy) return;
        busy = true; failed = false;
        try { const value = await bridge.loginItem(); if (alive) { loginState = value; failed = value === null; } }
        catch { if (alive) failed = true; }
        finally { if (alive) busy = false; }
    }
    $effect(() => {
        const currentLocale = $locale;
        if (supported && currentLocale !== loadedLocale) {
            loadedLocale = currentLocale;
            untrack(() => { void load(); });
        }
    });
    onMount(() => {
        bridge = (window as Window & { NolaDesktop?: Desktop }).NolaDesktop;
        if (bridge?.platform === 'windows' && bridge.loginItem) {
            supported = true;
            canOpenLogs = typeof bridge.openLogsFolder === 'function';
            loadedLocale = $locale;
            void load();
        }
        return () => { alive = false; };
    });
    async function openLogs() {
        if (!bridge?.openLogsFolder) return;
        try { await bridge.openLogsFolder(); } catch { /* native open failure is non-fatal */ }
    }
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
    {#if loginState.available}
    <label class="toggle"><span>{loginState.enabled ? loginState.labels.enabled : loginState.labels.disabled}</span><input type="checkbox" role="switch" checked={loginState.enabled} disabled={busy || failed} onchange={event => { event.currentTarget.checked = loginState!.enabled; void toggle(); }} /></label>
    {:else}
    <p class="muted">{loginState.labels.unavailable}</p>
    {/if}
    {/if}
    {#if busy}<p role="status">{t($locale, 'rooms.loading')}</p>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, 'settings.desktop.failed')}</p><button type="button" disabled={busy} onclick={load}>{t($locale, 'models.reload')}</button>{/if}
    {#if canOpenLogs}<div class="desktop-actions"><button type="button" onclick={openLogs}>{t($locale, 'diagnostics.openLogs')}</button></div>{/if}
</section>
{/if}
<style>.toggle { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 0; cursor: pointer; }.desktop-actions { display: flex; flex-wrap: wrap; gap: 8px; padding-top: 8px; border-top: 1px solid var(--border); }</style>
