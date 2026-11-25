<script lang="ts">
    import { onMount } from 'svelte';
    import { get } from 'svelte/store';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { workspacePageApi, workspacePageLocation } from '../lib/workspacePageLocation';
    import { locale, t } from '../lib/i18n';
    import { activeApiTestModel, activeLocalTestModel, modelTestParametersUrl, type LocalTestModel } from '../lib/modelTest';
    import type { RemoteProfile } from '../lib/modelProfiles';
    import {
        cancelV2Turn,
        createV2Conversation,
        streamV2ChatMessage,
        type V2TokenUsage,
    } from '../lib/v2Chat';
    import { conversationUrl } from '../lib/chatConversations';
    import Markdown from '../components/Markdown.svelte';

    const api = workspacePageApi(unscopedApi);
    const pageLocation = workspacePageLocation();
    const componentId = $props.id();

    let target = $state<'local' | 'cloud'>('cloud');
    let compactPane = $state<'setup' | 'output'>('setup');
    let prompt = $state(t(get(locale), 'modelTest.defaultPrompt'));
    let output = $state('');
    let error = $state('');
    let loading = $state(true);
    let busy = $state(false);
    let firstTokenMs = $state<number | null>(null);
    let totalMs = $state<number | null>(null);
    let lastUsage = $state<V2TokenUsage | null>(null);
    let profiles = $state<RemoteProfile[]>([]);
    let localModels = $state<LocalTestModel[]>([]);
    let selectedLocalId = $state('');
    let selectedRemoteId = $state('');
    let activeConversationId: string | null = null;
    let controller: AbortController | null = null;
    let disposed = false;

    const installedLocalModels = $derived(localModels.filter(m => m.installed));
    const availableRemoteProfiles = $derived(
        profiles.filter(p => p.enabled !== false && p.endpoint?.enabled !== false && (!p.capabilities || p.capabilities.includes('chat'))),
    );
    const activeRemote = $derived(
        availableRemoteProfiles.find(p => p.id === selectedRemoteId) ?? activeApiTestModel(profiles) ?? availableRemoteProfiles[0],
    );
    const activeLocal = $derived(
        installedLocalModels.find(m => m.id === selectedLocalId) ?? activeLocalTestModel(localModels) ?? installedLocalModels[0],
    );
    const activeName = $derived(
        target === 'local'
            ? activeLocal?.displayName || activeLocal?.name || activeLocal?.id
            : activeRemote?.displayName || activeRemote?.modelName,
    );
    const contextWindowTokens = $derived.by(() => {
        if (target === 'cloud') {
            const raw = activeRemote as unknown as Record<string, unknown> | undefined;
            const win = Number(raw?.contextWindow ?? raw?.context_window ?? raw?.maxContextTokens ?? 0);
            return Number.isFinite(win) && win > 0 ? win : null;
        }
        const raw = activeLocal as unknown as Record<string, unknown> | undefined;
        const win = Number(raw?.contextWindow ?? raw?.context_window ?? 0);
        return Number.isFinite(win) && win > 0 ? win : null;
    });
    const usedTokens = $derived(lastUsage ? lastUsage.input_tokens + lastUsage.output_tokens : null);
    const contextFraction = $derived(
        usedTokens !== null && contextWindowTokens ? Math.min(1, usedTokens / contextWindowTokens) : null,
    );
    const canRun = $derived(!loading && !!activeName && !!prompt.trim() && !busy);

    function navigateTabs(event: KeyboardEvent) {
        const button = event.currentTarget as HTMLButtonElement;
        const tabs = Array.from(button.parentElement?.querySelectorAll<HTMLButtonElement>('button[role="tab"]:enabled') ?? []);
        const index = tabs.indexOf(button);
        if (index < 0 || !tabs.length) return;
        const next =
            event.key === 'Home'
                ? 0
                : event.key === 'End'
                  ? tabs.length - 1
                  : event.key === 'ArrowRight'
                    ? (index + 1) % tabs.length
                    : event.key === 'ArrowLeft'
                      ? (index + tabs.length - 1) % tabs.length
                      : -1;
        if (next < 0) return;
        event.preventDefault();
        tabs[next].focus();
        tabs[next].click();
    }

    function handlePromptKeydown(event: KeyboardEvent) {
        if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && !event.isComposing) {
            event.preventDefault();
            if (canRun) void run();
        }
    }

    async function load() {
        loading = true;
        error = '';
        try {
            const [remote, local] = await Promise.all([
                api<{ profiles?: RemoteProfile[] }>('/api/model-profiles'),
                api<{ models?: LocalTestModel[] }>('/api/local-models'),
            ]);
            if (disposed) return;
            profiles = remote.profiles ?? [];
            localModels = local.models ?? [];
            if (!selectedLocalId) selectedLocalId = activeLocalTestModel(localModels)?.id ?? installedLocalModels[0]?.id ?? '';
            if (!selectedRemoteId) selectedRemoteId = activeApiTestModel(profiles)?.id ?? availableRemoteProfiles[0]?.id ?? '';
            if (!availableRemoteProfiles.length && installedLocalModels.length) target = 'local';
        } catch (cause) {
            if (!disposed) error = errorMessage(cause);
        } finally {
            if (!disposed) loading = false;
        }
    }

    async function run() {
        if (!canRun) return;
        const model = target === 'local' ? activeLocal : activeRemote;
        if (!model) return;
        busy = true;
        compactPane = 'output';
        error = '';
        output = '';
        firstTokenMs = null;
        totalMs = null;
        lastUsage = null;
        const startedAt = Date.now();
        controller = new AbortController();
        try {
            const conv = await createV2Conversation(
                {
                    title: t($locale, 'modelTest.title'),
                    selectedModelSource: target === 'local' ? 'local' : 'remote',
                    selectedModelId: model.id,
                },
                controller.signal,
            );
            activeConversationId = conv.conversationId;
            await streamV2ChatMessage(
                conv.conversationId,
                prompt,
                {
                    onTextDelta: delta => {
                        if (firstTokenMs === null) firstTokenMs = Date.now() - startedAt;
                        output += delta;
                    },
                    onTurnCompleted: (_finalText, usage) => {
                        if (usage) lastUsage = usage;
                    },
                    onError: msg => {
                        error = msg;
                    },
                },
                controller.signal,
            );
            totalMs = Date.now() - startedAt;
        } catch (cause) {
            if (!controller?.signal.aborted) error = errorMessage(cause);
        } finally {
            const toDelete = activeConversationId;
            activeConversationId = null;
            controller = null;
            busy = false;
            if (toDelete) {
                void api(conversationUrl(toDelete), { method: 'DELETE' }).catch(() => {});
            }
        }
    }

    function stop() {
        controller?.abort();
        if (activeConversationId) void cancelV2Turn(activeConversationId).catch(() => {});
    }

    function clear() {
        if (busy) return;
        output = '';
        error = '';
        firstTokenMs = null;
        totalMs = null;
        lastUsage = null;
    }

    onMount(() => {
        void load();
        return () => {
            disposed = true;
            stop();
        };
    });
</script>

<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'modelTest.title')} — Nola</title>{/if}</svelte:head>
<div class="model-test-page">
    <header class="page-header">
        <div>
            <h1>{t($locale, 'modelTest.title')}</h1>
            <p class="muted">{t($locale, 'modelTest.subtitle')}</p>
        </div>
        <div class="actions">
            {#if target === 'local' ? activeLocal : activeRemote}
                <a class="button" href={modelTestParametersUrl(target, (target === 'local' ? activeLocal : activeRemote)!.id)}>
                    {t($locale, 'modelTest.parameters')}
                </a>
            {/if}
            <button type="button" class="primary" onclick={busy ? stop : run} disabled={loading || (!busy && !canRun)}>
                {t($locale, busy ? 'modelTest.stop' : 'modelTest.run')}
            </button>
        </div>
    </header>
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}

    <div class="tabs compact-panes" role="tablist" aria-label={t($locale, 'modelTest.pane')}>
        <button
            type="button"
            role="tab"
            aria-selected={compactPane === 'setup'}
            aria-controls="{componentId}-setup"
            tabindex={compactPane === 'setup' ? 0 : -1}
            onclick={() => (compactPane = 'setup')}
            onkeydown={navigateTabs}
        >
            {t($locale, 'modelTest.setup')}
        </button>
        <button
            type="button"
            role="tab"
            aria-selected={compactPane === 'output'}
            aria-controls="{componentId}-output"
            tabindex={compactPane === 'output' ? 0 : -1}
            onclick={() => (compactPane = 'output')}
            onkeydown={navigateTabs}
        >
            {t($locale, 'modelTest.output')}
        </button>
    </div>

    <div class="model-test-grid" data-active-pane={compactPane}>
        <section id="{componentId}-setup" aria-labelledby="{componentId}-setup-title" class="card setup">
            <h2 id="{componentId}-setup-title">{t($locale, 'modelTest.setup')}</h2>
            <div class="tabs" role="tablist" aria-label={t($locale, 'models.type')}>
                <button
                    type="button"
                    role="tab"
                    aria-selected={target === 'local'}
                    aria-controls="{componentId}-model-panel"
                    tabindex={target === 'local' ? 0 : -1}
                    onclick={() => (target = 'local')}
                    onkeydown={navigateTabs}
                    disabled={busy}
                >
                    {t($locale, 'models.local')}
                </button>
                <button
                    type="button"
                    role="tab"
                    aria-selected={target === 'cloud'}
                    aria-controls="{componentId}-model-panel"
                    tabindex={target === 'cloud' ? 0 : -1}
                    onclick={() => (target = 'cloud')}
                    onkeydown={navigateTabs}
                    disabled={busy}
                >
                    {t($locale, 'models.api')}
                </button>
            </div>
            <div id="{componentId}-model-panel" class="model-select" role="tabpanel">
                {#if target === 'local'}
                    {#if installedLocalModels.length}
                        <select aria-label={t($locale, 'models.local')} value={activeLocal?.id ?? ''} onchange={e => (selectedLocalId = e.currentTarget.value)} disabled={busy}>
                            {#each installedLocalModels as model (model.id)}
                                <option value={model.id}>{model.displayName || model.name || model.id}</option>
                            {/each}
                        </select>
                    {:else}
                        <div class="model-name muted">{t($locale, 'modelTest.notConfigured')}</div>
                    {/if}
                {:else if availableRemoteProfiles.length}
                    <select aria-label={t($locale, 'models.api')} value={activeRemote?.id ?? ''} onchange={e => (selectedRemoteId = e.currentTarget.value)} disabled={busy}>
                        {#each availableRemoteProfiles as profile (profile.id)}
                            <option value={profile.id}>{profile.displayName || profile.modelName}</option>
                        {/each}
                    </select>
                {:else}
                    <div class="model-name muted">{t($locale, 'modelTest.notConfigured')}</div>
                {/if}
            </div>
            <label for="{componentId}-prompt">{t($locale, 'modelTest.prompt')}</label>
            <textarea
                id="{componentId}-prompt"
                bind:value={prompt}
                disabled={busy}
                rows="8"
                onkeydown={handlePromptKeydown}
            ></textarea>
            <p class="muted shortcut-hint">{t($locale, 'chatInspector.shortcutSendHint')}</p>
            <div class="metrics">
                <h3>{t($locale, 'modelTest.metrics')}</h3>
                <p>{t($locale, 'modelTest.firstToken')} <strong>{firstTokenMs === null ? '—' : `${(firstTokenMs / 1000).toFixed(2)} s`}</strong></p>
                <p>{t($locale, 'modelTest.totalTime')} <strong>{totalMs === null ? '—' : `${(totalMs / 1000).toFixed(2)} s`}</strong></p>
                <p>{t($locale, 'modelTest.characters')} <strong>{[...output].length}</strong></p>
                <p>
                    {t($locale, 'chatOverview.context')}
                    <strong>
                        {#if usedTokens !== null}
                            {usedTokens.toLocaleString($locale)}{contextWindowTokens ? ` / ${contextWindowTokens.toLocaleString($locale)}` : ''}
                        {:else}
                            —
                        {/if}
                    </strong>
                </p>
                {#if lastUsage}
                    <p class="sub-metric">
                        <span>{t($locale, 'chatInspector.inputTokens')} / {t($locale, 'chatInspector.outputTokens')}</span>
                        <strong>{lastUsage.input_tokens.toLocaleString($locale)} / {lastUsage.output_tokens.toLocaleString($locale)}</strong>
                    </p>
                    {#if lastUsage.cache_read_tokens > 0}
                        <p class="sub-metric">
                            <span>{t($locale, 'chatInspector.cacheHitRate')}</span>
                            <strong>{lastUsage.cache_hit_rate.toFixed(1)}%</strong>
                        </p>
                    {/if}
                {/if}
                {#if contextFraction !== null}
                    <progress value={contextFraction} max="1" aria-label={t($locale, 'chatOverview.context')}></progress>
                {/if}
            </div>
        </section>
        <section id="{componentId}-output" aria-labelledby="{componentId}-output-title" class="card output">
            <header class="output-header">
                <h2 id="{componentId}-output-title">{t($locale, 'modelTest.output')}</h2>
                <button type="button" onclick={clear} disabled={busy || (!output && !error)}>{t($locale, 'modelTest.clear')}</button>
            </header>
            {#if busy}<p role="status" class="muted">{t($locale, 'modelTest.running')}</p>{/if}
            {#if output}
                <div class="rendered-output"><Markdown source={output} /></div>
            {:else}
                <p class="muted">{t($locale, 'modelTest.empty')}</p>
            {/if}
        </section>
    </div>
</div>

<style>
    .model-test-page { min-height: 100%; container-type: inline-size; container-name: model-test; }
    .page-header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 20px; }
    .actions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
    .model-test-page > .notice { max-width: 1120px; margin: 0 auto 16px; padding: 12px 16px; }
    .compact-panes { display: none; max-width: 1120px; margin: 0 auto 12px; padding-inline: 16px; }
    .model-test-grid { display: grid; grid-template-columns: minmax(280px, 360px) minmax(0, 1fr); gap: 16px; max-width: 1120px; margin-inline: auto; padding: 0 32px 32px; }
    .card { min-width: 0; padding: 20px; }
    .setup { display: flex; flex-direction: column; gap: 12px; }
    .model-select select { width: 100%; }
    .shortcut-hint { margin: 0; font-size: 12px; }
    .metrics p { display: flex; justify-content: space-between; gap: 8px; margin: 6px 0; font-size: 13px; }
    .metrics .sub-metric { font-size: 12px; color: var(--muted); }
    .metrics progress { width: 100%; height: 6px; }
    .output-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px; }
    @container model-test (max-width: 820px) {
        .compact-panes { display: flex; gap: 6px; }
        .model-test-grid { grid-template-columns: minmax(0, 1fr); padding: 0 16px 24px; }
        .model-test-grid[data-active-pane='setup'] .output,
        .model-test-grid[data-active-pane='output'] .setup { display: none; }
    }
    @media (max-width: 760px) {
        .compact-panes { display: flex; gap: 6px; }
        .model-test-grid { grid-template-columns: minmax(0, 1fr); padding: 0 16px 24px; }
    }
</style>
