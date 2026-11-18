<script lang="ts">
    import { tick, untrack } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { conversationTabKey, contentTabDomId, type ConversationContentTabs } from '../lib/conversationContentTabs';
    let { state: tabsState, title, ownerKey, conversationId = null, canRename = false, onselect, onclose, onrename }: {
        state: ConversationContentTabs; title: string; ownerKey: string;
        conversationId?: string | null; canRename?: boolean;
        onselect: (key: string) => void; onclose: (key: string) => void | Promise<void>;
        onrename?: (conversationId: string, nextTitle: string) => Promise<string | undefined | void> | void;
    } = $props();
    let strip: HTMLDivElement;
    let renameTarget = $state<{ id: string; baseline: string } | null>(null);
    let draftTitle = $state('');
    let renaming = $state(false);
    let renameInput = $state<HTMLInputElement>();
    const entries = $derived([{key: conversationTabKey, title}, ...tabsState.items.map(item => ({key: item.key, title: item.kind === 'pdf' ? item.resource.displayName : t($locale, item.titleKey)}))]);
    $effect(() => {
        if (renameTarget && (!canRename || conversationId !== renameTarget.id)) {
            untrack(() => { renameTarget = null; });
        }
    });
    async function beginRename() {
        if (!canRename || !conversationId || renaming) return;
        renameTarget = { id: conversationId, baseline: title };
        draftTitle = title;
        await tick();
        renameInput?.focus();
        renameInput?.select();
    }
    async function commitRename() {
        const target = renameTarget;
        if (!target || renaming || !onrename) { renameTarget = null; return; }
        const trimmed = draftTitle.trim();
        if (!trimmed || trimmed === target.baseline) { renameTarget = null; return; }
        renaming = true;
        try {
            const failure = await onrename(target.id, trimmed);
            if (!failure) renameTarget = null;
        } finally {
            renaming = false;
        }
    }
    async function close(key: string) {
        await onclose(key); await tick();
        strip.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.focus();
    }
    async function move(event: KeyboardEvent, key: string) {
        const index = entries.findIndex(item => item.key === key);
        let next: string | undefined;
        if (event.key === 'ArrowRight') next = entries[(index + 1) % entries.length].key;
        else if (event.key === 'ArrowLeft') next = entries[(index + entries.length - 1) % entries.length].key;
        else if (event.key === 'Home') next = entries[0].key;
        else if (event.key === 'End') next = entries.at(-1)?.key;
        else if (event.key === 'Delete' && key !== conversationTabKey) {
            event.preventDefault(); await close(key);
            strip.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.focus(); return;
        }
        if (next) {
            event.preventDefault(); onselect(next); await tick();
            const button = strip.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]');
            button?.focus(); button?.scrollIntoView({block: 'nearest', inline: 'nearest'});
        }
    }
</script>
<div class="content-tabs" bind:this={strip} role="tablist" aria-label={t($locale, 'chatInspector.contentTabs')}>
    {#each entries as entry (entry.key)}
        {#if entry.key === conversationTabKey && renameTarget && conversationId === renameTarget.id && canRename}
            <form class="tab-rename" onsubmit={event => { event.preventDefault(); void commitRename(); }}>
                <input bind:this={renameInput} bind:value={draftTitle} disabled={renaming} aria-label={t($locale, 'chatConversations.titleField')}
                    onblur={() => { if (!renaming && renameTarget && draftTitle.trim() === renameTarget.baseline) renameTarget = null; }}
                    onkeydown={event => { if (event.key === 'Escape') { event.preventDefault(); renameTarget = null; } }} />
            </form>
        {:else}
            <div class="tab" class:conversation-tab={entry.key === conversationTabKey} class:selected={tabsState.selected === entry.key}>
                <button type="button" role="tab" id={contentTabDomId('tab', ownerKey, entry.key)} aria-controls={contentTabDomId('panel', ownerKey, entry.key)} aria-selected={tabsState.selected === entry.key} tabindex={tabsState.selected === entry.key ? 0 : -1}
                    title={entry.title} onclick={() => onselect(entry.key)} ondblclick={() => { if (entry.key === conversationTabKey && tabsState.selected === conversationTabKey && canRename) void beginRename(); }} onkeydown={event => void move(event, entry.key)}>{entry.title}</button>
                {#if entry.key === conversationTabKey && tabsState.selected === conversationTabKey && canRename}
                    <button type="button" class="edit-hint" tabindex="-1" title={t($locale, 'chatConversations.rename')} aria-label={t($locale, 'chatConversations.rename')} onclick={() => void beginRename()}>
                        <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M11.5 2.5a1.4 1.4 0 0 1 2 2L5 13l-3 .7.7-3z" /></svg>
                    </button>
                {/if}
                {#if entry.key !== conversationTabKey}<button type="button" class="close" tabindex="-1" title={`${t($locale, 'chatInspector.closeTab')}: ${entry.title}`} aria-label={`${t($locale, 'chatInspector.closeTab')}: ${entry.title}`}
                    onclick={() => void close(entry.key)}>×</button>{/if}
            </div>
        {/if}
    {/each}
</div>
<style>
    .content-tabs { display: flex; align-self: stretch; align-items: center; justify-content: safe center; flex: 1; min-width: 0; overflow-x: auto; scrollbar-width: thin; gap: 6px; padding-inline: 8px; }
    .tab { display: inline-flex; align-items: center; flex: 0 0 auto; min-width: 96px; max-width: 240px; height: 32px; border: 1px solid transparent; border-radius: 7px; padding-inline: 2px; transition: background-color var(--motion-selection) var(--motion-disclosure), border-color var(--motion-selection) var(--motion-disclosure); }
    .tab.selected { background: var(--surface); border-color: var(--border); box-shadow: 0 1px 2px rgb(0 0 0 / .04); }
    .tab > button[role="tab"] { min-width: 0; flex: 1; max-width: 210px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border: 0; border-radius: 6px; background: transparent; min-height: 28px; padding: 4px 10px; font-size: 12.5px; font-weight: 550; }
    .tab > .edit-hint { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 24px; min-height: 24px; padding: 0; border: 0; border-radius: 5px; background: transparent; color: var(--muted); opacity: 0; transition: opacity 120ms ease; }
    .tab > .edit-hint svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
    .tab:hover > .edit-hint, .tab:focus-within > .edit-hint { opacity: 1; }
    .tab > .close { flex: none; min-width: 26px; min-height: 28px; padding: 2px 4px; font-size: 16px; border: 0; background: transparent; }
    .tab-rename { display: flex; align-items: center; }
    .tab-rename input { min-height: 30px; height: 30px; width: min(260px, 42vw); padding: 2px 10px; border: 1px solid var(--accent); border-radius: 7px; background: var(--surface); font-size: 12.5px; }
    @media (pointer: coarse) { .tab > .edit-hint { opacity: .85; width: 30px; height: 30px; min-height: 30px; } }
    @media (max-width: 700px) { .content-tabs { justify-content: flex-start; } .tab { height: 40px; max-width: 200px; } .tab > .close { min-width: 44px; min-height: 44px; } }
</style>
