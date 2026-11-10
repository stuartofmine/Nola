<script lang="ts">
    import { onDestroy, tick } from 'svelte';
    import { api, ApiError } from '../lib/api';
    import { safeLoginNext, validHostToken, loginErrorKind } from '../lib/login';
    import { locale, localePreference, supportedLocales, t, setLocale } from '../lib/i18n';
    import type { Locale } from '../lib/i18n';
    import { languageNames } from '../lib/locales/shell';
    const params = new URLSearchParams(window.location.search);
    const next = safeLoginNext(params.get('next'));
    let token = $state('');
    let revealed = $state(false);
    let busy = $state(false);
    let error = $state(params.has('error') ? loginErrorKind(params.get('error') ?? '') : '');
    let copyStatus = $state('');
    let errorElement = $state<HTMLParagraphElement>();
    let input = $state<HTMLInputElement>();
    let disposed = false;
    const controller = new AbortController();
    onDestroy(() => { disposed = true; controller.abort(); token = ''; });
    const errorText = $derived(error ? t($locale, `shell.login.error.${error}`) : '');

    async function connect(event: SubmitEvent) {
        event.preventDefault();
        if (busy || !validHostToken(token)) return;
        busy = true; error = '';
        try {
            const response = await api<{ authenticated: boolean }>('/api/web/login', {
                method: 'POST', body: JSON.stringify({ token: token.trim() }), signal: controller.signal,
            });
            if (disposed) return;
            if (response.authenticated !== true) throw new Error('session not established');
            token = ''; revealed = false;
            window.location.replace(next);
        } catch (cause) {
            if (disposed) return;
            error = cause instanceof ApiError ? loginErrorKind(cause.message) : 'network';
            busy = false;
            await tick(); errorElement?.focus();
        }
    }

    async function copyCommand() {
        try {
            await navigator.clipboard.writeText('nola web');
            if (!disposed) copyStatus = 'shell.login.copied';
        } catch {
            if (!disposed) copyStatus = 'shell.login.copy_failed';
        }
    }
</script>

<svelte:head><title>{t($locale, 'shell.login.page_title')}</title><meta name="referrer" content="no-referrer" /></svelte:head>
<main id="page-main" tabindex="-1" class="login-shell" lang={$locale}>
    <section class="login-card" aria-labelledby="login-title">
        <div class="wordmark" aria-label="Nola">Nola</div>
        <label class="language-selector">{t($locale, 'shell.language')}
            <select value={$localePreference} onchange={event => setLocale(event.currentTarget.value as Locale | 'system')}>
                <option value="system">{t($locale, 'shell.language.system')}</option>
                {#each supportedLocales as language}<option value={language} lang={language}>{languageNames[language]}</option>{/each}
            </select>
        </label>
        <header><h1 id="login-title">{t($locale, 'shell.login.title')}</h1><p class="muted">{t($locale, 'shell.login.subtitle')}</p></header>
        {#if params.get('logged_out') === '1'}<p class="notice" role="status">{t($locale, 'shell.login.signed_out')}</p>{/if}
        {#if errorText}<p class="notice error" role="alert" tabindex="-1" bind:this={errorElement}>{errorText}</p>{/if}
        <form onsubmit={connect} aria-busy={busy} autocomplete="off">
            <label for="host-token">{t($locale, 'shell.login.token')}</label>
            <div class="secret">
                <input id="host-token" bind:this={input} type={revealed ? 'text' : 'password'} bind:value={token} oninput={() => error = ''} required maxlength="256" autocomplete="off" spellcheck="false" autocapitalize="none" disabled={busy} aria-describedby="token-hint token-validation" aria-invalid={token.trim().length > 0 && !validHostToken(token)} />
                <button type="button" disabled={busy} aria-controls="host-token" aria-pressed={revealed} onclick={() => { revealed = !revealed; input?.focus(); }}>{t($locale, revealed ? 'shell.login.hide' : 'shell.login.show')}</button>
            </div>
            <p id="token-hint" class="muted">{t($locale, 'shell.login.token_hint')}</p>
            <p id="token-validation" class="error" role="status">{token.trim().length > 0 && !validHostToken(token) ? t($locale, 'shell.login.token_validation') : ''}</p>
            <button class="primary connect" disabled={busy || !validHostToken(token)}>{t($locale, busy ? 'shell.login.connecting' : 'shell.login.connect')}</button>
        </form>
        <div class="token-source">
            <h2>{t($locale, 'shell.login.need_token')}</h2>
            <p class="muted">{t($locale, 'shell.login.run_command')}</p>
            <div class="command"><code>nola web</code><button type="button" onclick={copyCommand}>{t($locale, 'shell.login.copy_command')}</button></div>
            <p class="muted" role="status">{copyStatus ? t($locale, copyStatus) : ''}</p>
            <p class="muted">{t($locale, 'shell.login.token_location')} <code>~/.nola/cli/web/mobile-access-token</code></p>
        </div>
        <footer class="muted"><h2>{t($locale, 'shell.login.security')}</h2><p>{t($locale, 'shell.login.security_hint')}</p></footer>
    </section>
</main>

<style>
    .login-shell { min-height: 100dvh; display: grid; place-items: center; padding: 40px 20px; background: var(--panel); }
    .login-card { width: 100%; max-width: 480px; display: grid; gap: 24px; padding: 32px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface); }
    .wordmark { font-size: 20px; font-weight: 650; }
    .language-selector { display: grid; gap: 6px; font-size: 12px; color: var(--muted); }
    .language-selector select { width: 100%; min-width: 0; }
    form { display: grid; gap: 8px; }
    form p { font-size: 12px; margin: 0; }
    .secret, .command { display: flex; align-items: center; gap: 8px; }
    .secret input { flex: 1; min-width: 0; }
    .secret button { min-width: 64px; }
    .connect { margin-top: 8px; min-height: max(40px, var(--control-hit-size)); }
    .command { justify-content: space-between; flex-wrap: wrap; padding: 10px; border: 1px solid var(--border); border-radius: 7px; margin-top: 10px; }
    code { user-select: text; overflow-wrap: anywhere; font-size: 12px; }
    h2 { font-size: 13px; }
    footer { border-top: 1px solid var(--border); padding-top: 20px; font-size: 12px; }
    @media (max-width: 480px) { .login-shell { padding: 16px; } .login-card { padding: 24px 20px; } }
</style>
