'use strict';
// UI-only update states. Never fetch releases, download packages or install apps.
window.updatePreview = (() => {
    let status = 'not_checked', outcome = 'available', timer;
    const labels = {not_checked:'尚未检查更新',checking:'正在检查更新…',available:'演示：有可用的新版本',up_to_date:'演示：已是最新版本',failed:'检查失败，当前版本与已有数据保持不变。',incompatible_data:'数据格式不兼容，已阻止更新并保留原始数据。'};
    const button = (title, action, cls = 'text-action') => `<button type="button" class="${cls}" data-action="update:${action}">${title}</button>`;
    function content() {
        return `<section class="update-preview" data-update-page aria-busy="${status==='checking'}"><span class="demo-banner">交互原型 · 版本与检查结果均为演示</span><h2>Nola 1.0.0</h2><p role="${status==='failed'||status==='incompatible_data'?'alert':'status'}" class="update-status">${labels[status]}</p>${status==='available'?'<p>演示可用版本 1.0.1 · 修复与体验改进</p>':''}<div class="action-wrap"><button type="button" class="primary" data-action="update:check" ${status==='checking'?'disabled':''}>${status==='failed'?'重新检查':'检查更新（演示）'}</button>${status==='available'?button('预览安装步骤','steps'):''}</div><p class="dialog-note">安装前先完成运行中的任务。更新不会自动转换、删除或重建会话数据。</p><details class="prototype-state-controls"><summary>原型状态预览</summary><p>仅改变此演示的检查结果，不访问 GitHub 或当前 Host。</p><div class="action-wrap">${[['available','发现更新'],['up_to_date','已是最新'],['failed','检查失败'],['incompatible_data','格式不兼容']].map(([id,label])=>`<button type="button" aria-pressed="${outcome===id}" data-action="update:outcome:${id}">${label}</button>`).join('')}</div></details></section>`;
    }
    function refresh() {
        const action=document.activeElement?.dataset.action;
        render();
        if(action)document.querySelector(`[data-action="${CSS.escape(action)}"]:not(:disabled)`)?.focus({preventScroll:true});
    }
    function handle(action) {
        if(!action.startsWith('update:'))return false;
        const [,kind,value]=action.split(':');
        if(kind==='outcome'){outcome=value;refresh();}
        if(kind==='check'&&status!=='checking'){
            status='checking';refresh();clearTimeout(timer);
            const selected=outcome;
            timer=setTimeout(()=>{status=selected;if(document.querySelector('[data-update-page]'))refresh();},350);
        }
        if(kind==='steps')openDialog('安装前确认',`<p>以下仅预览安装流程，不下载、验证或打开真实安装程序。</p><ol><li>完成或明确暂停运行中的任务，保留草稿。</li><li>在真实客户端完成下载与签名、完整性验证。</li><li>用户明确打开安装程序；格式不兼容时停止，保留原始数据。</li></ol><div class="dialog-actions">${button('返回更新页','close')}</div>`);
        if(kind==='close')closeDialog();
        return true;
    }
    function view(route) {
        return route==='updates'?`<section class="page"><div class="subnav">${button('设置','settings')}</div>${pageHead('软件更新','版本、检查结果与下一步保持清楚。')}${content()}</section>`:null;
    }
    const dispatch=handle;
    return {content,view,handle:action=>{if(action==='update:settings'){navigate('settings');return true;}return dispatch(action);}};
})();
