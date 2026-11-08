<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import PrototypeIcon from './PrototypeIcon.svelte';
    type WorkspaceListCategory = 'conversations' | 'activity' | 'resources';
    let {category, onchange, visible = true}: {category: WorkspaceListCategory; onchange: (category: WorkspaceListCategory) => void; visible?: boolean} = $props();
    const categories = ['conversations', 'activity', 'resources'] as const;
    function move(event: KeyboardEvent, index: number) {
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? categories.length - 1
            : event.key === 'ArrowRight' ? (index + 1) % categories.length
            : event.key === 'ArrowLeft' ? (index + categories.length - 1) % categories.length : null;
        if (next === null) return;
        event.preventDefault();
        (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>('button')[next]?.focus();
    }
</script>
<nav class="list-categories" aria-label={t($locale, 'chatInspector.listCategories')}>
    {#each categories as item, index}
        <button type="button" title={t($locale, `chatInspector.list.${item}`)} aria-label={t($locale, `chatInspector.list.${item}`)}
            aria-pressed={visible && category === item} onclick={() => onchange(item)} onkeydown={event => move(event, index)}>
            <PrototypeIcon name={item === 'conversations' ? 'chat' : item === 'activity' ? 'clock' : 'folder'} size={18} />
        </button>
    {/each}
</nav>
<style>
    .list-categories { display: flex; flex: none; flex-direction: row; gap: 4px; }
    button { display: grid; place-items: center; width: 36px; height: 36px; padding: 0; background: transparent; border-color: transparent; border-radius: 8px; color: var(--muted); }
    button[aria-pressed='true'] { color:var(--content); background:var(--accent-subtle); border-color:transparent; position:relative; }
    @media (pointer: coarse) { button { width: 44px; height: 44px; } }
</style>
