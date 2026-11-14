/* Read-only synthetic research reply, shared by desktop and mobile prototypes. */
window.replyPresentation = (() => {
    const owners = new Map();
    const escape = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
    const sources = Array.from({length:31}, (_, i) => ({title: ['模型发布公告与能力说明','长任务与工具恢复的比较方法','跨平台交互与引用格式参考'][i % 3] + (i > 2 ? ` · ${i + 1}` : ''), url:`https://example.invalid/reference/${i + 1}`}));
    function state(owner) { if (!owners.has(owner)) owners.set(owner, {stage:'incomplete', all:false, expanded:false}); return owners.get(owner); }
    function render(owner) {
        const value = state(owner), live = ['reading','waiting'].includes(value.stage);
        const heading = live ? value.stage === 'reading' ? '正在阅读来源' : '正在处理中' : '执行过程';
        const status = live ? '进行中' : value.stage === 'completed' ? '已完成' : '未完成';
        return `<section class="reply-presentation" data-reply-owner="${escape(owner)}" aria-label="研究过程与来源">
            <div class="reply-table-scroll"><table><thead><tr><th>时间</th><th>事件</th><th>经过和影响</th></tr></thead><tbody><tr><td>阶段一</td><td>整理资料</td><td>正文说明保留阅读层级。<a class="inline-reference" href="https://example.invalid/inline/one" target="_blank" rel="noopener noreferrer">来源</a></td></tr><tr><td>阶段二</td><td>比较记录</td><td>引用紧随所支持的句子。<a class="inline-reference" href="https://example.invalid/inline/two" target="_blank" rel="noopener noreferrer">来源</a></td></tr></tbody></table></div>
            <details class="reply-process" ${value.expanded ? 'open' : ''}><summary><span ${live?'class="reply-live"':''}>${heading}</span><span class="reply-chevron" aria-hidden="true">⌄</span></summary><p>已读取来源并整理比较维度。单个工具步骤完成后，模型仍可继续；历史步骤不代表整轮结论。</p></details>
            <div class="reply-skill"><span>✧ 深度研究</span><small>${status}</small><details class="reply-info"><summary aria-label="研究详情">ⓘ</summary><div><p>模型轮次 8</p><p>工具调用 10</p><p>1 分 57 秒</p></div></details></div>
            <div class="reply-references"><p>来源 <span>31</span></p><ol>${sources.slice(0,value.all?31:3).map((source,i)=>`<li><span>[${i+1}]</span><a href="${source.url}" title="${source.url}" target="_blank" rel="noopener noreferrer">${escape(source.title)}</a></li>`).join('')}</ol><button type="button" data-reference-action="sources" aria-expanded="${value.all}">${value.all?'收起来源':'展开全部来源'}</button></div>
        </section>`;
    }
    document.addEventListener('click', event => {
        const control = event.target.closest('[data-reference-action]'); if (!control) return;
        const root = control.closest('[data-reply-owner]'); if (!root) return;
        const owner = root.dataset.replyOwner; state(owner).expanded = root.querySelector('.reply-process').open; state(owner).all = !state(owner).all;
        root.outerHTML = render(owner);
        document.querySelectorAll('[data-reply-owner]').forEach(element => { if (element.dataset.replyOwner === owner) element.querySelector('[data-reference-action]')?.focus({preventScroll:true}); });
    });
    document.addEventListener('toggle', event => {
        if (event.target.isConnected && event.target.matches?.('.reply-process')) state(event.target.closest('[data-reply-owner]').dataset.replyOwner).expanded = event.target.open;
    }, true);
    return {message: (message, owner) => message.research ? render(owner) : '', render,
        stage(owner, stage) { if (!['reading','waiting','incomplete','completed'].includes(stage)) return; const root = [...document.querySelectorAll('[data-reply-owner]')].find(element => element.dataset.replyOwner === owner); if (root) state(owner).expanded = root.querySelector('.reply-process').open; state(owner).stage = stage; return render(owner); }};
})();
