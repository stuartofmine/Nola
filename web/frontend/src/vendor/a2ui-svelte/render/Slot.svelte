<script lang="ts">
    import type { SlotContent } from '../catalog/types';
    import Node from './Node.svelte';
    import { inheritedPresentationOnly, providePresentationOnly } from './context';
    let { content, tab = 0, presentationOnly = false }: { content?: SlotContent; tab?: number; presentationOnly?: boolean } = $props();
    const inherited = inheritedPresentationOnly();
    providePresentationOnly(() => presentationOnly || inherited());
    const nodes = $derived(content?.kind === 'nodes' ? content.nodes : content?.tabs[tab]?.nodes ?? []);
</script>
{#each nodes as node (node.key)}<Node id={node.id} />{/each}
