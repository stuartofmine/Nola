<script lang="ts">
    import { workspaceUnloadScope } from '../lib/workspacePageLocation';
    const acceptsWorkspaceUnload = workspaceUnloadScope();
    import { onDestroy, tick, untrack } from 'svelte';
    import { locale } from '../lib/i18n.ts';
    import { editedResendLabel as label } from '../lib/editedResendLocale.ts';
    import { confirmEditedExpertResendCompletion } from '../lib/expertMessage.ts';
    import { editResendText, confirmEditedResendAdmission } from '../lib/editedResend.ts';
    import type { EditedExpertResendCompletion } from '../lib/expertMessage.ts';
    import type { EditedResendDraft, EditedResendCompletion } from '../lib/editedResend.ts';
    let { initial, busy=false, ownerChanged=false, onsubmit, onclose }: {
        initial: EditedResendDraft; busy?:boolean; ownerChanged?:boolean;
        onsubmit:(draft:EditedResendDraft)=>Promise<EditedResendCompletion | EditedExpertResendCompletion>; onclose:()=>void;
    }=$props();
    let draft=$state<EditedResendDraft>(untrack(()=>initial)), error=$state(''), sending=$state(false);
    let editor=$state<HTMLTextAreaElement>(), dialog=$state<HTMLDialogElement>();
    let copied=$state<'copied'|'manual'|null>(null);
    let alive=true;
    async function copyDraft(){
        const text=draft.input.edited_text;
        try{await navigator.clipboard.writeText(text);if(alive&&draft.input.edited_text===text)copied='copied';}
        catch{if(alive&&draft.input.edited_text===text){copied='manual';await tick();if(alive){editor?.focus();editor?.select();}}}
    }
    function portal(node:HTMLDialogElement){document.body.appendChild(node);void tick().then(()=>{if(alive){node.showModal();editor?.focus();}});return{destroy(){node.close();node.remove();}};}
    async function save(){
        if(sending||busy||ownerChanged||!draft.input.edited_text.trim())return;
        const captured=draft;
        sending=true;error='';
        try{
            const completed=await onsubmit(captured);
            if('receipts' in completed)confirmEditedExpertResendCompletion(captured,completed);
            else confirmEditedResendAdmission(captured,completed.envelope,completed.receipt);
            if(alive&&!ownerChanged)onclose();
        }catch(cause){if(alive)error=cause instanceof Error?cause.message:String(cause);}
        finally{if(alive)sending=false;}
    }
    function cancel(event:Event){event.preventDefault();if(!sending)onclose();}
    onDestroy(()=>{alive=false;});
</script>
<svelte:window onbeforeunload={event=>{if(acceptsWorkspaceUnload(event)&&(sending||draft.input.edited_text!==draft.source.originalText)){event.preventDefault();event.returnValue='';}}} />
<dialog bind:this={dialog} use:portal aria-labelledby="message-edit-title" oncancel={cancel}>
    <form onsubmit={event=>{event.preventDefault();void save();}}>
        <h2 id="message-edit-title">{label($locale,'edit')}</h2>
        <p class="muted">{label($locale,'detail')}</p>
        <textarea bind:this={editor} value={draft.input.edited_text} disabled={sending}
            aria-label={label($locale,'edit')} oninput={event=>{draft=editResendText(draft,event.currentTarget.value);error='';copied=null;}} rows="8"></textarea>
        {#if ownerChanged}<p role="status">{label($locale,'changed')}</p><button type="button" onclick={copyDraft}>{label($locale,'copyDraft')}</button>{:else if busy}<p role="status">{label($locale,'wait')}</p>{/if}
        {#if copied}<p role="status">{label($locale,copied)}</p>{/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="actions">
            <button type="button" disabled={sending} onclick={onclose}>{label($locale,'cancel')}</button>
            <button type="submit" class="primary" disabled={sending||busy||ownerChanged||!draft.input.edited_text.trim()}>{label($locale,sending?'sending':'save')}</button>
        </div>
    </form>
</dialog>
<style>
    dialog{width:min(560px,calc(100vw - 32px));max-height:calc(100dvh - 32px);padding:24px;box-sizing:border-box;overflow:auto;}
    form{display:grid;gap:16px;}h2,p{margin:0;overflow-wrap:anywhere;}h2{font-size:18px;}
    textarea{width:100%;min-height:160px;max-height:50dvh;box-sizing:border-box;resize:vertical;font:inherit;line-height:1.6;padding:12px;background:var(--canvas);color:var(--content);border:1px solid var(--border);border-radius:10px;}
    .actions{display:flex;justify-content:flex-end;gap:8px;flex-wrap:wrap;}.actions button{min-height:44px;}.error{color:var(--danger,#c63b3b);}
    @media(max-width:760px){dialog{padding:20px;}}
</style>
