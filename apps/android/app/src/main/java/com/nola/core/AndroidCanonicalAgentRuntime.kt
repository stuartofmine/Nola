package com.nola.core

import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.loadPreparedEditedResend
import com.nola.data.local.requireIdentity
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.int
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.intOrNull
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.ensureActive
import kotlin.coroutines.coroutineContext
import java.util.UUID

data class AndroidQueuedUserAdmission(
    val commandId: String,
    val leaseOwner: String,
    val deliveryAttempt: Long,
    val userMessageId: String,
)

data class AndroidCanonicalTurnDescriptor(
    val conversationId: String,
    val conversationTitle: String,
    val submissionId: String,
    val userMessageId: String,
    val userText: String,
    val userContentJson: String?,
    val assistantMessageId: String = "$submissionId:assistant",
    val contextSnapshotId: String? = null,
    val createdAtMs: Long = System.currentTimeMillis(),
    val automationLease: com.nola.data.local.AndroidAutomationLease? = null,
    val automationSpec: com.nola.data.local.AndroidAutomationSpec? = null,
    val expectedConfigurationRevision: Long? = null,
    val expectedParentMessageId: String? = null,
    val queuedUserAdmission: AndroidQueuedUserAdmission? = null,
    val editedResendOperationId: String? = null,
    val mainExpertSelection: JsonObject? = null,
)

/** Construct the current Core admission while keeping the durable action in Core. */
internal fun encodeQueuedUserTurnAdmission(
    descriptor: AndroidCanonicalTurnDescriptor,
    config: JsonElement,
    submission: JsonObject,
    nowMs: Long,
): JsonObject {
    val queued = checkNotNull(descriptor.queuedUserAdmission)
    require(descriptor.automationLease == null && queued.commandId == descriptor.submissionId)
    require(submission["type"]?.jsonPrimitive?.content == "user_input")
    return buildJsonObject {
        put("type", "begin_queued_user_turn")
        put("input", buildJsonObject {
            put("conversation_id", descriptor.conversationId)
            put("command_id", queued.commandId)
            put("lease_owner", queued.leaseOwner)
            put("delivery_attempt", queued.deliveryAttempt)
            put("expected_configuration_revision", checkNotNull(descriptor.expectedConfigurationRevision))
            putNullableString("expected_parent_message_id", descriptor.expectedParentMessageId)
            put("core_config", config)
            put("now_ms", nowMs)
            put("turn_context_items", submission.getValue("turn_context_items"))
            put("turn_context_platform", submission.getValue("turn_context_platform"))
            put("turn_context_components", submission.getValue("turn_context_components"))
        })
    }
}

/** A bounded canonical history window provides the complete typed journal owner. */
internal fun loadAndroidHistoryTerminal(identity: JsonObject, execute: (JsonObject) -> kotlinx.serialization.json.JsonElement): AndroidCanonicalRunTerminalSeal? {
    val conversation=identity.getValue("conversation_id").jsonPrimitive.content
    val run=identity.getValue("run_id").jsonPrimitive.content
    val assistant=identity.getValue("assistant_message_id").jsonPrimitive.content
    require(listOf(conversation,run,assistant,identity.getValue("source_store_epoch").jsonPrimitive.content).all { it.isNotBlank() })
    val raw=execute(buildJsonObject {put("type","local_assistant_run_terminal_seal");put("conversation_id",conversation);put("run_id",run)})
    if(raw==JsonNull)return null
    val seal=raw.jsonObject
    require(seal["identity"]==identity) { "Historical terminal source changed" }
    return loadAndroidCanonicalTerminalSeal(AndroidCanonicalTurnDescriptor(conversation,"",run,"","",null,assistant),
        seal.getValue("core_session_id").jsonPrimitive.content) {raw}
}

/** A decoded source Store seal, not a client completion flag or trace projection. */
data class AndroidCanonicalRunTerminalSeal internal constructor(
    val conversationId: String,
    val runId: String,
    val assistantMessageId: String,
    val coreSessionId: String,
    val result: JsonObject,
    val resultDigest: String,
    val sealDigest: String,
)

internal fun loadAndroidCanonicalTerminalSeal(
    descriptor: AndroidCanonicalTurnDescriptor,
    coreSessionId: String,
    execute: (JsonObject) -> kotlinx.serialization.json.JsonElement,
): AndroidCanonicalRunTerminalSeal? {
    require(coreSessionId.isNotBlank())
    val value = execute(buildJsonObject {
        put("type", "local_assistant_run_terminal_seal")
        put("conversation_id", descriptor.conversationId)
        put("run_id", descriptor.submissionId)
    })
    if (value == JsonNull) return null
    val seal = value.jsonObject
    val identity = seal.getValue("identity").jsonObject
    val result = seal.getValue("result").jsonObject
    val resultDigest = seal.string("result_digest")
    val digest = seal.string("seal_digest")
    require(seal["protocol_version"]?.jsonPrimitive?.intOrNull == 1 &&
        identity.string("conversation_id") == descriptor.conversationId &&
        identity.string("run_id") == descriptor.submissionId &&
        identity.string("assistant_message_id") == descriptor.assistantMessageId &&
        !identity.string("source_store_epoch").isNullOrBlank() &&
        seal.getValue("admission").jsonObject["identity"] == identity &&
        seal.string("core_session_id") == coreSessionId &&
        result.string("conversation_id") == descriptor.conversationId &&
        result.string("run_id") == descriptor.submissionId &&
        result.hasValidRunResultStructure() &&
        result.getValue("value").jsonObject.string("kind") in
            setOf("answered", "action_completed", "delegation_accepted", "incomplete", "blocked", "cancelled", "failed") &&
        resultDigest?.matches(Regex("[0-9a-f]{64}")) == true &&
        digest?.matches(Regex("[0-9a-f]{64}")) == true) {
        "Committed terminal seal does not match the exact local Run owner"
    }
    return AndroidCanonicalRunTerminalSeal(descriptor.conversationId, descriptor.submissionId,
        descriptor.assistantMessageId, coreSessionId, result, checkNotNull(resultDigest), checkNotNull(digest))
}

/** Resolve execution ownership from Core, never from the newest visible assistant. */
internal fun loadAndroidRecoveryDescriptor(
    conversationId: String,
    conversationTitle: String,
    expectedRunId: String,
    execute: (JsonObject) -> kotlinx.serialization.json.JsonElement,
): AndroidCanonicalTurnDescriptor {
    require(conversationId.isNotBlank() && expectedRunId.isNotBlank())
    val active = execute(buildJsonObject {
        put("type", "active_run"); put("conversation_id", conversationId)
    }) as? JsonObject ?: error("Conversation has no active Run")
    require(active.string("run_id") == expectedRunId) { "The active recovery Run changed" }
    val assistant = active.string("assistant_message_id")?.takeIf(String::isNotBlank)
        ?: error("The active recovery Run has no admitted assistant owner")
    return AndroidCanonicalTurnDescriptor(
        conversationId = conversationId, conversationTitle = conversationTitle,
        submissionId = expectedRunId, assistantMessageId = assistant,
        // Recovery never submits a new user message; do not synthesize one.
        userMessageId = "", userText = "", userContentJson = null,
    )
}

/**
 * Application-backed implementation of the Host loop transition port.
 *
 * Android presentation code never writes assistant text through this type.
 * Every submission and Host response is reduced and committed atomically by
 * the shared Application layer, which owns the durable conversation view.
 */
class AndroidCanonicalAgentRuntime(
    private val store: AndroidApplicationRuntime,
    val descriptor: AndroidCanonicalTurnDescriptor,
    private val effectLeaseMilliseconds: Long = EFFECT_LEASE_MILLISECONDS,
) : AgentCoreTransitionRuntime {
    internal fun mobileCommand(command: JsonObject): JsonElement = store.execute(command)
    internal fun mobileMcpHost(command: JsonObject): JsonObject = store.mobileMcpHost(command)
    internal fun startEmbeddedPump(input: JsonObject): Long = store.startChildPump(input)
    internal fun embeddedChild(invocation: JsonObject): AndroidCanonicalAgentRuntime = AndroidCanonicalAgentRuntime(store,
        descriptor.copy(conversationId=invocation.getValue("conversation_id").jsonPrimitive.content,
            submissionId=invocation.getValue("run_id").jsonPrimitive.content,
            assistantMessageId=invocation.getValue("assistant_message_id").jsonPrimitive.content,
            userMessageId="",userText="",userContentJson=null,automationLease=null,automationSpec=null,queuedUserAdmission=null))

    fun uiCapabilityContract(): JsonObject = store.execute(buildJsonObject {
        put("type", "read_ui_capability")
    }).jsonObject

    suspend fun publishUiSurface(request: JsonObject, delivery: JsonElement): JsonElement = commandMutex.withLock {
        val effectId = request["effect_id"]?.jsonPrimitive?.content ?: error("UI publication lacks effect identity")
        val requestId = request["request_id"]?.jsonPrimitive?.content ?: error("UI publication lacks request identity")
        store.execute(buildJsonObject {
            put("type", "execute_ui_tool")
            put("input", buildJsonObject {
                put("effect_id", effectId)
                put("request_id", requestId)
                put("delivery", delivery)
                put("now_ms", System.currentTimeMillis())
            })
        })
    }

    suspend fun taskCancellationResponse(request: JsonObject, delivery: JsonElement): JsonObject = commandMutex.withLock {
        executeAndroidTaskCancellation(request, delivery, System.currentTimeMillis()) { store.execute(it) }
    }

    fun executeCompactionCommand(command: JsonObject): JsonObject = store.execute(command).jsonObject

    fun evaluateCompactionMaintenance(facts: JsonObject): JsonObject = store.execute(buildJsonObject {
        put("type", "evaluate_compaction_maintenance"); put("facts", facts)
    }).jsonObject

    init { require(effectLeaseMilliseconds in 1..EFFECT_LEASE_MILLISECONDS) }
    private var coreConfig: JsonObject? = null
    private var invocationConfigDigest: String? = null
    private var started = false
    private var lastControl = JsonObject(emptyMap())
    private var permissionControlOwner: JsonObject? = null
    private var currentPermissions: JsonObject? = null
    private var committedRequests: List<kotlinx.serialization.json.JsonElement> = emptyList()
    private val workerLeaseIdentity by lazy {
        store.execute(buildJsonObject {
            put("type", "encode_host_worker_identity")
            put("identity", AndroidHostProcessObservation.currentWorkerIdentity())
        }).jsonObject
    }
    private val workerGeneration: String get() = workerLeaseIdentity.getValue("worker_generation").jsonPrimitive.content
    private val leaseOwner: String get() = workerLeaseIdentity.getValue("lease_owner").jsonPrimitive.content
    private val commandMutex = Mutex()

    companion object {
        // Core accepts at most a five-minute delivery extension. Keep a margin
        // for clock skew between the platform Host and Application service.
        private const val EFFECT_LEASE_MILLISECONDS = 4 * 60 * 1_000L
    }

    fun configure(config: JsonObject) {
        check(!started) { "Canonical Agent Run is already active" }
        val spec = descriptor.automationSpec
        require((spec == null) == (descriptor.automationLease == null))
        coreConfig = if (spec == null) config else {
            require(config.getValue("host_catalog").jsonObject.getValue("host_id").jsonPrimitive.content == spec.hostId)
            require(descriptor.userText == spec.prompt)
            JsonObject(config + ("automation_execution_spec" to spec.toJson()))
        }
    }

    private fun freshResendAdmissionConfiguration(config: JsonObject, submission: JsonObject): JsonObject {
        val spec = descriptor.automationSpec
        return descriptor.editedResendOperationId?.let { operation ->
            val prepared = checkNotNull(store.loadPreparedEditedResend(descriptor.conversationId))
            prepared.requireIdentity()
            require(prepared.admittedRunId == null && prepared.runId == operation &&
                prepared.userMessageId == descriptor.userMessageId && prepared.assistantMessageId == descriptor.assistantMessageId &&
                prepared.input.editedText == descriptor.userText && descriptor.userContentJson == null)
            require(operation == descriptor.submissionId && spec == null && descriptor.queuedUserAdmission == null)
            require(descriptor.expectedParentMessageId == null && config.getValue("initial_items").jsonArray.isEmpty())
            val proof = store.attestEditedResendHostPolicy(descriptor.conversationId, config, submission)
            require(proof.getValue("operation_id").jsonPrimitive.content == operation &&
                proof.getValue("conversation_id").jsonPrimitive.content == descriptor.conversationId)
            val identity = config.getValue("turn_identity").jsonObject
            JsonObject(config + ("turn_identity" to JsonObject(identity + ("fresh_resend_policy" to proof))))
        } ?: config
    }

    suspend fun applyPendingPermissionPolicy(): JsonObject? = commandMutex.withLock {
        val now = System.currentTimeMillis()
        val terminal = store.execute(buildJsonObject {
            put("type", "apply_pending_task_notice"); put("conversation_id", descriptor.conversationId)
            put("consumer_id", leaseOwner); put("now_ms", now)
        })
        if (terminal != JsonNull) return@withLock transition(terminal.jsonObject, { claimCommittedHostRequests() })
        if (store.execute(buildJsonObject { put("type", "peek_agent_command"); put("session_id", descriptor.conversationId) }) == JsonNull) return@withLock null
        val claimed = store.execute(buildJsonObject {
            put("type", "claim_next_agent_command"); put("session_id", descriptor.conversationId)
            put("lease_owner", leaseOwner); put("now_ms", now); put("lease_until_ms", now + effectLeaseMilliseconds)
        })
        if (claimed == JsonNull) return@withLock null
        val lease = claimed.jsonObject
        require(lease.string("session_id") == descriptor.conversationId && lease.string("lease_owner") == leaseOwner &&
            (lease["lease_until_ms"]?.jsonPrimitive?.content?.toLongOrNull() ?: 0) > now &&
            (lease["delivery_attempt"]?.jsonPrimitive?.content?.toLongOrNull() ?: 0) > 0)
        val command = checkNotNull(lease.string("command_id"))
        fun settle(type: String) { store.execute(buildJsonObject {
            put("type", type); put("session_id", descriptor.conversationId)
            put("command_id", command); put("lease_owner", leaseOwner)
        }) }
        try {
            val notice = kotlinx.serialization.json.Json.parseToJsonElement(checkNotNull(lease.string("payload_json"))).jsonObject
            val submission = notice["submission"] as? JsonObject
            if (notice.string("type") == "submission" && submission?.string("type") == "user_input" &&
                submission.string("submission_id") == command) {
                val submittedAt = checkNotNull(submission["submitted_at_epoch_ms"]?.jsonPrimitive?.content?.toLongOrNull())
                val accepted = store.execute(buildJsonObject {
                    put("type", "reduce_and_commit_active_submission")
                    put("input", buildJsonObject {
                        put("conversation_id", descriptor.conversationId)
                        put("run_id", descriptor.submissionId)
                        put("transition_id", "queued-input:$command")
                        put("invocation_config_digest", checkNotNull(invocationConfigDigest))
                        put("submission", submission)
                        put("committed_at_ms", submittedAt)
                    })
                }).jsonObject
                val result = transition(accepted, { claimCommittedHostRequests() })
                settle("acknowledge_agent_command")
                return@withLock result
            }
            val policy = notice["policy"] as? JsonObject
            val run = policy?.string("run_id")
            val revision = policy?.get("configuration_revision")?.jsonPrimitive?.content?.toLongOrNull()
            if (notice.string("type") != "permission_policy_available" || policy == null || run == null || revision == null || revision < 1 ||
                command != "permission-policy:$run:$revision") {
                settle("release_agent_command_lease"); return@withLock null
            }
            if (run != descriptor.submissionId) {
                val active = store.execute(buildJsonObject {
                    put("type", "active_run"); put("conversation_id", descriptor.conversationId)
                }) as? JsonObject
                if (active?.string("run_id") == run) {
                    settle("release_agent_command_lease"); return@withLock null
                }
            }
            val accepted = store.execute(buildJsonObject {
                put("type", "apply_active_permission_policy"); put("conversation_id", descriptor.conversationId)
                put("policy", policy); put("now_ms", now)
            })
            require(run == descriptor.submissionId || accepted == JsonNull) { "Another Run accepted this owner's policy notice" }
            val result = if (accepted == JsonNull) null else transition(accepted.jsonObject, { claimCommittedHostRequests() })
            settle("acknowledge_agent_command")
            result
        } catch (error: Throwable) {
            runCatching { settle("release_agent_command_lease") }
            throw error
        }
    }

    override suspend fun submit(submission: JsonObject): JsonObject = commandMutex.withLock {
        if (started && submission["type"] == JsonPrimitive("interrupt")) {
            val execution=store.executeQuery(buildJsonObject { put("type","load_host_execution_state"); put("conversation_id",descriptor.conversationId) }).jsonObject
            val control=execution.getValue("control").jsonObject
            if (control["active_turn_id"] == JsonPrimitive(descriptor.submissionId) &&
                control["turn_status"]?.jsonPrimitive?.content in setOf("cancelling","cancelled")) {
                val exact=loadExactRunInvocation()
                require(exact["invocation_config_digest"] == JsonPrimitive(checkNotNull(invocationConfigDigest))) { "Cancellation Run changed" }
                return@withLock transition(buildJsonObject { put("events",JsonArray(emptyList())); put("traces",JsonArray(emptyList())); put("control",control) },
                    { claimCommittedHostRequests() })
            }
        }
        val configured = checkNotNull(coreConfig) { "Canonical Agent Run has no Core configuration" }
        val config = if (!started) freshResendAdmissionConfiguration(configured, submission) else configured
        if (!started) coreConfig = config
        val result = if (!started) {
            require(descriptor.userMessageId.isNotBlank()) { "A recovery descriptor cannot begin a new user submission" }
            started = true
            val ordinaryCommand = buildJsonObject {
                put("type", if (descriptor.automationLease == null) "begin_assistant_turn" else "begin_automation_assistant_turn")
                descriptor.automationLease?.let { put("lease", it.toJson()) }
                put("input", buildJsonObject {
                    put("conversation_id", descriptor.conversationId)
                    put("conversation_title", descriptor.conversationTitle)
                    putNullableLong("expected_configuration_revision", descriptor.expectedConfigurationRevision)
                    putNullableString("expected_parent_message_id", descriptor.expectedParentMessageId)
                    put("user_message_id", descriptor.userMessageId)
                    put("user_part_id", "${descriptor.userMessageId}:text")
                    put("user_text", descriptor.userText)
                    putNullableString("user_content_json", descriptor.userContentJson)
                    put("assistant_message_id", descriptor.assistantMessageId)
                    putNullableString("context_checkpoint_id", descriptor.contextSnapshotId)
                    put("core_config", config)
                    put("submission", submission)
                    put("created_at_ms", descriptor.createdAtMs)
                })
            }
            val queued = descriptor.queuedUserAdmission
            val command = if (queued == null) ordinaryCommand else {
                encodeQueuedUserTurnAdmission(descriptor, config, submission, System.currentTimeMillis())
            }
            val begin = store.execute(command).jsonObject
            invocationConfigDigest = begin["invocation_config_digest"]
                ?.jsonPrimitive
                ?.content
                ?: error("Canonical Agent Run has no invocation config digest")
            transition(begin, { claimCommittedHostRequests() }, admission = true)
        } else if (submission["type"]?.jsonPrimitive?.content == "host_response") {
            val response = submission["response"] ?: error("Host response payload is missing")
            val accepted = store.execute(buildJsonObject {
                put("type", "reduce_and_accept_host_response")
                put("input", buildJsonObject {
                    put("conversation_id", descriptor.conversationId)
                    put("run_id", descriptor.submissionId)
                    put("invocation_config_digest", checkNotNull(invocationConfigDigest))
                    put("response", response)
                    put("received_at_ms", System.currentTimeMillis())
                })
            }).jsonObject
            transition(accepted, { claimCommittedHostRequests() })
        } else {
            val reduced = store.execute(buildJsonObject {
                put("type", "reduce_and_commit_active_submission")
                put("input", buildJsonObject {
                    put("conversation_id", descriptor.conversationId)
                    put("run_id", descriptor.submissionId)
                    put("transition_id", UUID.randomUUID().toString().lowercase())
                    put("invocation_config_digest", checkNotNull(invocationConfigDigest))
                    put("submission", submission)
                    put("committed_at_ms", System.currentTimeMillis())
                })
            }).jsonObject
            transition(reduced, { claimCommittedHostRequests() })
        }
        return result
    }

    suspend fun renewHostDelivery(
        delivery: JsonObject,
        heartbeatAtMs: Long = System.currentTimeMillis(),
    ): JsonObject = commandMutex.withLock {
        val attemptId = delivery["attempt_id"]?.jsonPrimitive?.content
            ?.takeIf(String::isNotBlank)
            ?: error("Host-effect delivery lease has no Attempt ID")
        val generation = delivery["worker_generation"]?.jsonPrimitive?.content
            ?.takeIf(String::isNotBlank)
            ?: error("Host-effect delivery lease has no worker generation")
        val owner = delivery["lease_owner"]?.jsonPrimitive?.content
            ?.takeIf(String::isNotBlank)
            ?: error("Host-effect delivery lease has no owner")
        val token = delivery["lease_token"]?.jsonPrimitive?.content
            ?.takeIf(String::isNotBlank)
            ?: error("Host-effect delivery lease has no token")
        val expiry = delivery["lease_expires_at_ms"]?.jsonPrimitive?.content?.toLongOrNull()
            ?: error("Host-effect delivery lease has no expiry")
        require(generation == workerGeneration && owner == leaseOwner) {
            "Host-effect delivery lease belongs to another Android worker"
        }
        store.execute(buildJsonObject {
            put("type", "heartbeat_effect_delivery")
            put("input", buildJsonObject {
                put("run_id", descriptor.submissionId)
                put("attempt_id", attemptId)
                put("worker_generation", generation)
                put("lease_owner", owner)
                put("lease_token", token)
                put("current_lease_expires_at_ms", expiry)
                put("heartbeat_at_ms", heartbeatAtMs)
                put("requested_lease_expires_at_ms", heartbeatAtMs + EFFECT_LEASE_MILLISECONDS)
            })
        }).jsonObject
    }

    override suspend fun snapshot(): JsonObject = lastControl

    fun committedTerminalSeal(coreSessionId: String): AndroidCanonicalRunTerminalSeal? =
        loadAndroidCanonicalTerminalSeal(descriptor, coreSessionId, store::execute)

    override suspend fun pendingHostRequests(): JsonObject = buildJsonObject {
        put("requests", JsonArray(committedRequests))
    }

    /**
     * Reopens an exact committed local Run after process death or explicit risk
     * authorization. This never authorizes an unknown effect. The
     * old worker keeps its lease until expiry; the new process waits for the
     * Application schedule and then receives a fresh fenced delivery.
     */
    internal fun observeTerminatedCancellationTarget(control: JsonObject): JsonObject {
        val plan = store.execute(buildJsonObject {
            put("type", "inspect_cancellation_process_owners")
            put("identity", buildJsonObject {
                put("conversation_id", descriptor.conversationId)
                put("expected_run_id", descriptor.submissionId)
                put("assistant_message_id", descriptor.assistantMessageId)
            })
            put("control", control)
        }).jsonObject
        val observations = buildJsonArray {
            for (owner in plan.getValue("target_owners").jsonArray) {
                val pid = owner.jsonObject.getValue("pid").jsonPrimitive.int
                add(buildJsonObject {
                    put("owner", owner)
                    put("observation", AndroidHostProcessObservation.observe(pid))
                })
            }
        }
        return store.execute(buildJsonObject {
            put("type", "validate_cancellation_process_observations")
            put("plan", plan)
            put("current_process", AndroidHostProcessObservation.observe(android.os.Process.myPid()))
            put("observations", observations)
        }).jsonObject
    }

    internal fun loadExactRunInvocation(): JsonObject {
        val value = store.execute(buildJsonObject {
            put("type", "load_exact_run_invocation")
            put("conversation_id", descriptor.conversationId)
            put("expected_run_id", descriptor.submissionId)
            put("assistant_message_id", descriptor.assistantMessageId)
        }).jsonObject
        require(value.string("conversation_id") == descriptor.conversationId &&
            value.string("run_id") == descriptor.submissionId &&
            value.string("assistant_message_id") == descriptor.assistantMessageId &&
            value.string("invocation_config_digest")?.matches(Regex("[0-9a-f]{64}")) == true &&
            !value.string("core_session_id").isNullOrBlank() &&
            value["host_invocation"] is JsonObject && value["context_budget"] is JsonObject &&
            value.string("checkpoint_sha256")?.matches(Regex("[0-9a-f]{64}")) == true) {
            "Recovered Run has invalid committed identity"
        }
        return value
    }

    internal suspend fun recoverExistingRun(
        awaitCancellation: suspend (Long) -> Boolean,
        prepareRecovery: suspend (JsonObject) -> Unit,
    ): AndroidRunRecoveryOutcome = commandMutex.withLock {
        coroutineContext.ensureActive()
        check(!started) { "Canonical Agent Run is already active" }
        var frozenForRace: JsonObject? = null
        try {
            var frozen = loadExactRunInvocation()
            frozenForRace = frozen
            loadAndroidRecoveryTerminalIfSealed(descriptor, frozen, frozen, store::execute)?.let {
                return@withLock AndroidRunRecoveryOutcome.Sealed(it)
            }
            val inspection = store.execute(buildJsonObject {
                put("type", "inspect_run_recovery")
                put("conversation_id", descriptor.conversationId)
                put("expected_run_id", descriptor.submissionId)
                put("assistant_message_id", descriptor.assistantMessageId)
            }).jsonObject
            require(inspection.string("conversation_id") == descriptor.conversationId &&
                inspection.string("run_id") == descriptor.submissionId &&
                inspection.string("assistant_message_id") == descriptor.assistantMessageId)
            val observation = inspection.getValue("observation").jsonObject
            require(observation.string("state") == "active" && observation["source_host_id"] == JsonNull) {
                "Only the local authoritative active Run can be resumed"
            }
            require(store.inspectUnknownEffectRecovery(descriptor.conversationId, descriptor.submissionId,
                descriptor.assistantMessageId)?.targets.orEmpty().isEmpty()) { "Unknown effects require an explicit decision" }
            prepareRecovery(frozen)
            val active = store.execute(buildJsonObject {
                put("type", "active_run")
                put("conversation_id", descriptor.conversationId)
            }).let { it as? JsonObject ?: error("Conversation has no active Agent Run") }
            require(active.string("run_id") == descriptor.submissionId) {
                "Active Agent Run does not match the requested recovery Run"
            }
            invocationConfigDigest = active.string("invocation_config_digest")
                ?.takeIf(String::isNotBlank)
                ?: error("Recovered Agent Run has no invocation config digest")
            require(invocationConfigDigest == frozen.string("invocation_config_digest")) { "Recovery Run changed" }

            var cancellationRequested = false
            while (true) {
                coroutineContext.ensureActive()
                if (awaitCancellation(0)) {
                    cancellationRequested = true
                    break
                }
                val now = System.currentTimeMillis()
                val prepared = store.execute(buildJsonObject {
                    put("type", "prepare_run_effect_recovery")
                    put("input", buildJsonObject {
                        put("conversation_id", descriptor.conversationId)
                        put("run_id", descriptor.submissionId)
                        put("invocation_config_digest", checkNotNull(invocationConfigDigest))
                        put("now_ms", now)
                    })
                }).jsonObject.getValue("rebound_count").jsonPrimitive.content.toLong()
                if (prepared > 0) {
                    // Core ResumeHostEffect advances its checkpoint and request IDs.
                    // Re-read the authoritative witness, never replay the old request.
                    val refreshed = loadExactRunInvocation()
                    require(sameAndroidFrozenRunAuthority(frozen, refreshed)) {
                        "Recovery invocation changed while preparing effects"
                    }
                    prepareRecovery(refreshed)
                    frozen = refreshed
                    frozenForRace = refreshed
                }
                val schedule = store.execute(buildJsonObject {
                    put("type", "load_runtime_effect_work_schedule")
                    put("input", buildJsonObject {
                        put("conversation_ids", JsonArray(listOf(JsonPrimitive(descriptor.conversationId))))
                        put("now_ms", now)
                        put("limit", 1)
                    })
                }).jsonObject
                val ready = schedule["runs"]?.jsonArray.orEmpty().mapNotNull { it as? JsonObject }
                    .firstOrNull { it.string("run_id") == descriptor.submissionId }
                if (ready != null) {
                    val reason = ready.string("reason")
                    require(reason in setOf("pending", "retry", "resume", "recover_expired_lease")) { "Unknown recovery work reason" }
                    if (reason != "recover_expired_lease") break
                    // Readiness does not mutate a Leased/ExecutionStarted delivery.
                    // Reconcile only the exact admitted Run, with expiry checked
                    // again transactionally; a renewed lease must remain untouched.
                    prepareRecovery(frozen)
                    coroutineContext.ensureActive()
                    require(loadExactRunInvocation() == frozen) { "Recovery proof changed before expired lease reconciliation" }
                    val interruptedAt = System.currentTimeMillis()
                    store.execute(buildJsonObject {
                        put("type", "recover_expired_run_deliveries")
                        put("conversation_id", descriptor.conversationId)
                        put("expected_run_id", descriptor.submissionId)
                        put("host_invocation", frozen.getValue("host_invocation"))
                        put("interrupted_at", java.time.Instant.ofEpochMilli(interruptedAt).toString())
                        put("interrupted_at_ms", interruptedAt)
                    })
                    require(loadExactRunInvocation() == frozen) { "Recovery proof changed during expired lease reconciliation" }
                    continue // Re-read current work; a heartbeat may have moved its wake time.
                }
                val wakeAt = schedule.string("next_wake_at_ms")?.toLongOrNull()
                    ?: error("Pending Agent Run has no recoverable Host-effect schedule")
                if (reconcileAndroidObservedRun(descriptor, frozen, store::execute,
                        AndroidHostProcessObservation::observe, System::currentTimeMillis)) {
                    continue // Re-read work after the existing Core reconciliation.
                }
                if (awaitCancellation((wakeAt - now).coerceAtLeast(1L))) {
                    cancellationRequested = true
                    break
                }
            }

            val committed = store.execute(buildJsonObject {
                put("type", "load_committed_host_requests")
                put("run_id", descriptor.submissionId)
            }).jsonObject["requests"]?.jsonArray
                ?: error("Recovered Agent Run has no committed Host requests")
            require(committed.isNotEmpty()) { "Recovered Run has no committed work to claim" }

            prepareRecovery(frozen)
            require(loadExactRunInvocation() == frozen) { "Recovery proof changed before claim" }
            val execution = store.execute(buildJsonObject {
                put("type", "load_host_execution_state")
                put("conversation_id", descriptor.conversationId)
            }).jsonObject
            val control = execution.getValue("control").jsonObject
            require(control.string("session_id") == frozen.string("core_session_id")) { "Recovery Core identity changed" }
            val permissions = readControlPermissions(control)
            val tasks = store.execute(buildJsonObject {
                put("type", "load_conversation_task_projection_v1")
                put("conversation_id", descriptor.conversationId)
            }).jsonObject
            require(listOf("core_session_id", "checkpoint_revision", "checkpoint_event_cursor").all { tasks[it] == frozen[it] }) {
                "Recovery Task projection does not match the committed witness"
            }
            val taskProjection = tasks.getValue("task_projection")
            require(decodeAgentCoreTaskProjection(taskProjection, checkNotNull(frozen.string("core_session_id"))) != null) {
                "Recovery Task projection has invalid ownership"
            }
            require(loadExactRunInvocation() == frozen) { "Recovery proof changed while reading control state" }
            coroutineContext.ensureActive()
            // A requested stop restores the exact control witness without claiming
            // the old model work. The coordinator commits Interrupt immediately next.
            val requests = if (cancellationRequested) JsonArray(emptyList()) else claimCommittedHostRequests()
            require(requests.all { claimed ->
                val request = claimed.jsonObject
                committed.any { previous ->
                    val original = previous.jsonObject
                    listOf("effect_id", "request_id", "session_id", "turn_id", "payload").all { key ->
                        request[key] == original[key]
                    }
                }
            }) { "Claimed work differs from the inspected committed work" }
            if (!cancellationRequested) validateRecoveredHostRequests(
                requests = requests,
                runId = descriptor.submissionId,
                sessionId = checkNotNull(frozen.string("core_session_id")),
                workerGeneration = workerGeneration,
                leaseOwner = leaseOwner,
                nowMs = System.currentTimeMillis(),
            )
            coroutineContext.ensureActive()
            prepareRecovery(frozen)
            require(loadExactRunInvocation() == frozen) { "Recovery proof changed after claim" }
            coroutineContext.ensureActive()
            committedRequests = requests
            lastControl = control
            started = true
            AndroidRunRecoveryOutcome.Active(buildJsonObject {
                put("ok", true)
                put("host_requests", requests)
                put("control", control)
                put("permission_projection", permissions)
                put("task_projection", taskProjection)
            })
        } catch (error: Exception) {
            if (error is kotlinx.coroutines.CancellationException) throw error
            val original = frozenForRace ?: throw error
            val current = loadExactRunInvocation()
            val seal = loadAndroidRecoveryTerminalIfSealed(descriptor, original, current, store::execute)
                ?: throw error
            AndroidRunRecoveryOutcome.Sealed(seal)
        }
    }

    private fun claimCommittedHostRequests(): JsonArray {
        val claimedAtMs = System.currentTimeMillis()
        val claimed = store.execute(buildJsonObject {
            put("type", "claim_committed_host_requests")
            put("input", buildJsonObject {
                put("run_id", descriptor.submissionId)
                put("worker_generation", workerGeneration)
                put("lease_owner", leaseOwner)
                put("claimed_at_ms", claimedAtMs)
                put("lease_expires_at_ms", claimedAtMs + effectLeaseMilliseconds)
                put("limit", 64)
            })
        }).jsonObject
        val requests = claimed["requests"]?.jsonArray
            ?: error("Claimed Host requests are missing")
        requests.forEach { element ->
            val request = element as? JsonObject ?: error("Claimed Host request is not an object")
            val delivery = validatedHostDelivery(request, nowMs = claimedAtMs)
            require(delivery.string("worker_generation") == workerGeneration &&
                delivery.string("lease_owner") == leaseOwner) {
                "Claimed Host delivery belongs to another Android worker"
            }
        }
        return requests
    }

    private fun transition(result: JsonObject, loadRequests: () -> JsonArray, admission: Boolean = false): JsonObject {
        validateExecutionReplyEnvelope(result)
        val publications = canonicalAssistantPublications(result)
        val execution = if (admission) result["execution"]?.jsonObject
            ?: error("Admission execution state is missing") else null
        val projection = (execution ?: result)["control"]?.takeUnless { it == JsonNull }?.jsonObject
        require(!admission || projection != null) { "Admission control is missing" }
        val permissions = if (admission) {
            val value = checkNotNull(execution)["permissions"]?.jsonObject
                ?: error("Admission permissions are missing")
            val revision = checkNotNull(projection)["permission_projection_revision"]
            require(revision != null && revision != JsonNull && value["revision"] == revision && value["pending"] is JsonArray) {
                "Admission permissions do not match the control witness"
            }
            value
        } else if (projection != null) readControlPermissions(projection) else null
        val requests = loadRequests()
        projection?.let { lastControl = it }
        committedRequests = requests
        return buildJsonObject {
            put("ok", true)
            put("transition", buildJsonObject {
                projection?.let { put("control", lastControl) }
                permissions?.let { put("permission_projection", it) }
                put("event_envelopes", result.getValue("events"))
                put("trace_events", result.getValue("traces"))
                put("assistant_journal_publications", JsonArray(publications))
                put("host_requests", requests)
            })
        }
    }

    private fun readControlPermissions(control: JsonObject): JsonObject {
        val session = control["session_id"]?.jsonPrimitive?.contentOrNull?.takeIf(String::isNotBlank)
            ?: error("Control has no Core session")
        val turn = control["active_turn_id"] ?: error("Control has no explicit turn identity")
        val revision = control["permission_projection_revision"] ?: error("Control has no permission revision")
        val cursor = control["last_event_cursor"]?.jsonObject?.get("sequence") ?: error("Control has no event cursor")
        val owner = buildJsonObject {
            put("session_id", session); put("active_turn_id", turn); put("permission_projection_revision", revision)
        }
        if (permissionControlOwner == owner && currentPermissions != null) return checkNotNull(currentPermissions)
        val response = store.execute(buildJsonObject {
            put("type", "load_conversation_permission_projection_v1")
            put("conversation_id", descriptor.conversationId); put("run_id", descriptor.submissionId)
            put("scope", buildJsonObject {
                put("type", "control"); owner.forEach { (key,value) -> put(key,value) }; put("event_cursor", cursor)
            })
        }).jsonObject
        val permissions = validateControlPermissionProjection(response, descriptor.conversationId,
            descriptor.submissionId, owner, cursor.jsonPrimitive.content)
        permissionControlOwner = owner
        currentPermissions = permissions
        return permissions
    }
}

internal fun validateControlPermissionProjection(response: JsonObject, conversationId: String,
    runId: String, owner: JsonObject, expectedCursorText: String): JsonObject {
    require(response["conversation_id"] == JsonPrimitive(conversationId) &&
        response["run_id"] == JsonPrimitive(runId) && owner.all { (key,value) -> response[key] == value }) {
        "Permission projection belongs to another control owner"
    }
    val actualCursor = response["event_cursor"]?.jsonPrimitive?.content?.toULongOrNull()
        ?: error("Permission projection has invalid cursor")
    val expectedCursor = expectedCursorText.toULongOrNull() ?: error("Control cursor is invalid")
    require(actualCursor >= expectedCursor) { "Permission projection cursor regressed" }
    val permissions = response["permissions"]?.jsonObject ?: error("Permission projection is missing")
    require(permissions.keys.all { it in setOf("revision", "pending", "active_policy") } &&
        permissions["revision"] == owner["permission_projection_revision"] && permissions["pending"] is JsonArray) {
        "Permission projection revision or payload differs"
    }
    permissions["active_policy"]?.takeUnless { it == JsonNull }?.let { value ->
        val policy = value as? JsonObject ?: error("Invalid active permission policy")
        require(policy.keys == setOf("run_id", "configuration_revision", "profile") &&
            policy["run_id"] == JsonPrimitive(runId) && policy["run_id"] == owner["active_turn_id"] &&
            (policy["configuration_revision"]?.jsonPrimitive?.content?.toLongOrNull() ?: 0) > 0 &&
            policy["profile"]?.jsonPrimitive?.content in setOf("cautious", "allow_all")) {
            "Active permission policy belongs to another Run or invalid configuration"
        }
    }
    return permissions
}

internal fun canonicalControlProjection(transition: JsonObject): JsonObject? {
    require(!transition.containsKey("snapshot") && !transition.containsKey("checkpoint")) {
        "Execution transitions cannot carry cumulative state"
    }
    val value = transition["control"] ?: run {
        require(!transition.containsKey("permission_projection")) { "Permissions have no control owner" }
        return null
    }
    val control = value as? JsonObject ?: error("Invalid execution control")
    val permissions = transition["permission_projection"] as? JsonObject
        ?: error("Execution control has no permission projection")
    val revision = control["permission_projection_revision"]
    require(revision != null && revision != JsonNull && permissions["revision"] == revision && permissions["pending"] is JsonArray) {
        "Execution permissions do not match the control witness"
    }
    return JsonObject(control + ("permissions" to permissions))
}

internal fun validateExecutionReplyEnvelope(result: JsonObject) {
    require(!result.containsKey("snapshot") && !result.containsKey("checkpoint")) {
        "Execution replies cannot carry cumulative state"
    }
    require(result["events"] is JsonArray && result["traces"] is JsonArray) {
        "Execution events or traces are missing or invalid"
    }
}

internal fun canonicalAssistantPublications(result: JsonObject): List<JsonObject> {
    val values = (result["commit"] as? JsonObject)?.get("assistant_publications")
        ?: result["assistant_publications"] ?: JsonArray(emptyList())
    return values.jsonArray.map { it.jsonObject }
}

internal fun validateRecoveredHostRequests(
    requests: JsonArray,
    runId: String,
    sessionId: String,
    workerGeneration: String,
    leaseOwner: String,
    nowMs: Long,
) {
    require(
        runId.isNotBlank() && sessionId.isNotBlank() && workerGeneration.isNotBlank() &&
            leaseOwner.isNotBlank() && requests.isNotEmpty()
    ) {
        "Run recovery has no claimed Host request"
    }
    requests.forEach { element ->
        val request = element as? JsonObject
            ?: error("Run recovery returned a non-object Host request")
        val delivery = validatedHostDelivery(request, nowMs = nowMs)
        require(
            delivery.string("worker_generation") == workerGeneration &&
                delivery.string("lease_owner") == leaseOwner &&
                delivery.string("lease_token")?.isNotBlank() == true &&
                (delivery.string("lease_expires_at_ms")?.toLongOrNull() ?: 0L) > 0L
        ) { "Recovered Host request has an incomplete delivery lease" }
        require(request.string("session_id") == sessionId) {
            "Recovered Host request belongs to another session"
        }
        require(request["payload"] is JsonObject) { "Recovered Host request has no payload" }
    }
}

private fun JsonObject.string(key: String): String? =
    (this[key] as? JsonPrimitive)?.contentOrNull
