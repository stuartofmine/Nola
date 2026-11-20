<script lang="ts">
    import { tick } from 'svelte';
    import { locale, t } from '../lib/i18n';
    import type { PersistentAgent, RoomParticipant } from '../lib/agentRooms';
    import { candidateIdentity, inlineQuery, mentionSegments, reconcileMentionEdit, replaceMentionRange, type InlineExpertMention } from '../lib/inlineExpertMentions';
    let {id, value, mentions, directory, members = [], disabled = false, placeholder = '', label, rows = 1,
        field = $bindable(), onedit, onload, onkeydown, onfocus, onblur, commandList, commandOption}: {
        id:string; value:string; mentions:InlineExpertMention[]; directory:PersistentAgent[]; members?:RoomParticipant[];
        disabled?:boolean; placeholder?:string; label?:string; rows?:number; field?:HTMLTextAreaElement;
        onedit:(text:string, occurrences:InlineExpertMention[])=>void; onload:()=>Promise<void>;
        onkeydown?:(event:KeyboardEvent)=>void; onfocus?:()=>void; onblur?:()=>void; commandList?:string; commandOption?:string;
    } = $props();
    let cursor=$state(0), focused=$state(false), composing=$state(false), dismissed=$state(false), index=$state(0);
    let loading=$state(false), failure=$state(''), scrollY=$state(0);
    const query=$derived(focused && !composing && !dismissed ? inlineQuery(value,cursor,mentions) : null);
    const options=$derived.by(()=>{
        if (!query) return [];
        const needle=query.query.toLocaleLowerCase();
        const joined=new Set(members.filter(m=>m.active).map(m=>m.agent_id));
        return directory.filter(a=>a.availability==='active' && (a.name+' '+a.description).toLocaleLowerCase().includes(needle))
            .sort((a,b)=>Number(joined.has(b.id))-Number(joined.has(a.id)));
    });
    const ambiguous=$derived.by(()=>{
        const counts=new Map<string,number>();
        for(const a of options){const key=JSON.stringify([a.name,a.description]);counts.set(key,(counts.get(key)??0)+1);}
        return counts;
    });
    const pieces=$derived(mentionSegments(value,mentions).segments);
    let marker=$state<HTMLSpanElement>(), menu=$state<HTMLDivElement>(), root=$state<HTMLDivElement>();
    let menuBelow=$state(false), inset=$state(0), available=$state(220);
    function layoutMenu(){
        if(!marker||!menu||!root)return;
        const caret=marker.getBoundingClientRect(),box=root.getBoundingClientRect(),viewport=window.visualViewport;
        const top=viewport?.offsetTop??0,bottom=top+(viewport?.height??window.innerHeight);
        const above=Math.max(0,box.top-top-8), below=Math.max(0,bottom-box.bottom-8);
        menuBelow=above<Math.min(menu.scrollHeight,220)&&below>above;
        available=Math.max(0,Math.min(220,menuBelow?below:above));
        inset=window.innerWidth<=700?0:Math.max(0,Math.min(caret.left-box.left,box.width-menu.offsetWidth));
    }
    $effect(()=>{
        if(!query||!menu||!root)return;
        query;options;marker;scrollY;
        void tick().then(layoutMenu);
        window.addEventListener('resize',layoutMenu);window.visualViewport?.addEventListener('resize',layoutMenu);
        return ()=>{window.removeEventListener('resize',layoutMenu);window.visualViewport?.removeEventListener('resize',layoutMenu);};
    });
    type Snapshot={text:string;mentions:InlineExpertMention[];start:number;end:number};
    let history:Snapshot[]=[], future:Snapshot[]=[];
    let ownValue='';
    $effect(()=>{if(value!==ownValue){history=[];future=[];before=null;ownValue=value;}});
    function update(text:string, items:InlineExpertMention[]){ownValue=text;onedit(text,items);}
    let before:{start:number;end:number;text:string}|null=null;
    function snapshot():Snapshot { return {text:value,mentions:[...mentions],start:field?.selectionStart??0,end:field?.selectionEnd??0}; }
    function remember() { history.push(snapshot()); if(history.length>100)history.shift();future=[]; }
    function place(start:number,end=start) { cursor=end; void tick().then(()=>{field?.focus();field?.setSelectionRange(start,end);}); }
    async function lookup() {
        if (!focused || composing || dismissed || !inlineQuery(value,cursor,mentions) || loading || directory.length) return;
        loading=true;failure='';
        try{await onload();}catch{failure=t($locale,'rooms.expertCatalogUnavailable');}finally{loading=false;}
    }
    function position() { if(!field)return;cursor=field.selectionEnd; if(field.selectionStart!==cursor)dismissed=true; }
    function changed(event:Event) {
        const ta=event.currentTarget as HTMLTextAreaElement, next=ta.value, caret=ta.selectionEnd;
        const old=before;before=null;
        const count=old && next.length-old.text.length+old.end-old.start;
        const exact=old && count!==null && count>=0 && old.text.slice(0,old.start)===next.slice(0,old.start)
            && old.text.slice(old.end)===next.slice(old.start+count);
        const out=exact && old && count!==null ? replaceMentionRange(old.text,mentions,old.start,old.end,next.slice(old.start,old.start+count)).mentions
            : reconcileMentionEdit(value,next,caret,mentions);
        update(next,out);cursor=caret;index=0;
        const kind=(event as InputEvent).inputType;
        dismissed=kind==='insertFromPaste'||kind==='insertFromDrop';
        if(!composing)void tick().then(lookup);
    }
    function beforeInput(event:InputEvent) {
        if(!field)return;
        let start=field.selectionStart,end=field.selectionEnd;
        if(event.inputType==='historyUndo'||event.inputType==='historyRedo'){
            const source=event.inputType==='historyUndo'?history:future,target=source===history?future:history;
            const previous=source.pop();
            if(previous){event.preventDefault();target.push(snapshot());update(previous.text,previous.mentions);dismissed=true;place(previous.start,previous.end);}
            return;
        }
        remember();
        if(start===end && !composing && ['deleteContentBackward','deleteContentForward'].includes(event.inputType)){
            const boundary=event.inputType==='deleteContentBackward'?start-1:start;
            const hit=mentions.find(m=>m.start<=boundary && boundary<m.start+m.text.length);
            if(hit){start=hit.start;end=hit.start+hit.text.length;field.setSelectionRange(start,end);}
        }
        before={start,end,text:value};
    }
    function choose(agent:PersistentAgent) {
        if(!query || disabled || !field)return;
        remember();
        const member=members.find(m=>m.active&&m.agent_id===agent.id);
        const result=replaceMentionRange(value,mentions,query.start,query.end,'@'+(member?.display_name??agent.name),candidateIdentity(agent,member));
        dismissed=true;update(result.text,result.mentions);place(result.cursor);
    }
    function key(event:KeyboardEvent){
        if(composing || event.isComposing || event.keyCode===229)return;
        if((event.ctrlKey||event.metaKey)&&!event.altKey&&event.key.toLowerCase()==='z'){
            const source=event.shiftKey?future:history,target=event.shiftKey?history:future,previous=source.pop();
            if(previous){event.preventDefault();target.push(snapshot());update(previous.text,previous.mentions);dismissed=true;place(previous.start,previous.end);}return;
        }
        if(query && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey){
            if(event.key==='Escape'){event.preventDefault();event.stopPropagation();dismissed=true;return;}
            if(['ArrowDown','ArrowUp','Enter','Tab'].includes(event.key) && (event.key!=='Tab'||options.length)){
                event.preventDefault();
                if(options.length){
                    index=Math.min(index,options.length-1);
                    if(event.key==='Enter'||event.key==='Tab')choose(options[index]);
                    else {index=(index+(event.key==='ArrowUp'?-1:1)+options.length)%options.length;document.getElementById(id+'-option-'+index)?.scrollIntoView({block:'nearest'});}
                }return;
            }
        }
        if(field && !event.shiftKey && !event.altKey && !event.metaKey && !event.ctrlKey && field.selectionStart===field.selectionEnd){
            const pos=field.selectionStart, left=event.key==='ArrowLeft';
            if(left||event.key==='ArrowRight'){
                const mention=mentions.find(m=>left?m.start<pos&&pos<=m.start+m.text.length:m.start<=pos&&pos<m.start+m.text.length);
                if(mention){event.preventDefault();dismissed=true;place(left?mention.start:mention.start+mention.text.length);return;}
            }
        }
        onkeydown?.(event);
    }
</script>
<div bind:this={root} class="inline-editor" class:editing={rows>1}>
    {#if query}
        <div bind:this={menu} class="options" class:below={menuBelow} style:--available={available+'px'} style:--inset={inset+'px'} role="listbox" id={id+'-options'} aria-label={t($locale,'rooms.experts')}>
            {#each options as agent,i (agent.id)}
                <button type="button" role="option" id={id+'-option-'+i} tabindex="-1" aria-selected={i===index}
                    onpointerdown={event=>event.preventDefault()} onclick={()=>choose(agent)}>
                    <strong>{agent.name}</strong><small>{agent.description}{#if (ambiguous.get(JSON.stringify([agent.name,agent.description]))??0)>1} · {agent.id}{/if}</small>
                </button>
            {:else}<span role="status">{failure || t($locale,loading?'rooms.loading':'rooms.noExperts')}</span>{/each}
        </div>
    {/if}
    <div class="stack" class:marked={mentions.length>0}>
        {#if query}<div class="highlight measure" aria-hidden="true"><div style:transform={'translateY(-'+scrollY+'px)'}>{value.slice(0,cursor)}<span bind:this={marker}>{'\u200b'}</span></div></div>{/if}
        {#if mentions.length}<div class="highlight" aria-hidden="true"><div style:transform={'translateY(-'+scrollY+'px)'}>{#each pieces as piece}{#if piece.agent_id}<strong>{piece.text}</strong>{:else}{piece.text}{/if}{/each}{'\u200b'}</div></div>{/if}
        <textarea bind:this={field} {id} {value} {rows} {placeholder} {disabled} aria-label={label} role="combobox" aria-autocomplete="list"
            aria-expanded={!!query||!!commandList} aria-controls={query?id+'-options':commandList} aria-activedescendant={query&&options.length?id+'-option-'+index:commandOption}
            onscroll={e=>scrollY=e.currentTarget.scrollTop} onselect={position} onclick={()=>{dismissed=false;position();void lookup();}}
            onfocus={()=>{focused=true;position();onfocus?.();void lookup();}} onblur={()=>{focused=false;onblur?.();}}
            oncompositionstart={()=>composing=true} oncompositionend={()=>{composing=false;void tick().then(lookup);}}
            onbeforeinput={beforeInput} oninput={changed} onkeydown={key}></textarea>
    </div>
</div>
<style>
.inline-editor{position:relative;min-width:0}
.stack{position:relative;color:var(--content);font:inherit}
textarea,.highlight{box-sizing:border-box;margin:0;width:100%;font:inherit;font-size:max(16px,1rem);line-height:1.5;letter-spacing:normal;padding:8px 12px;border:0;white-space:pre-wrap;overflow-wrap:break-word;tab-size:8}
textarea{display:block;position:relative;min-height:40px;max-height:min(30dvh,150px);resize:none;field-sizing:content;background:transparent;color:inherit;box-shadow:none}
textarea:focus{outline:none}textarea::placeholder{color:var(--muted);opacity:1}
.highlight{pointer-events:none;position:absolute;inset:0;overflow:hidden;color:var(--content);scrollbar-gutter:stable}.highlight strong{font-weight:inherit;color:var(--primary);text-decoration:underline;text-decoration-color:var(--accent-subtle);text-underline-offset:.14em}
.measure{visibility:hidden}
.marked textarea{color:transparent;caret-color:var(--content);scrollbar-gutter:stable}
.editing .stack{border:1px solid var(--border);border-radius:10px;background:var(--canvas)}.editing textarea{min-height:150px;max-height:45dvh}
.options{position:absolute;z-index:4;bottom:100%;inset-inline-start:var(--inset,0);width:min(100%,340px);box-sizing:border-box;max-height:var(--available,220px);overflow:auto;padding:4px;background:var(--surface,var(--canvas));border:1px solid var(--border);border-radius:12px;box-shadow:0 6px 20px #0002}
.options.below{bottom:auto;top:100%}.editing .options{position:static;max-height:180px}.options button{display:block;width:100%;text-align:start;border:0;background:transparent;color:inherit;padding:7px 10px;border-radius:7px;min-height:44px;box-shadow:none}.options strong,.options small{display:block}.options small{color:var(--muted)}.options button:hover,.options button[aria-selected=true]{background:var(--accent-subtle)}
@media(max-width:700px){.options{width:100%}textarea,.highlight{padding:10px 14px}textarea{min-height:44px}}
@media(forced-colors:active){.marked textarea{color:CanvasText;caret-color:CanvasText}.highlight{visibility:hidden}}
</style>
