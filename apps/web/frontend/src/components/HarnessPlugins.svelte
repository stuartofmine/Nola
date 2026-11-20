<script lang="ts">
    import { onMount, tick } from 'svelte';
    import { api as base, errorMessage } from '../lib/api';
    import { desktopPluginPath, desktopPluginsChanged } from '../lib/desktopPluginPages';
    import { locale, t } from '../lib/i18n';
    import { dismissibleDisclosure } from '../lib/dismissibleDisclosure';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import PrototypeIcon from './PrototypeIcon.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';

    type Plugin = {source: string; pages: {id:string; title:string}[]; links: {title:string; url:string}[]};
    type Profile = {runtime:'pi'|'dsh'; profile:string; available:boolean; packages:Plugin[]; error?:string};
    const api = workspacePageApi(base);
    const host = window.location.host;
    let profiles = $state<Profile[]>();
    let chosen = $state('');
    let draft = $state('');
    let adding = $state(false);
    let busy = $state(false);
    let error = $state('');
    let gate: ConfirmationHandle;
    let field = $state<HTMLInputElement>();
    let addButton = $state<HTMLButtonElement>();
    let list = $state<HTMLUListElement>();
    const selected = $derived(profiles?.find(profile => profile.profile === chosen));
    const entries = $derived(profiles?.flatMap(profile => profile.packages.map(plugin => ({profile,plugin}))) ?? []);
    const usable = $derived(profiles?.some(profile => profile.available));
    const profileErrors = $derived(profiles?.filter(profile => profile.error) ?? []);

    function label(profile: Profile) {
        return profile.runtime === 'pi' ? 'Pi' : 'DeepSeek Harness · ' + (profile.profile === 'web' ? 'Web' : t($locale,'plugin.terminal'));
    }
    function packageInfo(source: string) {
        const registryName = source.replace(/^npm:/,'');
        const npm = /^((?:@[\w.-]+\/)?[\w.-]+)(?:@([^/]+))?$/.exec(registryName);
        if (npm) return {name:npm[1],version:npm[2]};
        return {name:source.split(/[\\/]/).filter(Boolean).at(-1) ?? source,version:undefined};
    }
    async function read() {
        const response = await api<{profiles:Profile[]}>('/api/harness-plugins', {
            method:'POST', body:JSON.stringify({action:'library'}),
        });
        if (!Array.isArray(response.profiles)) throw Error(t($locale,'plugin.readError'));
        profiles = response.profiles;
    }
    async function refresh() {
        if (busy) return;
        busy = true;
        error = '';
        try { await read(); }
        catch (cause) { profiles = undefined; error = errorMessage(cause); }
        finally { busy = false; }
    }
    async function begin() {
        if (busy || !usable) return;
        chosen = profiles?.find(p => p.available)?.profile ?? '';
        draft = ''; error = ''; adding = true;
        await tick(); field?.focus();
    }
    async function end() {
        adding = false; draft = '';
        await tick(); addButton?.focus();
    }
    async function change(action:'install'|'remove', profile:Profile, source:string) {
        if (busy || !profile.available || !source) return;
        const verb = t($locale,action === 'install' ? 'plugin.install' : 'plugin.remove');
        const accepted = await gate.ask({
            title: verb + ' · ' + source,
            description: [label(profile), host, action === 'install' ? t($locale,'plugin.trust') : undefined].filter(Boolean).join(' · '),
            confirmLabel:verb, destructive:action === 'remove',
        });
        if (!accepted) return;
        const previous = new Set(profile.packages.map(plugin => plugin.source));
        busy = true; error = '';
        try {
            await api('/api/harness-plugins',{method:'POST', body:JSON.stringify({runtime:profile.runtime, profile:profile.profile, action, source})});
            if (action === 'install') adding = false;
        } catch (cause) { error = errorMessage(cause); }
        try { await read(); }
        catch (cause) { profiles = undefined; error = [error,errorMessage(cause)].filter(Boolean).join(' '); }
        finally {
            busy = false;
            window.dispatchEvent(new Event(desktopPluginsChanged));
            if (action === 'remove' && !error) { await tick(); addButton?.focus(); }
            if (action === 'install' && !adding) {
                draft = '';
                await tick();
                const added = Array.from(list?.children ?? []).find(row => row instanceof HTMLElement &&
                    row.dataset.profile === profile.profile && !previous.has(row.dataset.source ?? ''));
                if (added instanceof HTMLElement) (added.querySelector('a') ?? added).focus();
                else addButton?.focus();
            }
        }
    }
    onMount(() => { void refresh(); });
</script>
<ConfirmationDialog bind:this={gate}/>
<section class="library" aria-label={t($locale,'plugin.title')}>
    <header>
        <h1>{t($locale,adding ? 'plugin.add' : 'plugin.title')}</h1>
        {#if !adding && usable}
            <button class="primary add" bind:this={addButton} disabled={busy} onclick={begin}><PrototypeIcon name="plus" size={16}/>{t($locale,'plugin.add')}</button>
        {/if}
    </header>
    {#if adding}
        <form onsubmit={e => {e.preventDefault(); if(selected) void change('install',selected,draft.trim());}}>
            <label>{t($locale,'plugin.source')}<input bind:this={field} bind:value={draft} placeholder="@scope/package@1.0.0" required maxlength="214" disabled={busy} autocapitalize="off" spellcheck="false"/></label>
            <label>{t($locale,'plugin.destination')}<select bind:value={chosen} disabled={busy}>
                {#each profiles ?? [] as profile}<option value={profile.profile} disabled={!profile.available}>{label(profile)}{profile.available ? '' : ' · '+t($locale,'plugin.runtimeMissing')}</option>{/each}
            </select></label>
            <div class="form-actions"><button type="button" disabled={busy} onclick={end}>{t($locale,'shell.cancel')}</button><button class="primary" disabled={busy || !selected?.available || !draft.trim()}>{t($locale,'plugin.install')}</button></div>
        </form>
    {:else}
        {#if entries.length}
            <ul class="collection" bind:this={list} aria-label={t($locale,'plugin.title')}>
                {#each entries as {profile,plugin} (JSON.stringify([profile.profile,plugin.source]))}
                    {@const info = packageInfo(plugin.source)}
                    <li tabindex="-1" data-source={plugin.source} data-profile={profile.profile}>
                        <div class="resource">
                            {#each plugin.pages as page (page.id)}<a class="page-link" href={desktopPluginPath(page.id)}>{page.title}</a>{/each}
                            {#each plugin.links as link}<a class="site-link" href={link.url} target="_blank" rel="noopener noreferrer">{link.title}<small>{new URL(link.url).host}</small></a>{/each}
                            {#if !plugin.pages.length && !plugin.links.length}<strong>{info.name}</strong>{/if}
                            <small class="metadata">{label(profile)}{#if (plugin.pages.length || plugin.links.length) && ![...plugin.pages,...plugin.links].some(page => page.title === info.name)} · {info.name}{/if}{#if info.version} · {info.version}{/if}{#if !profile.available} · {t($locale,'plugin.runtimeMissing')}{/if}</small>
                        </div>
                        <details class="row-actions" use:dismissibleDisclosure>
                            <summary aria-label={t($locale,'plugin.more',{name:plugin.pages[0]?.title ?? plugin.links[0]?.title ?? info.name})}><PrototypeIcon name="more" size={18}/></summary>
                            <div class="menu"><button disabled={busy || !profile.available} onclick={event => {
                                const details = event.currentTarget.closest('details');
                                if (details) { details.open = false; details.querySelector('summary')?.focus(); }
                                void change('remove',profile,plugin.source);
                            }}>{t($locale,'plugin.remove')}</button></div>
                        </details>
                    </li>
                {/each}
            </ul>
        {:else if profiles && !error && !profileErrors.length}
            <p class="empty">{t($locale,usable ? 'plugin.empty' : 'plugin.noRuntime')}</p>
        {/if}
    {/if}
    {#if busy}<p class="feedback" role="status">{t($locale,'plugin.working')}</p>{/if}
    {#if error || profileErrors.length || (profiles && !usable && !adding)}
        <div class="feedback">{#if error || profileErrors.length}<div role="alert">{#if error}<p>{error}</p>{/if}{#each profileErrors as profile}<p>{label(profile)} · {profile.error}</p>{/each}</div>{/if}
            <button disabled={busy} onclick={refresh}>{t($locale,'plugin.refresh')}</button>
        </div>
    {/if}
</section>
<style>
    .library { max-width:52rem; margin-inline:auto; padding:clamp(1rem,4vw,2.5rem); }
    header,.form-actions,li,.add { display:flex;align-items:center;gap:.6rem; }
    header { justify-content:space-between;gap:1rem;margin-bottom:1.35rem; }
    h1 { margin:0;font-size:clamp(1.45rem,2.25vw,2rem);line-height:1.25; }
    .library button,.library input,.library select { min-height:44px; }
    .add { white-space:nowrap; }
    .collection { list-style:none;margin:0;padding:0; }
    li { justify-content:space-between;padding:.7rem .1rem;border-bottom:1px solid var(--border); }
    li:last-child { border-bottom:0; }
    .resource { display:grid;align-content:center;min-width:0;overflow-wrap:anywhere; }
    .resource strong,.page-link,.site-link { font-weight:600;line-height:1.4; }
    .resource a { display:flex;align-items:center;flex-wrap:wrap;gap:.4em;min-height:44px;width:fit-content;color:var(--content);text-decoration:underline;text-decoration-color:var(--border);text-underline-offset:.24em; }
    .metadata,.site-link small,.empty,.feedback { color:var(--muted); }
    small { font-size:.83rem;font-weight:400; }
    .row-actions { position:relative;flex:none; }
    .row-actions summary { display:grid;place-items:center;list-style:none;cursor:pointer;width:44px;height:44px;color:var(--muted);border-radius:.6rem; }
    .row-actions summary::-webkit-details-marker { display:none; }
    .menu { position:absolute;inset-inline-end:0;top:100%;z-index:2;min-width:8rem;padding:.22rem;background:var(--surface);border:1px solid var(--border);border-radius:.6rem;box-shadow:0 4px 16px #0002; }
    .menu button { width:100%;text-align:start;border:0;background:transparent; }
    @media(hover:hover){.row-actions summary:hover,.resource a:hover { color:var(--accent);background:var(--accent-subtle); }}
    .library :is(a,button,input,select,summary):focus-visible { outline:2px solid var(--accent);outline-offset:2px; }
    form { display:grid;gap:1.25rem;max-width:32rem; }
    label { display:grid;gap:.45rem;font-size:.93rem; }
    .form-actions { justify-content:flex-end; }
    .empty,.feedback { margin:.8rem 0; }
    .feedback p { margin:.5rem 0; }
</style>
