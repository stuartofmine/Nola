<script lang="ts">
    import { onMount, untrack } from 'svelte';
    import { activityRequestFocus } from '../lib/activityRequestFocus';
    import { locale, t } from '../lib/i18n';
    import { ChatPermissionSubmission, parseChatPermission, permissionExpired, permissionIdentity } from '../lib/chatPermission';
    import type { PermissionChoice, PermissionSubmissionState } from '../lib/chatPermission';

    // reviewText must be the caller's safe tool-detail projection, never raw secret-bearing args.
    let { challenge, runId = null, active = true, reviewText = '', onAccepted, onRefresh }: {
        challenge: unknown; runId?: string | null; active?: boolean; reviewText?: string;
        onAccepted?: (identity: string) => void; onRefresh?: () => void;
    } = $props();
    let parsed = $derived(parseChatPermission(challenge));
    let identity = $derived(parsed ? permissionIdentity(parsed) : '');
    let controller = $state<ChatPermissionSubmission | null>(null);
    let status = $state<PermissionSubmissionState>('ready');
    let instruction = $state('');
    let guidanceOpen = $state(false);
    let content = $state<HTMLDivElement>();
    let localError = $state('');
    let now = $state(Date.now());
    let expired = $derived(parsed ? permissionExpired(parsed, now) : true);
    let disabled = $derived(!active || expired || !parsed || status !== 'ready');
    const displayState = $derived(!active ? 'inactive' : status !== 'ready' ? status : expired ? 'expired' : 'ready');

    $effect(() => {
        const value = identity;
        const next = untrack(() => {
            controller?.dispose();
            const next = parsed && value ? new ChatPermissionSubmission(parsed) : null;
            controller = next;
            status = 'ready'; instruction = ''; guidanceOpen = false; localError = '';
            if (content) content.scrollTop = 0;
            return next;
        });
        return () => next?.dispose();
    });
    onMount(() => {
        const timer = setInterval(() => { now = Date.now(); }, 1000);
        return () => clearInterval(timer);
    });
    async function submit(choice: PermissionChoice, retry = false) {
        const current = controller;
        if (!current || !active || permissionExpired(current.challenge) || (!retry && disabled)) return;
        if (!retry && (choice === 'allow_once' || choice === 'allow_conversation') && !reviewText.trim()) return;
        localError = '';
        try {
            const pending = retry ? current.retry() : current.submit(choice, instruction);
            status = current.state;
            await pending;
            if (controller !== current || !active) return;
            status = current.state;
            if (status === 'accepted') { instruction = ''; onAccepted?.(permissionIdentity(current.challenge)); }
        } catch { if (controller === current) localError = 'chatPermission.instructionLimit'; }
    }
</script>

<section tabindex="-1" use:activityRequestFocus={{conversationId: parsed?.session_id ?? '', runId, active: active && !expired && status === 'ready'}} class="permission-panel" data-state={displayState} aria-label={t($locale, 'chatPermission.title')} aria-busy={status === 'submitting'}>
    <header>
        <span class="permission-symbol" aria-hidden="true">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 4 6v6c0 4 4 7 8 9 4-2 8-5 8-9V6l-8-3Z"/><path d="M12 8v5m0 3h.01"/></svg>
        </span>
        <div><h3>{t($locale, 'chatPermission.title')}</h3>{#if parsed}<p class="tool">{parsed.tool_name}</p>{/if}</div>
    </header>
    {#if parsed}
        <div class="permission-content" bind:this={content}>
        <section class="review" aria-label={t($locale, 'chatPage.details')}>
            {#if reviewText.trim()}<pre>{reviewText}</pre>{:else}<p class="unavailable">{t($locale, 'chatPermission.noDetails')}</p>{/if}
        </section>
        <p class="muted">{t($locale, 'chatPermission.scope')}</p>
        <p class="request-status" role="status" aria-live="polite">{t($locale, `chatPermission.${displayState}`)}</p>
        {#if localError}<p role="alert">{t($locale, localError)}</p>{/if}
        {#if parsed.permission_instructions.length}
        <section class="recorded-guidance" aria-label={t($locale, 'chatPermission.guidance')}>
            <h4>{t($locale, 'chatPermission.guidance')}</h4>
            {#each parsed.permission_instructions as record}<p>{record.instruction}</p>{/each}
            <p class="muted">{t($locale, 'chatPermission.guidanceHint')}</p>
        </section>
        {/if}
        <details class="guidance" bind:open={guidanceOpen}>
        <summary>{t($locale, 'chatPermission.guidance')}</summary>
        <label><span class="sr-only">{t($locale, 'chatPermission.guidance')}</span><textarea bind:value={instruction} disabled={disabled} rows="2" maxlength="4096"></textarea></label>
        <p class="muted">{t($locale, 'chatPermission.guidanceHint')}</p>
        <button disabled={disabled || !instruction.trim()} onclick={() => submit('provide_instruction')}>{t($locale, 'chatPermission.saveGuidance')}</button>
        </details>
        </div>
        <div class="actions">
            <button disabled={disabled} onclick={() => submit('deny')}>{t($locale, 'chatPermission.deny')}</button>
            <button class="secondary-action" disabled={disabled} onclick={() => submit('return_to_composer')}>{t($locale, 'chatPermission.return')}</button>
            {#if parsed.allow_conversation}<button disabled={disabled || !reviewText.trim()} onclick={() => submit('allow_conversation')}>{t($locale, 'chatPermission.allowConversation')}</button>{/if}
            <button class="primary" disabled={disabled || !reviewText.trim()} onclick={() => submit('allow_once')}>{t($locale, 'chatPermission.allowOnce')}</button>
        </div>

        <div class="recovery-actions">
        {#if status === 'unknown'}<button disabled={!active || expired} onclick={() => submit('deny', true)}>{t($locale, 'chatPermission.retrySame')}</button>{/if}
        {#if onRefresh}<button disabled={status === 'submitting'} onclick={() => onRefresh?.()}>{t($locale, 'chatPermission.refresh')}</button>{/if}
        </div>
    {:else}<p role="alert">{t($locale, 'chatPermission.invalid')}</p>{/if}
</section>

<style>
    .permission-panel { --permission-tone: var(--warning); flex-shrink: 0; align-self: center; width: calc(100% - 24px); max-width: 752px; max-height: max(220px, 50dvh); overflow: auto; box-sizing: border-box; margin: 8px 12px; background: var(--surface); border: 1px solid var(--border); border-radius: 16px; padding: 16px; box-shadow: 0 2px 12px color-mix(in srgb, var(--muted) 8%, transparent); display: flex; flex-direction: column; gap: 12px; }
    header { display: flex; gap: 10px; align-items: flex-start; flex-shrink: 0; }
    header > div { min-width: 0; }
    .permission-symbol { width: 32px; height: 32px; flex: 0 0 32px; display: grid; place-items: center; border-radius: 10px; color: var(--permission-tone); background: color-mix(in srgb, var(--permission-tone) 10%, transparent); }
    .permission-panel[data-state="accepted"] { --permission-tone: var(--success); }
    .permission-panel[data-state="rejected"], .permission-panel[data-state="expired"] { --permission-tone: var(--danger); }
    .permission-content { min-height: 48px; overflow: auto; overscroll-behavior: contain; display: flex; flex-direction: column; gap: 12px; padding: 2px; }
    .permission-content > * { flex-shrink: 0; }
    h3, h4, p { margin: 0; overflow-wrap: anywhere; }
    h3 { font-size: .9375rem; font-weight: 600; }
    h4 { font-size: .8125rem; font-weight: 600; }
    .recorded-guidance { border-left: 2px solid var(--border); padding-left: 12px; display: grid; gap: 8px; }
    .recorded-guidance p { white-space: pre-wrap; font-size: .8125rem; }
    .request-status { font-size: .8125rem; line-height: 1.5; }
    .muted { font-size: .75rem; line-height: 1.5; color: var(--muted); }
    .unavailable { color: var(--warning); font-size: .8125rem; }
    .review { min-width: 0; }
    .tool { margin-top: 4px; font-family: ui-monospace, monospace; color: var(--muted); font-size: .8125rem; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; margin: 0; padding: 12px; background: var(--panel); border-radius: 8px; font-size: .8125rem; line-height: 1.5; }
    .actions { flex-shrink: 0; display: flex; flex-wrap: wrap; gap: 8px; padding-top: 12px; justify-content: flex-end; border-top: 1px solid var(--border); }
    .actions button, .recovery-actions button { min-height: var(--control-hit-size); }
    .secondary-action { color: var(--muted); }
    .recovery-actions { display: flex; flex-wrap: wrap; gap: 8px; }
    .recovery-actions:empty { display: none; }
    .recovery-actions button { font-size: .75rem; color: var(--muted); }
    .guidance > summary { width: 100%; min-height: var(--control-hit-size); cursor: pointer; padding: 8px 0; font-size: .8125rem; color: var(--muted); }
    .guidance[open] > :not(summary) { margin-top: 8px; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    @media (max-width: 760px) { .actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); } .actions button { overflow-wrap: anywhere; padding-inline: 8px; } }
    label { display: grid; gap: 6px; }
    textarea { width: 100%; box-sizing: border-box; }
</style>
