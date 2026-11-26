<script lang="ts">
    import ExpertRoomManager from '../components/ExpertRoomManager.svelte';
    import { locale, t } from '../lib/i18n';
    import { workspacePageLocation } from '../lib/workspacePageLocation';

    const pageLocation = workspacePageLocation();
</script>

<svelte:head>
    {#if pageLocation.active()}
        <title>{t($locale, 'rooms.manageExperts')} — Nola</title>
    {/if}
</svelte:head>

<ExpertRoomManager standalone={true} />
