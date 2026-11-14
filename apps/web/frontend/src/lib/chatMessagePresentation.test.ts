import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { messagePresentation, messageFlowPresentation, isExecutionPart, executionProcessExpanded, executionProcessChoice, liveReasoningParts } from './chatMessagePresentation.ts';
import type { TimelinePart } from './chatTimeline.ts';
const text = (id: string, markdown: string): TimelinePart => ({source: 'canonical', id, value: {kind: 'text', part_id: id, revision: 1, markdown}});
const tool = (id: string, state = 'completed'): TimelinePart => ({source: 'canonical', id, value: {kind: 'tool', execution_source: {kind: 'model'}, part_id: id, revision: 1, title: id, state}});

test('process partition keeps typed reasoning/tool order without swallowing final prose or errors', () => {
    const reasoning: TimelinePart = {source: 'canonical', id: 'reason', value: {kind: 'reasoning', part_id: 'reason', revision: 1, summary: 'check'}};
    const parts = [reasoning, tool('one'), {...reasoning, id: 'verify'}, text('answer', 'done'),
        {source: 'canonical' as const, id: 'error', value: {kind: 'error', part_id: 'error', revision: 1, code: 'failed', display_key: 'unknown.failure', recoverable: false}}];
    assert.deepEqual(parts.filter(isExecutionPart).map(part => part.id), ['reason', 'one', 'verify']);
    assert.deepEqual(parts.filter(part => !isExecutionPart(part)).map(part => part.id), ['answer', 'error']);
    assert.equal(executionProcessExpanded(null, false), false);
    assert.equal(executionProcessExpanded(false, true), false);
    assert.equal(executionProcessExpanded(true, false), true);
    assert.equal(executionProcessExpanded(null, true), true);
});

test('real process component uses one localized disclosure before the answer', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const message = {id: 'process', role: 'assistant', state: 'running', canonical: null,
                parts: [{source: 'canonical', id: 'r', value: {kind: 'reasoning', summary: 'REASONING-MARKER'}}, tool('one'), text('answer', 'ANSWER-MARKER')]};
            const html = render(Component, {props: {ownerKey:"fixture-owner",message}}).body;
            assert.ok(html.includes(t(language, 'chatPage.processing')));
            assert.ok(html.indexOf('REASONING-MARKER') < html.indexOf('ANSWER-MARKER'));
            assert.equal((html.match(/class="execution-process /g) ?? []).length, 1);
        }
    } finally { await server.close(); }
});

test('only adjacent completed runs of three or more collapse, retaining all original boundaries and order', () => {
    const parts = [text('intro', 'Before'), tool('one'), tool('two'), text('between', 'Between'), tool('three'), tool('four'), tool('five'), tool('active', 'running'), text('after', 'After')];
    assert.deepEqual(messagePresentation(parts).map(group => [group.id, group.kind]), [
        ['intro', 'text'], ['one', 'part'], ['two', 'part'], ['between', 'text'], ['three', 'tools'], ['active', 'part'], ['after', 'text'],
    ]);
    for (const state of ['pending', 'running', 'waiting_for_input', 'waiting_for_permission', 'failed', 'cancelled', 'sealed', 'unknown']) {
        assert.ok(messagePresentation([tool('one'), tool('boundary', state), tool('two')]).every(group => group.kind === 'part'));
    }
});
test('failed replies remain visible without prose, and explicit failure details are escaped and localized', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const message = {id: 'failed', role: 'assistant', state: 'failed', canonical: null, parts: []};
            const empty = render(Component, {props: {ownerKey:"fixture-owner",message}}).body;
            assert.ok(empty.includes(t(language, 'chatPage.replyError_failed')));
            const explicit = render(Component, {props: {ownerKey:"fixture-owner",message: {...message, parts: [{source: 'canonical', id: 'error',
                value: {kind: 'error', part_id: 'error', revision: 1, code: '<script>failure</script>', display_key: 'untranslated.core.error', recoverable: true}}]}}}).body;
            assert.equal((explicit.match(/data-kind="failed"/g) ?? []).length, 1);
            assert.ok(!explicit.includes('<script>failure</script>'));
            assert.ok(explicit.includes('&lt;script>failure&lt;/script>'));
            assert.ok(explicit.includes(t(language, 'chatPage.partFailed')));
            assert.ok(explicit.indexOf('untranslated.core.error') > explicit.indexOf('<details'));
            assert.doesNotMatch(explicit, /<details[^>]*\bopen\b/);
            assert.doesNotMatch(explicit, /<button/);
            const known = render(Component, {props: {ownerKey:"fixture-owner",message: {...message, parts: [{source: 'canonical', id: 'error',
                value: {kind: 'error', part_id: 'error', revision: 1, code: 'typed-code', display_key: 'chatPage.toolFailed', recoverable: false}}]}}}).body;
            assert.ok(known.includes(t(language, 'chatPage.toolFailed')));
            assert.ok(!known.includes(t(language, 'chatPage.partFailed')));
            const normal = render(Component, {props: {ownerKey:"fixture-owner",message: {...message, state: 'running'}}}).body;
            assert.ok(!normal.includes('data-kind="failed"'));
        }
    } finally { await server.close(); }
});
test('Markdown joins adjacent text only, and undisclosed text cannot bridge a boundary', () => {
    assert.deepEqual(messagePresentation([text('a', '**hello'), text('b', '**')]), [{id: 'a', kind: 'text', text: '**hello**'}]);
    const pending = text('private', 'PRIVATE'); pending.value.review = 'pending';
    const result = messagePresentation([text('a', 'First'), pending, text('b', 'Last')]);
    assert.deepEqual(result.map(group => group.kind), ['text', 'part', 'text']);
    assert.ok(!result.some(group => group.kind === 'text' && group.text.includes('PRIVATE')));
});
test('public prose remains outside the process disclosure while activity preserves tool order', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        const parts = [text('intro', 'BEFORE_SENTINEL'), tool('LIVE_SENTINEL', 'running'), text('after', 'AFTER_SENTINEL'), tool('one'), tool('two'), tool('three')];
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Component, {props: {ownerKey:"fixture-owner",message: {id: 'message', role: 'assistant', state: 'running', canonical: null, parts}}}).body;
            assert.ok(html.indexOf('BEFORE_SENTINEL') < html.indexOf('<details class="execution-process'));
            assert.ok(html.indexOf('LIVE_SENTINEL') < html.indexOf('AFTER_SENTINEL'));
            assert.ok(html.indexOf('AFTER_SENTINEL') < html.lastIndexOf('<details class="execution-process'));
            assert.ok(html.indexOf('BEFORE_SENTINEL') < html.indexOf('AFTER_SENTINEL'));
            assert.ok(html.includes(t(language, 'chatPage.completedTools')));
            assert.equal((html.match(/<details/g) ?? []).length, 7); // Two process segments, four tool disclosures, and a completed-tool group.
        }
    } finally { await server.close(); }
});

test('live reasoning stays in compact process history as the public body grows', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const reasoning: TimelinePart = {source: 'canonical', id: 'r', value: {kind: 'reasoning', part_id: 'r', revision: 1, state: 'running', summary: 'LIVE-ONE'}};
        const message = {id: 'response', role: 'assistant', state: 'running', canonical: null, parts: [reasoning, tool('read', 'running')]};
        for (const summary of ['LIVE-ONE', 'LIVE-ONE LIVE-TWO']) {
            reasoning.value.summary = summary;
            const html: string = render(Component, {props: {ownerKey:"fixture-owner",message}}).body;
            assert.ok(html.indexOf(summary) > html.indexOf('<details class="execution-process'));
            assert.doesNotMatch(html, /<details class="execution-process[^>]*\bopen\b/);
            assert.equal(html.split(summary).length - 1, 1);
        }
        const html: string = render(Component, {props: {ownerKey:"fixture-owner",message: {...message, parts: [...message.parts, text('final', 'PUBLIC-ANSWER')]}}}).body;
        assert.ok(!html.slice(0, html.indexOf('<details class="execution-process')).includes('LIVE-ONE'));
        assert.ok(html.includes('PUBLIC-ANSWER'));
    } finally { await server.close(); }
});

test('delegated task summaries use readable ordinal labels without claiming sealed work succeeded', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale} = await server.ssrLoadModule('/src/lib/i18n.ts');
        locale.set('zh-Hans');
        const message = {id: 'tasks', role: 'assistant', state: 'completed', canonical: null,
            parts: [1, 2].map(number => ({source: 'journal', id: `part-${number}`, journalOwner: 'owner', order: {section: 'tasks', position: String(number)},
                value: {kind: 'task', part_id: `part-${number}`, revision: '1', task_id: `internal-task-${number}`, state: 'sealed', current_step: '排队中'}}))};
        const html = render(Component, {props: {ownerKey:"fixture-owner",message}}).body;
        const summaries = [...html.matchAll(/<summary[^>]*class="[^"]*\bpart-summary\b[^"]*"[^>]*>(.*?)<\/summary>/gs)].map(match => match[1]);
        assert.equal(summaries.length, 2);
        for (let index = 0; index < summaries.length; index++) {
            assert.ok(summaries[index].includes(`后台任务 ${index + 1}`));
            assert.ok(summaries[index].includes('执行已结束'));
            assert.doesNotMatch(summaries[index], /internal-task|成功|已完成/);
        }
        assert.ok(html.includes('internal-task-1'), 'technical identity remains inside expandable detail');
        assert.doesNotMatch(html, /排队中/, 'a terminal task must not present its stale live step');
    } finally { await server.close(); }
});

test('uncertain reply points to conversation navigation without a second create action', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatReplyError.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const html = render(Component, {props: {kind: 'uncertain', oncontinue: () => {}}}).body;
            assert.ok(html.includes(t(language, 'chatPage.openConversations')));
            assert.equal((html.match(/<button/g) ?? []).length, 1);
            assert.ok(!html.includes('new-conversation'));
        }
    } finally { await server.close(); }
});


test('A2UI and tool boundaries retain public prose and reveal the following reasoning round', async () => {
    const earlier = {source:'canonical', id:'r1', value:{kind:'reasoning',part_id:'r1',revision:1,state:'completed',summary:'EARLIER'}} as TimelinePart;
    const current = {source:'canonical', id:'r2', value:{kind:'reasoning',part_id:'r2',revision:1,state:'running',summary:'CURRENT'}} as TimelinePart;
    const surface = {source:'canonical', id:'surface', value:{kind:'ui_surface',part_id:'surface',revision:1}} as TimelinePart;
    const parts = [earlier, text('commentary','PUBLIC-PROGRESS'), tool('publish', 'running'), surface, current];
    assert.deepEqual(liveReasoningParts(parts).map(part => part.id), ['r2']);
    assert.deepEqual(liveReasoningParts([...parts, text('empty','')]).map(part => part.id), ['r2']);
    const privateText = text('private','PRIVATE-CANDIDATE'); privateText.value.review = 'pending';
    assert.deepEqual(liveReasoningParts([...parts, privateText]).map(part => part.id), ['r2']);
    const server = await createServer({server:{middlewareMode:true,ws:false},appType:'custom'});
    try {
        const {default:Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {createRawSnippet} = await server.ssrLoadModule('svelte');
        const surfaceSlot = createRawSnippet(() => ({render: () => '<section>SURFACE-ANCHOR</section>'}));
        for (const summary of ['CURRENT','CURRENT DELTA']) {
            current.value.summary = summary;
            const html = render(Component,{props:{ownerKey:"fixture-owner",message:{id:'message',role:'assistant',state:'running',canonical:null,parts:[...parts,privateText]},surface:surfaceSlot}}).body;
            assert.ok(html.indexOf('EARLIER') < html.indexOf('PUBLIC-PROGRESS'));
            assert.ok(html.indexOf('PUBLIC-PROGRESS') < html.indexOf('SURFACE-ANCHOR'));
            assert.ok(html.indexOf('SURFACE-ANCHOR') < html.indexOf(summary));
            const processes = [...html.matchAll(/<details class="execution-process[^>]*>/g)].map(match => match[0]);
            assert.equal(processes.length, 3);
            assert.doesNotMatch(processes[0], /\bopen\b/);
            assert.doesNotMatch(processes[2], /\bopen\b/);
            assert.ok(!html.includes('PRIVATE-CANDIDATE'));
        }
    } finally { await server.close(); }
});


test('process segments retain hard boundaries including empty and withheld text', () => {
    const reasoning = (id: string): TimelinePart => ({source: 'canonical', id, value: {kind: 'reasoning', part_id: id, revision: 1, state: 'completed', summary: 'same body'}});
    const hidden = text('hidden', 'PRIVATE'); hidden.value.review = 'pending';
    const surface: TimelinePart = {source: 'canonical', id: 'surface', value: {kind: 'ui_surface', part_id: 'surface', revision: 1}};
    const parts = [reasoning('r1'), tool('one'), surface, reasoning('r2'), text('empty', ''), reasoning('r3'), hidden, reasoning('r4'), text('answer', 'FINAL')];
    const groups = messageFlowPresentation(parts);
    assert.deepEqual(groups.map(group => [group.id, group.kind]), [
        ['r1', 'process'], ['surface', 'part'], ['r2', 'process'], ['empty', 'part'],
        ['r3', 'process'], ['hidden', 'part'], ['r4', 'process'], ['answer', 'text'],
    ]);
    assert.deepEqual(groups.filter(group => group.kind === 'process').flatMap(group => group.parts.map(part => part.id)), ['r1', 'one', 'r2', 'r3', 'r4']);
    assert.ok(!groups.some(group => group.kind === 'text' && group.text.includes('PRIVATE')));
});


test('expert Surface preview renders only its own typed anchor and rejects a foreign message', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatReadOnlySurfacePart.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        locale.set('en');
        const identity = {owner: {authority_id: 'host', conversation_id: 'expert-context', participant_id: 'expert',
            context_id: 'expert-context', originating_run_id: 'expert-run', message_id: 'expert-message', part_id: 'original-tool'},
            surface_id: 'card', generation: 2};
        const part = {source: 'canonical', id: 'surface-part', value: {kind: 'ui_surface', part_id: 'surface-part', revision: 3,
            surface: {identity, revision: 7}, summary: 'EXPERT-CARD-SUMMARY'}};
        const props = {part, hostId: 'host', conversationId: 'expert-context', messageId: 'expert-message', active: true};
        const html = render(Component, {props}).body;
        assert.ok(html.includes('EXPERT-CARD-SUMMARY'));
        assert.ok(html.includes(t('en', 'chatPage.surfaceReadOnly')));
        assert.ok(html.includes(t('en', 'activity.surfaceVersion')), 'immutable content remains readable without exposing revision metadata');
        assert.ok(!html.includes('View this version') && !html.includes('Version 7'));
        assert.throws(() => render(Component, {props: {...props, messageId: 'parent-message'}}).body, /Invalid UI surface/);
        assert.throws(() => render(Component, {props: {...props, conversationId: 'parent-conversation'}}).body, /Invalid UI surface/);
        const ordinaryPart = {...part, value: {kind: 'text', part_id: 'text', revision: 1, markdown: 'not a Surface'}};
        assert.ok(!render(Component, {props: {...props, part: ordinaryPart}}).body.includes('surface-reference'));
    } finally { await server.close(); }
});


test('message-owned process choices survive prepend and explicit collapse wins a regrouped segment', () => {
    const original = tool('original'), inserted = tool('inserted');
    assert.equal(executionProcessChoice([original], {}), null);
    assert.equal(executionProcessChoice([inserted, original], {original: true}), true);
    assert.equal(executionProcessChoice([original, inserted], {original: true, inserted: false}), false);
    assert.equal(executionProcessChoice([inserted, original], {original: true, inserted: true}), true);
    assert.equal(executionProcessExpanded(executionProcessChoice([inserted, original], {original: false}), true), false);
});


test('saved canonical metadata stays at its Part anchors without stale terminal progress or guessed retry', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        const metadata = (id: string, kind: string, value: Record<string, unknown>) => ({source: 'canonical', id, value: {kind, part_id: id, revision: 1, ...value}});
        const task = metadata('task', 'task', {task_id: 'exact-task', state: 'sealed', progress: 0.7, current_step: 'STALE-STEP'});
        const skill = metadata('skill', 'skill', {skill_id: 'exact-skill', display_name: 'SKILL-ANCHOR', state: 'completed', mode: 'typed-mode', model_turns: 4});
        const attempt = metadata('attempt', 'model_attempt', {state: 'failed', attempt_id: 'attempt', provider: 'RAW-PROVIDER', model: 'RAW-MODEL',
            failure: {message: 'Transport failed'}, fallback_attempt_id: 'actual-next'});
        const compact = metadata('compact', 'compaction', {summary: 'COMPACTION-ANCHOR', source_fingerprint: 'source-digest'});
        for (const language of supportedLocales) {
            locale.set(language);
            const message = {id: 'metadata', role: 'assistant', state: 'completed', canonical: null,
                parts: [text('before', 'BEFORE-ANCHOR'), task, skill, attempt, compact, text('after', 'AFTER-ANCHOR')]};
            const html = render(Component, {props: {ownerKey:"fixture-owner",message}}).body;
            const anchors = ['BEFORE-ANCHOR', 'data-part-id="task"', 'SKILL-ANCHOR', 'data-part-id="attempt"', 'COMPACTION-ANCHOR', 'AFTER-ANCHOR'];
            for (let index = 1; index < anchors.length; index++) assert.ok(html.indexOf(anchors[index - 1]) < html.indexOf(anchors[index]), anchors.join(' -> '));
            assert.doesNotMatch(html, /STALE-STEP|<progress/);
            assert.ok(html.includes(t(language, 'chatPage.stateSealed')));
            assert.ok(html.includes(t(language, 'chatPage.requestRestarted')));
            const summaryRows = [...html.matchAll(/<summary[^>]*class="[^"]*\bpart-summary\b[^"]*"[^>]*>(.*?)<\/summary>/gs)].map(match => match[1]);
            assert.equal(summaryRows.length, 3);
            assert.match(html, /class="skill-summary/);
            assert.doesNotMatch(html, /typed-mode|exact-skill/);
            const summaries = summaryRows.join(' ');
            assert.ok(!summaries.includes('RAW-PROVIDER') && !summaries.includes('RAW-MODEL'));
            assert.ok(!summaries.includes(t(language, 'chatPage.toolFailed')), 'a declared successor does not label the current request as failed');
            const unverified = {...message, parts: [metadata('unverified', 'model_attempt', {state: 'failed', model: 'model', fallback_model: 'another'})]};
            const noRelation = render(Component, {props: {ownerKey:"fixture-owner",message: unverified}}).body;
            assert.ok(!noRelation.includes(t(language, 'chatPage.requestRestarted')), 'a model name does not prove a retry relation');
        }
    } finally { await server.close(); }
});

test('an exact TaskControl reuses one task summary and a plain saved Task grants no controls', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatMessageContent.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, t} = await server.ssrLoadModule('/src/lib/i18n.ts'); locale.set('en');
        const task = {source: 'canonical', id: 'summary', value: {kind: 'task', part_id: 'summary', revision: 1, task_id: 'same-task', state: 'running'}};
        const control = {source: 'journal', id: 'control', journalOwner: JSON.stringify(['epoch', 'conversation', 'run', 'assistant']), order: {section: 'tasks', position: '1'},
            value: {kind: 'task_control', part_id: 'control', revision: '1', task_id: 'same-task', state: 'running', cancel_requested: false, supports_guidance: true, supports_cancellation: true}};
        const props = {ownerKey:"fixture-owner",message: {id: 'assistant', role: 'assistant', state: 'running', canonical: null, parts: [task, control]},
            taskOwner: {hostId: 'host', conversationId: 'conversation', runId: 'run'}};
        const html = render(Component, {props}).body;
        const title = t('en', 'chatPage.backgroundTaskNumber', {number: 1});
        assert.equal(html.split(title).length - 1, 1);
        assert.ok(html.includes('task-actions'));
        const savedOnly = render(Component, {props: {...props, message: {...props.message, parts: [{...task, value: {...task.value, supports_guidance: true, supports_cancellation: true}}]}}}).body;
        assert.ok(!savedOnly.includes('task-actions'));
    } finally { await server.close(); }
});


test('native Surface content and recovery entry stay distinct in every locale and phase', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatSurfaceReference.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        const identity = {owner: {authority_id: 'host', conversation_id: 'conversation', participant_id: 'assistant',
            context_id: 'conversation', originating_run_id: 'run', message_id: 'message', part_id: 'part'},
            surface_id: 'card', generation: 1};
        const snapshot = {identity, revision: 73519, catalog_id: 'fixture', catalog_digest: 'fixture', phase: 'ready',
            root_instance_id: 'root', nodes: {root: {instance_id: 'root', component_id: 'root', component: 'Text',
                props: {text: 'ACTUAL-CARD-CONTENT'}, slots: {}, bindings: {}, actions: {}, validation_errors: []}},
            summary: 'SUMMARY-FALLBACK'};
        const props = {reference: {identity, revision: snapshot.revision}, summary: snapshot.summary,
            oninteraction: async () => ({status: 'rejected', message: 'fixture'}), onrefresh: async () => {}};
        for (const language of supportedLocales) {
            locale.set(language);
            for (const phase of ['ready', 'pending', 'read_only', 'deleted']) {
                const html: string = render(Component, {props: {...props, snapshot: {...snapshot, phase}}}).body;
                assert.equal((html.match(/data-ui-surface="card"/g) ?? []).length, 1, `${language}/${phase}`);
                assert.ok(!html.includes(t(language, 'activity.surfaceVersion')));
                assert.ok(!html.includes('aria-expanded=') && !html.includes('73519'));
                if (phase === 'ready' || phase === 'read_only') {
                    assert.equal(html.split('ACTUAL-CARD-CONTENT').length - 1, 1);
                    assert.ok(!html.includes('SUMMARY-FALLBACK'));
                } else {
                    assert.ok(!html.includes('ACTUAL-CARD-CONTENT'));
                    assert.ok(html.includes(t(language, phase === 'pending' ? 'a2ui.pending' : 'a2ui.deleted')));
                }
            }
            const missing: string = render(Component, {props}).body;
            assert.ok(missing.includes('SUMMARY-FALLBACK'));
            assert.ok(missing.includes(t(language, 'activity.surfaceVersion')));
            assert.ok(missing.includes('aria-expanded="false"'));
            assert.ok(!missing.includes('73519'));
        }
    } finally { await server.close(); }
});


test('failed tool then successful retry stays compact with a neutral process heading', async () => {
    const server = await createServer({server: {middlewareMode: true, ws: false}, appType: 'custom'});
    try {
        const {default: Component} = await server.ssrLoadModule('/src/components/ChatExecutionProcess.svelte');
        const {render} = await server.ssrLoadModule('svelte/server');
        const {locale, supportedLocales, t} = await server.ssrLoadModule('/src/lib/i18n.ts');
        for (const language of supportedLocales) {
            locale.set(language);
            const failed = tool('FIRST-ATTEMPT', 'failed');
            const retry = tool('CURRENT-RETRY', 'running');
            for (const live of [true, false]) {
                if (!live) retry.value.state = 'completed';
                const html = render(Component, {props: {parts: [failed, retry], live, choices: {},
                    expandedChoice: null, onexpandedchange() {}, onprocessexpandedchange() {}}}).body;
                assert.doesNotMatch(html, /<details[^>]*\bopen\b/);
                assert.ok(!html.includes(t(language, 'chatPage.processAttention')));
                const summary = html.slice(html.indexOf('<summary'), html.indexOf('</summary>'));
                assert.ok(!summary.includes('FIRST-ATTEMPT'));
                assert.ok(summary.includes(live ? 'CURRENT-RETRY' : t(language, 'chatPage.executionProcess')));
                assert.ok(html.includes('FIRST-ATTEMPT'), 'The failed call remains inspectable in history');
            }
            const opened = render(Component, {props: {parts: [failed, retry], live: true, choices: {},
                expandedChoice: true, onexpandedchange() {}, onprocessexpandedchange() {}}}).body;
            assert.match(opened, /<details[^>]*\bopen\b/);
        }
    } finally { await server.close(); }
});
