'use strict';
window.sidebarNavigation = (() => {
    let leftQuery = '';
    const paths = {
        puzzle: 'M11 4a2 2 0 1 1 4 0h3a2 2 0 0 1 2 2v3a2 2 0 1 1 0 4v3a2 2 0 0 1-2 2h-3a2 2 0 1 1-4 0H6a2 2 0 0 1-2-2v-3a2 2 0 1 1 0-4V6a2 2 0 0 1 2-2Z',
        brain: 'M9.5 4A3.5 3.5 0 0 0 6 7.5c0 .4.1.8.2 1.1A3.5 3.5 0 0 0 4 12a3.5 3.5 0 0 0 1.4 2.8A3.5 3.5 0 0 0 9 19.5c1 0 2-.4 2.7-1.1V5.2A3.5 3.5 0 0 0 9.5 4Zm5 0a3.5 3.5 0 0 0-2.2 1.2v13.2a3.5 3.5 0 0 0 2.7 1.1 3.5 3.5 0 0 0 3.6-4.7A3.5 3.5 0 0 0 20 12a3.5 3.5 0 0 0-2.2-3.4c.1-.3.2-.7.2-1.1A3.5 3.5 0 0 0 14.5 4Z',
        progress: 'M5 5h2m3 0h9M5 12h2m3 0h9M5 19h2m3 0h9',
        artifacts: 'M8 3h11v15H8zM5 7H3v14h12v-1',
        file: 'M14 2H5v20h14V7Zm0 0v6h5M8 12h8M8 16h6'
    };
    const icon = n => paths[n] ? `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[n]}"/></svg>` : svg(n);
    const sections = [
        ['chat', '对话', 'chat'],
        ['extensions', '专家与技能', 'puzzle'],
        ['automations', '自动化中心', 'clock'],
        ['memory', '记忆与数据源', 'brain']
    ];
    const toolAreas = [
        ['任务', 'task', 'goal'],
        ['产物', 'artifacts', 'artifacts'],
        ['上下文', 'context', 'brain']
    ];
    const areaTitles = {task: '任务', artifacts: '产物', context: '上下文'};

    function conversationRow(c, currentSection) {
        const active = currentSection === 'chat' && activeId === c.id;
        return `<button class="history-item ${active ? 'active' : ''}" data-action="dk-select:${esc(c.id)}"><span class="history-row-top"><strong>${esc(c.title)}</strong><small>${esc(c.time)}</small></span><span class="history-row-sub">${esc(c.preview || '开始新对话')}</span>${c.experts?.length ? `<span class="history-row-experts">${icon('person')}${esc(c.experts.join(' · '))}</span>` : ''}</button>`;
    }

    function sidebar() {
        const snap = architectureModule.snapshot();
        const q = leftQuery.trim().toLowerCase();
        const filtered = conversations.filter(c => !q || (c.title + ' ' + c.preview).toLowerCase().includes(q));
        const pinned = filtered.filter(c => c.id === 'design');
        const recent = filtered.filter(c => c.id !== 'design');
        return `<nav class="rail-sections" aria-label="主导航">${sections.map(([id, label, ic]) => `<button class="nav-item ${snap.section === id ? 'active' : ''}" data-action="sn-section:${id}" ${snap.section === id ? 'aria-current="page"' : ''}>${icon(ic)}<span>${label}</span></button>`).join('')}</nav><div class="rail-divider" role="separator"></div><label class="rail-search">${svg('search')}<input id="left-search" aria-label="搜索对话" placeholder="搜索对话" value="${esc(leftQuery)}" autocomplete="off">${leftQuery ? `<button type="button" data-action="sn-clear-search" title="清除搜索" aria-label="清除搜索">${svg('close')}</button>` : ''}</label><div class="rail-history">${pinned.length ? `<div class="rail-caption">置顶</div>${pinned.map(c => conversationRow(c, snap.section)).join('')}` : ''}${recent.length ? `<div class="rail-caption">最近</div>${recent.map(c => conversationRow(c, snap.section)).join('')}` : (!pinned.length ? '<p class="muted" style="padding:12px;font-size:12px">没有匹配的对话</p>' : '')}</div>`;
    }

    function tools(selectedArea) {
        return `<nav class="icon-strip context-tools grouped-tools" aria-label="对话工具">${toolAreas.map(([label, area, ic]) => `<button class="strip-entry tool-group ${selectedArea === area ? 'selected' : ''}" data-action="sn-group:${label}" aria-label="${label}" title="${label}" aria-pressed="${selectedArea === area}">${icon(ic)}<span class="tool-group-label">${label}</span><span class="entry-tooltip" role="tooltip">${label}</span></button>`).join('')}</nav>`;
    }

    function right(selectedArea) {
        return `<div class="panel-caption panel-caption-compact"><strong data-inspector-area-title>${esc(areaTitles[selectedArea] || '任务')}</strong><button class="strip-close" aria-label="关闭工作面板" title="关闭工作面板" data-action="dk-inspector">${svg('close')}</button></div>`;
    }

    function handle(a) {
        if (a === 'sn-clear-search') {
            leftQuery = '';
            render();
            document.querySelector('#left-search')?.focus();
            return true;
        }
        if (a.startsWith('sn-section:')) {
            architectureModule.openSection(a.split(':')[1]);
            return true;
        }
        if (a.startsWith('sn-group:')) {
            const label = a.split(':')[1];
            const area = toolAreas.find(item => item[0] === label)?.[1] || 'task';
            architectureModule.toggleInspector(area);
            return true;
        }
        return false;
    }

    function bind() {
        const search = document.querySelector('#left-search');
        if (search) {
            search.oninput = () => {
                leftQuery = search.value;
                const pos = search.selectionStart;
                render();
                const next = document.querySelector('#left-search');
                next?.focus();
                next?.setSelectionRange(pos, pos);
            };
        }
        document.querySelectorAll('.icon-strip').forEach(strip => {
            const buttons = [...strip.querySelectorAll('button')];
            buttons.forEach((button, i) => {
                button.onkeydown = e => {
                    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
                    e.preventDefault();
                    const n = e.key === 'Home' ? 0 : e.key === 'End' ? buttons.length - 1 : (i + (e.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length;
                    buttons[n].focus();
                };
            });
        });
    }

    return {sidebar, right, tools, handle, bind, areaTitles};
})();
