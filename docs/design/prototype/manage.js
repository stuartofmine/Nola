/* Evidence-mapped local demonstrations. No network, credentials or remote actions. */
(() => {
    let capTab = '技能', sourceFilter = '全部', activityFilter = '全部', activityQuery = '';
    const capabilities = [
        {id: 'reading', type: '技能', host: '这台 Mac', name: '阅读与整理', description: '从资料中提取要点，并保留依据。', on: true, scope: '所选资料', status: '可用（演示）'},
        {id: 'writing', type: '技能', host: '这台 Mac', name: '写作协作', description: '围绕原意，整理结构与表达。', on: true, scope: '当前对话', status: '可用（演示）'},
        {id: 'review-skill', type: '技能', host: '已连接主机', name: '代码审查工作流', description: '检查改动范围、回归风险与可验证结果。', on: false, scope: '当前工作区', status: '远端主机示例（演示）'},
        {id: 'mcp', type: 'MCP', host: '这台 Mac', name: '参考资料服务', description: '为当前对话提供可检索的参考资料。', on: false, scope: '每次询问', endpoint: 'https://example.invalid/mcp', status: '未测试'},
        {id: 'files', type: '系统访问', host: '这台 Mac', name: '文件与文件夹', description: '只访问你明确选定的范围。', on: false, scope: '每次询问', status: '未授权'},
        {id: 'notifications', type: '系统访问', host: '这台 Mac', name: '通知', description: '完成结果与需要你确认的事项。', on: true, scope: '仅重要事项', status: '本地演示'},
        {id: 'extensions', type: '扩展与诊断', host: '这台 Mac', name: '桌面扩展', description: '连接桌面能力，并查看运行状态。', on: false, scope: '当前设备', status: '未启用'},
        {id: 'diagnostics', type: '扩展与诊断', host: '这台 Mac', name: '连接诊断', description: '检查模型、资料和扩展的模拟状态。', on: true, scope: '仅演示检查', status: '尚未检查'}
    ];
    const activities = [
        {id: 'attention', kind: 'Agent', title: '整理设计草案', status: '待确认', time: '刚刚', source: '让 Nola 更像 Nola', chat: 'design', detail: '草案已整理，等待你确认是否保存到工作区。', result: '已将草案加入演示资料。'},
        {id: 'input', kind: 'Agent', title: '补充阅读摘记的整理范围', status: '待补充', time: '10 分钟前', source: '阅读与整理', detail: '继续整理前，还需要你选择要保留的内容范围。', result: '已根据你补充的范围完成整理。'},
        {id: 'failure', kind: '计划', title: '每周阅读整理', status: '未完成', time: '昨天 18:30', source: '整理这一周的阅读摘记', detail: '演示资料源暂时不可用。已有结果已保留，可仅重试未完成部分。', result: '资料源恢复，阅读摘记已整理。'},
        {id: 'waiting', kind: 'Agent', title: '等资料更新后补充比较', status: '等事件', time: '刚刚', source: '本地模型，怎么选更合适', chat: 'compare', detail: '本次执行已结束，比较目标仍保留；没有需要你回答的问题。现有计划或来源事件到达执行 Host 后才可推进。', result: ''},
        {id: 'complete', kind: 'Agent', title: '本地模型比较', status: '已完成', time: '昨天', source: '本地模型，怎么选更合适', chat: 'compare', detail: '从速度、内存与日常体验三个维度整理了比较方法。', result: '固定同一组任务，分别记录首字延迟、生成速度与峰值内存；不要用总分掩盖取舍。'}
    ];
    const note = '<div class="demo-banner">虚构演示数据 · 操作仅更新本页状态，不连接服务、不授予系统权限、不提交或推送代码。</div>';
    const action = (text, a, cls = 'text-action') => `<button class="${cls}" data-action="mg-${a}">${text}</button>`;
    const row = c => `<button class="object-row" data-action="mg-cap:${c.id}"><div class="object-copy"><h3>${esc(c.name)}</h3><p>${esc(c.description)}</p><p class="meta">${esc(c.host || '这台 Mac')} · ${esc(c.scope)}</p></div><span class="object-meta">${c.on ? '已启用' : '未启用'}</span>${svg('chevron', 'chevron')}</button>`;
    let catalogFailed = false;

    function installPreviewLink(platform = 'mac', paired = false) {
        const scene = paired ? 'github-skill-host' : 'github-skill';
        const label = paired ? '在配对 Host 安装' : '从对话安装 GitHub Skill';
        return `<a class="text-action" href="scenarios.html?scene=${scene}&amp;platform=${platform}&amp;theme=${theme}">${label}</a>`;
    }

    function harnessLink(platform = 'mac') {
        return `<a class="text-action" href="scenarios.html?scene=harness-plugins&amp;platform=${platform}&amp;theme=${theme}">插件</a>`;
    }

    function capabilitiesForKind(kind) {
        const targetType = kind === 'mcp' ? 'MCP' : '技能';
        const filtered = capabilities.filter(c => c.type === targetType && (sourceFilter === '全部' || c.host === sourceFilter));
        const headerActions = targetType === '技能'
            ? `${harnessLink()}${installPreviewLink()}${action('新建技能…', 'new-skill-preview')}`
            : action('添加服务器…', 'new-mcp-preview');
        return `<div class="cap-toolbar"><div class="segment" role="group" aria-label="来源筛选">${['全部', '这台 Mac', '已连接主机'].map(src => `<button type="button" class="${sourceFilter === src ? 'selected' : ''}" data-action="mg-source-filter:${src}">${src}</button>`).join('')}</div><div class="action-wrap" style="margin:0">${headerActions}</div></div>${catalogFailed ? `<section class="catalog-recovery" role="alert"><h2>无法读取能力清单</h2><p>原有配置保留；重新读取前不能编辑。不会把不完整响应显示为空清单。</p>${action('重新读取', 'catalog-reload', 'primary')}</section>` : `<div class="objects">${filtered.length ? filtered.map(row).join('') : '<div class="empty-state"><h2>此筛选下暂无项目</h2><p>切换到「全部」查看本机与已连接主机项目。</p></div>'}</div>`}<details class="prototype-state-controls"><summary>原型状态预览</summary><p>仅模拟清单读取失败，不连接服务。</p>${action('演示清单读取失败', 'catalog-fail')}</details>${note}`;
    }

    function capabilitiesContent(platform = 'mac') {
        const filtered = capabilities.filter(c => c.type === capTab);
        return `<details class="prototype-state-controls"><summary>原型状态预览</summary><p>仅模拟清单读取失败，不连接服务。</p>${action('演示清单读取失败', 'catalog-fail')}</details>${catalogFailed ? `<section class="catalog-recovery" role="alert"><h2>无法读取能力清单</h2><p>原有配置保留；重新读取前不能编辑。不会把不完整响应显示为空清单。</p>${action('重新读取', 'catalog-reload', 'primary')}</section>` : `<div class="tabs" aria-label="能力类型">${['技能', 'MCP', '系统访问', '扩展与诊断'].map(t => action(t, 'cap-tab:' + t, t === capTab ? 'selected' : '')).join('')}</div>${capTab === '技能' ? `<div class="cap-toolbar">${harnessLink(platform)}${installPreviewLink(platform)}${['ios','android'].includes(platform) ? installPreviewLink(platform, true) : ''}</div>` : ''}<div class="objects">${filtered.map(row).join('')}</div>`}${note}`;
    }

    function capabilitiesView() {
        return `<section class="page">${crumbs('能力与访问')}${pageHead('能力与访问', '需要时启用，始终知道访问的范围。')}${capabilitiesContent()}</section>`;
    }

    function capDetail(id) {
        const c = capabilities.find(item => item.id === id);
        if (!c) return;
        openDialog(c.name, `<p class="muted">${esc(c.description)}</p><dl><div><dt>来源</dt><dd>${esc(c.host || '这台 Mac')}</dd></div><div><dt>访问范围</dt><dd>${esc(c.scope)}</dd></div><div><dt>状态</dt><dd>${esc(c.status)}</dd></div>${c.endpoint ? `<div><dt>演示地址</dt><dd>${esc(c.endpoint)}</dd></div>` : ''}</dl><div class="dialog-actions" style="flex-wrap:wrap">${c.type === 'MCP' ? action('配置', 'configure:' + id) : ''}${c.type === 'MCP' || c.type === '扩展与诊断' ? action('模拟检查', 'test:' + id) : ''}${action(c.on ? '停用' : '启用', 'toggle-cap:' + id, 'primary')}</div><p class="dialog-note">仅本地模拟，不调用服务、不申请操作系统权限。</p>`);
    }

    function syncActivities() {
        const m = conversations.find(c => c.id === 'design')?.messages.find(item => item.permission);
        const a = activities.find(item => item.id === 'attention');
        if (m && a) a.status = ({pending: '待确认', approved: '已完成', denied: '已拒绝'})[m.permission] || a.status;
    }

    function filteredActivities() {
        syncActivities();
        return activities.filter(a => (activityFilter === '全部' || (activityFilter === '需关注' ? ['待确认', '待补充', '未完成'].includes(a.status) : a.kind === activityFilter)) && (a.title + a.source + a.detail).includes(activityQuery));
    }

    function activityRows() {
        const found = filteredActivities();
        return found.length ? found.map(a => `<button class="object-row" data-action="mg-activity:${a.id}"><div class="object-copy"><h3>${esc(a.title)}</h3><p>${esc(a.source)} · ${esc(a.kind)}</p><p class="meta">${esc(a.time)}</p></div><span class="object-meta">${esc(a.status)}</span>${svg('chevron', 'chevron')}</button>`).join('') : `<div class="empty-state"><h2>没有符合条件的记录</h2><p>试试其他关键词或查看全部记录。</p>${action('清除筛选', 'clear-activity')}</div>`;
    }

    function activityView() {
        return `<section class="page">${crumbs('活动与结果')}${pageHead('活动与结果', '看看完成了什么，还有什么需要你。')}<label class="search">${svg('search')}<input id="mg-search" aria-label="搜索活动" placeholder="搜索内容、来源或结果" value="${esc(activityQuery)}"></label><div class="tabs">${['全部', '需关注', 'Agent', '计划'].map(t => action(t, 'filter:' + t, t === activityFilter ? 'selected' : '')).join('')}</div><div class="objects" id="mg-results">${activityRows()}</div>${note}</section>`;
    }

    function activityDetail(id) {
        syncActivities();
        const a = activities.find(x => x.id === id);
        if (!a) return;
        const waiting = ['待确认', '待补充'].includes(a.status);
        openDialog(a.title, `<div class="context">${esc(a.kind)} · ${esc(a.status)} · ${esc(a.time)}</div><div class="detail-section"><p>${esc(a.detail)}</p></div><dl class="execution-facts"><div><dt>工作归属</dt><dd>${esc(a.source)}${a.chat ? action('来源会话', 'source:' + a.chat) : ''}</dd></div><div><dt>本次执行</dt><dd>${waiting ? '等待你的决定，保留当前执行' : a.status === '等事件' ? '已结束 · 等待来源事件' : a.status === '已完成' ? '已结束 · 已交付结果' : '已结束 · 工作未完成'}</dd></div><div><dt>执行位置</dt><dd>本机 Mac · 虚构演示，不代表当前设备有执行器</dd></div><div><dt>上下文范围</dt><dd>任务说明、所选资料引用与独立工作记录。未复制来源会话的全部历史；摘要不增加读取权限。</dd></div></dl>${a.status === '已完成' ? `<div class="detail-section"><h3>结果</h3><p>${esc(a.result)}</p></div>` : ''}${a.answer ? `<p>你补充的范围：${esc(a.answer)}</p>` : ''}<div class="dialog-actions" style="flex-wrap:wrap">${a.chat ? action('回到来源对话', 'source:' + a.chat) : ''}${waiting && a.chat ? action('去来源处理', 'source:' + a.chat, 'primary') : ''}</div><p class="dialog-note">详情只查看记录；授权、补充输入与恢复回到来源处理，保留原工作归属。本页不模拟跨设备接管，不把一次执行结束当成目标完成。</p>`);
    }

    function handle(a) {
        if (!a.startsWith('mg-')) return false;
        const [key, ...rest] = a.slice(3).split(':'), id = rest.join(':');
        const c = capabilities.find(item => item.id === id), item = activities.find(act => act.id === id);
        if (key === 'catalog-fail') {catalogFailed = true; render();}
        else if (key === 'catalog-reload') {catalogFailed = false; render();}
        else if (key === 'source-filter') {sourceFilter = id; render();}
        else if (key === 'new-skill-preview') {openDialog('新建本地 Skill（演示）', `<p class="dialog-note">本地创建的 Skill 以工作区文件方式管理，启用时显式确认；此原型不写入磁盘。</p><div class="dialog-actions">${action('关闭', 'close-dialog', 'primary')}</div>`);}
        else if (key === 'new-mcp-preview') {openDialog('添加 MCP 服务器（演示）', `<p class="dialog-note">支持 HTTP 与本地 stdio 传输配置；原型不创建子进程或网络连接。</p><div class="dialog-actions">${action('关闭', 'close-dialog', 'primary')}</div>`);}
        else if (key === 'close-dialog') {closeDialog();}
        else if (key === 'reread-config') {
            const form = dialog.querySelector('#mg-config');
            if (form) {
                const saved = capabilities.find(entry => entry.id === form.dataset.capability);
                const notice = form.querySelector('.save-recovery');
                notice.hidden = false;
                notice.textContent = '演示重新读取：当前保存的名称为「' + saved.name + '」。请核对草稿后明确保存，不会自动重发。';
                form.dataset.unconfirmed = 'false';
                form.querySelector('button[type=submit]').disabled = false;
            }
            return true;
        }
        else if (key === 'save-unconfirmed') {
            const form = dialog.querySelector('#mg-config');
            if (form) {
                form.dataset.unconfirmed = 'true';
                form.querySelector('[data-save-preview]').textContent = '下一次保存将演示回执未确认';
            }
            return true;
        }
        else if (key === 'cap-tab') {
            capTab = id;
            if (architectureModule?.getMode?.() === 'desktop' && architectureModule.snapshot().section === 'extensions') {
                architectureModule.syncExtTab?.(id === 'MCP' ? 'mcp' : 'skills');
            }
            render();
        }
        else if (key === 'cap') capDetail(id);
        else if (key === 'configure') {
            openDialog('配置参考资料服务', `<form id="mg-config" data-capability="${esc(id)}"><label for="mg-name">名称</label><input id="mg-name" name="name" required maxlength="60" value="${esc(c.name)}"><label for="mg-endpoint">演示服务地址</label><input id="mg-endpoint" name="endpoint" type="url" required value="${esc(c.endpoint)}"><label for="mg-scope">访问策略</label><select id="mg-scope" name="scope">${['每次询问', '仅所选资料', '只读'].map(s => `<option ${s === c.scope ? 'selected' : ''}>${s}</option>`).join('')}</select><p class="dialog-note">仅保存演示配置。无需也请勿输入密钥；不会访问此地址。</p><details class="prototype-state-controls"><summary>原型保存状态</summary><p data-save-preview>默认演示完整保存回执。</p><button type="button" data-action="mg-save-unconfirmed">演示保存回执未确认</button></details><p class="save-recovery" role="alert" hidden></p><button type="button" data-action="mg-reread-config">重新读取并核对演示配置</button><div class="dialog-actions"><button class="primary" type="submit">保存演示配置</button></div></form>`);
            dialog.querySelector('form').onsubmit = e => {
                e.preventDefault();
                if (e.target.dataset.unconfirmed === 'true') {
                    const notice = e.target.querySelector('.save-recovery');
                    notice.hidden = false;
                    notice.textContent = '保存尚未确认，草稿已保留。请先重新读取配置核对，避免重复提交。';
                    e.target.querySelector('button[type=submit]').disabled = true;
                    return;
                }
                const f = new FormData(e.target);
                c.name = String(f.get('name')).trim();
                c.endpoint = String(f.get('endpoint')).trim();
                c.scope = String(f.get('scope'));
                c.status = '配置已更新，未测试';
                closeDialog();
                render();
                capDetail(c.id);
            };
        }
        else if (key === 'toggle-cap') {
            if (c.on) {c.on = false; c.status = '已停用'; closeDialog(); render(); return true;}
            openDialog('确认启用范围', `<p>${esc(c.name)}将使用「${esc(c.scope)}」范围。</p><p class="dialog-note">仅变更原型状态，不会授予真实权限。</p><div class="dialog-actions">${action('不启用', 'cancel-enable:' + id)}${action('允许本次演示', 'enable:' + id, 'primary')}</div>`);
        }
        else if (key === 'cancel-enable') {closeDialog(); toast('未启用，访问范围没有改变');}
        else if (key === 'enable') {c.on = true; c.status = '演示已启用'; closeDialog(); render(); capDetail(id);}
        else if (key === 'test') {c.status = c.id === 'mcp' && !c.on ? '模拟失败：服务未启用' : '模拟检查通过'; closeDialog(); render(); capDetail(id);}
        else if (key === 'filter') {activityFilter = id; render();}
        else if (key === 'clear-activity') {activityFilter = '全部'; activityQuery = ''; render();}
        else if (key === 'activity') activityDetail(id);
        else if (key === 'source') {closeDialog(); navigate('chat', id);}
        else if (key === 'deny') {if (item.id === 'attention') window.handle('deny:1:design'); item.status = '已拒绝'; closeDialog(); render(); activityDetail(id);}
        else if (key === 'request') {if (item.id === 'attention') window.handle('reapprove:1:design'); item.status = '待确认'; closeDialog(); render(); activityDetail(id);}
        else if (key === 'approve') {
            if (item.status === '待确认') {
                if (item.id === 'attention') window.handle('approve:1:design');
                item.status = '已完成';
                if (item.id !== 'attention' && !documents.some(d => d.name === '设计草案 · 活动保存')) documents.unshift({name: '设计草案 · 活动保存', type: '文档', info: '刚刚 · 来自演示活动', size: '演示', content: '内容优先，操作在需要时靠近。'});
            }
            closeDialog();
            render();
            activityDetail(id);
        }
        else if (key === 'retry') {item.status = '已完成'; item.detail = '重试已完成；先前生成的结果仍保留。'; closeDialog(); render(); activityDetail(id);}
        else if (key === 'input') {
            openDialog('补充整理范围', `<form id="mg-input"><label for="mg-answer">想保留哪些内容？</label><textarea id="mg-answer" name="answer" required maxlength="600" placeholder="例如：保留关键观点和引用来源"></textarea><p class="dialog-note">提交后仅推进本地演示，不启动真实任务。</p><div class="dialog-actions"><button type="submit" class="primary">补充并继续演示</button></div></form>`);
            dialog.querySelector('form').onsubmit = e => {
                e.preventDefault();
                const answer = new FormData(e.target).get('answer').trim();
                if (!answer) return;
                item.answer = answer;
                item.status = '已完成';
                closeDialog();
                render();
                activityDetail(id);
            };
        }
        return true;
    }

    window.manageModule = {
        capabilitiesContent,
        capabilitiesForKind,
        view: p => p === 'capabilities' ? capabilitiesView() : p === 'activity' ? activityView() : null,
        handle,
        bind() {
            const input = document.querySelector('#mg-search');
            if (input) input.oninput = e => {activityQuery = e.target.value; document.querySelector('#mg-results').innerHTML = activityRows(); bind(document.querySelector('#mg-results'));};
        },
        snapshot: () => {
            syncActivities();
            return {capabilities, activities, capTab, sourceFilter};
        }
    };
})();
