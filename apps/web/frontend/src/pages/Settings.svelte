<script lang="ts">
    import { appearance } from '../lib/appearance';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspaceElements } from '../lib/workspacePageLocation';
    const pageElements = workspaceElements();
    import DesktopSettings from '../components/DesktopSettings.svelte';
    import SoftwareUpdateSettings from '../components/SoftwareUpdateSettings.svelte';
    import LanguagePreference from '../components/LanguagePreference.svelte';
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    $effect(() => { if (pageLocation.active()) untrack(resize); });
    import { onMount, untrack, tick } from 'svelte';
    import { settingsCategories as categories, resolveSettingsCategory, type SettingsCategory as Category } from '../lib/settingsCategories';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { latestRequest } from '../lib/latestRequest';
    import { settingsSession, logoutConfirmed } from '../lib/settingsSession';
    import { visibleTabScrollLeft } from '../lib/settingsTabVisibility.ts';
    import type { SettingsSession } from '../lib/settingsSession';
    import ConfirmationDialog from '../components/ConfirmationDialog.svelte';
    import AgentAccessSettings from '../components/AgentAccessSettings.svelte';
    import MessagingSettings from '../components/MessagingSettings.svelte';
    import HostDiagnostics from '../components/HostDiagnostics.svelte';
    import ProviderSettings from '../components/ProviderSettings.svelte';
    import MemoryDefaultSettings from '../components/MemoryDefaultSettings.svelte';
    import RetrievalDefaultSettings from '../components/RetrievalDefaultSettings.svelte';
    import NotificationSettings from '../components/NotificationSettings.svelte';
    let confirmation: ConfirmationDialog;
    let session = $state<SettingsSession>();
    let loading = $state(false), busy = $state(false);
    let error = $state('');
    let disposed = false;
    let selected = $state<Category>('general');
    let narrow = $state(window.innerWidth <= 760);
    let overview = $state(!resolveSettingsCategory(pageLocation.read().searchParams.get('section')));
    let visited = $state<Set<Category>>(new Set(['general']));
    function select(category: Category) {
        selected = category; overview = false;
        visited = new Set([...visited, category]);
        const url = pageLocation.read();
        url.searchParams.set('section', category);
        pageLocation.replace(url);
        revealTab(category);
    }
    function showOverview() {
        overview = true;
        const url = pageLocation.read(); url.searchParams.delete('section'); pageLocation.replace(url);
    }
    function openCategory(event: MouseEvent, category: Category) {
        if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
        event.preventDefault(); select(category);
    }
    function revealTab(category: Category) {
        if (!pageLocation.active() || !narrow) return;
        requestAnimationFrame(() => {
            if (!pageLocation.active() || disposed) return;
            const rail = pageElements.query<HTMLElement>('.settings-sidebar [role="tablist"]');
            const tab = pageElements.element(`settings-tab-${category}`);
            if (!rail || !tab) return;
            const railBounds = rail.getBoundingClientRect();
            const tabBounds = tab.getBoundingClientRect();
            rail.scrollLeft = visibleTabScrollLeft(rail.scrollLeft, railBounds.left, railBounds.right, tabBounds.left, tabBounds.right);
        });
    }
    function resize() {
        if (!pageLocation.active() || disposed) return;
        narrow = window.innerWidth <= 760;
        revealTab(selected);
    }
    function navigate(event: KeyboardEvent, category: Category) {
        const index = categories.indexOf(category);
        let next: number;
        if (event.key === 'Home') next = 0;
        else if (event.key === 'End') next = categories.length - 1;
        else if (event.key === 'ArrowDown' || event.key === 'ArrowRight') next = (index + 1) % categories.length;
        else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') next = (index + categories.length - 1) % categories.length;
        else return;
        event.preventDefault(); select(categories[next]);
        pageElements.element(`settings-tab-${categories[next]}`)?.focus();
    }
    const reads = latestRequest<SettingsSession>();
    async function load() {
        if (busy || disposed) return;
        loading = true; error = '';
        try {
            const result = await reads.run(async signal => settingsSession(await api('/api/web/session', { signal })));
            if (result && !disposed) session = result;
        } catch { if (!disposed) error = 'settings.sessionError'; }
        finally { if (!disposed) loading = false; }
    }
    onMount(() => {
        narrow = window.innerWidth <= 760;
        const section = pageLocation.read().searchParams.get('section');
        const destination = resolveSettingsCategory(section);
        if (destination) select(destination);
        if (section === 'notifications') void tick().then(() => {
            if (!disposed && pageLocation.active()) pageElements.element('settings-notifications')?.scrollIntoView({block: 'start'});
        });
        void load(); return () => { disposed = true; reads.cancel(); };
    });
    async function logout() {
        if (busy || !session?.sessionAuthenticated) return;
        busy = true;
        const approved = await confirmation.ask({ title: t($locale, 'settings.logoutTitle'), description: t($locale, 'settings.logoutBody'), confirmLabel: t($locale, 'settings.logout'), destructive: true });
        if (!approved || disposed) { busy = false; return; }
        reads.cancel(); error = '';
        try {
            const result = await api('/api/web/logout', { method: 'POST', body: '{}' });
            if (disposed) return;
            if (!logoutConfirmed(result)) throw new Error('Unconfirmed logout');
            session = { ...session, sessionAuthenticated: false, authenticated: session.localTrust };
            window.location.assign('/login?logged_out=1');
        } catch { if (!disposed) error = 'settings.logoutError'; }
        finally { if (!disposed) busy = false; }
    }
</script>

<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'shell.settings')} — Nola</title>{/if}</svelte:head>
<svelte:window onresize={resize} />
<ConfirmationDialog bind:this={confirmation} />
{#if !overview}<div class="settings-back"><button type="button" onclick={showOverview}><span aria-hidden="true">‹</span>{t($locale, 'shell.settings')}</button></div>{/if}
<header class="page-header" class:mobile-overview={narrow && overview}><div><h1 id="settings-mobile-title">{t($locale, !overview ? `settings.category.${selected}` : 'shell.settings')}</h1><p class="muted">{t($locale, overview ? 'settings.overviewHint' : narrow ? `settings.categoryHint.${selected}` : 'settings.subtitle')}</p></div></header>
<div class="content management-content settings-layout" class:mobile-overview={narrow && overview} class:overview>
    {#if overview}<nav class="settings-overview" aria-label={t($locale, 'settings.sections')}>
        {#each categories as category}<a href={`/settings?section=${category}`} onclick={event => openCategory(event, category)}><span><strong>{t($locale, `settings.category.${category}`)}</strong><small>{t($locale, `settings.categoryHint.${category}`)}</small></span><span aria-hidden="true">›</span></a>{/each}
    </nav>{/if}
    <aside class="settings-sidebar" hidden={overview}>
        <div role="tablist" aria-label={t($locale, 'settings.sections')} aria-orientation={narrow ? 'horizontal' : 'vertical'}>
            {#each categories as category}
                <button role="tab" id={`settings-tab-${category}`} aria-controls={`settings-panel-${category}`} aria-selected={selected === category} tabindex={selected === category ? 0 : -1} onclick={() => select(category)} onkeydown={event => navigate(event, category)}>{t($locale, `settings.category.${category}`)}</button>
            {/each}
        </div>
        <nav class="settings-links" aria-label={t($locale, 'settings.workspace')}>
            <a href="/capabilities">{t($locale, 'shell.nav.capabilities')}</a>
            <a href="/scheduled-actions">{t($locale, 'shell.nav.automation')}</a>
        </nav>
    </aside>
    <div class="settings-panels" hidden={overview}>
    <div role={narrow ? "region" : "tabpanel"} id="settings-panel-general" aria-labelledby={narrow ? "settings-mobile-title" : "settings-tab-general"} hidden={selected !== 'general'}>
    <section class="card preferences-card"><LanguagePreference />
    <label>{t($locale, 'shell.appearance')}<select bind:value={$appearance}>
        <option value="system">{t($locale, 'shell.appearanceSystem')}</option><option value="light">{t($locale, 'shell.appearanceLight')}</option><option value="dark">{t($locale, 'shell.appearanceDark')}</option>
    </select></label></section>
    <div id="settings-notifications"><NotificationSettings /></div>
    <SoftwareUpdateSettings />
    <DesktopSettings />

    </div>
    <div role={narrow ? "region" : "tabpanel"} id="settings-panel-agent" aria-labelledby={narrow ? "settings-mobile-title" : "settings-tab-agent"} hidden={selected !== 'agent'}>
        {#if narrow}<nav class="settings-child-links" aria-label={t($locale, 'settings.workspace')}><a class="button" href="/capabilities">{t($locale, 'shell.nav.capabilities')}</a><a class="button" href="/scheduled-actions">{t($locale, 'shell.nav.automation')}</a></nav>{/if}
        {#if visited.has('agent')}<MemoryDefaultSettings /><RetrievalDefaultSettings /><div id="agent-access"><AgentAccessSettings /></div>{/if}
    </div>
    <div role={narrow ? "region" : "tabpanel"} id="settings-panel-providers" aria-labelledby={narrow ? "settings-mobile-title" : "settings-tab-providers"} hidden={selected !== 'providers'}>
        {#if visited.has('providers')}<ProviderSettings />{/if}
    </div>
    <div role={narrow ? "region" : "tabpanel"} id="settings-panel-data" aria-labelledby={narrow ? "settings-mobile-title" : "settings-tab-data"} hidden={selected !== 'data'}>
        <section class="card">
            <h2>{t($locale, 'settings.category.data')}</h2>
            <p class="muted">{t($locale, 'settings.data.detail')}</p>
            <a class="button" href="/data-sources">{t($locale, 'settings.data.open')}</a>
            <p class="muted">{t($locale, 'settings.data.managementUnavailable')}</p>
        </section>
    </div>
    <div role={narrow ? "region" : "tabpanel"} id="settings-panel-connections" aria-labelledby={narrow ? "settings-mobile-title" : "settings-tab-connections"} hidden={selected !== 'connections'}>
    <section class="card" id="host-session" aria-busy={loading || busy}>
        <div class="row between"><h2>{t($locale, 'settings.session')}</h2><button disabled={loading || busy} onclick={load}>{t($locale, 'models.reload')}</button></div>
        {#if loading}<p role="status">{t($locale, 'settings.loading')}</p>{/if}
        {#if error}<p role="alert" class="error">{t($locale, error)}</p>{/if}
        {#if session}
            <dl><dt>{t($locale, 'settings.host')}</dt><dd><code>{session.hostId}</code></dd><dt>{t($locale, 'settings.connection')}</dt><dd>{t($locale, session.sessionAuthenticated ? 'settings.signedIn' : session.localTrust ? 'settings.local' : 'settings.signedOut')}</dd></dl>
            {#if session.transportIngress === "tailcat"}<p class="muted">{t($locale, 'settings.tailcatIngress')}</p>{/if}
            {#if session.localTrust}<p class="muted">{t($locale, 'settings.localHint')}</p>{/if}
            {#if session.sessionAuthenticated}<div class="actions"><button disabled={busy || loading} onclick={logout}>{t($locale, 'settings.logout')}</button></div>
            {:else}<a class="button" href="/login?next=%2Fsettings">{t($locale, 'settings.signIn')}</a>{/if}
        {/if}
        <p class="muted">{t($locale, 'settings.tokenAuth')}</p>
    </section>
        {#if visited.has('connections')}<div id="messaging-settings"><MessagingSettings /></div>{/if}
    </div>
    <div role={narrow ? "region" : "tabpanel"} id="settings-panel-diagnostics" aria-labelledby={narrow ? "settings-mobile-title" : "settings-tab-diagnostics"} hidden={selected !== 'diagnostics'}>
        {#if visited.has('diagnostics')}<HostDiagnostics />{/if}
    </div>
    </div>
</div>
<style>
    .preferences-card :global(label){display:flex;align-items:center;justify-content:space-between;gap:24px;min-height:56px}
    .preferences-card :global(select){width:min(240px,55%)}
    .settings-layout.overview{display:block}
    .settings-sidebar[hidden]{display:none}
    .settings-layout{display:grid;grid-template-columns:180px minmax(0,1fr);gap:40px;align-items:start}
    .settings-sidebar{position:sticky;top:20px;min-width:0}.settings-sidebar [role=tablist],.settings-links{display:flex;flex-direction:column;gap:6px}
    .settings-sidebar button{border-color:transparent;background:transparent;box-shadow:none}
    .settings-sidebar button,.settings-links a{text-align:start;white-space:normal;overflow-wrap:anywhere;padding:10px 12px}
    .settings-sidebar button[aria-selected=true]{background:var(--accent-subtle);color:var(--content);border-color:transparent}
    .settings-links{margin-top:18px;border-top:1px solid var(--border);padding-top:12px}.settings-links a{color:var(--muted);text-decoration:none;border-radius:8px}
    .settings-links a:hover{background:var(--panel);color:var(--content)}.settings-panels{min-width:0}.settings-panels [role=tabpanel]{outline-offset:4px}.settings-panels > div:not([hidden]){display:grid;gap:24px}.settings-panels[hidden],.settings-panels > div[hidden]{display:none}
    @media(max-width:760px){.settings-layout{grid-template-columns:minmax(0,1fr);gap:16px}.settings-sidebar{position:static}.settings-sidebar [role=tablist]{display:flex;flex-direction:row;overflow-x:auto;scrollbar-width:thin;gap:4px;padding:4px;scroll-padding-inline:4px}.settings-sidebar button{flex:0 0 auto;text-align:center;padding:8px 12px;white-space:nowrap}.settings-links{flex-direction:row;flex-wrap:wrap;margin-top:10px;padding-top:8px;gap:2px}.settings-links a{padding:6px 8px}}
    .settings-child-links { display: flex; flex-wrap: wrap; gap: 12px; }
    .settings-overview { display: grid; gap: 0; border-top: 1px solid var(--border); }
    .settings-overview a { display: flex; align-items: center; gap: 16px; padding: 18px 0; min-height: 86px; border: 0; border-bottom: 1px solid var(--border); border-radius: 0; }
    .settings-overview a > span:first-child { flex: 1; min-width: 0; display: grid; gap: 4px; }
    .settings-overview strong { font-size: 17px; line-height: 24px; font-weight: 600; }
    .settings-overview small { font-size: 14px; line-height: 21px; }
    .settings-overview a > span:last-child { color: var(--muted); font-size: 25px; }
    .settings-back { padding: 4px 16px 0; }
    .settings-back button { min-height: 44px; padding: 4px; background: transparent; box-shadow: none; border: 0; display: flex; gap: 8px; align-items: center; }
    .settings-back span { font-size: 24px; }
    @media(max-width:760px) {
        .settings-sidebar { display: none; }
        .page-header.mobile-overview { padding: 4px 20px 22px; }
        .page-header.mobile-overview h1 { font-size: 28px; line-height: 36px; font-weight: 650; }
        .page-header.mobile-overview p { font-size: 15px; line-height: 24px; margin-top: 6px; }
        .settings-layout.mobile-overview { padding: 0 20px 28px; }
    }
    dl { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 8px 16px; }
    dt { color: var(--muted); } dd { margin: 0; overflow-wrap: anywhere; }
    #agent-access, #messaging-settings, #host-session { scroll-margin-top: 20px; }
    .settings-panels :global([data-workspace-id="agent-access"]), .settings-panels :global([data-workspace-id="messaging-settings"]), .settings-panels :global([data-workspace-id="host-session"]) { scroll-margin-top: 20px; }
</style>
