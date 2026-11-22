let pendingMessageEdit=null;
function messageActions(m,i,c){return `<div class="message-actions" aria-label="消息操作">${m.text?`<button data-action="message-copy:${c.id}:${i}" title="复制消息">${svg('copy')}<span>复制</span></button>`:''}${m.role==='user'?`<button data-action="message-edit:${c.id}:${i}" ${stream?'disabled title="请先停止或等待当前生成完成"':'title="编辑后在当前会话重发送"'}>${svg('edit')}<span>编辑重发</span></button>${stream?'<span class="action-note">生成中，暂不可编辑</span>':''}`:''}${m.error?`<button data-action="message-retry:${c.id}:${i}" ${stream?'disabled':''}>${svg('retry')}<span>重试</span></button>`:''}</div>`}
async function copyMessage(cid,index){const m=conversations.find(c=>c.id===cid)?.messages[index];if(!m)return;try{if(!navigator.clipboard?.writeText)throw new Error('unavailable');await navigator.clipboard.writeText(m.text);toast('已复制消息')}catch(e){if(dialog.open)closeDialog();openDialog('手动复制消息',`<p class="dialog-note">浏览器未允许自动复制。请选中文字后使用系统复制。</p><textarea id="manual-message-copy" aria-label="待复制消息" readonly>${esc(m.text)}</textarea><div class="dialog-actions"><button class="primary" data-action="close">完成</button></div>`);dialog.querySelector('textarea').select()}}
function editMessage(cid,index){
    if(stream){toast('请先停止或等待当前生成完成');return;}
    const conversation=conversations.find(c=>c.id===cid);
    const message=conversation?.messages[index];
    if(message?.role!=='user')return;
    const snapshot=structuredClone({
        mentions:message.sendContext?.mentions||[],
        mainExpertSelection:message.sendContext?.mainExpertSelection,
        model:conversation.model||defaultModel,
        reasoningEffort:conversation.desktopEffort||conversation.reasoningEffort||'自动',
        permissionMode:conversation.permissionMode||'谨慎'
    });
    const edit={cid,index,message,original:message.text,snapshot,submitted:false};
    pendingMessageEdit=edit;
    openDialog('编辑并重发送',`
        <p class="dialog-note">重发后，从这条消息起重新生成。后面的回复及其专家任务会一并替换。</p>
        <label for="resend-text">消息内容</label>
        <div class="er-edit-field"><textarea id="resend-text" rows="5">${esc(message.text)}</textarea></div>
        <p class="dialog-note">本次重发使用 ${esc(snapshot.model)}</p>
        <p id="resend-error" role="status" class="dialog-note"></p>
        <button type="button" class="text-action" id="resend-as-text" hidden>将本条提及转为普通文字</button>
        <div class="dialog-actions"><button class="text-action" data-action="message-edit-cancel">取消</button>
        <button class="primary" data-action="message-edit-submit">保存并重发送</button></div>`);
    const input=dialog.querySelector('#resend-text'),submit=dialog.querySelector('[data-action="message-edit-submit"]');
    edit.readMentions=expertRolesModule.editMentions(input,cid,snapshot);
    input.oninput=()=>{submit.disabled=!input.value.trim();};
    input.focus();
}
function submitMessageEdit(){
    const edit=pendingMessageEdit;
    if(!edit||edit.submitted)return;
    const text=dialog.querySelector('#resend-text').value,error=dialog.querySelector('#resend-error');
    if(!text.trim()){error.textContent='消息不能为空';return;}
    if(stream){error.textContent='请先停止或等待当前生成完成';return;}
    const original=conversations.find(c=>c.id===edit.cid);
    if(original?.messages[edit.index]!==edit.message||edit.original!==edit.message.text){
        error.textContent='原消息已变化，请重新打开编辑';return;
    }
    const selection=edit.readMentions();
    if(selection.error){
        error.textContent=selection.error;
        const button=dialog.querySelector('#resend-as-text');
        button.hidden=!selection.asText;
        button.onclick=()=>{selection.asText();button.hidden=true;error.textContent='';};
        return;
    }
    edit.submitted=true;
    const context={...edit.snapshot,...selection};
    original.messages.splice(edit.index,original.messages.length-edit.index,
        {role:'user',text,sendContext:context},
        {role:'Nola',text:`已按修改后的消息重新整理：\n\n${text}\n\n这是当前会话的本地演示回复。`,expertTasks:demoExpertTasks(context)});
    original.preview=text;
    original.time='刚刚';
    pendingMessageEdit=null;
    closeDialog();
    navigate('chat',original.id);
    toast('已在当前会话重发送');
}
