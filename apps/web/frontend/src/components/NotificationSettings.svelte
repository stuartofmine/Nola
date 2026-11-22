<script lang="ts">
    import { onMount } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { browserNotifications, notificationPreferenceKey } from '../lib/browserNotifications';
    let status = $state(browserNotifications.snapshot());
    let busy = $state(false), error = $state('');
    let tested = $state(false);
    let disposed = false;
    function refresh() {
        if (disposed) return;
        const next = browserNotifications.snapshot();
        if (next.permission !== status.permission || next.enabled !== status.enabled) {
            error = ''; tested = false;
        }
        status = next;
    }
    async function change(event: Event) {
        if (busy) return;
        const toggle = event.currentTarget as HTMLInputElement;
        const requested = toggle.checked;
        busy = true; error = ''; tested = false;
        try {
            const allowed = await browserNotifications.setEnabled(requested);
            if (disposed) return;
            refresh();
            if (requested && !allowed && status.permission === 'default') error = 'settings.notifications.dismissed';
        } catch { if (!disposed) { refresh(); error = 'settings.notifications.failed'; } }
        finally {
            if (!disposed) {
                toggle.checked = status.enabled;
                busy = false;
            }
        }
    }
    function test() {
        error = ''; tested = false;
        try {
            browserNotifications.test(t($locale, 'settings.notifications.testTitle'), t($locale, 'settings.notifications.testBody'));
            tested = true;
        } catch { refresh(); error = 'settings.notifications.failed'; }
    }
    onMount(() => {
        refresh();
        let permission: PermissionStatus | undefined;
        navigator.permissions?.query({name: 'notifications'}).then(result => {
            if (disposed) return;
            permission = result;
            result.addEventListener('change', refresh);
            refresh();
        }).catch(() => {});
        return () => { disposed = true; permission?.removeEventListener('change', refresh); };
    });
</script>
<svelte:window onfocus={refresh} onstorage={event => { if (event.key === notificationPreferenceKey || event.key === null) refresh(); }} />
<svelte:document onvisibilitychange={refresh} />
<section class="card" aria-busy={busy}>
    <h2>{t($locale, 'settings.category.notifications')}</h2>
    <p class="muted">{t($locale, 'settings.notifications.summary')}</p>
    <p role="status">{t($locale, `settings.notifications.${status.permission}`)}</p>
    {#if status.permission === 'denied'}<p class="muted">{t($locale, 'settings.notifications.deniedHint')}</p>{/if}
    {#if error}<p role="alert" class="error">{t($locale, error)}</p>{/if}
    <label><input type="checkbox" role="switch" checked={status.enabled} onchange={change} disabled={busy || (!status.enabled && (!status.supported || status.permission === 'denied'))} />{t($locale, 'settings.notifications.enable')}</label>
    <div class="actions"><button disabled={busy} onclick={refresh}>{t($locale, 'models.reload')}</button><button disabled={busy || status.permission !== 'granted'} onclick={test}>{t($locale, 'settings.notifications.test')}</button></div>
    {#if tested}<p role="status">{t($locale, 'settings.notifications.testSent')}</p>{/if}
    <details><summary>{t($locale, 'settings.notifications.details')}</summary><p class="muted">{t($locale, 'settings.notifications.scope')}</p></details>
</section>
<style>
    section{display:grid;gap:14px}h2,p{margin:0}label{display:flex;gap:8px;align-items:center}input{width:auto;flex-shrink:0}.actions{display:flex;flex-wrap:wrap;gap:8px}
</style>
