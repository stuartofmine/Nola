// SPDX-License-Identifier: Apache-2.0
// Standalone design review. No product API, credentials, files or model execution.
(() => {
    'use strict';
    const $ = selector => document.querySelector(selector);
    const escape = value => String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
    const memories = new Map(), visited = new Set();
    const names = {mac:'macOS',ios:'iOS / iPadOS',android:'Android',web:'Web',windows:'Windows',cli:'CLI'};
    const kindNames = {empty:'尚未开始',choice:'等待选择',form:'编辑草稿',waiting:'等待中',error:'需要处理',success:'已完成',stream:'正在回复',answer:'回复',reader:'阅读',review:'核对',danger:'需要确认',warning:'待核对',progress:'执行中',selection:'手动选择',list:'内容列表',permission:'等待授权',blocked:'已阻止',inspector:'会话信息'};
    let catalog, scene, platform = 'mac', stateId = '0', pending = null, priorFocus = null;
    function memory() {
        const key = `${platform}:${scene.id}`;
        if (!memories.has(key)) {
            const values = Object.fromEntries(scene.fields.map((field, index) => [index, field.value]));
            memories.set(key, {draft:{...values}, saved:{...values}, state:'0'});
        }
        return memories.get(key);
    }
    function urlFor(id, state = '0', selectedPlatform = platform) {
        const url = new URL(location.href);
        url.searchParams.set('scene', id); url.searchParams.set('state', state);
        url.searchParams.set('platform', selectedPlatform); url.searchParams.set('theme', $('#theme').value);
        return url.pathname + url.search;
    }
    function syncLocation(replace = false) {
        history[replace ? 'replaceState' : 'pushState']({}, '', urlFor(scene.id, stateId));
    }
    function readLocation() {
        const params = new URLSearchParams(location.search);
        platform = Object.hasOwn(names, params.get('platform')) ? params.get('platform') : 'mac';
        scene = catalog.scenarios.find(item => item.id === params.get('scene')) || catalog.scenarios[0];
        stateId = scene.states.some(item => item.id === params.get('state')) ? params.get('state') : '0';
        memory().state = stateId;
        $('#platform').value = platform;
        const theme = params.get('theme');
        $('#theme').value = ['light','dark'].includes(theme) ? theme : 'system';
        applyTheme(); render();
    }
    function applyTheme() {
        const value = $('#theme').value;
        if (value === 'system') delete document.documentElement.dataset.theme;
        else document.documentElement.dataset.theme = value;
    }
    function renderList() {
        const query = $('#search').value.trim().toLocaleLowerCase();
        const onlyApplicable = $('#applicable').checked;
        let total = 0;
        $('#scene-list').innerHTML = catalog.groups.map(group => {
            const items = catalog.scenarios.filter(item => item.group === group.id && (!onlyApplicable || item.platforms.includes(platform)) &&
                [item.title,item.entry,item.id,...item.states.map(state => `${state.title} ${state.body.join(' ')}`)].join(' ').toLocaleLowerCase().includes(query));
            total += items.length;
            return items.length ? `<section class="catalog-group"><h2>${escape(group.title)}</h2>${items.map(item => `<a class="scene-link ${item.platforms.includes(platform) ? '' : 'unavailable'}" href="${escape(urlFor(item.id, memories.get(`${platform}:${item.id}`)?.state || '0'))}" data-scene="${item.id}" ${item.id === scene.id ? 'aria-current="page"' : ''}>${escape(item.title)}${item.platforms.includes(platform) ? '' : '<small>此平台无独立设计 · 查看适用范围</small>'}</a>`).join('')}</section>` : '';
        }).join('') || '<p class="empty-catalog">没有匹配场景。清除搜索或取消平台筛选。</p>';
        $('#coverage-count').textContent = `${catalog.scenarios.length} 个业务场景 · ${catalog.scenarios.reduce((n,s) => n+s.states.length,0)} 个状态 · 当前显示 ${total} 项`;
    }
    function shell(content) {
        const mobile = ['ios','android'].includes(platform);
        const system = platform === 'web' ? '浏览器 · 当前 Host' : platform === 'windows' ? 'Windows · Web Host 客户端' : platform === 'cli' ? '终端 · 文本交互草图' : `${names[platform]} · 原生控件布局示意`;
        return `<div class="device-system"><span>${escape(system)}</span><span>仅演示</span></div>${platform === 'cli' ? '' : `<header class="app-top"><strong>Nola</strong><span>${escape(scene.entry)}</span></header>${mobile ? '' : '<div class="review-tabs"><span class="current">当前内容</span><span>任务</span><span>产物</span><span>上下文</span></div>'}`}${content}${mobile ? '<footer class="app-foot"><span>对话</span><span>活动</span><span>资料</span><span>设置</span></footer>' : ''}`;
    }
    function formMarkup(state) {
        if (!['form','choice','review'].includes(state.kind) || !scene.fields.length) return '';
        return `<form class="sample-form" id="sample-form" novalidate>${scene.fields.map((field, index) => `<label>${escape(field.label)}${field.required ? '<small>必填 · 仅样例</small>' : ''}<textarea aria-label="${escape(field.label)}" data-field="${index}" ${field.required ? 'required' : ''} rows="${field.value.length > 35 ? 3 : 2}">${escape(memory().draft[index])}</textarea></label>`).join('')}<small>修改只保留在此平台、此场景的本页草稿中。</small></form>`;
    }
    function domainMarkup(state) {
        if (scene.id === 'permission' && state.id === '0') return '<details class="operation-review"><summary>完整操作与授权范围</summary><pre>Command: python3 remote_job.py\nWorking directory: Project directory\n执行位置：已配对宿主机\n范围：仅本次操作</pre><p>批准后才开始执行；拒绝不会修改文件。</p></details>';
        if (['long-run','remote-reconnect','host-cold','cancel'].includes(scene.id)) {
            const done=state.kind==='success', disconnected=state.kind==='warning';
            return `<section class="task-summary" aria-label="当前任务样例"><div><strong>资料对比报告</strong><span class="muted">${done?'已核对':disconnected?'等待重新连接':'当前执行'}</span></div><dl><div><dt>执行位置</dt><dd>已配对宿主机</dd></div><div><dt>执行归属</dt><dd>当前会话 · 原任务</dd></div><div><dt>最新进度</dt><dd>${done?'200 / 200':disconnected?'连接中断前 132 / 200':'132 / 200'} 个依赖步骤</dd></div></dl><ol class="task-timeline"><li>读取来源 <span>已完成</span></li><li>核对依赖 <span>${done?'已完成':'已记录检查点'}</span></li><li>生成报告 <span>${done?'已完成':'待确认'}</span></li></ol>${done&&scene.id!=='cancel'?'<button data-detail="report">查看报告样例 ↗</button>':''}</section>`;
        }
        if (scene.id === 'compaction') return '<dl class="metric-list"><div><dt>上下文占用</dt><dd>73% <small>样例 · 压缩前</small></dd></div><div><dt>缓存命中率</dt><dd>未上报</dd></div><div><dt>当前任务</dt><dd>继续执行原目标</dd></div></dl>';
        if (scene.id === 'mcp-config' && state.id==='0') return `<label class="domain-picker">传输方式<select id="mcp-transport" aria-label="MCP 传输方式"><option value="http" ${memory().transport==='stdio'?'':'selected'}>HTTP 服务</option><option value="stdio" ${memory().transport==='stdio'?'selected':''}>本地 stdio · 由执行宿主机启动</option></select></label><p class="transport-note muted">${memory().transport==='stdio'?'在实际客户端填写启动命令与参数；原型不会创建进程。':'在实际客户端核对 URL 与认证引用；原型不接受真实凭据。'}</p>`;
        if (scene.id === 'catalog-select' && ['choice','success'].includes(state.kind)) return '<fieldset class="model-options"><legend>可用模型 · 样例目录</legend><label><input type="checkbox" checked data-model-enabled> Demo Reasoning · 五档</label><label><input type="radio" checked name="selected-demo-model"> 选为默认模型</label><small>启用集合与默认选择分别保存。</small></fieldset>';
        if (scene.id === 'goal-input' && state.kind!=='form') return '<ul class="checklist"><li>目标：生成带来源的对比报告</li><li>范围：当前项目公开资料</li><li>验收：来源、差异、未确定事项均可核对</li><li>尚未完成：最终人工验收</li></ul>';
        if (scene.id === 'general-fonts') return `<label class="domain-picker">正文字体预览<select id="font-preview" aria-label="正文字体预览"><option value="system" ${memory().font==='serif'?'':'selected'}>系统默认 · 跟随平台</option><option value="serif" ${memory().font==='serif'?'selected':''}>系统衬线 · 长文预览</option></select></label><div class="font-specimen ${memory().font==='serif'?'serif':''}"><p>让答案更清楚，让操作在需要时出现。</p><p>Nola keeps your work close, and the next step clear.</p><code>progress = 132 / 200</code></div>`;
        if (scene.id==='account-browser' && state.kind==='waiting') return '<ol class="task-timeline"><li>开始本次授权 <span>已准备</span></li><li>系统浏览器确认 <span>等待你完成</span></li><li>获取并启用模型 <span>下一步</span></li></ol>';
        return '';
    }
    function specialMarkup(state) {
        const lines = state.body;
        if (state.kind === 'list') return `<ul class="sample-items">${lines.map((line,index) => `<li>${escape(line)}<small>样例 ${index+1} · 当前来源</small></li>`).join('')}</ul>`;
        if (['stream','answer'].includes(state.kind)) return `<div class="message-sample">${escape(scene.fields[0]?.value || '请核对这次操作的结果与来源。')}</div><div class="answer-sample ${state.kind === 'stream' ? 'reveal' : ''}">${lines.map(line=>`<p>${escape(line)}</p>`).join('')}</div><div class="text-actions"><button data-copy>复制</button>${state.kind === 'stream' ? '<button data-stop>停止演示回复</button>' : ''}</div>`;
        if (state.kind === 'progress') return `<div class="content-lines">${lines.map(line=>`<p>${escape(line)}</p>`).join('')}</div><div class="demo-progress" aria-label="样例进度，不来自真实任务"><span></span></div><small>进度为固定样例；没有运行真实任务。</small>`;
        if (state.kind === 'inspector') return `<div class="sample-inspector">${['概览','进度','目标','审阅','产物','上下文','执行','参与者'].map((title,index)=>`<details ${index===0 || index===5 ? 'open' : ''}><summary>${title}</summary><p>${escape(lines[index % lines.length])}</p>${title === '上下文' ? '<label>Wiki 草稿<textarea data-wiki aria-label="检查器 Wiki 草稿">'+escape(memory().wiki || '')+'</textarea></label>' : ''}</details>`).join('')}</div>`;
        if (state.kind === 'selection') return `<div class="content-lines">${lines.map(line=>`<p>${escape(line)}</p>`).join('')}</div><div tabindex="0" aria-label="可手动复制的样例文字" class="selection-text">演示回复：原始记录已保留，未确认事项仍需核对。</div>`;
        return `<div class="content-lines ${['error','warning','blocked','permission'].includes(state.kind) ? `state-callout ${state.kind}` : ''}">${lines.map(line=>`<p>${escape(line)}</p>`).join('')}</div>${state.kind === 'blocked' && scene.id === 'permission-missing' ? '<div class="permission-disabled"><button disabled>仅允许本次</button><button disabled>本次对话允许</button></div>' : ''}${scene.id === 'reasoning' && state.id === '0' ? '<fieldset class="sample-form"><legend>推理档位 · 模型声明样例</legend><label><select id="effort" aria-label="实际推理档位">'+['继承','none','low','medium','high','xhigh'].map(value=>`<option ${value===(memory().effort || '继承')?'selected':''}>${value}</option>`).join('')+'</select></label></fieldset>' : ''}${state.kind === 'reader' ? '<pre class="sample-code" tabindex="0" aria-label="可横向滚动的代码样例">记录 | 原始来源 | 已验证状态\n001  | project/source-001.md | 已保留\n002  | project/source-002.md | 尚待核对</pre>' : ''}`;
    }
    function renderDevice() {
        const available = scene.platforms.includes(platform);
        const state = scene.states.find(item=>item.id===stateId);
        $('#device').className = `${['ios','android'].includes(platform) ? 'mobile' : platform} ${$('#large-text').checked ? 'large' : ''}`;
        $('#device').dataset.scene = scene.id; $('#device').dataset.state = stateId;
        $('#device').dataset.available = String(available);
        if (!available) {
            $('#device').innerHTML = shell(`<section class="app-content"><div class="status-heading status-blocked">此平台无独立设计</div><h3>使用适用的入口</h3><p>此业务场景的设计适用范围：${scene.platforms.map(id=>names[id]).join('、')}。</p><p class="muted">不在此平台画出不存在的系统权限、安装或执行入口。平台标签是设计范围，真实支持程度仍以客户端为准。</p></section>`);
            return;
        }
        visited.add(`${platform}:${scene.id}:${state.id}`);
        const actions = state.actions.map((action,index)=>`<button data-transition="${index}" class="${action.style === 'primary' ? 'primary' : action.style === 'danger' ? 'danger' : ''}">${escape(action.label)}</button>`).join('');
        const heading = `<div class="status-heading status-${state.kind}"><span class="status-dot" aria-hidden="true"></span>${escape(kindNames[state.kind])} · 样例状态</div><h3>${escape(state.title)}</h3>`;
        let content = `${heading}${specialMarkup(state)}${domainMarkup(state)}${formMarkup(state)}<p id="field-error" class="inline-error" role="alert"></p><div class="actions">${actions}</div>`;
        if (platform === 'cli') content = `<section class="terminal"><pre class="terminal-command">$ ${escape(scene.command || 'nola')}\n# 文本布局草图；下方按钮为原型审阅控制，不是终端命令。</pre>${content}</section>`;
        else content = `<section class="app-content">${content}</section>`;
        $('#device').innerHTML = shell(content);
        $('#visit-count').textContent = `本页已检查 ${visited.size} 个平台状态`;
    }
    function render() {
        $('#group-name').textContent = catalog.groups.find(group=>group.id===scene.group).title;
        $('#preview-title').textContent = scene.title;
        $('#scene-entry').textContent = scene.entry;
        $('#scene-number').textContent = `${catalog.scenarios.indexOf(scene)+1} / ${catalog.scenarios.length}`;
        $('#contract').textContent = scene.contract;
        $('#boundary').textContent = scene.boundary;
        $('#sources').innerHTML = scene.sources.map(source=>`<li>${escape(source)}</li>`).join('');
        const available = scene.platforms.includes(platform);
        $('#state-list').innerHTML = scene.states.map((state,index)=>`<button data-state="${state.id}" ${state.id===stateId?'aria-current="step"':''} ${available?'':'disabled'}><span class="step-no">${String(index+1).padStart(2,'0')}</span>${escape(state.title)}</button>`).join('');
        renderList(); renderDevice();
        document.title = `${scene.title} · ${names[platform]} · Nola 业务场景`;
    }
    function transition(action, direct = false) {
        if (!action) return;
        const current = scene.states.find(item=>item.id===stateId);
        if (!direct && !action.cancel && action.style === 'primary' && current.kind === 'form') {
            const invalid = scene.fields.findIndex((field,index)=>field.required && !String(memory().draft[index] || '').trim());
            if (invalid !== -1) {
                $('#field-error').textContent = `请填写${scene.fields[invalid].label}；其他草稿已保留。`;
                $(`[data-field="${invalid}"]`).focus(); return;
            }
        }
        if (action.confirm && !direct) {
            pending = action; priorFocus = document.activeElement; $('#confirm').showModal(); $('#confirm-cancel').focus(); return;
        }
        if (action.cancel) memory().draft = {...memory().saved};
        stateId = action.to; memory().state = stateId;
        if (!direct && scene.states.find(item=>item.id===stateId).kind === 'success') memory().saved = {...memory().draft};
        syncLocation(); render();
        const focusTarget = $('#device h3'); focusTarget?.setAttribute('tabindex','-1'); focusTarget?.focus({preventScroll:true});
        $('#live').textContent = `演示状态：${scene.states.find(item=>item.id===stateId).title}。未执行真实操作。`;
    }
    function closeConfirm() {
        $('#confirm').close(); pending = null; priorFocus?.focus({preventScroll:true});
    }
    document.addEventListener('click', async event => {
        const target = event.target.closest('button,a');
        if (!target || !catalog) return;
        if (target.dataset.scene) {
            event.preventDefault(); scene = catalog.scenarios.find(item=>item.id===target.dataset.scene);
            stateId = memory().state; syncLocation(); render();
            if (innerWidth<=700) { $('#catalog').hidden = true; $('#catalog-toggle').setAttribute('aria-expanded','false'); $('#preview-title').focus({preventScroll:true}); }
        } else if (target.dataset.state) transition({to:target.dataset.state}, true);
        else if (target.dataset.detail) {
            const block=document.createElement('div'); block.className='selection-text';block.tabIndex=0;
            block.textContent='报告样例：200 条记录，10 个检查点；来源、差异与未确定事项已列出。此处未读取真实文件。';
            target.after(block);target.disabled=true;block.focus();
        }
        else if (target.dataset.transition !== undefined) transition(scene.states.find(item=>item.id===stateId).actions[Number(target.dataset.transition)]);
        else if (target.hasAttribute('data-stop')) {
            $('#live').textContent = '已停止本页合成回复；未取消真实任务。';
            $('#device .answer-sample').classList.remove('reveal'); target.disabled = true; target.textContent = '演示回复已停止';
        } else if (target.hasAttribute('data-copy')) {
            const text = $('#device .answer-sample').textContent;
            try { await navigator.clipboard.writeText(text); target.textContent = '已复制'; }
            catch { const block=document.createElement('textarea'); block.readOnly=true; block.value=text; block.setAttribute('aria-label','手动复制回复'); target.closest('.text-actions').after(block); block.focus(); block.select(); $('#live').textContent='无法写入剪贴板，请手动复制已选文字。'; }
        }
    });
    document.addEventListener('input', event => {
        if (!catalog) return;
        if (event.target.dataset.field !== undefined) memory().draft[event.target.dataset.field] = event.target.value;
        if (event.target.hasAttribute('data-wiki')) memory().wiki = event.target.value;
    });
    $('#search').addEventListener('input', () => catalog && renderList());
    $('#applicable').addEventListener('change', () => catalog && renderList());
    $('#platform').addEventListener('change', () => { platform=$('#platform').value; stateId=memory().state; syncLocation(); render(); });
    $('#theme').addEventListener('change', () => { applyTheme(); if (catalog) syncLocation(true); });
    $('#large-text').addEventListener('change', () => catalog && renderDevice());
    $('#reduce-motion').addEventListener('change', () => document.documentElement.dataset.reduce = String($('#reduce-motion').checked));
    $('#catalog-toggle').addEventListener('click', () => { $('#catalog').hidden=!$('#catalog').hidden; $('#catalog-toggle').setAttribute('aria-expanded',String(!$('#catalog').hidden)); });
    $('#reset-scene').addEventListener('click', () => { memories.delete(`${platform}:${scene.id}`); stateId='0'; memory(); syncLocation(); render(); $('#live').textContent='已重置此场景样例。其他场景未改变。'; });
    $('#confirm-cancel').addEventListener('click', closeConfirm);
    $('#confirm').addEventListener('cancel', event => { event.preventDefault(); closeConfirm(); });
    $('#confirm-apply').addEventListener('click', () => { const action=pending; $('#confirm').close(); pending=null; transition({...action,confirm:false}); });
    $('#device').addEventListener('change', event => {
        if (event.target.id==='effort') memory().effort=event.target.value;
        if (event.target.id==='font-preview') { memory().font=event.target.value; renderDevice(); $('#font-preview').focus(); }
        if (event.target.id==='mcp-transport') { memory().transport=event.target.value; renderDevice(); $('#mcp-transport').focus(); }
    });
    addEventListener('popstate', () => catalog && readLocation());
    if (innerWidth<=700) { $('#catalog').hidden=true; $('#catalog-toggle').setAttribute('aria-expanded','false'); }
    fetch('BUSINESS-SCENARIOS.json').then(response=>{ if (!response.ok) throw Error('场景清单读取失败'); return response.json(); }).then(value=>{
        if (value.schema_version!==1 || !Array.isArray(value.scenarios)) throw Error('场景清单格式不支持');
        catalog=value; readLocation(); syncLocation(true);
    }).catch(error=>{ $('#preview-title').textContent='无法读取业务场景'; $('#scene-entry').textContent='请通过本地 HTTP 服务打开原型后重新加载。'; $('#coverage-count').textContent=error.message; });
})();
