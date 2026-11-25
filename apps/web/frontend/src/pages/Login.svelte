<script lang="ts">
    import { onDestroy, onMount, tick } from 'svelte';
    import { api, ApiError } from '../lib/api';
    import { safeLoginNext, validHostToken, loginErrorKind } from '../lib/login';
    import { settingsSession } from '../lib/settingsSession';
    import { locale, localePreference, supportedLocales, t, setLocale } from '../lib/i18n';
    import type { Locale } from '../lib/i18n';
    import { languageNames } from '../lib/locales/shell';
    const params = new URLSearchParams(window.location.search);
    const next = safeLoginNext(params.get('next'));
    let token = $state('');
    let revealed = $state(false);
    let busy = $state(false);
    let canReturn = $state(false);
    let pasteHint = $state(false);
    let pastePending = $state(false);
    let inputRevision = 0;
    let error = $state(params.has('error') ? loginErrorKind(params.get('error') ?? '') : '');
    let attempted = $state(false);
    const invalidToken = $derived(attempted && !validHostToken(token));
    let errorElement = $state<HTMLParagraphElement>();
    let input = $state<HTMLInputElement>();
    let disposed = false;
    const controller = new AbortController();
    onDestroy(() => { disposed = true; controller.abort(); token = ''; });
    onMount(() => {
        void api('/api/web/session', {signal: controller.signal}).then(value => {
            if (!disposed) canReturn = settingsSession(value).authenticated;
        }).catch(() => { /* Keep sign-in available when an existing session cannot be confirmed. */ });
    });
    const errorText = $derived(error ? t($locale, `shell.login.error.${error}`) : '');
    const isWindows = typeof navigator !== 'undefined' && /Win/i.test(navigator.platform || navigator.userAgent);

    async function pasteToken() {
        if (busy || pastePending) return;
        const revision = inputRevision;
        pastePending = true;
        pasteHint = false;
        try {
            const text = (await navigator.clipboard?.readText())?.trim();
            if (disposed || busy || revision !== inputRevision) return;
            if (text) {
                token = text;
                error = '';
                attempted = false;
            } else {
                pasteHint = true;
            }
        } catch {
            if (!disposed && !busy && revision === inputRevision) pasteHint = true;
        } finally {
            pastePending = false;
            if (!disposed && !busy && revision === inputRevision) input?.focus();
        }
    }

    async function connect(event: SubmitEvent) {
        event.preventDefault();
        if (busy) return;
        inputRevision++; pasteHint = false;
        attempted = true;
        if (!validHostToken(token)) { input?.focus(); return; }
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

</script>

<svelte:head><title>{t($locale, 'shell.login.page_title')}</title><meta name="referrer" content="no-referrer" /></svelte:head>
<main id="page-main" tabindex="-1" class="login-shell" lang={$locale}>
    <section class="login-card" aria-labelledby="login-title">
        <div class="topline">
            <div class="brand"><span class="mark" aria-hidden="true">n</span><span class="wordmark">Nola</span></div>
            <label class="language-selector">{t($locale, 'shell.language')}
                <select value={$localePreference} onchange={event => setLocale(event.currentTarget.value as Locale | 'system')}>
                    <option value="system">{t($locale, 'shell.language.system')}</option>
                    {#each supportedLocales as language}<option value={language} lang={language}>{languageNames[language]}</option>{/each}
                </select>
            </label>
        </div>
        <header><h1 id="login-title">{t($locale, 'shell.login.title')}</h1><p class="muted">{t($locale, 'shell.login.subtitle')}</p></header>
        {#if params.get('logged_out') === '1'}<p class="notice" role="status">{t($locale, 'shell.login.signed_out')}</p>{/if}
        {#if errorText}<p class="notice error" role="alert" tabindex="-1" bind:this={errorElement}>{errorText}</p>{/if}
        <form novalidate onsubmit={connect} aria-busy={busy} autocomplete="off">
            <label for="host-token">{t($locale, 'shell.login.token')}</label>
            <div class="secret">
                <input id="host-token" bind:this={input} type={revealed ? 'text' : 'password'} bind:value={token} placeholder={t($locale, 'shell.login.token_placeholder')} oninput={() => { inputRevision++; error = ''; attempted = false; pasteHint = false; }} required maxlength="256" autocomplete="off" spellcheck="false" autocapitalize="none" disabled={busy} aria-describedby={invalidToken ? "token-validation" : pasteHint ? "paste-help" : undefined} aria-invalid={invalidToken} />
                <button type="button" disabled={busy || pastePending} onclick={pasteToken}>{t($locale, 'shell.login.paste')}</button>
                <button type="button" disabled={busy} aria-controls="host-token" aria-pressed={revealed} onclick={() => { revealed = !revealed; input?.focus(); }}>{t($locale, revealed ? 'shell.login.hide' : 'shell.login.show')}</button>
            </div>
            {#if pasteHint}<p id="paste-help" class="muted" role="status">{t($locale, 'shell.login.paste_help')}</p>{/if}
            {#if invalidToken}<p id="token-validation" class="error" role="alert">{t($locale, 'shell.login.token_validation')}</p>{/if}
            <button class="primary connect" disabled={busy || !token.trim()}>{t($locale, busy ? 'shell.login.connecting' : 'shell.login.connect')}</button>
            {#if canReturn}<button type="button" disabled={busy} onclick={() => window.location.replace(next)}>{t($locale, 'shell.cancel')}</button>{/if}
        </form>
        <details>
            <summary>{t($locale, 'shell.login.need_token')}</summary>
            <p>{t($locale, 'shell.login.token_location')}</p>
            <div class="token-paths">
                {#if isWindows}
                    <div><span class="path-label">Windows</span><code>%USERPROFILE%\.nola\web\mobile-access-token</code></div>
                    <div><span class="path-label">macOS / Linux</span><code>~/.nola/web/mobile-access-token</code></div>
                {:else}
                    <div><span class="path-label">macOS / Linux</span><code>~/.nola/web/mobile-access-token</code></div>
                    <div><span class="path-label">Windows</span><code>%USERPROFILE%\.nola\web\mobile-access-token</code></div>
                {/if}
            </div>
        </details>
    </section>
</main>

<style>
    .login-shell { min-height: 100dvh; display: grid; place-items: center; padding: 40px 20px; background: var(--panel); }
    .login-card { width: 100%; max-width: 480px; display: grid; gap: 20px; padding: 32px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface); }
    .brand { display: inline-flex; align-items: center; gap: 10px; padding: 0; }
    .mark { width: 32px; height: 32px; display: grid; place-items: center; border-radius: 9px; background: var(--accent); color: var(--on-primary); font: italic 600 20px/1 Georgia, serif; }
    .wordmark { font-size: 20px; font-weight: 650; letter-spacing: -.015em; }
    .topline { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; }
    .language-selector { display: flex; align-items: center; gap: 8px; white-space: nowrap; font-size: 12px; color: var(--muted); }
    .language-selector select { width: auto; min-width: 0; min-height: 44px; }
    header h1 { margin: 0; }
    header p { margin: 8px 0 0; }
    form { display: grid; gap: 8px; }
    form p { font-size: 12px; margin: 0; }
    .secret { display: flex; align-items: center; gap: 8px; }
    .secret input { flex: 1; min-width: 0; }
    .secret button { min-width: 60px; flex: none; }
    .connect { margin-top: 8px; min-height: max(44px, var(--control-hit-size)); }
    details { border-top: 1px solid var(--border); color: var(--muted); }
    summary { cursor: pointer; padding-block: 12px; min-height: 44px; color: var(--content); }
    details p { margin: 0 0 8px; }
    .token-paths { display: grid; gap: 6px; }
    .token-paths > div { display: grid; gap: 2px; }
    .path-label { font-size: 11px; font-weight: 600; color: var(--content); }
    code { user-select: text; overflow-wrap: anywhere; font-size: 12px; }
    @media (max-width: 480px) { .login-shell { padding: 16px; } .login-card { padding: 24px 20px; } .secret { flex-wrap: wrap; } .secret input { flex: 1 1 100%; } }
</style>
