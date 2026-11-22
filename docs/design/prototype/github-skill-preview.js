// SPDX-License-Identifier: Apache-2.0
// Content for the GitHub Skill scenarios in the shared review shell.
window.githubSkillPreview = (() => {
    const repository = 'Yuan1z0825/nature-skills';
    const skill = 'nature-paper-card';
    const directory = 'skills/' + skill;
    const pin = 'a65a8364b419e05f7d8d5d96842a1ff7e544b1bb';
    const instructions = '阅读论文原文，整理研究问题、实验方法、证据与局限。\n使用共享术语表统一论文中的术语。';
    const ownerNames = {ios:'这部 iPhone', android:'这部手机', mac:'这台 Mac', windows:'这台 Windows 电脑', web:'当前 Web Host', cli:'当前 CLI Host'};
    const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${window.nolaIconPaths[name]}"/></svg>`;
    function sourceInfo() {
        return `<details class="gs-source"><summary>${icon('file')}GitHub · ${repository}</summary><code>https://github.com/${repository}/tree/${pin}/${directory}</code></details>`;
    }
    function render(scene, state, platform, memory, actions, escape) {
        const paired = scene.id === 'github-skill-host';
        const owner = paired ? '书房 Mac · 配对 Host' : ownerNames[platform];
        const cli = platform === 'cli';
        const settings = ['settings','off'].includes(state.id);
        const request = '帮我找 Nature Skills 里的论文精读卡，装到' + (paired ? '书房 Mac' : owner) + '。';
        const reviewRef = memory.pinnedSelection ? pin : 'main';
        const panel = (title, detail) => `<div class="gs-package">${icon('file')}<div><strong>${title}</strong><small>${detail}</small></div></div>`;
        const title = `<h3>${escape(state.title)}</h3>`;
        let content;
        switch (state.id) {
        case '0':
            content = title + `<p class="gs-prompt">${request}</p>`;
            break;
        case 'choose':
            content = `<p class="gs-prompt">${request}</p>` + title + panel(skill, '整理研究问题、方法与证据') + `<p class="gs-origin">GitHub · ${repository}</p>`;
            break;
        case 'review':
            content = title + `<p>${escape(state.body[0])}</p><dl class="gs-facts"><div><dt>安装到</dt><dd>${owner}</dd></div><div><dt>GitHub 来源</dt><dd>${repository}<small class="gs-directory">${directory}</small><details class="gs-ref"><summary>查看完整链接 · ${reviewRef.slice(0, 7)}</summary><code>https://github.com/${repository}/tree/${reviewRef}/${directory}</code></details></dd></div></dl>`;
            break;
        case 'installed':
            content = title + panel(skill, owner) + `<p>${escape(state.body[0])}</p>`;
            break;
        case 'read':
            content = `<p class="gs-prompt">调用刚装的精读卡，读一下随包术语表的英文标题。</p>` + title + '<p>术语表的英文标题是 <strong lang="en">Terminology Ledger</strong>。</p><p class="gs-citation">随包文件 · <code>nature-shared/core/terminology-ledger.md</code></p>';
            break;
        case 'settings':
        case 'off': {
            const current = memory.instructions ?? instructions;
            const editor = `<details class="gs-editor"><summary>编辑技能指令</summary><form data-skill-editor><label>技能指令<textarea name="instructions" required>${escape(current)}</textarea></label><div class="actions"><button type="submit" class="primary">保存</button><button type="button" data-skill-cancel>取消</button></div></form></details>`;
            content = title + panel(skill, state.id === 'off' ? '已停用' : '已启用') + sourceInfo() + (cli ? '<pre>$ nola skills list</pre>' : editor);
            break;
        }
        default:
            content = title + `<p>${escape(state.body[0])}</p>`;
        }
        const section = settings ? 'Skills 与 MCP' : state.id === 'read' ? '新会话' : '安装论文精读卡';
        return `<div class="device-system"><span>${platform === 'ios' ? 'iOS' : ({mac:'macOS',android:'Android',windows:'Windows · Tauri',cli:'CLI',web:'Web'})[platform]}</span><span>原型预览</span></div><header class="gs-bar"><strong>${cli ? 'Nola CLI' : section}</strong><span>${owner}</span></header><section class="gs-content${cli ? ' gs-terminal' : ''}">${content}<div class="actions">${actions}</div><p class="gs-feedback" role="status"></p></section>`;
    }
    return {render};
})();
