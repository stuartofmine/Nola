<script lang="ts">
    import { workspacePageApi } from '../lib/workspacePageLocation';
    const api = workspacePageApi(unscopedApi);
    import { workspacePageLocation } from '../lib/workspacePageLocation';
    const pageLocation = workspacePageLocation();
    const editorStorageKey = (name: string) => `nola.model-test.${name}.${encodeURIComponent(pageLocation.key ?? 'standalone')}`;
    import { onMount } from 'svelte';
    import { get } from 'svelte/store';
    import { api as unscopedApi, errorMessage } from '../lib/api';
    import { freezeHostToolPlan } from '../lib/hostToolPlan';
    import { locale, t } from '../lib/i18n';
    import { parseChatConversation, conversationUrl } from '../lib/chatConversations';
    import type { ChatConversation } from '../lib/chatConversations';
    import { createChatTimeline, reduceChatTimeline, timelineMessageText } from '../lib/chatTimeline';
    import { AssistantJournalReader, parseJournalCursor } from '../lib/assistantJournal';
    import type { JournalIdentity } from '../lib/assistantJournal';
    import type { AdmittedMessage } from '../lib/messageAdmissions';
    import { readTerminal } from '../lib/assistantTerminal';
    import type { ChatTimeline } from '../lib/chatTimeline';
    import { createChatTurn, chatTurnIdentity, reduceChatTurn, canReleaseChatTurn, buildChatResumeRequest } from '../lib/chatTurn';
    import { SourceSubmissionStorage, createSourceSubmission, sourceIdentity, bindSourceDigest, acceptSourceReceipt,
        sourceSubmissionQuery, sourceSubmissionBody, exactSubmissionCancel, settledSourceSubmission, parseObservedSourceReceipt } from '../lib/sourceSubmission';
    import { RunObserver } from '../lib/runObservation';
    import { RunContextUsageProjection, readCurrentRunContextUsage, contextUsageFraction } from '../lib/runContextUsage';
    import type { CommittedRunContextUsage } from '../lib/runContextUsage';
    import type { PendingSourceSubmission } from '../lib/sourceSubmission';
    import type { ChatTurn, ChatTurnOwner } from '../lib/chatTurn';
    import { chatEventStream } from '../lib/chatTransport';
    import { activeApiTestModel, activeLocalTestModel, modelTestConversationRequest, modelTestParametersUrl, modelTestRunRequest } from '../lib/modelTest';
    import type { LocalTestModel } from '../lib/modelTest';
    import type { RemoteProfile } from '../lib/modelProfiles';
    import Markdown from '../components/Markdown.svelte';

    let target = $state<'local' | 'cloud'>('local');
    let compactPane = $state<'setup' | 'output'>('setup');
    let prompt = $state(t(get(locale), 'modelTest.defaultPrompt'));
    let output = $state('');
    let error = $state('');
    let warning = $state('');
    let loading = $state(true);
    let preparing = $state(false);
    let cleaning = $state(false);
    let firstTokenMs = $state<number | null>(null);
    let totalMs = $state<number | null>(null);
    let startedAt = 0;
    let profiles = $state<RemoteProfile[]>([]);
    let localModels = $state<LocalTestModel[]>([]);
    let hostId = '';
    let clientId = '';
    let conversation = $state<ChatConversation | null>(null);
    let timeline: ChatTimeline | null = null;
    let turn = $state<ChatTurn | null>(null);
    let streamController: AbortController | null = null;
    let generation = 0;
    let journalReader: AssistantJournalReader | null = null;
    let disposed = false;
    let observer: RunObserver | null = null;
    let observedRecovery = $state<string | null>(null);
    let resumeAuthority = $state<string | null>(null);
    const consumedResumeAuthorities = new Set<string>();
    let pendingSubmission = $state<PendingSourceSubmission | null>(null);
    const usageProjection = new RunContextUsageProjection();
    let contextUsage = $state<CommittedRunContextUsage | null>(null);
    let usageReading = false;
    async function refreshRunUsage(serial: number) {
        const receipt = pendingSubmission?.receipt;
        if (!turn || !receipt?.admission || receipt.phase !== 'admitted' || usageReading || serial !== generation || disposed) return;
        const captured = turn;
        const current = () => serial === generation && !disposed && turn?.identity.submissionId === captured.identity.submissionId;
        usageReading = true;
        try {
            const value = await readCurrentRunContextUsage(captured.owner, receipt.admission.identity, usageProjection,
                body => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body) }), current);
            if (current()) contextUsage = value;
        } catch { if (current()) contextUsage = null; }
        finally { usageReading = false; }
    }
    const submissionStorage = () => new SourceSubmissionStorage(sessionStorage, 'modeltest');
    function acceptSubmission(receipt: unknown) {
        if (!pendingSubmission || !turn) throw new Error('Missing original model-test submission');
        pendingSubmission = acceptSourceReceipt(pendingSubmission, receipt); submissionStorage().save(pendingSubmission);
        const accepted = pendingSubmission.receipt!;
        if (accepted.phase === 'admitted') turn = reduceChatTurn(turn, { type: 'run_bound', owner: turn.owner, runId: accepted.admission!.identity.run_id }).state;
        else if (settledSourceSubmission(pendingSubmission)) {
            turn = reduceChatTurn(turn, { type: 'submission_not_admitted', owner: turn.owner, outcome: accepted.phase as 'rejected' | 'cancelled' }).state;
            submissionStorage().clear(pendingSubmission); pendingSubmission = null; error = accepted.phase === 'rejected' ? t($locale, 'modelTest.failed') : '';
        }
    }
    async function reconcileSubmission(serial: number, signal?: AbortSignal): Promise<boolean> {
        const captured = pendingSubmission;
        if (!captured || captured.receipt?.phase === 'admitted') return true;
        if (captured.request_digest === null) return false;
        const response = await api<{ source_host_id: string; result: unknown }>('/api/chat/agent/v2/submission', {
            method: 'POST', signal, body: JSON.stringify(sourceSubmissionQuery(captured)) });
        if (signal?.aborted || serial !== generation || disposed || pendingSubmission?.identity.submission_id !== captured.identity.submission_id) return false;
        if (response.source_host_id !== captured.identity.source_host_id) throw new Error('Model-test source changed');
        if (response.result === null) return false;
        acceptSubmission(response.result);
        return !pendingSubmission || pendingSubmission.receipt?.phase === 'admitted';
    }
    function stopObservation() {
        observer?.stop(); observer = null;
        journalReader?.dispose(); journalReader = null;
        observedRecovery = null; resumeAuthority = null;
    }
    function observeSubmission() {
        if (disposed || document.visibilityState === 'hidden' || streamController || !turn || canReleaseChatTurn(turn)) return;
        stopObservation();
        const serial = generation; const captured = turn; const owner = captured.owner;
        const runObserver = new RunObserver(); observer = runObserver;
        const current = () => observer === runObserver && serial === generation && !disposed &&
            turn?.identity.submissionId === captured.identity.submissionId;
        void runObserver.run(owner,
            (body, signal) => api('/api/chat/agent/v2/observe', { method: 'POST', body: JSON.stringify(body), signal }),
            async value => {
                if (!current() || !turn || !timeline) return false;
                if (!value) throw new Error('Submission observation lacks an admitted Run');
                const receipt = parseObservedSourceReceipt(value.submission_receipt, owner, value.run_id, value.assistant_message_id);
                acceptSubmission(receipt);
                if (!value.admission) throw new Error('Observed Run lacks message admission');
                try {
                    usageProjection.bind(owner, value.admission);
                    const usage = await usageProjection.accept(value.context_usage, owner, value.admission, current);
                    if (current()) contextUsage = usage;
                } catch { if (current()) contextUsage = null; }
                if (!journalReader) journalReader = new AssistantJournalReader(owner.hostId, owner.conversationId, serial,
                    (body, signal) => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(body), signal }));
                const declarations = await journalReader.admit(value.admission.identity, value.admission);
                if (!current() || !timeline) return false;
                timeline = reduceChatTimeline(timeline, { type: 'admissions', owner, messages: declarations }).state;
                let after: string | null = null;
                for (let page = 0; page < 64; page++) {
                    const response = await api<{ source_host_id: string; result: unknown[] }>('/api/chat/agent/v2/journal', {
                        method: 'POST', signal: runObserver.signal, body: JSON.stringify({ operation: 'heads',
                            conversation_id: owner.conversationId, run_id: value.run_id, after_message_id: after, limit: 32 }) });
                    if (!current() || !timeline) return false;
                    if (response.source_host_id !== owner.hostId || !Array.isArray(response.result) || response.result.length > 32)
                        throw new Error('Invalid observed model-test heads');
                    for (const raw of response.result) {
                        const cursor = parseJournalCursor(raw);
                        if (cursor.identity.run_id !== value.run_id || cursor.identity.conversation_id !== owner.conversationId ||
                            after !== null && cursor.identity.assistant_message_id <= after) throw new Error('Observed head owner/order changed');
                        const update = await journalReader.read(cursor, value.run_id);
                        if (!current() || !timeline) return false;
                        if (update) {
                            const reduced = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId, runId: value.run_id, update });
                            if (reduced.ignored && reduced.ignored !== 'duplicate') throw new Error('Observed model-test journal was not admitted');
                            timeline = reduced.state;
                        }
                        after = cursor.identity.assistant_message_id;
                    }
                    if (response.result.length < 32) break;
                    if (page === 63) throw new Error('Observed head budget exceeded');
                }
                if (!current() || !turn || !timeline) return false;
                output = timeline.messages.filter(message => message.role === 'assistant').map(timelineMessageText).join('\n\n');
                if (output && firstTokenMs === null) firstTokenMs = Date.now() - startedAt;
                observedRecovery = value.recovery;
                resumeAuthority = value.recovery === 'resume_required'
                    ? JSON.stringify([value.run_id, value.assistant_message_id, value.worker.generation]) : null;
                if (value.terminal_seal) {
                    if (!await verifyTerminal(serial, String(value.terminal_seal.seal_digest), value.run_id))
                        throw new Error('Observed terminal was not materialized');
                    if (!current()) return false;
                    totalMs = Date.now() - startedAt;
                    error = turn?.outcome === 'answered' || turn?.outcome === 'action_completed' ? '' : t($locale, 'modelTest.failed');
                    if (conversation) await retireConversation(conversation.conversationId);
                    return false;
                }
                return current();
            }, cause => { if (current()) error = errorMessage(cause); },
            turn.runId ? { run_id: turn.runId, assistant_message_id: turn.identity.assistantMessageId } : undefined,
            async () => {
                const ready = await reconcileSubmission(serial, runObserver.signal);
                if (current() && turn && canReleaseChatTurn(turn)) runObserver.stop();
                return current() && !!turn && !canReleaseChatTurn(turn) && ready;
            }, () => turn?.runId ? { run_id: turn.runId, assistant_message_id: turn.identity.assistantMessageId } : undefined);
    }
    async function cancelCapturedSubmission(captured: PendingSourceSubmission) {
        if (captured.request_digest === null && !captured.sent) { submissionStorage().clear(captured); return; }
        const response = await api<{ source_host_id: string; result: unknown }>('/api/chat/agent/v2/submission', {
            method: 'POST', body: JSON.stringify(sourceSubmissionQuery(captured, true)) });
        if (response.source_host_id !== captured.identity.source_host_id || response.result === null) throw new Error('Cancellation remains unresolved');
        const settled = acceptSourceReceipt(captured, response.result); submissionStorage().save(settled);
        if (settled.receipt?.phase === 'admitted') await api('/api/chat/agent/v2/cancel', { method: 'POST', body: JSON.stringify(exactSubmissionCancel(settled)) });
        else if (settledSourceSubmission(settled)) submissionStorage().clear(settled);
    }
    const activeRemote = $derived(activeApiTestModel(profiles));
    const activeLocal = $derived(activeLocalTestModel(localModels));
    const activeName = $derived(target === 'local'
        ? activeLocal?.displayName || activeLocal?.name || activeLocal?.id
        : activeRemote?.displayName || activeRemote?.modelName);
    const busy = $derived(preparing || cleaning || !!turn && !canReleaseChatTurn(turn));
    const canRun = $derived(!loading && !!hostId && !!activeName && !!prompt.trim() && !busy);

    async function load() {
        loading = true; error = '';
        try {
            const [session, remote, local] = await Promise.all([
                api<{ authenticated?: boolean; hostId?: string }>('/api/web/session'),
                api<{ profiles: RemoteProfile[] }>('/api/model-profiles'),
                api<{ models: LocalTestModel[] }>('/api/local-models'),
            ]);
            if (!session.authenticated || !session.hostId?.trim()) throw new Error('Session unavailable');
            if (disposed) return;
            hostId = session.hostId;
            clientId = sessionStorage.getItem(editorStorageKey('client')) || `web-model-test-${crypto.randomUUID()}`;
            sessionStorage.setItem(editorStorageKey('client'), clientId);
            profiles = remote.profiles;
            localModels = local.models;
            const retained = sessionStorage.getItem(editorStorageKey('pending-conversation'));
            if (retained) {
                pendingSubmission = submissionStorage().read(hostId, clientId, retained);
                if (pendingSubmission) {
                    conversation = parseChatConversation(await api(conversationUrl(retained)));
                    const serial = ++generation; const saved = pendingSubmission.identity;
                    const owner: ChatTurnOwner = { conversationId: retained, generation: serial, hostId, clientId };
                    turn = createChatTurn(owner, { submissionId: saved.submission_id, userMessageId: saved.user_message_id, assistantMessageId: saved.assistant_message_id });
                    usageProjection.reset(); contextUsage = null;
                    timeline = createChatTimeline(owner);
                    turn = reduceChatTurn(turn, { type: 'transport_interrupted', owner, reason: 'eof' }).state;
                    await reconcileSubmission(serial);
                    if (pendingSubmission?.receipt?.phase === 'admitted') {
                        turn = reduceChatTurn(turn, { type: 'run_bound', owner, runId: pendingSubmission.receipt.admission!.identity.run_id }).state;
                        await refreshRunUsage(serial);
                        await verifyTerminal(serial);
                    }
                    observeSubmission();
                }
            }
        } catch (cause) { if (!disposed) error = errorMessage(cause); }
        finally { if (!disposed) loading = false; }
    }
    onMount(() => {
        const visibility = () => { if (document.visibilityState === 'hidden') stopObservation(); else observeSubmission(); };
        document.addEventListener('visibilitychange', visibility);
        void load();
        return () => {
            disposed = true;
            generation++;
            stopObservation(); document.removeEventListener('visibilitychange', visibility);
            streamController?.abort();
            journalReader?.dispose();
            if (pendingSubmission && turn && !canReleaseChatTurn(turn)) void cancelCapturedSubmission(pendingSubmission).catch(() => {});
        };
    });

    async function retireConversation(id: string) {
        cleaning = true;
        try {
            await api(conversationUrl(id), { method: 'DELETE' });
            if (conversation?.conversationId === id) conversation = null;
        } catch {
            warning = t($locale, 'modelTest.cleanupFailed');
        } finally { cleaning = false; }
    }

    async function verifyTerminal(serial: number, sealDigest?: string, candidateRunId?: string): Promise<boolean> {
        if (pendingSubmission && pendingSubmission.receipt?.phase !== 'admitted') return false;
        if (!turn || !timeline || serial !== generation || disposed) return false;
        if (canReleaseChatTurn(turn)) return true;
        const captured = turn; const owner = captured.owner;
        if (!journalReader) journalReader = new AssistantJournalReader(owner.hostId, owner.conversationId, serial,
            (request, signal) => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(request), signal }));
        const current = () => serial === generation && !disposed && !!turn && turn.identity.submissionId === captured.identity.submissionId;
        const evidence = await readTerminal(captured, journalReader,
            request => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(request) }),
            update => {
                if (!update || !current() || !timeline) return;
                const result = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId, runId: update.through.identity.run_id, update });
                if (result.ignored && result.ignored !== 'duplicate') throw new Error('Journal timeline admission failed');
                timeline = result.state;
            }, current, sealDigest, candidateRunId);
        if (!evidence || !current() || !turn || !timeline) return false;
        const rendered = reduceChatTimeline(timeline, { type: 'terminal', owner, evidence });
        if (rendered.ignored && rendered.ignored !== 'duplicate') throw new Error('Terminal journal not materialized');
        const accepted = reduceChatTurn(turn, { type: 'terminal_verified', owner, evidence });
        if (accepted.ignored && accepted.ignored !== 'committed') throw new Error('Terminal owner changed');
        timeline = rendered.state; turn = accepted.state;
        void refreshRunUsage(serial);
        if (pendingSubmission?.receipt?.phase === 'admitted') { submissionStorage().clear(pendingSubmission); pendingSubmission = null; sessionStorage.removeItem(editorStorageKey('pending-conversation')); }
        output = timeline.messages.filter(message => evidence.heads.some(head => head.identity.assistant_message_id === message.id)).map(timelineMessageText).join('\n\n');
        if (output && firstTokenMs === null) firstTokenMs = Date.now() - startedAt;
        return canReleaseChatTurn(turn);
    }
    async function consume(path: string, body: Record<string, unknown>, serial: number) {
        if (!turn || !timeline) return;
        stopObservation();
        const owner = turn.owner;
        turn = reduceChatTurn(turn, { type: 'stream_opened', owner }).state;
        const controller = new AbortController();
        streamController = controller;
        try {
            for await (const event of chatEventStream(path, { method: 'POST', body: JSON.stringify(body), signal: controller.signal })) {
                if (serial !== generation || disposed || !turn || !timeline) return;
                if (event.type === 'submission_receipt') { acceptSubmission(event.receipt); void refreshRunUsage(serial); continue; }
                if (pendingSubmission && pendingSubmission.receipt?.phase !== 'admitted') {
                    if (['assistant_journal_committed_v1', 'assistant_run_terminal_committed_v1', 'permission_request'].includes(String(event.type)))
                        throw new Error('Model-test event preceded source admission');
                    continue;
                }
                if (event.type === 'context_usage') {
                    if (event.source_host_id === owner.hostId && event.session_id === owner.conversationId) void refreshRunUsage(serial);
                    continue;
                }
                if (event.type === 'assistant_run_terminal_committed_v1') {
                    if (event.source_host_id === owner.hostId && event.session_id === owner.conversationId
                        && (event.submission_id === undefined || event.submission_id === turn.identity.submissionId)) {
                        if (typeof event.run_id !== 'string' || turn.runId !== null && event.run_id !== turn.runId || typeof event.seal_digest !== 'string') continue;
                        await verifyTerminal(serial, event.seal_digest, event.run_id);
                    }
                    if (turn && canReleaseChatTurn(turn)) break;
                    continue;
                }
                if (event.type === 'assistant_journal_committed_v1') {
                    if (!journalReader) journalReader = new AssistantJournalReader(owner.hostId, owner.conversationId, serial,
                        (request, signal) => api('/api/chat/agent/v2/journal', { method: 'POST', body: JSON.stringify(request), signal }));
                    const submission = turn.identity.submissionId;
                    const through = parseJournalCursor(event.through);
                    if (event.source_host_id !== owner.hostId || event.session_id !== owner.conversationId || turn.runId !== null && through.identity.run_id !== turn.runId) throw new Error('Journal admission owner mismatch');
                    const root: JournalIdentity = { ...through.identity, assistant_message_id: turn.identity.assistantMessageId };
                    const declarations: readonly AdmittedMessage[] = journalReader.knownAdmission(root, through.identity.assistant_message_id) ?? await journalReader.admit(root);
                    if (!declarations.some(message => message.input.id === through.identity.assistant_message_id && message.input.role === 'assistant')) throw new Error('Journal assistant lacks message admission');
                    if (serial !== generation || disposed || !turn || !timeline || turn.identity.submissionId !== submission) return;
                    turn = reduceChatTurn(turn, { type: 'run_bound', owner, runId: root.run_id }).state;
                    timeline = reduceChatTimeline(timeline, { type: 'admissions', owner, messages: declarations }).state;
                    const update = await journalReader.notification(event, turn.runId ?? submission);
                    if (serial !== generation || disposed || !turn || !timeline || turn.identity.submissionId !== submission) return;
                    if (update) {
                        const result = reduceChatTimeline(timeline, { type: 'journal', owner, sourceHostId: owner.hostId,
                            runId: update.through.identity.run_id, update });
                        if (result.ignored && result.ignored !== 'duplicate') throw new Error('Journal timeline admission failed');
                        timeline = result.state;
                    }
                    const assistant = timeline.messages.find(message => message.id === turn?.identity.assistantMessageId);
                    output = assistant ? timelineMessageText(assistant) : '';
                    if (output && firstTokenMs === null) firstTokenMs = Date.now() - startedAt;
                    continue; // Only the independent terminal contract unlocks this run.
                }
                turn = reduceChatTurn(turn, { type: 'event', owner, payload: event }).state;
                const assistant = timeline.messages.find(message => message.id === turn?.identity.assistantMessageId);
                const text = assistant ? timelineMessageText(assistant) : '';
                if (text && firstTokenMs === null) firstTokenMs = Date.now() - startedAt;
                output = text;
                if (canReleaseChatTurn(turn)) break;
            }
            if (serial !== generation || disposed || !turn) return;
            if (!canReleaseChatTurn(turn)) { await reconcileSubmission(serial); await verifyTerminal(serial); }
            if (serial !== generation || disposed || !turn) return;
            if (canReleaseChatTurn(turn)) {
                totalMs = Date.now() - startedAt;
                if (turn.outcome !== 'answered' && turn.outcome !== 'action_completed') error = t($locale, 'modelTest.failed');
                if (conversation) await retireConversation(conversation.conversationId);
            } else {
                turn = reduceChatTurn(turn, { type: 'transport_interrupted', owner, reason: 'eof' }).state;
                error = t($locale, 'modelTest.interrupted');
            }
        } catch (cause) {
            if (serial !== generation || disposed || !turn) return;
            try {
                await reconcileSubmission(serial);
                if (await verifyTerminal(serial)) {
                    totalMs = Date.now() - startedAt;
                    if (turn?.outcome !== 'answered' && turn?.outcome !== 'action_completed') error = t($locale, 'modelTest.failed');
                    if (conversation) await retireConversation(conversation.conversationId);
                    return;
                }
            } catch { /* Recovery stays uncertain when the Store is unavailable. */ }
            if (serial !== generation || disposed || !turn) return;
            turn = reduceChatTurn(turn, { type: 'transport_interrupted', owner, reason: 'error' }).state;
            error = errorMessage(cause);
        } finally {
            if (streamController === controller) streamController = null;
            if (serial === generation && !disposed) observeSubmission();
        }
    }

    async function run() {
        if (!canRun) return;
        const model = target === 'local' ? activeLocal : activeRemote;
        if (!model) return;
        const serial = ++generation;
        usageProjection.reset(); contextUsage = null;
        stopObservation(); consumedResumeAuthorities.clear();
        journalReader?.dispose(); journalReader = null;
        compactPane = 'output';
        preparing = true; error = ''; warning = ''; output = ''; firstTokenMs = null; totalMs = null;
        try {
            if (target === 'local' && activeLocal && !activeLocal.running) {
                await api(`/api/local-models/${encodeURIComponent(activeLocal.id)}/start`, { method: 'POST' });
                if (serial !== generation || disposed) return;
                localModels = localModels.map(item => item.id === activeLocal.id ? { ...item, running: true } : item);
            }
            const created = parseChatConversation(await api('/api/chat/conversations', {
                method: 'POST', body: JSON.stringify(modelTestConversationRequest(target, model.id, t($locale, 'modelTest.title'))),
            }));
            if (serial !== generation || disposed) { await retireConversation(created.conversationId); return; }
            conversation = created;
            const owner: ChatTurnOwner = { conversationId: created.conversationId, generation: serial, hostId, clientId };
            const submissionId = crypto.randomUUID();
            turn = createChatTurn(owner, chatTurnIdentity(submissionId));
            timeline = reduceChatTimeline(createChatTimeline(owner), { type: 'beginSubmission', owner, submissionId }).state;
            startedAt = Date.now();
            preparing = false;
            const request = { ...modelTestRunRequest(
                created, clientId, submissionId, prompt, $locale,
                Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
            ), expected_host_id: hostId };
            const frozenRequest = await freezeHostToolPlan(request, api);
            if (serial !== generation || disposed) return;
            pendingSubmission = createSourceSubmission(sourceIdentity(owner, turn.identity), frozenRequest);
            submissionStorage().save(pendingSubmission); sessionStorage.setItem(editorStorageKey('pending-conversation'), created.conversationId);
            const captured = pendingSubmission;
            const prepared = await api('/api/chat/agent/v2/submission', { method: 'POST', body: JSON.stringify({ operation: 'prepare', identity: captured.identity, request: captured.request }) });
            if (serial !== generation || disposed || pendingSubmission?.identity.submission_id !== captured.identity.submission_id) return;
            pendingSubmission = bindSourceDigest(captured, prepared); submissionStorage().save(pendingSubmission);
            const body = sourceSubmissionBody(pendingSubmission);
            pendingSubmission = { ...pendingSubmission, sent: true }; submissionStorage().save(pendingSubmission);
            await consume('/api/chat/agent/v2/stream', body, serial);
        } catch (cause) { if (serial === generation && !disposed) error = errorMessage(cause); }
        finally { if (serial === generation && !disposed) { preparing = false; observeSubmission(); } }
    }

    async function resume() {
        if (!turn || turn.phase !== 'recovering' || observedRecovery !== 'resume_required' || !resumeAuthority || consumedResumeAuthorities.has(resumeAuthority)) return;
        const body = buildChatResumeRequest(turn);
        if (!body) { error = t($locale, 'modelTest.interrupted'); return; }
        // Lost ACK/409 returns to observation, never repeats execution for the
        // same observed worker authority or creates a replacement submission.
        consumedResumeAuthorities.add(resumeAuthority);
        error = '';
        await consume('/api/chat/agent/v2/resume', body, generation);
    }

    async function stop() {
        if (!conversation || !turn || !pendingSubmission || canReleaseChatTurn(turn) || turn.phase === 'cancel_requested') return;
        try {
            if (pendingSubmission.request_digest === null && !pendingSubmission.sent) {
                submissionStorage().clear(pendingSubmission); pendingSubmission = null;
                turn = reduceChatTurn(turn, { type: 'submission_not_admitted', owner: turn.owner, outcome: 'cancelled' }).state;
                stopObservation(); sessionStorage.removeItem(editorStorageKey('pending-conversation'));
                return;
            }
            const captured = pendingSubmission;
            const response = await api<{ source_host_id: string; result: unknown }>('/api/chat/agent/v2/submission', {
                method: 'POST', body: JSON.stringify(sourceSubmissionQuery(captured, true)) });
            if (pendingSubmission?.identity.submission_id !== captured.identity.submission_id || response.source_host_id !== hostId || response.result === null) throw new Error('Cancellation remains unresolved');
            acceptSubmission(response.result);
            if (!pendingSubmission) { stopObservation(); return; }
            const result = await api<{ cancelRequested?: boolean; terminationConfirmed?: boolean }>('/api/chat/agent/v2/cancel', {
                method: 'POST', body: JSON.stringify(exactSubmissionCancel(pendingSubmission)),
            });
            if (result.cancelRequested || result.terminationConfirmed) turn = reduceChatTurn(turn, { type: 'cancel_requested', owner: turn.owner }).state;
            else error = t($locale, 'modelTest.interrupted');
        } catch (cause) { error = errorMessage(cause); }
    }

    function clear() {
        if (busy) return;
        stopObservation();
        usageProjection.reset(); contextUsage = null;
        journalReader?.dispose(); journalReader = null;
        output = ''; error = ''; warning = ''; firstTokenMs = null; totalMs = null; turn = null; timeline = null;
    }
</script>

<svelte:head>{#if pageLocation.active()}<title>{t($locale, 'modelTest.title')} — Nola</title>{/if}</svelte:head>
<div class="model-test-page">
    <header class="page-header">
        <div><h1>{t($locale, 'modelTest.title')}</h1><p class="muted">{t($locale, 'modelTest.subtitle')}</p></div>
        <div class="actions"><a href="/models">{t($locale, 'modelTest.back')}</a>{#if target === 'local' ? activeLocal : activeRemote}<a class="button" href={modelTestParametersUrl(target, (target === 'local' ? activeLocal : activeRemote)!.id)}>{t($locale, 'modelTest.parameters')}</a>{/if}<button class="primary" onclick={busy ? stop : run} disabled={loading || cleaning || turn?.phase === 'cancel_requested' || !busy && !canRun}>{t($locale, busy ? 'modelTest.stop' : 'modelTest.run')}</button></div>
    </header>
    {#if error}<p class="notice error" role="alert">{error}</p>{/if}
    {#if warning}<p class="notice" role="status">{warning}{#if conversation}<a href={`/chat?conversation=${encodeURIComponent(conversation.conversationId)}`}>{t($locale, 'modelTest.openConversation')}</a>{/if}</p>{/if}
    <div class="tabs compact-panes" role="tablist" aria-label={t($locale, 'modelTest.pane')}>
        <button role="tab" aria-controls="model-test-setup" aria-selected={compactPane === 'setup'} tabindex={compactPane === 'setup' ? 0 : -1} onclick={() => compactPane = 'setup'}>{t($locale, 'modelTest.setup')}</button>
        <button role="tab" aria-controls="model-test-output" aria-selected={compactPane === 'output'} tabindex={compactPane === 'output' ? 0 : -1} onclick={() => compactPane = 'output'}>{t($locale, 'modelTest.output')}</button>
    </div>
    <div class="model-test-grid" data-active-pane={compactPane}>
        <section id="model-test-setup" class="card setup">
            <h2>{t($locale, 'modelTest.setup')}</h2>
            <div class="tabs" role="tablist" aria-label={t($locale, 'models.type')}>
                <button role="tab" aria-selected={target === 'local'} onclick={() => target = 'local'} disabled={busy}>{t($locale, 'models.local')}</button>
                <button role="tab" aria-selected={target === 'cloud'} onclick={() => target = 'cloud'} disabled={busy}>{t($locale, 'models.api')}</button>
            </div>
            <p class="model-name">{activeName || t($locale, 'modelTest.notConfigured')}</p>
            <label for="model-test-prompt">{t($locale, 'modelTest.prompt')}</label>
            <textarea id="model-test-prompt" bind:value={prompt} disabled={busy} rows="9"></textarea>
            <p class="muted">{t($locale, 'modelTest.safeHint')}</p>
            <div class="metrics">
                <h3>{t($locale, 'modelTest.metrics')}</h3>
                <p>{t($locale, 'modelTest.firstToken')} <strong>{firstTokenMs === null ? '—' : `${(firstTokenMs / 1000).toFixed(2)} s`}</strong></p>
                <p>{t($locale, 'modelTest.totalTime')} <strong>{totalMs === null ? '—' : `${(totalMs / 1000).toFixed(2)} s`}</strong></p>
                <p>{t($locale, 'modelTest.characters')} <strong>{[...output].length}</strong></p>
                <p>{t($locale, 'chatOverview.context')} <strong>{contextUsage ? `${BigInt(contextUsage.usage.used_tokens).toLocaleString($locale)} / ${BigInt(contextUsage.usage.context_window_tokens).toLocaleString($locale)}` : '—'}</strong></p>
                {#if contextUsage && contextUsageFraction(contextUsage) !== null}<progress value={contextUsageFraction(contextUsage)!} max="1" aria-label={t($locale, 'chatOverview.context')}></progress>{/if}
                {#if contextUsage?.usage.estimated}<p class="muted">{t($locale, 'chatOverview.estimated')}</p>{/if}
            </div>
        </section>
        <section id="model-test-output" class="card output">
            <header><h2>{t($locale, 'modelTest.output')}</h2><div class="actions">{#if turn?.phase === 'recovering'}<button onclick={resume} disabled={!turn.runId || observedRecovery !== 'resume_required' || !resumeAuthority || consumedResumeAuthorities.has(resumeAuthority)}>{t($locale, 'modelTest.resume')}</button>{/if}{#if conversation && (turn?.phase === 'recovering' || turn?.phase === 'cancel_requested')}<a href={`/chat?conversation=${encodeURIComponent(conversation.conversationId)}`}>{t($locale, 'modelTest.openConversation')}</a>{/if}<button onclick={clear} disabled={busy || !output && !error}>{t($locale, 'modelTest.clear')}</button></div></header>
            {#if busy}<p role="status" class="muted">{t($locale, 'modelTest.running')}</p>{/if}
            {#if output}<div class="rendered-output"><Markdown source={output} /></div>{:else}<p class="empty muted">{t($locale, 'modelTest.empty')}</p>{/if}
        </section>
    </div>
</div>

<style>
    .model-test-page { min-height: 100%; container: model-test / inline-size; }
    .page-header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 20px; }
    .page-header h1 { margin-bottom: 4px; }
    .actions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
    .model-test-grid { display: grid; grid-template-columns: minmax(280px, 360px) minmax(0, 1fr); gap: 16px; padding: 20px; }
    .compact-panes { display: none; }
    .card { min-width: 0; padding: 20px; }
    .setup { display: flex; flex-direction: column; gap: 12px; }
    .setup h2, .output h2 { margin: 0; }
    .tabs { display: flex; gap: 4px; }
    .model-name { font-weight: 600; margin: 0; }
    textarea { width: 100%; min-height: 160px; resize: vertical; }
    .metrics { margin-top: 12px; padding: 14px; border-radius: 12px; background: color-mix(in srgb, currentColor 4%, transparent); }
    .metrics h3 { margin: 0 0 12px; font-size: 0.95rem; }
    .metrics p { display: flex; justify-content: space-between; margin: 7px 0; }
    .output { display: flex; flex-direction: column; min-height: 420px; }
    .output header { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
    .rendered-output { padding: 20px 0; overflow-wrap: anywhere; }
    .empty { margin: auto; text-align: center; }
    @container model-test (max-width: 820px) {
        .compact-panes { display: flex; margin: 16px auto 0; }
        .model-test-grid { grid-template-columns: 1fr; }
        .model-test-grid[data-active-pane="setup"] .output,
        .model-test-grid[data-active-pane="output"] .setup { display: none; }
        .page-header { align-items: flex-start; }
    }
</style>
