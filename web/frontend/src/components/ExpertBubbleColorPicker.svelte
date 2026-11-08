<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import { expertBubbleColor, expertBubbleColors, expertBubblePalette, saveExpertBubbleColor, type ExpertBubbleColor } from '../lib/expertBubbleColors';
    let {hostId, agentId, name, avatar = false}: {hostId: string; agentId: string; name: string; avatar?: boolean} = $props();
    let draft = $state<ExpertBubbleColor>('blue'), notice = $state('');
    let dialog: HTMLDialogElement;
    function configure() { draft = expertBubbleColor(hostId, agentId); notice = ''; dialog.showModal(); }
    function save() {
        notice = saveExpertBubbleColor(hostId, agentId, draft) ? '' : 'expertColor.sessionOnly';
        dialog.close();
    }
</script>
<button type="button" class:avatar disabled={!hostId || !agentId} onclick={configure} aria-label={t($locale, 'expertColor.configure', {name})} title={t($locale, 'expertColor.configure', {name})}>
    {#if avatar}<span aria-hidden="true">{Array.from(name.trim())[0] ?? '?'}</span>{:else}<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a9 9 0 1 0 0 18h2a2 2 0 0 0 0-4h-1a1 1 0 0 1 0-2h3a5 5 0 0 0 0-10z"/><circle cx="7" cy="10" r="1"/><circle cx="11" cy="7" r="1"/><circle cx="16" cy="9" r="1"/></svg>{/if}
</button>
{#if notice}<p class="status" role="status">{t($locale, notice)}</p>{/if}
<dialog bind:this={dialog} aria-label={t($locale, 'expertColor.configure', {name})}>
    <form onsubmit={event => {event.preventDefault(); save();}}>
        <h2>{t($locale, 'expertColor.configure', {name})}</h2>
        <fieldset><legend>{t($locale, 'expertColor.bubble')}</legend>
            {#each expertBubbleColors as value}<label><input type="radio" name="bubble-color" value={value} bind:group={draft} /><span class="swatch" style:background={expertBubblePalette[value][0]}></span>{t($locale, `expertColor.${value}`)}</label>{/each}
        </fieldset>
        <div class="preview-row"><div class="preview-identity"><span aria-hidden="true">{Array.from(name)[0]}</span><strong>{name}</strong></div><div class="preview" style:--expert-light={expertBubblePalette[draft][0]} style:--expert-dark={expertBubblePalette[draft][1]}><p>{t($locale, 'expertColor.preview')}</p></div></div>
        <div class="actions"><button type="button" onclick={() => dialog.close()}>{t($locale, 'shell.cancel')}</button><button class="primary" type="submit">{t($locale, 'expertColor.save')}</button></div>
    </form>
</dialog>
<style>
    button:not(.primary) { padding: 6px; min-width: 36px; min-height: 36px; border: 1px solid transparent; background: transparent; }
    button.avatar { display: grid; place-items: center; padding: 0; }
    .avatar span { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 11px; background: var(--panel); color: var(--content); }
    svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.5; }
    fieldset { border: 0; padding: 0; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
    label { min-height: 44px; }
    .swatch { display: inline-block; width: 18px; height: 18px; border: 1px solid var(--outline); border-radius: 11px; }
    .preview-row { display:flex; align-items:flex-start; gap:12px; }
    .preview-identity { display:grid; justify-items:center; width:48px; flex:none; gap:6px; font-size:12px; text-align:center; overflow-wrap:anywhere; }
    .preview-identity span { width:32px; height:32px; display:grid; place-items:center; border-radius:11px; background:var(--panel); }
    .preview { padding: 14px 18px; border-radius: 5px 16px 16px; background: var(--expert-light); color: var(--content); }
    .status { color: var(--muted); font-size: 12px; }
    @media (prefers-color-scheme: dark) { :global(:root:not([data-appearance="light"])) .preview { background: var(--expert-dark); } }
    :global(:root[data-appearance="dark"]) .preview { background:var(--expert-dark); }
    @media (pointer: coarse), (max-width: 700px) { button { min-width: 44px; min-height: 44px; } .avatar span { width: 30px; height: 30px; } }
</style>
