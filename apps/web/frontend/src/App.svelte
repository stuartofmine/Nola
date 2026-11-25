<script lang="ts">
    import PrototypeIcon from './components/PrototypeIcon.svelte';
    import ConfirmationDialog from './components/ConfirmationDialog.svelte';
    import ToastHost from './components/ToastHost.svelte';
    import type { ConfirmationHandle } from './lib/confirmation';
    import { onMount, setContext, tick } from 'svelte';
    import { loadWorkspaceWidths } from './lib/workspaceLayout';
    let viewportWidth = $state(1280);
    let confirmation: ConfirmationHandle;
    import { mobileInformationTabs } from './lib/dataSources';
    import { resolveSettingsCategory } from './lib/settingsCategories';
    import { pageLoader } from './lib/pageRoutes';
    import { observedPageLoad } from './lib/pageLoad';
    function loadPage(path: string) {
        const loader = pageLoader(path);
        return loader ? observedPageLoad<Awaited<ReturnType<typeof loader>>>(path, loader, failure => console.error('[Nola] page load failed', JSON.stringify(failure))) : undefined;
    }
    import { locale, t } from './lib/i18n';
    import { createWorkspaceDrafts, workspaceDraftsContext } from './lib/workspaceDrafts';
    import { initialWorkspaceLocation, workspaceClick, workspaceDestination, workspaceNavigationEvent, workspaceContentNavigationEvent } from './lib/workspaceNavigation';
    const drafts = createWorkspaceDrafts();
    setContext(workspaceDraftsContext, drafts);
    const initialLocation = initialWorkspaceLocation(window.location.href, window.innerWidth);
    if (initialLocation.href !== window.location.href) window.history.replaceState(window.history.state, '', initialLocation);
    function isMobileDetail(target: URL) {
        const tab = target.searchParams.get('tab');
        return target.pathname === '/data-sources' && (tab === 'usage' || mobileInformationTabs.some(item => item === tab))
            || target.pathname === '/settings' && !!resolveSettingsCategory(target.searchParams.get('section'));
    }
    let loadingDetail = $state(isMobileDetail(initialLocation));
    let path = $state(initialLocation.pathname);
    let routeVersion = $state(0);
    import { darkAppearance, toggleAppearance, initializeAppearance } from './lib/appearance';
    import { dismissKeyboardOnBackgroundTap } from './lib/touchKeyboard';
    onMount(initializeAppearance);
    onMount(dismissKeyboardOnBackgroundTap);
    const primaryRoot = $derived(['/today', '/conversations', '/activity', '/settings', '/data-sources'].includes(path));
    let page = $state(loadPage(initialLocation.pathname));
    let content: HTMLDivElement;
    async function mayLeave() {
        // Preserve all existing editor guards when navigation no longer unloads
        // the document. The browser itself only prompts on trusted unload events.
        const event = new Event('beforeunload', {cancelable: true});
        window.dispatchEvent(event);
        if (!event.defaultPrevented) return true;
        if (confirmation) {
            return confirmation.ask({
                title: t($locale, 'shell.confirm'),
                description: t($locale, 'shell.navigationDiscard'),
                confirmLabel: t($locale, 'shell.confirm'),
                destructive: true,
            });
        }
        return false;
    }
    function show(target: URL) {
        path = target.pathname; loadingDetail = isMobileDetail(target); routeVersion++;
        page = loadPage(path);
        window.scrollTo(0, 0);
    }
    function pageReady(node: HTMLElement) {
        const generation = routeVersion;
        let cancelled = false, frame = 0;
        if (generation > 0) void tick().then(() => {
            if (cancelled) return;
            frame = requestAnimationFrame(() => {
                if (cancelled || generation !== routeVersion || !node.isConnected
                    || document.querySelector('dialog:modal')) return;
                const focused = document.activeElement;
                if (focused instanceof HTMLInputElement || focused instanceof HTMLTextAreaElement
                    || focused instanceof HTMLSelectElement || (focused instanceof HTMLElement && focused.isContentEditable)) return;
                (document.getElementById('page-main') ?? content)?.focus({preventScroll: true});
            });
        });
        return {destroy() { cancelled = true; if (frame) cancelAnimationFrame(frame); }};
    }
    onMount(() => {
        try { loadWorkspaceWidths(localStorage); } catch { /* Optional browser preference. */ }
        let index = 0;
        let restoring = false;
        history.replaceState({...history.state, nolaWorkspaceIndex: index}, '');
        const navigate = (href: string) => {
            const target = workspaceDestination(href, location.href);
            if (!target) return false;
            if (viewportWidth <= 760 && target.pathname === '/chat' && !target.searchParams.has('conversation')) target.pathname = '/conversations';
            if (path === '/chat' && viewportWidth > 760 && (target.pathname !== '/chat' || !target.searchParams.has('conversation'))) {
                const content = new CustomEvent(workspaceContentNavigationEvent, {detail: target.href, cancelable: true});
                if (!window.dispatchEvent(content)) return true;
            }
            void mayLeave().then(allowed => {
                if (!allowed) return;
                history.pushState({nolaWorkspaceIndex: ++index}, '', target);
                show(target);
            });
            return true;
        };
        const click = (event: MouseEvent) => {
            const anchor = event.target instanceof Element ? event.target.closest('a') : null;
            if (anchor instanceof HTMLAnchorElement && workspaceClick(event, anchor) && navigate(anchor.href)) event.preventDefault();
        };
        const requested = (event: Event) => {
            if (event instanceof CustomEvent && typeof event.detail === 'string' && navigate(event.detail)) event.preventDefault();
        };
        const pop = (event: PopStateEvent) => {
            if (restoring) { restoring = false; return; }
            const next = event.state?.nolaWorkspaceIndex;
            if (!Number.isSafeInteger(next)) { location.reload(); return; }
            // Native fragment entries keep the same workspace index. They do
            // not discard an editor and must not remount the current page.
            if (next === index) return;
            const prevIndex = index;
            void mayLeave().then(allowed => {
                if (!allowed) { restoring = true; history.go(prevIndex - next); return; }
                index = next;
                show(new URL(location.href));
            });
        };
        const unload = (event: BeforeUnloadEvent) => {
            if (event.isTrusted && drafts.needsUnloadConfirmation) { event.preventDefault(); event.returnValue = ''; }
        };
        document.addEventListener('click', click);
        window.addEventListener(workspaceNavigationEvent, requested);
        window.addEventListener('popstate', pop);
        window.addEventListener('beforeunload', unload);
        return () => {
            document.removeEventListener('click', click);
            window.removeEventListener(workspaceNavigationEvent, requested);
            window.removeEventListener('popstate', pop);
            window.removeEventListener('beforeunload', unload);
            drafts.clear();
        };
    });
</script>

{#snippet pageContent()}
    {#key routeVersion}{#await page}
        <div class="content" class:loading-detail={loadingDetail} role="status">{t($locale, 'shell.pageLoading')}</div>
    {:then module}
        {#if module}<module.default /><span hidden aria-hidden="true" use:pageReady></span>{:else}<div class="content" role="alert">{t($locale, 'shell.pageLoadFailed')}</div>{/if}
    {:catch}
        <div class="content" role="alert"><p>{t($locale, 'shell.pageLoadFailed')}</p><button onclick={() => window.location.reload()}>{t($locale, 'shell.retry')}</button></div>
    {/await}{/key}
{/snippet}

<svelte:window bind:innerWidth={viewportWidth} />
<ConfirmationDialog bind:this={confirmation} />
<ToastHost />
<a class="skip-content" href="#page-main" onclick={event => { event.preventDefault(); (document.getElementById('page-main') ?? content)?.focus(); }}>{t($locale, 'shell.skipContent')}</a>
<div bind:this={content} tabindex="-1" class="workspace-content" class:with-mobile-navigation={path !== '/chat' && path !== '/login'}>
{#if primaryRoot}<div class="mobile-brand-bar"><a href="/today" aria-label="Nola"><span class="nola-mark">n</span><strong>Nola</strong></a><button type="button" aria-label={t($locale, 'shell.toggleAppearance')} onclick={toggleAppearance}><svg aria-hidden="true" viewBox="0 0 24 24">{#if $darkAppearance}<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1"/>{:else}<path d="M20 14A8 8 0 0 1 10 4a8 8 0 1 0 10 10Z"/>{/if}</svg></button></div>{/if}
{#if path === '/login' || path === '/chat' || path === '/conversations' || path === '/today'}
    {@render pageContent()}
{:else}
<div class="management-shell">
    {#if !primaryRoot || viewportWidth > 760}<header class="management-navigation">
        <button type="button" aria-label={t($locale, 'shell.back')} onclick={() => {
            if (path === '/data-sources' && new URL(window.location.href).searchParams.has('tab')) window.dispatchEvent(new CustomEvent(workspaceNavigationEvent, {detail: '/data-sources', cancelable: true}));
            else if ((history.state?.nolaWorkspaceIndex ?? 0) > 0) history.back();
            else window.dispatchEvent(new CustomEvent(workspaceNavigationEvent, {detail: viewportWidth > 760 ? '/chat' : '/today', cancelable: true}));
        }}><PrototypeIcon name="back" size={18} /></button>
        <a href={viewportWidth > 760 ? '/chat' : '/conversations'}>{t($locale, 'chatInspector.list.conversations')}</a>
    </header>{/if}
    <main id="page-main" tabindex="-1">{@render pageContent()}</main>
</div>
{/if}
{#if path !== '/chat' && path !== '/login'}
<nav class="mobile-primary-navigation" aria-label={t($locale, 'shell.primaryNavigation')}>
    <a href="/conversations" aria-label={t($locale, 'shell.conversations')} aria-current={path === '/conversations' ? 'page' : undefined}><PrototypeIcon name="chat" size={19} /><span>{t($locale, 'shell.mobileConversations')}</span></a>
    <a href="/activity" aria-current={['/activity', '/results', '/scheduled-actions'].includes(path) ? 'page' : undefined}><PrototypeIcon name="clock" size={19} /><span>{t($locale, 'shell.mobileActivity')}</span></a>
    <a href="/data-sources" aria-label={t($locale, 'shell.information')} aria-current={path === '/data-sources' ? 'page' : undefined}><PrototypeIcon name="folder" size={19} /><span>{t($locale, 'shell.mobileInformation')}</span></a>
    <a href="/settings" aria-current={['/settings', '/models', '/capabilities', '/model-test'].includes(path) ? 'page' : undefined}><PrototypeIcon name="settings" size={19} /><span>{t($locale, 'shell.settings')}</span></a>
</nav>
{/if}
</div>

<style>
    .mobile-brand-bar { display: none; }
    .workspace-content:has(.loading-detail, :global(.information-back), :global(.settings-back)) .mobile-brand-bar { display:none; }
    @media(max-width:760px) {
        .mobile-brand-bar { display:flex; align-items:center; justify-content:space-between; padding:calc(20px + env(safe-area-inset-top)) 20px 12px; min-height:80px; }
        .mobile-brand-bar a { display:flex; align-items:center; gap:14px; color:var(--content); }
        .mobile-brand-bar strong { font-size:22px; font-weight:600; letter-spacing:-.5px; }
        .nola-mark { font: bold 34px Georgia,serif; }
        .mobile-brand-bar button { width:44px; height:44px; padding:10px; border:0; background:transparent; color:var(--muted); }
        .mobile-brand-bar svg { width:20px; height:20px; fill:none; stroke:currentColor; stroke-width:1.5; }
    }
    @media(max-width:359px) { .mobile-brand-bar { padding-inline:16px; } }

    .mobile-primary-navigation { display: none; }
    @media (max-width: 760px) {
        .with-mobile-navigation { display: flex; flex-direction: column; min-height: 100dvh; padding-bottom: calc(58px + max(7px, env(safe-area-inset-bottom, 0px))); }
        .with-mobile-navigation .management-shell { flex: 1; min-height: 0; }
        .mobile-primary-navigation { position: fixed; inset: auto 0 0; z-index: 30; display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 4px; padding: 5px 12px max(7px, env(safe-area-inset-bottom, 0px)); background: var(--canvas); border-top: 1px solid var(--border); }
        .mobile-primary-navigation a { min-width: 0; min-height: 52px; display: flex; flex-direction: column; gap: 4px; align-items: center; justify-content: center; padding: 4px 2px; font-size: 12px; line-height: 16px; color: var(--muted); text-decoration: none; border: 1px solid transparent; border-radius: 8px; }
        .mobile-primary-navigation a[aria-current] { color: var(--content); font-weight: 600; background: var(--panel); }
    }

    @media (max-width: 359px) { .mobile-primary-navigation { padding-inline: 8px; } }
    .mobile-primary-navigation span { max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .workspace-content:focus, #page-main:focus { outline: none; }
    .management-shell { display: flex; flex-direction: column; min-height: 100dvh; }
    .management-shell > main { flex: 1; display: flex; flex-direction: column; min-height: 0; }
    .management-navigation { min-height: 48px; display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: env(safe-area-inset-top, 0px) 16px 0; border-bottom: 1px solid var(--border); }
    .management-navigation button, .management-navigation a { min-height: 44px; display: flex; align-items: center; color: var(--muted); }
    .management-navigation button { min-width: 44px; justify-content: center; border: 0; background: transparent; }
    .management-navigation a { text-decoration: none; font-size: .875rem; }
</style>
