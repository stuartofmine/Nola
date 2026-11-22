<script lang="ts">
    import { protocolTitle } from '../lib/modelEndpoints';
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { onMount, tick } from 'svelte';
    import GatewayClients from './GatewayClients.svelte';
    import SourceAccounts from './SourceAccounts.svelte';
    import ModelCapabilityProbe from './ModelCapabilityProbe.svelte';
    import ConfirmationDialog from './ConfirmationDialog.svelte';
    import type { ConfirmationHandle } from '../lib/confirmation';
    import { api as unscopedApi } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { randomId } from '../lib/randomId';
    import { loadConnections, modelSelectionCommand, type ConnectionInventory, type ConnectionSnapshot, type ImportPlan, type ImportResult } from '../lib/modelConnections';
    import type { RemoteProfile } from '../lib/modelProfiles';
    let { openConnectionId = '', profiles, onChanged }: { openConnectionId?: string; profiles?: RemoteProfile[]; onChanged?: () => Promise<void> | void } = $props();
    let confirmation: ConfirmationHandle;
    function importCredentialLabel(status: string) {
        switch (status) {
            case 'available': return t($locale, 'connections.importKeyAvailable');
            case 'not_requested': return t($locale, 'connections.importKeyNotRequested');
            case 'unavailable': return t($locale, 'connections.importKeyUnavailable');
            default: return status;
        }
    }
    function importIssueLabel(issue: string) {
        switch (issue) {
            case 'api_credential_required': return t($locale, 'connections.importKeyRequired');
            case 'subscription_credential_reuse_not_supported': return t($locale, 'connections.importSubscriptionUnsupported');
            default: return issue;
        }
    }
    let inventory = $state<ConnectionInventory>({connections: [], accounts: [], selections: []});
    let query = $state('');
    let expanded = $state<Record<string, boolean>>({});
    $effect(() => { if (openConnectionId) expanded[openConnectionId] = true; });
    const visibleConnections = $derived(inventory.connections.filter(item => `${item.document.connection.display_name} ${item.document.connection.base_url} ${(item.document.catalog?.models ?? []).map(model => model.display_name).join(' ')}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())));
    function modelEnabled(connectionId: string, key: string) {
        return inventory.selections.some(item => item.connection_id === connectionId && item.catalog_key === key && item.enabled);
    }
    function selectModel(event: Event, snapshot: ConnectionSnapshot, key: string) {
        const checkbox = event.currentTarget as HTMLInputElement;
        const requested = checkbox.checked;
        // Only a confirmed inventory refresh can change the visible enabled state.
        checkbox.checked = modelEnabled(snapshot.document.connection.id, key);
        toggle(snapshot, key, requested);
    }
    let loading = $state(false), busy = $state(false);
    let failed = $state<'connections.failed' | 'connections.loadFailed' | null>(null);
    let source = $state('open_code'), path = $state(''), credentialPath = $state('');
    let includeCredentials = $state(false), allowEnvironment = $state(false);
    let plan = $state<ImportPlan | null>(null);
    let selected = $state<string[]>([]);
    let updates = $state<string[]>([]);
    let enableModels = $state(false);
    let results = $state<ImportResult[]>([]);
    let gateway = $state({revision: 0, enabled: false});
    let accessPanel: HTMLDetailsElement;
    function openGatewayAccess() {
        accessPanel.open = true;
        accessPanel.querySelector('button')?.focus();
    }
    let gatewayToken = $state('');
    let tokenCopied = $state(false);
    function attemptLabel(profileId: string) {
        const match = inventory.selections.find(item => `${item.connection_id}:${item.catalog_key}` === profileId);
        if (!match) return profileId;
        const conn = inventory.connections.find(item => item.document.connection.id === match.connection_id);
        const model = conn?.document.catalog?.models.find(item => `${item.protocol}:${item.model_id}` === match.catalog_key || item.model_id === match.catalog_key);
        return model?.display_name || conn?.document.connection.display_name || profileId;
    }
    async function copyGatewayToken() {
        if (!gatewayToken) return;
        try {
            await navigator.clipboard?.writeText(gatewayToken);
            tokenCopied = true;
            setTimeout(() => { if (alive) tokenCopied = false; }, 2000);
        } catch { tokenCopied = false; }
    }
    let attempts = $state<{record: {id: string; profile_id: string; outcome: string; usage: {input_tokens?: number | null; output_tokens?: number | null} | null}}[]>([]);
    let planSource = $state('');
    let expiredPlan = $state(false);
    let discovery = $state<{request_id: string; connection_id: string; expected_revision: number} | null>(null);
    let cancellingDiscovery = $state(false);
    const sourceIdentity = $derived(JSON.stringify([source, path, credentialPath, includeCredentials, allowEnvironment]));
    $effect(() => { if (plan && planSource !== sourceIdentity) { plan = null; selected = []; updates = []; } });
    let alive = true;
    let controller: AbortController | undefined;
    async function refresh() {
        controller?.abort(); controller = new AbortController(); const signal = controller.signal;
        loading = true;
        try { const value = await loadConnections(signal); if (alive && !signal.aborted) { inventory = value; failed = null; } }
        catch { if (alive && !signal.aborted) failed = 'connections.loadFailed'; }
        finally { if (alive && !signal.aborted) loading = false; }
    }
    async function operation(action: () => Promise<void>) {
        if (busy) return;
        const trigger = document.activeElement;
        busy = true; failed = null;
        try { await action(); } catch { if (alive) failed = 'connections.failed'; }
        finally {
            if (alive) {
                busy = false;
                await tick();
                if (trigger instanceof HTMLElement && trigger.isConnected && document.activeElement === document.body) {
                    trigger.focus({preventScroll: true});
                }
            }
        }
    }
    function toggle(snapshot: ConnectionSnapshot, key: string, enabled: boolean) {
        void operation(async () => {
            await api('/api/model-connections', {method: 'POST', body: JSON.stringify(modelSelectionCommand(snapshot, inventory, key, enabled))});
            if (alive) { await refresh(); await onChanged?.(); }
        });
    }
    function discover(snapshot: ConnectionSnapshot) {
        const request = {request_id: randomId(), connection_id: snapshot.document.connection.id, expected_revision: snapshot.revision};
        void operation(async () => {
            discovery = request;
            try {
                await api('/api/model-connections/discover', {method: 'POST', body: JSON.stringify(request)});
                if (alive) { await refresh(); await onChanged?.(); }
            } catch (error) {
                if (discovery?.request_id === request.request_id) throw error;
            } finally {
                if (discovery?.request_id === request.request_id) discovery = null;
            }
        });
    }
    async function cancelDiscovery(teardown = false) {
        const request = discovery;
        if (!request || cancellingDiscovery) return;
        cancellingDiscovery = true;
        try {
            const result = teardown
                ? await unscopedApi<{cancelled: boolean}>('/api/model-connections/discover/cancel', {method: 'POST', body: JSON.stringify(request), signal: AbortSignal.timeout(15_000)})
                : await api<{cancelled: boolean}>('/api/model-connections/discover/cancel', {method: 'POST', body: JSON.stringify(request)});
            if (!teardown && alive) {
                if (result.cancelled && discovery?.request_id === request.request_id) { discovery = null; failed = null; }
                await refresh();
                await onChanged?.();
            }
        } catch { if (!teardown && alive) failed = 'connections.failed'; }
        finally { if (!teardown && alive) cancellingDiscovery = false; }
    }
    function preview() {
        const identity = sourceIdentity; expiredPlan = false;
        void operation(async () => {
            const value = await api<ImportPlan>('/api/model-import/plan', {method: 'POST', body: JSON.stringify({source,
                path: path.trim(), credential_path: includeCredentials && credentialPath.trim() ? credentialPath.trim() : null,
                include_credentials: includeCredentials, allow_environment: allowEnvironment})});
            if (alive && identity === sourceIdentity) { planSource = identity; plan = value; selected = []; updates = []; results = []; }
        });
    }
    function commit() {
        if (!plan) return;
        if (Date.now() >= plan.expires_at_ms) { plan = null; expiredPlan = true; return; }
        const current = plan;
        void operation(async () => {
            const value = await api<ImportResult[]>('/api/model-import/commit', {method: 'POST', body: JSON.stringify({plan_id: current.plan_id,
                selections: selected.map(candidate_id => ({candidate_id, display_name: null, update_existing: updates.includes(candidate_id), enable_models: enableModels}))})});
            if (alive) { results = value; plan = null; await refresh(); await onChanged?.(); }
        });
    }
    async function configureGateway(revoke: boolean) {
        if (busy) return;
        if (revoke) {
            const approved = await confirmation.ask({
                title: t($locale, 'connections.revokeToken'),
                description: t($locale, 'connections.gatewayHint'),
                confirmLabel: t($locale, 'connections.revokeToken'),
                destructive: true,
            });
            if (!approved || !alive) return;
        }
        void operation(async () => {
            const value = await api<{revision: number; token?: string}>('/api/model-gateway/access', {method: 'POST', body: JSON.stringify({expected_revision: gateway.revision, revoke})});
            if (alive) { gatewayToken = value.token ?? ''; tokenCopied = false; gateway = {revision: value.revision, enabled: !revoke}; }
        });
    }
    function readAttempts() { void operation(async () => { const value = await api<typeof attempts>('/api/model-gateway/attempts'); if (alive) attempts = value; }); }
    onMount(() => { void refresh(); void api<typeof gateway>('/api/model-gateway/access').then(value => { if (alive) gateway = value; }).catch(() => { if (alive) failed = 'connections.loadFailed'; }); return () => { void cancelDiscovery(true); alive = false; gatewayToken = ''; controller?.abort(); }; });
</script>

<ConfirmationDialog bind:this={confirmation} />
<section class="connections"><div class="row between connections-heading"><h2>{t($locale, 'connections.title')}</h2><button disabled={busy || loading} onclick={refresh}>{t($locale, 'rooms.refresh')}</button></div>
    {#if loading || busy}<p role="status">{t($locale, busy ? 'rooms.saving' : 'rooms.loading')}</p>{/if}
    {#if discovery}<button disabled={cancellingDiscovery} onclick={() => void cancelDiscovery()}>{t($locale, 'rooms.cancel')}</button>{/if}
    {#if failed}<p role="alert" class="error">{t($locale, failed)}</p>{/if}
    <label>{t($locale, 'connections.search')}<input type="search" bind:value={query} /></label>
    {#if !loading && !failed && !inventory.connections.length}<p class="muted">{t($locale, 'connections.empty')}</p>{/if}
    {#if !loading && !failed && inventory.connections.length && !visibleConnections.length}<p class="muted" role="status">{t($locale, 'connections.noMatches')}</p>{/if}
    {#each visibleConnections as snapshot (snapshot.document.connection.id)}
        {@const connection = snapshot.document.connection}
        <details class="connection-detail" open={expanded[connection.id] ?? false}
            ontoggle={event => { expanded[connection.id] = event.currentTarget.open; }}><summary class="connection-summary"><svg class="connection-chevron" aria-hidden="true" viewBox="0 0 16 16"><path d="m6 3 5 5-5 5" /></svg><span class="connection-identity"><strong>{connection.display_name}</strong><small title={connection.base_url}>{connection.base_url}</small></span><span class="connection-count">{snapshot.document.catalog?.models.length ?? 0} · {t($locale, connection.enabled ? 'connections.enabled' : 'connections.disabled')}</span></summary>
        <div class="connection-body">
            <button disabled={busy} onclick={() => discover(snapshot)}>{t($locale, 'connections.discover')}</button>
            {#each inventory.accounts.filter(item => item.account.id === connection.account_id) as item}<p class="muted">{item.account.display_name}</p>{/each}
            {#each snapshot.document.catalog?.models ?? [] as model (`${model.protocol}:${model.model_id}`)}
                {@const key = `${model.protocol}:${model.model_id}`}
                <label class="model"><input type="checkbox" role="switch" disabled={busy} checked={modelEnabled(connection.id, key)} onchange={event => selectModel(event, snapshot, key)} />
                    <span>{model.display_name} <small>{protocolTitle(model.protocol)}{#if model.context_window} · {model.context_window.toLocaleString($locale)}{/if}</small></span></label>
                <ModelCapabilityProbe {snapshot} {model} refresh={async () => { await refresh(); await onChanged?.(); }} accountRevision={inventory.accounts.find(item => item.account.id === snapshot.document.connection.account_id)?.revision ?? null} />
            {/each}
        </div></details>
    {/each}
    <section class="integrations" aria-label={t($locale, 'connections.manage')}><h3>{t($locale, 'connections.manage')}</h3>
    <details><summary>{t($locale, 'connections.import')}</summary>
        <form onsubmit={event => { event.preventDefault(); preview(); }}>
            <label>{t($locale, 'connections.source')}<select bind:value={source} disabled={busy}><option value="open_code">OpenCode</option><option value="codex">Codex</option><option value="kimi_code">Kimi Code CLI</option></select></label>
            <label>{t($locale, 'connections.path')}<input bind:value={path} required disabled={busy} /></label>
            <label class="model"><input type="checkbox" bind:checked={includeCredentials} disabled={busy} />{t($locale, 'connections.credentials')}</label>
            {#if includeCredentials}<label>{t($locale, 'connections.credentialPath')}<input bind:value={credentialPath} disabled={busy} /></label>{/if}
            <label class="model"><input type="checkbox" bind:checked={allowEnvironment} disabled={busy} />{t($locale, 'connections.environment')}</label>
            <button disabled={busy || !path.trim()}>{t($locale, 'connections.preview')}</button>
        </form>
        {#if expiredPlan}<p role="status">{t($locale, 'activity.importExpired')}</p>{/if}
        {#if plan}<div class="plan">
            {#each plan.candidates as candidate (candidate.candidate_id)}<article>
                <label class="model"><input type="checkbox" bind:group={selected} value={candidate.candidate_id} disabled={busy || !!candidate.issue} />
                    <span>{candidate.connection.display_name} · {importCredentialLabel(candidate.credential_status)}</span></label>
                <p class="muted">{candidate.connection.base_url}</p>
                {#if candidate.issue}<p class="error">{importIssueLabel(candidate.issue)}</p>{/if}
                {#if candidate.existing_revision !== null}<label class="model"><input type="checkbox" bind:group={updates} value={candidate.candidate_id} disabled={busy} />{t($locale, 'connections.update')}</label>{/if}
            </article>{/each}
            <label class="model"><input type="checkbox" bind:checked={enableModels} disabled={busy} />{t($locale, 'connections.enable')}</label>
            <button class="primary" disabled={busy || !selected.length} onclick={commit}>{t($locale, 'connections.confirm')}</button>
        </div>{/if}
        {#each results as result}<p role="status" class:error={result.status !== 'imported'}>{t($locale, result.status === 'imported' ? 'connections.imported' : 'connections.importFailed')} {result.error ?? ''}</p>{/each}
    </details>
    <details bind:this={accessPanel}><summary>{t($locale, 'connections.gateway')}</summary>
        <p>{t($locale, 'connections.gatewayHint')}</p><code>{typeof location === 'undefined' ? '' : location.origin}/gateway/v1</code>
        <p><button disabled={busy} onclick={() => configureGateway(false)}>{t($locale, 'connections.issueToken')}</button>
        {#if gateway.enabled}<button class="danger" disabled={busy} onclick={() => configureGateway(true)}>{t($locale, 'connections.revokeToken')}</button>{/if}</p>
        {#if gatewayToken}<div class="gateway-token"><label>{t($locale, 'connections.tokenOnce')}<input readonly type="text" class="token-value" value={gatewayToken} onclick={event => event.currentTarget.select()} /></label><button type="button" onclick={copyGatewayToken}>{t($locale, tokenCopied ? 'shell.login.copied' : 'shell.markdown.copy')}</button></div>{/if}
    </details>
    <GatewayClients {gateway} {profiles} openAccess={openGatewayAccess} />
    <SourceAccounts />
    <details><summary>{t($locale, 'connections.gatewayUsage')}</summary>
        <button disabled={busy} onclick={readAttempts}>{t($locale, 'rooms.refresh')}</button>
        {#each attempts as attempt (attempt.record.id)}<p>{attemptLabel(attempt.record.profile_id)} · {attempt.record.outcome} · {attempt.record.usage?.input_tokens ?? '—'} / {attempt.record.usage?.output_tokens ?? '—'}</p>{/each}
    </details>
    </section>
</section>

<style>
    .connections { display: grid; gap: 20px; min-width: 0; }
    .connections-heading h2 { font-size: 1.125rem; }
    .connections-heading button { background: transparent; border-color: transparent; }
    .connections > label { font-size: .8125rem; color: var(--muted); }
    .connections > label input { color: var(--content); min-height: 44px; }
    .connection-detail { border-bottom: 1px solid var(--border); }
    .connection-summary { display: flex; gap: 16px; align-items: center; padding: 16px 0; min-height: 64px; }
    .connection-summary::-webkit-details-marker { display: none; }
    .connection-chevron { width: 16px; height: 16px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.5; color: var(--muted); transition: transform var(--motion-base) ease; }
    .connection-detail[open] > summary .connection-chevron { transform: rotate(90deg); }
    .connection-identity { display: grid; min-width: 0; flex: 1; gap: 4px; }
    .connection-identity small { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--muted); }
    .connection-count { flex-shrink: 0; font-size: 12px; color: var(--muted); text-align: end; }
    .connection-identity strong { font-size: 1rem; overflow-wrap: anywhere; }
    .connection-body { padding: 12px 0 24px; }
    .integrations { border-top: 1px solid var(--border); padding-top: 8px; }

    form, .plan { display: grid; gap: 12px; padding: 12px 0; }
    .model { display: flex; align-items: center; gap: 10px; min-height: 44px; } .model input { width: auto; flex: none; }
    .connection-body > .model { border-top: 1px solid var(--border); padding-block: 12px; }
    .model > span { min-width: 0; overflow-wrap: anywhere; }
    .model small { display: block; font-size: .75rem; }
    .integrations > h3 { font-size: .9375rem; font-weight: 600; margin: 0 0 8px; }
    summary { cursor: pointer; min-height: 44px; padding: 10px 12px; }
    .gateway-token { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 10px; }
    .gateway-token label { flex: 1 1 240px; min-width: 0; }
    .token-value { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
    @media (max-width: 480px) { .connection-summary { align-items: start; gap: 12px; } .connection-count { flex: 0 1 90px; overflow-wrap: anywhere; } }
</style>
