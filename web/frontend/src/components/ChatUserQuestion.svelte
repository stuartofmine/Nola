<script lang="ts">
    import { activityRequestFocus } from '../lib/activityRequestFocus';
    import { api, ApiError } from '../lib/api';
    import { conversationUrl } from '../lib/chatConversations';
    import { locale, t } from '../lib/i18n';
    import { freezeWaitResponse, type PresentedWait, type WaitAnswer, type WaitResponse } from '../lib/userInputWait';
    let { record, stale = false, onresolved, onrefresh }: { record: PresentedWait; stale?: boolean; onresolved: () => void; onrefresh: () => void } = $props();
    let choices = $state<Record<string, string>>({});
    let texts = $state<Record<string, string>>({});
    let writing = $state(false);
    let pendingBody = $state<string | null>(null);
    let error = $state('');
    let conflict = $state(false);
    let resolved = $state(false);
    const locked = $derived(writing || pendingBody !== null || stale || conflict || resolved);
    async function send(response?: WaitResponse) {
        if (writing || stale || conflict || resolved) return;
        if (!pendingBody && response) pendingBody = freezeWaitResponse(record, response, crypto.randomUUID());
        if (!pendingBody) return;
        writing = true; error = '';
        try {
            const reply = await api<{ type: string; wait?: { conversation_id: string; run_id: string; wait: { wait_id: string; state: { type: string; command_id?: string } } }; wake_pending?: boolean }>(
                `${conversationUrl(record.conversation_id)}/user-input-waits/respond`, { method: 'POST', body: pendingBody });
            const owner = reply.wait;
            const command = JSON.parse(pendingBody).response.command_id;
            if (reply.type !== 'resolved' || owner?.conversation_id !== record.conversation_id || owner.run_id !== record.run_id
                || owner.wait.wait_id !== record.wait.wait_id || owner.wait.state.type !== 'resolved' || owner.wait.state.command_id !== command)
                throw new Error('Invalid answer receipt');
            resolved = true; pendingBody = null;
            if (reply.wake_pending) error = 'chatActivity.answerWakePending';
            onresolved();
        } catch (e) {
            if (e instanceof ApiError && e.status === 409) { conflict = true; error = 'chatActivity.answerConflict'; }
            else error = 'chatActivity.answerRetry';
        } finally { writing = false; }
    }
    function answer(event: SubmitEvent) {
        event.preventDefault();
        const answers: WaitAnswer[] = [];
        for (const q of record.wait.question_set.questions) {
            const choice = choices[q.id];
            if (choice && choice !== 'freeform') answers.push({ question_id: q.id, value: { type: 'option', option_id: choice.slice(7) } });
            else if (q.allow_freeform && texts[q.id]?.trim()) answers.push({ question_id: q.id, value: { type: 'text', text: texts[q.id] } });
        }
        void send({ type: 'answers', answers });
    }
</script>
<article tabindex="-1" use:activityRequestFocus={{conversationId: record.conversation_id, runId: record.run_id, active: !stale && !resolved}} class="question" aria-label={t($locale, 'chatActivity.questionTitle')}>
    <h3>{t($locale, record.task_id ? 'chatActivity.taskQuestion' : 'chatActivity.questionTitle')}</h3>
    {#if resolved}<p role="status">{t($locale, 'chatActivity.answerSaved')}</p>{:else}
    <form onsubmit={answer}>
        {#each record.wait.question_set.questions as q (q.id)}
            <fieldset disabled={locked}>
                <legend>{q.prompt}{q.required ? ' *' : ''}</legend>
                {#each q.options as option (option.id)}
                    <label class="option"><input type="radio" name={`${record.wait.wait_id}:${q.id}`} value={`option:${option.id}`} bind:group={choices[q.id]} required={q.required && !q.allow_freeform} />
                        <span>{option.label}{#if option.description}<small>{option.description}</small>{/if}</span></label>
                {/each}
                {#if q.allow_freeform}
                    {#if q.options.length}<label class="option"><input type="radio" name={`${record.wait.wait_id}:${q.id}`} value="freeform" bind:group={choices[q.id]} /><span>{t($locale, 'chatActivity.freeformAnswer')}</span></label>{/if}
                    <textarea aria-label={q.prompt} bind:value={texts[q.id]} oninput={() => choices[q.id] = 'freeform'} required={q.required && (!choices[q.id] || choices[q.id] === 'freeform')} rows="2"></textarea>
                {/if}
            </fieldset>
        {/each}
        <div class="actions">
            <button type="submit" disabled={locked}>{t($locale, 'chatActivity.answerSend')}</button>
            <button type="button" disabled={locked} onclick={() => send({ type: 'decline', reason: null })}>{t($locale, 'chatActivity.answerDecline')}</button>
        </div>
    </form>
    {/if}
    {#if error}<p role="alert">{t($locale, error)}</p>{/if}
    {#if pendingBody && !writing && !conflict}<button onclick={() => send()} disabled={stale}>{t($locale, 'chatActivity.retryAnswer')}</button>{/if}
    {#if conflict}<button onclick={onrefresh}>{t($locale, 'chatActivity.refresh')}</button>{/if}
</article>
<style>
    .question { border: 1px solid var(--border); border-radius: 12px; padding: 1rem; margin-block: 1rem; }
    h3 { font-size: 1rem; margin: 0 0 .75rem; } small { display: block; font-weight: normal; opacity: .7; overflow-wrap: anywhere; }
    fieldset { border: 0; padding: 0; margin: 0 0 1rem; min-width: 0; } legend { margin-bottom: .5rem; overflow-wrap: anywhere; }
    .option { display: flex; gap: .65rem; align-items: center; min-height: 44px; padding: .4rem; cursor: pointer; }
    textarea { width: 100%; box-sizing: border-box; resize: vertical; } .actions { display: flex; flex-wrap: wrap; gap: .5rem; }
    button { min-height: 44px; } p { overflow-wrap: anywhere; }
</style>
