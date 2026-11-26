<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import PrototypeIcon from './PrototypeIcon.svelte';
    type WorkspaceListCategory = 'conversations' | 'resources';
    let {category, onchange, visible = true, collapsed = false}: {category: WorkspaceListCategory; onchange: (category: WorkspaceListCategory) => void; visible?: boolean; collapsed?: boolean} = $props();
    const categories = ['conversations', 'resources'] as const;
    function move(event: KeyboardEvent, index: number) {
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? categories.length - 1
            : event.key === (collapsed ? 'ArrowDown' : 'ArrowRight') ? (index + 1) % categories.length
            : event.key === (collapsed ? 'ArrowUp' : 'ArrowLeft') ? (index + categories.length - 1) % categories.length : null;
        if (next === null) return;
        event.preventDefault();
        (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>('button')[next]?.focus();
    }
</script>
<nav class="list-categories" class:collapsed aria-label={t($locale, 'chatInspector.listCategories')}>
    {#each categories as item, index}
        <button type="button" title={t($locale, `chatInspector.list.${item}`)} aria-label={t($locale, `chatInspector.list.${item}`)}
            aria-pressed={visible && category === item} onclick={() => onchange(item)} onkeydown={event => move(event, index)}>
            <PrototypeIcon name={item === 'conversations' ? 'chat' : 'folder'} size={18} />
        </button>
    {/each}
</nav>
<style>
    .list-categories { display: flex; flex: none; flex-direction: row; gap: 4px; padding: 2px; max-width: 100%; background: var(--glass-track-bg); border: 1px solid var(--glass-track-border); border-radius: 10px; }
    .list-categories.collapsed { flex-direction: column; align-items: center; }
    button { display: grid; place-items: center; width: 32px; height: 32px; min-width: 32px; min-height: 32px; padding: 0; background: transparent; border: 1px solid transparent; border-radius: 7px; color: var(--muted); }
    button:hover:not(:disabled) { background: var(--composer-pill-hover-bg); border-color: transparent; color: var(--content); }
    button[aria-pressed='true'] { color: var(--content); background: var(--glass-pill-bg); border-color: var(--glass-pill-border); box-shadow: var(--glass-pill-shadow); position: relative; }
    @media (pointer: coarse) { button { width: 44px; height: 44px; } }
</style>
