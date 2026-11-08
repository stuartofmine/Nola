<script lang="ts">
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onDestroy, untrack } from 'svelte';
    import { api } from '../lib/api';
    import { committedConversationConfiguration } from '../lib/conversationConfigurationCommit';
    import { locale, t } from '../lib/i18n';
    import { knowledgeDocument, knowledgeDirty, knowledgePatch, knowledgeUrl, reconcileKnowledge, resolveKnowledge } from '../lib/chatKnowledge';
    import type { KnowledgeDraft, KnowledgeField } from '../lib/chatKnowledge';
    import ChatRecall from './ChatRecall.svelte';
    let { conversationId, generation, busy = false, drafts = new Map<string, KnowledgeDraft>() }: {
        conversationId: string; generation: number; busy?: boolean; drafts?: Map<string, KnowledgeDraft>;
    } = $props();
    let draft = $state<KnowledgeDraft>();
    const panes = ['knowledgePoints', 'wiki', 'recall'] as const;
    let pane = $state<typeof panes[number]>('knowledgePoints');
    function paneKey(event: KeyboardEvent, index: number) {
        if (event.altKey || event.ctrlKey || event.metaKey) return;
        const next = event.key === 'ArrowRight' ? (index + 1) % panes.length
            : event.key === 'ArrowLeft' ? (index + panes.length - 1) % panes.length
            : event.key === 'Home' ? 0 : event.key === 'End' ? panes.length - 1 : null;
        if (next === null) return;
        event.preventDefault(); pane = panes[next];
        (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
    }
    let loading = $state(false), writing = $state(false), error = $state(false), saved = $state(false);
    let disposed = false, sequence = 0;
    let reader: AbortController | undefined;
    const dirty = $derived(!!draft && knowledgeDirty(draft));
    const locked = $derived(busy || loading || writing);
    const saveBlocked = $derived(locked || error || !!draft?.uncertain || !!draft?.conflict);
    const current = (id: string, epoch: number, token: number) => !disposed && id === conversationId && epoch === generation && token === sequence;
    async function load() {
        const id = conversationId, epoch = generation, token = ++sequence;
        reader?.abort(); const controller = new AbortController(); reader = controller;
        loading = true; error = false; saved = false;
        try {
            const document = knowledgeDocument(await api(knowledgeUrl(id), {signal: controller.signal}), id);
            if (!current(id, epoch, token)) return;
            const retained = drafts.get(id);
            draft = reconcileKnowledge(retained, document);
            if (knowledgeDirty(draft)) drafts.set(id, draft);
            else drafts.delete(id);
        } catch { if (current(id, epoch, token)) error = true; }
        finally { if (current(id, epoch, token)) loading = false; }
    }
    $effect(() => {
        conversationId; generation;
        untrack(() => { draft = drafts.get(conversationId); writing = false; void load(); });
    });
    onDestroy(() => { disposed = true; reader?.abort(); });
    $effect(() => {
        if (!draft || !dirty || saveBlocked) return;
        const timer = setTimeout(() => void save(), 450);
        return () => clearTimeout(timer);
    });
    function edit(field: KnowledgeField, value: string) {
        if (!draft || locked || draft.uncertain) return;
        draft = {...draft, baseline: {...draft.baseline}, value: {...draft.value, [field]: value}};
        drafts.set(conversationId, draft);
        saved = false;
    }
    function resolve(choice: 'remote' | 'local') {
        if (!draft?.conflict || locked) return;
        draft = resolveKnowledge(draft, choice);
        if (knowledgeDirty(draft)) drafts.set(conversationId, draft);
        else drafts.delete(conversationId);
        error = false;
    }
    async function save() {
        if (!draft || !dirty || saveBlocked) return;
        const id = conversationId, epoch = generation, token = ++sequence;
        const patch = knowledgePatch(draft);
        const submitted = {...draft.value};
        draft = {...draft, uncertain: true};
        drafts.set(id, draft);
        writing = true; error = false; saved = false;
        try {
            const result = knowledgeDocument(committedConversationConfiguration(await api(knowledgeUrl(id), {method: 'PATCH', body: JSON.stringify(patch)})), id);
            if (!current(id, epoch, token)) return;
            // CAS guarantees the untouched field is still our original base.
            // Preserve Markdown byte-for-byte, including surrounding whitespace.
            if (result.revision !== patch.expectedRevision + 1
                || result.document.knowledgePoints !== submitted.knowledgePoints
                || result.document.wiki !== submitted.wiki) throw new Error('Unconfirmed write');
            draft = reconcileKnowledge(undefined, result);
            drafts.delete(id); saved = true;
        } catch { if (current(id, epoch, token)) error = true; }
        finally { if (current(id, epoch, token)) writing = false; }
    }
</script>

<svelte:window onbeforeunload={event => { if (!acceptsWorkspaceUnload(event)) return; if (dirty || writing) { event.preventDefault(); event.returnValue = ''; } }} />
<div class="knowledge-panes" role="tablist" aria-label={t($locale, 'chatPage.knowledge')}>
    {#each panes as item, index}<button type="button" role="tab" id={`knowledge-tab-${item}`} aria-controls={`knowledge-panel-${item}`} aria-selected={pane === item} tabindex={pane === item ? 0 : -1} onclick={() => pane = item} onkeydown={event => paneKey(event, index)}>{t($locale, `chatKnowledge.${item}`)}</button>{/each}
</div>
<section class="knowledge-editor" aria-busy={loading || writing} hidden={pane === 'recall'}>
    <p class="muted">{t($locale, 'chatKnowledge.hint')}</p>
    <p class="muted">{t($locale, 'chatKnowledge.autosave')}</p>
    {#if loading}<p role="status">{t($locale, 'chatKnowledge.loading')}</p>{/if}
    {#if error}<p role="alert">{t($locale, 'chatKnowledge.failed')}</p>{/if}
    {#each ['knowledgePoints', 'wiki'] as field}
        <div role="tabpanel" id={`knowledge-panel-${field}`} aria-labelledby={`knowledge-tab-${field}`} hidden={pane !== field} tabindex="0">
            {#if draft}<label>{t($locale, `chatKnowledge.${field}`)}<textarea value={draft.value[field as KnowledgeField]} oninput={event => edit(field as KnowledgeField, event.currentTarget.value)} disabled={locked || draft.uncertain} rows="12"></textarea></label>{/if}
        </div>
    {/each}
    {#if draft?.conflict && draft.remote}
        <p role="alert">{t($locale, 'chatKnowledge.conflict')}</p>
        {#each ['knowledgePoints', 'wiki'] as field}
            <label>{t($locale, 'chatKnowledge.latest')}: {t($locale, `chatKnowledge.${field}`)}<textarea readonly value={draft.remote.document[field as KnowledgeField]} rows="4"></textarea></label>
        {/each}
        <div class="actions"><button type="button" disabled={locked} onclick={() => resolve('remote')}>{t($locale, 'chatKnowledge.useLatest')}</button><button type="button" disabled={locked} onclick={() => resolve('local')}>{t($locale, 'chatKnowledge.useDraft')}</button></div>
    {/if}
    <div class="actions"><button type="button" disabled={saveBlocked || !dirty} onclick={save}>{t($locale, 'chatKnowledge.save')}</button><button type="button" disabled={locked} onclick={load}>{t($locale, 'chatKnowledge.reload')}</button></div>
    {#if dirty || saved}<p role="status">{t($locale, dirty ? 'chatKnowledge.unsaved' : 'chatKnowledge.saved')}</p>{/if}
</section>
<div role="tabpanel" id="knowledge-panel-recall" aria-labelledby="knowledge-tab-recall" hidden={pane !== 'recall'} tabindex="0"><ChatRecall {conversationId} {generation} /></div>
<style>
    .knowledge-panes { display: flex; gap: 3px; padding: 3px; background: var(--panel); border-radius: 10px; }
    .knowledge-panes button { flex: 1; min-width: 0; padding: 7px 4px; border-color: transparent; background: transparent; font-size: 11px; overflow-wrap: anywhere; }
    .knowledge-panes button[aria-selected='true'] { border-color: transparent; background: var(--accent-subtle); }
    [hidden] { display: none !important; }
    .knowledge-editor, label { display: grid; gap: 10px; min-width: 0; }
    .knowledge-editor { padding: 12px; border: 1px solid var(--border); border-radius: 10px; }
    textarea { width: 100%; min-width: 0; resize: vertical; box-sizing: border-box; padding: 12px; border: 1px solid var(--outline); border-radius: 8px; background: var(--surface); color: var(--content); font: inherit; font-weight: 400; line-height: 1.5; }
    label { font-size: 12px; font-weight: 600; }
    p { margin: 0; font-size: 12px; overflow-wrap: anywhere; }
    .muted { color: var(--muted); } [role="alert"] { color: var(--danger); }
    .actions { display: flex; flex-wrap: wrap; gap: 8px; } button { min-height: 44px; }
</style>
