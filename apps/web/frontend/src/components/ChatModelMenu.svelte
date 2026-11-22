<script lang="ts">
    import { tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import type { SessionModelChoiceState } from '../lib/chatSessionSettings';
    let { selection, onselect, onsettings, onreload, onopen }: {
        selection: SessionModelChoiceState | null;
        onselect: (choice: string) => unknown;
        onsettings: () => void;
        onreload: () => unknown;
        onopen: () => unknown;
    } = $props();
    let open = $state(false);
    let root: HTMLDetailsElement;
    let choiceShift = $state(0);
    function positionChoices() {
        const choices = root?.querySelector<HTMLElement>('.choices');
        if (!open || !choices || getComputedStyle(choices).position !== 'absolute') { choiceShift = 0; return; }
        const bounds = choices.getBoundingClientRect();
        const naturalLeft = bounds.left - choiceShift;
        const edge = window.visualViewport?.offsetLeft ?? 0;
        const width = window.visualViewport?.width ?? window.innerWidth;
        choiceShift = Math.min(Math.max(naturalLeft, edge + 8), Math.max(edge + 8, edge + width - bounds.width - 8)) - naturalLeft;
    }
    $effect(() => { if (open) void tick().then(positionChoices); else choiceShift = 0; });
    const selected = $derived(selection?.options.find(option => option.value === selection.choice));
    const label = $derived(selected?.name ?? t($locale, selection?.choice && selection.choice !== 'auto' ? 'chatSession.unavailableModel' : (selection?.options.length ? 'chatOverview.automatic' : 'rooms.model')));
    async function navigate(event: KeyboardEvent) {
        if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
        if (open && event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            close();
            return;
        }
        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        const fromTrigger = event.target === root?.querySelector('summary');
        open = true;
        await tick();
        if (!open || !root?.isConnected) return;
        const choices = [...root.querySelectorAll<HTMLElement>('.choices button:not(:disabled), .choices a')];
        if (!choices.length) return;
        const index = choices.indexOf(document.activeElement as HTMLElement);
        const selectedIndex = choices.findIndex(choice => choice.getAttribute('aria-pressed') === 'true');
        const entry = selectedIndex >= 0 ? selectedIndex : event.key === 'ArrowUp' ? choices.length - 1 : 0;
        const target = event.key === 'Home' ? 0 : event.key === 'End' ? choices.length - 1
            : fromTrigger || index < 0 ? entry
            : (index + (event.key === 'ArrowDown' ? 1 : -1) + choices.length) % choices.length;
        choices[target].focus({ preventScroll: true });
        const list = choices[target].parentElement?.closest<HTMLElement>('.choices');
        if (list) {
            const item = choices[target].getBoundingClientRect(), bounds = list.getBoundingClientRect();
            if (item.top < bounds.top) list.scrollTop -= bounds.top - item.top;
            else if (item.bottom > bounds.bottom) list.scrollTop += item.bottom - bounds.bottom;
        }
    }
    function close() { open = false; root?.querySelector('summary')?.focus({ preventScroll: true }); }
</script>
<svelte:window onclick={event => { if (open && !event.composedPath().includes(root)) open = false; }} onfocusin={event => { if (root && event.target instanceof Node && !root.contains(event.target)) { root.open = false; open = false; } }} onkeydown={event => { if (event.composedPath().includes(root)) void navigate(event); }} onresize={positionChoices} />
<details class="model-menu" bind:this={root} bind:open ontoggle={event => { if (event.currentTarget.open) onopen(); }}>
    <summary aria-label={`${t($locale, 'chatSession.model')}: ${label}`} title={label}><span>{label}</span><svg aria-hidden="true" class="chevron" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></svg>{#if selection?.errorKey}<span class="error-mark" role="img" aria-label={t($locale, selection.errorKey)} title={t($locale, selection.errorKey)}>!</span>{/if}</summary>
    <div class="choices" style:translate={choiceShift ? `${choiceShift}px 0` : undefined} aria-label={t($locale, 'chatSession.model')}>
        {#if selection?.options.length}
            <button type="button" disabled={selection.disabled} aria-pressed={selection?.choice === 'auto'} onclick={() => { close(); onselect('auto'); }}>{t($locale, 'chatSession.autoModel')}</button>
        {/if}
        {#each [false, true] as local}
            {#if selection?.options.some(option => option.local === local)}
                <p class="group">{t($locale, local ? 'models.local' : 'models.api')}</p>
                {#each selection.options.filter(option => option.local === local) as option (option.value)}
                    <button type="button" disabled={selection.disabled} aria-pressed={selection.choice === option.value} onclick={() => { close(); onselect(option.value); }}><span>{selection.choice === option.value ? '✓ ' : ''}{option.name}</span>{#if option.detail}<small>{option.detail}</small>{/if}</button>
                {/each}
            {/if}
        {/each}
        {#if selection?.errorKey}<p role="status">{t($locale, selection.errorKey)}</p><button type="button" onclick={event => { event.stopPropagation(); onreload(); }}>{t($locale, 'chatSession.reload')}</button>{/if}
        <div class="management" class:separated={!!selection?.options.length || !!selection?.errorKey}><button type="button" onclick={() => { close(); onsettings(); }}>{t($locale, 'chatSession.title')}</button><a href="/models">{t($locale, selection?.options.length ? 'models.title' : 'chatSession.configureModel')}</a></div>
    </div>
</details>
<style>
    .model-menu { position: relative; min-width: 0; max-width: min(30vw, 280px); flex-shrink: 1; }
    summary { display: flex; align-items: center; gap: 6px; cursor: pointer; min-height: max(36px, var(--control-hit-size)); font-size: 14px; list-style: none; }
    summary::-webkit-details-marker { display: none; }
    summary span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .chevron { width: 13px; height: 13px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; transition: transform var(--motion-base) ease-out; }
    details[open] .chevron { transform: rotate(180deg); }
    .error-mark { color: var(--danger); font-weight: 700; }
    details[open] .choices { animation: reveal var(--motion-base) ease-out; }
    @keyframes reveal { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }
    @media (prefers-reduced-motion: reduce) { details[open] .choices { animation: none; } .chevron { transition: none; } }
    .choices { position: absolute; top: 100%; right: 0; z-index: 50; width: min(340px, calc(100vw - 48px)); max-height: 65dvh; overflow-y: auto; padding: 8px; border: 1px solid var(--dialog-border); border-radius: 12px; background: var(--surface-elevated, var(--surface)); box-shadow: inset 0 1px 0 var(--surface-highlight), 0 8px 32px rgb(0 0 0 / .18); }
    button, a { display: block; width: 100%; text-align: left; padding: 8px 10px; border: 1px solid transparent; background: transparent; color: inherit; border-radius: 6px; font-size: 14px; box-sizing: border-box; text-decoration: none; overflow-wrap: anywhere; }
    button:hover:not(:disabled), a:hover { background: var(--composer-pill-hover-bg); border-color: transparent; }
    button[aria-pressed="true"] { background: var(--glass-pill-bg); border-color: var(--glass-pill-border); color: var(--content); }
    small { display: block; color: var(--muted); font-size: 12px; }
    .group { color: var(--muted); margin: 12px 10px 4px; font-size: 12px; }
    .management.separated { margin-top: 8px; padding-top: 8px; border-top: 1px solid var(--border); }
    @media(max-width:760px) { .model-menu { max-width: 30vw; } .choices { position: fixed; left: 12px; top: 96px; max-height: min(65dvh, calc(100dvh - 108px)); width: calc(100vw - 24px); } }
</style>
