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
            if (controller !== current) return;
            status = current.state;
            if (status === 'accepted') { instruction = ''; onAccepted?.(permissionIdentity(current.challenge)); }
        } catch (cause) {
            if (controller === current) {
                status = current.state;
                localError = cause instanceof Error && cause.message.startsWith('chatPermission.') ? cause.message : 'chatPermission.invalid';
            }
        }
    }
</script>

<section tabindex="-1" use:activityRequestFocus={{conversationId: parsed?.session_id ?? '', runId, active: active && !expired && status === 'ready'}} class="permission-panel" aria-label={t($locale, 'chatPermission.title')} aria-busy={status === 'submitting'}>
    <header class="permission-header">
        <h3>{t($locale, 'chatPermission.title')}</h3>
        {#if onRefresh}
            <button type="button" class="refresh-icon" disabled={status === 'submitting'} aria-label={t($locale, 'chatPermission.refresh')} title={t($locale, 'chatPermission.refresh')} onclick={() => onRefresh?.()}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 7v5h-5M20 12a8 8 0 1 0-2 5" /></svg>
            </button>
        {/if}
    </header>
    {#if parsed}
        <div class="permission-content" bind:this={content}>
        <p class="tool">{parsed.tool_name}</p>
        {#if reviewText.trim()}<pre>{reviewText}</pre>{:else}<p>{t($locale, 'chatPermission.noDetails')}</p>{/if}
        <p class="muted">{t($locale, 'chatPermission.scope')}</p>
        {#if !active || status !== 'ready' || expired}
            <p role="status" aria-live="polite">{t($locale, !active ? 'chatPermission.inactive' : status !== 'ready' ? `chatPermission.${status}` : 'chatPermission.expired')}</p>
        {/if}
        {#if localError}<p role="alert" class="error">{t($locale, localError)}</p>{/if}
        <details class="guidance" bind:open={guidanceOpen}>
        <summary>{t($locale, 'chatPermission.guidance')}</summary>
        <label><span class="sr-only">{t($locale, 'chatPermission.guidance')}</span><textarea bind:value={instruction} disabled={disabled} rows="2" maxlength="4096"></textarea></label>
        <p class="muted">{t($locale, 'chatPermission.guidanceHint')}</p>
        <button disabled={disabled || !instruction.trim()} onclick={() => submit('provide_instruction')}>{t($locale, 'chatPermission.saveGuidance')}</button>
        </details>
        </div>
        <div class="actions">
            {#if status === 'submitting'}<span class="muted" role="status">{t($locale, 'chatPermission.submitting')}</span>{/if}
            {#if status === 'unknown'}<button disabled={!active || expired} onclick={() => submit('deny', true)}>{t($locale, 'chatPermission.retrySame')}</button>{/if}
            <button class="danger" disabled={disabled} onclick={() => submit('deny')}>{t($locale, 'chatPermission.deny')}</button>
            <button disabled={disabled} onclick={() => submit('return_to_composer')}>{t($locale, 'chatPermission.return')}</button>
            {#if parsed.allow_conversation}<button disabled={disabled || !reviewText.trim()} onclick={() => submit('allow_conversation')}>{t($locale, 'chatPermission.allowConversation')}</button>{/if}
            <button class="primary" disabled={disabled || !reviewText.trim()} onclick={() => submit('allow_once')}>{t($locale, 'chatPermission.allowOnce')}</button>
        </div>
    {:else}<p role="alert" class="error">{t($locale, 'chatPermission.invalid')}</p>{/if}
</section>

<style>
    .permission-panel { flex-shrink: 0; align-self: center; width: calc(100% - 24px); max-width: 752px; max-height: min(420px, 50dvh); overflow: hidden; margin: 8px 12px; background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 16px; display: flex; flex-direction: column; gap: 12px; }
    .permission-header { flex-shrink: 0; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    .refresh-icon { flex: none; display: grid; place-items: center; width: 32px; height: 32px; min-width: 32px; min-height: 32px; padding: 0; border: 0; background: transparent; box-shadow: none; color: var(--muted); border-radius: 8px; }
    .refresh-icon:hover:not(:disabled) { background: var(--panel); color: var(--content); }
    .refresh-icon svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
    .permission-content { flex: 1; min-height: 0; overflow: auto; overscroll-behavior: contain; display: flex; flex-direction: column; gap: 12px; padding: 2px; }
    .permission-content > * { flex-shrink: 0; }
    h3, p { margin: 0; overflow-wrap: anywhere; }
    h3 { font-size: .9375rem; font-weight: 600; }
    .tool { font-family: ui-monospace, monospace; color: var(--muted); font-size: .8125rem; }
    pre { white-space: pre-wrap; overflow-wrap: anywhere; margin: 0; padding: 12px; background: var(--panel); border-radius: 8px; font-size: .8125rem; line-height: 1.5; }
    .actions { flex-shrink: 0; display: flex; flex-wrap: wrap; gap: 8px; padding-top: 12px; border-top: 1px solid var(--border); }
    .guidance > summary { width: 100%; min-height: var(--control-hit-size); cursor: pointer; padding: 8px 0; font-size: .8125rem; color: var(--muted); }
    .guidance[open] > :not(summary) { margin-top: 8px; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    @media (max-width: 760px) { .actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); } .actions button { overflow-wrap: anywhere; padding-inline: 8px; } }
    @media (pointer: coarse) { .refresh-icon { width: 44px; height: 44px; min-width: 44px; min-height: 44px; } }
    label { display: grid; gap: 6px; }
    textarea { width: 100%; box-sizing: border-box; }
</style>
