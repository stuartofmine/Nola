// SPDX-License-Identifier: Apache-2.0
// Synthetic design fixtures only. No Host, model, OAuth, file or network operations.
'use strict';
window.expertRolesModule = (() => {
const catalog = {
  "version": 1,
  "features": {
    "reasoning_effort": true,
    "explicit_generation_reasoning": true,
    "rejoin": true,
    "main_agent_delegation": true
  },
  "templates": [
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000001",
      "role_id": "code-writer",
      "name": {
        "zh": "代码编写",
        "en": "Code writer",
        "fr": "Développement",
        "de": "Codeentwicklung",
        "es": "Programación",
        "ja": "コード作成",
        "ru": "Разработка кода"
      },
      "description": {
        "zh": "代码编写的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for code writer."
      },
      "prompt": "Implement requested changes in the existing codebase. Inspect repository guidance, architecture and call sites before editing. Make focused, compatible changes; preserve user work. Use actual tools for edits, run relevant checks, and explain behavior, validation and remaining limits.",
      "tool_pack": {
        "id": "coding",
        "tools": [
          "code_search",
          "read_file",
          "edit_file",
          "write_file",
          "bash"
        ]
      },
      "skills": [
        {
          "id": "implement-code",
          "name": {
            "zh": "代码实现",
            "en": "Code implementation"
          },
          "instructions": "Inspect scope and existing contracts; plan the smallest coherent change; edit current callers together; verify relevant behavior and report actual evidence."
        }
      ]
    },
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000002",
      "role_id": "code-reviewer",
      "name": {
        "zh": "代码审查",
        "en": "Code reviewer",
        "fr": "Revue de code",
        "de": "Codeprüfung",
        "es": "Revisión de código",
        "ja": "コードレビュー",
        "ru": "Ревью кода"
      },
      "description": {
        "zh": "代码审查的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for code reviewer."
      },
      "prompt": "Review changes for concrete correctness, security, compatibility and maintainability defects. Read surrounding code and trace triggering conditions. Prioritize actionable findings with file locations, impact and reproducible evidence. Do not edit code unless asked. Distinguish verified findings from questions; say when no actionable defects were found.",
      "tool_pack": {
        "id": "code-review",
        "tools": [
          "git_diff",
          "code_search",
          "read_file"
        ]
      },
      "skills": [
        {
          "id": "review-code",
          "name": {
            "zh": "代码审查",
            "en": "Code review"
          },
          "instructions": "Read the diff and relevant implementation; test plausible failure paths; report severity, location, trigger and impact; avoid speculative findings and style-only noise."
        }
      ]
    },
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000003",
      "role_id": "software-tester",
      "name": {
        "zh": "测试",
        "en": "Software tester",
        "fr": "Tests logiciels",
        "de": "Softwaretests",
        "es": "Pruebas de software",
        "ja": "ソフトウェアテスト",
        "ru": "Тестирование ПО"
      },
      "description": {
        "zh": "测试的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for software tester."
      },
      "prompt": "Design and execute tests based on user journeys, contracts, risks and failure recovery. Cover asynchronous ownership, persistence, permissions and cross-platform behavior where relevant. Use isolated data and explicit oracles. Keep unit, fixture, real model, device and visual evidence separate; skipped or untested paths are not passed.",
      "tool_pack": {
        "id": "testing",
        "tools": [
          "read_file",
          "edit_file",
          "write_file",
          "bash"
        ]
      },
      "skills": [
        {
          "id": "test-software",
          "name": {
            "zh": "软件验证",
            "en": "Software verification"
          },
          "instructions": "Define expected behavior independently of implementation, reproduce failures, run meaningful checks with isolated fixtures, inspect actual UI where applicable, and report passed, failed, skipped and untested scope."
        }
      ]
    },
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000004",
      "role_id": "private-secretary",
      "name": {
        "zh": "私人秘书",
        "en": "Personal secretary",
        "fr": "Secrétaire personnel",
        "de": "Persönliches Sekretariat",
        "es": "Secretaría personal",
        "ja": "パーソナル秘書",
        "ru": "Личный секретарь"
      },
      "description": {
        "zh": "私人秘书的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for personal secretary."
      },
      "prompt": "Help organize the user's schedule, reminders, correspondence and follow-ups. Read current authoritative records before planning changes. Preserve time zones and distinguish tentative plans, drafts and committed actions. Obtain the required explicit authorization before sending messages or making external commitments. Show concrete next steps and receipts; do not claim a reminder or email was created without confirmation.",
      "tool_pack": {
        "id": "personal-assistance",
        "tools": [
          "search_calendar_events",
          "create_calendar_event",
          "search_reminders",
          "create_reminder"
        ]
      },
      "skills": [
        {
          "id": "plan-schedule-tasks",
          "name": {
            "zh": "计划与日程",
            "en": "Planning and scheduling"
          },
          "instructions": "Inspect dates, time zones, conflicts and requested scope; prepare a concrete plan; perform only authorized changes and verify persisted events or reminders."
        },
        {
          "id": "assist-email",
          "name": {
            "zh": "邮件助手",
            "en": "Email assistance"
          },
          "instructions": "Draft concise correspondence grounded in verified facts. Sending requires explicit authorization and a provider receipt; a draft is not a sent message."
        }
      ]
    },
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000005",
      "role_id": "visual-artist",
      "name": {
        "zh": "画师",
        "en": "Visual artist",
        "fr": "Artiste visuel",
        "de": "Illustration",
        "es": "Artista visual",
        "ja": "ビジュアルアーティスト",
        "ru": "Художник"
      },
      "description": {
        "zh": "画师的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for visual artist."
      },
      "prompt": "Translate the user's visual intent into a concrete composition, palette, typography and deliverable. Preserve supplied references and requested transparency, dimensions and style. Discover an actually available image-generation, design or editing tool from its schema before using it. Use that tool when the user requests an image; a text prompt is not a delivered image. Inspect the resulting image and report limitations without inventing output.",
      "tool_pack": {
        "id": "visual-art",
        "tools": [
          "tool_search"
        ]
      },
      "skills": [
        {
          "id": "create-visual-art",
          "name": {
            "zh": "视觉创作",
            "en": "Visual creation"
          },
          "instructions": "Clarify only missing essential visual constraints; choose an available generation or editing tool; create the requested artifact, inspect composition and text, and iterate on visible defects. Do not substitute a prompt for an image or claim unavailable generation capabilities."
        }
      ]
    },
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000006",
      "role_id": "presentation-designer",
      "name": {
        "zh": "PPT制作",
        "en": "Presentation designer",
        "fr": "Création de présentations",
        "de": "Präsentationsgestaltung",
        "es": "Diseño de presentaciones",
        "ja": "プレゼンテーション制作",
        "ru": "Создание презентаций"
      },
      "description": {
        "zh": "PPT制作的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for presentation designer."
      },
      "prompt": "Create a clear, editable presentation for the stated audience, objective and speaking time. Establish a narrative and consistent slide system; source factual claims and create appropriate visuals. Use the presentation Skill and actual artifact tools. Render every slide, check overflow, contrast and legibility, and deliver the verified editable file with notes when requested. An outline alone is not a finished deck.",
      "tool_pack": {
        "id": "presentations",
        "tools": [
          "read_file",
          "write_file",
          "bash"
        ]
      },
      "skills": [
        {
          "id": "powerpoint-presentation-authoring",
          "name": {
            "zh": "PowerPoint制作",
            "en": "PowerPoint authoring"
          },
          "instructions": "Load the bundled presentation Skill and its manifest. Build an editable deck, render all slides, correct visible layout defects, validate the final package and deliver the actual artifact."
        }
      ]
    },
    {
      "id": "9f7d0000-4e6f-4c61-8000-000000000007",
      "role_id": "academic-writer",
      "name": {
        "zh": "论文写作",
        "en": "Academic writer",
        "fr": "Rédaction scientifique",
        "de": "Wissenschaftliches Schreiben",
        "es": "Redacción académica",
        "ja": "学術論文執筆",
        "ru": "Научное письмо"
      },
      "description": {
        "zh": "论文写作的提示词、工具组合和配套 Skill。",
        "en": "Prompt, tool pack and workflow Skills for academic writer."
      },
      "prompt": "Assist with rigorous scholarly writing while preserving the author's claims, evidence and voice. Separate literature evidence, user-provided results and inference. Never fabricate references, data, experiments or citations. Check source documents and reference metadata, present uncertainty and methodological limits, and produce the requested manuscript structure, tables and revision notes with traceable sources.",
      "tool_pack": {
        "id": "academic-writing",
        "tools": [
          "web_search",
          "web_fetch",
          "read_file",
          "write_file"
        ]
      },
      "skills": [
        {
          "id": "nature-writing",
          "name": {
            "zh": "论文写作",
            "en": "Scientific writing"
          },
          "instructions": "Load the scholarly-writing Skill; organize research questions, methods, results and discussion around verified evidence; preserve source anchors and mark missing data."
        },
        {
          "id": "nature-ref-verifier",
          "name": {
            "zh": "参考文献核验",
            "en": "Reference verification"
          },
          "instructions": "Check each citation against accessible primary metadata and the actual source; distinguish verified, mismatched and unavailable references and never fill gaps with invented details."
        }
      ]
    }
  ]
};
const profiles = [
    {id:'inherit',name:'继承演示默认模型',efforts:null},
    {id:'five-level',name:'演示推理模型 · 五档',efforts:['minimal','low','medium','high','xhigh']},
    {id:'max-level',name:'演示推理模型 · 含 Max',efforts:['low','medium','high','max']},
    {id:'unknown',name:'未声明推理能力的演示模型',efforts:[]}
];
const defaultProfileID = 'five-level';
const effortNames = {automatic:'自动 · 继承配置',none:'无',minimal:'最少',low:'低',medium:'中',high:'高',xhigh:'更高',max:'最高'};
const experts = [
    {id:'expert-research',name:'研究',description:'核实来源，分清事实与推断',revision:2,defaults:{modelSelectionID:'max-level',selectedPromptTemplateID:'custom-research',expertReasoningEffort:'high',modelOverrides:{'max-level':{temperature:.2}},futureSetting:'preserved'}},
    {id:'expert-writing',name:'写作',description:'保留原意，整理表达',revision:1,defaults:{modelSelectionID:'inherit',selectedPromptTemplateID:'custom-writing'}},
    {id:'expert-reasoning',name:'推理',description:'拆解问题，检查推导',revision:1,defaults:{modelSelectionID:'five-level'}},
    {id:'expert-code',name:'代码助手',description:'代码实现与验证',revision:1,defaults:{modelSelectionID:'five-level',selectedPromptTemplateID:catalog.templates[0].id}}
];
const selections = {}, members = {}, observed = {}, retainedContexts = [];
let editor = null, pendingRole = null, pendingMention = null, counter = 0, highlighted = 0, closing = null;
const title = role => role.name.zh || role.name.en;
const roleFor = id => catalog.templates.find(role=>role.id===id);
const owner = () => current()?.id;
function selected(id=owner()) {
    if(!selections[id]) selections[id] = (conversations.find(c=>c.id===id)?.experts||[]).map(name=>experts.find(e=>e.name===name)?.id).filter(Boolean);
    return selections[id];
}
function membership(id=owner()) {
    if(!members[id]) members[id] = Object.fromEntries((conversations.find(c=>c.id===id)?.experts||[]).map(name=>experts.find(e=>e.name===name)).filter(Boolean).map(e=>[e.id,{revision:e.id==='expert-research'?1:e.revision,contextID:'demo-context:'+id+':'+e.id,participantID:'demo-member:'+id+':'+e.id,displayName:e.name,messages:structuredClone(conversations.find(c=>c.id===id)?.expertHistory?.[e.name]||[])}]));
    return members[id];
}
function efforts(modelID) {
    const profile=profiles.find(p=>p.id===(modelID==='inherit'?defaultProfileID:modelID));
    return [...(profile?.efforts||[])];
}
const action = (text,id,cls='text-action') => `<button type="button" class="${cls}" data-action="er:${id}">${esc(text)}</button>`;
const field = (label,id,value) => `<label class="er-field" for="${id}"><span>${esc(label)}</span><input id="${id}" value="${esc(value)}"></label>`;
function directory() {
    openDialog('专家库',`<h3>系统内置</h3><div class="er-directory">${catalog.templates.map(r=>`<button type="button" class="choice-row" data-action="er:builtin:${r.id}"><div><h3>${esc(title(r))}</h3><p>${esc(r.description.zh||r.description.en)}</p></div></button>`).join('')}</div><h3>我的专家</h3><p class="dialog-note">合成专家目录 · 与当前会话成员分开，所有修改仅影响本页。</p><div class="er-directory">${experts.map(e=>`<button type="button" class="choice-row" data-action="er:edit:${e.id}"><div><h3>${esc(e.name)}</h3><p>${esc(e.description)}</p></div></button>`).join('')}</div><div class="dialog-actions">${action('创建专家','new','primary')}${action('返回选择专家','picker')}</div>`);
}
function builtin(id) {
    const role=roleFor(id);if(!role)return;
    openDialog(title(role),`<p>${esc(role.description.zh||role.description.en)}</p><h3>提示词</h3><p>${esc(role.prompt)}</p><h3>工具</h3><p>${role.tool_pack.tools.map(esc).join(' · ')}</p>${role.skills.map(skill=>`<h3>${esc(skill.name.zh||skill.name.en)}</h3><p>${esc(skill.instructions)}</p>`).join('')}<div class="dialog-actions">${action('返回专家库','manage')}${action('创建可编辑副本','copy:'+id,'primary')}</div>`);
}
function picker() {
    openDialog('选择专家',`<p class="dialog-note">为 Nola 选择专家，也可以在输入框使用 @。Nola 统一分派任务并回复你。</p>${experts.map(e=>`<button type="button" class="choice-row" data-action="er:pick:${e.id}" role="checkbox" aria-checked="${selected().includes(e.id)}"><div><h3>${esc(e.name)}</h3><p>${esc(e.description)}</p></div><span>${selected().includes(e.id)?'已选':''}</span></button>`).join('')}<div class="dialog-actions">${action('专家库','manage')}${action('完成','done','primary')}</div>`);
}
function readEditor() {
    if(!editor||!dialog.querySelector('#er-name')) return;
    editor.name=dialog.querySelector('#er-name').value;
    editor.description=dialog.querySelector('#er-description').value;
    editor.modelID=dialog.querySelector('#er-model').value;
    editor.effort=dialog.querySelector('#er-effort').value;
    editor.previewRole=dialog.querySelector('#er-role').value;
}
function effortOptions() {
    const declared=efforts(editor.modelID),valid=editor.effort==='automatic'||declared.includes(editor.effort);
    return `<option value="automatic" ${editor.effort==='automatic'?'selected':''}>${effortNames.automatic}</option>${declared.map(v=>`<option value="${v}" ${editor.effort===v?'selected':''}>${effortNames[v]||esc(v)}</option>`).join('')}${valid?'':`<option value="${esc(editor.effort)}" selected disabled>当前值不受此模型支持 · ${esc(editor.effort)}</option>`}`;
}
function preview() {
    const role=roleFor(editor.previewRole);
    if(!role) return '<p class="dialog-note">选择角色后先查看提示词、工具组合与 Skill，再决定是否应用。</p>';
    return `<section class="er-role-preview" data-role-preview="${role.role_id}"><p>${esc(role.description.zh||role.description.en)}</p><details><summary>提示词</summary><pre>${esc(role.prompt)}</pre></details><details><summary>工具组合 · ${esc(role.tool_pack.id)}</summary><ul class="er-tools">${role.tool_pack.tools.map(tool=>`<li><code>${esc(tool)}</code></li>`).join('')}</ul><p class="dialog-note">这是工作流程指导，不授予工具权限，也不自动启用能力。</p></details><details><summary>配套 Skill · ${role.skills.length}</summary>${role.skills.map(skill=>`<article><strong>${esc(skill.name.zh||skill.name.en)}</strong><p>${esc(skill.instructions)}</p></article>`).join('')}</details>${action(editor.templateID===role.id?'已应用此角色':'应用角色模板','apply-role','primary')}</section>`;
}
function showEditor() {
    const currentRole=roleFor(editor.templateID);
    openDialog(editor.isNew?'创建专家':'配置专家',`<form id="er-editor" data-expert-id="${editor.id}">${field('专家名称','er-name',editor.name)}<label class="er-field" for="er-description"><span>职责与擅长</span><textarea id="er-description">${esc(editor.description)}</textarea></label><label class="er-field" for="er-model"><span>模型</span><select id="er-model">${profiles.map(p=>`<option value="${p.id}" ${editor.modelID===p.id?'selected':''}>${esc(p.name)}</option>`).join('')}</select></label><label class="er-field" for="er-effort"><span>推理档位</span><select id="er-effort">${effortOptions()}</select></label><p class="dialog-note" id="er-effort-note">${efforts(editor.modelID).length?'档位仅来自所选演示模型的声明；自动表示继承，不默认使用 none。':'此模型未声明推理档位，仅可选择自动。'}</p><label class="er-field" for="er-role"><span>角色模板</span><select id="er-role"><option value="">先选择角色</option>${catalog.templates.map(r=>`<option value="${r.id}" ${editor.previewRole===r.id?'selected':''}>${esc(title(r))}</option>`).join('')}</select></label><p class="er-template-state" id="er-current-template">当前提示词：${esc(currentRole?title(currentRole):editor.templateID?'自定义模板（保留在模板库）':'继承默认模板')}</p><div id="er-role-detail">${preview()}</div><p class="er-error" role="alert" id="er-error"></p><div class="dialog-actions">${action('取消','editor-cancel')}${action('保存专家','save','primary')}</div></form>`);
}
function edit(id) {
    const expert=experts.find(e=>e.id===id);
    if(!editor||editor.id!==id) editor={id:expert?.id||'expert-created-'+(++counter),isNew:!expert,name:expert?.name||'',description:expert?.description||'',modelID:expert?.defaults.modelSelectionID||'inherit',effort:expert?.defaults.expertReasoningEffort||'automatic',templateID:expert?.defaults.selectedPromptTemplateID||'',previewRole:roleFor(expert?.defaults.selectedPromptTemplateID)?.id||''};
    showEditor();
}
function applyRole() {
    const role=roleFor(editor.previewRole); if(!role)return;
    editor.templateID=role.id;
    if(!editor.name.trim())editor.name=title(role);
    if(!editor.description.trim())editor.description=role.description.zh||role.description.en;
    pendingRole=null;showEditor();
}
function save() {
    readEditor();if(!editor.name.trim()){dialog.querySelector('#er-error').textContent='请填写专家名称。';return;}
    if(editor.effort!=='automatic'&&!efforts(editor.modelID).includes(editor.effort)){dialog.querySelector('#er-error').textContent='请选择此模型声明的档位，或使用自动。';return;}
    let expert=experts.find(e=>e.id===editor.id);
    if(!expert){expert={id:editor.id,revision:0,defaults:{}};experts.push(expert);}
    expert.name=editor.name.trim();expert.description=editor.description.trim();expert.revision++;
    expert.defaults={...expert.defaults,modelSelectionID:editor.modelID};
    if(editor.templateID)expert.defaults.selectedPromptTemplateID=editor.templateID;else delete expert.defaults.selectedPromptTemplateID;
    if(editor.effort==='automatic')delete expert.defaults.expertReasoningEffort;else expert.defaults.expertReasoningEffort=editor.effort;
    editor=null;directory();toast('专家配置已更新；已有上下文保留原快照。');
}
function query(input) {return /(?:^|\s)@([^\s@]*)$/u.exec(input?.value||'');}
function choose(id,input=document.querySelector('#compose')) {
    const expert=experts.find(e=>e.id===id);if(!expert)return;
    const idOwner=owner(),old=membership(idOwner)[id];
    const intent={id,revision:expert.revision,owner:idOwner,text:input?.value??null};
    if(old&&old.revision!==expert.revision){pendingMention=intent;openDialog('使用专家的最新配置？',`<p>按最新模型、档位和角色创建新的专家上下文。之前的回复与上下文继续保留，不改写历史。</p><div class="dialog-actions">${action('保留原配置','rejoin-cancel')}${action('以最新配置重新引入','rejoin-confirm','primary')}</div>`);return;}
    commitMention(intent,false);
}
function commitMention(intent,rejoin) {
    const expert=experts.find(e=>e.id===intent.id);
    if(!expert||expert.revision!==intent.revision||owner()!==intent.owner){toast('专家或会话已变化，请重新选择。');return;}
    const map=membership(intent.owner),old=map[expert.id];
    if(rejoin&&old)retainedContexts.push({...old,agentID:expert.id,conversationID:intent.owner,messages:structuredClone(old.messages||[])});
    (observed[intent.owner] ||= {})[expert.id]=intent.revision;
    if((!old||rejoin)&&current().messages.length)map[expert.id]={revision:expert.revision,contextID:'demo-context:'+intent.owner+':'+expert.id+':'+expert.revision,participantID:'demo-member:'+intent.owner+':'+expert.id+':'+expert.revision,displayName:expert.name,messages:[]};
    if(!selected(intent.owner).includes(expert.id))selected(intent.owner).push(expert.id);
    const names=selected(intent.owner).map(id=>experts.find(e=>e.id===id)?.name).filter(Boolean);
    selectedExperts=[...names];current().experts=[...names];
    const compose=document.querySelector('#compose');
    if(intent.text!==null&&compose?.value===intent.text){const match=query({value:intent.text});if(match){const at=match.index+(match[0].startsWith('@')?0:1);compose.value=intent.text.slice(0,at);drafts[intent.owner]=compose.value;}}
    pendingMention=null;if(dialog.open)closeDialog();render();document.querySelector('#compose')?.focus();if(rejoin)toast('演示专家已重新引入，历史回复保留。');
}
function paintComposer() {
    const input=document.querySelector('#compose');if(!input)return;
    const form=input.closest('form')||input.parentElement;
    form.querySelector('.er-selected-experts')?.remove();form.querySelector('.er-mentions')?.remove();
    const ids=selected(),chips=document.createElement('div');chips.className='er-selected-experts';chips.setAttribute('aria-label','已选择的专家');
    chips.innerHTML=ids.map(id=>{const e=experts.find(e=>e.id===id);return e?`<span class="er-expert-chip" data-expert-id="${id}">${esc(e.name)}<button type="button" data-er-remove="${id}" aria-label="取消选择${esc(e.name)}">×</button></span>`:'';}).join('');
    if(ids.length){form.prepend(chips);chips.querySelectorAll('[data-er-remove]').forEach(b=>b.onclick=()=>{selections[owner()]=selected().filter(id=>id!==b.dataset.erRemove);selectedExperts=selected().map(id=>experts.find(e=>e.id===id)?.name).filter(Boolean);current().experts=[...selectedExperts];paintComposer();input.focus();});}
    const match=query(input);if(!match)return;
    const filtered=experts.filter(e=>!match[1]||e.name.includes(match[1]));highlighted=Math.min(highlighted,Math.max(0,filtered.length-1));
    const panel=document.createElement('div');panel.className='er-mentions';panel.setAttribute('role','listbox');panel.setAttribute('aria-label','选择专家');
    panel.innerHTML=filtered.map((e,i)=>`<button type="button" role="option" aria-selected="${i===highlighted}" data-er-mention="${e.id}"><strong>${esc(e.name)}</strong><small>${esc(e.description)}</small></button>`).join('')||'<p>没有匹配的专家</p>';
    form.prepend(panel);panel.querySelectorAll('[data-er-mention]').forEach(b=>b.onclick=()=>choose(b.dataset.erMention,input));
}
function bind(root) {
    if(!dialog.dataset.expertRoleCancelBound){dialog.dataset.expertRoleCancelBound='true';dialog.addEventListener('cancel',event=>{if(handle('close'))event.preventDefault();});}
    if(root.querySelector('#er-editor')) {
        root.querySelector('#er-role').onchange=()=>{readEditor();root.querySelector('#er-role-detail').innerHTML=preview();window.bind?.(root);};
        root.querySelector('#er-model').onchange=()=>{readEditor();root.querySelector('#er-effort').innerHTML=effortOptions();root.querySelector('#er-effort-note').textContent=efforts(editor.modelID).length?'档位仅来自所选演示模型的声明。':'未声明推理档位，仅可选择自动。';};
        root.querySelector('#er-editor').onsubmit=e=>{e.preventDefault();save();};
    }
    const input=root.querySelector('#compose');if(!input)return;
    if(!input.dataset.expertRoleBound){input.dataset.expertRoleBound='true';input.addEventListener('input',()=>{highlighted=0;paintComposer();});input.addEventListener('keydown',e=>{const panel=input.closest('form')?.querySelector('.er-mentions');if(!panel)return;const candidates=[...panel.querySelectorAll('[data-er-mention]')];if(['ArrowDown','ArrowUp','Enter','Escape'].includes(e.key)){e.preventDefault();e.stopImmediatePropagation();if(e.key==='Escape'){panel.remove();return;}if(e.key==='Enter'){if(candidates[highlighted])choose(candidates[highlighted].dataset.erMention,input);return;}highlighted=(highlighted+(e.key==='ArrowDown'?1:-1)+candidates.length)%Math.max(1,candidates.length);paintComposer();}},true);}
    paintComposer();
}
function handle(a) {
    if(a==='close'){
        if(pendingRole){pendingRole=null;showEditor();return true;}
        if(pendingMention){pendingMention=null;closeDialog();document.querySelector('#compose')?.focus();return true;}
        if(dialog.querySelector('#er-editor')){handle('er:editor-cancel');return true;}
        if(closing){editor=closing;closing=null;showEditor();return true;}
    }
    if(a==='im:agents'){directory();return true;}
    if(a==='experts'||a==='im:participants'){picker();return true;}
    if(!a.startsWith('er:'))return false;
    const [kind,id]=a.slice(3).split(':');
    if(kind==='builtin')builtin(id);if(kind==='copy'){editor=null;edit();editor.previewRole=id;applyRole();}if(kind==='manage')directory();if(kind==='picker')picker();if(kind==='edit')edit(id);if(kind==='new'){editor=null;edit();}
    if(kind==='done'){closeDialog();render();}if(kind==='pick'){
        if(selected().includes(id)){selections[owner()]=selected().filter(value=>value!==id);selectedExperts=selected().map(value=>experts.find(e=>e.id===value)?.name).filter(Boolean);current().experts=[...selectedExperts];picker();}
        else choose(id,null);
    }
    if(kind==='save')save();
    if(kind==='apply-role'){readEditor();if(editor.templateID&&editor.templateID!==editor.previewRole){pendingRole=editor.previewRole;openDialog('替换当前提示词模板？',`<p>专家将改用所选角色。自定义模板仍保留在模板库，不会删除。</p><div class="dialog-actions">${action('保留当前模板','role-cancel')}${action('应用角色模板','role-confirm','primary')}</div>`);}else applyRole();}
    if(kind==='role-cancel'){pendingRole=null;showEditor();}if(kind==='role-confirm'){editor.previewRole=pendingRole;applyRole();}
    if(kind==='editor-cancel'){readEditor();closing=editor;openDialog('放弃未保存的配置？',`<p>已保存的专家配置不会变化。</p><div class="dialog-actions">${action('继续编辑','continue')}${action('放弃修改','discard','primary')}</div>`);}
    if(kind==='continue'){editor=closing;closing=null;showEditor();}if(kind==='discard'){editor=null;closing=null;directory();}
    if(kind==='rejoin-cancel'){pendingMention=null;closeDialog();document.querySelector('#compose')?.focus();}
    if(kind==='rejoin-confirm'&&pendingMention)commitMention(pendingMention,true);
    return true;
}
function childContexts(parentID) {
    const active=Object.entries(membership(parentID)).map(([agentID,m])=>({...m,agentID,parentID,isCurrent:true}));
    return [...active,...retainedContexts.filter(c=>c.conversationID===parentID).map(c=>({...c,parentID,isCurrent:false}))];
}
function continueChild(parentID,contextID) {
    const child=childContexts(parentID).find(c=>c.contextID===contextID&&c.isCurrent);
    if(!child){toast('旧配置历史仅可阅读。');return false;}
    if(!selected(parentID).includes(child.agentID))selected(parentID).push(child.agentID);
    (observed[parentID] ||= {})[child.agentID]=child.revision;
    const c=conversations.find(c=>c.id===parentID);
    c.experts=selected(parentID).map(id=>experts.find(e=>e.id===id)?.name).filter(Boolean);
    selectedExperts=[...c.experts];
    return true;
}
function validateSend() {
    const id=owner();
    for(const agentID of selected(id)){
        const revision=observed[id]?.[agentID],member=membership(id)[agentID];
        if(revision!==undefined&&(member?member.revision!==revision:experts.find(e=>e.id===agentID)?.revision!==revision)){toast('专家上下文已变化，请重新打开专家选择。');return false;}
    }
    return true;
}
function sendContext(id) {
    const map=membership(id);
    for(const agentID of selected(id))if(!map[agentID]){const e=experts.find(e=>e.id===agentID);if(e)map[agentID]={revision:e.revision,contextID:'demo-context:'+id+':'+e.id+':'+e.revision,participantID:'demo-member:'+id+':'+e.id+':'+e.revision,displayName:e.name,messages:[]};}
    return {mainExpertSelection:{version:1,selected_experts:selected(id).map(agentID=>({agent_id:agentID,agent_revision:map[agentID]?.revision,participant_id:map[agentID]?.participantID||null,context_conversation_id:map[agentID]?.contextID||null}))}};
}
return {demoMainReply:id=>(selections[id]||[]).includes('expert-code')?'我请代码审查专家核对了重试与取消流程，并整理出两处优先修改的问题。\n\n1. 取消后，较早请求的回调仍可能覆盖新结果。应按会话与请求版本校验，再提交状态。\n\n2. 保存失败时，编辑稿被提前清空。应收到持久保存回执后再消费草稿，失败保留输入。\n\n建议先修这两处，再验证取消后重试、保存失败恢复和快速切换会话。专家的核对记录可在本会话的任务详情中查看。': '我整理了专家返回的核对结果。先保留用户正在做的事与现有草稿，再把需要确认的权限和失败恢复放到对应操作旁；详细核对记录可在本会话的专家任务中查看。',handle,bind,validateSend,sendContext,childContexts,continueChild,fixtures:{catalog,profiles,defaultProfileID},snapshot:()=>({experts:structuredClone(experts),selections:structuredClone(selections),members:structuredClone(members),retainedContexts:structuredClone(retainedContexts)})};
})();
