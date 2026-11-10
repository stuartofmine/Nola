<script lang="ts">
    import { tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import { conversationTabKey, contentTabDomId, type ConversationContentTabs } from '../lib/conversationContentTabs';
    let { state, title, ownerKey, onselect, onclose }: {state: ConversationContentTabs; title: string; ownerKey: string;
        onselect: (key: string) => void; onclose: (key: string) => void | Promise<void>} = $props();
    let strip: HTMLDivElement;
    const entries = $derived([{key: conversationTabKey, title}, ...state.items.map(item => ({key: item.key, title: item.kind === 'pdf' ? item.resource.displayName : t($locale, item.titleKey)}))]);
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
        <div class="tab" class:selected={state.selected === entry.key}>
            <button type="button" role="tab" id={contentTabDomId('tab', ownerKey, entry.key)} aria-controls={contentTabDomId('panel', ownerKey, entry.key)} aria-selected={state.selected === entry.key} tabindex={state.selected === entry.key ? 0 : -1}
                title={entry.title} onclick={() => onselect(entry.key)} onkeydown={event => void move(event, entry.key)}>{entry.title}</button>
            {#if entry.key !== conversationTabKey}<button type="button" class="close" tabindex="-1" title={`${t($locale, 'chatInspector.closeTab')}: ${entry.title}`} aria-label={`${t($locale, 'chatInspector.closeTab')}: ${entry.title}`}
                onclick={() => void close(entry.key)}>×</button>{/if}
        </div>
    {/each}
</div>
<style>
    .content-tabs { display: flex; align-self: stretch; align-items: end; flex: 1; min-width: 0; overflow-x: auto; scrollbar-width: thin; gap: 2px; }
    .tab { display: flex; align-items: center; flex: 0 0 auto; min-width: 110px; max-width: 230px; height: 43px; border: 1px solid transparent; border-bottom: 0; border-radius: 7px 7px 0 0; transition: background-color var(--motion-selection) var(--motion-disclosure), border-color var(--motion-selection) var(--motion-disclosure); }
    .tab.selected { background: var(--accent-subtle); border-color: transparent; }
    .tab > button { min-width: 0; max-width: 210px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border: 0; border-radius: 6px; background: transparent; padding: 8px 12px; font-size: 12px; }
    .tab > .close { flex: none; min-width: 28px; min-height: 32px; padding: 4px; font-size: 18px; }
    @media (max-width: 700px) { .content-tabs { align-items: center; } .tab { height: 44px; max-width: 200px; } .tab > .close { min-width: 44px; min-height: 44px; } }
</style>
