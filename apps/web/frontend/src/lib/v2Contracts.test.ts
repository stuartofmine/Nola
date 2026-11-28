import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';
import { supportedLocales, t } from './i18n.ts';
import {
    canSelectProfile,
    executionParameterValidationError,
    groupedProfiles,
    metadataChanges,
    metadataDraft,
    parameterChanges,
    reasoningChanges,
    reasoningDraft,
    reasoningExecutionValidationError,
    reasoningOptions,
    reasoningValidationError,
} from './modelProfiles.ts';
import type { ModelExecutionCapabilities, ReasoningConfiguration, RemoteProfile } from './modelProfiles.ts';
import {
    endpointDirty,
    endpointRequest,
    endpointSnapshot,
    endpointValidationError,
} from './modelEndpoints.ts';
import {
    automationApprovalRequest,
    automationCanEdit,
    automationCanRun,
    automationCapabilities,
    automationCreateRequest,
    automationEditDraft,
    automationEditRequest,
    automationHistoryLink,
    automationRunConversationLink,
    automationRunIntent,
    automationRunResponse,
    automationScript,
    automationWeekday,
    isAbsoluteWorkspaceRoot,
    newAutomationDraft,
    parseAutomations,
    parseAutomationSkills,
    parseStoredAutomationRunIntent,
    sameAutomationRevision,
} from './automations.ts';
import type { Automation } from './automations.ts';

const server = await createServer({ server: { middlewareMode: true, ws: false }, appType: 'custom' });
after(async () => { await server.close(); });

function stubFetch(handler: (url: string, init?: RequestInit) => Promise<Response>) {
    const originalFetch = globalThis.fetch;
    const originalWindow = (globalThis as Record<string, unknown>).window;
    const originalDocument = (globalThis as Record<string, unknown>).document;
    (globalThis as Record<string, unknown>).window = { location: { origin: 'http://127.0.0.1:4096' } };
    (globalThis as Record<string, unknown>).document = { cookie: '' };
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
        return handler(url, init);
    }) as typeof fetch;
    return () => {
        globalThis.fetch = originalFetch;
        (globalThis as Record<string, unknown>).window = originalWindow;
        (globalThis as Record<string, unknown>).document = originalDocument;
    };
}

test('v2 room update, attach, and detach use POST /api/agent-rooms update_room and surface failures', async () => {
    const { attachV2ConversationToRoom, updateV2AgentRoom } = await server.ssrLoadModule('/src/lib/v2Chat.ts');

    const calls: Array<{ method: string; url: string; body?: string }> = [];
    let failDetach = false;
    const restore = stubFetch(async (url, init) => {
        const method = (init?.method ?? 'GET').toUpperCase();
        calls.push({ method, url, body: typeof init?.body === 'string' ? init.body : undefined });
        if (failDetach) {
            return new Response(JSON.stringify({ ok: false, error: 'detach failed' }), {
                status: 500,
                headers: { 'content-type': 'application/json' },
            });
        }
        return new Response(JSON.stringify({ ok: true, result: { ok: true, room: { id: 'room-7', name: 'Core', session_id: 'conv-1' } }, rooms: [], experts: [] }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
        });
    });

    try {
        await updateV2AgentRoom('room-42', { name: 'Design Squad', participantExpertIds: ['exp-1'] });
        assert.equal(calls.some(c => c.method === 'PUT'), false, 'must never issue PUT /api/agent-rooms/<id>');
        const updatePayload = JSON.parse(calls[0].body ?? '{}');
        assert.equal(updatePayload.command?.type, 'update_room');
        assert.equal(updatePayload.command?.input?.id, 'room-42');
        assert.equal(updatePayload.command?.input?.name, 'Design Squad');
        assert.deepEqual(updatePayload.command?.input?.participant_expert_ids, ['exp-1']);

        calls.length = 0;
        const attached = await attachV2ConversationToRoom('conv-1', 'room-7');
        assert.equal(attached?.id, 'room-7');
        assert.equal(calls[0].method, 'POST');
        assert.deepEqual(JSON.parse(calls[0].body ?? '{}'), {
            command: {
                type: 'update_room',
                input: { id: 'room-7', room_id: 'room-7', session_id: 'conv-1', conversation_id: 'conv-1' },
            },
        });

        calls.length = 0;
        const detached = await attachV2ConversationToRoom('conv-1', null, undefined, 'room-7');
        assert.equal(detached, null);
        assert.deepEqual(JSON.parse(calls[0].body ?? '{}'), {
            command: { type: 'update_room', input: { id: 'room-7', detach: true, session_id: null } },
        });

        failDetach = true;
        await assert.rejects(
            () => attachV2ConversationToRoom('conv-1', null, undefined, 'room-7'),
            /detach failed/,
        );
    } finally {
        restore();
    }
});

test('cancel and openLogsFolder failures surface localized UI errors instead of being swallowed', () => {
    const chat = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
    const modelTest = readFileSync(new URL('../pages/ModelTest.svelte', import.meta.url), 'utf8');
    const desktopSettings = readFileSync(new URL('../components/DesktopSettings.svelte', import.meta.url), 'utf8');
    const hostDiagnostics = readFileSync(new URL('../components/HostDiagnostics.svelte', import.meta.url), 'utf8');

    const cancelFn = chat.slice(chat.indexOf('async function cancelActiveRun()'), chat.indexOf('async function refreshModelChoices()'));
    assert.match(cancelFn, /await cancelV2Turn\(targetId\)/);
    assert.match(cancelFn, /pushToast\(/);
    assert.doesNotMatch(cancelFn, /\.catch\(\(\) => \{\}\)/);

    const stopFn = modelTest.slice(modelTest.indexOf('async function stop()'), modelTest.indexOf('function clear()'));
    assert.match(stopFn, /await cancelV2Turn\(convId\)/);
    assert.match(stopFn, /pushToast\(/);
    assert.doesNotMatch(stopFn, /\.catch\(\(\) => \{\}\)/);

    assert.match(desktopSettings, /diagnostics\.openLogsFailed/);
    assert.match(desktopSettings, /pushToast\(/);
    assert.match(hostDiagnostics, /diagnostics\.openLogsFailed/);
    assert.match(hostDiagnostics, /pushToast\(/);

    for (const language of supportedLocales) {
        assert.notEqual(t(language, 'diagnostics.openLogsFailed'), 'diagnostics.openLogsFailed');
        assert.ok(t(language, 'diagnostics.openLogsFailed').trim());
    }
});

test('model profile reasoning validation handles effort, budget, unsupported control, and protocol constraints', () => {
    const baseProfile = { enabled: true, capabilities: ['chat'], endpoint: { enabled: true } } as RemoteProfile;
    assert.equal(canSelectProfile(baseProfile), true);
    assert.equal(canSelectProfile({ ...baseProfile, enabled: false }), false);
    assert.equal(canSelectProfile({ ...baseProfile, capabilities: ['retrieval'] }), false);
    assert.equal(canSelectProfile({ ...baseProfile, endpoint: { ...baseProfile.endpoint, enabled: false } }), false);

    // Budget control on Anthropic
    const budgetDraft = reasoningDraft({ control: 'budget', budgets: [1024, 2048, 4096], efforts: [], defaultPreference: 'detailed' });
    assert.equal(reasoningValidationError(budgetDraft, 'anthropic', 8192), null);
    assert.deepEqual(reasoningOptions(budgetDraft, 'anthropic', 8192), ['concise', 'balanced', 'detailed']);
    assert.ok(reasoningValidationError(budgetDraft, 'anthropic', 4096));
    assert.ok(reasoningValidationError(budgetDraft, 'responses', 8192));

    // Effort control across protocols including siwc_responses and gemini
    const effortDraft = reasoningDraft({ control: 'effort', efforts: ['none', 'low', 'high'], budgets: [], defaultPreference: 'high' });
    for (const protocol of ['responses', 'siwc_responses', 'chat_completions', 'anthropic', 'gemini']) {
        assert.equal(reasoningValidationError(effortDraft, protocol, 16384), null);
        assert.deepEqual(reasoningOptions(effortDraft, protocol), ['none', 'low', 'high']);
    }
    assert.ok(reasoningValidationError(effortDraft, 'unknown', 16384));
    assert.ok(reasoningValidationError({ ...effortDraft, efforts: [] }, 'responses', 16384));

    // control: "unsupported" is accepted without requiring executionCapabilities.reasoning
    const noReasoningExec: ModelExecutionCapabilities = {
        toolCalls: true,
        imageInput: false,
        reasoning: false,
        samplingParameters: true,
    };
    const unsupportedReasoning = { control: 'unsupported' } as unknown as ReasoningConfiguration;
    assert.equal(reasoningExecutionValidationError(unsupportedReasoning, noReasoningExec), null);
    assert.equal(reasoningExecutionValidationError({ control: 'automatic' }, noReasoningExec), null);
    assert.equal(reasoningExecutionValidationError(effortDraft, noReasoningExec)?.code, 'modelValidation.executionReasoning');
    assert.equal(reasoningExecutionValidationError(unsupportedReasoning, null)?.code, 'modelValidation.executionDeclaration');
    assert.equal(reasoningDraft(unsupportedReasoning).control, 'automatic');

    // Execution parameters, sparse metadata/reasoning/parameter changes, and grouping
    const profile: RemoteProfile = {
        id: 'siwc-1',
        displayName: 'SIWC',
        modelName: 'gpt-5',
        revision: 1,
        enabled: true,
        selected: true,
        priority: 50,
        order: 0,
        capabilities: ['chat'],
        endpoint: { id: 'siwc', name: 'ChatGPT', baseURL: 'https://api.openai.com/v1', wireProtocol: 'siwc_responses', enabled: true, order: 0 },
        parameters: { temperature: 0.7, topP: 0.9 },
        reasoning: effortDraft,
        executionCapabilities: { toolCalls: true, imageInput: true, reasoning: true, samplingParameters: true, samplingRequiresNoReasoning: true },
    };
    assert.equal(executionParameterValidationError(profile), null);
    assert.equal(executionParameterValidationError({ ...profile, parameters: { ...profile.parameters, seed: 42 } })?.code, 'modelValidation.protocolParameters');
    assert.equal(executionParameterValidationError({ ...profile, executionCapabilities: null })?.code, 'modelValidation.executionDeclaration');

    const metaDraft = metadataDraft(profile);
    metaDraft.displayName = 'Renamed';
    metaDraft.priority = 0;
    assert.deepEqual(metadataChanges(profile, metaDraft), { displayName: 'Renamed', priority: 0 });
    assert.deepEqual(reasoningChanges(effortDraft, reasoningDraft(effortDraft)), {});
    assert.deepEqual(parameterChanges({ temperature: 0.7, seed: 123 }, { temperature: 0, seed: null }), { temperature: 0, seed: null });

    const modelAt = (id: string, endpoint: string, endpointOrder: number, order: number) => ({ id, order, endpoint: { id: endpoint, order: endpointOrder } } as RemoteProfile);
    const groups = groupedProfiles([modelAt('b', 'first', 0, 2), modelAt('c', 'last', 1, 0), modelAt('a', 'first', 0, 1)]);
    assert.deepEqual(groups.map(group => group.models.map(item => item.id)), [['a', 'b'], ['c']]);
});

test('model endpoints snapshot member revisions, reject unsafe URLs, and stay prototype-safe', () => {
    const model = (id: string, endpoint = 'provider/中文'): RemoteProfile => ({
        id,
        revision: 4,
        modelName: id,
        displayName: id,
        enabled: true,
        selected: false,
        priority: 50,
        order: 0,
        endpoint: { id: endpoint, name: '提供商', baseURL: 'https://example.com/v1', wireProtocol: 'responses', enabled: true, order: 0 },
        capabilities: ['chat'],
        parameters: { maxOutputTokens: 16384 },
        reasoning: { control: 'effort', efforts: ['low', 'high'], budgets: [] },
    });

    const snapshot = endpointSnapshot([model('a'), model('b'), model('foreign', 'other')], 'provider/中文');
    const draft = { ...snapshot.draft, name: ' 新名称 ', enabled: false };
    assert.equal(endpointDirty(snapshot, draft), true);
    assert.deepEqual(endpointRequest(snapshot, draft), {
        expectedRevisions: { a: 4, b: 4 },
        name: '新名称',
        baseURL: 'https://example.com/v1',
        wireProtocol: 'responses',
        enabled: false,
    });

    for (const baseURL of ['file:///tmp/test', 'javascript:alert(1)', 'https://user:secret@example.com', 'https://example.com/#frag', 'invalid']) {
        assert.ok(endpointValidationError(snapshot, { ...snapshot.draft, baseURL }), baseURL);
    }
    assert.ok(endpointValidationError(snapshot, { ...snapshot.draft, wireProtocol: 'unknown' }));
    assert.throws(() => endpointSnapshot([model('a')], 'missing'));

    const protoSnap = endpointSnapshot([model('__proto__'), model('constructor')], 'provider/中文');
    const protoPayload = JSON.parse(JSON.stringify(endpointRequest(protoSnap, protoSnap.draft)));
    assert.equal(protoPayload.expectedRevisions.__proto__, 4);
    assert.equal(protoPayload.expectedRevisions.constructor, 4);
});

test('automations enforce canonical format, explicit authority, revision binding, and fail-closed states', () => {
    const action: Automation = {
        id: 'a /?#',
        name: '私有任务',
        url: '/tmp/task',
        format: 'script',
        userPrompt: JSON.stringify({ kind: 'script_automation', runtime: 'bash', script: 'echo one\necho two', cwd: '/tmp/task' }),
        revision: 4,
        frequency: 'daily',
        weekday: null,
        time: '09:00',
        timezone: 'local',
        enabled: false,
        updatedAt: 'now',
        scriptApproved: false,
        scriptCwd: '/tmp/task',
        latestResult: null,
    };

    const agentDraft = { ...newAutomationDraft(), name: 'Review', prompt: 'Review the workspace', workspaceRoot: '/tmp', permissionProfile: 'cautious' };
    const agentBody = automationCreateRequest(agentDraft);
    assert.equal(agentBody.format, 'agent');
    assert.equal(agentBody.script, undefined);
    assert.equal(automationCanRun({ ...action, format: 'agent', enabled: true }), true);

    const scriptDraft = { ...newAutomationDraft(), workspaceRoot: '/tmp/task', permissionProfile: 'cautious', prompt: 'Inspect', name: ' task ', format: 'script', script: 'echo hi', cwd: '/tmp', frequency: 'weekly', weekday: 7 };
    const created = automationCreateRequest(scriptDraft);
    assert.equal(created.name, 'task');
    assert.equal(created.approved, undefined);

    const owned = { ...action, executionSpec: created.executionSpec };
    const editDraft = automationEditDraft(owned);
    assert.equal(editDraft.script, automationScript(action)!.script);
    assert.equal(automationCanEdit(owned), true);
    assert.equal(automationCanEdit({ ...owned, revision: undefined }), false);
    const editReq = automationEditRequest(owned, { ...editDraft, time: '10:30' });
    assert.equal(editReq.expectedRevision, 4);
    assert.equal((editReq.definition as Record<string, unknown>).time, '10:30');

    for (const root of ['/tmp/work', 'C:\\workspace\\project', '\\\\server\\share\\dir']) {
        assert.equal(isAbsoluteWorkspaceRoot(root), true);
    }
    for (const bad of ['', 'relative/path', 'C:relative\\path']) {
        assert.equal(isAbsoluteWorkspaceRoot(bad), false);
    }

    assert.equal(automationWeekday(1, 'en'), 'Sunday');
    assert.equal(automationWeekday(2, 'zh-Hans'), '星期一');
    assert.deepEqual(automationCapabilities(null), { manualRun: false, scheduledDispatch: false, promptAutomations: false });
    assert.equal(automationApprovalRequest(action).expectedRevision, 4);
    assert.equal(sameAutomationRevision(action, { ...action }), true);
    assert.equal(sameAutomationRevision(action, { ...action, revision: 5 }), false);

    assert.equal(automationCanRun(action), false);
    assert.equal(automationCanRun({ ...action, scriptApproved: true }), true);
    assert.equal(automationHistoryLink(action), '/results?sourceId=a%20%2F%3F%23');
    assert.throws(() => parseAutomations({ actions: [action, action] }));

    const skill = { id: 'review', name: 'Review', description: 'Inspect', base_directory: '/private' };
    assert.deepEqual(parseAutomationSkills({ skills: [skill] }), [{ id: 'review', name: 'Review', description: 'Inspect' }]);

    const intent = automationRunIntent(action);
    assert.deepEqual(parseStoredAutomationRunIntent(JSON.stringify(intent), action), { previous: intent, clear: false });
    assert.deepEqual(parseStoredAutomationRunIntent(JSON.stringify(intent), { ...action, revision: 5 }), { clear: true });
    assert.deepEqual(parseStoredAutomationRunIntent('{not-json', action), { clear: true });

    const binding = { occurrence_id: 'occurrence', definition_revision: 4, conversation_id: 'chat /', run_id: 'run ?' };
    assert.equal(automationRunConversationLink({ occurrenceId: 'occurrence', execution: { binding } }, 4), '/chat?conversation=chat%20%2F&run=run%20%3F');
    const receipt = { binding, terminal: { result: { value: { kind: 'answered' } } } };
    assert.deepEqual(automationRunResponse({ occurrenceId: 'occurrence', execution: {}, receipt }, 4), {
        terminal: true,
        message: 'automation.outcome.answered',
    });
});

test('responsive layout breakpoints keep single conversation manager, inspector tabs, and compact sidebar collapse', () => {
    const chat = readFileSync(new URL('../pages/Chat.svelte', import.meta.url), 'utf8');
    const inspector = readFileSync(new URL('../components/ConversationInspector.svelte', import.meta.url), 'utf8');
    const roomManager = readFileSync(new URL('../components/ExpertRoomManager.svelte', import.meta.url), 'utf8');
    const textReader = readFileSync(new URL('../components/ChatTextReaderDialog.svelte', import.meta.url), 'utf8');

    const markup = chat.slice(chat.indexOf('</script>'));
    assert.equal((markup.match(/<ChatConversationManager\b/g) ?? []).length, 1);
    assert.equal((markup.match(/<ConversationInspector\b/g) ?? []).length, 1);
    assert.match(roomManager, /dialog\.showModal\(\)/);
    assert.match(textReader, /dialog\?\.showModal\(\)/);
    assert.match(chat, /\.composer-box:focus-within\s*\{[^}]*border-color: var\(--composer-focus-border\)/);
    assert.match(inspector, /role="tablist"/);
    assert.match(inspector, /class="inspector-scrim"/);
    for (const tab of ['tasks', 'artifacts', 'recall', 'diff', 'browser', 'context', 'data-sources', 'knowledge']) {
        assert.ok(inspector.includes(`id: '${tab}'`), `inspector tab ${tab}`);
    }
    assert.match(chat, /height: 100dvh/);
    assert.match(chat, /@media \(max-width: 760px\)[^]*?\.chat-sidebar\s*\{\s*display: none;/);
    assert.doesNotMatch(chat, /max-height: 42dvh|max-height: 58dvh|min-height: 560px/);
});
