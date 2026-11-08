import test from 'node:test';
import assert from 'node:assert/strict';
import { newAutomationDraft, automationCreateRequest, parseAutomations, automationScript, automationApprovalRequest, sameAutomationRevision, automationCanRun, automationHistoryLink, automationWeekday, automationCapabilities, automationRunIntent, automationRunResponse } from './automations.ts';
import type { Automation } from './automations.ts';
import { automationRunConversationLink } from './automations.ts';
import { automationEditDraft, automationEditRequest, automationCanEdit, parseAutomationSkills } from './automations.ts';

test('skill catalog rejects malformed and duplicate identities and strips local paths', () => {
    const skill = { id: 'review', name: 'Review', description: 'Inspect', base_directory: '/private' };
    assert.deepEqual(parseAutomationSkills({ skills: [skill] }), [{ id: 'review', name: 'Review', description: 'Inspect' }]);
    for (const value of [{}, { skills: [skill, skill] }, { skills: [{ ...skill, id: '' }] }, { skills: [{ ...skill, name: null }] }]) assert.throws(() => parseAutomationSkills(value));
});

test('default agent tasks need only a prompt and explicit execution authority, never a fake source', () => {
    const draft = { ...newAutomationDraft(), name: 'Review', prompt: 'Review the workspace', workspaceRoot: '/tmp', permissionProfile: 'cautious' };
    const body = automationCreateRequest(draft);
    assert.equal(body.format, 'agent');
    for (const key of ['url', 'path', 'query', 'cwd', 'script', 'approved']) assert.equal(body[key], undefined);
    assert.throws(() => automationCreateRequest({ ...draft, prompt: '' }));
    assert.equal(automationCanRun({ ...action, format: 'agent', enabled: true }), true);
});

test('editing round trips script and authority with exact revision and no implicit approval', () => {
    const base = { ...newAutomationDraft(), name: 'Script', format: 'script', script: 'echo hi', cwd: '/tmp', prompt: 'Inspect', workspaceRoot: '/tmp', permissionProfile: 'cautious' };
    const owned = { ...action, executionSpec: automationCreateRequest(base).executionSpec };
    const draft = automationEditDraft(owned);
    assert.equal(draft.script, automationScript(action)!.script);
    assert.equal(draft.prompt, 'Inspect');
    const request = automationEditRequest(owned, { ...draft, time: '10:30' });
    assert.equal(request.expectedRevision, 4);
    assert.equal((request.definition as Record<string, unknown>).time, '10:30');
    assert.equal((request.definition as Record<string, unknown>).approved, undefined);
    assert.equal(automationCanEdit({ ...owned, revision: undefined }), false);
    const withReferences = structuredClone(owned) as Automation & { executionSpec: { spec: { skill_refs: string[] } } };
    withReferences.executionSpec.spec.skill_refs = ['must-not-be-dropped'];
    assert.equal(automationCanEdit(withReferences), true);
    assert.deepEqual(automationEditDraft(withReferences).skillIds, ['must-not-be-dropped']);
});

test('pending and terminal execution navigation uses canonical matching binding only', () => {
    const binding = { occurrence_id: 'occurrence', definition_revision: 4, conversation_id: 'chat /', run_id: 'run ?' };
    const expected = '/chat?conversation=chat%20%2F&run=run%20%3F';
    assert.equal(automationRunConversationLink({ occurrenceId: 'occurrence', execution: { binding } }, 4), expected);
    assert.equal(automationRunConversationLink({ occurrenceId: 'occurrence', receipt: { binding } }, 4), expected);
    assert.equal(automationRunConversationLink({ occurrenceId: 'other', execution: { binding } }, 4), undefined);
    assert.equal(automationRunConversationLink({ occurrenceId: 'occurrence', execution: { binding } }, 5), undefined);
    assert.equal(automationRunConversationLink({ occurrenceId: 'occurrence', execution: {} }, 4), undefined);
});

const action: Automation = { id: 'a /?#', name: '私有任务', url: '/tmp/task', format: 'script', userPrompt: JSON.stringify({ kind: 'script_automation', runtime: 'bash', script: 'echo first\necho second\necho third\necho fourth', cwd: '/tmp/task' }), revision: 4, frequency: 'daily', weekday: null, time: '09:00', timezone: 'local', enabled: false, updatedAt: 'now', scriptApproved: false, scriptCwd: '/tmp/task', latestResult: null };
test('weekday labels match host Sunday=1 through Saturday=7', () => {
    assert.equal(automationWeekday(1, 'en'), 'Sunday');
    assert.equal(automationWeekday(7, 'en'), 'Saturday');
    assert.equal(automationWeekday(2, 'zh-Hans'), '星期一');
});
test('absent and unknown host execution capabilities fail closed', () => {
    for (const payload of [null, {}, { executionCapabilities: { manualRun: 'true', scheduledDispatch: 1 } }]) assert.deepEqual(automationCapabilities(payload), { manualRun: false, scheduledDispatch: false, promptAutomations: false });
    assert.equal(automationCapabilities({ executionCapabilities: { manualRun: true, scheduledDispatch: false } }).manualRun, true);
});

test('creates canonical source fields and never automatically approves scripts', () => {
    const draft = { ...newAutomationDraft(), workspaceRoot: '/tmp/task', permissionProfile: 'cautious', prompt: 'Inspect the workspace', name: ' task ', format: 'script', script: 'echo hi', cwd: '/tmp', frequency: 'weekly', weekday: 7 };
    assert.deepEqual(automationCreateRequest(draft), { name: 'task', format: 'script', userPrompt: draft.prompt, frequency: 'weekly', weekday: 7, time: '09:00', timezone: 'local', script: 'echo hi', cwd: '/tmp',
        executionSpec: { kind: 'agent', spec: { prompt: draft.prompt, execution_scope: { kind: 'local_workspace', root: '/tmp/task' }, permission_profile: 'cautious', model_selection: 'use_current_default_at_admission', context_refs: [], skill_refs: [] } } });
    for (const [format, field] of [['search', 'query'], ['file', 'path'], ['folder', 'path'], ['html', 'url'], ['rss', 'url'], ['json', 'url'], ['icalendar', 'url']]) {
        const body = automationCreateRequest({ ...draft, format, source: ' source ' });
        assert.equal(body[field], 'source'); assert.equal(body.script, undefined); assert.equal(body.approved, undefined);
    }
});
test('unknown formats, invalid schedules and empty script fail without mutating draft', () => {
    const draft = { ...newAutomationDraft(), workspaceRoot: '/tmp/task', permissionProfile: 'cautious', prompt: 'Inspect', name: 'Task', source: 'https://example.com' };
    for (const change of [{ format: 'nola.mac.automation.v1' }, { format: 'reminder' }, { frequency: 'hourly' }, { time: '24:00' }, { time: '9:00' }, { frequency: 'weekly', weekday: 0 }, { frequency: 'weekly', weekday: 8 }, { format: 'script', script: '' }, { name: '' }]) assert.throws(() => automationCreateRequest({ ...draft, ...change }));
    assert.equal(draft.source, 'https://example.com');
});

test('execution scope and permissions are explicit, never inferred from source metadata', () => {
    const draft = { ...newAutomationDraft(), name: 'Task', source: '/tmp', prompt: 'Inspect' };
    for (const change of [{}, { workspaceRoot: '/tmp' }, { workspaceRoot: 'relative', permissionProfile: 'cautious' }, { workspaceRoot: '/tmp', permissionProfile: 'unknown' }, { workspaceRoot: '/tmp', permissionProfile: 'cautious', prompt: '' }]) assert.throws(() => automationCreateRequest({ ...draft, ...change }));
});
test('manual retry preserves occurrence identity and rejects a changed revision', () => {
    const intent = automationRunIntent(action);
    const restored = JSON.parse(JSON.stringify(intent));
    assert.deepEqual(automationRunIntent(action, restored), intent);
    assert.throws(() => automationRunIntent({ ...action, revision: 5 }, restored));
    assert.throws(() => automationRunIntent({ ...action, id: 'other' }, restored));
    assert.notEqual(automationRunIntent(action).occurrenceKey, intent.occurrenceKey);
});
test('manual execution requires canonical receipt, preserves all six outcomes and pending state', () => {
    assert.throws(() => automationRunResponse({ success: true }, 4));
    const base = { occurrenceId: 'occurrence', execution: {} };
    assert.deepEqual(automationRunResponse(base, 4), { terminal: false, message: 'automation.runAccepted' });
    assert.deepEqual(automationRunResponse({ ...base, outcomeUnknown: true }, 4), { terminal: false, message: 'automation.runUnknown' });
    for (const kind of ['answered', 'action_completed', 'incomplete', 'blocked', 'cancelled', 'failed']) {
        const receipt = { binding: { occurrence_id: 'occurrence', definition_revision: 4 }, terminal: { result: { value: { kind } } } };
        assert.deepEqual(automationRunResponse({ ...base, receipt }, 4), { terminal: true, message: `automation.outcome.${kind}` });
        assert.throws(() => automationRunResponse({ ...base, receipt }, 5));
        assert.throws(() => automationRunResponse({ ...base, occurrenceId: 'other', receipt }, 4));
    }
});
test('approval reviews complete source and binds canonical revision, not preview', () => {
    assert.match(automationScript(action)!.script, /echo fourth$/);
    assert.deepEqual(automationApprovalRequest(action), { confirmSafety: true, expectedRevision: 4 });
    for (const revision of [undefined, NaN, 0, -1, 1.5]) assert.throws(() => automationApprovalRequest({ ...action, revision }));
    assert.throws(() => automationApprovalRequest({ ...action, userPrompt: 'echo old raw script' }));
    assert.equal(automationScript({ ...action, userPrompt: JSON.stringify({ kind: 'script_automation', runtime: 'python', script: 'print(1)', cwd: '' }) }), null);
});
test('stale or changed exact approval object is rejected', () => {
    assert.equal(sameAutomationRevision(action, { ...action }), true);
    for (const change of [{ revision: 5 }, { id: 'other' }, { userPrompt: 'changed' }, { updatedAt: 'later' }, { format: 'html' }]) assert.equal(sameAutomationRevision(action, { ...action, ...change }), false);
    assert.equal(sameAutomationRevision(action, undefined), false);
});
test('unknown states fail closed and history uses exact encoded source identity', () => {
    assert.equal(automationCanRun(action), false);
    assert.equal(automationCanRun({ ...action, scriptApproved: true }), true);
    assert.equal(automationCanRun({ ...action, format: 'unknown', scriptApproved: true }), false);
    assert.equal(automationHistoryLink(action), '/results?sourceId=a%20%2F%3F%23');
    assert.equal(parseAutomations({ actions: [{ ...action, scriptApproved: 'true' }] })[0].scriptApproved, null);
    assert.throws(() => parseAutomations({ actions: [action, action] }));
    assert.throws(() => parseAutomations({ actions: [{ ...action, enabled: 'true' }] }));
    assert.throws(() => parseAutomations({}));
});
