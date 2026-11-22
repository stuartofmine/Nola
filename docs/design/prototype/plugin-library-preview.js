// SPDX-License-Identifier: Apache-2.0
// Interactive review of the installed collection, installation and workspace navigation.
window.pluginLibraryPreview = function (state, platform, escape, session) {
    const id = state.id;
    const mobile = platform === 'ios' || platform === 'android';
    const cli = platform === 'cli';
    const device = mobile ? '书房 Host' : platform === 'mac' ? '这台 Mac' : '当前 Host';
    const hasBoard = session.hasBoard || ['both-installed','dsh-installed','page','sources','workspace-chat','dsh-remove'].includes(id);
    const hasTool = session.hasTool || ['both-installed','pi-installed','pi-remove','partial'].includes(id);
    const button = (dest, label, style = '') => {
        const index = state.actions.findIndex(a => a.to === dest);
        if (index < 0) return '';
        return `<button type="button" class="${style}" data-transition="${index}">${escape(label)}</button>`;
    };
    if (cli) {
        const commands = {add:'nola plugin',pi:'nola plugin pi install npm:pi-statusline',dsh:'nola plugin dsh --profile web install @example/research',page:'nola plugin desktop-pages',sources:'nola plugin desktop-pages'};
        return `<section class="terminal"><pre class="terminal-command">$ ${commands[id] || 'nola plugin desktop-library'}</pre><h3>${escape(state.title)}</h3><p>${escape(state.body[0])}</p><div class="actions">${state.actions.map(a=>button(a.to,a.label)).join('')}</div></section>`;
    }
    const boardRow = `<article class="pl-row"><div><div class="pl-page-links">${button('page','任务看板')} ${button('sources','参考资料')}</div><small>DSH Web · @example/research</small></div><details><summary aria-label="研究工作台的操作">···</summary>${button('dsh-remove','移除')}</details></article>`;
    const toolRow = `<article class="pl-row"><div><strong>pi-statusline</strong><small>Pi</small></div><details><summary aria-label="pi-statusline 的操作">···</summary>${button('pi-remove','移除')}</details></article>`;
    const collection = ['0','both-installed','dsh-installed','pi-installed','partial'].includes(id);
    const assembled = [
        '<div class="pl-top"><div><small>'+device+'</small><h3>插件</h3></div>',
        (hasBoard || hasTool) ? button('add','添加插件') : '', '</div>',
        hasBoard ? boardRow : '', hasTool ? toolRow : '',
        id==='partial' ? '<p>'+escape(state.body[0])+' '+button('pi-installed','重新读取')+'</p>' : '',
        !hasBoard && !hasTool ? '<div class="pl-empty"><p>尚未安装插件</p>'+button('add','添加插件','primary')+'</div>' : ''
    ].join('');
    const workspace = ['page','sources','workspace-chat'].includes(id);
    let content;
    if (collection) content = assembled;
    else if (workspace) {
        const details = id==='page'
            ? '<ul class="gs-board"><li><small>待处理</small>整理访谈笔记</li><li><small>进行中</small>校对研究报告</li><li><small>已完成</small>归档来源清单</li></ul>'
            : id==='sources' ? '<ul><li>用户访谈记录</li><li>研究报告草稿</li></ul>' : '<p>研究报告准备好了吗？</p>';
        content = `<h3>${escape(state.title)}</h3>${details}${mobile ? button('dsh-installed','返回插件') : ''}`;
    } else if (['add','pi','dsh'].includes(id)) {
        const pi = id !== 'dsh';
        const dest = pi ? 'dsh':'pi';
        const index = state.actions.findIndex(a=>a.to===dest);
        const value = pi ? 'pi-statusline':'@example/research';
        content = `<div class="pl-add"><h3>添加插件</h3><label>安装位置<select data-plugin-location="${index}">
            <option selected>${device} · ${pi?'Pi':'DeepSeek Harness · Web'}</option>
            <option>${device} · ${pi?'DeepSeek Harness · Web':'Pi'}</option></select></label>
            <label>示例包<input value="${value}" readonly></label>
            <div class="actions">${button('0','取消')}${button(pi?'pi-review':'dsh-review','继续','primary')}</div></div>`;
    } else {
        const pi = id.startsWith('pi');
        const packageName = pi ? 'pi-statusline' : '@example/research';
        let body = '';
        if (id.endsWith('-review')) body = `<dl class="pl-facts"><dt>插件包</dt><dd>${packageName}</dd><dt>安装到</dt><dd>${device} · ${pi ? 'Pi' : 'DSH Web'}</dd></dl>`;
        const controls = state.actions.filter(a=>!a.preview_only);
        content = `<h3>${escape(state.title)}</h3>${body}${id.endsWith('-review') || id.endsWith('-remove') || id==='offline' ? '<p>'+escape(state.body[0])+'</p>':''}<div class="actions">${controls.map(a=>button(a.to,a.label,a.style)).join('')}</div>`;
    }
    const navItem = (dest,label,active) => active ? `<span aria-current="page">${label}</span>` : button(dest,label) || `<span>${label}</span>`;
    const nav = `<nav aria-label="工作区">${navItem('workspace-chat','对话',id==='workspace-chat')}${navItem('dsh-installed','能力库',!workspace)}${hasBoard ? navItem('page','任务看板',id==='page')+navItem('sources','参考资料',id==='sources'):''}</nav>`;
    return `<div class="device-system"><span>${platform==='mac'?'macOS':platform==='ios'?'iOS':escape(platform)}</span><span>Nola</span></div><div class="plugin-workspace ${mobile?'pl-mobile':''}">${mobile?'':nav}<section class="gs-content pl-detail">${content}</section></div>`;
};
