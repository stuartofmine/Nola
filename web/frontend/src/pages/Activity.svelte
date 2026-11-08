<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { activityOwner } from '../lib/activity';
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    import { navigateWorkspace } from '../lib/workspaceNavigation';
    import WorkspaceActivityList from '../components/WorkspaceActivityList.svelte';
    const page = workspacePageLocation();
    const requestedOwner = activityOwner(page.read());
</script>
<svelte:head>{#if page.active()}<title>{t($locale, 'chatInspector.list.activity')} — Nola</title>{/if}</svelte:head>
<header class="page-header"><h1>{t($locale, 'chatInspector.list.activity')}</h1></header>
<div class="content"><WorkspaceActivityList full {requestedOwner} onopen={navigateWorkspace} /></div>
