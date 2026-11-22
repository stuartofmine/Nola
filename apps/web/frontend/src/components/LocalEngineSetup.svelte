<script lang="ts">
    import { onDestroy, untrack } from 'svelte';
    import { api as unscopedApi, ApiError, errorMessage } from '../lib/api';
    import { t, locale } from '../lib/i18n';
    import { latestRequest } from '../lib/latestRequest';
    import { engineName, engineGuideUrl, type EngineStatus, type LocalEngine } from '../lib/modelAcquisition';
    import { workspacePageApi } from '../lib/workspacePageLocation';

    let {engine = $bindable(), alternative, runningName, nativeRuntime, onStatus, openConnections}: {
        engine: LocalEngine; alternative?: LocalEngine; runningName?: string; nativeRuntime: string;
        onStatus: (status: EngineStatus) => void; openConnections: () => void;
    } = $props();
    const api = workspacePageApi(unscopedApi);
    const reads = latestRequest<{engineStatus:EngineStatus}>();
    let status = $state<EngineStatus>();
    let checking = $state(false);
    let installing = $state<LocalEngine>();
    let error = $state('');
    let alive = true;
    let checkVersion = 0;
    let panel: HTMLElement;
    export function reveal() {
        panel?.scrollIntoView({block:'center'});
        panel?.focus({preventScroll:true});
    }
    const current = $derived(status?.engine === engine ? status : undefined);
    const guide = $derived(engineGuideUrl(current?.guideUrl));
    const reasonKey = $derived.by(() => {
        const reason = current?.reason || 'manual';
        if (reason === 'manual' && nativeRuntime) return engine === 'vllm' ? 'local.setup.windowsVllm' : 'local.setup.windowsLlama';
        if (engine === 'vllm' && reason === 'unsupported') return 'local.setup.unsupportedVllm';
        return 'local.setup.' + (['installable','homebrew_missing','unsupported','manual','install_on_host','invalid_override'].includes(reason) ? reason : 'manual');
    });

    $effect(() => {
        const selected = engine;
        untrack(() => { status = undefined; error = ''; void check(selected); });
    });
    onDestroy(() => { alive = false; reads.cancel(); });

    async function check(selected: LocalEngine) {
        const version = ++checkVersion;
        checking = true; error = '';
        try {
            const value = await reads.run(signal => api('/api/local-models/engine-status?engine=' + encodeURIComponent(selected),{signal}));
            if (value && alive && selected === engine) receive(value.engineStatus,selected);
        } catch (cause) {
            if (alive && selected === engine) error = errorMessage(cause);
        } finally {
            if (alive && version === checkVersion && selected === engine) checking = false;
        }
    }

    function receive(next: EngineStatus, selected: LocalEngine) {
        if (next?.engine !== selected || typeof next.ready !== 'boolean' || typeof next.canInstall !== 'boolean') {
            throw Error(t($locale, 'local.setup.checkFailed'));
        }
        if (selected === engine) status = next;
        onStatus(next);
    }

    async function install() {
        if (!current?.canInstall || current.ready || installing || checking) return;
        const selected = engine;
        installing = selected; error = '';
        try {
            const value = await api<{engineStatus:EngineStatus}>('/api/local-models/setup-engine',{
                method:'POST',body:JSON.stringify({engine:selected})
            });
            if (alive) {
                if (selected === engine) { ++checkVersion; reads.cancel(); checking = false; }
                receive(value.engineStatus, selected);
            }
        } catch (cause) {
            if (alive && selected === engine) {
                if (cause instanceof ApiError && cause.status === 403 && cause.code === 'install_on_host' && current) {
                    receive({...current,canInstall:false,reason:'install_on_host'},selected);
                } else error = errorMessage(cause);
            }
        } finally {
            if (alive) installing = undefined;
        }
    }
</script>

<section bind:this={panel} tabindex="-1" class="engine-setup" aria-label={t($locale, 'local.engine_status')} aria-busy={checking || installing === engine}>
    <div class="status" data-local-runtime-host>
        <strong role="status">{runningName ? t($locale, 'local.running_model', {name:runningName})
            : t($locale, installing === engine ? 'local.setup.installing' : !current ? (error ? 'local.setup.checkFailed' : 'local.setup.checking')
            : current.ready ? 'local.engineReady' : 'local.engineMissing', {engine:engineName(engine)})}</strong>
        {#if nativeRuntime && engine === 'llama.cpp'}<small class="desktop-native-runtime">{nativeRuntime}</small>{/if}
    </div>
    {#if current && !current.ready && installing !== engine}<p>{t($locale, reasonKey)}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="setup-actions">
        {#if current && !current.ready}
            {#if current.canInstall}<button type="button" disabled={!!installing || checking} onclick={install}>{t($locale, 'local.setup.install', {engine:engineName(engine)})}</button>{/if}
            {#if guide}<a href={guide} target="_blank" rel="noopener noreferrer">{t($locale, current.reason === 'homebrew_missing' ? 'local.setup.getBrew' : 'local.setup.guide')}</a>{/if}
            {#if alternative}<button class="text-action" type="button" onclick={() => { if (alternative) engine = alternative; }}>{t($locale, 'local.setup.switch', {engine:engineName(alternative)})}</button>{/if}
            {#if engine === 'vllm' && (current.reason === 'unsupported' || current.reason === 'manual')}<button class="text-action" type="button" onclick={openConnections}>{t($locale, 'local.setup.connect')}</button>{/if}
        {/if}
        <button type="button" class="text-action" disabled={checking || installing === engine} onclick={() => check(engine)}>{t($locale, 'local.setup.recheck')}</button>
    </div>
</section>

<style>
    .engine-setup { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 14px; border-block-end: 1px solid var(--border); padding: 12px 0; }
    .status { display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: baseline; font-size: .875rem; }
    .status small, p { color: var(--muted); font-size: .8125rem; }
    p { margin: 0; overflow-wrap: anywhere; }
    p.error { color: var(--danger); }
    .setup-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 12px; }
    .setup-actions button, .setup-actions a { display: inline-flex; align-items: center; min-height: 34px; font-size: .8125rem; }
    .text-action { border: 0; background: transparent; padding: 4px 6px; }
    .text-action:hover:enabled { background: var(--hover); }
    .setup-actions a:hover, .setup-actions a:focus-visible { text-decoration: underline; }
    @media (pointer: coarse) { .setup-actions button, .setup-actions a { min-height: 44px; } }
</style>
