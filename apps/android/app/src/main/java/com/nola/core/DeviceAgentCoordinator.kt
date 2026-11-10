@file:OptIn(kotlinx.serialization.ExperimentalSerializationApi::class)

package com.nola.core

import android.util.Log
import android.os.SystemClock
import com.nola.agent.trace.AgentTraceEvent
import com.nola.llm.LlmRequestException
import java.io.IOException
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.selects.onTimeout
import kotlinx.coroutines.selects.select
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.Json
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

internal fun deviceAgentCancellationSubmission(reason: String): JsonObject = buildJsonObject {
    put("type", "interrupt")
    put("cancellation_id", UUID.randomUUID().toString())
    put("reason", reason)
}

/** A cold lease wait must not hide the user's stop behind that lease. */
@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
internal suspend fun awaitRecoveryCancellation(cancellations: Channel<String>, waitMs: Long): String? {
    cancellations.tryReceive().getOrNull()?.let { return it }
    if (waitMs <= 0) return null
    return select {
        cancellations.onReceive { it }
        onTimeout(waitMs) { null }
    }
}

private const val AGENT_PROTOCOL_LOG_TAG = "NolaAgentProtocol"
private const val EFFECT_HEARTBEAT_INTERVAL_MILLISECONDS = 2 * 60 * 1_000L

sealed interface DeviceAgentCoordinatorEvent {
    /** Emitted only after the canonical Begin/recovery transaction is durable. */
    data class Admitted(val conversationId: String, val submissionId: String) : DeviceAgentCoordinatorEvent
    data class CancellationRequested(val effectId: String, val externalOutcomeUnknown: Boolean) : DeviceAgentCoordinatorEvent
    data class JournalPublications(val cursors: List<JsonObject>) : DeviceAgentCoordinatorEvent
    data class Snapshot(val snapshot: AgentCoreSnapshot) : DeviceAgentCoordinatorEvent
    data class TaskProjectionRestored(
        val projection: AgentCoreTaskProjection,
    ) : DeviceAgentCoordinatorEvent
    data class TaskLifecycle(val update: AgentCoreTaskUpdate) : DeviceAgentCoordinatorEvent
    data class SkillProjectionUpdated(
        val projection: AgentCoreSkillRuntimeProjection,
    ) : DeviceAgentCoordinatorEvent
    data class CapabilitiesActivated(
        val activation: AgentCoreCapabilityActivationEnvelope,
    ) : DeviceAgentCoordinatorEvent
    data class Trace(val event: AgentTraceEvent) : DeviceAgentCoordinatorEvent
    data class PermissionRequested(
        val request: AndroidAgentPermissionRequest,
    ) : DeviceAgentCoordinatorEvent
    data class Completed(val seal: AndroidCanonicalRunTerminalSeal) : DeviceAgentCoordinatorEvent
}

enum class DeviceAgentInputOrigin {
    INTERACTIVE_USER,
    SCHEDULED_ACTION,
}

internal fun encodeDeviceAgentInputSubmission(
    input: String,
    inputOrigin: DeviceAgentInputOrigin = DeviceAgentInputOrigin.INTERACTIVE_USER,
    turnContextComponents: List<AgentCoreContextComponent> = emptyList(),
    submissionId: String = UUID.randomUUID().toString(),
    messageId: String = UUID.randomUUID().toString(),
    submittedAtEpochMs: Long = System.currentTimeMillis(),
): JsonObject = buildJsonObject {
    when (inputOrigin) {
        DeviceAgentInputOrigin.INTERACTIVE_USER -> {
            put("type", "user_input")
            put("delivery", "current_turn")
        }
        DeviceAgentInputOrigin.SCHEDULED_ACTION -> {
            put("type", "automated_input")
            put("origin", "scheduled_action")
        }
    }
    put("submission_id", submissionId)
    put("submitted_at_epoch_ms", submittedAtEpochMs)
    put("turn_context_items", JsonArray(emptyList()))
    put("turn_context_platform", "android")
    put("turn_context_components", JsonArray(turnContextComponents.map { component ->
        Json.encodeToJsonElement(AgentCoreContextComponent.serializer(), component)
    }))
    put(
        "items",
        buildJsonArray {
            add(
                buildJsonObject {
                    put("type", "message")
                    put("id", messageId)
                    put("role", "user")
                    put("content", input)
                },
            )
        },
    )
}

/**
 * Android Host loop for Agent Core v2.
 *
 * Core is the only owner of session/Turn/tool/context state. This coordinator
 * performs model, permission, persistence, and device I/O, then returns every
 * result as a normalized Host response before exposing Core events to the UI.
 */
class DeviceAgentCoordinator(
    private val modelClient: AgentModelClient,
    private val permissionBroker: AndroidAgentPermissionBroker,
    private val hostExecutor: AndroidAgentHostExecutor,
    private val canonicalApplication: AndroidCanonicalAgentRuntime,
    private val modelCapability: AndroidModelCapabilityProfile,
    private val refreshToolPlan: (() -> AgentCoreAndroidToolPlan)? = null,
    private val childToolPlan: ((String) -> AgentCoreAndroidToolPlan)? = null,
    private val embeddedMailbox: Boolean = false,
    private val captureDelegation: Boolean = false,
    private val prepareRecovery: (suspend (JsonObject, JsonObject) -> AgentCoreAndroidToolPlan)? = null,
) {
    private val modelSupportsToolCalls: Boolean get() = modelCapability.nativeToolCalls
    private val runMutex = Mutex()
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
        encodeDefaults = false
    }

    @Volatile
    private var permissionChannel: Channel<AndroidAgentPermissionDecision>? = null
    @Volatile
    private var cancellationChannel: Channel<String>? = null
    @Volatile
    private var taskCancellationChannel: Channel<String>? = null
    @Volatile
    private var taskGuidanceChannel: Channel<TaskGuidanceRequest>? = null
    @Volatile private var policyChanges: Channel<Unit>? = null

    fun refreshPermissionPolicy() { policyChanges?.trySend(Unit) }

    suspend fun resolvePermission(decision: AndroidAgentPermissionDecision) {
        permissionChannel?.send(decision)
            ?: error("The device agent is not waiting for permission")
    }

    @Volatile private var requestedCancellation: String? = null
    @Synchronized fun requestCancellation(reason: String = "user_requested"): Boolean {
        if (requestedCancellation != null) return true
        requestedCancellation = reason
        cancellationChannel?.trySend(reason)
        return true
    }

    fun requestTaskCancellation(taskId: String): Boolean {
        if (taskId.isBlank()) return false
        return taskCancellationChannel?.trySend(taskId)?.isSuccess == true
    }

    fun sendTaskGuidance(taskId: String, content: String): Boolean {
        val request = TaskGuidanceRequest(taskId.trim(), content.trim())
        if (request.taskId.isEmpty() || request.content.isEmpty()) return false
        return taskGuidanceChannel?.trySend(request)?.isSuccess == true
    }

    fun run(
        sessionId: String?,
        input: String,
        history: List<AgentCoreMessage>,
        tools: List<JsonElement>,
        toolPlan: AgentCoreAndroidToolPlan,
        config: AgentCoreRuntimeConfig,
        inputOrigin: DeviceAgentInputOrigin = DeviceAgentInputOrigin.INTERACTIVE_USER,
        turnContextComponents: List<AgentCoreContextComponent> = emptyList(),
        recoverExistingRun: Boolean = false,
    ): Flow<DeviceAgentCoordinatorEvent> = channelFlow {
        val executionMode = AndroidAgentExecutionMode.fromWireValue(config.executionMode)
        require(executionMode?.supportsDeviceExecution == true) {
            "DeviceAgentCoordinator requires a device-capable mode"
        }
        require(tools == (if (modelSupportsToolCalls) toolPlan.tools else emptyList())) {
            "Tool schemas do not match the active Core tool plan"
        }
        require(toolPlan.receiptIsValid()) {
            "The active Android tool plan is expired or invalid"
        }

        runMutex.withLock {
            val decisions = Channel<AndroidAgentPermissionDecision>(Channel.RENDEZVOUS)
            val cancellations = Channel<String>(capacity = 1)
            val taskCancellations = Channel<String>(capacity = Channel.UNLIMITED)
            val taskGuidance = Channel<TaskGuidanceRequest>(capacity = Channel.UNLIMITED)
            val policyNotifications = Channel<Unit>(capacity = Channel.CONFLATED)
            policyChanges = policyNotifications
            permissionChannel = decisions
            cancellationChannel = cancellations
            requestedCancellation?.let { cancellations.trySend(it) }
            taskCancellationChannel = taskCancellations
            taskGuidanceChannel = taskGuidance
            val stableSessionId = sessionId?.trim().takeUnless { it.isNullOrEmpty() }
                ?: UUID.randomUUID().toString()
            val runtimeConfig = buildRuntimeConfig(
                sessionId = stableSessionId,
                history = history,
                tools = tools,
                toolPlan = toolPlan,
                config = config,
            )
            val admittedConfig = if (captureDelegation && childToolPlan != null && config.preparedHostInvocation != null) {
                canonicalApplication.mobileCommand(buildJsonObject {
                    put("type", "capture_mobile_delegation_configuration"); put("config",runtimeConfig)
                    put("settings",buildJsonObject { put("project_root", "android-device"); put("selected_profile",JsonNull); put("agents",buildJsonObject {}) })
                }).jsonObject
            } else runtimeConfig
            canonicalApplication.configure(admittedConfig)
            val runtime: AgentCoreTransitionRuntime = canonicalApplication
            val state = RunState(
                sessionId = stableSessionId,
                executionMode = config.executionMode,
                hostId = runtimeConfig.getValue("host_catalog").jsonObject.getValue("host_id").jsonPrimitive.content,
            )

            try {
                var activeToolPlan = toolPlan
                var recoveryCancellation: String? = null
                var transition: JsonObject? = if (recoverExistingRun) {
                    val recovery = canonicalApplication.recoverExistingRun(awaitCancellation = { waitMs ->
                        val reason = awaitRecoveryCancellation(cancellations, waitMs)
                        if (reason != null) recoveryCancellation = reason
                        reason != null
                    }) { frozen ->
                        activeToolPlan = checkNotNull(prepareRecovery) { "Recovery requires frozen model reconstruction" }(frozen, runtimeConfig)
                    }
                    val recovered = when (recovery) {
                        is AndroidRunRecoveryOutcome.Active -> recovery.projection
                        is AndroidRunRecoveryOutcome.Sealed -> {
                            send(DeviceAgentCoordinatorEvent.Admitted(canonicalApplication.descriptor.conversationId,
                                canonicalApplication.descriptor.submissionId))
                            send(DeviceAgentCoordinatorEvent.Completed(recovery.seal))
                            return@channelFlow
                        }
                    }
                    state.hostJournal.restore(recovered)
                    val recoveredControl = checkNotNull(canonicalControlProjection(recovered))
                    state.taskProjection = checkNotNull(decodeAgentCoreTaskProjection(recovered["task_projection"], stableSessionId)).copy(
                        lastCursor = checkNotNull(recoveredControl.objectValue("last_event_cursor")?.long("sequence")),
                        lastRevision = checkNotNull(recoveredControl.long("revision")),
                    )
                    send(DeviceAgentCoordinatorEvent.TaskProjectionRestored(state.taskProjection))
                    send(DeviceAgentCoordinatorEvent.Snapshot(recoveredControl.toClientSnapshot(state)))
                    null
                } else {
                    runtime.submit(
                        encodeDeviceAgentInputSubmission(
                            input = input,
                            inputOrigin = inputOrigin,
                            turnContextComponents = turnContextComponents,
                            submissionId = canonicalApplication.descriptor.submissionId,
                            messageId = canonicalApplication.descriptor.userMessageId,
                        ),
                    ).agentCoreTransitionOrThrow()
                }

                send(DeviceAgentCoordinatorEvent.Admitted(canonicalApplication.descriptor.conversationId,
                    canonicalApplication.descriptor.submissionId))
                while (true) {
                    transition?.let {
                        acceptTransition(it, activeToolPlan, state) { event -> send(event) }
                        transition = null
                    }
                    canonicalApplication.applyPendingPermissionPolicy()?.let {
                        transition = it.agentCoreTransitionOrThrow()
                    }
                    if (transition != null) continue

                    val cancellationReason = recoveryCancellation ?: cancellations.tryReceive().getOrNull()
                    recoveryCancellation = null
                    if (cancellationReason != null) {
                        transition = runtime.submit(
                            deviceAgentCancellationSubmission(cancellationReason),
                        ).agentCoreTransitionOrThrow()
                        state.interruptionCommitted = true
                        continue
                    }

                    val taskId = taskCancellations.tryReceive().getOrNull()
                    if (taskId != null) {
                        transition = runtime.submit(
                            agentCoreTaskCancellationSubmission(taskId),
                        ).agentCoreTransitionOrThrow()
                        continue
                    }

                    val guidance = taskGuidance.tryReceive().getOrNull()
                    if (guidance != null) {
                        val task = state.taskProjection.tasks[guidance.taskId]
                            ?: error("Task guidance references an unknown Task")
                        require(task.kind == AgentCoreTaskKind.Agent && task.state.isActive && !task.cancelRequested) {
                            "Only an active Agent Task accepts guidance"
                        }
                        transition = runtime.submit(
                            agentCoreTaskGuidanceSubmission(
                                task = task,
                                content = guidance.content,
                                messageId = UUID.randomUUID().toString(),
                                enqueuedAtEpochMs = System.currentTimeMillis(),
                            ),
                        ).agentCoreTransitionOrThrow()
                        continue
                    }

                    val request = state.hostJournal.poll()
                    if (request != null) {
                        activeToolPlan = androidToolPlanForHostRequest(
                            request.objectValue("payload")?.string("type"), activeToolPlan, refreshToolPlan)
                        val execute: suspend () -> JsonObject = { handleHostRequest(
                            runtime = runtime,
                            request = request,
                            config = config,
                            toolPlan = activeToolPlan,
                            decisions = decisions,
                            cancellations = cancellations,
                            state = state,
                            emitEvent = { send(it) },
                        ) }
                        val type = request.objectValue("payload")?.string("type")
                        state.cancellationEvidence.dispatching(request)
                        transition = if (type in setOf("model_stream", "execute_tool", "compact")) {
                            awaitOwnedHostEffect(cancellations, interrupt = { reason ->
                                val effectId = checkNotNull(request.string("effect_id"))
                                if (type == "execute_tool") state.cancellationEvidence.outcomeUnknown(request)
                                val interrupted = runtime.submit(deviceAgentCancellationSubmission(reason)).agentCoreTransitionOrThrow()
                                state.interruptionCommitted = true
                                send(DeviceAgentCoordinatorEvent.CancellationRequested(effectId, type == "execute_tool"))
                                interrupted
                            }, operation = execute)
                        } else execute()
                        // awaitOwnedHostEffect's coroutineScope has joined the
                        // actual model/compaction operation before this point.
                        // This closes only local I/O, never remote effects.
                        if (type in setOf("model_stream", "compact", "request_permission",
                                "resolve_permission_resources", "fetch_context", "run_lifecycle_hook")) {
                            state.cancellationEvidence.returnedLocally(request,
                                canonicalApplication.descriptor.conversationId)
                        }
                        continue
                    }

                    val seal = canonicalApplication.committedTerminalSeal(state.sessionId)
                    checkNotNull(seal) {
                        "Agent Core has no pending Host effect or exact committed terminal seal"
                    }
                    Log.d(AGENT_PROTOCOL_LOG_TAG, "run_sealed session=${state.sessionId}")
                    state.hostJournal.finishTurn()
                    // End transport while retaining the business result, including
                    // incomplete/blocked/cancelled. Sealing is not task success.
                    send(DeviceAgentCoordinatorEvent.Completed(seal))
                    break
                }
            } finally {
                permissionChannel = null
                cancellationChannel = null
                taskCancellationChannel = null
                taskGuidanceChannel = null
                policyChanges = null
                decisions.close()
                cancellations.close()
                taskCancellations.close()
                taskGuidance.close()
                policyNotifications.close()
            }
        }
    }.flowOn(Dispatchers.IO) // JNI Store replay and reduction must never block UI input.

    private data class TaskGuidanceRequest(val taskId: String, val content: String)

    private suspend fun acceptTransition(
        transition: JsonObject,
        toolPlan: AgentCoreAndroidToolPlan,
        state: RunState,
        emitEvent: suspend (DeviceAgentCoordinatorEvent) -> Unit,
    ) {
        val snapshot = canonicalControlProjection(transition)?.toClientSnapshot(state)
        if (snapshot != null && snapshot != state.lastSnapshot) {
            require(snapshot.sessionId == state.sessionId) { "Control belongs to another Core session" }
            state.lastSnapshot = snapshot
            // Establish control ownership before emitting Task/Skill deltas.
            emitEvent(DeviceAgentCoordinatorEvent.Snapshot(snapshot))
        }
        val publications = transition.array("assistant_journal_publications").map { it.jsonObject }
        if (publications.isNotEmpty()) emitEvent(DeviceAgentCoordinatorEvent.JournalPublications(publications))
        transition.array("event_envelopes").forEach { element ->
            val envelope = element as? JsonObject ?: return@forEach
            decodeAgentCoreTaskUpdate(envelope)?.let { update ->
                val reduction = AgentCoreTaskProjectionReducer.reduce(state.taskProjection, update)
                if (reduction.applied) {
                    state.taskProjection = reduction.projection
                    emitEvent(DeviceAgentCoordinatorEvent.TaskLifecycle(update))
                }
            }
            decodeAgentCoreSkillRuntimeEnvelope(envelope)?.let { update ->
                val reduction = AgentCoreSkillRuntimeProjectionReducer.reduce(
                    current = state.skillProjection,
                    envelope = update,
                    expectedSessionId = state.sessionId,
                )
                if (reduction.applied) {
                    state.skillProjection = reduction.projection
                    emitEvent(DeviceAgentCoordinatorEvent.SkillProjectionUpdated(reduction.projection))
                }
            }
            decodeAgentCoreCapabilityActivationEnvelope(envelope)?.let { activation ->
                if (
                    activation.sessionId == state.sessionId &&
                    activation.cursor > state.capabilityActivationCursor &&
                    activation.journalRevision >= state.capabilityActivationRevision
                ) {
                    state.capabilityActivationCursor = activation.cursor
                    state.capabilityActivationRevision = activation.journalRevision
                    emitEvent(DeviceAgentCoordinatorEvent.CapabilitiesActivated(activation))
                }
            }
        }

        transition.array("trace_events").forEach { traceElement ->
            val trace = json.decodeFromJsonElement<AgentTraceEvent>(traceElement)
            // Diagnostic terminal traces never authorize completion. The loop
            // queries the immutable source seal after committed work settles.
            emitEvent(DeviceAgentCoordinatorEvent.Trace(trace))
        }

        transition.array("host_requests").forEach { element ->
            (element as? JsonObject)?.let(state.hostJournal::accept)
        }

    }

    internal suspend fun executeEmbeddedRequest(request:JsonObject,runtime:AgentCoreTransitionRuntime,config:AgentCoreRuntimeConfig,toolPlan:AgentCoreAndroidToolPlan,state:RunState,parentConversationId:String) {
        check(embeddedMailbox)
        state.hostJournal.accept(request)
        val currentToolPlan = androidToolPlanForHostRequest(
            request.objectValue("payload")?.string("type"), toolPlan, refreshToolPlan)
        val decisions=Channel<AndroidAgentPermissionDecision>(Channel.RENDEZVOUS)
        val cancellations=Channel<String>(Channel.RENDEZVOUS)
        try {
            kotlinx.coroutines.coroutineScope {
                handleHostRequest(runtime,request,config,currentToolPlan,decisions,cancellations,state) { event ->
                    if (event is DeviceAgentCoordinatorEvent.PermissionRequested) {
                        launch {
                            try {
                                decisions.send(AndroidChildPermissions.await(parentConversationId,
                                    canonicalApplication.descriptor.conversationId, event.request))
                            } finally { permissionBroker.clearPermissionRequest(event.request.requestId) }
                        }
                    }
                }
            }
        } finally { decisions.close();cancellations.close() }
    }

    private suspend fun handleHostRequest(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        config: AgentCoreRuntimeConfig,
        toolPlan: AgentCoreAndroidToolPlan,
        decisions: Channel<AndroidAgentPermissionDecision>,
        cancellations: Channel<String>,
        state: RunState,
        emitEvent: suspend (DeviceAgentCoordinatorEvent) -> Unit,
    ): JsonObject {
        val payload = request.objectValue("payload")
            ?: error("Host request has no payload")
        val requestType = payload.string("type")
            ?: error("Host request has no type")
        val supportedRequestTypes = setOf(
            "model_stream",
            "execute_tool",
            "request_permission",
            "resolve_permission_resources",
            "compact",
            "fetch_context",
            "cancel",
            "cancel_task",
            "run_lifecycle_hook",
        )
        require(requestType in supportedRequestTypes) {
            "Android Host cannot execute $requestType"
        }
        if (requestType == "cancel") {
            // Reject cold absence, malformed proof and unknown tool outcomes
            // before publishing ExecutionStarted for the control itself.
            try {
                state.cancellationEvidence.requireCancellationProof(request,
                    canonicalApplication.descriptor.conversationId)
            } catch (uncertain: HostEffectCancellationUncertain) {
                val target = canonicalApplication.observeTerminatedCancellationTarget(request)
                state.cancellationEvidence.ownerTerminationProven(target, canonicalApplication.descriptor.conversationId)
                state.cancellationEvidence.requireCancellationProof(request,
                    canonicalApplication.descriptor.conversationId)
            }
        }
        val started = submitHostResponse(
            runtime,
            request,
            buildJsonObject { put("type", "execution_started") },
            state,
        )
        acceptTransition(started, toolPlan, state, emitEvent)
        return withLeaseHeartbeat(request, state) {
            when (requestType) {
            "model_stream" -> runModel(
                runtime = runtime,
                request = request,
                payload = payload,
                config = config,
                toolPlan = toolPlan,
                state = state,
                emitEvent = emitEvent,
            )
            "execute_tool" -> executeTool(runtime, request, payload, toolPlan, state, config)
            "resolve_permission_resources" -> submitHostResponse(runtime, request, buildJsonObject {
                put("type", "failed")
                put("error", buildJsonObject {
                    put("code", "permission_resource_resolution_unsupported")
                    put("message", "This Android Host cannot resolve the declared resource plan. The operation was not dispatched.")
                    put("retryable", false)
                })
            }, state)
            "request_permission" -> resolveHostPermission(
                runtime = runtime,
                request = request,
                payload = payload,
                toolPlan = toolPlan,
                decisions = decisions,
                cancellations = cancellations,
                state = state,
                emitEvent = emitEvent,
            )
            "compact" -> compactContext(
                runtime,
                request,
                payload,
                config,
                toolPlan,
                state,
                emitEvent,
            )
            "fetch_context" -> submitHostResponse(
                runtime,
                request,
                buildJsonObject {
                    put("type", "context_fetched")
                    put("components", JsonArray(emptyList()))
                },
                state,
            )
            "cancel" -> submitHostResponse(
                runtime,
                request,
                buildJsonObject {
                    put("type", "cancellation_confirmed")
                    put(
                        "target_effect_id",
                        payload.string("target_effect_id")
                            ?: error("Cancel Host request has no target effect ID"),
                    )
                },
                state,
            )
            "cancel_task" -> submitHostResponse(
                runtime, request,
                canonicalApplication.taskCancellationResponse(request, state.hostJournal.responseDelivery(request)),
                state,
            )
            "run_lifecycle_hook" -> submitHostResponse(
                runtime,
                request,
                buildJsonObject {
                    put("type", "completed")
                    put("output", buildJsonObject { put("lifecycle", "session_end") })
                    put("full_output_ref", JsonNull)
                    put("artifacts", JsonArray(emptyList()))
                },
                state,
            )
                else -> error("Validated Android Host request type was not dispatched")
            }
        }
    }

    private suspend fun <T> withLeaseHeartbeat(
        request: JsonObject,
        state: RunState,
        operation: suspend () -> T,
    ): T = coroutineScope {
        if (embeddedMailbox) return@coroutineScope operation()
        val heartbeat = launch {
            while (isActive) {
                delay(EFFECT_HEARTBEAT_INTERVAL_MILLISECONDS)
                val renewed = canonicalApplication.renewHostDelivery(
                    state.hostJournal.responseDelivery(request),
                )
                state.hostJournal.bindResponseDelivery(request, renewed)
            }
        }
        try {
            operation()
        } finally {
            heartbeat.cancel()
        }
    }

    private suspend fun runModel(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        payload: JsonObject,
        config: AgentCoreRuntimeConfig,
        toolPlan: AgentCoreAndroidToolPlan,
        state: RunState,
        emitEvent: suspend (DeviceAgentCoordinatorEvent) -> Unit,
    ): JsonObject {
        val turnId = request.string("turn_id")
            ?: error("Model Host request has no Turn ID")
        var textPartId = payload.string("text_part_id")
            ?: error("Model Host request has no text Part ID")
        var reasoningPartId = payload.string("reasoning_part_id")
            ?: error("Model Host request has no reasoning Part ID")
        val invocation = payload.objectValue("settings")
            ?.objectValue("model_invocation_plan")
        val modelRequest = AgentModelRequest(
            sessionId = state.sessionId,
            turnId = turnId,
            iteration = ++state.modelIteration,
            messages = androidCoreModelMessages(payload.array("messages")),
            tools = if (modelSupportsToolCalls) payload.array("tools") else emptyList(),
            reservedOutputTokens = config.reservedOutputTokens,
            stablePromptDigest = payload["settings"]
                ?.jsonObject
                ?.get("model_prefix_digest")
                ?.jsonPrimitive
                ?.contentOrNull,
            invocationPlan = AgentModelInvocationPlan(
                nativeToolCalls = invocation?.boolean("native_tool_calls") ?: false,
                parallelToolCalls = invocation?.boolean("parallel_tool_calls") ?: false,
                reasoning = invocation?.boolean("reasoning") ?: false,
                promptCache = invocation?.boolean("prompt_cache") ?: false,
                streamText = invocation?.boolean("stream_text") ?: false,
                streamReasoning = invocation?.boolean("stream_reasoning") ?: false,
                streamToolCalls = invocation?.boolean("stream_tool_calls") ?: false,
            ),
            providerContinuation = invocation?.objectValue("provider_continuation"),
        )
        val streamedText = StringBuilder()
        val streamedReasoning = StringBuilder()
        var completion: AgentModelCompletion? = null
        var emittedToolCall = false
        var attempt = 1
        val streamStartedAt = SystemClock.elapsedRealtime()
        var observedFirstText = false
        Log.d(
            AGENT_PROTOCOL_LOG_TAG,
            "model_stream_begin session=${state.sessionId} turn=$turnId iteration=${modelRequest.iteration}",
        )
        while (completion == null) {
            try {
                val sampling = modelSamplingAttempt(request, attempt - 1)
                val attemptId = sampling.id
                if (sampling.resampling) {
                    textPartId = "$attemptId:text"
                    reasoningPartId = "$attemptId:reasoning"
                }
                val admitted = submitHostResponse(
                    runtime,
                    request,
                    buildJsonObject {
                        put("type", "model_attempt_started")
                        put("provider", config.executionMode)
                        put("model", config.model ?: "android-active-model")
                        put("reason", if (sampling.resampling) "retry_or_recovery" else "initial")
                    },
                    state,
                    attemptId = attemptId,
                )
                acceptTransition(admitted, toolPlan, state, emitEvent)
                modelClient.generateAgentTurn(modelRequest).coalesceQueuedTextDeltas().collect { modelEvent ->
                    when (modelEvent) {
                        is AgentModelEvent.TextDelta -> {
                            val firstText = !observedFirstText
                            val receivedAt = SystemClock.elapsedRealtime()
                            if (firstText) {
                                observedFirstText = true
                                Log.d(AGENT_PROTOCOL_LOG_TAG,
                                    "model_first_text_received session=${state.sessionId} elapsed_ms=${receivedAt - streamStartedAt}")
                            }
                            streamedText.append(modelEvent.text)
                            val transition = submitHostResponse(
                                runtime,
                                request,
                                modelDeltaPayload("text", textPartId, modelEvent.text),
                                state,
                            )
                            val committedAt = SystemClock.elapsedRealtime()
                            acceptTransition(transition, toolPlan, state, emitEvent)
                            if (firstText) Log.d(AGENT_PROTOCOL_LOG_TAG,
                                "model_first_text_published session=${state.sessionId} commit_ms=${committedAt - receivedAt} publish_ms=${SystemClock.elapsedRealtime() - committedAt}")
                        }
                        is AgentModelEvent.ReasoningDelta -> {
                            streamedReasoning.append(modelEvent.text)
                            val transition = submitHostResponse(
                                runtime,
                                request,
                                modelDeltaPayload("reasoning", reasoningPartId, modelEvent.text),
                                state,
                            )
                            acceptTransition(transition, toolPlan, state, emitEvent)
                        }
                        is AgentModelEvent.ToolCallDelta -> {
                            emittedToolCall = true
                            val transition = submitHostResponse(
                                runtime,
                                request,
                                modelToolCallDeltaPayload(modelEvent),
                                state,
                            )
                            acceptTransition(transition, toolPlan, state, emitEvent)
                        }
                        is AgentModelEvent.Completed -> {
                            completion = modelEvent.completion
                            Log.d(
                                AGENT_PROTOCOL_LOG_TAG,
                                "model_stream_terminal session=${state.sessionId} turn=$turnId reason=${modelEvent.completion.finishReason}",
                            )
                        }
                    }
                }
                Log.d(
                    AGENT_PROTOCOL_LOG_TAG,
                    "model_stream_collected session=${state.sessionId} turn=$turnId completed=${completion != null}",
                )
                if (completion == null) error("The model stream ended without a completion")
            } catch (error: Throwable) {
                if (error is CancellationException) throw error
                val retryable = error.isRetryableModelFailure()
                val canRetry = retryable && attempt < 3 &&
                    streamedText.isEmpty() && streamedReasoning.isEmpty() && !emittedToolCall
                if (!canRetry) {
                    return submitHostResponse(
                        runtime,
                        request,
                        modelFailurePayload(error, retryable),
                        state,
                    )
                }
                val delayMs = 2_000L shl (attempt - 1)
                val transition = submitHostResponse(
                    runtime,
                    request,
                    buildJsonObject {
                        put("type", "tool_progress")
                        put("title", "model_retry")
                        put("detail", "attempt=$attempt delay_ms=$delayMs")
                        put("metadata", buildJsonObject {
                            put("attempt", attempt)
                            put("delay_ms", delayMs)
                        })
                    },
                    state,
                )
                acceptTransition(transition, toolPlan, state, emitEvent)
                delay(delayMs)
                attempt += 1
            }
        }

        val result = completion ?: error("The model stream ended without a completion")
        result.providerContinuation?.let { continuation ->
            val continuationTransition = submitHostResponse(
                runtime,
                request,
                buildJsonObject {
                    put("type", "model_continuation")
                    put("continuation", continuation)
                },
                state,
            )
            acceptTransition(continuationTransition, toolPlan, state, emitEvent)
        }
        val finalText = result.content.ifEmpty { streamedText.toString() }
        val finalReasoning = result.reasoning.ifEmpty { streamedReasoning.toString() }
        val items = buildJsonArray {
            add(
                buildJsonObject {
                    put("type", "message")
                    put("id", textPartId)
                    put("role", "assistant")
                    put("content", finalText)
                },
            )
            if (finalReasoning.isNotEmpty()) {
                add(
                    buildJsonObject {
                        put("type", "reasoning")
                        put("id", reasoningPartId)
                        put("summary", "")
                        put("content", finalReasoning)
                    },
                )
            }
        }
        val transition = submitHostResponse(
            runtime,
            request,
            buildJsonObject {
                put(
                    "type",
                    if (result.toolCalls.isEmpty() && isModelOutputLimitFinishReason(result.finishReason)) {
                        "model_output_limited"
                    } else {
                        "model_completed"
                    },
                )
                put("items", items)
                put(
                    "tool_calls",
                    normalizeToolCalls(
                        result.toolCalls,
                        modelRequest.turnId,
                        modelRequest.iteration,
                    ),
                )
                put(
                    "usage",
                    result.usage.toCoreUsageJson(),
                )
            },
            state,
        )
        Log.d(
            AGENT_PROTOCOL_LOG_TAG,
            "model_completed_submitted session=${state.sessionId} turn=$turnId traces=${transition.array("trace_events").size} requests=${transition.array("host_requests").size}",
        )
        return transition
    }

    private fun Throwable.isRetryableModelFailure(): Boolean = when (this) {
        is LlmRequestException -> retryable
        is IOException -> true
        else -> false
    }

    private fun modelFailurePayload(error: Throwable, retryable: Boolean): JsonObject = buildJsonObject {
        put("type", "failed")
        put("error", buildJsonObject {
            put("code", (error as? LlmRequestException)?.code ?: "android_model_failure")
            put("message", error.message ?: error::class.simpleName.orEmpty())
            put("retryable", retryable)
        })
    }

    private suspend fun emitMissingSuffix(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        channel: String,
        partId: String,
        streamed: String,
        complete: String,
        toolPlan: AgentCoreAndroidToolPlan,
        state: RunState,
        emitEvent: suspend (DeviceAgentCoordinatorEvent) -> Unit,
    ) {
        val suffix = when {
            complete == streamed -> ""
            complete.startsWith(streamed) -> complete.removePrefix(streamed)
            streamed.isEmpty() -> complete
            else -> ""
        }
        if (suffix.isEmpty()) return
        val transition = submitHostResponse(
            runtime,
            request,
            modelDeltaPayload(channel, partId, suffix),
            state,
        )
        acceptTransition(transition, toolPlan, state, emitEvent)
    }

    private suspend fun executeTool(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        payload: JsonObject,
        toolPlan: AgentCoreAndroidToolPlan,
        state: RunState,
        config: AgentCoreRuntimeConfig,
    ): JsonObject {
        val toolName = payload.string("capability_id")
            ?: error("Tool Host request has no capability ID")
        // Application approval is already bound to this effect. Never infer
        // it from the selected conversation or from the tool's arguments.
        AndroidAgentPermissionMode.fromWireValue(payload.string("permission_profile")
            ?: error("Tool Host request has no admitted permission profile"))
        if (toolName == "read_task" || toolName == "wait_task") {
            check(!embeddedMailbox && childToolPlan != null) { "Task result executor is unavailable" }
            var deadline: Long? = null
            try {
              while (true) {
                currentCoroutineContext().ensureActive()
                val observed = canonicalApplication.mobileCommand(buildJsonObject {
                    put("type", "observe_mobile_task"); put("input", buildJsonObject {
                        put("request", JsonObject(request + ("delivery" to state.hostJournal.responseDelivery(request))))
                        put("now_ms", System.currentTimeMillis())
                    })
                }).jsonObject
                val wait = observed.getValue("wait_ms").jsonPrimitive.content.toLong()
                if (deadline == null) deadline = SystemClock.elapsedRealtime() + wait
                val remaining = deadline - SystemClock.elapsedRealtime()
                if (observed.getValue("terminal").jsonPrimitive.booleanOrNull == true || remaining <= 0) {
                    return submitHostResponse(runtime, request, observed.getValue("response").jsonObject, state)
                }
                delay(minOf(remaining, 250L))
              }
            } finally {
                // Only the local observation ended; this is no child-stop ACK.
                state.cancellationEvidence.returnedLocally(request, canonicalApplication.descriptor.conversationId)
            }
        }
        if (toolName == "send_agent_message") {
            check(!embeddedMailbox && childToolPlan != null) { "Task guidance executor is unavailable" }
            val response = canonicalApplication.mobileCommand(buildJsonObject {
                put("type", "prepare_mobile_task_guidance"); put("input", buildJsonObject {
                    put("request", JsonObject(request + ("delivery" to state.hostJournal.responseDelivery(request))))
                    put("now_ms", System.currentTimeMillis())
                })
            }).jsonObject
            state.cancellationEvidence.returnedLocally(request, canonicalApplication.descriptor.conversationId)
            return submitHostResponse(runtime, request, response, state)
        }
        if (toolName == "spawn_agent") {
            check(!embeddedMailbox && childToolPlan != null) { "Independent child executor is unavailable" }
            val prepared=canonicalApplication.mobileCommand(buildJsonObject {
                put("type","prepare_mobile_delegated_task");put("input",buildJsonObject {
                    put("request",JsonObject(request + ("delivery" to state.hostJournal.responseDelivery(request))))
                    put("now_ms",System.currentTimeMillis())
                })
            }).jsonObject
            val promotion=prepared["response"] as? JsonObject ?: error("Core delegation response is missing")
            val committed=submitHostResponse(runtime,request,promotion,state)
            val task=prepared.getValue("prepared").jsonObject.getValue("task").jsonObject
            AndroidEmbeddedChildExecutor.start(task.getValue("id").jsonPrimitive.content,canonicalApplication) { childConfig ->
                val refresh = { checkNotNull(childToolPlan)(childConfig.getValue("session_id").jsonPrimitive.content) }
                val plan = refresh()
                AndroidPreparedChildExecutor(plan.tools) { childApplication ->
                    val child=DeviceAgentCoordinator(modelClient,permissionBroker,hostExecutor,childApplication,modelCapability,
                        refreshToolPlan=refresh,embeddedMailbox=true)
                    val execute: suspend (JsonObject,AgentCoreTransitionRuntime,RunState)->Unit = { envelope,sink,childState ->
                        child.executeEmbeddedRequest(envelope,sink,config,plan,childState,canonicalApplication.descriptor.conversationId)
                    }
                    execute
                }
            }
            return committed
        }
        if (toolName == "render_ui") {
            val completion = androidUiPublicationCompletion(payload) {
                canonicalApplication.publishUiSurface(request, state.hostJournal.responseDelivery(request))
            }
            state.cancellationEvidence.returnedLocally(request, canonicalApplication.descriptor.conversationId)
            return submitHostResponse(runtime, request, completion, state)
        }
        val host=request.getValue("host_id").jsonPrimitive.content
        val session=request.getValue("session_id").jsonPrimitive.content
        val mcp=prepareMcp(host,session,refreshToolPlan?.invoke() ?: toolPlan)
        if(mcp["capabilities"]?.jsonObject?.containsKey(toolName)==true) {
            val ids=hostExecutor.readableMcpServerIds()
            val command=buildJsonObject {
                put("type","execute");put("host_id",host);put("session_id",session)
                put("policy",(refreshToolPlan?.invoke() ?: toolPlan).originalIntent?.let { Json.parseToJsonElement(Json.encodeToString(it)) } ?: JsonNull)
                put("allowed_server_ids",JsonArray(ids.map(::JsonPrimitive)))
                put("request",JsonObject(request + ("delivery" to state.hostJournal.responseDelivery(request))))
            }
            val cancel=buildJsonObject {put("type","cancel");put("host_id",host);put("session_id",session);put("call_id",payload.getValue("call_id"))}
            state.cancellationEvidence.outcomeUnknown(request)
            val result=kotlinx.coroutines.suspendCancellableCoroutine<JsonObject> { continuation ->
                continuation.invokeOnCancellation { runCatching { canonicalApplication.mobileMcpHost(cancel) } }
                kotlin.concurrent.thread(name="nola-mobile-mcp") {
                    val result=runCatching { canonicalApplication.mobileMcpHost(command) }
                    if(continuation.isActive)continuation.resumeWith(result)
                }
            }
            if(result["output"]?.jsonObject?.get("execution")?.jsonObject?.get("state")?.jsonPrimitive?.content in setOf("completed","not_started")) {
                state.cancellationEvidence.returnedLocally(request,canonicalApplication.descriptor.conversationId)
            }
            return submitHostResponse(runtime,request,result,state)
        }
        val allowed = toolPlan.allowedTools.firstOrNull { it.toolId == toolName }
            ?: return submitHostResponse(
                runtime,
                request,
                androidRejectedToolCallPayload(
                    payload = payload,
                    code = "capability_unavailable",
                    message = "Tool is not available in the active Android plan.",
                ),
                state,
            )
        if (payload["input"] !is JsonObject) {
            return submitHostResponse(
                runtime,
                request,
                androidRejectedToolCallPayload(
                    payload = payload,
                    code = "invalid_tool_input",
                    message = "Tool input must be an object.",
                ),
                state,
            )
        }
        val call = androidAgentHostCallRequest(payload, allowed.capability)
        val result = hostExecutor.execute(call, toolPlan)
        if (result.isError) {
            state.cancellationEvidence.outcomeUnknown(request)
        } else {
            state.cancellationEvidence.returnedLocally(request,
                canonicalApplication.descriptor.conversationId)
        }
        return submitHostResponse(
            runtime,
            request,
            buildJsonObject {
                put("type", "tool_completed")
                put("call_id", result.callId)
                put("output", androidToolResult(result.output, if (result.isError) "unknown" else "completed"))
                put("outcome", if (result.isError) "failed" else "succeeded")
                put("full_output_ref", JsonNull)
                put("artifacts", result.artifacts)
            },
            state,
        )
    }

    private suspend fun resolveHostPermission(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        payload: JsonObject,
        toolPlan: AgentCoreAndroidToolPlan,
        decisions: Channel<AndroidAgentPermissionDecision>,
        cancellations: Channel<String>,
        state: RunState,
        emitEvent: suspend (DeviceAgentCoordinatorEvent) -> Unit,
    ): JsonObject {
        val permissionRequestId = payload.string("permission_request_id")
            ?: error("Permission Host request has no permission request ID")
        val inspected = canonicalApplication.mobileCommand(buildJsonObject {
            put("type", "inspect_mobile_permission_request")
            put("request", JsonObject(request + ("delivery" to state.hostJournal.responseDelivery(request))))
            put("now_ms", System.currentTimeMillis())
        }).jsonObject
        require(inspected["conversation_id"]?.jsonPrimitive?.content == canonicalApplication.descriptor.conversationId &&
            inspected["run_id"]?.jsonPrimitive?.content == canonicalApplication.descriptor.submissionId &&
            inspected["core_session_id"]?.jsonPrimitive?.content == state.sessionId &&
            inspected["effect_id"] == request["effect_id"] &&
            inspected["request_id"]?.jsonPrimitive?.content == permissionRequestId && inspected["payload"] == payload) {
            "Core permission presentation belongs to another request"
        }
        val toolName = payload.string("capability_id") ?: error("Missing permission capability")
        val internalConfirmation = inspected.getValue("permission_domain") == JsonNull ||
            inspected["capability"]?.jsonObject?.get("input_normalization")?.jsonObject?.get("origin")?.jsonObject?.get("kind") == JsonPrimitive("mcp")
        val allowed = if (internalConfirmation) null else (toolPlan.allowedTools.firstOrNull { it.toolId == toolName }
            ?: error("Permission Host request is outside the active Android plan"))
        val permissionRequest = AndroidAgentPermissionRequest(
            allowConversation = (payload["allow_conversation"] as? JsonPrimitive)?.takeUnless { it.isString }?.booleanOrNull
                ?: error("Missing or invalid permission scope eligibility"),
            requestId = permissionRequestId,
            callId = permissionRequestId,
            toolName = toolName,
            capability = allowed?.capability ?: "none",
            authorization = allowed?.permission ?: "none",
            rationale = if(inspected["capability"]?.jsonObject?.get("input_normalization")?.jsonObject?.get("origin")?.jsonObject?.get("kind") == JsonPrimitive("mcp"))
                inspected["display_title"]?.jsonPrimitive?.content.orEmpty()+"\n"+payload.string("reason").orEmpty()
                else payload.string("reason").orEmpty(),
            originKind = inspected["capability"]?.jsonObject?.get("input_normalization")?.jsonObject?.get("origin")?.jsonObject?.get("kind")?.jsonPrimitive?.content,
            scope = payload.string("resource"),
            permissionInstructions = payload["permission_instructions"]?.let {
                Json.decodeFromJsonElement<List<AgentPermissionInstruction>>(it)
            } ?: emptyList(),
        )
        state.permissionRequests[permissionRequestId] = permissionRequest
        if (internalConfirmation) permissionBroker.bindCoreConfirmation(permissionRequest)
        else check(permissionBroker.bindPermissionRequest(permissionRequest, toolPlan, refreshToolPlan)) {
            "Permission request could not be bound to the active Core tool plan"
        }
        emitEvent(DeviceAgentCoordinatorEvent.PermissionRequested(permissionRequest))
        var waitResult: PermissionWaitResult
        while (true) {
            waitResult = select {
                decisions.onReceive { PermissionWaitResult.Decision(it) }
                cancellations.onReceive { PermissionWaitResult.Cancel(it) }
                policyChanges?.let { changes -> changes.onReceive { PermissionWaitResult.PolicyChanged } }
            }
            if (waitResult != PermissionWaitResult.PolicyChanged) break
            val updated = canonicalApplication.applyPendingPermissionPolicy()?.agentCoreTransitionOrThrow() ?: continue
            val stillPending = canonicalControlProjection(updated)?.objectValue("permissions")?.array("pending")
                ?.any { (it as? JsonObject)?.string("request_id") == permissionRequestId } == true
            if (stillPending) {
                acceptTransition(updated, toolPlan, state, emitEvent)
                continue
            }
            // Application resolves the original pending call atomically. This
            // wakeup carries no allow/deny decision and creates no new turn.
            state.permissionRequests.remove(permissionRequestId)
            permissionBroker.clearPermissionRequest(permissionRequestId)
            return updated
        }
        if (waitResult is PermissionWaitResult.Cancel) {
            state.permissionRequests.remove(permissionRequestId)
            permissionBroker.clearPermissionRequest(permissionRequestId)
            return runtime.submit(
                deviceAgentCancellationSubmission(waitResult.reason),
            ).agentCoreTransitionOrThrow()
        }
        val decision = (waitResult as PermissionWaitResult.Decision).decision
        require(decision.requestId == permissionRequest.requestId) {
            "Permission request ID does not match"
        }
        val validatedDecision = if (decision.decision == AndroidAgentPermissionUserDecision.ProvideInstruction) {
            decision.copy(instruction = normalizedPermissionInstruction(permissionRequestId,
                permissionRequest.permissionInstructions, requireNotNull(decision.instruction)))
        } else decision
        val resolution = corePermissionResolution(validatedDecision)
        state.permissionRequests.remove(permissionRequestId)
        permissionBroker.clearPermissionRequest(permissionRequestId)
        return submitHostResponse(
            runtime,
            request,
            buildJsonObject {
                put("type", "permission_resolved")
                put("request_id", permissionRequestId)
                put("resolution", resolution)
            },
            state,
        )
    }

    private suspend fun compactContext(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        payload: JsonObject,
        config: AgentCoreRuntimeConfig,
        toolPlan: AgentCoreAndroidToolPlan,
        state: RunState,
        emitEvent: suspend (DeviceAgentCoordinatorEvent) -> Unit,
    ): JsonObject {
        val instructions = payload.string("instructions")?.takeIf { it.isNotBlank() }
            ?: error("Compaction Host request has no Core instructions")
        val target = payload["target_tokens"]?.jsonPrimitive?.intOrNull?.takeIf { it > 0 }
            ?: error("Compaction Host request has no target budget")
        val window = config.contextWindowTokens?.takeIf { it > 0 }
            ?: error("Compaction Host request has no context window")
        var maintenanceRequest = request
        var attempts = 0
        val executor = CompactionSummaryExecutor(canonicalApplication::executeCompactionCommand) { messages, policy, continuation ->
            val sampling = modelSamplingAttempt(request, attempts++)
            val attemptId = sampling.id
            maintenanceRequest = JsonObject(request + ("attempt_id" to JsonPrimitive(attemptId)))
            val admitted = submitHostResponse(runtime, maintenanceRequest, buildJsonObject {
                put("type", "model_attempt_started"); put("provider", config.executionMode)
                put("model", config.model ?: "android-active-model")
                put("reason", if (sampling.resampling) "compaction_resampling" else "initial")
            }, state, attemptId = attemptId)
            acceptTransition(admitted, toolPlan, state, emitEvent)
            var completion: AgentModelCompletion? = null
            modelClient.generateAgentTurn(AgentModelRequest(
                sessionId = state.sessionId, turnId = request.string("turn_id").orEmpty(),
                iteration = ++state.modelIteration, messages = messages, tools = emptyList(),
                reservedOutputTokens = config.reservedOutputTokens,
                policyFragments = listOf(AgentPromptFragment(id = "core-context-compaction",
                    layer = AgentPromptLayer.ExecutionPolicy, trust = AgentPromptTrust.HostInstruction,
                    cacheable = false, content = policy)), providerContinuation = continuation,
            )).collect { event -> if (event is AgentModelEvent.Completed) completion = event.completion }
            completion ?: error("Compaction model stream ended without a completion")
        }
        val result = executor.summarize(payload.array("messages"), window, config.reservedOutputTokens, target, instructions)
        currentCoroutineContext().ensureActive()
        return submitHostResponse(runtime, maintenanceRequest, buildJsonObject {
            put("type", "compaction_completed"); put("summary", result.content)
            put("usage", result.usage.toCoreUsageJson())
        }, state)
    }

    private suspend fun submitHostResponse(
        runtime: AgentCoreTransitionRuntime,
        request: JsonObject,
        payload: JsonObject,
        state: RunState,
        attemptId: String? = null,
    ): JsonObject {
        currentCoroutineContext().ensureActive()
        val effectId = request.string("effect_id")
            ?: error("Host request has no effect ID")
        val responseAttemptId = attemptId
            ?: state.hostJournal.responseAttemptId(request)
            ?: request.string("attempt_id")
        val response = buildJsonObject {
            putNullableString("attempt_id", responseAttemptId)
            put("delegated_invocation_id", request["delegated_invocation_id"] ?: JsonNull)
            put("request_id", request.string("request_id").orEmpty())
            put("effect_id", effectId)
            put("host_id", request["host_id"] ?: JsonPrimitive(state.hostId))
            put("lease_id", request["lease_id"] ?: JsonNull)
            put("delivery", state.hostJournal.responseDelivery(request))
            put("session_id", request.string("session_id").orEmpty())
            putNullableString("turn_id", request.string("turn_id"))
            put("catalog_revision", request.long("catalog_revision") ?: 0)
            put("availability_epoch", request.long("availability_epoch") ?: 0)
            put(
                "sequence",
                state.hostJournal.nextSequence(request),
            )
            put("payload", payload)
        }
        attemptId?.let { state.hostJournal.bindResponseAttempt(request, it) }
        if (runtime is AndroidCommittedChildMailbox) {
            runtime.commit(response) // returns only after Core's committed ACK
            if (payload.closesHostEffect()) state.hostJournal.completeEffect(effectId)
            // No transition projection is returned by this Host-only adapter.
            // Child publication consumers read their own committed journal.
            return JsonObject(emptyMap())
        }
        val transition = runtime.submit(
            buildJsonObject {
                put("type", "host_response")
                put("response", response)
            },
        ).agentCoreTransitionOrThrow()
        if (payload.closesHostEffect()) state.hostJournal.completeEffect(effectId)
        return transition
    }

    private fun JsonObject.closesHostEffect(): Boolean = string("type") in setOf(
        "model_completed",
        "model_failed",
        "tool_completed",
        "permission_resolved",
        "compaction_completed",
        "context_fetched",
        "cancellation_confirmed",
        "completed",
        "failed",
    )

    private suspend fun prepareMcp(host:String,session:String,plan:AgentCoreAndroidToolPlan):JsonObject {
        val ids=hostExecutor.readableMcpServerIds()
        return withContext(Dispatchers.IO) {canonicalApplication.mobileMcpHost(buildJsonObject {
            put("type","prepare");put("host_id",host);put("session_id",session)
            put("policy",plan.originalIntent?.let { Json.parseToJsonElement(Json.encodeToString(it)) } ?: JsonNull)
            put("allowed_server_ids",JsonArray(ids.map(::JsonPrimitive)))
        })}
    }

    private suspend fun buildRuntimeConfig(
        sessionId: String,
        history: List<AgentCoreMessage>,
        tools: List<JsonElement>,
        toolPlan: AgentCoreAndroidToolPlan,
        config: AgentCoreRuntimeConfig,
    ): JsonObject {
        val hostId = config.hostId?.trim().takeUnless { it.isNullOrEmpty() }
            ?: toolPlan.identity.hostId?.trim().takeUnless { it.isNullOrEmpty() }
            ?: "android-device"
        val mcp = prepareMcp(hostId,sessionId,toolPlan)
        val uiContract = canonicalApplication.uiCapabilityContract()
        val uiDescriptor = uiContract.getValue("descriptor").jsonObject
        val capabilities = buildJsonObject {
            put("model.stream", internalCapability("model.stream", "Model", "model", streaming = true))
            put(
                "permission.ask",
                internalCapability("permission.ask", "Permission", "permission"),
            )
            put(
                "context.compact",
                internalCapability("context.compact", "Compaction", "compaction"),
            )
            put(
                "host.cancel",
                internalCapability("host.cancel", "Cancellation", "tool", exposure = "hidden"),
            )
            mcp["capabilities"]?.jsonObject?.forEach { (id,descriptor)->put(id,descriptor) }
            toolPlan.allowedTools.forEach { allowed ->
                put(allowed.toolId, if (allowed.toolId == "render_ui") uiDescriptor else toolCapability(allowed, tools))
            }
        }
        val result = buildJsonObject {
            put("session_id", sessionId)
            put("model", config.model?.trim().takeUnless { it.isNullOrEmpty() } ?: "android-active-model")
            put("initial_items", initialItems(history, sessionId))
            put("tools", JsonArray(if (modelSupportsToolCalls) tools + mcp.getValue("tools").jsonArray else emptyList()))
            put("memory_policy", automaticMemoryPolicyConfig())
            val stablePromptDigest = config.stablePromptDigest
                ?.trim()
                ?.takeIf(String::isNotEmpty)
            val promptFragmentIds = config.promptFragmentIds
                .map(String::trim)
                .filter(String::isNotEmpty)
            val stablePromptFragmentDigests = config.stablePromptFragmentDigests
                .mapKeys { (id, _) -> id.trim() }
                .mapValues { (_, digest) -> digest.trim() }
                .filter { (id, digest) -> id.isNotEmpty() && digest.isNotEmpty() }
            run {
                put(
                    "turn_identity",
                    buildJsonObject {
                        put("model_capability_id", modelCapability.id)
                        put("model_capability_revision", modelCapability.revision)
                        put("model_capability", modelCapability.wire())
                        put(
                            "prompt",
                            buildJsonObject {
                                stablePromptDigest?.let { put("stable_prefix_digest", it) }
                                put(
                                    "fragment_ids",
                                    JsonArray(promptFragmentIds.map(::JsonPrimitive)),
                                )
                                put(
                                    "stable_fragment_digests",
                                    buildJsonObject {
                                        stablePromptFragmentDigests.forEach { (id, digest) ->
                                            put(id, digest)
                                        }
                                    },
                                )
                            },
                        )
                    },
                )
            }
            put(
                "context_window",
                buildJsonObject {
                    put("context_window_tokens", config.contextWindowTokens ?: 128_000)
                    put("reserved_output_tokens", config.reservedOutputTokens)
                    put("auto_compact_percent", 80)
                    put("compaction_target_percent", 45)
                },
            )
            put(
                "host_catalog",
                buildJsonObject {
                    put("host_id", hostId)
                    put("revision", 3)
                    put("availability_epoch", 1)
                    put("capabilities", capabilities)
                },
            )
            val rules=permissionRules(toolPlan)
            put("permission_rules",JsonObject(rules + ("rules" to JsonArray(rules.getValue("rules").jsonArray + mcp.getValue("permission_rules").jsonArray))))
        }
        val prepared = config.preparedHostInvocation ?: return result
        val identity = result["turn_identity"]?.jsonObject.orEmpty().toMutableMap()
        identity["host_invocation"] = bindAndroidInvocation(prepared, result)
        return JsonObject(result + ("turn_identity" to JsonObject(identity)))
    }

    private fun internalCapability(
        id: String,
        title: String,
        route: String,
        streaming: Boolean = false,
        exposure: String = "internal",
    ): JsonObject = capabilityDescriptor(
        id = id,
        title = title,
        route = route,
        exposure = exposure,
        permissionDomain = null,
        inputSchema = JsonObject(emptyMap()),
        streaming = streaming,
        argumentStreaming = false,
        parallelism = "serial",
        cancellation = if (streaming) "cooperative" else "best_effort",
        replaySafety = "recoverable",
        effect = "execute",
        originKind = "internal",
    )

    private fun toolCapability(
        tool: AgentCoreAllowedAndroidTool,
        schemas: List<JsonElement>,
    ): JsonObject {
        val schema = schemas.firstOrNull { toolName(it) == tool.toolId }
        val inputSchema = (schema as? JsonObject)
            ?.objectValue("function")
            ?.get("parameters")
            ?: JsonObject(emptyMap())
        val effect = engineeringEffect(schema)
        val readOnly = effect == "investigate"
        return capabilityDescriptor(
            id = tool.toolId,
            title = tool.toolId,
            route = "tool",
            exposure = "visible",
            permissionDomain = tool.capability.takeUnless { tool.permission == "none" },
            inputSchema = inputSchema,
            streaming = false,
            argumentStreaming = true,
            parallelism = if (readOnly) "concurrent_safe" else "serial",
            cancellation = "best_effort",
            replaySafety = if (readOnly) {
                "idempotent"
            } else {
                "requires_recovery_decision"
            },
            effect = effect,
            originKind = "device",
            permissionContract = (schema as? JsonObject)?.objectValue("x_nola")?.get("permission_contract"),
        )
    }

    private fun capabilityDescriptor(
        id: String,
        title: String,
        route: String,
        exposure: String,
        permissionDomain: String?,
        inputSchema: JsonElement,
        streaming: Boolean,
        argumentStreaming: Boolean,
        parallelism: String,
        cancellation: String,
        replaySafety: String,
        effect: String,
        originKind: String,
        permissionContract: JsonElement? = null,
    ): JsonObject = buildJsonObject {
        put("id", id)
        put("title", title)
        put("route", route)
        put("availability", "available")
        put("exposure", exposure)
        putNullableString("permission_domain", permissionDomain)
        put("input_schema", inputSchema)
        put(
            "input_normalization",
            buildJsonObject {
                put(
                    "origin",
                    buildJsonObject {
                        put("kind", originKind)
                        put("provider_id", "android-device")
                        put("provider_title", "Android")
                        put("namespace", if (originKind == "internal") "internal" else "device")
                    },
                )
                put(
                    "effect",
                    buildJsonObject {
                        put("schema_version", 1)
                        put("fixed", effect)
                        put("variants", buildJsonObject {})
                    },
                )
                put("permission_resource", permissionResourceDeclaration(id))
                put("permission_contract", permissionContract ?: buildJsonObject { put("risk", "unknown") })
            },
        )
        put(
            "output_policy",
            buildJsonObject {
                put("preview_char_limit", 8_000)
                put("persist_full_output", true)
                put("allow_artifacts", true)
            },
        )
        put("supports_streaming", streaming)
        put("supports_argument_streaming", argumentStreaming)
        put("parallelism", parallelism)
        put("cancellation", cancellation)
        put("replay_safety", replaySafety)
    }

    private fun permissionResourceDeclaration(capabilityId: String): JsonObject = when (capabilityId) {
        "device_read_file" -> buildJsonObject {
            put("kind", "first_of")
            put("keys", JsonArray(listOf("path", "url", "uri").map(::JsonPrimitive)))
        }
        "device_write_file" -> buildJsonObject {
            put("kind", "first_of")
            put("keys", JsonArray(listOf("destination", "path", "uri").map(::JsonPrimitive)))
        }
        "device_clipboard_read", "device_clipboard_write" -> buildJsonObject {
            put("kind", "constant")
            put("value", "clipboard")
        }
        "device_get_location" -> buildJsonObject {
            put("kind", "constant")
            put("value", "current_location")
        }
        "device_search_calendar_events" -> buildJsonObject {
            put("kind", "constant")
            put("value", "calendar_events")
        }
        else -> buildJsonObject { put("kind", "full_input") }
    }

    private fun engineeringEffect(schema: JsonElement?): String {
        val declared = (schema as? JsonObject)
            ?.objectValue("x_nola")
            ?.string("effect")
        return when (declared) {
            "read", "inspect" -> "investigate"
            "write", "external_write", "mutate" -> "mutate"
            "validate" -> "validate"
            "mutate_and_validate" -> "mutate_and_validate"
            else -> "execute"
        }
    }

    private fun permissionRules(toolPlan: AgentCoreAndroidToolPlan): JsonObject =
        androidAgentPermissionRules(toolPlan)

    private fun initialItems(history: List<AgentCoreMessage>, sessionId: String): JsonArray =
        buildJsonArray {
            history.forEachIndexed { messageIndex, message ->
                val identityPrefix = "$sessionId:history:${messageIndex.toString().padStart(8, '0')}"
                when (message.role) {
                    "tool" -> {
                        val callId = message.toolCallId ?: return@forEachIndexed
                        add(
                            buildJsonObject {
                                put("type", "tool_result")
                                put("id", "$identityPrefix:tool-result")
                                put("call_id", callId)
                                put("name", message.name ?: "tool")
                                put("output", parseJsonOrString(message.content))
                                put("is_error", false)
                            },
                        )
                    }
                    else -> {
                        add(
                            buildJsonObject {
                                put("type", "message")
                                put("id", "$identityPrefix:message")
                                put("role", normalizeRole(message.role))
                                put("content", message.content)
                            },
                        )
                        message.toolCalls.forEachIndexed { callIndex, raw ->
                            val normalized = normalizeToolCall(
                                raw,
                                "$identityPrefix:tool-call:$callIndex",
                            )
                            add(
                                buildJsonObject {
                                    put("type", "tool_call")
                                    put("id", "$identityPrefix:tool-call:$callIndex")
                                    put("call_id", normalized.string("call_id").orEmpty())
                                    put("name", normalized.string("name").orEmpty())
                                    put("arguments", normalized["arguments"] ?: JsonObject(emptyMap()))
                                },
                            )
                        }
                    }
                }
            }
        }

    private fun normalizeToolCalls(
        calls: List<JsonObject>,
        turnId: String,
        iteration: Int,
    ): JsonArray =
        buildJsonArray {
            calls.forEachIndexed { index, call ->
                add(
                    normalizeToolCall(
                        call,
                        fallbackCallId = "$turnId-$iteration-tool-call-$index",
                    ),
                )
            }
        }

    private fun normalizeToolCall(raw: JsonObject, fallbackCallId: String): JsonObject {
        val function = raw.objectValue("function")
        val rawArguments = raw["arguments"] ?: function?.get("arguments") ?: JsonObject(emptyMap())
        return buildJsonObject {
            put(
                "call_id",
                raw.string("call_id")?.takeIf(String::isNotBlank)
                    ?: raw.string("id")?.takeIf(String::isNotBlank)
                    ?: fallbackCallId,
            )
            put("name", raw.string("name") ?: function?.string("name").orEmpty())
            put(
                "arguments",
                if (rawArguments is JsonPrimitive && rawArguments.isString) {
                    parseJsonOrString(rawArguments.content)
                } else {
                    rawArguments
                },
            )
            raw.string("permission_scope")?.let { put("permission_scope", it) }
            raw.string("permission_rationale")?.let { put("permission_rationale", it) }
        }
    }

    private fun modelDeltaPayload(
        channel: String,
        partId: String,
        delta: String,
    ): JsonObject = buildJsonObject {
        put("type", "model_delta")
        put("channel", channel)
        put("part_id", partId)
        put("delta", delta)
    }

    private fun modelToolCallDeltaPayload(event: AgentModelEvent.ToolCallDelta): JsonObject = buildJsonObject {
        put("type", "model_tool_call_delta")
        put("part_id", event.partId)
        put("call_id", event.callId)
        put("capability_id", event.capabilityId)
        put("arguments_delta", event.argumentsDelta)
    }

    private fun toolName(schema: JsonElement): String? =
        (schema as? JsonObject)?.objectValue("function")?.string("name")
            ?: (schema as? JsonObject)?.string("name")

    private fun normalizeRole(role: String): String = when (role) {
        "system", "developer", "user", "assistant" -> role
        else -> "user"
    }

    private fun parseJsonOrString(value: String): JsonElement =
        runCatching { json.parseToJsonElement(value) }.getOrElse { JsonPrimitive(value) }

    private fun JsonObject.toClientSnapshot(state: RunState): AgentCoreSnapshot =
        AgentCoreSnapshot(
            platform = "android",
            executionMode = state.executionMode,
            hostId = state.hostId,
            phase = when (string("turn_status")) {
                "awaiting_model" -> "model"
                "awaiting_tools" -> "tool"
                "awaiting_permission" -> "permission"
                "compacting" -> "compacting"
                "completed" -> "completed"
                "cancelled" -> "cancelled"
                "failed" -> "error"
                else -> "idle"
            },
            sessionId = string("session_id"),
            turnId = string("active_turn_id"),
            iteration = state.modelIteration,
            messageCount = int("history_item_count") ?: 0,
            estimatedContextTokens = int("context_used_tokens") ?: 0,
            contextWindowTokens = int("context_window_tokens"),
            contextUsagePercent = double("context_usage_percent"),
            pendingToolCount = int("active_tool_count") ?: 0,
            awaitingPermission = string("turn_status") == "awaiting_permission",
            awaitingHostCall = (int("pending_host_effect_count") ?: 0) > 0,
            promptCacheDiagnostic = objectValue("prompt_cache_diagnostic")?.let { diagnostic ->
                runCatching {
                    json.decodeFromJsonElement<AgentCorePromptCacheDiagnostic>(diagnostic)
                }.getOrNull()
            },
            permissions = objectValue("permissions")?.let { projection ->
                json.decodeFromJsonElement<AgentCorePermissionProjection>(projection)
            },
        )

    private fun JsonObject.string(key: String): String? =
        (this[key] as? JsonPrimitive)?.contentOrNull

    private fun JsonObject.boolean(key: String): Boolean? =
        (this[key] as? JsonPrimitive)?.booleanOrNull

    private fun JsonObject.int(key: String): Int? =
        (this[key] as? JsonPrimitive)?.intOrNull

    private fun JsonObject.long(key: String): Long? =
        (this[key] as? JsonPrimitive)?.contentOrNull?.toLongOrNull()

    private fun JsonObject.double(key: String): Double? =
        (this[key] as? JsonPrimitive)?.doubleOrNull

    private fun JsonObject.objectValue(key: String): JsonObject? =
        this[key] as? JsonObject

    private fun JsonObject.array(key: String): List<JsonElement> =
        (this[key] as? JsonArray)?.toList().orEmpty()

    internal class RunState(
        val sessionId: String,
        val executionMode: String,
        val hostId: String,
    ) {
        val hostJournal = DeviceHostExactlyOnceCoordinator(sessionId, hostId)
        val permissionRequests = mutableMapOf<String, AndroidAgentPermissionRequest>()
        val cancellationEvidence = AndroidCancellationEvidence()
        var interruptionCommitted = false
        var assistantText: String = ""
        var modelIteration: Int = 0
        var lastSnapshot: AgentCoreSnapshot? = null
        var taskProjection: AgentCoreTaskProjection = AgentCoreTaskProjection(sessionId = sessionId)
        var skillProjection: AgentCoreSkillRuntimeProjection =
            AgentCoreSkillRuntimeProjection(sessionId = sessionId)
        var capabilityActivationCursor: Long = 0
        var capabilityActivationRevision: Long = 0

    }

    private sealed interface PermissionWaitResult {
        data object PolicyChanged : PermissionWaitResult
        data class Decision(val decision: AndroidAgentPermissionDecision) : PermissionWaitResult
        data class Cancel(val reason: String) : PermissionWaitResult
    }

}

internal fun corePermissionDecisionWireValue(decision: AndroidAgentPermissionDecision): String =
    decision.decision.wireValue

internal fun androidAgentHostCallRequest(
    payload: JsonObject,
    capability: String,
): AndroidAgentHostCallRequest {
    val callId = (payload["call_id"] as? JsonPrimitive)
        ?.contentOrNull
        ?.takeIf(String::isNotBlank)
        ?: error("Tool Host request has no call ID")
    val toolName = (payload["capability_id"] as? JsonPrimitive)
        ?.contentOrNull
        ?.takeIf(String::isNotBlank)
        ?: error("Tool Host request has no capability ID")
    require(capability.isNotBlank()) { "Tool Host request has no resolved capability" }
    return AndroidAgentHostCallRequest(
        callId = callId,
        toolName = toolName,
        capability = capability,
        arguments = payload["input"] as? JsonObject ?: JsonObject(emptyMap()),
        grant = payload["grant"] as? JsonObject,
    )
}

/** Only a durable rejection is a known failed publication; unknown outcomes propagate. */
internal suspend fun androidUiPublicationCompletion(
    payload: JsonObject,
    publish: suspend () -> JsonElement,
): JsonObject {
    val publication = try { publish() }
    catch (rejected: com.nola.data.local.AndroidApplicationRuntime.RejectedMutation) {
        return androidRejectedToolCallPayload(payload, "ui_publication_rejected",
            rejected.message ?: "UI publication was rejected without changes.")
    }
    return buildJsonObject {
        put("type", "tool_completed")
        put("call_id", payload["call_id"]?.jsonPrimitive?.content ?: error("UI effect lacks call identity"))
        put("output", androidToolResult(publication, "completed"))
        put("outcome", "succeeded")
        put("full_output_ref", JsonNull)
        put("artifacts", JsonArray(emptyList()))
    }
}

internal fun androidRejectedToolCallPayload(
    payload: JsonObject,
    code: String,
    message: String,
): JsonObject {
    val callId = (payload["call_id"] as? JsonPrimitive)
        ?.contentOrNull
        ?.takeIf(String::isNotBlank)
        ?: error("Tool Host request has no call ID")
    val toolName = (payload["capability_id"] as? JsonPrimitive)
        ?.contentOrNull
        ?.takeIf(String::isNotBlank)
        ?: error("Tool Host request has no capability ID")
    val originalInput = payload["input"] ?: JsonNull
    return buildJsonObject {
        put("type", "tool_completed")
        put("call_id", callId)
        put("output", androidToolResult(buildJsonObject {
            put("error", buildJsonObject {
                put("code", code)
                put("message", message)
            })
            put("requested_call", buildJsonObject {
                put("call_id", callId)
                put("name", toolName)
                put("arguments", originalInput)
            })
        }, "not_started"))
        put("outcome", "failed")
        put("full_output_ref", JsonNull)
        put("artifacts", JsonArray(emptyList()))
    }
}


internal fun androidToolResult(data: JsonElement, state: String): JsonObject = buildJsonObject {
    put("contract", "nola.tool_result.v1")
    put("data", data)
    put("presentation", buildJsonObject { put("text", ""); put("summary", "") })
    put("execution", buildJsonObject {
        put("state", state)
        put("local_stop", JsonNull)
        put("effects", JsonArray(emptyList()))
        put("checks", JsonArray(emptyList()))
    })
    put("full_output_ref", JsonNull)
    put("memory_usage", JsonNull)
}

internal fun normalizedPermissionInstruction(instruction: String): String {
    val text = instruction.trim()
    require(text.isNotEmpty() && text.toByteArray(Charsets.UTF_8).size <= 4096) { "Permission instruction must contain 1–4096 UTF-8 bytes" }
    return text
}

internal fun corePermissionResolution(decision: AndroidAgentPermissionDecision): JsonObject = buildJsonObject {
    if (decision.decision == AndroidAgentPermissionUserDecision.ProvideInstruction) {
        put("type", "intervention")
        put("intervention", buildJsonObject {
            put("type", "provide_instruction")
            put("instruction", normalizedPermissionInstruction(requireNotNull(decision.instruction)))
        })
    } else {
        put("type", "authorization")
        put("decision", decision.decision.wireValue)
        put("grant", decision.scopedUri?.let { uri -> buildJsonObject { put("uri", uri) } } ?: JsonNull)
    }
}

internal fun normalizedPermissionInstruction(
    requestId: String,
    history: List<AgentPermissionInstruction>,
    instruction: String,
): String {
    val text = normalizedPermissionInstruction(instruction)
    require(requestId.isNotBlank() && history.size < 16) { "Permission instruction history is full; choose approve or deny" }
    val total = history.sumOf { it.permissionRequestId.toByteArray(Charsets.UTF_8).size.toLong() + it.instruction.toByteArray(Charsets.UTF_8).size } +
        requestId.toByteArray(Charsets.UTF_8).size + text.toByteArray(Charsets.UTF_8).size
    require(total <= 16384) { "Permission instruction history exceeds 16 KiB; choose approve or deny" }
    return text
}
