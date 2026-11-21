<script lang="ts">
    import { onMount } from 'svelte';
    import { settingsSession } from '../lib/settingsSession';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { randomId } from '../lib/randomId';
    import { sourceAccountActionKey, sourceAccountStateKey } from '../lib/locales/sourceAccounts';
    import CodexSourceLogin from './CodexSourceLogin.svelte';
    import KimiSourceLogin from './KimiSourceLogin.svelte';
    import OpenCodeSourceLogin from './OpenCodeSourceLogin.svelte';
    type Status = {account: {revision: number; account: {id: string; display_name: string}}; login_state: string; inference_supported: boolean; user_action: string | null; expires_at_ms: number | null};
    let source = $state('codex'), path = $state(''), name = $state('');
    let consent = $state(false), busy = $state(false), failed = $state(false);
    let status = $state<Status | null>(null);
    let management = $state<'checking' | 'local' | 'remote' | 'unavailable'>('checking');
    async function checkManagement() {
        if (location.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname)) {
            management = 'remote';
            return;
        }
        management = 'checking';
        try {
            const session = settingsSession(await api('/api/web/session'));
            management = session.localTrust ? 'local' : 'remote';
        } catch {
            management = 'unavailable';
        }
    }
    onMount(() => { void checkManagement(); });

    async function inspect(refresh = false) {
        if (busy || (!refresh && (!consent || !path.trim() || !name.trim()))) return;
        busy = true; failed = false;
        try {
            const body = refresh && status ? {account_id: status.account.account.id, expected_revision: status.account.revision}
                : {account_id: randomId(), display_name: name.trim(), source, path: path.trim(), expected_revision: null};
            status = await api<Status>(`/api/model-connections/source-accounts/${refresh ? 'refresh' : 'bind'}`, {method: 'POST', body: JSON.stringify(body)});
        } catch { failed = true; } finally { busy = false; }
    }
</script>

{#if management === 'remote'}
    <p class="muted" role="status">{t($locale, 'sourceAccounts.manageOnHost')}</p>
{:else if management === 'unavailable'}
    <p role="alert" class="error">{t($locale, 'settings.sessionError')}</p>
    <button type="button" onclick={checkManagement}>{t($locale, 'models.reload')}</button>
{:else if management === 'local'}
    <CodexSourceLogin />
    <KimiSourceLogin />
    <details><summary>OpenCode</summary><OpenCodeSourceLogin /></details>
    <details><summary>{t($locale, 'connections.sourceAccount')}</summary>
        <p>{t($locale, 'connections.accountLimits')}</p>
        <form onsubmit={event => { event.preventDefault(); void inspect(); }}>
            <label>{t($locale, 'connections.source')}<select bind:value={source} disabled={busy}><option value="codex">Codex</option><option value="kimi_code">Kimi Code CLI</option></select></label>
            <label>{t($locale, 'connections.accountName')}<input bind:value={name} required disabled={busy} /></label>
            <label>{t($locale, 'connections.credentialPath')}<input bind:value={path} required disabled={busy} /></label>
            <label><input type="checkbox" bind:checked={consent} disabled={busy} />{t($locale, 'connections.credentials')}</label>
            <button disabled={busy || !consent || !path.trim() || !name.trim()}>{t($locale, 'connections.readAccount')}</button>
        </form>
        {#if busy}<p role="status">{t($locale, 'rooms.loading')}</p>{/if}
        {#if failed}<p role="alert" class="error">{t($locale, 'connections.failed')}</p>{/if}
        {#if status}<p role="status">{status.account.account.display_name} · {t($locale, sourceAccountStateKey(status.login_state))}</p><p>{t($locale, sourceAccountActionKey(status.user_action))}</p>
            <button disabled={busy} onclick={() => inspect(true)}>{t($locale, 'connections.readAccount')}</button>{/if}
    </details>
{/if}

<style>form { display: grid; gap: 12px; padding: 12px 0; } summary { cursor: pointer; min-height: 44px; padding: 10px 12px; } input[type=checkbox] { width: auto; } p { overflow-wrap: anywhere; }</style>
