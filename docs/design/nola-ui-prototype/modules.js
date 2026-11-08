/* Interactive design reference. Fictional data, local page state, no provider or device calls. */
'use strict';
const $ = (s, root = document) => root.querySelector(s);
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const paths = {
    logo: '<path d="M5 19V5l14 14V5"/>', chat: '<path d="M20 11.5a8 8 0 0 1-8 8H5l-3 3V11.5a9 9 0 0 1 18 0Z"/><path d="M7 9h8M7 13h5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>', compose: '<path d="m15 4 5 5M13 6l5 5-9 9H4v-5L16 3a2 2 0 0 1 3 0l2 2a2 2 0 0 1 0 3"/>',
    back: '<path d="m14 5-7 7 7 7"/>', chevron: '<path d="m9 5 7 7-7 7"/>', down: '<path d="m6 9 6 6 6-6"/>',
    arrow: '<path d="M12 20V4m-6 6 6-6 6 6"/>', latest: '<path d="M12 4v16m-6-6 6 6 6-6"/>', close: '<path d="m6 6 12 12M6 18 18 6"/>',
    more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
    data: '<ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v7c0 4 16 4 16 0V5M4 12v7c0 4 16 4 16 0v-7"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 2"/>',
    settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3" fill="var(--surface)"/><circle cx="15" cy="17" r="3" fill="var(--surface)"/>',
    person: '<circle cx="12" cy="8" r="4"/><path d="M4 21v-2a8 8 0 0 1 16 0v2"/>',
    model: '<rect x="6" y="6" width="12" height="12" rx="3"/><path d="M9 2v4m6-4v4M9 18v4m6-4v4M2 9h4m-4 6h4m12-6h4m-4 6h4"/>',
    skills: '<path d="m12 3 9 5v8l-9 5-9-5V8l9-5Zm0 10v8M3 8l9 5 9-5"/>',
    search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
    folder: '<path d="M3 7V5a2 2 0 0 1 2-2h5l3 3h6a2 2 0 0 1 2 2v11H3V7Z"/>',
    file: '<path d="M14 2H5v20h14V7l-5-5Zm0 0v6h5M8 12h8m-8 4h6"/>',
    check: '<path d="m5 12 4 4L19 6"/>', shield: '<path d="m12 2 8 4v6c0 5-8 10-8 10S4 17 4 12V6l8-4Z"/><path d="m8 12 3 3 5-6"/>',
    copy: '<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M15 4H4v12"/>',
    refresh: '<path d="M20 10a8 8 0 1 0-1 7M20 4v6h-6"/>', stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1 1m12 12 1 1M5 19l1-1M18 6l1-1"/>',
    link: '<path d="m9 15 6-6M8 16l-2 2a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0m2 1 2-2a4 4 0 0 1 6 6l-5 5a4 4 0 0 1-6 0" transform="translate(1 0) scale(.95)"/>',
    bell: '<path d="M5 16h14l-2-3V8a5 5 0 0 0-10 0v5l-2 3Zm5 4h4"/>',
};
const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name] || paths.file}</svg>`;
const button = (label, action, cls = '', data = '') => `<button class="${cls}" data-action="${action}" ${data}>${label}</button>`;
const ib = (name, label, action, data = '', cls = '') => button(icon(name), action, `icon ${cls}`, `aria-label="${label}" title="${label}" ${data}`);
const row = (name, detail, action, symbol = 'file', extra = '', meta = '') => `<button class="list-row" data-action="${action}" ${extra}><span class="avatar">${icon(symbol)}</span><span class="grow"><strong>${esc(name)}</strong><small>${esc(detail)}</small></span>${meta ? `<span class="row-meta">${meta}</span>` : icon('chevron')}</button>`;
const field = (label, name, value = '', type = 'text', required = false) => `<label class="field">${label}<input name="${name}" type="${type}" value="${esc(value)}" ${required ? 'required' : ''}></label>`;
const area = (label, name, value = '') => `<label class="field">${label}<textarea name="${name}">${esc(value)}</textarea></label>`;
const choose = (label, name, values, value) => `<label class="field">${label}<select name="${name}" aria-label="${label}">${values.map(v => `<option ${v === value ? 'selected' : ''}>${esc(v)}</option>`).join('')}</select></label>`;
const empty = (title, text, symbol = 'search') => `<div class="empty">${icon(symbol)}<h2>${title}</h2><p>${text}</p></div>`;
const kv = (name, value) => `<div class="kv"><span>${name}</span><span>${esc(value)}</span></div>`;
const navItems = [['conversations','对话','chat'],['experts','专家','person'],['workspace','工作区','folder'],['models','模型','model'],['skills','技能与连接','skills']];
const mobileItems = [['conversations','对话','chat'],['workspace','工作区','folder'],['settings','设置','settings']];
let sequence = 10;
const S = {
    route: {page: 'conversations'}, back: [], theme: 'system', dataTab: 'sources', modelTab: 'cloud',
    filter: '', recipient: '', draft: '', draftRecipient: '', draftAttachment: '', draftModel: '', draftSettings: {}, attachment: '', defaultModel: 'Qwen · 云端', scenario: 'normal', run: null,
    chats: [
        {id:'weekend', title:'把周末留给山野', preview:'一条轻松的路线，和一个不用赶路的下午。', time:'14:32', model:'Qwen · 云端', draft:'', messages:[
            {role:'user', text:'想找个近一点的地方走走。不赶路，周日下午回家。'},
            {role:'assistant', author:'Nola', html:'<p>那就把行程留白一点。选一段轻松的山路，午后坐下来吃饭，天黑前回家。</p><h2>一天就够，不必排满。</h2><p>我先整理了一份可以随时调整的安排。</p>', card:'plan', process:'已整理你在这段对话中的偏好：短途、轻松步行、周日下午返回。没有读取其他会话或私人数据。'}
        ]},
        {id:'release', title:'下一版，先做好三件事', preview:'周也：已把反馈整理成三个优先级。', time:'昨天', model:'Qwen · 云端', draft:'', messages:[{role:'user',text:'帮我把发布前的工作整理成三个重点。'}, {role:'assistant',author:'周也',text:'先解决影响完成任务的问题，再统一交互，最后打磨视觉细节。\n\n1. 会话能可靠发送、停止和恢复。\n2. 模型、专家和资料有明确的归属。\n3. 删除重复状态和低频常驻按钮。'}]},
        {id:'reading', title:'读书时记下的几句话', preview:'有用的笔记，不一定很长。', time:'周四', model:'本地模型 · 8B', draft:'', messages:[{role:'user',text:'怎样让读书笔记更容易回看？'},{role:'assistant',author:'Nola',text:'每条笔记只保留一个观点，再用自己的话写一个例子。标题写成问题，下次需要的时候更容易找到。'}]},
    ],
    experts: [{id:'zhou',name:'周也',description:'把复杂问题梳理清楚',instructions:'先明确目标，再给出简洁、可执行的建议。'}, {id:'lin',name:'林序',description:'研究、核实与整理资料',instructions:'区分事实和推断，附上可核实的来源。'}, {id:'yan',name:'言一',description:'文字润色与表达',instructions:'保留原意，使用自然、简练的语言。'}],
    sources: [{id:'notes',name:'项目笔记',detail:'文件夹 · 12 篇文档',status:'可用',scope:'仅此工作区',icon:'folder'}, {id:'web',name:'产品参考资料',detail:'网页 · 4 个来源',status:'可用',scope:'仅此工作区',icon:'link'}, {id:'calendar',name:'日历',detail:'设备来源',status:'未连接',scope:'请求时确认',icon:'clock'}],
    memories: [{id:'m1',name:'回答先给结论，再展开必要细节。',scope:'全局偏好'}, {id:'m2',name:'当前项目优先修复会话可靠性。',scope:'当前工作区'}],
    tasks: [{id:'t1',sourceId:'notes',name:'每周工作回顾',detail:'整理本周的项目笔记，列出下周重点。',when:'每周五',time:'17:00',enabled:true}, {id:'t2',sourceId:'web',name:'阅读清单',detail:'整理本周保存的阅读资料。',when:'每周日',time:'10:00',enabled:false}],
    models: [{id:'cloud',name:'Qwen · 云端',detail:'OpenAI 兼容连接',kind:'cloud',endpoint:'https://example.invalid/v1'}, {id:'local',name:'本地模型 · 8B',detail:'已下载 · 示例 4.8 GB',kind:'local',endpoint:''}],
    skills: [{id:'writing',name:'清晰写作',detail:'Skill · 工作区',enabled:true,description:'帮助组织文章结构、检查表达。'}, {id:'files',name:'工作区文件',detail:'MCP · 本机',enabled:false,description:'通过已有连接访问工作区文件。启用能力不等于批准文件操作。'}],
    notifications: true, planSaved: false, planLate: false, connectionVerified: false, logs: [],
};
const currentChat = () => S.chats.find(c => c.id === S.route.id);
const pageNames = {conversations:'对话',experts:'专家',workspace:'工作区',models:'模型',skills:'技能与连接',settings:'设置',access:'系统访问'};
function rememberView() {
    const c = currentChat();
    if (S.route.page === 'chat' && c) { c.draft = $('#prompt')?.value ?? c.draft; c.recipient = S.recipient; c.attachment = S.attachment; c.scroll = $('#transcript')?.scrollTop ?? 0; }
    if (S.route.page === 'chat' && !c) { S.draft = $('#prompt')?.value ?? S.draft; S.draftRecipient=S.recipient; S.draftAttachment=S.attachment; }
}
function navigate(page, id, push = true) {
    rememberView(); closeSheet(true); $('#toast').hidden=true;
    if (push) S.back.push({...S.route});
    S.route = {page, id}; S.filter = '';
    if (page === 'chat') { const c = currentChat(); S.recipient = c ? c.recipient || '' : S.draftRecipient; S.attachment = c ? c.attachment || '' : S.draftAttachment; }
    render();
    $('#main').focus({preventScroll:true});
}
function back() {
    const route = S.back.pop() || {page:'conversations'};
    navigate(route.page, route.id, false);
}
function navButton(item, mobile = false) {
    const [page, label, name] = item;
    const active = S.route.page === page || (mobile && page === 'conversations' && S.route.page === 'chat') || (mobile && page === 'settings' && ['experts','models','skills','access'].includes(S.route.page));
    return button(`${icon(name)}<span>${label}</span>`, 'nav', '', `data-page="${page}" ${active ? 'aria-current="page"' : ''}`);
}
function renderNavigation() {
    $('#sidebar').innerHTML = `<div class="brand">${icon('logo')}<span>Nola</span></div>${button(`${icon('compose')}新对话`, 'new', 'new-chat')}<nav class="nav-group">${navItems.map(i => navButton(i)).join('')}</nav><div class="nav-caption">最近对话</div><div class="recents">${S.chats.map(c => button(`<span>${esc(c.title)}</span>`, 'chat', `recent ${S.route.id === c.id ? 'selected' : ''}`, `data-id="${c.id}"`)).join('')}</div><div class="side-footer">${navButton(['settings','设置','settings'])}<span class="demo-label">交互原型 · 虚构示例数据</span></div>`;
    $('#mobile-nav').innerHTML = mobileItems.map(i => navButton(i, true)).join('');
    document.body.classList.toggle('detail-view', S.route.page === 'chat' || !!S.route.id || S.route.page === 'access');
}
function header(title, actions = '', detail = false) {
    return `<header class="page-header ${detail ? '' : 'root-header'}"><div class="header-title">${detail ? ib('back','返回','back') : ''}<h1>${esc(title)}</h1></div><div class="header-actions">${actions}</div></header>`;
}
function page(title, content, actions = '', detail = false) {
    return `${header(title, actions, detail)}<div class="page-scroll"><div class="page-content">${content}</div></div>`;
}
function search(label) { return `<label class="search">${icon('search')}<input id="search" aria-label="${label}" placeholder="${label}" value="${esc(S.filter)}" autocomplete="off"></label>`; }
function tabs(values, selected, key) { return `<div class="tabs" aria-label="筛选">${values.map(([value,label]) => button(label,'tab','',`data-key="${key}" data-value="${value}" aria-pressed="${selected === value}"`)).join('')}</div>`; }
function render() {
    renderNavigation();
    const views = {conversations:conversationList, chat:chatPage, workspace:workspacePage, experts:expertsPage, models:modelsPage, skills:skillsPage, settings:settingsPage, access:accessPage};
    $('#main').innerHTML = views[S.route.page]();
    if (S.route.page === 'chat') {
        const c = currentChat(); const input = $('#prompt'); input.value = c ? c.draft : S.draft;
        resizeInput(); updateSend();
        const transcript = $('#transcript'); transcript.scrollTop = c?.scroll ?? 0;
        transcript.addEventListener('scroll', updateLatest, {passive:true});
        const stopFollowing=()=>{if(S.run && S.run.chatId===c?.id)S.run.follow=false;};
        transcript.addEventListener('wheel',e=>{if(e.deltaY<0)stopFollowing();},{passive:true});
        let touchY=0;
        transcript.addEventListener('touchstart',e=>{touchY=e.touches[0].clientY;},{passive:true});
        transcript.addEventListener('touchmove',e=>{if(e.touches[0].clientY>touchY+4)stopFollowing();},{passive:true});
        transcript.addEventListener('keydown',e=>{if(['ArrowUp','PageUp','Home'].includes(e.key))stopFollowing();});
        if (S.run?.chatId === c?.id) updateRun();
        updateLatest();
    }
}
function conversationList() {
    const chats = S.chats.filter(c => `${c.title} ${c.preview}`.toLowerCase().includes(S.filter.toLowerCase()));
    return page('对话', `${search('搜索对话')}<p class="section-label">最近</p><div id="results" class="chat-list">${chats.length ? chats.map(c => row(c.title,c.preview,'chat','chat',`data-id="${c.id}"`,`${c.time}${S.run?.chatId === c.id ? '<br>回复中' : ''}`)).join('') : empty('没有找到这段对话','换个关键词试试。')}</div>`, ib('compose','新对话','new','','mobile-only'));
}
function planCard() {
    return `<section class="inline-card" aria-label="周末安排"><div class="card-heading"><h3>周日 · 山间慢行</h3><small>可调整的安排</small></div><div class="plan-item"><time>${S.planLate ? '10:30' : '09:30'}</time><div><strong>出发，不赶早</strong><p>留一点时间吃早餐，再去山脚。</p></div></div><div class="plan-item"><time>11:00</time><div><strong>沿林间小路走走</strong><p>轻松步行，累了就停下来。</p></div></div><div class="plan-item"><time>14:00</time><div><strong>午饭后慢慢回家</strong><p>把剩下的半天还给自己。</p></div></div><div class="card-actions">${button(S.planSaved ? `${icon('check')}已保存到本段对话` : '保存安排','save-plan',S.planSaved ? 'outline' : 'primary')}${button('调整时间','edit-plan')}</div></section>`;
}
function messageHTML(m, i) {
    if (m.role === 'user') return `<article class="message user">${esc(m.text).replace(/\n/g,'<br>')}${m.attachment ? `<small><br>${esc(m.attachment)}</small>` : ''}</article>`;
    return `<article class="message assistant"><div class="message-author">${icon(m.author === 'Nola' ? 'logo' : 'person')}<span>${esc(m.author || 'Nola')}</span></div>${m.process ? `<details class="process"><summary>查看处理过程</summary><div class="process-body">${esc(m.process)}</div></details>` : ''}${m.html || `<p>${esc(m.text).replace(/\n/g,'<br>')}</p>`}${m.card ? planCard() : ''}<div class="message-actions">${ib('copy','复制回复','copy',`data-index="${i}"`)}${ib('more','回复操作','message-menu',`data-index="${i}"`)}</div></article>`;
}
function chatPage() {
    const c = currentChat(); const model = c?.model || S.draftModel || S.defaultModel; const messages = c?.messages || [];
    const modelButton = button(`<span>${esc(model)}</span>${icon('down')}`,'model-picker','model-trigger');
    return `<header class="chat-header"><div class="header-title">${ib('back','返回对话列表','chat-back','','mobile-only')}<div class="chat-heading desktop-only"><strong>${esc(c?.title || '新对话')}</strong></div><button class="mobile-heading mobile-only" data-action="model-picker" aria-label="选择模型"><strong>${esc(c?.title || '新对话')}</strong><small>${esc(model)} ${icon('down')}</small></button></div><div class="header-actions"><div class="desktop-only">${modelButton}</div>${ib('more','对话更多操作','chat-menu')}</div></header><div class="transcript" id="transcript" tabindex="0" aria-label="对话消息"><div class="messages">${messages.length ? `<div class="date">${c.id === 'weekend' ? '今天 · 14:32' : '本段对话'}</div>${messages.map(messageHTML).join('')}` : empty('从一个想法开始','输入想法。用 / 选择指令，用 @ 邀请专家。','logo')}<div id="run-slot"></div></div></div><div class="input-dock">${ib('latest','回到最新消息','latest','hidden','latest')}<div class="composer-wrap"><div id="suggestions" class="mentions" role="listbox" aria-label="输入建议" hidden></div><div id="recipient" class="recipient" ${!S.recipient && !S.attachment ? 'hidden' : ''}>${recipientHTML()}</div><form id="composer" class="composer">${ib('plus','添加附件或专家','attach-menu','type="button"')}<textarea id="prompt" aria-autocomplete="list" aria-controls="suggestions" aria-expanded="false" rows="1" aria-label="消息" placeholder="说点什么，或输入 /" autocomplete="off"></textarea>${ib('arrow','发送消息','send','type="submit"','send')}</form><p id="command-hint" class="command-hint" hidden></p><p id="command-error" class="command-error" role="alert" hidden></p></div></div>`;
}
function recipientHTML() { return `${S.recipient ? button(`${esc(S.experts.find(e => e.id === S.recipient)?.name || '专家')}${icon('close')}`,'clear-recipient') : ''}${S.attachment ? button(`${icon('file')}${esc(S.attachment)}${icon('close')}`,'clear-attachment') : ''}`; }
function updateRecipient() { $('#recipient').innerHTML = recipientHTML(); $('#recipient').hidden = !S.recipient && !S.attachment; }
function resizeInput() { const input = $('#prompt'); if (!input) return; input.style.height = '44px'; input.style.height = `${Math.min(142,input.scrollHeight)}px`; }
function updateSend() {
    const b = $('#composer .send'); if (!b) return;
    const isCommand=$('#prompt').value.trim().startsWith('/');
    const running = !isCommand && S.run?.chatId === currentChat()?.id && S.run?.phase === 'stream';
    b.innerHTML = icon(running ? 'stop' : 'arrow'); b.setAttribute('aria-label', isCommand ? '执行指令' : running ? '停止回复' : '发送消息');
    const hint=$('#command-hint'), name=$('#prompt').value.trim().split(/\s/)[0].slice(1);
    const command=isCommand&&commands.find(c=>c.name===name);
    hint.hidden=!command||!/^\/\w+\s/.test($('#prompt').value);
    hint.textContent=command?`${command.title}${command.argument?' · 可填写'+command.argument:''} · 执行后直接打开操作，不发送给模型`:'';
    if(!hint.hidden)$('#prompt').setAttribute('aria-describedby','command-hint');
    b.disabled = !running && (!$('#prompt').value.trim() || (!isCommand && S.run?.chatId === currentChat()?.id && S.run?.phase === 'approval'));
}
function updateLatest() {
    const t = $('#transcript'), b = $('.latest'); if (!t || !b) return;
    const distance=t.scrollHeight-t.clientHeight-t.scrollTop;
    b.hidden = distance <= 180;
    if(distance<=24&&S.run && S.run.chatId===currentChat()?.id)S.run.follow=true;
}
function toLatest() { if(S.run && S.run.chatId===currentChat()?.id)S.run.follow=true; const t = $('#transcript'); if (t) t.scrollTo({top:t.scrollHeight,behavior:'smooth'}); }
function matches(item) { return `${item.name} ${item.detail || item.description || ''}`.toLowerCase().includes(S.filter.toLowerCase()); }
function workspacePage() {
    if (S.route.id && S.tasks.some(t=>t.id===S.route.id)) return taskDetail();
    if (S.route.id) {
        const source = S.sources.find(s => s.id === S.route.id);
        if (source) return page(source.name, `<div class="detail-lead"><span class="avatar">${icon(source.icon)}</span><h1>${esc(source.name)}</h1><p>${esc(source.detail)}</p></div>${kv('数据归属',source.scope)}${kv('连接状态',source.status)}${kv('默认读取','每次任务按实际权限检查')}<p class="inline-notice">连接资料不会自动把内容发送给模型。在对话资源中查看本次实际使用的资料。</p><div class="card-actions">${button(source.status === '未连接' ? '连接来源' : '验证连接','verify-source','primary',`data-id="${source.id}"`)}${button('访问设置','source-access','',`data-id="${source.id}"`)}</div>${S.tasks.some(t=>t.sourceId===source.id)?`<div class="section-head"><h3>使用这份资料的计划</h3></div>${S.tasks.filter(t=>t.sourceId===source.id).map(t=>row(t.name,`${t.when} ${t.time}`,'task','clock',`data-id="${t.id}"`)).join('')}`:''}`,'',true);
    }
    let content = '';
    if (S.dataTab === 'sources') content = `<p class="workspace-intro">${S.sources.length} 个来源 · ${S.sources.filter(s=>s.status==='可用').length} 个可用</p><div class="list">${S.sources.map(s => row(s.name,s.detail,'source',s.icon,`data-id="${s.id}"`,s.status)).join('')}</div>`;
    if (S.dataTab === 'memory') content = `<p class="muted">保留有用的信息，随时查看、修订或移除。</p><div class="list">${S.memories.map(m => row(m.name,m.scope,'memory','file',`data-id="${m.id}"`)).join('')}</div>`;
    if (S.dataTab === 'usage') content = S.logs.length ? `<p class="workspace-intro">计划运行与资料使用，共用一份可追溯记录。</p><div class="list">${S.logs.map(l=>row(S.tasks.find(t=>t.id===l.task)?.name||'计划',l.text,'task','clock',`data-id="${l.task}"`,'刚刚 · 演示')).join('')}</div>` : empty('每次使用，都有来处','计划运行和资料使用会记录在这里。原型尚未试运行，也没有读取你的资料。','shield');
    if (S.dataTab === 'plans') content = `<p class="workspace-intro">让资料成为行动。选择何时运行，以及每个计划能使用的资料。</p><div class="list">${S.tasks.map(t=>row(t.name,`${t.when} ${t.time} · ${t.enabled?'已启用':'已暂停'}`,'task','clock',`data-id="${t.id}"`)).join('')}</div>`;
    const add = S.dataTab==='plans'?ib('plus','新建计划','task-new'):S.dataTab==='memory'?ib('plus','添加记忆','memory-new'):S.dataTab==='sources'?ib('plus','添加资料来源','source-new'):'';
    return page('工作区', `<div class="workspace-scope">${icon('folder')}个人工作区</div>${tabs([['sources','资料'],['memory','记忆'],['plans','计划'],['usage','记录']],S.dataTab,'dataTab')}${content}`, add);
}
function expertsPage() {
    const e = S.experts.find(e=>e.id===S.route.id);
    if (e) return page(e.name, `<div class="detail-lead"><span class="avatar">${icon('person')}</span><h1>${esc(e.name)}</h1><p>${esc(e.description)}</p></div><h3>工作指令</h3><p style="margin-top:12px">${esc(e.instructions)}</p>${kv('默认模型','跟随应用默认')}${kv('历史与记忆','由每段对话独立管理')}<p class="inline-notice">专家是可重复使用的配置。在对话中输入 @ 邀请加入，不会在这里另开一段会话。</p>${button('编辑专家','expert-edit','outline',`data-id="${e.id}"`)}`,'',true);
    return page('专家',`<div class="page-intro"><h1>各有所长，一起思考。</h1><p>保存适合你的工作方式，在需要时邀请加入对话。</p></div>${search('搜索专家')}<div id="results" class="list">${S.experts.filter(matches).map(e=>row(e.name,e.description,'expert','person',`data-id="${e.id}"`)).join('') || empty('没有找到专家','试试姓名或职责。')}</div>`,ib('plus','新建专家','expert-new'));
}
function taskDetail() {
    const t = S.tasks.find(t=>t.id===S.route.id);
    const source = S.sources.find(s=>s.id===t.sourceId);
    if (t) return page(t.name, `<div class="detail-lead"><span class="avatar">${icon('clock')}</span><h1>${esc(t.name)}</h1><p>${esc(t.detail)}</p></div>${kv('运行时间',`${t.when} ${t.time}`)}${kv('状态',t.enabled?'已启用':'已暂停')}${row(source.name,'本计划使用的资料 · 个人工作区','source',source.icon,`data-id="${source.id}"`)}<div class="card-actions">${button('编辑计划','task-edit','outline',`data-id="${t.id}"`)}${button(t.enabled?'暂停':'启用','task-toggle','',`data-id="${t.id}"`)}${button('试运行','task-run','',`data-id="${t.id}"`)}</div><div class="section-head"><h3>最近运行</h3></div>${S.logs.filter(l=>l.task===t.id).map(l=>`<div class="kv"><span>刚刚 · 演示</span><span>${l.text}</span></div>`).join('') || '<p class="muted">暂无运行记录。</p>'}`,'',true);

}
function modelsPage() {
    const m = S.models.find(m=>m.id===S.route.id);
    if (m) return page(m.name, `<div class="detail-lead"><span class="avatar">${icon('model')}</span><h1>${esc(m.name)}</h1><p>${esc(m.detail)}</p></div>${kv('用途',m.kind==='cloud'?'远程推理':'本机推理')}${m.kind==='cloud'?kv('服务地址',m.endpoint):kv('运行状态',m.running?'运行中（演示）':'未启动')}${kv('默认聊天模型',S.defaultModel===m.name?'当前默认':'未设为默认')}<div class="card-actions">${button(S.defaultModel===m.name?'当前默认':'设为默认','model-default',S.defaultModel===m.name?'outline':'primary',`data-id="${m.id}"`)}${button(m.kind==='cloud'?'编辑连接':m.running?'停止本地运行':'启动本地运行',m.kind==='cloud'?'model-edit':'model-run','',`data-id="${m.id}"`)}</div><details class="process"><summary>能力与高级参数</summary><div class="process-body">工具调用：跟随 Provider 声明<br>推理强度：跟随模型默认<br>此处的型号与运行状态均为演示，不代表实际能力验证。</div></details>`,'',true);
    return page('模型',`<div class="page-intro"><h1>选一个合适的模型。</h1><p>当前默认：${esc(S.defaultModel)}。切换默认值不会改动已有会话。</p></div>${tabs([['cloud','云端连接'],['local','本地模型']],S.modelTab,'modelTab')}<div class="list">${S.models.filter(m=>m.kind===S.modelTab).map(m=>row(m.name,m.detail,'model','model',`data-id="${m.id}"`,S.defaultModel===m.name?'默认':'')).join('')}</div>`,ib('plus','添加模型连接','model-new'));
}
function skillsPage() {
    const s = S.skills.find(s=>s.id===S.route.id);
    if (s) return page(s.name,`<div class="detail-lead"><span class="avatar">${icon('skills')}</span><h1>${s.name}</h1><p>${s.description}</p></div>${kv('来源与范围',s.detail)}<label class="switch-row"><span>启用此能力<small>具体操作仍遵守当前会话的权限</small></span><input type="checkbox" data-toggle="skill" data-id="${s.id}" ${s.enabled?'checked':''}></label>${button('查看配置','skill-config','outline',`data-id="${s.id}"`)}`,'',true);
    return page('技能与连接',`<div class="page-intro"><h1>在需要的时候，多一种能力。</h1><p>技能定义工作方法，连接提供工具。启用不等于授予操作权限。</p></div>${search('搜索技能或连接')}<div id="results" class="list">${S.skills.filter(matches).map(s=>row(s.name,s.detail,'skill','skills',`data-id="${s.id}"`,s.enabled?'已启用':'未启用')).join('') || empty('没有匹配的能力','换个关键词试试。')}</div>`,ib('plus','添加技能或连接','skill-new'));
}
function settingsPage() {
    return page('设置', `<div class="settings-group"><h2>工作方式</h2>${row('专家','保存可重复使用的工作方式','nav','person','data-page="experts"')}${row('模型',S.defaultModel,'nav','model','data-page="models"')}${row('技能与连接','管理 Skills 和 MCP','nav','skills','data-page="skills"')}</div><div class="settings-group"><h2>偏好与访问</h2>${row('外观',({system:'跟随系统',light:'浅色',dark:'深色'})[S.theme],'appearance','sun')}${row('系统访问','文件、日历与设备能力','nav','shield','data-page="access"')}<label class="switch-row"><span>任务通知<small>只在完成、失败或需要确认时提醒</small></span><input type="checkbox" aria-label="任务通知" data-toggle="notifications" ${S.notifications?'checked':''}></label></div><div class="settings-group"><h2>设计预览</h2>${row('交互场景','流式、长回复、审批与恢复','scenarios','chat')}${row('关于原型','虚构示例 · 不连接模型或设备','about','logo')}</div>`);
}
function accessPage() {
    return page('系统访问',`<div class="page-intro"><h1>清楚知道，什么可以被访问。</h1><p>在需要的时刻请求授权。这里展示能力和访问范围，不用开关假装授予系统权限。</p></div>${['文件与文件夹','日历','浏览器','桌面操作'].map((name,i)=>row(name,'尚未检查系统权限','access-detail',['folder','clock','link','settings'][i],`data-name="${name}"`)).join('')}`,'',true);
}
let focusBeforeSheet = null, saveEditor = null, initialForm = '', toastTimer;
function openSheet(title, content, footer = '') {
    const d = $('#sheet'); if (!d.open) focusBeforeSheet = document.activeElement;
    saveEditor = null;
    d.innerHTML = `<div class="sheet-head"><h2 id="sheet-title">${title}</h2>${ib('close','关闭','close-sheet')}</div><div class="sheet-body">${content}</div>${footer?`<div class="sheet-footer">${footer}</div>`:''}`;
    if (!d.open) d.showModal();
    const target = $('input, textarea, button', $('.sheet-body')) || $('.sheet-head button'); target?.focus({preventScroll:true});
}
function closeSheet(force = false) {
    const d = $('#sheet');
    if (!force && saveEditor && $('#editor') && JSON.stringify(Object.fromEntries(new FormData($('#editor')))) !== initialForm) {
        if (!$('#discard')) $('.sheet-body').insertAdjacentHTML('beforeend',`<div id="discard" class="inline-notice" role="alert">更改尚未保存。${button('继续编辑','keep-editing')}${button('丢弃更改','discard','danger')}</div>`);
        return;
    }
    saveEditor = null; if (d.open) d.close(); focusBeforeSheet?.isConnected && focusBeforeSheet.focus({preventScroll:true});
}
function editor(title, content, onSave) {
    openSheet(title,`<form id="editor" class="form">${content}</form><p id="form-error" class="form-error" role="alert"></p>`,`${button('取消','close-sheet')}${button('保存','save-editor','primary')}`);
    saveEditor = onSave; initialForm = JSON.stringify(Object.fromEntries(new FormData($('#editor'))));
}
function toast(text) { clearTimeout(toastTimer); $('#toast').textContent=text; $('#toast').hidden=false; toastTimer=setTimeout(()=>$('#toast').hidden=true,3200); }
function menuRow(title, action, name, detail = '', data = '') { return row(title,detail,action,name,data); }
function expertEditor(id) {
    const e=S.experts.find(e=>e.id===id);
    editor(e?'编辑专家':'新建专家',field('名称','name',e?.name,'text',true)+field('职责简介','description',e?.description,'text',true)+area('工作指令','instructions',e?.instructions),v=>{
        if(e) Object.assign(e,v); else S.experts.push({id:`e${++sequence}`,...v});
        closeSheet(true); render(); toast('专家已保存；可在对话中输入 @ 选择。');
    });
}
function taskEditor(id) {
    const t=S.tasks.find(t=>t.id===id);
    editor(t?'编辑计划':'新建计划',field('名称','name',t?.name,'text',true)+area('要完成什么','detail',t?.detail)+choose('频率','when',['每天','每周五','每周日'],t?.when||'每周五')+field('时间','time',t?.time||'09:00','time',true)+`<label class="field">使用的资料<select name="sourceId" aria-label="使用的资料">${S.sources.map(source=>`<option value="${source.id}" ${source.id===(t?.sourceId||'notes')?'selected':''}>${esc(source.name)} · ${esc(source.status)}</option>`).join('')}</select></label><p class="sheet-description">仅使用所选资料，不扩展到整个工作区。新计划保存后默认暂停；连接状态不等于操作授权。</p>`,v=>{
        if(t) Object.assign(t,v); else S.tasks.push({id:`t${++sequence}`,enabled:false,...v});
        closeSheet(true); render(); toast('计划已保存到演示；未创建真实定时任务。');
    });
}
function modelEditor(id) {
    const m=S.models.find(m=>m.id===id);
    editor(m?'编辑连接':'添加云端连接',field('显示名称','name',m?.name,'text',true)+field('服务地址','endpoint',m?.endpoint||'https://example.invalid/v1','url',true)+'<p class="sheet-description">原型不接收 API Key。生产应用应使用安全凭据输入与系统存储。</p>',v=>{
        if(!/^https?:\/\//.test(v.endpoint)) throw Error('请输入 http 或 https 服务地址。');
        if(m) Object.assign(m,v); else S.models.push({id:`model${++sequence}`,kind:'cloud',detail:'OpenAI 兼容连接',...v});
        closeSheet(true); render(); toast('演示连接已保存；没有发送网络请求。');
    });
}
function memoryEditor(id) {
    const m=S.memories.find(m=>m.id===id);
    editor(m?'修订记忆':'添加记忆',area('记忆内容','name',m?.name)+choose('归属范围','scope',['当前工作区','全局偏好'],m?.scope||'当前工作区'),v=>{
        if(!v.name.trim()) throw Error('请填写需要保留的内容。');
        if(m) Object.assign(m,v); else S.memories.push({id:`m${++sequence}`,...v});
        closeSheet(true);render();toast('已保存演示记忆。');
    });
}
// Explicit UI commands, never natural-language tool routing or model messages.
const commands = [
    {name:'model',title:'选择模型',description:'为当前对话选择模型',action:'model-picker',icon:'model'},
    {name:'experts',title:'选择专家',description:'选择下一条消息的收件人',action:'participants',icon:'person'},
    {name:'resources',title:'对话资料',description:'查看本段对话使用的资料',action:'resources',icon:'folder'},
    {name:'permissions',title:'权限设置',description:'查看并调整本段对话设置',action:'conversation-settings',icon:'shield'},
    {name:'plan',title:'新建计划',description:'填写任务说明，再确认时间与范围',argument:'任务说明',icon:'clock'},
    {name:'search',title:'搜索对话',description:'在会话列表中查找',argument:'关键词',icon:'search'},
    {name:'stop',title:'停止回复',description:'只停止当前对话的运行',icon:'stop'},
    {name:'help',title:'指令帮助',description:'查看全部可用指令',icon:'chat'},
];
function hideSuggestions() {
    $('#suggestions').hidden=true;
    $('#prompt').setAttribute('aria-expanded','false');
    $('#prompt').removeAttribute('aria-activedescendant');
}
function showSuggestions(label, html) {
    const panel=$('#suggestions');panel.setAttribute('aria-label',label);panel.innerHTML=html;panel.hidden=false;
    $('#prompt').setAttribute('aria-expanded','true');
    const first=$('[role="option"]',panel);if(first)$('#prompt').setAttribute('aria-activedescendant',first.id);
}
function renderSuggestions() {
    const input=$('#prompt'), before=input.value.slice(0,input.selectionStart);
    if(input.value.startsWith('/')) {
        const query=input.value.slice(1);
        if(/\s/.test(query)){hideSuggestions();return;}
        const list=commands.filter(c=>c.name.startsWith(query.toLowerCase())||c.title.includes(query));
        showSuggestions('指令建议', `<div class="suggestion-caption">指令 <span>选择后填写，执行时不会发送给模型</span></div>${list.length?list.map((c,i)=>`<button id="command-option-${i}" class="list-row command-option" role="option" aria-selected="${i===0}" data-action="command-select" data-name="${c.name}"><span class="avatar">${icon(c.icon)}</span><span class="grow"><strong><code>/${c.name}</code><span>${c.title}</span></strong><small>${c.argument?'参数：'+c.argument+' · ':''}${c.description}</small></span></button>`).join(''):'<p class="no-suggestions">没有匹配的指令。输入 / 查看列表，或删除 / 作为普通消息输入。</p>'}`);
        return;
    }
    const match=before.match(/(?:^|\s)@([^\s@]*)$/);
    if(!match){hideSuggestions();return;}
    const q=match[1].toLowerCase(), list=S.experts.filter(e=>e.name.toLowerCase().includes(q)||e.description.toLowerCase().includes(q));
    showSuggestions('选择专家',list.length?list.map((e,i)=>`<button id="expert-option-${i}" class="list-row" role="option" aria-selected="${i===0}" data-action="mention" data-id="${e.id}"><span class="avatar">${icon('person')}</span><span class="grow"><strong>${esc(e.name)}</strong><small>${esc(e.description)}</small></span></button>`).join(''):'<p class="no-suggestions">没有匹配的专家。</p>');
}
function selectCommand(name) {
    closeSheet(true);
    const input=$('#prompt');input.value=`/${name} `;hideSuggestions();resizeInput();updateSend();rememberView();input.focus();
    input.setSelectionRange(input.value.length,input.value.length);
}
function commandError(message) {
    $('#command-error').textContent=message;$('#command-error').hidden=false;
    $('#prompt').setAttribute('aria-describedby','command-error');
}
function executeCommand(text) {
    const match=text.match(/^\/([a-z]+)(?:\s+([\s\S]*))?$/i);
    const c=match&&commands.find(c=>c.name===match[1].toLowerCase());
    if(!c){commandError('未知指令。输入 / 查看可用指令；这条内容没有发送给模型。');return;}
    const argument=match[2]?.trim()||'';
    if(argument&&!c.argument){commandError(`/${c.name} 不接收参数，请移除后再执行。`);return;}
    if(c.name==='stop'&&(!S.run||S.run.chatId!==currentChat()?.id||S.run.phase!=='stream')){commandError('当前对话没有正在生成的回复。');return;}
    $('#prompt').value='';hideSuggestions();resizeInput();updateSend();rememberView();
    if(c.action){actions[c.action]();return;}
    if(c.name==='plan'){
        taskEditor();$('#editor [name="detail"]').value=argument;
        initialForm=JSON.stringify(Object.fromEntries(new FormData($('#editor'))));return;
    }
    if(c.name==='search'){
        navigate('conversations');S.filter=argument;render();$('#search').focus();return;
    }
    if(c.name==='stop'){stopRun();return;}
    openSheet('指令帮助','<p class="sheet-description">选择只补全输入，不会立即执行。可用按钮或 Enter 执行完整指令；不会进入模型消息或任务队列。</p>'+commands.filter(x=>x.name!=='help').map(x=>row('/'+x.name,x.description,'command-select',x.icon,`data-name="${x.name}"`)).join(''));
}
function selectMention(id) {
    S.recipient=id; const input=$('#prompt');
    input.value=input.value.slice(0,input.selectionStart).replace(/@[^\s@]*$/,'')+input.value.slice(input.selectionStart);
    hideSuggestions(); updateRecipient();resizeInput();updateSend();input.focus();rememberView();
}
function submit() {
    const commandText=$('#prompt').value.trim();
    if(commandText.startsWith('/')){executeCommand(commandText);return;}
    if(S.run?.chatId===currentChat()?.id && S.run?.phase==='stream'){stopRun();return;}
    const text=$('#prompt').value.trim();if(!text)return;
    if(S.run && S.run.phase!=='done'){toast('另一个演示任务尚未结束；请先停止或完成确认。');return;}
    let c=currentChat();
    if(!c){c={id:`c${++sequence}`,title:text.slice(0,24),preview:text,time:'刚刚',model:S.draftModel||S.defaultModel,settings:{...S.draftSettings},draft:'',messages:[]};S.chats.unshift(c);S.route.id=c.id;S.draft='';S.draftModel='';S.draftRecipient='';S.draftAttachment='';S.draftSettings={};}
    c.messages.push({role:'user',text,attachment:S.attachment});c.draft='';c.recipient=S.recipient;c.attachment='';S.attachment='';
    S.run={chatId:c.id,author:S.experts.find(e=>e.id===S.recipient)?.name||'Nola',phase:S.scenario==='approval'?'approval':S.scenario==='error'?'error':'stream',index:0,text:'',reason:'正在整理你的问题…',follow:true};
    render();toLatest();if(S.run.phase==='stream')startStream();
}
let streamTimer;
const longAnswer = '先从最重要的一件事开始。\n\n'+Array.from({length:24},(_,i)=>`${i+1}. 给第 ${i+1} 个工作环节留出足够空间：明确目标，记录依据，只在需要做决定时展示操作。完成后把结果放回原来的对话，方便继续讨论。`).join('\n\n')+'\n\n长回复演示结束。你可以向上阅读，再点击箭头回到最新位置。';
function startStream() {
    const r=S.run;if(!r)return;clearInterval(streamTimer);
    const output=S.scenario==='long'?longAnswer:'可以。先把目标收敛成一件具体的事，再保留完成它所需的信息。\n\n让内容占据主要空间，低频设置按需打开；正在进行的任务保留清晰反馈，遇到问题时给出下一步。\n\n这是原型的预设回复，用于检查流式显示与交互，不是模型生成结果。';
    streamTimer=setInterval(()=>{
        if(S.run!==r){clearInterval(streamTimer);return;}
        r.index++;r.reason=r.index<4?'正在整理你的问题…':'已明确目标，正在组织回复。';
        if(r.index>=4)r.text=output.slice(0,Math.min(output.length,(r.index-3)*(S.scenario==='long'?45:12)));
        updateRun();
        if(r.text.length===output.length){
            clearInterval(streamTimer); const c=S.chats.find(c=>c.id===r.chatId); c.messages.push({role:'assistant',author:r.author,text:r.text,process:'演示：整理目标 → 组织内容 → 完成回复。这里只展示可公开的处理摘要，不伪造模型隐藏思考。'});c.preview=r.text.slice(0,70);S.run=null;
            if(S.route.page==='chat'&&currentChat()?.id===c.id){const t=$('#transcript');c.scroll=t.scrollTop;const follow=t.scrollHeight-t.clientHeight-t.scrollTop<180;rememberView();render();if(follow)toLatest();}else renderNavigation();
        }
    },180);
}
function updateRun() {
    const r=S.run,slot=$('#run-slot');if(!slot||!r||currentChat()?.id!==r.chatId)return;
    const t=$('#transcript'), follow=r.follow;
    if(r.phase==='approval')slot.innerHTML=`<article class="message assistant"><div class="message-author">${icon('person')}${esc(r.author)}</div><section class="approval"><h3>允许读取这份文件吗？</h3><p>为完成当前请求，需要读取工作区中的预算表。</p>${kv('资源','工作区 / 预算示例.csv')}${kv('操作','读取内容，不写入')}${kv('授权范围','仅本次读取')}<div class="card-actions">${button('允许本次','approve','primary')}${button('不允许','deny')}</div></section></article>`;
    else if(r.phase==='error')slot.innerHTML=`<article class="message assistant"><section class="error-card"><h3>连接中断，回复没有完成</h3><p>你的消息已保留。重试会继续当前请求，不另建会话。</p><div class="card-actions">${button('重试','retry','primary')}${button('结束本次','deny')}</div></section></article>`;
    else {
        if(!$('#live-text',slot))slot.innerHTML=`<article class="message assistant"><div class="message-author">${icon(r.author==='Nola'?'logo':'person')}<span>${esc(r.author)}</span></div><details class="process" open><summary><span class="pulse"></span><span id="live-phase">处理中</span></summary><div class="process-body" id="live-reason"></div></details><p id="live-text" class="stream-text"></p></article>`;
        $('#live-phase').textContent=r.text?'正在回复':'正在整理';$('#live-reason').textContent=r.reason;$('#live-text').textContent=r.text;
    }
    if(follow)t.scrollTop=t.scrollHeight;updateSend();updateLatest();
}
function stopRun() {
    const r=S.run;if(!r)return;clearInterval(streamTimer);
    const c=S.chats.find(c=>c.id===r.chatId);c.messages.push({role:'assistant',author:r.author,text:r.text||'本次回复已停止。',process:'已停止 · 已生成内容保留。'});S.run=null;
    rememberView();render();toast('已停止回复。');
}
const actions = {
    nav: el=>navigate(el.dataset.page), chat: el=>navigate('chat',el.dataset.id), back,
    'chat-back': ()=>navigate('conversations',undefined,false),
    new: ()=>{if(!S.draftModel)S.draftModel=S.defaultModel;navigate('chat','draft');$('#prompt').focus();},
    tab: el=>{S[el.dataset.key]=el.dataset.value;render();},
    source: el=>navigate('workspace',el.dataset.id), expert: el=>navigate('experts',el.dataset.id),
    task: el=>navigate('workspace',el.dataset.id), model: el=>navigate('models',el.dataset.id),skill:el=>navigate('skills',el.dataset.id),
    'close-sheet':()=>closeSheet(),discard:()=>closeSheet(true),'keep-editing':()=>$('#discard').remove(),
    'save-editor':()=>{
        const form=$('#editor');if(!form.reportValidity())return;
        try {const values=Object.fromEntries(new FormData(form));for(const k of Object.keys(values))values[k]=values[k].trim();if(values.name!==undefined&&!values.name)throw Error('名称不能为空。');saveEditor?.(values);}catch(e){$('#form-error').textContent=e.message;}
    },
    'chat-menu':()=>openSheet('这段对话',menuRow('参与专家','participants','person','为下一条消息选择收件人')+menuRow('资料与附件','resources','folder','只属于这段对话')+menuRow('任务进度','progress','clock','查看当前回复的执行状态')+menuRow('对话设置','conversation-settings','settings','模型、权限与记忆')),
    'model-picker':()=>openSheet('选择模型',S.models.map(m=>`<button class="list-row ${m.name===(currentChat()?.model||S.draftModel||S.defaultModel)?'selected':''}" data-action="select-model" data-id="${m.id}"><span class="avatar">${icon('model')}</span><span class="grow"><strong>${esc(m.name)}</strong><small>${esc(m.detail)}</small></span>${m.name===(currentChat()?.model||S.draftModel||S.defaultModel)?icon('check'):''}</button>`).join('')+menuRow('管理模型','open-models','settings')),
    'open-models':()=>navigate('models'),
    'select-model':el=>{const m=S.models.find(m=>m.id===el.dataset.id);const c=currentChat();if(c)c.model=m.name;else S.draftModel=m.name;rememberView();closeSheet(true);render();toast('模型已选择，应用于后续消息。');},
    participants:()=>openSheet('参与专家','<p class="sheet-description">选择下一条消息的收件人，也可以在输入框中使用 @。每段对话的历史独立保存。</p>'+S.experts.map(e=>row(e.name,e.description,'pick-expert','person',`data-id="${e.id}"`,S.recipient===e.id?'已选择':'')).join('')),
    'pick-expert':el=>{S.recipient=el.dataset.id;closeSheet(true);updateRecipient();rememberView();$('#prompt').focus();},
    resources:()=>openSheet('对话资料','<p class="sheet-description">这里的附件只属于本段对话。资料库中的来源需要明确引用后才用于当前任务。</p>'+row('当前对话内容','本段消息 · 未读取其他历史','resource-detail','file')+(S.attachment?row(S.attachment,'待发送附件 · 演示','resource-detail','file'):'')+button('添加示例附件','sample-attachment','outline')),
    'resource-detail':()=>openSheet('资料来源',kv('归属','当前对话')+kv('内容',currentChat()?.messages.find(m=>m.role==='user')?.text||'尚未发送消息')+'<p class="inline-notice">引用的是本段对话中明确提供的信息，不代表访问了其他会话或私人数据。</p>'),
    progress:()=>openSheet('任务进度',S.run && S.run.chatId===currentChat()?.id?kv('当前状态',S.run.phase==='approval'?'等待你的确认':S.run.phase==='error'?'连接中断':'正在回复')+kv('执行者',S.run.author)+kv('归属',currentChat().title):empty('当前没有运行中的任务','完成的处理过程保留在对应回复中。','check')),
    'conversation-settings':()=>editor('对话设置',choose('权限策略','permission',['谨慎','按需确认'],(currentChat()?.settings||S.draftSettings).permission||'谨慎')+choose('长期记忆','memory',['跟随默认','仅本段对话'],(currentChat()?.settings||S.draftSettings).memory||'跟随默认')+'<p class="sheet-description">设置仅作用于本段对话。敏感操作仍需要明确确认。</p>',v=>{const c=currentChat();if(c)c.settings=v;else S.draftSettings=v;closeSheet(true);toast('对话设置已保存到演示。');}),
    'attach-menu':()=>openSheet('添加到消息',menuRow('示例附件','sample-attachment','file','不打开或读取你的文件')+menuRow('邀请专家','participants','person','为下一条消息选择收件人')),
    'sample-attachment':()=>{S.attachment='项目说明（示例）.txt';closeSheet(true);updateRecipient();rememberView();$('#prompt').focus();},
    'clear-recipient':()=>{S.recipient='';updateRecipient();rememberView();},
    'clear-attachment':()=>{S.attachment='';updateRecipient();rememberView();},
    'command-select':el=>selectCommand(el.dataset.name), mention:el=>selectMention(el.dataset.id),latest:toLatest,
    'save-plan':()=>{S.planSaved=!S.planSaved;rememberView();render();toast(S.planSaved?'安排已保存在本段演示对话。':'已取消保存。');},
    'edit-plan':()=>editor('调整出发时间',choose('出发时间','time',['09:30','10:30'],S.planLate?'10:30':'09:30'),v=>{S.planLate=v.time==='10:30';rememberView();closeSheet(true);render();toast('安排已更新。');}),
    copy:async el=>{const m=currentChat()?.messages[Number(el.dataset.index)];const text=m?.text||new DOMParser().parseFromString(m?.html||'','text/html').body.textContent;try{await navigator.clipboard.writeText(text);toast('回复已复制。');}catch{openSheet('复制回复',`<label class="field">选择并复制<textarea readonly>${esc(text)}</textarea></label>`);}},
    'message-menu':el=>openSheet('回复操作',menuRow('复制回复','copy','copy','',`data-index="${el.dataset.index}"`)+menuRow('查看来源','resource-detail','file')),
    'expert-new':()=>expertEditor(), 'expert-edit':el=>expertEditor(el.dataset.id),
    'task-new':()=>taskEditor(),'task-edit':el=>taskEditor(el.dataset.id),
    'task-toggle':el=>{const t=S.tasks.find(t=>t.id===el.dataset.id);t.enabled=!t.enabled;render();toast(t.enabled?'演示计划已启用。':'演示计划已暂停。');},
    'task-run':el=>{S.logs.unshift({task:el.dataset.id,text:'演示完成 · 没有读取资料或调用模型'});render();toast('试运行演示完成。');},
    'model-new':()=>modelEditor(),'model-edit':el=>modelEditor(el.dataset.id),
    'model-default':el=>{S.defaultModel=S.models.find(m=>m.id===el.dataset.id).name;render();toast('新对话默认模型已更新。');},
    'model-run':el=>{const m=S.models.find(m=>m.id===el.dataset.id);m.running=!m.running;render();toast('仅更新演示状态，没有启动本地模型。');},
    memory:el=>{const m=S.memories.find(m=>m.id===el.dataset.id);openSheet('记忆详情',`<p>${esc(m.name)}</p>${kv('归属',m.scope)}${kv('来源','虚构演示样本')}<div class="card-actions">${button('修订','memory-edit','outline',`data-id="${m.id}"`)}${button('移除','memory-delete','danger',`data-id="${m.id}"`)}</div>`);},
    'memory-new':()=>memoryEditor(),'memory-edit':el=>memoryEditor(el.dataset.id),
    'memory-delete':el=>openSheet('移除这条记忆？','<p>这条演示记忆将从列表中移除，对话历史不会改变。</p>',button('取消','close-sheet')+button('移除记忆','memory-confirm','danger outline',`data-id="${el.dataset.id}"`)),
    'memory-confirm':el=>{S.memories=S.memories.filter(m=>m.id!==el.dataset.id);closeSheet(true);render();toast('记忆已移除。');},
    'source-new':()=>editor('添加资料来源',field('名称','name','','text',true)+choose('类型','kind',['网页','文件夹'],'网页')+field('演示位置','location','https://example.invalid/docs')+'<p class="sheet-description">仅用于设计预览，不会访问这个地址或目录。</p>',v=>{S.sources.push({id:`s${++sequence}`,name:v.name,detail:`${v.kind} · ${v.location}`,scope:'仅此工作区',status:'未验证',icon:v.kind==='网页'?'link':'folder'});closeSheet(true);render();toast('演示来源已添加。');}),
    'verify-source':el=>{const s=S.sources.find(s=>s.id===el.dataset.id);s.status='可用';render();toast('模拟验证完成；未读取设备或网络。');},
    'source-access':el=>{const s=S.sources.find(s=>s.id===el.dataset.id);editor('访问设置',choose('资料范围','scope',['仅此工作区','请求时确认'],s.scope),v=>{s.scope=v.scope;closeSheet(true);render();toast('演示访问范围已更新。');});},
    'skill-config':el=>{const s=S.skills.find(s=>s.id===el.dataset.id);editor('配置能力',choose('使用范围','scope',['工作区','全局'],'工作区')+area('说明','description',s.description),v=>{s.description=v.description;s.detail=`${s.id==='writing'?'Skill':'MCP'} · ${v.scope}`;closeSheet(true);render();toast('演示配置已保存。');});},
    'skill-new':()=>editor('添加技能或连接',field('名称','name','','text',true)+choose('类型','kind',['Skill','MCP'],'Skill')+area('说明','description'),v=>{S.skills.push({id:`skill${++sequence}`,name:v.name,description:v.description,detail:`${v.kind} · 工作区`,enabled:false});closeSheet(true);render();toast('演示能力已添加，默认未启用。');}),
    appearance:()=>openSheet('外观', [['system','跟随系统'],['light','浅色'],['dark','深色']].map(([v,label])=>`<button class="list-row ${S.theme===v?'selected':''}" data-action="theme" data-value="${v}"><span class="grow">${label}</span>${S.theme===v?icon('check'):''}</button>`).join('')),
    theme:el=>{S.theme=el.dataset.value;document.documentElement.dataset.theme=S.theme;closeSheet(true);render();},
    'access-detail':el=>openSheet(el.dataset.name,'<p class="sheet-description">此原型不请求系统权限。实际应用应在需要使用时说明用途，并由系统授权界面确认。</p>'+kv('系统授权','尚未检查')+kv('应用操作策略','敏感读取按具体资源确认')),
    scenarios:()=>openSheet('交互场景','<p class="sheet-description">选择后，在新对话或现有对话中发送消息。以下都是预设演示，不会调用真实模型。</p>'+[['normal','普通流式回复','处理摘要与正文逐步显示'],['long','长回复与阅读位置','向上阅读时不强制拉回底部'],['approval','需要确认的操作','具体资源、操作与授权范围'],['error','连接中断与重试','消息保留，不重复创建会话']].map(([v,title,desc])=>row(title,desc,'scenario','chat',`data-value="${v}"`,S.scenario===v?'已选择':'')).join('')),
    scenario:el=>{S.scenario=el.dataset.value;closeSheet(true);toast('场景已选择。回到对话发送一条消息即可体验。');},
    about:()=>openSheet('关于这份原型','<p>为对话留出空间，让每个操作在需要时出现。</p><p class="inline-notice">所有会话、模型、专家、资料和运行结果均为虚构演示。刷新页面恢复初始样本，不读取或修改实际应用数据。</p><div class="list">'+menuRow('体验异常与长任务','scenarios','chat')+'</div>'),
    approve:()=>{S.run.phase='stream';S.scenario='normal';updateRun();startStream();},
    deny:()=>{const c=currentChat();c.messages.push({role:'assistant',author:S.run.author,text:S.run.phase==='approval'?'未读取文件。你可以提供其他资料，或调整这次任务。':'本次请求已结束。你的消息已保留。'});S.run=null;rememberView();render();toLatest();},
    retry:()=>{S.run.phase='stream';S.scenario='normal';updateRun();startStream();},
};
document.addEventListener('click',event=>{
    const el=event.target.closest('[data-action]');if(!el||el.disabled)return;
    const action=el.dataset.action;if(action==='send')return;
    const handler=actions[action];if(handler){event.preventDefault();handler(el);}
});
document.addEventListener('submit',e=>{if(e.target.id==='composer'){e.preventDefault();submit();}else if(e.target.id==='editor'){e.preventDefault();actions['save-editor']();}});
document.addEventListener('input',e=>{
    if(e.target.id==='prompt'){$('#command-error').hidden=true;e.target.removeAttribute('aria-describedby');resizeInput();updateSend();if(!e.isComposing)renderSuggestions();rememberView();}
    if(e.target.id==='search'){
        const pos=e.target.selectionStart;S.filter=e.target.value;
        const view=({conversations:conversationList,experts:expertsPage,skills:skillsPage})[S.route.page];
        if(view){const doc=new DOMParser().parseFromString(view(),'text/html');$('#results').innerHTML=$('#results',doc).innerHTML;}
        e.target.setSelectionRange(pos,pos);
    }
});
document.addEventListener('change',e=>{
    if(e.target.dataset.toggle==='notifications'){S.notifications=e.target.checked;toast('通知偏好已更新到演示。');}
    if(e.target.dataset.toggle==='skill'){S.skills.find(s=>s.id===e.target.dataset.id).enabled=e.target.checked;toast('能力状态已更新；具体操作仍需权限检查。');}
});
document.addEventListener('keydown',e=>{
    if(e.key==='Escape'&&!$('#sheet').open&&$('#suggestions')&&!$('#suggestions').hidden){hideSuggestions();e.preventDefault();return;}
    if(e.target.id==='prompt'&&!e.isComposing){
        const panel=$('#suggestions');
        if(!panel.hidden&&['ArrowDown','ArrowUp'].includes(e.key)){
            e.preventDefault();const options=[...panel.querySelectorAll('[role="option"]')];if(!options.length)return;
            const index=options.findIndex(o=>o.getAttribute('aria-selected')==='true'),next=(index+(e.key==='ArrowDown'?1:-1)+options.length)%options.length;
            options.forEach((o,i)=>o.setAttribute('aria-selected',String(i===next)));options[next].scrollIntoView({block:'nearest'});$('#prompt').setAttribute('aria-activedescendant',options[next].id);return;
        }
        if((e.key==='Enter'&&!e.shiftKey)||e.key==='Tab'&&!e.shiftKey&&!panel.hidden){
            if(!panel.hidden){const option=$('[aria-selected="true"]',panel);if(!option&&e.key==='Tab'){hideSuggestions();return;}e.preventDefault();if(option)actions[option.dataset.action](option);else if(e.target.value.startsWith('/'))executeCommand(e.target.value.trim());return;}
            if(e.target.value.trim().startsWith('/') || !matchMedia('(max-width:760px)').matches || e.metaKey || e.ctrlKey){e.preventDefault();submit();}
        }
    }
    if((e.metaKey||e.ctrlKey)&&e.key==='k'){e.preventDefault();navigate('conversations');$('#search').focus();}
});
document.addEventListener('compositionend',e=>{if(e.target.id==='prompt')renderSuggestions();});
$('#sheet').addEventListener('cancel',e=>{e.preventDefault();closeSheet();});
$('#sheet').addEventListener('click',e=>{if(e.target===$('#sheet')){const r=e.target.getBoundingClientRect();if(e.clientY<r.top||e.clientY>r.bottom||e.clientX<r.left||e.clientX>r.right)closeSheet();}});
let swipe;
$('#main').addEventListener('touchstart',e=>{const p=e.touches[0];swipe=p.clientX<28?{x:p.clientX,y:p.clientY}:null;},{passive:true});
$('#main').addEventListener('touchend',e=>{if(!swipe||$('#sheet').open)return;const p=e.changedTouches[0];if(p.clientX-swipe.x>90&&Math.abs(p.clientY-swipe.y)<70&&(S.route.page==='chat'||S.route.id)){S.route.page==='chat'?actions['chat-back']():back();}swipe=null;},{passive:true});
// Desktop starts in a readable example. Phones start in the conversation list.
if(!matchMedia('(max-width:760px)').matches)S.route={page:'chat',id:'weekend'};
render();
