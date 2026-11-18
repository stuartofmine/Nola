'use strict';
window.organizerModule = (() => {
    const b = (t, a, c = 'text-action') => `<button class="${c}" data-action="${a}">${t}</button>`;
    const section = (t, body) => `<section class="work-section"><h3>${t}</h3>${body}</section>`;
    function data() {
        return {
            d: deliverablesModule.snapshot(),
            g: deliveryModule.snapshot(),
            pending: current().messages.some(m => m.permission === 'pending')
        };
    }

    function review(inCanvas) {
        const {d} = data(), r = d.review;
        if (!r) return `<div class="work-empty"><h2>没有代码变更</h2><p>这段对话没有关联工作区改动，不为它建立审阅任务。</p></div>`;
        const f = r.files[r.file];
        return `<div class="work-status"><strong>${r.status}</strong><span>${r.files.length} 文件 · ${d.openFindings} 发现待处理</span></div>${section('变更文件', r.files.map((file, i) => `<button class="work-row ${i === r.file ? 'selected' : ''}" data-action="rv-file:${i}"><span>${esc(file.name)}</span><small>+${file.added} −${file.removed}</small></button>`).join(''))}${section('选中差异', `<h4>${esc(f.hunks[r.hunk].title)}</h4><pre class="work-diff">${esc(f.hunks[r.hunk].code)}</pre>${b('审阅工作区更改', 'rv-open-review', 'primary')}`)}${section('审阅发现', r.findings.map(x => `<button class="work-finding" data-action="rv-locate:${x.id}"><span class="work-severity">${esc(x.severity)}</span><span>${esc(x.title)}<small>${x.resolved ? '已解决' : '待处理'}</small></span>${svg('chevron')}</button>`).join(''))}`;
    }

    function artifacts() {
        return deliverablesModule.panel('产物');
    }

    function progress(inCanvas) {
        const {g, pending} = data();
        const chatPending = manageModule.snapshot().activities.find(a => a.chat === activeId && ['待确认', '待补充'].includes(a.status));
        if (!g.goal) {
            return `${chatPending ? `<div class="work-status"><strong>${esc(chatPending.status)}</strong>${b('查看当前请求', 'mg-activity:' + chatPending.id)}</div>` : ''}<p class="inspector-empty">当前会话暂无计划或待办</p>`;
        }
        return `<div class="work-status"><strong>${g.completed}/${g.steps.length} 步完成</strong><span>${esc(g.goal.milestone)}</span></div>${section('当前执行与计划', g.steps.map((s, i) => `<button class="work-step" data-action="gp-detail:${s.id}"><span class="work-number">${s.state === '已完成' ? '✓' : i + 1}</span><span><strong>${esc(s.name)}</strong><small>${esc(s.dep)}</small></span><em>${esc(s.state)}</em></button>`).join(''))}${inCanvas ? '<p class="work-note">执行记录已打开。选择步骤可查看依赖。</p>' : `<div class="work-next compact"><span class="work-kicker">${pending ? '等待确认' : '当前步骤'}</span><h3>${pending ? '保存请求尚未处理' : g.goal.paused ? '检查已暂停' : g.goal.status === 'running' ? '检查进行中' : g.goal.status === 'done' ? '检查已完成' : '检查中断，已有结果保留'}</h3>${b(pending ? '查看保存请求' : g.goal.status === 'running' ? (g.goal.paused ? '继续演示' : '暂停演示') : g.goal.status === 'done' ? '重新演示失败与恢复' : '重试检查', pending ? 'mg-activity:attention' : g.goal.status === 'running' ? (g.goal.paused ? 'gp-resume' : 'gp-pause') : g.goal.status === 'done' ? 'gp-reset' : 'gp-run', 'primary')}</div>${b('查看完整执行日志', 'gp-canvas')}`}`;
    }

    function goals(inCanvas) {
        const {g} = data();
        if (!g.goal) return deliveryModule.panel('目标');
        return `<div class="work-status"><strong>交付目标</strong><span>${esc(g.goal.priority)}优先级</span></div><h2 class="work-goal">${esc(g.goal.objective)}</h2>${!inCanvas ? `<p class="work-scope">${esc(g.goal.scope)}</p>${b('编辑目标与范围', 'gp-edit')}` : ''}${section('完成要求 · ' + g.criteria.filter(x => x.done).length + '/' + g.criteria.length, g.criteria.map(c => `<button class="work-criterion" data-action="${c.action}"><span>${c.done ? '✓' : '○'}</span><span>${esc(c.name)}</span>${svg('chevron')}</button>`).join(''))}<div class="work-milestone"><span class="meta">工作记录</span><p>${esc(g.goal.milestone)}</p></div>`;
    }

    function canvas() {
        const {g, pending} = data();
        if (!g.goal) return `<div class="studio-document">${deliveryModule.panel('目标')}</div>`;
        return `<div class="studio-document"><div class="eyebrow">执行详情 · 本地演示</div><h1>${esc(g.goal.milestone)}</h1><p class="muted" style="margin-top:12px">输入：目标范围与来源产物。输出：边界检查记录，不运行真实项目测试。</p><div class="section-heading"><h2>完整执行日志</h2><span class="meta">${g.goal.logs.length} 条</span></div>${g.goal.logs.map(l => `<div class="work-log">${esc(l)}</div>`).join('')}<div class="work-next compact" style="margin-top:24px"><h3>${pending ? '等待保存授权' : g.goal.paused ? '执行已暂停' : g.goal.status === 'done' ? '演示检查已完成' : '执行与恢复'}</h3>${b(pending ? '查看保存请求' : g.goal.status === 'running' ? (g.goal.paused ? '继续演示' : '暂停演示') : g.goal.status === 'done' ? '重新演示失败与恢复' : '重试检查', pending ? 'mg-activity:attention' : g.goal.status === 'running' ? (g.goal.paused ? 'gp-resume' : 'gp-pause') : g.goal.status === 'done' ? 'gp-reset' : 'gp-run', 'primary')}</div><p class="demo-banner">选择步骤可查看输入、输出和依赖。</p></div>`;
    }

    return {
        canvas,
        panel: (name, canvasKind) => name === '审阅' ? review(canvasKind === 'review') : name === '产物' ? artifacts() : name === '进度' ? progress(canvasKind === 'progress') : name === '目标' ? goals(canvasKind === 'progress') : null
    };
})();
