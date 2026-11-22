'use strict';
window.architectureModule = (() => {
    const media = window.matchMedia?.('(max-width: 700px)');
    let mode = media?.matches ? 'mobile' : 'desktop';
    const states = {
        mobile: {page: new URLSearchParams(location.search).get('demo')==='expert-mention'?'chat':'conversations', id: 'design', settings: ''},
        desktop: {page: 'workspace', id: 'design', settings: ''}
    };
    let sidebarVisible = true,
        inspectorWidth = 360,
        section = 'chat',
        extTab = 'experts',
        dataPane = 'memory',
        inspector = 'task',
        knowledgeTab = 'points',
        knowledgeEditing = false,
        showInspector = new URLSearchParams(location.search).get('models') !== 'empty',
        settingsOpen = false,
        settingsCategory = '',
        settingsScroll = 0;

    const btn = (t, a, cls = 'text-action') => `<button class="${cls}" data-action="${a}">${t}</button>`;
    const scrolls = {},
        panelScrolls = {},
        browserStates = {},
        knowledgeDrafts = {},
        knowledgeExpanded = {},
        recallQueries = {},
        memoryMode = {},
        retrievalOn = {},
        sourceEnabled = {};

    const sectionTitles = {
        extensions: '专家与技能',
        automations: '自动化中心',
        memory: '记忆与数据源'
    };

    function capture() {
        const panel = document.querySelector('.inspector-content');
        if (panel) panelScrolls[panel.dataset.panelKey] = panel.scrollTop;
        const settingsDetail = document.querySelector('.settings-window-detail');
        if (settingsDetail) settingsScroll = settingsDetail.scrollTop;
        if (window.expertRolesModule?.readInlineEditor) window.expertRolesModule.readInlineEditor();
        document.querySelectorAll('[data-knowledge-field]').forEach(el => {
            const owner = el.dataset.owner;
            (knowledgeDrafts[owner] || (knowledgeDrafts[owner] = {}))[el.dataset.knowledgeField] = el.value;
        });
        const knowledge = document.querySelector('[data-knowledge-owner]');
        if (knowledge) {
            knowledgeExpanded[knowledge.dataset.knowledgeOwner] = new Set([...knowledge.querySelectorAll('details[open]')].map(el => el.dataset.knowledgePane));
        }
        const el = document.querySelector('.studio-center');
        if (el) {
            scrolls[el.dataset.context + ':' + el.dataset.tabkey] = {
                main: el.scrollTop,
                detail: el.querySelector('.studio-document,.page')?.scrollTop || 0,
                chat: el.querySelector('#transcript')?.scrollTop || 0
            };
            const input = el.querySelector('#compose');
            if (input) drafts[el.dataset.context] = input.value;
        }
    }

    function tabbar() {
        return `<header class="main-tabbar app-topbar" aria-label="应用顶栏"><div class="topbar-left"><button class="strip-entry" aria-label="显示或隐藏侧栏" title="显示或隐藏侧栏" aria-pressed="${sidebarVisible}" data-action="dk-sidebar">${svg('sidebar')}</button><button class="strip-entry" aria-label="新对话" title="新对话" data-action="new">${svg('edit')}</button></div><div class="topbar-title">${section === 'chat' ? `<button type="button" class="topbar-chat-title" data-action="rename" title="点击修改对话标题">${esc(current().title)}</button>` : `<span class="topbar-section-title">${esc(sectionTitles[section] || 'Nola')}</span>`}</div>${section === 'chat' ? `<div class="topbar-right"><button class="topbar-add" aria-label="搜索对话" title="搜索对话" data-action="conversation-search">${svg('search')}</button>${sidebarNavigation.tools(showInspector ? inspector : '')}</div>` : ''}</header>`;
    }

    function usageCenterView() {
        return `<section class="page"><header class="page-head"><div><h1>使用记录</h1><p>查看本页原型会话的本地演示记录。</p></div></header><div class="objects">${conversations.slice(0, 5).map(c => `<button class="object-row" data-action="dk-select:${esc(c.id)}"><div class="object-copy"><h3>${esc(c.title)}</h3><p>${c.messages.length} 条消息 · ${esc(c.model || defaultModel)}</p></div><span class="object-meta">${esc(c.time)}</span>${svg('chevron')}</button>`).join('')}</div><div class="demo-banner">演示统计只记录消息与模型名称，不展示或校验内部摘要哈希。</div></section>`;
    }

    function extensionsHub() {
        const tabs = [['experts', '专家'], ['skills', '技能'], ['mcp', 'MCP'], ['plugins', '插件']];
        return `<section class="studio-document center-hub" aria-label="专家与技能"><header class="hub-header"><div class="tabs" role="tablist" aria-label="专家与技能分类">${tabs.map(([id, title]) => `<button type="button" role="tab" aria-selected="${extTab === id}" class="${extTab === id ? 'selected' : ''}" data-action="dk-ext-tab:${id}">${title}</button>`).join('')}</div></header><div class="hub-body">${extTab === 'experts' ? window.expertRolesModule.embeddedView() : window.manageModule.capabilitiesForKind(extTab)}</div></section>`;
    }

    function memoryHub() {
        const tabs = [['memory', '记忆'], ['knowledge', '知识'], ['sources', '来源'], ['usage', '使用记录']];
        return `<section class="prototype-data-center studio-document center-hub" aria-label="记忆与数据源"><header class="hub-header"><div class="tabs" role="tablist" aria-label="数据中心分类">${tabs.map(([id, title]) => `<button type="button" role="tab" aria-selected="${dataPane === id}" class="${dataPane === id ? 'selected' : ''}" data-action="dk-data-pane:${id}">${title}</button>`).join('')}</div></header><div class="hub-body">${dataPane === 'memory' ? workspaceModule.view('memory') : dataPane === 'knowledge' ? workspaceModule.view('workspace') : dataPane === 'sources' ? enhancedModule.view('data') : usageCenterView()}</div></section>`;
    }

    function content() {
        if (section === 'extensions') return extensionsHub();
        if (section === 'automations') return `<section class="studio-document center-hub" aria-label="自动化中心">${plansView()}</section>`;
        if (section === 'memory') return memoryHub();
        return chatView();
    }

    function record() {
        return workspaceModule.records.find(r => r.id === activeId);
    }

    function recallMatchesHTML(c, r, q) {
        const needle = (q || '').trim().toLowerCase();
        const rawItems = [
            ...(r?.history || []),
            ...c.messages.map(m => `${m.role === 'user' ? '你' : 'Nola'}：${m.text}`)
        ];
        const matches = rawItems.filter(text => !needle || text.toLowerCase().includes(needle));
        return matches.length
            ? matches.slice(0, 6).map(text => `<p class="recall-hit">${esc(text)}</p>`).join('')
            : '<p class="meta">没有匹配的原始消息。</p>';
    }

    function knowledgeEditors(r, c) {
        const owner = c.id,
            draftsMap = knowledgeDrafts[owner] || {},
            hasSummary = !!(r?.summary && r.summary !== '尚无压缩摘要。');
        const activeField = knowledgeTab === 'wiki' ? 'wiki' : 'keyPointsMarkdown';
        const activeText = draftsMap[activeField] ?? r?.[activeField] ?? '';
        const tabs = [['points', '知识点'], ['wiki', 'Wiki'], ['recall', '回忆']];
        let paneHTML = '';
        if (knowledgeTab === 'recall') {
            paneHTML = `<input id="knowledge-${encodeURIComponent(owner)}-recall" type="search" data-recall-owner="${esc(owner)}" placeholder="搜索本会话原始消息" value="${esc(recallQueries[owner] || '')}"><div class="recall-list">${recallMatchesHTML(c, r, recallQueries[owner] || '')}</div>`;
        } else if (knowledgeEditing) {
            paneHTML = `<textarea id="knowledge-${encodeURIComponent(owner)}-${activeField}" data-knowledge-field="${activeField}" data-owner="${esc(owner)}" rows="5" placeholder="${knowledgeTab === 'wiki' ? '用 Markdown 编写会话 Wiki…' : '每行添加一条知识点…'}">${esc(activeText)}</textarea><div class="action-wrap" style="margin-top:8px"><button class="primary" data-action="dk-knowledge-save:${activeField}">保存</button><button class="text-action" data-action="dk-knowledge-edit:off">取消</button></div>`;
        } else {
            paneHTML = `${activeText ? `<div class="prose" style="font-size:13px;line-height:1.6">${esc(activeText).split('\n\n').map(p => `<p>${p.replace(/\n/g, '<br>')}</p>`).join('')}</div>` : '<p class="meta">还没有整理内容，点击编辑开始记录。</p>'}<div class="action-wrap" style="margin-top:8px"><button class="text-action" data-action="dk-knowledge-edit:on">编辑${knowledgeTab === 'wiki' ? ' Wiki' : '知识点'}</button></div>`;
        }
        return `<div class="inspector-knowledge" data-knowledge-owner="${esc(owner)}"><div class="inspector-row-head"><h3>会话知识</h3><span class="meta">${c.messages.length} 条消息 · ${hasSummary ? 1 : 0} 份摘要</span></div><div class="segment" role="tablist" aria-label="会话知识切换" style="margin:8px 0 10px">${tabs.map(([id, label]) => `<button type="button" role="tab" aria-selected="${knowledgeTab === id}" class="${knowledgeTab === id ? 'selected' : ''}" data-action="dk-knowledge-tab:${id}">${label}</button>`).join('')}</div>${paneHTML}</div>`;
    }

    function contextPane() {
        const r = record(), c = current();
        let body = '';
        if (inspector === 'task') {
            body = `<section class="inspector-block" aria-label="任务与进度">${organizerModule.panel('进度')}<details class="local-tech-details" style="margin-top:12px"><summary>阶段目标与安排</summary><div style="margin-top:8px">${organizerModule.panel('目标')}</div></details></section>${c.experts.length ? `<hr class="inspector-divider"><section class="inspector-block" aria-label="协作参与者"><div class="inspector-row-head"><h3>协作参与者</h3>${btn('调整', 'experts')}</div><p class="muted" style="margin-top:4px;font-size:12px">Nola · ${esc(c.experts.join(' · '))}</p></section>` : ''}${window.expertHistoryModule?.rows(c.id) ? `<hr class="inspector-divider"><section class="inspector-block" aria-label="专家会话">${window.expertHistoryModule.rows(c.id)}</section>` : ''}`;
        } else if (inspector === 'artifacts') {
            body = `<section class="inspector-block" aria-label="产物与资料">${deliverablesModule.panel('产物')}<div class="inspector-subsection"><div class="inspector-row-head"><h3>会话资料</h3>${btn('添加引用', 'attach')}</div>${documents.filter(d => !d.sourceId || d.sourceId === c.id).slice(0, 3).map(d => btn(esc(d.name), 'ex-source:' + documents.indexOf(d), 'desk-resource')).join('')}</div></section><hr class="inspector-divider"><section class="inspector-block" aria-label="工作区变更">${deliverablesModule.panel('审阅')}</section>`;
        } else {
            const mem = memoryMode[c.id] || '默认', ret = retrievalOn[c.id] !== false;
            const dsList = documents.map((d, idx) => ({d, idx})).filter(x => x.d.type === '数据源');
            body = `<section class="inspector-block" aria-label="上下文用量"><div class="context-usage"><strong>上下文窗口</strong><span>约 12% · 128k 上限</span></div><div class="local-progress-track" style="margin:6px 0 0"><div class="local-progress-fill" style="width:12%"></div></div></section><hr class="inspector-divider"><section class="inspector-block" aria-label="会话知识">${knowledgeEditors(r, c)}</section><hr class="inspector-divider"><section class="inspector-block" aria-label="记忆与数据源"><div class="inspector-kv"><label for="ctx-memory-${esc(c.id)}">长期记忆</label><select id="ctx-memory-${esc(c.id)}" data-action-change="dk-memory-mode">${['默认', '启用', '停用'].map(m => `<option ${mem === m ? 'selected' : ''}>${m}</option>`).join('')}</select></div><div class="inspector-kv"><span>使用检索</span><button role="switch" class="toggle" aria-label="在此对话中使用检索" aria-checked="${ret}" data-action="dk-retrieval-toggle"></button></div>${dsList.map(({d, idx}) => `<div class="inspector-kv"><span>${esc(d.name)}</span><button role="switch" class="toggle" aria-label="在此对话中启用${esc(d.name)}" aria-checked="${sourceEnabled[c.id + ':' + idx] !== false}" data-action="dk-source-toggle:${idx}"></button></div>`).join('')}<div style="margin-top:8px">${btn('管理数据源…', 'dk-open-sources')}</div></section>${!browserStates[activeId] ? `<hr class="inspector-divider"><section class="inspector-block inspector-browser-setup" aria-label="托管浏览器"><div class="inspector-row-head"><h3>托管浏览器</h3><span class="badge">未连接</span></div><p class="muted" style="font-size:12px;margin:4px 0 8px">Chrome 扩展尚未连接，连接后可直接浏览网页上下文。</p>${btn('连接扩展', 'dk-browser')}</section>` : ''}<hr class="inspector-divider"><details class="context-diagnostics"><summary>诊断详情</summary><p class="meta" style="margin-top:6px">缓存命中率：未上报 · 请求 #1 · 演示模式不记录内部摘要哈希。</p></details>`;
        }
        return `<aside class="studio-inspector" aria-label="当前工作上下文" style="--inspector-width:${inspectorWidth}px"><div class="inspector-resize" role="separator" tabindex="0" aria-label="调整工作面板宽度" aria-orientation="vertical" aria-valuemin="320" aria-valuemax="480" aria-valuenow="${inspectorWidth}"></div>${window.sidebarNavigation.right(inspector)}<div class="inspector-content" data-panel-key="${activeId}:${inspector}" data-inspector-area="${inspector}">${body}<p class="desk-footnote">本地演示 · 与真实执行、文件和仓库无连接</p></div></aside>`;
    }

    function desktop() {
        const visible = section === 'chat' && showInspector;
        return `<section class="studio ${visible ? 'has-inspector' : ''}"><div class="studio-body"><div class="studio-center" id="main-content" data-context="${activeId}" data-tabkey="${section}" data-canvas-kind="${section}">${content()}</div>${visible ? contextPane() : ''}</div></section>`;
    }

    function stash() {
        states[mode] = {page, id: activeId, settings: settingsSection, query, searchScroll};
    }

    function activate(next) {
        if (next === mode) return;
        saveDraft();
        stash();
        if (dialog.open) closeDialog();
        mode = next;
        const s = states[mode];
        page = s.page;
        activeId = conversations.some(c => c.id === s.id) ? s.id : 'design';
        settingsSection = s.settings;
        query = s.query || '';
        searchScroll = s.searchScroll || 0;
        selectedExperts = [...(current()?.experts || [])];
        document.documentElement.dataset.experience = mode;
        history.replaceState({page, id: activeId, mode}, '', `#${mode}/${page}`);
        render();
    }

    function start() {
        mode = media?.matches ? 'mobile' : 'desktop';
        const s = states[mode];
        page = s.page;
        activeId = s.id;
        selectedExperts = [...current().experts];
        document.documentElement.dataset.experience = mode;
        history.replaceState({page, id: activeId, mode}, '', `#${mode}/${page}`);
        media?.addEventListener('change', e => activate(e.matches ? 'mobile' : 'desktop'));
        render();
    }

    function openSection(nextSection, subTab) {
        capture();
        section = ['chat', 'extensions', 'automations', 'memory'].includes(nextSection) ? nextSection : 'chat';
        if (section === 'extensions' && subTab) extTab = subTab;
        if (section === 'memory' && subTab) dataPane = subTab;
        settingsOpen = false;
        page = 'workspace';
        render();
    }

    function toggleInspector(area, forceOpen = false) {
        capture();
        if (dialog.open) closeDialog();
        const target = ['task', 'artifacts', 'context'].includes(area) ? area : 'task';
        const switchingFromOtherSection = section !== 'chat';
        section = 'chat';
        settingsOpen = false;
        page = 'workspace';
        if (!forceOpen && !switchingFromOtherSection && showInspector && inspector === target) {
            showInspector = false;
        } else {
            showInspector = true;
            inspector = target;
        }
        render();
    }

    function openSettings(category = '') {
        if (mode !== 'desktop') {navigate('settings'); return;}
        capture();
        settingsOpen = true;
        settingsCategory = category;
        settingsScroll = 0;
        render();
    }

    function closeSettings() {
        settingsOpen = false;
        render();
    }

    function view(p) {
        if (mode === 'mobile' && p === 'workspace') return window.assistantModule?.view(p);
        if (mode === 'desktop' && ['workspace', 'chat'].includes(p)) return desktop();
        return null;
    }

    function handle(a) {
        if (a === 'dk-sidebar') {sidebarVisible = !sidebarVisible; render(); return true;}
        if (a === 'dk-settings-close') {closeSettings(); return true;}
        if (!a.startsWith('dk-')) return false;
        if (a === 'dk-workspace') {
            capture();
            const cur = current().workspace || '临时工作区';
            openDialog('工作环境', `<p class="dialog-note">选择当前对话的演示工作目录；不授予或读取真实系统目录。</p><button class="choice-row" data-action="dk-workspace-set:临时工作区"><div><h3>临时工作区</h3><p>不绑定本地项目文件夹，保持轻量隔离</p></div>${cur === '临时工作区' ? svg('check') : ''}</button><button class="choice-row" data-action="dk-workspace-set:演示项目"><div><h3>选择文件夹…（演示项目）</h3><p>关联示例工程目录并显示变更审阅入口</p></div>${cur === '演示项目' ? svg('check') : ''}</button><div class="action-wrap">${btn('在 Finder 中打开（演示）', 'dk-workspace-finder')}${btn('审阅工作区更改', 'rv-open-review')}${cur !== '临时工作区' ? btn('断开文件夹连接', 'dk-workspace-set:临时工作区') : ''}</div>`);
            return true;
        }
        if (a === 'dk-workspace-finder') {toast('演示模式不打开 Finder 或读取本地目录'); return true;}
        if (a.startsWith('dk-workspace-set:')) {current().workspace = a.split(':')[1]; closeDialog(); render(); return true;}
        if (a === 'dk-browser') {browserStates[activeId] = !browserStates[activeId]; render(); toast(browserStates[activeId] ? 'Chrome 演示扩展已连接，提示已收起' : '演示扩展已断开'); return true;}
        if (a === 'dk-retrieval-toggle') {capture(); retrievalOn[activeId] = !(retrievalOn[activeId] !== false); render(); return true;}
        if (a.startsWith('dk-source-toggle:')) {capture(); const idx = a.split(':')[1]; const key = activeId + ':' + idx; sourceEnabled[key] = !(sourceEnabled[key] !== false); render(); return true;}
        if (a === 'dk-open-sources') {openSection('memory', 'sources'); return true;}
        if (a.startsWith('dk-ext-tab:')) {extTab = a.split(':')[1]; render(); return true;}
        if (a.startsWith('dk-data-pane:')) {dataPane = a.split(':')[1]; render(); return true;}
        if (a.startsWith('dk-knowledge-tab:')) {knowledgeTab = a.split(':')[1]; knowledgeEditing = false; render(); return true;}
        if (a.startsWith('dk-knowledge-edit:')) {knowledgeEditing = a.split(':')[1] === 'on'; render(); return true;}
        const [key, v] = a.split(':');
        saveDraft();
        if (key === 'dk-area' || key === 'dk-panel') {
            const area = ({任务: 'task', 产物: 'artifacts', 上下文: 'context', 进度: 'task', 目标: 'task', 审阅: 'artifacts', task: 'task', artifacts: 'artifacts', context: 'context'})[v] || 'task';
            toggleInspector(area, true);
            return true;
        }
        if (key === 'dk-knowledge-save' && ['keyPointsMarkdown', 'wiki'].includes(v)) {
            capture();
            const r = record(), draft = knowledgeDrafts[activeId]?.[v];
            if (r && draft !== undefined) {
                r[v] = draft;
                r.version = (r.version || 1) + 1;
                knowledgeEditing = false;
                render();
                toast('已保存本页演示稿');
            }
            return true;
        }
        if (key === 'dk-inspector') {showInspector = !showInspector; render(); return true;}
        if (key === 'dk-select') {
            if (dialog.open) closeDialog();
            capture();
            activeId = v;
            section = 'chat';
            settingsOpen = false;
            selectedExperts = [...current().experts];
            navigate('workspace', v);
            return true;
        }
        if (key === 'dk-wiki') {toggleInspector('context', true); return true;}
        return true;
    }

    function afterRender() {
        const panel = document.querySelector('.inspector-content');
        if (panel) panel.scrollTop = panelScrolls[panel.dataset.panelKey] || 0;
        const settingsDetail = document.querySelector('.settings-window-detail');
        if (settingsDetail) settingsDetail.scrollTop = settingsScroll || 0;
        document.querySelector('.shell')?.classList.toggle('sidebar-collapsed', !sidebarVisible);
        const el = document.querySelector('.studio-center');
        if (el) {
            const saved = scrolls[activeId + ':' + section];
            el.scrollTop = saved?.main || 0;
            const detail = el.querySelector('.studio-document,.page');
            if (detail) detail.scrollTop = saved?.detail || 0;
            const transcript = el.querySelector('#transcript');
            if (transcript) transcript.scrollTop = saved?.chat || 0;
        }
        const memSelect = document.querySelector('[data-action-change="dk-memory-mode"]');
        if (memSelect) {
            memSelect.onchange = () => {
                memoryMode[activeId] = memSelect.value;
                toast('已切换本会话记忆模式：' + memSelect.value);
            };
        }
        const recallInput = document.querySelector('[data-recall-owner]');
        if (recallInput) {
            recallInput.oninput = () => {
                const owner = recallInput.dataset.recallOwner;
                recallQueries[owner] = recallInput.value;
                const listEl = recallInput.parentElement?.querySelector('.recall-list');
                if (listEl) listEl.innerHTML = recallMatchesHTML(current(), record(), recallQueries[owner]);
            };
        }
        window.deliveryModule?.bind?.(document);
        const backdrop = document.querySelector('[data-settings-backdrop]');
        if (backdrop) {
            backdrop.onclick = e => {if (e.target === backdrop) closeSettings();};
        }
        const grip = document.querySelector('.inspector-resize');
        if (!grip) return;
        const apply = w => {
            inspectorWidth = Math.max(320, Math.min(480, w));
            const pane = document.querySelector('.studio-inspector');
            pane?.style.setProperty('--inspector-width', inspectorWidth + 'px');
            grip.setAttribute('aria-valuenow', inspectorWidth);
        };
        grip.onkeydown = e => {if (['ArrowLeft', 'ArrowRight'].includes(e.key)) {e.preventDefault(); apply(inspectorWidth + (e.key === 'ArrowLeft' ? 20 : -20));}};
        grip.onpointerdown = e => {
            const x = e.clientX, w = inspectorWidth;
            grip.setPointerCapture(e.pointerId);
            grip.onpointermove = ev => apply(w + x - ev.clientX);
            grip.onpointerup = () => {grip.onpointermove = null;};
        };
    }

    function restore(e) {
        saveDraft();
        if (dialog.open) closeDialog();
        const s = e.state?.mode === mode ? e.state : states[mode];
        page = s.page || 'workspace';
        activeId = conversations.some(c => c.id === s.id) ? s.id : 'design';
        settingsSection = s.settings || '';
        query = s.query || '';
        searchScroll = s.searchScroll || 0;
        selectedExperts = [...(current()?.experts || [])];
        render();
        if (page === 'conversations') window.scrollTo(0, searchScroll);
        stash();
    }

    function nav() {
        stash();
        history.replaceState({page, id: activeId, mode}, '', `#${mode}/${page}${page === 'chat' ? '/' + activeId : ''}`);
    }

    document.addEventListener('keydown', e => {
        if (mode !== 'desktop') return;
        if (e.key === 'Escape' && settingsOpen && !dialog.open) {e.preventDefault(); closeSettings(); return;}
        if ((e.metaKey || e.ctrlKey) && e.key === ',' && !dialog.open) {e.preventDefault(); openSettings(''); return;}
        if (!['workspace', 'chat'].includes(page) || dialog.open || settingsOpen) return;
        if ((e.metaKey || e.ctrlKey) && ['1', '2', '3'].includes(e.key)) {
            e.preventDefault();
            saveDraft();
            if (e.key === '1') openSection('chat');
            else toggleInspector(e.key === '2' ? 'artifacts' : 'task', true);
        }
    });

    return {
        topbar: () => mode === 'desktop' ? tabbar() : '',
        settingsOverlay: () => mode === 'desktop' && settingsOpen ? window.enhancedModule.settingsWindowHTML(settingsCategory) : '',
        capture,
        openSection,
        syncExtTab: t => {extTab = t;},
        toggleInspector,
        openSettings,
        closeSettings,
        openRoute: route => {
            if (route === 'settings') {openSettings(''); return;}
            if (['capabilities', 'skills', 'mcp'].includes(route)) {openSection('extensions', route === 'mcp' ? 'mcp' : 'skills'); return;}
            if (['plans', 'automations'].includes(route)) {openSection('automations'); return;}
            if (['data', 'memory'].includes(route)) {openSection('memory', route === 'data' ? 'sources' : 'memory'); return;}
            if (route === 'models') {openSettings('providers'); return;}
            openSection('chat');
        },
        view,
        handle,
        afterRender,
        start,
        nav,
        restore,
        activate,
        beforeNavigate: p => {if (p === 'chat') {section = 'chat'; settingsOpen = false;}},
        getMode: () => mode,
        snapshot: () => ({mode, states, section, extTab, dataPane, inspector, showInspector, inspectorWidth, sidebarVisible, settingsOpen, settingsCategory})
    };
})();
