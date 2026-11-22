'use strict';
window.enhancedModule = (() => {
    let preferences = {notifications: true, retrieval: true, memory: true, startup: false, debugLog: false, shareMac: false, sharePort: '8765', remoteUrl: '', agent: '每次确认'},
        runs = [{name: '每周留一点时间复盘', state: '已完成', time: '周日 20:00', result: '整理了本周 3 个讨论要点。此为演示运行结果。'}];
    const button = (t, a, c = 'text-action') => `<button class="${c}" data-action="${a}">${t}</button>`;
    const routes = [
        ['data', '记忆与数据源', '长期记忆、资料连接与访问策略'],
        ['plans', '自动化中心', '周期与运行历史'],
        ['activity', '活动与结果', 'Agent、通知与需要回应的事项'],
        ['models', '模型与提供方', '本地与 API 配置'],
        ['capabilities', '专家与技能', '角色目录、Skills 与 MCP']
    ];

    function management() {
        return `<section class="page">${crumbs('管理')}${pageHead('管理', '能力放在这里，要点留在前面。')}<div class="objects">${routes.map(([p, t, s]) => `<button class="object-row" data-action="nav:${p}"><div class="object-copy"><h3>${t}</h3><p>${s}</p></div>${svg('chevron')}</button>`).join('')}</div></section>`;
    }

    function data() {
        return `<section class="page">${crumbs('来源与资料')}${pageHead('来源与资料', '从内容到来源，保留可追溯的关系。', 'ex-source-new', '添加')}<div class="tabs">${['全部', '文档', '笔记', '链接', '数据源'].map(t => button(t, 'data-filter:' + t, t === dataFilter ? 'selected' : '')).join('')}</div><div class="objects">${documents.filter(d => dataFilter === '全部' || d.type === dataFilter).map(d => {const i = documents.indexOf(d); return `<button class="object-row" data-action="ex-source:${i}"><div class="object-copy"><h3>${esc(d.name)}</h3><p>${esc(d.type)} · ${esc(d.policy || '每次确认')} · ${esc(d.health || '待检查')}</p><p class="meta">${esc(d.info)}</p></div>${svg('chevron')}</button>`;}).join('') || '<div class="empty-state"><h2>暂时没有这类资料</h2><p>添加一份内容，或从对话中保存。</p></div>'}</div><p class="demo-banner">连接、测试与权限修改仅在本页模拟，不读取文件或访问远程服务。</p></section>`;
    }

    function sourceForm(i) {
        const d = documents[i] || {};
        openDialog(i === undefined ? '添加资料或数据源' : '编辑资料或数据源', `<form id="source-form"><label>名称<input name="name" value="${esc(d.name || '')}" required maxlength="100"></label><label>类型<select name="type">${['文档', '笔记', '链接', '数据源'].map(t => `<option ${t === d.type ? 'selected' : ''}>${t}</option>`).join('')}</select></label><label>来源 / 连接位置<input name="location" value="${esc(d.location || '')}" placeholder="示例位置，请勿填写凭据"></label><label>内容摘要<textarea name="content">${esc(d.content || '')}</textarea></label><label>访问策略<select name="policy">${['每次确认', '仅本对话可用', '禁止访问'].map(t => `<option ${t === d.policy ? 'selected' : ''}>${t}</option>`).join('')}</select></label><p class="dialog-note">所有值仅用于演示，不能填写真实密钥。</p><div class="dialog-actions">${button('取消', 'close')}<button class="primary" type="submit">保存演示项</button></div></form>`);
        dialog.querySelector('form').onsubmit = e => {
            e.preventDefault();
            const f = new FormData(e.target), n = String(f.get('name')).trim();
            if (!n) return;
            const obj = {...d, name: n, type: f.get('type'), location: f.get('location'), content: f.get('content'), policy: f.get('policy'), info: '刚刚 · 本地演示', size: '演示', health: '待检查'};
            if (i === undefined) documents.unshift(obj); else documents[i] = obj;
            closeDialog();
            render();
            toast('已保存演示配置');
        };
    }

    function source(i) {
        const d = documents[i];
        openDialog(d.name, `<p class="prose">${esc(d.content || ({文档: '界面以中性色调为底，保证阅读与操作的一致性。', 笔记: '比较体验时，要同时关注速度、稳定性与可恢复性。', 链接: '设计参考链接，用于讨论信息层级与阅读节奏。'}[d.type] || '演示数据源，未连接任何真实内容。'))}</p><dl><div><dt>来源</dt><dd>${esc(d.location || d.info)}</dd></div><div><dt>访问</dt><dd>${esc(d.policy || '每次确认')}</dd></div><div><dt>状态</dt><dd>${esc(d.health || '待检查')}</dd></div></dl><div class="action-wrap">${button('在对话中使用', 'discuss-document:' + i)}${d.sourceId ? button('回到来源对话', 'ex-source-chat:' + i) : ''}${button('编辑', 'ex-source-edit:' + i)}${button('测试连接', 'ex-source-test:' + i)}${button('删除', 'ex-source-delete:' + i)}</div>`);
    }

    function model(i) {
        const m = models[i];
        openDialog(m.name, `<dl><div><dt>方式</dt><dd>${esc(m.provider)}</dd></div><div><dt>端点</dt><dd>${esc(m.endpoint || '未配置 · 演示')}</dd></div><div><dt>状态</dt><dd>${esc(m.state)}</dd></div></dl><div class="action-wrap">${button('配置', 'ex-model-edit:' + i)}${button('测试连接', 'ex-model-test:' + i)}${button('设为默认', 'default-model:' + i)}${button('删除', 'ex-model-delete:' + i)}</div><p class="dialog-note">不会访问端点或调用模型。连接测试模拟失败及修复过程。</p>`);
    }

    function modelForm(i) {
        const m = models[i] || {};
        openDialog(i === undefined ? '添加模型' : '配置模型', `<form id="model-form"><label>名称<input name="name" value="${esc(m.name || '')}" required></label><label>提供方式<select name="provider"><option ${m.provider === '本地模型' ? 'selected' : ''}>本地模型</option><option ${m.provider === 'API 连接' ? 'selected' : ''}>API 连接</option></select></label><label>模型标识<input name="modelId" value="${esc(m.modelId || '')}" placeholder="例如 demo-model"></label><label>演示端点<input name="endpoint" value="${esc(m.endpoint || '')}" placeholder="https://example.invalid/v1"></label><p class="dialog-note">密钥不在原型中收集。保存仅创建本地配置；首次测试模拟失败，再次测试模拟恢复。</p><div class="dialog-actions">${button('取消', 'close')}<button class="primary" type="submit">保存</button></div></form>`);
        dialog.querySelector('form').onsubmit = e => {
            e.preventDefault();
            const f = new FormData(e.target), name = String(f.get('name')).trim();
            if (!name) return;
            const n = {...m, name, provider: f.get('provider'), modelId: f.get('modelId'), endpoint: f.get('endpoint'), info: '自定义演示配置', state: '待测试', attempts: 0};
            if (i === undefined) models.push(n);
            else {
                if (defaultModel === m.name) defaultModel = name;
                models[i] = n;
            }
            closeDialog();
            render();
        };
    }

    function plan(i) {
        const p = plans[i];
        openDialog(p.name, `<p class="prose">${esc(p.prompt || '整理这一阶段的重要讨论，并保留来源。')}</p><dl><div><dt>周期</dt><dd>${esc(p.time)}</dd></div><div><dt>状态</dt><dd>${p.on ? '已开启' : '已暂停'}</dd></div></dl><div class="action-wrap">${button('编辑', 'ex-plan-edit:' + i)}${button(p.on ? '暂停' : '开启', 'plan-switch:' + i)}${button('演示运行一次', 'ex-plan-run:' + i)}${button('运行历史', 'ex-plan-history:' + i)}${button('删除', 'ex-plan-delete:' + i)}</div><p class="dialog-note">不创建真实定时任务。运行仅生成本页演示记录。</p>`);
    }

    function planForm(i) {
        const p = plans[i] || {};
        openDialog(i === undefined ? '新建计划' : '编辑计划', `<form id="plan-form"><label>名称<input name="name" value="${esc(p.name || '')}" required></label><label>要做什么<textarea name="prompt" required>${esc(p.prompt || '整理重要讨论，并给出来源。')}</textarea></label><label>周期<select name="period"><option>每天</option><option selected>每周日</option><option>每周五</option></select></label><label>时间<input type="time" name="time" value="${esc((p.time || '20:00').slice(-5))}" required></label><div class="dialog-actions">${button('取消', 'close')}<button type="submit" class="primary">保存演示计划</button></div></form>`);
        dialog.querySelector('form').onsubmit = e => {
            e.preventDefault();
            const f = new FormData(e.target), name = String(f.get('name')).trim();
            if (!name) return;
            const time = f.get('period') + ' ' + f.get('time'), obj = {...p, name, prompt: f.get('prompt'), time, next: time, on: p.on ?? true};
            if (i === undefined) plans.push(obj); else plans[i] = obj;
            closeDialog();
            render();
        };
    }

    const desktopSettingsGroups = [
        {group: '通用', items: [['general', '通用', '外观、通知、软件更新与启动']]},
        {group: '模型', items: [['providers', '模型服务', 'API 服务商、默认对话模型与本地模型'], ['agent', '智能体', '对话默认记忆、检索与上下文']]},
        {group: '工作区管理', items: [['connections', '连接', '共享这台 Mac 与远端服务器'], ['data', '数据', '记忆与数据源、导出与系统访问']]},
        {group: '系统', items: [['diagnostics', '诊断', '调试日志与本地状态检查']]}
    ];

    function settingsWindowHTML(category) {
        let detailHTML = '';
        if (!category) {
            detailHTML = `<div class="settings-landing"><h1>设置</h1><p class="muted" style="margin:8px 0 18px">从左侧选择分类以查看外观、软件更新、模型服务、智能体默认值、共享与远端连接、系统访问权限或本地诊断。所有设置均保留在独立窗口中，与日常会话分开。</p><dl><div><dt>当前默认模型</dt><dd>${esc(defaultModel || '未配置')}</dd></div><div><dt>外观与更新</dt><dd>${theme === 'light' ? '浅色主题' : '深色主题'} · Nola 1.0.0（演示）</dd></div><div><dt>运行模式</dt><dd>本地交互原型（不修改系统配置或磁盘文件）</dd></div></dl><details class="prototype-state-controls"><summary>原型预览</summary><p><a href="scenarios.html">全业务场景与状态总览 ↗</a></p>${button('关于与演示控制', 'ex-settings:prototype')}</details></div>`;
        } else if (category === 'general') {
            detailHTML = `<div class="prototype-general"><h1>通用</h1><p class="muted" style="margin:6px 0 18px">外观、通知与这台设备上的偏好。</p><section class="inline-card"><h2>外观</h2><div class="settings-row"><span>主题</span><div class="segment">${[['system', '跟随系统'], ['light', '浅色'], ['dark', '深色']].map(([mode, label]) => `<button type="button" aria-pressed="${theme === mode}" class="${theme === mode ? 'selected' : ''}" data-action="set-theme:${mode === 'system' ? (matchMedia('(prefers-color-scheme:dark)').matches ? 'dark' : 'light') : mode}">${label}</button>`).join('')}</div></div></section><section class="inline-card"><h2>通知</h2><p>任务结束或需要回应时提醒，不包含会话正文。</p><div class="settings-row"><span>显示演示活动提醒</span><button role="switch" class="toggle" aria-label="显示演示活动提醒" aria-checked="${preferences.notifications}" data-action="ex-pref:notifications:general"></button></div></section><section class="inline-card"><h2>软件更新</h2>${window.updatePreview.content()}</section><section class="inline-card"><h2>启动</h2><div class="settings-row"><span>登录时启动 Nola</span><button role="switch" class="toggle" aria-label="登录时启动 Nola（演示）" aria-checked="${!!preferences.startup}" data-action="ex-pref:startup:general"></button></div></section></div>`;
        } else if (category === 'providers') {
            detailHTML = `<div class="settings-detail-inner">${modelsView()}</div>`;
        } else if (category === 'agent') {
            detailHTML = `<div class="settings-detail-inner"><h1>智能体</h1><p class="muted" style="margin:6px 0 18px">新对话的记忆、检索与上下文默认值。</p><section class="inline-card" style="padding:18px;margin-bottom:18px"><div class="settings-row"><div><h3>在对话中使用记忆</h3><p>按会话声明提供已确认的个人记忆。</p></div><button role="switch" class="toggle" aria-label="在对话中使用记忆" aria-checked="${preferences.memory}" data-action="ex-pref:memory:agent"></button></div><div class="settings-row" style="border-bottom:0"><div><h3>默认检索授权资料</h3><p>开启后，新会话可在授权范围内检索关联数据源。</p></div><button role="switch" class="toggle" aria-label="默认检索授权资料" aria-checked="${preferences.retrieval}" data-action="ex-pref:retrieval:agent"></button></div></section></div>`;
        } else if (category === 'connections') {
            detailHTML = `<div class="settings-detail-inner"><h1>连接</h1><section class="inline-card" style="padding:18px;margin-bottom:18px"><h2>共享为 Nola 服务器</h2><p>开启服务后，可以把连接信息交给手机或其他设备。</p><a class="primary" href="scenarios.html?scene=share-host&amp;platform=mac">共享设置</a></section><section class="inline-card" style="padding:18px"><h2>连接到 Nola 服务器</h2><p>粘贴另一台设备提供的连接信息，或手动填写地址与令牌。</p><a href="scenarios.html?scene=host-pair&amp;platform=mac">连接设置</a></section></div>`;
        } else if (category === 'data') {
            detailHTML = `<div class="settings-detail-inner"><h1>隐私与系统访问</h1><p class="muted" style="margin:6px 0 18px">管理本机隐私边界与系统级访问权限（记忆与数据源请直接从主侧栏进入）。</p><div class="objects" style="margin-bottom:20px"><button class="object-row" data-action="ex-settings:privacy"><div class="object-copy"><h3>隐私与存储边界说明</h3><p>查看本机存储与演示模式说明</p></div>${svg('chevron')}</button></div><h2 style="font-size:15px;margin-bottom:10px">系统访问</h2><div class="objects">${manageModule.snapshot().capabilities.filter(c => c.type === '系统访问').map(c => `<div class="object-row object-row-with-switch"><button type="button" class="object-copy" style="text-align:left;padding:0" data-action="mg-cap:${c.id}"><h3>${esc(c.name)}</h3><p>${esc(c.description)}</p></button><button type="button" class="toggle" role="switch" aria-label="${esc(c.name)}" aria-checked="${c.on}" data-action="mg-quick-toggle:${c.id}"></button></div>`).join('')}</div></div>`;
        } else if (category === 'diagnostics') {
            detailHTML = `<div class="settings-detail-inner"><h1>诊断</h1><p class="muted" style="margin:6px 0 18px">运行模式与本地存储可读性自检。</p><section class="inline-card" style="padding:18px;margin-bottom:18px"><div class="settings-row" style="border-bottom:0"><div><h3>调试日志（演示）</h3><p>在真实客户端用于排障记录，不包含密钥或用户隐私内容。</p></div><button role="switch" class="toggle" aria-label="调试日志" aria-checked="${preferences.debugLog}" data-action="ex-pref:debugLog:diagnostics"></button></div></section><dl><div><dt>运行模式</dt><dd>浏览器内本地演示</dd></div><div><dt>本地对象</dt><dd>${documents.length} 项资料 · ${models.length} 个模型配置</dd></div><div><dt>连接状态</dt><dd>无真实远端连接</dd></div></dl>${button('运行演示检查', 'ex-diagnostic', 'primary')}</div>`;
        } else if (category === 'privacy' || category === 'prototype') {
            detailHTML = `<div class="settings-detail-inner"><div class="subnav"><button data-action="ex-settings:">设置首页</button>${svg('chevron')}<span>${category === 'privacy' ? '隐私与数据' : '关于与演示控制'}</span></div>${category === 'privacy' ? `<div class="detail-section"><h2>你的演示，不离开浏览器</h2><p>此原型不调用模型、不连接资料库、不执行工具，也不收集输入。外观偏好保存在本机，演示会话仅在本页内存中。</p><p>请不要在演示表单中填写真实密钥或敏感资料。</p></div>` : `<div class="detail-section"><h2>Nola · 交互原型</h2><p>使用中性表面层级、单一主导航与三栏工作上下文。</p></div><div class="settings-row"><div><h3>长列表样例</h3><p>补充到 30 条，检查滚动与返回位置。</p></div><button class="text-action" data-action="long-list">加载</button></div><div class="settings-row"><div><h3>恢复初始演示</h3><p>清空本次页面中的模拟修改，不影响真实数据。</p></div><button class="text-action" data-action="reset">恢复</button></div>`}</div>`;
        }
        return `<div class="settings-window-backdrop" data-settings-backdrop><section class="settings-window" role="region" aria-label="设置窗口"><aside class="settings-sidebar" aria-label="设置分类"><div class="settings-sidebar-head"><strong>设置</strong><button type="button" class="strip-close" aria-label="关闭设置" title="关闭设置" data-action="dk-settings-close">${svg('close')}</button></div><div class="settings-groups">${desktopSettingsGroups.map(grp => `<div class="settings-group-block"><div class="settings-group-label">${grp.group}</div>${grp.items.map(([id, title]) => `<button type="button" class="settings-nav-item ${category === id ? 'selected' : ''}" data-action="ex-settings:${id}" ${category === id ? 'aria-current="page"' : ''}>${title}</button>`).join('')}</div>`).join('')}</div></aside><div class="settings-window-detail">${detailHTML}</div></section></div>`;
    }

    function settings() {
        return `<section class="page">${crumbs('设置')}${pageHead('设置', '你的习惯，而不是更多打扰。')}<div class="objects">${[['general', '通用', '外观、通知、软件更新与启动'], ['providers', '模型服务', '本地与 API 模型配置'], ['agent', '智能体', '记忆、检索与访问默认项'], ['connections', '连接', '当前 Host 与连接范围'], ['data', '数据', '资料、记忆与隐私管理'], ['diagnostics', '诊断', '状态检查与问题恢复']].map(([id, t, s]) => `<button class="object-row" data-action="ex-settings:${id}"><div class="object-copy"><h3>${t}</h3><p>${s}</p></div>${svg('chevron')}</button>`).join('')}</div><details class="prototype-state-controls"><summary>原型预览</summary><p><a href="scenarios.html">全业务场景与状态总览 ↗</a></p>${button('关于与演示控制', 'ex-settings:prototype')}</details></section>`;
    }

    function general() {
        return `<section class="page prototype-general"><div class="subnav">${button('设置', 'settings-back')}</div>${pageHead('通用', '外观、通知与这台设备上的偏好。')}<section class="inline-card"><h2>外观</h2><div class="settings-row"><span>主题</span><div class="segment">${['light', 'dark'].map(mode => `<button type="button" aria-pressed="${theme === mode}" class="${theme === mode ? 'selected' : ''}" data-action="set-theme:${mode}">${mode === 'light' ? '浅色' : '深色'}</button>`).join('')}</div></div><p class="dialog-note">原型可预览两种主题；实际客户端还支持跟随系统。</p></section><section class="inline-card"><h2>通知</h2><p>任务结束或需要回应时提醒，不包含会话正文。</p><div class="settings-row"><span>显示演示活动提醒</span><button role="switch" class="toggle" aria-label="显示演示活动提醒" aria-checked="${preferences.notifications}" data-action="ex-pref:notifications:general"></button></div><p class="dialog-note">原型不申请系统或浏览器通知权限。</p></section><section class="inline-card"><h2>软件更新</h2>${window.updatePreview.content()}</section><section class="inline-card"><h2>启动</h2><div class="settings-row"><span>登录时启动 Nola</span><button role="switch" class="toggle" aria-label="登录时启动 Nola（演示）" aria-checked="${!!preferences.startup}" data-action="ex-pref:startup:general"></button></div><p class="dialog-note">仅预览本机选项，浏览器客户端不提供此项；不改变系统启动配置。</p></section></section>`;
    }

    function setting(id) {
        if (architectureModule?.getMode?.() === 'desktop') {
            architectureModule.openSettings?.(id);
            return;
        }
        if (['general', 'notifications', 'updates', 'appearance'].includes(id)) {settingsSection = 'general'; render(); return;}
        if (id === 'prototype' || id === 'privacy') {settingsSection = id; render(); return;}
        if (id === 'providers') {navigate('models'); return;}
        if (id === 'agent') {openDialog('智能体', `<p>在执行每个需要权限的动作前确认。</p><div class="action-wrap">${button('默认记忆与检索', 'ex-settings:agent-defaults')}${button('能力与访问', 'ex-route:capabilities')}${button('等待授权的活动', 'ex-route:activity')}</div>`); return;}
        if (id === 'connections') {openDialog('连接', '<p>当前为本地演示，无真实 Host 连接。</p><p>在产品中管理 Host 身份、连接状态与账户；原型不收集令牌或发起登录。</p>'); return;}
        if (id === 'data') {openDialog('数据与隐私', `<div class="action-wrap">${button('资料与数据源', 'ex-route:data')}${button('长期记忆', 'ex-route:memory')}${button('隐私说明', 'ex-settings:privacy')}</div>`); return;}
        if (id === 'agent-defaults') {
            const keys = ['memory', 'retrieval'];
            openDialog('智能体默认项', keys.map(k => `<div class="settings-row"><div><h3>${({memory: '允许生成演示记忆', retrieval: '默认检索演示资料', notifications: '显示演示活动提醒'})[k]}</h3><p>只影响此原型显示状态，不修改账户设置。</p></div><button role="switch" class="toggle" aria-label="${k}" aria-checked="${preferences[k]}" data-action="ex-pref:${k}:${id}"></button></div>`).join(''));
            return;
        }
        if (id === 'diagnostics') {
            openDialog('诊断', `<dl><div><dt>运行模式</dt><dd>浏览器内本地演示</dd></div><div><dt>数据健康</dt><dd>${documents.length} 项资料 · ${models.length} 个模型配置</dd></div><div><dt>连接</dt><dd>无真实连接</dd></div></dl>${button('运行演示检查', 'ex-diagnostic', 'primary')}`);
        }
    }

    function restoreSettingFocus(action) {
        document.querySelector(`[data-action="${CSS.escape(action)}"]`)?.focus({preventScroll: true});
    }

    function handle(action) {
        const [a, v, w] = action.split(':');
        if (a === 'set-theme' && (settingsSection === 'general' || (architectureModule?.getMode?.() === 'desktop' && architectureModule.snapshot().settingsOpen))) {
            const detailEl = document.querySelector('.settings-window-detail');
            const detailTop = detailEl?.scrollTop;
            theme = v;
            applyTheme();
            render();
            const nextDetail = document.querySelector('.settings-window-detail');
            if (nextDetail && detailTop !== undefined) nextDetail.scrollTop = detailTop;
            restoreSettingFocus(action);
            return true;
        }
        if (!a.startsWith('ex-')) return false;
        const i = Number(v);
        if (a === 'ex-source-new') sourceForm();
        if (a === 'ex-source') source(i);
        if (a === 'ex-source-edit') sourceForm(i);
        if (a === 'ex-source-chat') {closeDialog(); navigate('chat', documents[i].sourceId);}
        if (a === 'ex-source-test') {documents[i].health = documents[i].policy === '禁止访问' ? '已阻止 · 访问策略不允许' : '演示检查通过'; source(i);}
        if (a === 'ex-model-edit') modelForm(i);
        if (a === 'ex-model-new') modelForm();
        if (a === 'ex-model') model(i);
        if (a === 'ex-model-test') {const m = models[i]; m.attempts = (m.attempts || 0) + 1; m.state = m.attempts === 1 ? '演示超时 · 可重试' : '演示连接正常'; model(i);}
        if (a === 'ex-plan') plan(i);
        if (a === 'ex-plan-edit') planForm(i);
        if (a === 'ex-plan-new') planForm();
        if (a === 'ex-plan-run') {runs.unshift({name: plans[i].name, state: '已完成', time: '刚刚', result: '演示结果：已整理关键讨论，保留原会话中的知识点。'}); toast('演示运行完成'); handle('ex-plan-history:' + i);}
        if (a === 'ex-plan-history') {openDialog('运行历史', runs.filter(r => r.name === plans[i].name).map(r => `<div class="detail-section"><h3>${esc(r.state)} · ${esc(r.time)}</h3><p>${esc(r.result)}</p>${button('查看整理内容', 'ex-route:memory')}</div>`).join('') || '<p>暂无运行记录。可以回到计划，演示运行一次。</p>');}
        if (['ex-source-delete', 'ex-model-delete', 'ex-plan-delete'].includes(a)) {openDialog('删除这个演示项？', `<p>只删除本页演示数据，不影响真实配置。</p><div class="dialog-actions">${button('取消', 'close')}${button('确认删除', a + '-confirm:' + i, 'primary')}</div>`);}
        if (a.endsWith('-delete-confirm')) {
            if (a === 'ex-source-delete-confirm') documents.splice(i, 1);
            if (a === 'ex-model-delete-confirm') {
                const name = models[i].name;
                models.splice(i, 1);
                if (defaultModel === name) defaultModel = models[0]?.name || '未选择';
                conversations.forEach(c => {if (c.model === name) c.model = undefined;});
            }
            if (a === 'ex-plan-delete-confirm') plans.splice(i, 1);
            closeDialog();
            render();
        }
        if (a === 'ex-settings') setting(v);
        if (a === 'ex-pref') {
            const detailEl = document.querySelector('.settings-window-detail');
            const detailTop = detailEl?.scrollTop;
            preferences[v] = !preferences[v];
            if (architectureModule?.getMode?.() === 'desktop' && architectureModule.snapshot().settingsOpen) {
                render();
                const nextDetail = document.querySelector('.settings-window-detail');
                if (nextDetail && detailTop !== undefined) nextDetail.scrollTop = detailTop;
            } else {
                setting(w);
            }
            restoreSettingFocus(action);
        }
        if (a === 'ex-route') {
            closeDialog();
            if (architectureModule?.getMode?.() === 'desktop') {
                architectureModule.closeSettings?.();
                if (v === 'capabilities') {architectureModule.openSection?.('extensions', 'skills'); return true;}
                if (v === 'memory' || v === 'data') {architectureModule.openSection?.('memory', v === 'data' ? 'sources' : 'memory'); return true;}
                if (v === 'activity') {openDialog('活动与结果', manageModule.view('activity')); return true;}
            }
            navigate(v);
        }
        if (a === 'ex-diagnostic') {openDialog('演示检查完成', '<p>本地页面状态可读；外部连接未执行检查。无真实服务诊断结果。</p>');}
        return true;
    }

    return {
        settingsWindowHTML,
        view: p => p === 'manage' ? management() : p === 'data' ? data() : p === 'settings' ? (settingsSection === 'general' ? general() : !settingsSection ? settings() : null) : null,
        handle,
        model,
        modelForm,
        plan,
        planForm,
        preferences,
        runs
    };
})();
