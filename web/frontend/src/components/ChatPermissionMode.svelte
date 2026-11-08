<script lang="ts">
    import { locale, t } from '../lib/i18n';
    import type { PermissionChoice, SessionPermissionModeState } from '../lib/chatSessionSettings';

    let { state, onselect, onsave, onreload }: {
        state: SessionPermissionModeState | null;
        onselect: (choice: PermissionChoice) => void;
        onsave: () => void;
        onreload: () => void;
    } = $props();
    function selected(event: Event) {
        const select = event.currentTarget as HTMLSelectElement;
        const value = select.value;
        // The owning settings component is authoritative; do not leave a
        // browser-only selection visible if a concurrent lock rejects it.
        select.value = state?.choice ?? 'cautious';
        if (['cautious', 'allow_all'].includes(value)) onselect(value as PermissionChoice);
    }
</script>

<div class="permission-mode" aria-label={t($locale, 'chatSession.permission')}>
    <label for="chat-permission-mode"><span>{t($locale, 'chatSession.permission')}</span>
    <select id="chat-permission-mode" value={state?.choice ?? 'cautious'} onchange={selected} disabled={!state?.ready || state.busy || state.conflict}>
        <option value="cautious">{t($locale, 'chatSession.cautious')}</option>
        <option value="allow_all">{t($locale, 'chatSession.allowAll')}</option>
    </select></label>
    <span class="scope">{t($locale, 'chatPermissionMode.scope')}</span>
    {#if state?.applicationStatus && state.applicationStatus !== 'saved'}<span class="notice" role="status" aria-live="polite">{t($locale, `chatPermissionMode.${state.applicationStatus}`)}</span>{/if}
    {#if state?.canSave}<button type="button" class="save" onclick={onsave}>{t($locale, state.saving ? 'chatSession.saving' : 'chatSession.save')}</button>{/if}
    {#if state?.conflict || state?.errorKey}<button type="button" onclick={onreload} disabled={state.busy}>{t($locale, 'chatSession.reload')}</button>{/if}
    {#if state?.errorKey}<span class="error" role="alert">{t($locale, state.errorKey)}</span>{/if}
</div>
<style>
    .permission-mode { display: flex; align-items: center; flex-wrap: wrap; gap: 7px; min-width: 0; padding: 0; color: var(--muted); font-size: 12px; }
    label { display: flex; align-items: center; gap: 8px; min-height: var(--control-hit-size); font-weight: 600; white-space: nowrap; }
    select { width: auto; max-width: 150px; min-width: 0; padding: 3px 7px; font-size: 12px; }
    button { padding: 3px 8px; font-size: 12px; }
    .scope { white-space: normal; }
    select, button { min-height: max(28px, var(--control-hit-size)); }
    select:focus-visible, button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .error { flex-basis: 100%; color: var(--danger); }
    .notice { padding: 0; border: 0; color: var(--muted); }
    @media (max-width: 760px) {
        .permission-mode { padding: 0; }
        select, button { min-height: 44px; font-size: 13px; }
    }
</style>
