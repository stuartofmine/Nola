// SPDX-License-Identifier: MIT
// Versioned Agent Trace reducer and SSE decoder for the CLI Web surface.
(function installNolaAgentTrace(global) {
    'use strict';

    const CONTROL_TRACE_KINDS = new Set([
        'run.started', 'prompt.composed', 'phase.changed', 'tool.started', 'tool.progress',
        'tool.permission.requested', 'tool.permission.resolved', 'tool.completed',
        'hook.started', 'hook.completed', 'plan.updated', 'subagent.updated',
        'usage.snapshot', 'memory.used', 'memory.updated', 'context.compacted',
        'validation.updated', 'run.completed', 'run.failed', 'run.cancelled',
    ]);

    function asObject(value) {
        return value && typeof value === 'object' && !Array.isArray(value) ? value : {};
    }

    function firstText(...values) {
        const value = values.find(item => typeof item === 'string' && item.length);
        return value || '';
    }

    function owns(value, field) {
        return Object.prototype.hasOwnProperty.call(value, field);
    }

    function publicText(part, field) {
        // Review is disclosure state, not completion or execution authority.
        // Missing/null means unreviewed public text; every non-approved state
        // withholds the draft, including an unknown malformed state.
        if (part.review != null && part.review !== 'approved') return '';
        return typeof part[field] === 'string' ? part[field] : '';
    }

    function outputReference(value, messageId, partId, callId) {
        if (value == null) return null;
        const reference = asObject(value);
        const fields = ['message_id', 'part_id', 'tool_call_id'];
        if (Object.keys(reference).length !== fields.length
            || fields.some(field => !owns(reference, field))
            || reference.message_id !== messageId || reference.part_id !== partId
            || reference.tool_call_id !== callId) {
            throw new Error('Invalid public tool-output owner');
        }
        // This descriptor is display-only. It contains no private path and is
        // not a read capability, permission grant or executable request.
        return { ...reference };
    }

    function publicSourceEvidence(value) {
        if (value == null) return null;
        const evidence = asObject(value);
        const usage = asObject(evidence.usage);
        const textFields = ['source', 'title', 'observed_at', 'event_time', 'range_start', 'range_end'];
        const fields = ['record_id', 'component_id', 'scope', 'namespace', 'provenance_kind', 'source_role',
            'requires_current_state_verification', 'truncated', 'supersedes', 'usage_count', 'last_used_at_epoch_ms', ...textFields];
        if (evidence.kind !== 'memory' || Object.keys(evidence).some(field => !['kind', 'usage'].includes(field))
            || !owns(evidence, 'usage') || Object.keys(usage).some(field => !fields.includes(field))
            || typeof usage.record_id !== 'string' || typeof usage.component_id !== 'string'
            || textFields.some(field => usage[field] != null && typeof usage[field] !== 'string')
            || ['requires_current_state_verification', 'truncated'].some(field => usage[field] != null && typeof usage[field] !== 'boolean')
            || (usage.supersedes != null && (!Array.isArray(usage.supersedes) || usage.supersedes.some(item => typeof item !== 'string')))) {
            throw new Error('Invalid public source evidence');
        }
        for (const [field, allowed] of [
            ['scope', ['turn', 'conversation', 'global']],
            ['provenance_kind', ['explicit_user', 'agent_inference', 'tool_result', 'imported', 'synced', 'other']],
            ['source_role', ['user', 'assistant', 'tool', 'system', 'other']],
        ]) {
            if (usage[field] != null && !allowed.includes(usage[field])) throw new Error('Invalid source evidence enum');
        }
        if (usage.namespace != null) {
            const namespace = asObject(usage.namespace);
            if (!['personal', 'workspace', 'conversation', 'custom'].includes(namespace.kind)
                || Object.keys(namespace).some(field => !(namespace.kind === 'custom' ? ['kind', 'name'] : ['kind']).includes(field))
                || (namespace.kind === 'custom' && typeof namespace.name !== 'string')) throw new Error('Invalid memory namespace');
        }
        // Native JSON numbers cannot represent all Domain u64/i64 values. Fail
        // explicitly here until this transport supplies lossless numeric tokens.
        if ((usage.usage_count != null && (!Number.isSafeInteger(usage.usage_count) || usage.usage_count < 0))
            || (usage.last_used_at_epoch_ms != null && !Number.isSafeInteger(usage.last_used_at_epoch_ms))) {
            throw new Error('Source evidence requires lossless integer transport');
        }
        return evidence;
    }

    function numericSequence(value) {
        const sequence = Number(value);
        return Number.isSafeInteger(sequence) && sequence > 0 ? sequence : 0;
    }

    function contextUsageSnapshot(value) {
        const usage = asObject(value);
        const usedTokens = Math.max(0, Number(usage.used_tokens ?? usage.usedTokens ?? 0));
        const contextWindowTokens = Math.max(0, Number(
            usage.context_window_tokens ?? usage.contextWindowTokens ?? 0
        ));
        const promptTokenLimit = Math.max(0, Number(
            usage.prompt_token_limit ?? usage.promptTokenLimit ?? contextWindowTokens
        ));
        const computedPercent = contextWindowTokens > 0
            ? Math.min(100, usedTokens / contextWindowTokens * 100)
            : 0;
        return {
            used_tokens: Number.isFinite(usedTokens) ? usedTokens : 0,
            context_window_tokens: Number.isFinite(contextWindowTokens) ? contextWindowTokens : 0,
            prompt_token_limit: Number.isFinite(promptTokenLimit) ? promptTokenLimit : 0,
            usage_percent: Number.isFinite(Number(usage.usage_percent ?? usage.usagePercent))
                ? Number(usage.usage_percent ?? usage.usagePercent)
                : computedPercent,
            usage_is_estimated: Boolean(
                usage.usage_is_estimated ?? usage.usageIsEstimated ?? true
            ),
            cached_input_tokens: Math.max(0, Number(
                usage.cached_input_tokens ?? usage.cachedInputTokens ?? 0
            )),
            cached_input_tokens_reported: Boolean(
                // Missing cache counters remain unknown rather than a miss.
                usage.cached_input_tokens_reported ?? usage.cachedInputTokensReported ?? false
            ),
            cache_usage: usage.cache_usage ?? null,
            cache_write_input_tokens: Math.max(0, Number(
                usage.cache_write_input_tokens ?? usage.cacheWriteInputTokens ?? 0
            )),
            cache_write_input_tokens_reported: Boolean(
                usage.cache_write_input_tokens_reported
                    ?? usage.cacheWriteInputTokensReported
                    ?? false
            ),
        };
    }

    function toolResultText(result) {
        const value = asObject(result);
        const direct = firstText(
            typeof result === 'string' ? result : '',
            value.text,
            value.display,
            value.message,
            value.output_preview,
            value.outputPreview,
            asObject(value.result).text,
        );
        if (direct) return direct;
        try {
            return result === undefined ? '' : JSON.stringify(result, null, 2);
        } catch (_) {
            return String(result || '');
        }
    }

    function toolCallId(trace) {
        const payload = asObject(trace.payload);
        return firstText(
            payload.call_id,
            trace.span_id,
            `tool-${trace.sequence}`,
        );
    }

    function makeRunState(trace) {
        return {
            schemaVersion: Number(trace.schema_version || 1),
            traceId: firstText(trace.trace_id),
            runId: firstText(trace.run_id),
            conversationId: firstText(trace.conversation_id),
            messageId: firstText(trace.message_id),
            turnId: firstText(trace.turn_id),
            nextSequence: 1,
            lastSequence: 0,
            status: 'running',
            phase: '',
            phaseDetail: '',
            assistantText: '',
            partOrder: [],
            parts: new Map(),
            latestPartCursorById: new Map(),
            thinkingText: '',
            toolOrder: [],
            tools: new Map(),
            memories: [],
            usage: null,
            validation: null,
            terminal: false,
            terminalKind: '',
            failure: '',
            surface: null,
            version: 0,
            seenEventIds: new Set(),
            pendingBySequence: new Map(),
            diagnostics: [],
        };
    }

    function currentTool(state) {
        const currentPartId = firstText(state.surface?.currentActivityPartId);
        if (currentPartId) {
            const projected = state.toolOrder
                .map(callId => state.tools.get(callId))
                .find(tool => firstText(tool?.partId) === currentPartId);
            if (projected) return { ...projected };
        }
        const activeStatuses = new Set([
            'queued',
            'running',
            'waiting',
            'permission_required',
            'waiting_for_input',
            'suspended',
            'retry_waiting',
        ]);
        for (let index = state.toolOrder.length - 1; index >= 0; index -= 1) {
            const tool = state.tools.get(state.toolOrder[index]);
            if (tool && activeStatuses.has(String(tool.status || ''))) {
                return { ...tool };
            }
        }
        return null;
    }

    function upsertTool(state, trace, payload, kind) {
        const callId = toolCallId(trace);
        const previous = state.tools.get(callId) || {};
        const status = kind === 'tool.permission.requested'
            ? 'permission_required'
            : kind === 'tool.permission.resolved'
                ? (trace.status === 'failed' ? 'failed' : 'running')
                : kind === 'tool.completed'
                    ? firstText(trace.status, 'succeeded')
                    : firstText(trace.status, previous.status, 'running');
        if (!state.tools.has(callId)) state.toolOrder.push(callId);
        state.tools.set(callId, {
            callId,
            name: firstText(payload.tool_name, previous.name, 'unknown'),
            arguments: payload.arguments ?? previous.arguments ?? null,
            result: payload.result ?? previous.result ?? null,
            resultText: payload.result === undefined
                ? firstText(previous.resultText)
                : toolResultText(payload.result),
            resources: Array.isArray(payload.resources)
                ? payload.resources.map(resource => ({ ...asObject(resource) }))
                : (previous.resources || []),
            status,
            summary: firstText(payload.summary, previous.summary),
            detail: firstText(payload.detail, previous.detail),
            progress: Number.isFinite(Number(payload.progress))
                ? Number(payload.progress)
                : (previous.progress ?? null),
            version: Number(previous.version || 0) + 1,
        });
    }

    function publicProjection(state) {
        const projectedParts = state.partOrder
            .map(partId => state.parts.get(partId))
            .filter(Boolean)
            .map(part => ({ ...part }));
        return {
            schemaVersion: state.schemaVersion,
            traceId: state.traceId,
            runId: state.runId,
            conversationId: state.conversationId,
            messageId: state.messageId,
            turnId: state.turnId,
            nextSequence: state.nextSequence,
            lastSequence: state.lastSequence,
            status: state.status,
            phase: state.phase,
            phaseDetail: state.phaseDetail,
            assistantText: state.assistantText,
            parts: projectedParts,
            sources: projectedParts.filter(part => part.kind === 'source'),
            citations: projectedParts.filter(part => part.kind === 'citation'),
            compactions: projectedParts.filter(part => part.kind === 'compaction'),
            modelAttempts: projectedParts.filter(part => part.kind === 'model_attempt'),
            thinkingText: state.thinkingText,
            tools: state.toolOrder
                .map(callId => state.tools.get(callId))
                .filter(Boolean)
                .map(tool => ({ ...tool })),
            currentTool: currentTool(state),
            memories: state.memories.map(memory => ({ ...memory })),
            usage: state.usage ? { ...state.usage } : null,
            validation: state.validation ? { ...state.validation } : null,
            terminal: state.terminal,
            terminalKind: state.terminalKind,
            failure: state.failure,
            surface: state.surface ? {
                ...state.surface,
                archivedActivityPartIds: [...state.surface.archivedActivityPartIds],
            } : null,
            version: state.version,
            diagnostics: state.diagnostics.map(item => ({ ...item })),
        };
    }

    function presentationFingerprint(state) {
        return JSON.stringify({
            status: state.status,
            phase: state.phase,
            phaseDetail: state.phaseDetail,
            assistantText: state.assistantText,
            parts: state.partOrder.map(partId => state.parts.get(partId)),
            thinkingText: state.thinkingText,
            tools: state.toolOrder.map(callId => state.tools.get(callId)),
            memories: state.memories,
            usage: state.usage,
            validation: state.validation,
            terminal: state.terminal,
            terminalKind: state.terminalKind,
            failure: state.failure,
            surface: state.surface,
        });
    }

    function applyEvent(state, entry) {
        const { trace } = entry;
        const payload = asObject(trace.payload);
        const kind = firstText(trace.kind, 'phase.changed');
        const before = presentationFingerprint(state);
        state.lastSequence = trace.sequence;
        state.nextSequence = trace.sequence + 1;
        state.status = firstText(trace.status, state.status);

        switch (kind) {
            case 'run.started':
                state.phase = firstText(payload.phase, state.phase);
                state.phaseDetail = firstText(payload.detail, payload.summary, state.phaseDetail);
                break;
            case 'phase.changed':
                state.phase = firstText(payload.phase, state.phase);
                state.phaseDetail = firstText(payload.detail, payload.summary, state.phaseDetail);
                break;
            case 'context.compacted':
                state.phase = 'processing';
                state.phaseDetail = '';
                state.status = 'running';
                break;
            case 'tool.started':
            case 'tool.progress':
            case 'tool.permission.requested':
            case 'tool.permission.resolved':
            case 'tool.completed': {
                if (kind === 'tool.progress' && !payload.tool_name && !payload.call_id) {
                    if (state.phase !== 'compacting') {
                        if (payload.summary === 'model_retry') state.phase = 'model_retry';
                        if (payload.summary === 'model_attempt_started') state.phase = 'processing';
                        if (['model_retry', 'model_attempt_started'].includes(payload.summary)) state.phaseDetail = '';
                    }
                    break;
                }
                if (kind === 'tool.progress' && payload.summary === 'model_retry') {
                    state.phase = 'model_retry';
                    state.phaseDetail = '';
                }
                upsertTool(state, trace, payload, kind);
                break;
            }
            case 'usage.snapshot':
                state.usage = {
                    ...contextUsageSnapshot(payload.usage),
                    prompt_cache_diagnostic: asObject(payload.prompt_cache_diagnostic),
                };
                break;
            case 'memory.used':
                state.memories = Array.isArray(payload.memories)
                    ? payload.memories.map(memory => ({ ...asObject(memory) }))
                    : [];
                break;
            case 'validation.updated':
                state.validation = asObject(payload.validation);
                break;
            case 'run.completed':
            case 'run.failed':
            case 'run.cancelled':
                if (['compacting', 'model_retry'].includes(state.phase)) {
                    state.phase = '';
                    state.phaseDetail = '';
                }
                state.terminal = true;
                state.terminalKind = kind;
                state.status = kind === 'run.completed'
                    ? 'succeeded'
                    : kind === 'run.cancelled'
                        ? 'cancelled'
                        : 'failed';
                state.failure = kind === 'run.failed'
                    ? firstText(payload.summary, payload.detail, payload.reason)
                    : '';
                break;
            default:
                break;
        }
        if (presentationFingerprint(state) !== before) state.version += 1;
        return entry;
    }

    function createTraceStore(options = {}) {
        const runs = new Map();
        const coreSessions = new Map();
        const taskSessions = new Map();
        const skillSessions = new Map();
        const pendingCapabilityActivations = new Map();
        const maximumPendingCapabilityTurns = 128;

        function capabilityActivationKey(sessionId, turnId) {
            return `${sessionId}::${turnId}`;
        }

        function stageCapabilityActivation(event) {
            const key = capabilityActivationKey(event.sessionId, event.turnId);
            let bySource = pendingCapabilityActivations.get(key);
            if (!bySource) {
                bySource = new Map();
                pendingCapabilityActivations.set(key, bySource);
                while (pendingCapabilityActivations.size > maximumPendingCapabilityTurns) {
                    pendingCapabilityActivations.delete(pendingCapabilityActivations.keys().next().value);
                }
            }
            const previous = bySource.get(event.source);
            if (!previous || previous.cursor < event.cursor) bySource.set(event.source, event);
        }

        function applyCapabilityActivation(state, event) {
            const callId = `core-capability:${event.turnId}:${event.source}`;
            const previous = state.tools.get(callId);
            if (previous && Number(previous.activationCursor || 0) >= event.cursor) return false;
            if (!previous) state.toolOrder.push(callId);
            state.tools.set(callId, {
                callId,
                name: event.source,
                arguments: null,
                result: event.capabilityIds,
                resultText: event.capabilityIds.join('\n'),
                capabilityIds: [...event.capabilityIds],
                resources: [],
                status: 'succeeded',
                summary: event.capabilityIds.join(' · '),
                detail: '',
                progress: null,
                version: Number(previous?.version || 0) + 1,
                activationCursor: event.cursor,
            });
            state.version += 1;
            emit(state, [], 'capabilities_activated');
            return true;
        }

        function registerRun(key, state) {
            runs.set(key, state);
            const pending = pendingCapabilityActivations.get(
                capabilityActivationKey(state.conversationId, state.turnId)
            );
            pending?.forEach(event => applyCapabilityActivation(state, event));
            return state;
        }
        function publicCoreProjection(state) {
            return {
                sessionId: state.sessionId,
                cursor: state.cursor,
                revision: state.revision,
                projectionRevision: state.projectionRevision,
                order: [...state.order],
                parts: Object.fromEntries(state.order.map(id => [id, { ...state.parts.get(id) }])),
                latestProjectionRevisionByPart: Object.fromEntries(state.latestByPart),
                terminalByTurn: Object.fromEntries(state.terminalByTurn),
                version: state.version,
            };
        }

        function normalizedTask(value) {
            const task = asObject(value);
            const durability = asObject(task.durability);
            const lineage = asObject(task.lineage);
            const optionalNumber = value => Number.isFinite(Number(value)) ? Number(value) : null;
            return {
                id: firstText(task.id),
                ownerSessionId: firstText(task.owner_session_id, task.ownerSessionId),
                ownerTurnId: firstText(task.owner_turn_id, task.ownerTurnId),
                kind: firstText(task.kind),
                state: firstText(task.state),
                progress: Number.isFinite(Number(task.progress)) ? Number(task.progress) : null,
                currentStep: firstText(task.current_step, task.currentStep),
                resultRef: firstText(task.result_ref, task.resultRef),
                cancelRequested: Boolean(task.cancel_requested ?? task.cancelRequested),
                lineage: {
                    parentTaskId: firstText(lineage.parent_task_id, lineage.parentTaskId),
                    childSessionId: firstText(lineage.child_session_id, lineage.childSessionId),
                    parentCallId: firstText(lineage.parent_call_id, lineage.parentCallId),
                    permissionScopeId: firstText(lineage.permission_scope_id, lineage.permissionScopeId),
                    cancellationScopeId: firstText(lineage.cancellation_scope_id, lineage.cancellationScopeId),
                },
                durability: {
                    workerId: firstText(durability.worker_id, durability.workerId),
                    ownershipToken: firstText(durability.ownership_token, durability.ownershipToken),
                    leaseUntilEpochMs: optionalNumber(durability.lease_until_epoch_ms ?? durability.leaseUntilEpochMs),
                    lastHeartbeatEpochMs: optionalNumber(durability.last_heartbeat_epoch_ms ?? durability.lastHeartbeatEpochMs),
                    retryCount: Math.max(0, optionalNumber(durability.retry_count ?? durability.retryCount) || 0),
                    retryAtEpochMs: optionalNumber(durability.retry_at_epoch_ms ?? durability.retryAtEpochMs),
                    lastError: firstText(durability.last_error, durability.lastError),
                    inputWatermark: optionalNumber(durability.input_watermark ?? durability.inputWatermark),
                    completionWatermark: optionalNumber(durability.completion_watermark ?? durability.completionWatermark),
                },
            };
        }

        function publicTaskProjection(state) {
            return {
                sessionId: state.sessionId,
                revision: state.revision,
                order: [...state.order],
                tasks: Object.fromEntries(state.order.map(id => [id, { ...state.tasks.get(id) }])),
                guidance: Object.fromEntries(Array.from((state.guidance || new Map()).entries(), ([id, queue]) => [id, {
                    ...queue,
                    pending: queue.pending.map(message => ({ ...message })),
                    acknowledgedMessageIds: [...queue.acknowledgedMessageIds],
                }])),
                version: state.version,
            };
        }

        function normalizedTaskGuidanceQueue(value) {
            const queue = asObject(value);
            const pending = Array.isArray(queue.pending) ? queue.pending.map(item => {
                const message = asObject(item);
                return {
                    sequence: Math.max(0, Number(message.sequence || 0)),
                    messageId: firstText(message.message_id, message.messageId),
                    parentSessionId: firstText(message.parent_session_id, message.parentSessionId),
                    parentTurnId: firstText(message.parent_turn_id, message.parentTurnId),
                    content: firstText(message.content),
                    enqueuedAtEpochMs: Number(message.enqueued_at_epoch_ms ?? message.enqueuedAtEpochMs ?? 0),
                };
            }).filter(message => message.messageId && message.content) : [];
            return {
                lastSequence: Math.max(0, Number(queue.last_sequence ?? queue.lastSequence ?? 0)),
                acknowledgedSequence: Math.max(0, Number(queue.acknowledged_sequence ?? queue.acknowledgedSequence ?? 0)),
                pending,
                acknowledgedMessageIds: Array.isArray(queue.acknowledged_message_ids)
                    ? queue.acknowledged_message_ids.map(String)
                    : (Array.isArray(queue.acknowledgedMessageIds) ? queue.acknowledgedMessageIds.map(String) : []),
            };
        }

        function emitTasks(state, reason, task = null) {
            const projection = publicTaskProjection(state);
            options.onTaskProjection?.(projection, { reason, task });
            return projection;
        }

        function normalizedSkillRun(value) {
            const run = asObject(value);
            return {
                runId: firstText(run.run_id, run.runId),
                owner: { ...asObject(run.owner) },
                descriptor: { ...asObject(run.descriptor) },
                mode: firstText(run.mode),
                status: firstText(run.status),
                budget: { ...asObject(run.budget) },
                usage: { ...asObject(run.usage) },
                missingTools: Array.isArray(run.missing_tools) ? [...run.missing_tools] : [],
                catalogRevision: Math.max(0, Number(run.catalog_revision || 0)),
                resultRef: firstText(run.result_ref, run.resultRef),
                failure: firstText(run.failure),
            };
        }

        function publicSkillProjection(state) {
            return {
                sessionId: state.sessionId,
                revision: state.revision,
                cursor: state.cursor,
                catalogRevision: state.catalogRevision,
                order: [...state.order],
                runs: Object.fromEntries(state.order.map(id => [id, { ...state.runs.get(id) }])),
                version: state.version,
            };
        }

        function emitSkills(state, reason) {
            const projection = publicSkillProjection(state);
            options.onSkillProjection?.(projection, { reason });
            return projection;
        }

        function ingestSkillProjection(projectionValue, identity = {}) {
            const projection = asObject(projectionValue);
            const requestedSessionId = firstText(identity.conversationId, identity.sessionId);
            const sessionId = firstText(projection.sessionId);
            const cursor = Number(projection.cursor);
            const revision = Number(projection.revision);
            const catalogRevision = Number(projection.catalogRevision);
            const sourceRuns = asObject(projection.runs);
            const order = Array.isArray(projection.order) ? projection.order.map(String) : null;
            const validCounter = value => Number.isSafeInteger(value) && value >= 0;
            if (!requestedSessionId || sessionId !== requestedSessionId
                || !validCounter(cursor) || !validCounter(revision)
                || !validCounter(catalogRevision) || !order) {
                return { state: null, ignored: 'invalid_skill_projection' };
            }
            const sourceIds = Object.keys(sourceRuns);
            if (new Set(order).size !== order.length
                || order.length !== sourceIds.length
                || order.some(id => !Object.prototype.hasOwnProperty.call(sourceRuns, id))) {
                return { state: null, ignored: 'invalid_skill_order' };
            }
            const restoredRuns = new Map();
            for (const runId of order) {
                const sourceRun = asObject(sourceRuns[runId]);
                const owner = asObject(sourceRun.owner);
                if (!runId || firstText(sourceRun.run_id) !== runId
                    || firstText(owner.session_id) !== sessionId
                    || !firstText(owner.parent_turn_id)) {
                    return { state: null, ignored: 'invalid_skill_owner' };
                }
                restoredRuns.set(runId, normalizedSkillRun(sourceRun));
            }
            const existing = skillSessions.get(sessionId);
            if (existing && (cursor < existing.cursor || revision < existing.revision
                || (cursor === existing.cursor && revision === existing.revision))) {
                return { state: publicSkillProjection(existing), ignored: 'stale_skill_projection' };
            }
            const state = {
                sessionId,
                cursor,
                revision,
                catalogRevision,
                order: [...order],
                runs: restoredRuns,
                version: (existing?.version || 0) + 1,
            };
            skillSessions.set(sessionId, state);
            return { state: emitSkills(state, 'skill_checkpoint_applied'), ignored: '' };
        }

        function ingestCoreSkill(eventValue) {
            const event = asObject(eventValue);
            const update = asObject(event.update);
            const sessionId = firstText(event.session_id, event.conversation_id);
            const cursor = numericSequence(event.cursor);
            if (!sessionId || !cursor || !update.type) {
                return { state: null, ignored: 'invalid_core_skill' };
            }
            let state = skillSessions.get(sessionId);
            if (!state) {
                state = {
                    sessionId, revision: 0, cursor: 0, catalogRevision: 0,
                    order: [], runs: new Map(), version: 0,
                };
                skillSessions.set(sessionId, state);
            }
            if (cursor <= state.cursor) {
                return { state: publicSkillProjection(state), ignored: 'stale_core_skill' };
            }
            const runValue = asObject(update.run);
            const runId = firstText(runValue.run_id, update.run_id);
            if (update.type === 'started' && runId) {
                if (!state.runs.has(runId)) state.order.push(runId);
                state.runs.set(runId, normalizedSkillRun(runValue));
            } else if (update.type === 'catalog_replaced') {
                state.catalogRevision = Math.max(state.catalogRevision, Number(update.revision || 0));
            } else if (runId && state.runs.has(runId)) {
                const run = state.runs.get(runId);
                if (update.type === 'capability_refresh_requested') {
                    run.status = 'waiting_for_catalog';
                    run.recoveryErrors = Array.isArray(update.errors) ? [...update.errors] : [];
                } else if (update.type === 'resumed') {
                    run.status = 'running';
                    run.catalogRevision = Math.max(0, Number(update.catalog_revision || 0));
                    run.recoveryErrors = [];
                } else if (update.type === 'usage_updated') {
                    run.usage = { ...asObject(update.usage) };
                } else if (update.type === 'budget_exceeded') {
                    run.status = 'failed';
                    run.failure = `budget_exceeded:${firstText(update.dimension)}`;
                } else if (update.type === 'completed') {
                    run.status = 'completed';
                    run.resultRef = firstText(update.result_ref);
                } else if (update.type === 'failed') {
                    run.status = 'failed';
                    run.failure = firstText(update.error);
                } else if (update.type === 'cancelled') {
                    run.status = 'cancelled';
                }
            }
            state.cursor = cursor;
            state.revision = Math.max(state.revision, Number(event.skill_runtime_revision || 0));
            state.version += 1;
            return { state: emitSkills(state, 'core_skill_applied'), ignored: '' };
        }

        function ingestTaskProjection(projectionValue, identity = {}) {
            const projection = asObject(projectionValue);
            const sessionId = firstText(identity.conversationId, identity.sessionId);
            if (!sessionId) return null;
            const sourceTasks = asObject(projection.tasks);
            const sourceGuidance = asObject(projection.guidance);
            const order = Array.isArray(projection.order) ? projection.order.map(String) : Object.keys(sourceTasks);
            const state = {
                sessionId,
                revision: Math.max(0, Number(projection.revision || 0)),
                order: [],
                tasks: new Map(),
                guidance: new Map(),
                version: 1,
            };
            order.forEach(id => {
                const task = normalizedTask(sourceTasks[id]);
                if (!id || !task.id) return;
                state.order.push(id);
                state.tasks.set(id, task);
                if (sourceGuidance[id]) state.guidance.set(id, normalizedTaskGuidanceQueue(sourceGuidance[id]));
            });
            taskSessions.set(sessionId, state);
            return emitTasks(state, 'task_checkpoint_applied');
        }

        function ingestCoreTask(eventValue) {
            const event = asObject(eventValue);
            const sessionId = firstText(event.session_id, event.conversation_id);
            const task = normalizedTask(event.task);
            const guidance = event.guidance ? normalizedTaskGuidanceQueue(event.guidance) : null;
            const revision = Math.max(0, Number(event.task_projection_revision || 0));
            if (!sessionId || !task.id) return { state: null, ignored: 'invalid_core_task' };
            let state = taskSessions.get(sessionId);
            if (!state) {
                state = { sessionId, revision: 0, order: [], tasks: new Map(), guidance: new Map(), version: 0 };
                taskSessions.set(sessionId, state);
            }
            if (revision && revision <= state.revision) {
                return { state: publicTaskProjection(state), ignored: 'stale_core_task' };
            }
            if (!state.tasks.has(task.id)) state.order.push(task.id);
            state.tasks.set(task.id, task);
            if (guidance) state.guidance.set(task.id, guidance);
            state.revision = Math.max(state.revision, revision);
            state.version += 1;
            return { state: emitTasks(state, 'core_task_applied', task), ignored: '' };
        }

        function ingestCoreCapabilities(eventValue) {
            const event = asObject(eventValue);
            const sessionId = firstText(event.session_id, event.conversation_id);
            const turnId = firstText(event.turn_id);
            const source = firstText(event.source);
            const cursor = numericSequence(event.cursor);
            const capabilityIds = Array.isArray(event.capability_ids)
                ? [...new Set(event.capability_ids.map(String).filter(Boolean))]
                : [];
            if (!sessionId || !turnId || !source || !cursor || !capabilityIds.length) {
                return { state: null, ignored: 'invalid_core_capability_activation' };
            }
            const activation = { sessionId, turnId, source, cursor, capabilityIds };
            stageCapabilityActivation(activation);
            const matchingRuns = Array.from(runs.values()).filter(state =>
                state.conversationId === sessionId && state.turnId === turnId
            );
            if (!matchingRuns.length) {
                return { state: null, applied: [], ignored: 'buffered_capability_activation' };
            }
            let latest = null;
            matchingRuns.forEach(state => {
                applyCapabilityActivation(state, activation);
                latest = publicProjection(state);
            });
            return { state: latest, applied: capabilityIds, ignored: '' };
        }

        function ingestCoreEnvelope(envelopeValue) {
            const envelope = asObject(envelopeValue);
            const event = asObject(envelope.event);
            const sessionId = firstText(envelope.session_id, event.session_id);
            const cursor = numericSequence(asObject(envelope.cursor).sequence);
            if (!sessionId || !cursor || !event.type) {
                return { state: sessionId && coreSessions.has(sessionId) ? publicCoreProjection(coreSessions.get(sessionId)) : null, ignored: 'invalid_core_envelope' };
            }
            let state = coreSessions.get(sessionId);
            if (!state) {
                state = {
                    sessionId, cursor: 0, revision: 0, projectionRevision: 0,
                    order: [], parts: new Map(), latestByPart: new Map(),
                    terminalByTurn: new Map(), version: 0,
                };
                coreSessions.set(sessionId, state);
            }
            if (cursor <= state.cursor) return { state: publicCoreProjection(state), ignored: 'stale_core_envelope' };
            const turnId = firstText(envelope.turn_id, event.turn_id);
            let forwardedResult = null;
            if (event.type === 'part_updated') {
                const part = asObject(event.part);
                const update = asObject(event.update);
                const partId = firstText(update.part_id, part.id);
                if (!partId || state.terminalByTurn.has(turnId)) {
                    return { state: publicCoreProjection(state), ignored: state.terminalByTurn.has(turnId) ? 'terminal_turn' : 'missing_part_id' };
                }
                if (!state.parts.has(partId)) state.order.push(partId);
                state.parts.set(partId, { ...part, id: partId });
                state.projectionRevision = Math.max(state.projectionRevision, Number(event.projection_revision || 0));
                state.latestByPart.set(partId, Number(event.projection_revision || 0));
                ingestCorePart({
                    protocol_version: 2,
                    type: 'core_part_updated',
                    session_id: sessionId,
                    conversation_id: sessionId,
                    turn_id: turnId,
                    run_id: turnId,
                    message_id: firstText(part.message_id, partId),
                    cursor,
                    revision: Number(envelope.revision || 0),
                    projection_revision: Number(event.projection_revision || 0),
                    part_id: partId,
                    update,
                    part,
                });
            } else if (event.type === 'task_updated') {
                ingestCoreTask({
                    type: 'core_task_updated',
                    session_id: sessionId,
                    turn_id: turnId,
                    task_projection_revision: event.task_projection_revision,
                    task: event.task,
                });
            } else if (event.type === 'skill_runtime_updated') {
                ingestCoreSkill({
                    type: 'core_skill_runtime_updated',
                    session_id: sessionId,
                    turn_id: turnId,
                    cursor,
                    skill_runtime_revision: event.skill_runtime_revision,
                    update: event.update,
                });
            } else if (event.type === 'capabilities_activated') {
                forwardedResult = ingestCoreCapabilities({
                    type: 'core_capabilities_activated',
                    session_id: sessionId,
                    turn_id: turnId,
                    cursor,
                    revision: Number(envelope.revision || 0),
                    source: event.source,
                    capability_ids: event.capability_ids,
                });
            } else if (['turn_completed', 'turn_cancelled', 'turn_failed'].includes(event.type) && turnId) {
                state.terminalByTurn.set(turnId, event.type);
                Array.from(runs.values())
                    .filter(run => run.turnId === turnId)
                    .forEach(run => {
                        run.terminal = true;
                        run.terminalKind = event.type;
                        run.status = event.type === 'turn_completed' ? 'succeeded' : event.type === 'turn_cancelled' ? 'cancelled' : 'failed';
                        run.version += 1;
                        emit(run, [], 'core_terminal_applied');
                    });
            }
            state.cursor = cursor;
            state.revision = Math.max(state.revision, Number(envelope.revision || 0));
            state.version += 1;
            return {
                state: publicCoreProjection(state),
                applied: forwardedResult?.applied || [],
                ignored: forwardedResult?.ignored || '',
            };
        }

        function canonicalEntry(event) {
            const source = asObject(asObject(event).trace);
            const sequence = numericSequence(source.sequence);
            if (!source.kind || !source.run_id || !source.event_id || !sequence) return null;
            if (!CONTROL_TRACE_KINDS.has(source.kind) || owns(asObject(source.payload), 'delta')) return null;
            const trace = {
                ...source,
                sequence,
                payload: asObject(source.payload),
            };
            const key = `${trace.run_id}::${firstText(trace.message_id)}`;
            return { key, trace };
        }

        function emit(state, applied, reason = '') {
            const projection = publicProjection(state);
            options.onProjection?.(projection, {
                applied: applied.map(item => ({
                    trace: { ...item.trace, payload: { ...item.trace.payload } },
                })),
                reason,
            });
            return projection;
        }

        function ingestCorePart(event) {
            const part = asObject(event.part);
            const update = asObject(event.update);
            const partId = firstText(event.part_id, update.part_id, part.id);
            if (!partId) return { state: null, applied: [], ignored: 'missing_part_id' };
            const runId = firstText(event.run_id, event.turn_id, 'core-part-run');
            const messageId = firstText(event.message_id, partId);
            const visible = { ...part, id: partId };
            if (part.kind === 'text') visible.text = publicText(part, 'text');
            if (part.kind === 'source') visible.evidence = publicSourceEvidence(part.evidence);
            if (part.kind === 'tool') {
                // Validate before advancing cursors or changing the Part map.
                delete visible.raw_input;
                delete visible.full_output_ref;
                visible.full_output = outputReference(part.full_output ?? (part.full_output_ref == null ? null : {
                    message_id: messageId, part_id: partId, tool_call_id: part.call_id,
                }), messageId, partId, part.call_id);
            }
            const key = `${runId}::${messageId}`;
            let state = runs.get(key);
            if (!state) {
                state = makeRunState({
                    schema_version: Number(event.protocol_version || 2),
                    trace_id: firstText(event.trace_id),
                    run_id: runId,
                    conversation_id: firstText(event.conversation_id, event.session_id),
                    message_id: messageId,
                    turn_id: firstText(event.turn_id),
                });
                registerRun(key, state);
            }
            const eventId = `core-part:${firstText(event.session_id)}:${Number(event.cursor || 0)}:${partId}`;
            if (state.seenEventIds.has(eventId)) {
                return { state: publicProjection(state), applied: [], ignored: 'duplicate_core_part' };
            }
            const cursor = Math.max(0, Number(event.cursor || 0));
            const previousCursor = Number(state.latestPartCursorById.get(partId) || 0);
            if (cursor > 0 && previousCursor > 0 && cursor <= previousCursor) {
                return { state: publicProjection(state), applied: [], ignored: 'stale_core_part' };
            }
            state.seenEventIds.add(eventId);
            if (cursor > 0) state.latestPartCursorById.set(partId, cursor);
            if (!state.parts.has(partId)) state.partOrder.push(partId);
            state.parts.set(partId, visible);
            projectCorePresentation(state);
            // An accepted canonical Part revision always changes the public
            // projection. Avoid serializing the complete accumulated trace
            // before and after every streaming text delta.
            state.version += 1;
            const applied = [{
                trace: {
                    kind: 'core.part.updated',
                    event_id: eventId,
                    payload: { part_id: partId, update: part.kind === 'text'
                        && part.review != null && part.review !== 'approved'
                        ? { type: update.type, part_id: partId } : update, part: visible },
                },
            }];
            return { state: emit(state, applied, 'core_part_applied'), applied, ignored: '' };
        }

        function projectCorePresentation(state) {
            const ordered = state.partOrder.map(id => state.parts.get(id)).filter(Boolean);
            state.assistantText = ordered
                // Core Part snapshots are authoritative while they are open as
                // well as after completion. Hiding open text Parts makes the
                // entire answer appear only when `completed` arrives and turns
                // semantic streaming into a final snapshot swap.
                .filter(part => firstText(part.kind, part.type) === 'text' || (part.kind === 'historical' && !part.historicalReasoning))
                .map(part => part.kind === 'historical' ? part.historicalText : publicText(part, 'text'))
                .join('');
            state.thinkingText = ordered
                .filter(part => firstText(part.kind, part.type) === 'reasoning' || (part.kind === 'historical' && part.historicalReasoning))
                .map(part => part.kind === 'historical' ? part.historicalText : firstText(part.detail))
                .join('');
            const toolParts = ordered.filter(part => firstText(part.kind, part.type) === 'tool');
            state.toolOrder = toolParts.map(part => firstText(part.call_id, part.id));
            state.tools = new Map(toolParts.map(part => {
                const callId = firstText(part.call_id, part.id);
                const status = part.state === 'completed'
                    ? 'succeeded'
                    : part.state === 'waiting_for_input'
                        ? 'waiting_for_input'
                    : part.state === 'waiting_for_permission'
                        ? 'waiting'
                        : ['failed', 'cancelled'].includes(part.state)
                            ? part.state
                            : 'running';
                const artifacts = Array.isArray(part.artifacts) ? part.artifacts : [];
                const hasStructuredResult = owns(part, 'structured_output');
                const preview = typeof part.output_preview === 'string' ? part.output_preview : null;
                return [callId, {
                    callId,
                    partId: firstText(part.id),
                    name: firstText(part.capability_id, part.title, 'tool'),
                    title: typeof part.title === 'string' ? part.title : '',
                    origin: part.origin ?? null,
                    arguments: owns(part, 'input') ? part.input : null,
                    hasStructuredResult,
                    ...(hasStructuredResult ? { structuredResult: part.structured_output } : {}),
                    outputPreview: preview,
                    result: hasStructuredResult ? part.structured_output : preview,
                    resultText: preview !== null ? preview
                        : hasStructuredResult ? toolResultText(part.structured_output) : '',
                    artifacts,
                    resources: artifacts.map(artifact => ({
                        uri: firstText(artifact.uri),
                        operation: firstText(artifact.operation),
                        display_name: firstText(artifact.name),
                        mime_type: firstText(artifact.mime_type),
                    })),
                    status,
                    summary: typeof part.summary === 'string' ? part.summary : null,
                    detail: firstText(part.output_preview, asObject(part.permission).rationale),
                    permission: asObject(part.permission),
                    fullOutput: outputReference(part.full_output, state.messageId, part.id, callId),
                    startedAtMs: part.started_at_ms ?? null,
                    finishedAtMs: part.finished_at_ms ?? null,
                    capabilityIds: [],
                    version: state.version + 1,
                }];
            }));
        }

        function normalizedConversationPart(partValue, message) {
            const part = asObject(partValue);
            const kind = firstText(part.kind);
            const partId = firstText(part.part_id);
            if (!kind || !partId) return null;
            const common = {
                ...part,
                id: partId,
                message_id: firstText(message.message_id),
            };
            if (kind === 'historical') {
                const projection = asObject(part.projection);
                const status = asObject(projection.status);
                if (!['supported', 'deferred', 'unsupported', 'not_loaded'].includes(status.state)) {
                    throw new Error('Unknown historical projection status');
                }
                const fields = asObject(projection.fields);
                for (const field of Object.values(fields)) {
                    if (!['complete', 'deferred'].includes(asObject(field).availability)) {
                        throw new Error('Unknown historical field availability');
                    }
                }
                const originalKind = projection.canonical_kind;
                const fieldName = {
                    'agent_part:text': 'text', 'message_part_view:text': 'markdown',
                    'agent_part:reasoning': 'detail', 'message_part_view:reasoning': 'summary',
                }[`${projection.schema}:${originalKind}`];
                const names = fieldName ? [fieldName] : [];
                let decoded = null;
                if (status.state !== 'unsupported' && status.state !== 'not_loaded') {
                    for (const name of names) {
                        const field = asObject(fields[name]);
                        if (field.availability === 'complete' && typeof field.value === 'string') {
                            decoded = field.value;
                            break;
                        }
                    }
                }
                // Keep the historical kind and complete original wire projection.
                // No call_id, live lifecycle, permission, or executable input mapping.
                return { ...common,
                    historicalReasoning: originalKind === 'reasoning',
                    // Keep protocol diagnostics on the explicit details model;
                    // ordinary chat content is only original text or its
                    // canonical bounded preview.
                    historicalText: decoded ?? (typeof projection.canonical_text?.text === 'string' ? projection.canonical_text.text : ''),
                    is_complete: true,
                };
            }
            if (kind === 'text') {
                return {
                    ...common,
                    text: publicText(part, 'markdown'),
                    markdown: publicText(part, 'markdown'),
                    is_complete: Boolean(part.is_final),
                };
            }
            if (kind === 'reasoning') {
                return {
                    ...common,
                    detail: firstText(part.summary),
                    is_complete: ['completed', 'cancelled', 'failed'].includes(firstText(part.state)),
                };
            }
            if (kind === 'compaction') {
                return {
                    ...common,
                    source_item_ids: Array.isArray(part.source_item_ids)
                        ? [...part.source_item_ids]
                        : (Array.isArray(part.retained_item_ids) ? [...part.retained_item_ids] : []),
                };
            }
            if (kind === 'tool') {
                const normalized = {
                    ...common,
                    call_id: firstText(part.tool_call_id),
                    full_output: outputReference(part.full_output, common.message_id,
                        partId, part.tool_call_id),
                };
                if (owns(part, 'output')) normalized.structured_output = part.output;
                return normalized;
            }
            if (kind === 'citation') {
                return {
                    ...common,
                    // The citation identity and optional location are source
                    // facts, not the Part id or a manufactured empty object.
                    range: part.range_start == null && part.range_end == null ? null
                        : { start: part.range_start, end: part.range_end },
                };
            }
            if (kind === 'source') return { ...common, evidence: publicSourceEvidence(part.evidence) };
            return common;
        }

        function ingestProjection(projection, identity = {}) {
            const snapshot = asObject(projection);
            const touched = new Map();
            // Validate owner-bound descriptors before mutating any existing
            // state; a malformed later Part must not partially replace history.
            const messages = (Array.isArray(snapshot.messages) ? snapshot.messages : [])
                .map(asObject).filter(message => message.role === 'assistant')
                .map(message => ({ message, parts: (Array.isArray(message.parts) ? message.parts : [])
                    .map(part => normalizedConversationPart(part, message)).filter(Boolean) }));
            const surface = asObject(snapshot.surface);
            const committedMessageIds = new Set(
                Array.isArray(surface.committed_message_ids)
                    ? surface.committed_message_ids.map(String)
                    : []
            );
            const archivedActivityPartIds = Array.isArray(surface.archived_activity_part_ids)
                ? surface.archived_activity_part_ids.map(String)
                : [];
            const liveMessageId = firstText(surface.live_message_id);
            const currentActivityPartId = firstText(surface.current_activity_part_id);
            messages.forEach(({ message, parts }) => {
                const messageId = firstText(message.message_id);
                if (!messageId || firstText(message.role) !== 'assistant') return;
                const existingLiveState = Array.from(runs.values()).find(candidate =>
                    candidate.messageId === messageId
                    && candidate.conversationId === firstText(identity.conversationId)
                    && candidate.traceId
                );
                const runId = firstText(identity.runId, existingLiveState?.runId, `history:${messageId}`);
                const key = `${runId}::${messageId}`;
                let state = existingLiveState || runs.get(key);
                if (!state) {
                    state = makeRunState({
                        schema_version: Number(identity.schemaVersion || 2),
                        trace_id: firstText(identity.traceId),
                        run_id: runId,
                        conversation_id: firstText(identity.conversationId),
                        message_id: messageId,
                        turn_id: '',
                    });
                    registerRun(key, state);
                }
                // Canonical snapshots authoritatively replace this message's
                // loaded Parts; absence here is not a whole-history deletion.
                state.parts.clear();
                state.partOrder = [];
                parts.forEach(part => {
                    if (!state.parts.has(part.id)) state.partOrder.push(part.id);
                    state.parts.set(part.id, part);
                });
                const messageState = firstText(message.state);
                state.surface = {
                    revision: Math.max(0, Number(surface.revision || snapshot.revision || 0)),
                    isCommitted: committedMessageIds.has(messageId),
                    isLive: liveMessageId === messageId,
                    currentActivityPartId,
                    archivedActivityPartIds: [...archivedActivityPartIds],
                    showAssistantPlaceholder: liveMessageId === messageId
                        && Boolean(surface.show_assistant_placeholder),
                };
                state.terminal = ['completed', 'cancelled', 'failed'].includes(messageState);
                state.status = messageState === 'completed' ? 'succeeded' : messageState;
                state.terminalKind = state.terminal ? `run.${messageState}` : '';
                touched.set(key, state);
            });
            return Array.from(touched.values()).map(state => {
                projectCorePresentation(state);
                state.version += 1;
                return emit(state, [], 'history_projection_applied');
            });
        }

        function ingest(event) {
            const eventObject = asObject(event);
            if (eventObject.event && asObject(eventObject.cursor).sequence) {
                return ingestCoreEnvelope(eventObject);
            }
            if (eventObject.type === 'core_task_updated') {
                return ingestCoreTask(eventObject);
            }
            if (eventObject.type === 'core_skill_runtime_updated') {
                return ingestCoreSkill(eventObject);
            }
            if (eventObject.type === 'core_capabilities_activated') {
                return ingestCoreCapabilities(eventObject);
            }
            if (eventObject.type === 'core_part_updated') {
                return ingestCorePart(eventObject);
            }
            const entry = canonicalEntry(event);
            if (!entry) {
                return { state: null, applied: [], ignored: 'non_canonical_event' };
            }
            let state = runs.get(entry.key);
            if (!state) {
                state = makeRunState(entry.trace);
                registerRun(entry.key, state);
            } else {
                state.traceId = state.traceId || firstText(entry.trace.trace_id);
                state.conversationId = state.conversationId || firstText(entry.trace.conversation_id);
                state.turnId = state.turnId || firstText(entry.trace.turn_id);
            }
            const eventId = firstText(entry.trace.event_id);
            if (eventId && state.seenEventIds.has(eventId)) {
                return { state: publicProjection(state), applied: [], ignored: 'duplicate_event_id' };
            }
            if (entry.trace.sequence < state.nextSequence) {
                state.diagnostics.push({
                    type: 'out_of_order',
                    sequence: entry.trace.sequence,
                    expected: state.nextSequence,
                    eventId,
                });
                options.onDiagnostic?.(state.diagnostics[state.diagnostics.length - 1]);
                return { state: emit(state, [], 'out_of_order'), applied: [], ignored: 'out_of_order' };
            }
            if (entry.trace.sequence > state.nextSequence) {
                if (options.allowSequenceGaps === true) {
                    const diagnostic = {
                        type: 'filtered_sequence_gap',
                        sequence: entry.trace.sequence,
                        expected: state.nextSequence,
                        eventId,
                    };
                    state.diagnostics.push(diagnostic);
                    options.onDiagnostic?.(diagnostic);
                    state.nextSequence = entry.trace.sequence;
                } else {
                if (!state.pendingBySequence.has(entry.trace.sequence)) {
                    state.pendingBySequence.set(entry.trace.sequence, entry);
                    if (eventId) state.seenEventIds.add(eventId);
                }
                const diagnostic = {
                    type: 'sequence_gap',
                    sequence: entry.trace.sequence,
                    expected: state.nextSequence,
                    eventId,
                };
                state.diagnostics.push(diagnostic);
                options.onDiagnostic?.(diagnostic);
                return {
                    state: publicProjection(state),
                    applied: [],
                    ignored: 'buffered_gap',
                };
                }
            }

            const applied = [];
            const previousVersion = state.version;
            let current = entry;
            while (current) {
                const currentId = firstText(current.trace.event_id);
                if (currentId) state.seenEventIds.add(currentId);
                applied.push(applyEvent(state, current));
                current = state.pendingBySequence.get(state.nextSequence) || null;
                if (current) state.pendingBySequence.delete(state.nextSequence);
            }
            const projection = state.version === previousVersion
                ? publicProjection(state)
                : emit(state, applied, 'applied');
            return { state: projection, applied, ignored: '' };
        }

        return {
            ingest,
            ingestCoreEnvelope,
            ingestProjection,
            ingestTaskProjection,
            ingestSkillProjection,
            coreProjection(sessionId) {
                const state = coreSessions.get(sessionId);
                return state ? publicCoreProjection(state) : null;
            },
            taskProjection(sessionId) {
                const state = taskSessions.get(sessionId);
                return state ? publicTaskProjection(state) : null;
            },
            skillProjection(sessionId) {
                const state = skillSessions.get(sessionId);
                return state ? publicSkillProjection(state) : null;
            },
            projection(runId, messageId = '') {
                const state = runs.get(`${runId}::${messageId}`);
                return state ? publicProjection(state) : null;
            },
            projections() {
                return Array.from(runs.values(), publicProjection);
            },
            clear() {
                runs.clear();
                coreSessions.clear();
                taskSessions.clear();
                skillSessions.clear();
                pendingCapabilityActivations.clear();
            },
        };
    }

    function sourceItemMessageIndex(projection) {
        const index = Object.create(null);
        const messages = Array.isArray(projection?.messages) ? projection.messages : [];
        messages.forEach(messageValue => {
            const message = asObject(messageValue);
            const messageId = firstText(message.message_id);
            if (!messageId) return;
            index[messageId] = messageId;
            const parts = Array.isArray(message.parts) ? message.parts : [];
            parts.forEach(partValue => {
                const partId = firstText(asObject(partValue).part_id);
                if (partId) index[partId] = messageId;
            });
        });
        return index;
    }

    global.NolaAgentTrace = Object.freeze({
        createTraceStore,
        sourceItemMessageIndex,
    });
})(window);
