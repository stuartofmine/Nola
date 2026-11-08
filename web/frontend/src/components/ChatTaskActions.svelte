<script lang="ts">
    import { onMount } from 'svelte';
    import ChatUnknownEffectRecovery from './ChatUnknownEffectRecovery.svelte';
    import { parseRecoveryInspection, type RecoveryOwner, type RecoveryInspection, type OwnedTaskRecoveryContext } from '../lib/unknownEffectRecovery';
    import { api } from '../lib/api';
    import { locale, t } from '../lib/i18n';
    import { taskControl, TaskActionSubmission } from '../lib/chatTaskActions';
    import type { TaskOwner, TaskActionState } from '../lib/chatTaskActions';
    import type { TimelinePart } from '../lib/chatTimeline';
    let { part, owner, clientId = '', onRecovered }: { part: TimelinePart; owner: TaskOwner; clientId?:string; onRecovered?:()=>void } = $props();
    const control = $derived(taskControl(part, owner));
    let controller = $state<TaskActionSubmission | null>(null);
    let status = $state<TaskActionState>('ready');
    let recoveryOwner = $state<RecoveryOwner | null>(null);
    let taskContext = $state<OwnedTaskRecoveryContext | undefined>(undefined);
    async function inspectUnknown() {
        if (!control || !clientId || status==='sending') return;
        const captured={...owner}; const taskId=control.taskId;
        const task={host_id:captured.hostId,conversation_id:captured.conversationId,parent_run_id:captured.runId,task_id:taskId};
        status='sending';
        try {
            const response=await api<{source_host_id:string;inspection:RecoveryInspection|null}>('/api/chat/agent/v2/task/recovery',{method:'POST',body:JSON.stringify({task,recovery:null})});
            if (response.source_host_id!==captured.hostId || !response.inspection) throw new Error('No exact task recovery');
            const inspected=response.inspection;
            const child={expected_host_id:captured.hostId,client_id:clientId,conversation_id:inspected.conversation_id,
                run_id:inspected.run_id,assistant_message_id:inspected.assistant_message_id};
            parseRecoveryInspection({source_host_id:captured.hostId,owner:child,result:{inspection:inspected}},child);
            if (owner.hostId!==captured.hostId || owner.conversationId!==captured.conversationId || owner.runId!==captured.runId || control?.taskId!==taskId) return;
            taskContext=task; recoveryOwner=child;
        } catch { status='unknown'; }
        finally { if (status==='sending') status=controller?.state ?? 'unknown'; }
    }
    let draft = $state('');
    let mounted = $state(false);
    let storageRevision = $state(0);
    let loadedStorageRevision = -1;
    onMount(() => {
        mounted = true;
        const changed = (event: StorageEvent) => {
            if (event.storageArea === localStorage && (event.key === null || event.key.startsWith('nola.web.task-intent.v1:'))) storageRevision += 1;
        };
        window.addEventListener('storage', changed);
        return () => window.removeEventListener('storage', changed);
    });
    $effect(() => {
        const current = control;
        const revision = storageRevision;
        if (!mounted || !current) return;
        if (controller?.control.identity === current.identity && loadedStorageRevision === revision) { controller.refreshControl(current); status = controller.state; return; }
        try {
            const next = new TaskActionSubmission({...owner}, current, localStorage);
            const sameOwner = controller?.control.identity === current.identity;
            controller = next; status = next.state;
            loadedStorageRevision = revision;
            if (!sameOwner) draft = next.pendingGuidanceText ?? '';
            else if (!draft && next.pendingGuidanceText) draft = next.pendingGuidanceText;
        } catch { status = 'unknown'; }
    });
    const disabled = $derived(!control || !controller || !['ready', 'queued', 'rejected', 'delivered'].includes(status));
    async function submit(action: 'guidance' | 'cancel') {
        const captured = controller;
        if (disabled || !captured || !control || captured.control.identity !== control.identity
            || (action === 'guidance' ? !control.guidance : !control.cancellation)) return;
        const submittedDraft = draft;
        status = 'sending';
        const pending = captured.submit(action, submittedDraft, (path, body) => api(path, {method: 'POST', body: JSON.stringify(body), signal: AbortSignal.timeout(30_000)}));
        await pending;
        if (controller !== captured) return;
        status = captured.state;
        if (['queued', 'delivered'].includes(status) && draft === submittedDraft) draft = '';
    }
    async function recover() {
        const captured = controller;
        if (!captured?.canRecover) return;
        const submittedDraft = captured.pendingGuidanceText;
        status = 'sending';
        const pending = captured.recover((path, body) => api(path, {method:'POST', body:JSON.stringify(body), signal: AbortSignal.timeout(30_000)}));
        await pending;
        if (controller !== captured) return;
        status = captured.state;
        if (['queued', 'delivered'].includes(status) && submittedDraft !== null && draft.trim() === submittedDraft) draft = '';
    }

</script>

{#if control}
    <div class="task-actions">
        {#if control.guidance || controller?.pendingGuidanceText}<label>{t($locale, 'chatPage.taskGuidance')}<textarea bind:value={draft} disabled={disabled || !control.guidance} rows="2" maxlength="4096"></textarea></label>
            {#if control.guidance}<button disabled={disabled || !draft.trim()} onclick={() => submit('guidance')}>{t($locale, 'chatPage.taskSend')}</button>{/if}{/if}
        {#if control.cancellation}<button disabled={disabled} onclick={() => submit('cancel')}>{t($locale, 'chatPage.taskCancel')}</button>{/if}
        {#if status !== 'sending' && controller?.canRecover}<button onclick={recover}>{t($locale, 'chatPage.taskRecover')}</button>{/if}
        {#if clientId && control.cancellationObserved && !control.terminal && status !== 'sending'}
            <button onclick={inspectUnknown}>{t($locale, 'recovery.title')}</button>
        {/if}
        {#if recoveryOwner && taskContext}<ChatUnknownEffectRecovery owner={recoveryOwner} {taskContext} available={true}
            onResume={() => {}} onObserved={() => { recoveryOwner=null; taskContext=undefined; void recover(); onRecovered?.(); }} />{/if}
        {#if status !== 'ready'}<p role="status">{t($locale, `chatPage.taskAction_${status}`)}</p>{/if}
    </div>
{/if}

<style>
    .task-actions { display: flex; flex-wrap: wrap; gap: 8px; margin: 8px 0; }
    label { flex: 1 0 100%; display: grid; gap: 4px; font-size: 12px; }
    textarea { box-sizing: border-box; width: 100%; min-width: 0; }
    button { min-height: 44px; } p { flex: 1 0 100%; margin: 0; font-size: 12px; color: var(--muted); }
</style>
