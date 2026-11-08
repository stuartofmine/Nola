<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { onMount } from 'svelte';
    import { api as unscopedApi } from '../lib/api';
    import RemoteProfiles from './RemoteProfiles.svelte';
    import ModelConnections from '../components/ModelConnections.svelte';
    import type { RemoteProfile } from '../lib/modelProfiles';
    import { loadModelCatalog } from '../lib/modelCatalog';
    import { locale, t } from '../lib/i18n';

    let { openProfileId = '' }: { openProfileId?: string } = $props();
    let profiles = $state<RemoteProfile[]>([]);
    let error = $state(false);
    let loading = $state(false);
    let loaded = $state(false);
    let loadController: AbortController | undefined;

    async function load() {
        loadController?.abort();
        const controller = new AbortController();
        loadController = controller;
        loading = true;
        try {
            const snapshot = await loadModelCatalog(<T,>(path: string) => api<T>(path, { signal: controller.signal }));
            if (controller.signal.aborted) return;
            profiles = snapshot.profiles;
            loaded = true;
            error = false;
        } catch {
            if (!controller.signal.aborted) error = true;
        } finally {
            if (loadController === controller) loading = false;
        }
    }

    onMount(() => { void load(); return () => loadController?.abort(); });

    function applySelectionSnapshot(snapshot: RemoteProfile[]) {
        loadController?.abort();
        loading = false;
        profiles = snapshot;
        loaded = true;
        error = false;
    }
</script>

{#if loaded || loading || error}
<section class="profile-settings" aria-label={t($locale, 'models.api')}>
    {#if error}<p class="notice error" role="alert">{t($locale, 'connections.catalogLoadFailed')} <button onclick={load} disabled={loading}>{t($locale, 'models.reload')}</button></p>{/if}
    {#if loading && !error}<p role="status">{t($locale, 'remote.loading')}</p>{/if}
    {#if loaded}<RemoteProfiles {profiles} {openProfileId} reload={load} onSelectionSaved={applySelectionSnapshot} />{/if}
</section>
{/if}
<ModelConnections />
<style>.profile-settings { margin-block: 24px 36px; }</style>
