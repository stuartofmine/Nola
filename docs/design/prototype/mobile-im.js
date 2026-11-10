'use strict';
// Mobile design proposal. Synthetic, in-memory interactions; no product/API calls.
window.mobileIM = (() => {
    const state = { filter:'all', os:'ios', textSize:false, themeMode:'system', returnPage:'settings', lastPage:null,
        origins:{}, efforts:{}, effortOwner:'', scrolls:{}, source:null, raf:0, interrupted:false, newCount:0, detail:null, outgoing:null, navigation:null, gestureReturn:false };
    const active = () => window.architectureModule?.getMode() === 'mobile';
    const icon = (name,label,action,extra='') => `<button type="button" class="im-icon ${extra}" aria-label="${esc(label)}" title="${esc(label)}" data-action="im:${action}">${svg(name)}</button>`;
    const button = (title,action,cls='im-primary') => `<button type="button" class="${cls}" data-action="im:${action}">${esc(title)}</button>`;
    const header = (title,back='',right='',sub='') => `<header class="im-header">${back?icon('back','返回','back:'+back):'<span class="im-header-space"></span>'}<div class="im-heading"><h1>${esc(title)}</h1>${sub?`<p>${esc(sub)}</p>`:''}</div>${right||'<span class="im-header-space"></span>'}</header>`;
    const row = (ic,title,sub,action,trailing='') => `<button type="button" class="im-setting-row" data-action="im:${action}"><span class="im-row-icon">${svg(ic)}</span><span class="im-row-copy"><strong>${esc(title)}</strong>${sub?`<small>${esc(sub)}</small>`:''}</span><span class="im-trailing">${esc(trailing)}</span>${svg('chevron')}</button>`;
    const parent = (route, fallback='information') => state.origins[route] || fallback;
    const initials = c => c.experts?.length ? c.experts.map(x=>x.slice(0,1)).join('').slice(0,2) : c.title.slice(0,1);
    const subtitle = c => c.pending ? '等你确认 · 研究、写作' : c.messages.some(m=>m.error) ? '部分未完成 · 查看原因' : c.experts.length ? c.experts.join('、')+' · 本机' : 'Nola · 私人对话';
    const pendingCount=()=>1+(conversations.find(c=>c.id==='design')?.pending?1:0);
    function tabs() {
        const selected = state.origins[page]==='settings' ? 'settings' : page === 'conversations' ? 'conversations' : ['activity','im-activity'].includes(page) ? 'activity' : ['information','data','memory','plans','knowledge'].includes(page) ? 'information' : 'settings';
        return `<nav class="im-tabs" aria-label="主导航">${[['conversations','chat','对话'],['activity','clock','活动'],['information','folder','资料'],['settings','settings','设置']].map(([id,ic,title])=>`<button type="button" data-action="im:nav:${id}" ${selected===id?'aria-current="page"':''}>${svg(ic)}<span>${title}</span>${id==='activity'?`<i aria-label="${pendingCount()} 项待回应">${pendingCount()}</i>`:''}</button>`).join('')}</nav>`;
    }
    function list() {
        const filtered = conversations.filter(c=>(state.filter!=='waiting'||c.pending) && (c.title+' '+c.preview+' '+(drafts[c.id]||'')+' '+c.messages.map(m=>m.text).join(' ')).toLowerCase().includes(query.toLowerCase()));
        return `<section class="im-screen im-list ${new URLSearchParams(location.search).get('list')==='marker'?'im-list-marked':''}">${header('对话','',state.os==='ios'?icon('edit','新建对话','new'):icon('search','搜索对话','focus-search'))}
        <label class="im-search">${svg('search')}<input id="im-search" type="search" placeholder="搜索对话与内容" aria-label="搜索对话与内容" value="${esc(query)}"></label>
        <div class="im-filters" aria-label="会话筛选">${[['all','全部'],['waiting','待回应']].map(([id,title])=>`<button type="button" aria-pressed="${state.filter===id}" data-action="im:filter:${id}">${title}${id==='waiting'?`<span>${conversations.filter(c=>c.pending).length}</span>`:''}</button>`).join('')}<span class="im-prototype-label">交互原型</span></div>
        <div class="im-list-rows" id="im-list-rows">${filtered.map(c=>`<button type="button" class="im-chat-row" data-action="im:chat:${c.id}" aria-label="${esc(c.title)}，${esc(drafts[c.id]?'草稿':subtitle(c))}">
        ${new URLSearchParams(location.search).get('list')==='marker'?`<span class="im-list-marker" aria-hidden="true">${svg(c.pending?'clock':c.messages.some(m=>m.error)?'retry':'chat')}</span>`:''}<span class="im-chat-copy"><span class="im-chat-line"><strong>${esc(c.title)}</strong><time>${esc(c.time)}</time></span><span class="im-preview">${drafts[c.id]?`<b class="im-draft-label">草稿</b><span class="im-preview-separator"> · </span>${esc(drafts[c.id])}`:c.pending?`<b class="im-wait-label">待确认</b><span class="im-preview-separator"> · </span>${esc(c.preview)}`:c.messages.some(m=>m.error)?`<b class="im-failed-label">部分未完成</b><span class="im-preview-separator"> · </span>${esc(c.preview)}`:esc(c.preview)}</span></span></button>`).join('')||'<div class="im-empty"><h2>没有找到对话</h2><p>换一个词试试，或创建一段新对话。</p></div>'}</div>
        ${state.os==='android'?icon('edit','新建对话','new','im-fab'):''}</section>`;
    }
    function actions(i,user=false) {return `<div class="im-message-actions ${user?'is-user':''}">${icon('copy','复制消息','copy:'+i)}${user?`<button type="button" class="im-icon" aria-label="编辑并重发" data-action="im:edit:${i}" ${state.source?'disabled title="请先停止或等待当前生成完成"':''}>${svg('edit')}</button>`:`<button type="button" class="im-message-quote" data-action="im:quote:${i}">引用</button>`}</div>`;}
    function message(m,i) {
        const user=m.role==='user',role=user?'你':m.role;
        const rich=m.rich?'<pre class="im-code" tabindex="0" aria-label="可横向滚动的代码"><code>async function resumeTask(taskId, expectedRevision) {\n  return core.resume({ taskId, expectedRevision });\n}</code></pre><div class="im-table-scroll" tabindex="0" aria-label="可横向滚动的表格"><table><thead><tr><th>环节</th><th>所有权</th><th>用户可见状态</th></tr></thead><tbody><tr><td>等待授权</td><td>Core 精确请求</td><td>等你确认</td></tr><tr><td>正在执行</td><td>绑定的 Host</td><td>执行中</td></tr><tr><td>封存结果</td><td>Core 终态证据</td><td>已完成或未完成</td></tr></tbody></table></div>':'';
        const body = m.error ? '<p>这部分没有完成，已生成的内容仍保留。</p>'+button('查看原因','error','im-inline') : esc(m.text).split('\n\n').map(p=>`<p>${p.replace(/\n/g,'<br>')}</p>`).join('');
        return `<article class="im-message ${user?'im-outgoing':''}" ${!user&&role!=='Nola'?`data-expert-color="${expertColor(role)}"`:''}>
        ${!user?`<aside class="im-message-person"><span class="im-mini-avatar">${role==='Nola'?'n':esc(role.slice(0,1))}</span><span>${esc(role)}</span></aside>`:''}
        <div class="im-message-content"><div class="im-bubble">${body}${rich}<span class="im-message-time">${i===0?'10:24':'10:25'}</span></div>
        ${m.permission==='pending'?`<section class="im-permission"><strong>允许保存设计草案？</strong><p>只写入本地工作区的「设计草案.md」。本次不会对外发送。</p><div>${button('暂不保存','deny:'+i,'im-quiet')}${button('仅允许本次','allow:'+i,'im-inline')}</div></section>`:''}${actions(i,user)}</div></article>`;
    }
    function liveHTML(c) {return state.source?.id===c.id?`<article class="im-message im-live"><aside class="im-message-person"><span class="im-mini-avatar">n</span><span>Nola</span></aside><div class="im-message-content"><div class="im-bubble"><p id="im-stream-body">${esc(state.source.text.slice(0,state.source.shown))}</p><span class="im-message-time">本地回放</span></div></div></article>`:'';}
    function chat() {
        const c=current();if(!c)return '';
        return `<section class="im-screen im-chat" data-conversation="${esc(c.id)}"><header class="im-header im-chat-header">${icon('back','返回对话列表','back:conversations')}<button type="button" class="im-chat-title" data-action="im:nav:im-details"><span class="im-title-avatar">${esc(initials(c))}</span><span><strong>${esc(c.title)}</strong><small id="im-chat-status">${esc(state.source?.id===c.id?'本地回放 · 正在生成':subtitle(c))}</small></span></button>${icon('search','搜索本段对话','chat-search')}</header>
        <div class="im-transcript" id="transcript" tabindex="0" aria-label="对话消息"><div class="im-date">今天</div>${c.branchOf?button('查看原会话','chat:'+c.branchOf,'im-inline'):''}${c.messages.map(message).join('')}${liveHTML(c)}${c.messages.length||state.source?.id===c.id?'':`<div class="im-empty"><span class="im-empty-mark">n</span><h2>${models.length?'从一句话开始':'先连接一个模型'}</h2><p>${models.length?'提问、写作，或一起完成一件事。':'API、ChatGPT 账户或本地模型，选择适合你的方式。'}</p>${models.length?'':button('配置模型','nav:models')}</div>`}</div>
        <button type="button" class="im-jump" aria-label="回到最新消息" hidden>${svg('down')}<span>新内容</span></button>
        <footer class="im-composer-wrap"><form id="im-composer" class="im-composer"><textarea id="compose" rows="1" aria-label="消息" placeholder="发消息…">${esc(drafts[c.id]||'')}</textarea><div class="im-composer-toolbar">${icon('plus','添加资料','attach')}<button type="button" class="im-model-control" data-action="im:composer-model">${svg('models')}<span>${esc(c.model||defaultModel||'选择模型')}</span>${svg('down')}</button>${models.find(m=>m.name===(c.model||defaultModel))?.reasoningEfforts?`<button type="button" class="im-effort-control" data-action="im:composer-effort" aria-label="推理档位">${esc(state.efforts['conversation:'+c.id]||c.reasoningEffort||'默认')}${svg('down')}</button>`:''}<span class="im-toolbar-space"></span>${icon(state.source?.id===c.id?'stop':'arrow',state.source?.id===c.id?'停止本地回放':'发送消息',state.source?.id===c.id?'stop':'send','im-send')}</div></form></footer></section>`;
    }
    function details() {
        const c=current();return `<section class="im-screen im-grouped">${header('会话详情','chat')}<div class="im-profile"><span class="im-avatar im-profile-avatar">${esc(initials(c))}</span><h2>${esc(c.title)}</h2><p>${esc(subtitle(c))}</p></div><div class="im-group">${row('models','模型',defaultModel||'尚未配置','nav:models')}${row('person','参与者',c.experts.length?c.experts.join('、'):'Nola','participants')}${row('host','执行位置','本机 · App 可执行期间','host')}</div><div class="im-group">${row('goal','任务与进度','状态、待回应事项和执行结果','nav:activity')}${row('folder','本段资料','附件、产物与引用','nav:information')}${row('overview','上下文','当前会话与已授权引用','context')}</div><div class="im-group">${row('edit','重命名对话','','rename')}${row('pin','置顶对话','','pin')}</div><p class="im-footnote">会话保留长期记录。任务和执行尝试有各自的状态，不因切换页面而被结束。</p></section>`;
    }
    function activity() {
        return `<section class="im-screen im-grouped">${header('活动','',icon('calendar','查看计划','nav:plans'),'来自本机可见的任务与结果')}<div class="im-section-label">需要你回应 · ${pendingCount()}</div><div class="im-group">${conversations.find(c=>c.id==='design')?.pending?row('review','设计草案等待保存','让 Nola 更像 Nola · 等待授权','activity:permission'):''}${row('chat','阅读摘记，保留哪些内容？','整理已暂停 · 等你补充范围','activity:input')}</div><div class="im-section-label">已完成</div><div class="im-group">${row('check','本地模型比较已整理','本地模型，怎么选更合适 · 10:18','activity:result')}</div><div class="im-section-label">未完成与失败</div><div class="im-group">${row('execution','模型请求失败','连接被拒绝 · 已保留原有记录','activity:failed')}</div><p class="im-footnote">切到其他会话不会替你批准、重试或停止任务。这里展示本地演示记录。</p></section>`;
    }
    function activityDetail() {
        const failed=state.detail==='failed';const sourceId=state.detail==='permission'?'design':state.detail==='input'?'reading':'compare';const source=conversations.find(c=>c.id===sourceId);const waiting=state.detail==='input'||state.detail==='permission'&&!!source?.pending;return `<section class="im-screen im-grouped">${header('活动详情','activity')}<div class="im-detail-summary"><span class="im-state-label">${failed?'失败':waiting?'需要你回应':state.detail==='result'?'已完成':'已回应（演示）'}</span><h2>${failed?'模型请求失败':state.detail==='permission'?'保存设计草案':state.detail==='input'?'补充阅读整理范围':'本地模型比较'}</h2><p>${failed?'连接被拒绝。该次请求已结束，保留失败记录；不会自动重新发送。':waiting?'已有内容保留，执行等待你的决定。':state.detail==='result'?'按速度、内存和稳定性分别比较，没有用一个总分隐藏取舍。':'授权决定已在本地演示中记录；没有执行真实文件写入。'}</p></div><div class="im-group">${failed?'':row('chat','来源对话',source?.title||'来源不可用','chat:'+sourceId)}${row('host','执行位置','本机 · 本次执行','host')}</div>${failed?'<section class="im-note"><h3>失败原因</h3><p>Connection refused · 虚构示例</p><h3>来源对话</h3><p>当前设备暂时无法打开此来源。详情可以查看和返回，不显示没有作用的跳转按钮。</p></section>':waiting?`<section class="im-note"><h3>${state.detail==='permission'?'待授权操作':'等待补充'}</h3><p>${state.detail==='permission'?'将设计草案写入本地工作区。不对外发送。':'请在来源会话说明保留哪些阅读内容。'}</p>${button('回到会话处理','chat:'+sourceId)}</section>`:'<section class="im-note"><h3>结果摘要</h3><p>固定相同输入，再记录首字延迟、生成速度与资源占用。工具失败和中断恢复单独说明。</p></section>'}<p class="im-footnote">详情不创造新的审批权。原生实现必须读取 Core 当前有效的请求和结果。</p></section>`;
    }
    function library() {return `<section class="im-screen im-grouped">${header('资料','','','会话知识、来源与长期记忆')}<div class="im-group">${row('file','会话知识','保留知识点与原始对话的关联','nav:knowledge')}${row('person','长期记忆','偏好、依据与可见范围','nav:memory')}${row('folder','资料与数据源','已选择文档、连接与访问策略','nav:data')}</div><p class="im-footnote">打开资料不代表授权访问，也不会自动进入所有会话。</p></section>`;}
    function dataPage() {return `<section class="im-screen im-grouped">${header('资料与数据源',parent('data'),icon('plus','添加资料或数据源','source-new'))}<div class="im-group">${documents.map((d,i)=>row('file',d.name,`${d.type} · ${d.policy||'仅所选资料'} · ${d.health||'未验证'}`,'source:'+i)).join('')}</div><p class="im-footnote">只展示演示资料，不读取文件或连接远程地址。返回保留实际入口。</p></section>`;}
    function memoryPage() {return `<section class="im-screen im-grouped">${header('长期记忆',parent('memory'))}<section class="im-note"><h2>自动管理的记忆</h2><p>只保留仍然有效且有来源支持的理解。记忆更新不移动整页文本。</p></section><div class="im-group">${workspaceModule.memories.map(m=>row('person',m.text,`${m.category} · 置信度${m.confidence}`,'memory-evidence:'+m.id)).join('')}</div><p class="im-footnote">本页为只读演示；会话知识与长期记忆分别管理。</p></section>`;}
    function knowledgePage() {return `<section class="im-screen im-grouped">${header('会话知识',parent('knowledge'))}<div class="im-group">${workspaceModule.records.map(r=>row('file',r.title,r.keyPointsMarkdown||'还没有整理知识点','chat:'+r.id)).join('')}</div><p class="im-footnote">查看具体来源会话，不把不同会话的知识合并成一个授权范围。</p></section>`;}
    function settings() {
        return `<section class="im-screen im-grouped">${header('设置')}<div class="im-group">${row('settings','通用','外观、通知与软件更新','nav:im-general')}</div><div class="im-group">${row('models','模型与账户',defaultModel||'未配置 · 点击开始','nav:models')}${state.os==='ios'?row('person','管理专家','配置专家与执行范围','agents'):''}${row('execution','Skills 与 MCP','管理可复用能力','nav:capabilities')}</div><div class="im-group">${row('review','隐私与数据','数据、导出与访问范围','privacy')}</div><details class="im-preview-tools"><summary>原型预览</summary><p class="im-footnote"><a href="scenarios.html?platform=${state.os}">全业务场景与状态总览 ↗</a></p><div class="im-group">${row('host','交互平台',state.os==='ios'?'iOS':'Android','platform')}${row('progress','减少动态效果',reduced?'已开启':'跟随系统偏好，并可预览','reduce',reduced?'开启':'关闭')}${row('overview','大字预览','只改变此原型的文字尺寸','text',state.textSize?'开启':'关闭')}${row('chat','体验消息与流式动画','合成回放，不调用模型','demo')}${row('file','长内容排版','代码和表格在自身横向滚动','rich-demo')}${row('overview','设计与交互说明','导航、状态与验证边界','spec')}</div></details><p class="im-footnote">原型同步 2026-11-10 · 不修改 App 或个人配置。</p></section>`;
    }
    function general() {
        return `<section class="im-screen im-grouped">${header('通用','settings')}<h2 class="prototype-setting-heading">外观</h2><div class="im-group">${row(theme==='dark'?'moon':'sun','主题',state.themeMode==='system'?'跟随系统':theme==='dark'?'深色':'浅色','theme','选择')}</div><p class="im-footnote">动画跟随系统的减少动态效果设置；预览控制在设置首页。</p><h2 class="prototype-setting-heading">通知</h2><div class="im-group">${state.os==='android'?row('bell','通知中心','任务完成与需要回应的事项','notifications'):`<div class="prototype-toggle-row"><span>启用通知<small>只改变本页演示状态</small></span><button type="button" role="switch" class="prototype-switch" aria-label="启用通知（演示）" aria-checked="${window.enhancedModule.preferences.notifications}" data-action="im:notification-toggle"></button></div>`}${row('review','系统通知权限','权限受限时显示设置入口，不重复申请','notification-permission')}</div><section class="prototype-settings-update im-management-content"><h2 class="prototype-setting-heading">软件更新</h2>${window.updatePreview.content()}</section>${state.os==='android'?'<section class="im-note"><h2>关于</h2><p>Nola 1.0.0 · 版本为原型示例</p></section>':''}</section>`;
    }
    function modelList() {return `<section class="im-screen im-grouped">${header('模型与账户',state.returnPage,icon('plus','添加模型连接','nav:im-add-model'))}<section class="im-note"><h2>${models.length?(['chat','im-details'].includes(state.returnPage)?'选择会话模型':'选择默认模型'):'先连接一个模型'}</h2><p>账户、API 连接与本地模型分别管理。导入完成不等于请求可用。</p>${button('添加连接','nav:im-add-model')}</section>${models.length?`<div class="im-section-label">已配置</div><div class="im-group">${models.map((m,i)=>row('models',m.name,m.provider,'model:'+i,m.name===(['chat','im-details'].includes(state.returnPage)?current().model||defaultModel:defaultModel)?(['chat','im-details'].includes(state.returnPage)?'当前':'默认'):'')).join('')}</div>`:''}<p class="im-footnote">这是布局演示，不读取登录文件或保存真实密钥。</p></section>`;}
    function modelDetail() {const m=models[Number(state.detail)]||models[0];return `<section class="im-screen im-grouped">${header(m?.name||'模型详情','models')}<section class="im-note"><span class="im-state-label">演示配置 · 未验证</span><h2>${esc(m?.name||'模型')}</h2><p>${esc(m?.provider||'API 连接')}</p></section><div class="im-group">${row('models',['chat','im-details'].includes(state.returnPage)?'为本段对话选择':'设为默认',['chat','im-details'].includes(state.returnPage)?'只影响本段对话的后续发送':'只改变新对话的默认选择','model-default')}${Array.isArray(m?.reasoningEfforts)?row('review','推理档位',state.efforts['model:'+state.detail]||'继承模型默认','reasoning'):'<p class="im-footnote">此演示配置未声明推理档位，不推断支持范围。</p>'}${row('host','连接状态','状态与错误保留在本页','model-check')}</div><p class="im-footnote">公开推理摘要可能为空；不把私有思考伪装成可见内容。</p></section>`;}
    function addModel() {return `<section class="im-screen im-grouped">${header('添加连接','models')}<section class="im-note"><h2>从哪里连接？</h2><p>先选来源，再填写和验证。不同账户的授权与可用模型不混用。</p></section><div class="im-group">${row('link','API 服务','服务地址、API 密钥与模型','connect:API 服务')}${row('person','ChatGPT 账户','在支持的 Host 上管理授权与可用模型','connect:ChatGPT 账户')}${row('models','本地模型','先选择模型，再检查设备与下载','connect:本地模型')}</div><div class="im-group">${row('file','导入配置文件','预览来源、服务地址与凭据状态','connect:导入配置文件')}</div></section>`;}
    function view(p) {
        if(!active())return null;
        document.documentElement.dataset.mobileProposal='im24';
        if(state.themeMode==='system'){theme=matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light';applyTheme();}
        document.documentElement.classList.toggle('im-large-type',state.textSize);
        const content = p==='conversations'?list():p==='chat'?chat():p==='im-details'?details():p==='activity'?activity():p==='im-activity'?activityDetail():p==='information'?library():p==='settings'?settings():['im-general','im-appearance','im-updates'].includes(p)?general():p==='data'?dataPage():p==='memory'?memoryPage():p==='knowledge'?knowledgePage():p==='capabilities'?`<section class="im-screen im-grouped">${header('Skills 与 MCP',parent('capabilities','settings'))}<div class="im-management-content">${manageModule.capabilitiesContent()}</div></section>`:p==='models'?modelList():p==='im-model'?modelDetail():p==='im-add-model'?addModel():p==='plans'?`<section class="im-screen im-grouped">${header('计划',parent('plans','activity'))}<div class="im-group">${plans.map(p=>row('calendar',p.name,p.time,'plan',p.on?'已开启':'已暂停')).join('')}</div><p class="im-footnote">触发时间与执行状态由 Core 提供。手机不可执行时不承诺持续后台运行。</p></section>`:null;
        if(!content)return null;
        return `${p==='chat'?`<div class="im-backdrop" inert aria-hidden="true">${list()}${tabs()}</div>`:''}<div class="im-stage" data-route="${p}" data-os="${state.os}">${content}</div>`;
    }
    const rootPages=['conversations','activity','information','settings'];
    const bottomNav=()=>page==='chat'||page==='im-details'||['chat','im-details'].includes(state.returnPage)&&['models','im-model','im-add-model'].includes(page)?'':tabs();
    const motionReduced = () => reduced || matchMedia('(prefers-reduced-motion: reduce)').matches;
    const ease = 'cubic-bezier(.22,.8,.25,1)';
    function capture() {
        if(!active())return;
        const t=document.querySelector('.im-transcript'),owner=t?.closest('.im-chat')?.dataset.conversation;
        if(owner)state.scrolls[owner]=t.scrollTop;
        const old=document.querySelector('.im-stage:not(.im-departing)');
        if(old&&old.dataset.route!==page&&!motionReduced()&&!state.gestureReturn){
            state.outgoing=old.cloneNode(true);const nav=document.querySelector('.im-tabs');if(nav)state.outgoing.append(nav.cloneNode(true));state.outgoing.inert=true;state.outgoing.setAttribute('aria-hidden','true');
            state.outgoingScroll=t?.scrollTop||0;
        }
    }
    function go(p,id) {saveDraft();navigate(p,id);}
    function sheet(title,body) {openDialog(title,`<div class="im-sheet">${body}</div>`);}
    function handle(a) {
        if(!active()||!a.startsWith('im:'))return false;
        const [action,...rest]=a.slice(3).split(':'),v=rest.join(':');saveDraft();
        if(action==='nav'&&['data','memory','knowledge','plans','capabilities','im-updates','im-general'].includes(v))state.origins[v]=page;
        if(action==='nav'||action==='back'){state.navigation=action==='back'?'back':'forward';if(v==='models'&&['settings','im-details'].includes(page))state.returnPage=page;go(v);}
        if(action==='chat')go('chat',v);
        if(action==='new'){const id='im-'+Date.now();conversations.unshift({id,title:'新对话',preview:'',time:'刚刚',group:'今天',experts:[],messages:[]});go('chat',id);}
        if(action==='focus-search')document.querySelector('#im-search')?.focus();
        if(action==='filter'){state.filter=v;render();}
        if(action==='notification-toggle'){window.enhancedModule.preferences.notifications=!window.enhancedModule.preferences.notifications;render();document.querySelector('[data-action="im:notification-toggle"]')?.focus({preventScroll:true});}
        if(action==='theme')sheet('外观',button('跟随系统','theme-mode:system','im-sheet-action')+button('浅色','theme-mode:light','im-sheet-action')+button('深色','theme-mode:dark','im-sheet-action'));
        if(action==='theme-mode'){state.themeMode=v;theme=v==='system'?(matchMedia('(prefers-color-scheme:dark)').matches?'dark':'light'):v;applyTheme();closeDialog();render();document.querySelector('[data-action="im:theme"]')?.focus({preventScroll:true});}
        if(action==='reduce'){reduced=!reduced;applyTheme();render();}
        if(action==='text'){state.textSize=!state.textSize;render();}
        if(action==='platform'){state.os=state.os==='ios'?'android':'ios';render();}
        if(action==='activity'){state.detail=v;go('im-activity');}
        if(action==='source-new')window.handle('ex-source-new');
        if(action==='source')window.handle('ex-source:'+v);
        if(action==='memory-evidence'){const m=workspaceModule.memories.find(m=>m.id===v);if(m)sheet('记忆依据',`<p>${esc(m.text)}</p>${m.evidence.map(e=>row('chat',e.text,e.detail,'chat:'+e.id)).join('')}`);}
        if(action==='composer-model'){state.returnPage='chat';go('models');}
        if(action==='composer-effort'||action==='reasoning'){
            const index=action==='composer-effort'?models.findIndex(m=>m.name===(current().model||defaultModel)):Number(state.detail),m=models[index];
            state.effortOwner=action==='composer-effort'?'conversation:'+current().id:'model:'+index;
            if(m?.reasoningEfforts)sheet('推理档位',`<p>${action==='composer-effort'?'仅应用于本段对话的后续发送。':'应用于此演示模型配置。'}只展示此演示模型声明的档位，不验证真实能力。</p>${['inherit',...m.reasoningEfforts].map(e=>`<button type="button" class="im-sheet-action" aria-pressed="${(state.efforts[state.effortOwner]||'inherit')===e}" data-action="im:effort:${e}">${e==='inherit'?'继承模型默认':esc(e)}</button>`).join('')}`);
        }
        if(action==='effort'){state.efforts[state.effortOwner]=v==='inherit'?'':v;closeDialog();render();}
        if(action==='model'){state.detail=v;go('im-model');}
        if(action==='model-default'){const selected=models[Number(state.detail)]?.name;if(selected){if(['chat','im-details'].includes(state.returnPage))current().model=selected;else defaultModel=selected;}render();toast(['chat','im-details'].includes(state.returnPage)?'已更新本段对话的后续模型':'已更新新对话的默认模型');}
        if(action==='allow'||action==='deny'){window.handle(`${action==='allow'?'approve':'deny'}:${v}:${current().id}`);toast(action==='allow'?'演示：已允许本次保存，未写入真实文件':'演示：未执行保存');}
        if(action==='copy')copyMessage(current().id,Number(v));
        if(action==='edit'){if(state.source)toast('请先停止或等待当前生成完成');else window.handle('message-edit:'+current().id+':'+v);}
        if(action==='message')sheet('消息操作',button('复制消息','copy:'+v,'im-sheet-action')+button('引用到输入框','quote:'+v,'im-sheet-action')+'<p>在原生端，长按消息打开同一组操作。</p>');
        if(action==='quote'){drafts[activeId]='引用：'+current().messages[Number(v)].text.slice(0,72)+'\n';closeDialog();render();document.querySelector('#compose')?.focus();}
        if(action==='attach')sheet('添加资料','<p>只选择你打算提供给这段会话的内容。</p>'+button('选择本地文件','attachment-note','im-sheet-action')+button('从已有资料选择','nav:information','im-sheet-action'));
        if(action==='send')send();
        if(action==='replay')replay();
        if(action==='rich-demo'){go('chat','geometry');if(!current().messages.some(m=>m.rich))current().messages.push({role:'推理',text:'让结果留在会话里，长代码和表格只在自身范围内横向滚动。\n\n执行状态来自 Core；消息显示结束，不代表外部操作已经完成。',rich:true});render();document.querySelector('.im-transcript').scrollTop=999999;}
        if(action==='demo'){go('chat','weekend');replay();}
        if(action==='stop')stop();
        const descriptions={participants:['参与者','当前示例包含研究与写作。专家名称与头像位于气泡外侧，不用颜色代替身份。'],host:['执行位置','本机负责执行。离开页面不会取消任务；App 挂起后的能力取决于实际 Host 生命周期。'],context:['上下文范围','当前会话、已选择资料、经允许的记忆引用分别列明。上下文可见不等于获得工具权限。'],privacy:['隐私与权限','授权针对具体工具与资源。普通聊天、任务完成和外部操作的结果分别由 Core 确认。'],agents:['Agent 与执行器','模型与执行器分开选择。Codex、Claude Code、OpenCode 仅在对应 Host 已安装并授权时可用。'],capabilities:['Skills 与 MCP','技能提供工作说明；MCP 提供能力目录。启用并不自动授权调用。'],notifications:['通知','任务结束或需要回应时提醒。这里仅说明原型状态，不发送真实通知。'], 'notification-permission':['系统通知权限','真实客户端会读取系统状态；被拒绝时引导到系统设置。本原型不会申请权限或打开系统设置。'],error:['未完成原因','示例连接中断，保留已有回答；是否能重试必须先读取 Core 当前状态。'],plan:['计划详情','此处仅演示计划入口，不运行真实计划或发送通知。'],pin:['置顶对话','原生实现的行操作：置顶、归档与恢复；不通过横滑直接永久删除。'],rename:['重命名','原生实现保留输入、取消和保存；本原型不修改真实会话。'], 'attachment-note':['本地文件','本原型不读取文件。原生使用系统选择器并说明此会话的访问范围。'], 'model-check':['连接验证','原型不发请求，不显示虚假的“验证成功”。真实产品将网络、账户和能力错误留在连接详情。']};
        if(descriptions[action])sheet(...descriptions[action].map((x,i)=>i?`<p>${x}</p>`:x));
        if(action==='library')sheet(v,'<p>列表、来源与访问范围在同一页呈现。打开内容不会自动复制到全部会话。</p>'+button('查看来源对话','chat:design','im-sheet-action'));
        if(action==='connect')sheet(v,v==='ChatGPT 账户'?'<p>账户授权由支持该流程的本机或已连接 Host 管理。手机不复制 Mac 的账户凭据，也不把已登录等同于已启用模型。</p><p>保存账户 → 发现允许的模型 → 启用并在聊天中选择。真实支持范围由执行 Host 决定；原型不发起登录。</p>':v==='API 服务'?'<label>服务地址<input type="url" placeholder="https://api.example.com/v1"></label><label>模型 ID<input placeholder="服务提供的模型标识"></label><p>真实密钥请勿输入原型。下一步是核对配置，再在真实客户端保存与验证。</p>':'<p>先预览来源与可用范围，再确认。未验证状态必须明确保留，不把导入当作成功调用。</p>');
        if(action==='spec')sheet('设计说明','<p><a href="docs/MOBILE-IM-V24.md" target="_blank" rel="noopener">打开完整设计与动效说明</a></p><p>原型已对照本轮客户端验收修订；平台实现与未覆盖项见同步记录，不代表全部客户端完全一致。</p>');
        if(action==='chat-search')sheet('搜索本段对话','<label>搜索内容<input id="im-find" type="search" placeholder="输入关键词"></label><p id="im-find-count" role="status">仅搜索本段演示消息。</p>');
        const find=document.querySelector('#im-find');if(find)find.oninput=()=>document.querySelector('#im-find-count').textContent=`找到 ${current().messages.filter(m=>find.value&&m.text.includes(find.value)).length} 条消息`;
        return true;
    }
    function send() {
        if(state.source){toast('此原型一次只回放一段会话；不代表产品并发限制。');return;}const input=document.querySelector('#compose');const text=input?.value.trim();if(!text)return;
        if(!models.length){go('models');return;}const c=current();c.messages.push({role:'user',text,sendContext:{experts:[...c.experts],model:c.model||defaultModel,reasoningEffort:state.efforts['conversation:'+c.id]||c.reasoningEffort||null}});c.title=c.messages.length===1?text.slice(0,28):c.title;c.preview=text;drafts[c.id]='';render();if(!motionReduced())document.querySelector('.im-outgoing:last-of-type')?.animate([{opacity:.4,transform:'translateY(9px) scale(.98)'},{opacity:1,transform:'translateY(0) scale(1)'}],{duration:240,easing:ease});replay();
    }
    function stop() {
        if(!state.source)return;state.source.stopped=true;cancelAnimationFrame(state.raf);const el=document.querySelector('#im-stream-body');const c=conversations.find(c=>c.id===state.source.id);c.messages.push({role:'Nola',text:(el?.textContent||'')+'\n\n回放已停止，已有内容保留。'});state.source=null;render();
    }
    function replay() {
        if(state.source||page!=='chat')return;
        const c=current(),t=document.querySelector('.im-transcript');state.interrupted=!!t&&t.scrollHeight-t.scrollTop-t.clientHeight>40;
        const text='我们先把事情放回对话里。\n\n会话列表帮你找回正在做的事，草稿留在各自的会话。任务等你确认时，消息旁直接说明要做什么、影响哪里。\n\n新的文字轻轻浮现，已经读过的内容保持稳定。你向上翻阅时，界面停在原处；想继续看，只需点一下“回到最新消息”。';
        state.source={id:c.id,text,shown:0,received:0,started:performance.now(),doneAt:null};
        render();const transcript=document.querySelector('.im-transcript');
        const source=state.source;let lastPacket=0;
        function tick(now) {
            if(state.source!==source||source.stopped)return;
            const visible=page==='chat'&&activeId===source.id;
            const target=visible?document.querySelector('#im-stream-body'):null;
            const elapsed=now-source.started;
            // Synthetic packets, not a model-speed throttle. Display every received character immediately.
            if(elapsed-lastPacket>=110){
                source.received=Math.min(text.length,source.received+8);lastPacket=elapsed;
                if(source.received===text.length&&!source.doneAt)source.doneAt=now;
            }
            if(source.received>source.shown){
                const fresh=text.slice(source.shown,source.received);
                if(target){
                    for(const part of fresh.match(/[\s\S]{1,3}/gu)||[]){
                        const span=document.createElement('span');span.className='im-reveal';span.textContent=part;target.append(span);
                        span.addEventListener('animationend',()=>{span.classList.remove('im-reveal');},{once:true});
                    }
                    target.dataset.received=source.received;target.dataset.shown=source.received;
                    const sc=document.querySelector('.im-transcript');
                    if(!state.interrupted)sc.scrollTop=sc.scrollHeight;
                    else{state.newCount++;document.querySelector('.im-jump').hidden=false;}
                }
                source.shown=source.received;
            }
            if(source.received===text.length){
                c.messages.push({role:'Nola',text});state.source=null;
                if(target){
                    // Final fragments settle within 160ms; no long reveal queue after reception ends.
                    for(const animation of target.getAnimations({subtree:true})){
                        const remaining=Number(animation.effect.getTiming().duration)-Number(animation.currentTime||0);
                        if(remaining>160)animation.playbackRate=remaining/160;
                    }
                    document.querySelector('#im-chat-status').textContent='本地回放结束 · 内容已保留';
                    const send=document.querySelector('.im-send');send.innerHTML=svg('arrow');send.setAttribute('aria-label','发送消息');send.onclick=()=>handle('im:send');
                    const live=document.querySelector('.im-live');live.querySelector('.im-message-time').textContent=motionReduced()?'接收完成 · 即时显示':'接收完成 · 浮现收尾 ≤160 ms';
                    live.querySelector('.im-message-content').insertAdjacentHTML('beforeend',actions(c.messages.length-1));live.classList.remove('im-live');
                    live.querySelectorAll('.im-message-actions [data-action]').forEach(el=>el.onclick=()=>handle(el.dataset.action));
                    if(!state.interrupted)document.querySelector('.im-transcript').scrollTop=document.querySelector('.im-transcript').scrollHeight;
                }
                return;
            }
            state.raf=requestAnimationFrame(tick);
        }
        state.raf=requestAnimationFrame(tick);
    }
    function afterRender() {
        if(!active())return;
        const stage=document.querySelector('.im-stage:not(.im-departing)');if(!stage)return;
        const changed=state.lastPage!==null&&state.lastPage!==page;
        if(changed&&!motionReduced()&&!state.gestureReturn){
            const rootSwitch=rootPages.includes(page)&&rootPages.includes(state.lastPage);
            const back=state.navigation==='back';
            const old=state.outgoing;state.outgoing=null;
            if(old){
                old.classList.add('im-departing');old.style.zIndex=back?'11':'0';stage.after(old);const scroll=old.querySelector('.im-transcript');if(scroll)scroll.scrollTop=state.outgoingScroll;
                const anim=old.animate(rootSwitch?[{opacity:1},{opacity:0}]:[{transform:'translateX(0)'},{transform:`translateX(${back?'100%':'-22%'})`}],{duration:rootSwitch?130:300,easing:ease});
                anim.finished.finally(()=>old.remove());
            }
            stage.style.zIndex=back?'1':'10';
            stage.animate(rootSwitch?[{opacity:.7},{opacity:1}]:[{transform:`translateX(${back?'-22%':'100%'})`},{transform:'translateX(0)'}],{duration:rootSwitch?160:300,easing:ease}).finished.finally(()=>stage.style.zIndex='');
        }
        state.lastPage=page;state.navigation=null;state.gestureReturn=false;state.outgoing=null;
        const input=document.querySelector('#compose');if(input){
            const resize=()=>{const sc=document.querySelector('.im-transcript'),bottom=sc.scrollHeight-sc.scrollTop-sc.clientHeight<40;input.style.height='auto';input.style.height=Math.min(input.scrollHeight,128)+'px';if(bottom)sc.scrollTop=sc.scrollHeight;};
            resize();input.onkeydown=null;input.oninput=()=>{drafts[activeId]=input.value;resize();};
            document.querySelector('#im-composer').onsubmit=e=>{e.preventDefault();send();};
        }
        const search=document.querySelector('#im-search');if(search)search.oninput=()=>{query=search.value;const pos=search.selectionStart;render();const next=document.querySelector('#im-search');next.focus();next.setSelectionRange?.(pos,pos);};
        const find=document.querySelector('#im-find');if(find)find.oninput=()=>{document.querySelector('#im-find-count').textContent=`找到 ${current().messages.filter(m=>m.text.includes(find.value)&&find.value).length} 条消息`;};
        const transcript=document.querySelector('.im-transcript');
        if(transcript){transcript.scrollTop=state.scrolls[activeId]??transcript.scrollHeight;transcript.addEventListener('wheel',e=>{if(e.deltaY<0)state.interrupted=true;},{passive:true});transcript.addEventListener('touchmove',()=>state.interrupted=true,{passive:true});transcript.addEventListener('scroll',()=>{const atBottom=transcript.scrollHeight-transcript.scrollTop-transcript.clientHeight<40;if(atBottom){state.interrupted=false;state.newCount=0;document.querySelector('.im-jump').hidden=true;}});document.querySelector('.im-jump').onclick=()=>{state.interrupted=false;transcript.scrollTo({top:transcript.scrollHeight,behavior:reduced?'instant':'smooth'});document.querySelector('.im-jump').hidden=true;};
            let edge=null,settling=null;
            const under=document.querySelector('.im-backdrop');
            function position(x){stage.style.transform=`translateX(${x}px)`;if(under){under.style.visibility='visible';under.style.transform=`translateX(${-innerWidth*.22*(1-x/innerWidth)}px)`;}}
            function resetEdge(){stage.style.transform='';if(under){under.style.visibility='hidden';under.style.transform='';}edge=null;}
            stage.onpointerdown=e=>{if((e.clientX<24||settling?.playState==='running')&&!e.target.closest('button,input,textarea')){const base=Math.max(0,new DOMMatrix(getComputedStyle(stage).transform).m41);settling?.cancel();edge={x:e.clientX,y:e.clientY,time:performance.now(),base,last:base};stage.setPointerCapture(e.pointerId);}};
            stage.onpointermove=e=>{if(!edge)return;const x=Math.min(innerWidth,Math.max(0,edge.base+e.clientX-edge.x));if(Math.abs(e.clientY-edge.y)>30&&x<30){resetEdge();return;}edge.last=x;position(x);};
            function settle(complete){
                const x=edge.last;edge=null;
                settling=stage.animate([{transform:`translateX(${x}px)`},{transform:`translateX(${complete?innerWidth:0}px)`}],{duration:motionReduced()?0:Math.max(100,240*Math.abs((complete?innerWidth:0)-x)/innerWidth),easing:ease,fill:'forwards'});
                if(under)under.animate([{transform:under.style.transform},{transform:complete?'translateX(0)':`translateX(${-innerWidth*.22}px)`}],{duration:motionReduced()?0:180,easing:ease});
                settling.finished.then(()=>{if(complete){state.gestureReturn=true;go('conversations');}else{settling.cancel();resetEdge();}}).catch(()=>{});
            }
            stage.onpointerup=e=>{if(!edge)return;const dt=performance.now()-edge.time;settle(edge.last>innerWidth*.32||edge.last>55&&dt<240);};
            stage.onpointercancel=()=>{if(edge)settle(false);};
        }
        document.querySelectorAll('.im-message-content').forEach((el,i)=>{el.oncontextmenu=e=>{e.preventDefault();handle('im:message:'+i);};});
    }
    return {view,handle,afterRender,capture,bottomNav,active};
})();
