<script lang="ts">
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    import { onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { browserNotifications } from '../lib/browserNotifications';
    let status = $state(browserNotifications.snapshot());
    let enabled = $state(false);
    let busy = $state(false), error = $state('');
    let disposed = false;
    function refresh() { if (!pageLocation.active() || disposed) return; status = browserNotifications.snapshot(); enabled = status.enabled; }
    async function change(event: Event) {
        if (busy) return;
        const requested = (event.currentTarget as HTMLInputElement).checked;
        enabled = requested;
        busy = true; error = '';
        try {
            if (requested && !await browserNotifications.setEnabled(true)) error = 'settings.notifications.deniedHint';
            else if (!requested) await browserNotifications.setEnabled(false);
        } catch { if (!disposed) error = 'settings.notifications.failed'; }
        finally { if (!disposed) { refresh(); busy = false; } }
    }
    function test() {
        error = '';
        try { browserNotifications.test(t($locale, 'settings.notifications.testTitle'), t($locale, 'settings.notifications.testBody')); }
        catch { error = 'settings.notifications.failed'; }
    }
    onMount(() => { refresh(); return () => { disposed = true; }; });
</script>
<svelte:window onfocus={refresh} />
<svelte:document onvisibilitychange={refresh} />
<section class="card" aria-busy={busy}>
    <h2>{t($locale, 'settings.category.notifications')}</h2>
    <p class="muted">{t($locale, 'settings.notifications.scope')}</p>
    <p role="status">{t($locale, `settings.notifications.${status.permission}`)}</p>
    {#if status.permission === 'denied'}<p class="muted">{t($locale, 'settings.notifications.deniedHint')}</p>{/if}
    {#if error}<p role="alert">{t($locale, error)}</p>{/if}
    <label><input type="checkbox" checked={enabled} onchange={change} disabled={busy || (!status.enabled && (!status.supported || status.permission === 'denied'))} />{t($locale, 'settings.notifications.enable')}</label>
    <div class="actions"><button disabled={busy} onclick={refresh}>{t($locale, 'models.reload')}</button><button disabled={busy || status.permission !== 'granted'} onclick={test}>{t($locale, 'settings.notifications.test')}</button></div>
</section>
<style>
    section{display:grid;gap:14px}h2,p{margin:0}label{display:flex;gap:8px;align-items:center}input{width:auto;flex-shrink:0}.actions{display:flex;flex-wrap:wrap;gap:8px}
</style>
