package com.nola.data.local

import android.content.Context
import com.nola.core.AgentCoreNative
import com.nola.domain.model.CustomDataSourceConfig
import com.nola.domain.model.DataSourceUsageRecord
import com.nola.domain.model.StructuredMemoryRecord
import com.nola.domain.model.UploadedContent
import com.nola.domain.model.McpServerConfig
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.domain.projection.ConversationProjectionV1Decoder
import com.nola.domain.projection.ConversationProjectionWindowV1
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.io.File
import java.security.MessageDigest
import java.util.UUID
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton

/** Thin Android adapter for the canonical Application command boundary. */
@Singleton
class AndroidApplicationRuntime(
    context: Context,
    val databaseFile: File = defaultDatabaseFile(context),
) : AutoCloseable {
    @Inject
    constructor(@ApplicationContext context: Context) : this(
        context,
        defaultDatabaseFile(context),
    )

    companion object {
        val isAvailable: Boolean get() = AgentCoreNative.isApplicationAvailable

        fun defaultDatabaseFile(context: Context): File =
            File(context.noBackupFilesDir, "nola-product.db")
    }
    internal val expertHostFactory by lazy { AndroidExpertHostFactory(context.applicationContext, this) }
    private val json = Json { ignoreUnknownKeys = true }
    private val lock = Any()
    private var handle: Long
    /** Core owns the worker and all leases; native only services its mailbox. */
    fun startChildPump(input: JsonObject): Long = synchronized(lock) {
        check(handle != 0L) { "Application is closed" }
        AgentCoreNative.nativeChildPumpStart(handle,input.toString().encodeToByteArray()).also {
            check(it != 0L) { "Core child pump did not start" }
        }
    }

    private val submissionJournal = ApplicationSubmissionJournal(File(databaseFile.path + ".pending-commands.json")) { runId ->
        val identity = executeQuery(buildJsonObject {
            put("type", "load_run_identity"); put("run_id", runId)
        }).jsonObject
        require(identity.keys == setOf("run_id", "conversation_id") && identity["run_id"] == JsonPrimitive(runId)) {
            "Run identity does not match the pending command"
        }
        val owner = identity["conversation_id"]?.jsonPrimitive?.takeIf { it.isString }?.content
        require(!owner.isNullOrBlank()) { "Run has no canonical conversation owner" }
        owner
    }
    // Distinct private namespace: ordinary queued renderer JSON is never
    // recovered through the trusted Host transport, even after a crash.
    private val expertHostJournal = ApplicationSubmissionJournal(File(databaseFile.path + ".expert-host-intents.json"), privateCommandScope = ::expertHostOperationScope)
    private val recoveredReceipts = mutableListOf<JsonObject>()

    init {
        check(AgentCoreNative.isApplicationAvailable) { "Nola Application runtime is unavailable" }
        if (databaseFile.absoluteFile == defaultDatabaseFile(context).absoluteFile) {
            secureApplicationDirectory(context.noBackupFilesDir, context.dataDir)
        }
        databaseFile.parentFile?.mkdirs()
        handle = AgentCoreNative.nativeApplicationOpen(databaseFile.absolutePath.encodeToByteArray())
        check(handle != 0L) { "Unable to open Nola Application runtime" }
        try {
            val startupReceipts = retryPendingCommands()
            recoveredReceipts.addAll(startupReceipts)
        } catch (failure: Exception) {
            AgentCoreNative.nativeApplicationFree(handle)
            handle = 0L
            throw failure
        }
    }

    fun readHistoricalPart(owner: JsonObject, revision: Long): ConversationProjectionV1.HistoricalProjection? {
        val result = execute(buildJsonObject {
            put("type", "read_historical_part")
            put("owner", owner)
            put("expected_conversation_revision", revision)
            put("maximum_output_bytes", 32768)
        })
        return if (result == JsonNull) null else ConversationProjectionV1Decoder.decodeHistoricalProjection(result)
    }

    fun execute(command: JsonObject): JsonElement {
        if (ApplicationCommandEffect.isObservation(command)) return executeQuery(command)
        // A queue acknowledgement may be lost after the reducer committed. Use
        // its original Application receipt instead of reducing the input twice.
        val requestId = if (command["type"]?.jsonPrimitive?.content == "reduce_and_commit_active_submission") {
            (command["input"] as? JsonObject)?.get("transition_id")?.jsonPrimitive?.content
                ?.let { "active-submission:$it" }
        } else if (command["type"]?.jsonPrimitive?.content == "begin_queued_user_turn") {
            val input = command.getValue("input").jsonObject
            "queued-admission:${input.getValue("command_id").jsonPrimitive.content}:${input.getValue("delivery_attempt").jsonPrimitive.content}"
        } else null
        return submitMutation(command, requestId)
    }

    internal fun mobileMcpHost(command: JsonObject): JsonObject {
        // Retain the store while I/O runs, without holding its native lock or
        // preventing lease renewal/cancellation on another coroutine.
        val retained = if(command["type"] == JsonPrimitive("cancel")) 0L else synchronized(lock) {
            check(handle != 0L) { "Application is closed" }
            AgentCoreNative.nativeMobileMcpRetain(handle).also { check(it != 0L) }
        }
        try {
            val bytes=checkNotNull(AgentCoreNative.nativeMobileMcpHost(retained,command.toString().encodeToByteArray()))
            val reply=Json.parseToJsonElement(bytes.decodeToString()).jsonObject
            check(reply["ok"]==JsonPrimitive(true)) { reply["error"]?.jsonObject?.get("message")?.jsonPrimitive?.content ?: "MCP Host rejected request" }
            return reply.getValue("result") as? JsonObject ?: JsonObject(emptyMap())
        } finally { if(retained != 0L) AgentCoreNative.nativeApplicationFree(retained) }
    }

    fun executeQuery(command: JsonObject): JsonElement = synchronized(lock) {
        require(ApplicationCommandEffect.isObservation(command)) { "Command is not an explicitly classified query" }
        terminalResult(transmitEnvelope(transportEnvelope(command), readOnly = true))
    }

    /** Only the native fresh Host factory calls this with its actual complete config. */
    internal fun attestEditedResendHostPolicy(target: String, config: JsonObject, submission: JsonObject): JsonObject = synchronized(lock) {
        check(handle != 0L) { "Application is closed" }
        val bytes = checkNotNull(AgentCoreNative.nativeApplicationAttestEditedResendHostPolicy(
            handle, target.encodeToByteArray(), buildJsonObject {
                put("core_config", config); put("submission", submission)
            }.toString().encodeToByteArray())) {
            "Fresh resend policy attestation returned no receipt"
        }
        val envelope = Json.parseToJsonElement(bytes.decodeToString()).jsonObject
        check(envelope["ok"] == JsonPrimitive(true)) {
            envelope["error"]?.jsonObject?.get("message")?.jsonPrimitive?.content
                ?: "Fresh resend policy attestation rejected"
        }
        envelope.getValue("result").jsonObject
    }

    internal open class ExpertHostPending(val requestId: String, val reopenRequired: Boolean = false) :
        IllegalStateException("The original expert request is awaiting confirmation: $requestId")

    internal class ExpertHostRejected(val code: String, message: String) : IllegalStateException("$code: $message")

    internal class ExpertHostBlocked(val originalRequestId: String) :
        IllegalStateException("An earlier native Host request still requires confirmation; this new intent was not sent")

    /** Only a native factory builds these operations, never execute()/renderer JSON. */
    internal data class PendingExpertHostOperation(val requestId: String, val operation: JsonObject)

    internal fun pendingExpertHostOperations(authority: JsonObject): List<PendingExpertHostOperation> = synchronized(lock) {
        expertHostJournal.transaction {
            expertHostJournal.pending().mapNotNull { envelope ->
                require(envelope["protocol_version"] == JsonPrimitive(AgentCoreNative.nativeApplicationProtocolVersion())) {
                    "The pending native Host protocol is incompatible; the original intent was preserved"
                }
                require(envelope.getValue("command").jsonObject.getValue("type") == JsonPrimitive("native_expert_host_operation"))
                val operation = envelope.getValue("command").jsonObject.getValue("arguments").jsonObject.getValue("operation").jsonObject
                val saved = expertHostAuthority(operation)
                if (saved != authority) null else PendingExpertHostOperation(envelope.getValue("request_id").jsonPrimitive.content, operation)
            }
        }
    }

    private fun expertHostAuthority(operation: JsonObject): JsonElement? =
        if (operation["kind"] == JsonPrimitive("admit")) operation.getValue("context").jsonObject.let { capture ->
            buildJsonObject { put("actor", capture.getValue("actor")); put("request_host_id", capture.getValue("request_host_id")) }
        } else operation["authority"]

    internal fun pendingExpertHostMutation(kind: String, input: JsonElement, authority: JsonObject): JsonObject? = synchronized(lock) {
        require(kind in setOf("admit", "prepare_edited_resend"))
        pendingExpertHostOperations(authority).filter {
            it.operation["kind"] == JsonPrimitive(kind) && it.operation["input"] == input
        }.also { require(it.size <= 1) { "The original Host intent is not unique" } }.singleOrNull()?.operation
    }

    internal fun executeExpertHostMutation(operation: JsonObject): JsonElement = synchronized(lock) {
        require(operation["kind"]?.jsonPrimitive?.content in setOf("admit", "prepare_edited_resend"))
        check(handle != 0L) { "Application is closed" }
        var activeRequest: JsonObject? = null
        try {
            expertHostJournal.transaction {
                // Close the preview→send race: a concurrent native factory may
                // have durably prepared this same business intent meanwhile.
                val authority = checkNotNull(expertHostAuthority(operation)).jsonObject
                val actualOperation = pendingExpertHostOperations(authority).filter {
                    it.operation["kind"] == operation["kind"] && it.operation["input"] == operation["input"]
                }.also { require(it.size <= 1) { "The original Host intent is not unique" } }.singleOrNull()?.operation ?: operation
                val request = expertHostJournal.prepare(buildJsonObject {
                    put("type", "native_expert_host_operation"); put("operation", actualOperation)
                }, AgentCoreNative.nativeApplicationProtocolVersion(), null, null)
                activeRequest = request
                val wire = buildJsonObject { put("request_id", request.getValue("request_id")); put("operation", actualOperation) }
                val response = try { AgentCoreNative.nativeApplicationExpertHostOperation(handle, wire.toString().encodeToByteArray()) }
                    catch (_: Exception) { throw ApplicationSubmissionJournal.PendingSubmission(request.getValue("request_id").jsonPrimitive.content,
                        "Native Host transport outcome is unavailable") }
                expertHostResult(expertHostJournal.acceptExpertHostResponse(request, response) { result ->
                    when (actualOperation.getValue("kind").jsonPrimitive.content) {
                        "admit" -> {
                            val manifest = Json.decodeFromJsonElement<AndroidExpertMessageManifest>(result)
                            require(Json.encodeToJsonElement(manifest.input) == actualOperation["input"] &&
                                manifest.entries.map { it.participant } == manifest.input.selectedParticipants)
                        }
                        "prepare_edited_resend" -> {
                            val prepared = Json.decodeFromJsonElement<AndroidPreparedExpertResend>(result)
                            require(Json.encodeToJsonElement(prepared.input) == actualOperation["input"] && prepared.entries.isNotEmpty())
                            prepared.entries.forEach { it.prepared.requireIdentity() }
                        }
                        else -> error("Unknown private mutation")
                    }
                }, definitiveMutation = true)
            }
        } catch (pending: ApplicationSubmissionJournal.PendingSubmission) {
            if (activeRequest == null) throw ExpertHostBlocked(pending.requestId)
            throw ExpertHostPending(pending.requestId)
        }
    }

    internal fun executeExpertHostQuery(operation: JsonObject): JsonElement = synchronized(lock) {
        require(operation["kind"]?.jsonPrimitive?.content in
            setOf("load_manifest", "load_prepared_resend", "load_child_instructions", "readable_templates", "preview_capture"))
        check(handle != 0L) { "Application is closed" }
        val requestId = UUID.randomUUID().toString()
        val wire = buildJsonObject { put("request_id", requestId); put("operation", operation) }
        val response = checkNotNull(AgentCoreNative.nativeApplicationExpertHostOperation(handle, wire.toString().encodeToByteArray())) {
            "Expert Host query is unavailable"
        }
        val receipt = Json.parseToJsonElement(response.decodeToString(throwOnInvalidSequence = true)).jsonObject
        val returnedId = if (receipt["ok"] == JsonPrimitive(true)) receipt["request_id"] else
            (receipt["error"] as? JsonObject)?.get("request_id")
        require(returnedId == JsonPrimitive(requestId)) { "Expert Host query receipt owner changed" }
        expertHostResult(receipt)
    }

    private fun expertHostResult(receipt: JsonObject, definitiveMutation: Boolean = false): JsonElement {
        if (receipt["ok"] == JsonPrimitive(true)) return receipt.getValue("result")
        val error = receipt.getValue("error").jsonObject
        if (error["outcome_pending"] == JsonPrimitive(true))
            throw ExpertHostPending(error.getValue("request_id").jsonPrimitive.content,
                error["reopen_required"] == JsonPrimitive(true))
        val code = error.getValue("code").jsonPrimitive.content
        val message = error.getValue("message").jsonPrimitive.content
        if (definitiveMutation) throw ExpertHostRejected(code, message)
        throw RejectedMutation("$code: $message")
    }

    private fun transportEnvelope(command: JsonObject): JsonObject = buildJsonObject {
        put("protocol_version", AgentCoreNative.nativeApplicationProtocolVersion())
        put("client_id", submissionJournal.clientId)
        put("request_id", UUID.randomUUID().toString())
        put("command", buildJsonObject {
            put("type", command.getValue("type"))
            put("arguments", JsonObject(command.filterKeys { it != "type" }))
        })
    }

    class RejectedMutation(message: String) : IllegalStateException(message)

    class PendingMutation(val requestId: String) : IllegalStateException("Command outcome pending: $requestId")

    private fun <T> durableTransaction(block: () -> T): T = synchronized(lock) {
        submissionJournal.transaction(block)
    }

    /** Replays exact durable envelopes; accepted/unknown outcomes remain available for later retry. */
    fun retryPendingCommands(): List<JsonObject> = durableTransaction {
        check(handle != 0L) { "Nola Application runtime is closed" }
        submissionJournal.pending().forEach { request ->
            requireCurrentEnvelope(request)
            val wire = request.getValue("command").jsonObject
            val command = JsonObject(wire.getValue("arguments").jsonObject + ("type" to wire.getValue("type")))
            if (ApplicationCommandEffect.isObservation(command)) {
                submissionJournal.retireObservation(request)
                return@forEach
            }
            if (unknownEffectIntent(request) != null) return@forEach
            try {
                recoveredReceipts += acceptSubmission(request, recoverEnvelope(request))
            } catch (_: PendingMutation) {
                // A later retry must use this same client, request ID, and command.
            }
        }
        recoveredReceipts.toList().also { recoveredReceipts.clear() }
    }

    fun submitMutation(command: JsonObject, requestId: String? = null): JsonElement = durableTransaction {
        check(handle != 0L) { "Nola Application runtime is closed" }
        val pendingIds = submissionJournal.pending().map { it["request_id"] }.toSet()
        val request = prepareSubmission(command, requestId, null)
        terminalResult(acceptSubmission(request,
            if (request["request_id"] in pendingIds) recoverEnvelope(request) else transmitEnvelope(request)))
    }

    private fun prepareSubmission(command: JsonObject, requestId: String?, expectedRevision: Long?): JsonObject = try {
        check(handle != 0L) { "Nola Application runtime is closed" }
        submissionJournal.prepare(command, AgentCoreNative.nativeApplicationProtocolVersion(), requestId, expectedRevision)
    } catch (pending: ApplicationSubmissionJournal.PendingSubmission) {
        throw PendingMutation(pending.requestId)
    }

    private fun transmitEnvelope(request: JsonObject, readOnly: Boolean = false): JsonObject {
        check(handle != 0L) { "Nola Application runtime is closed" }
        requireCurrentEnvelope(request)
        val response = try {
            AgentCoreNative.nativeApplicationSubmit(handle, request.toString().encodeToByteArray())
        } catch (_: Exception) {
            throw PendingMutation(request.getValue("request_id").jsonPrimitive.content)
        }
        return try {
            ApplicationReceiptValidation.decode(request, response, readOnly)
        } catch (pending: ApplicationSubmissionJournal.PendingSubmission) {
            throw PendingMutation(pending.requestId)
        }
    }

    private fun acceptSubmission(request: JsonObject, receipt: JsonObject): JsonObject = try {
        submissionJournal.acceptReceipt(request, receipt) { validated ->
            val intent = unknownEffectIntent(request)
            if (intent != null && validated["status"]?.jsonPrimitive?.content == "committed") {
                Json.decodeFromJsonElement<UnknownEffectRecoveryReply>(validated.getValue("result")).validate(intent.decision)
            }
        }
    } catch (pending: ApplicationSubmissionJournal.PendingSubmission) { throw PendingMutation(pending.requestId) }

    private fun recoverEnvelope(request: JsonObject): JsonObject {
        requireCurrentEnvelope(request)
        val lookup = transmitEnvelope(transportEnvelope(buildJsonObject {
            put("type", "command_receipt"); put("request_id", request.getValue("request_id"))
        }))
        if (lookup["status"]?.jsonPrimitive?.content != "committed") {
            throw PendingMutation(request.getValue("request_id").jsonPrimitive.content)
        }
        val result = lookup.getValue("result")
        if (result == JsonNull) return transmitEnvelope(request)
        val receipt = result as? JsonObject ?: throw PendingMutation(request.getValue("request_id").jsonPrimitive.content)
        return ApplicationReceiptValidation.recovered(request, receipt)
    }

    private fun unknownEffectIntent(request: JsonObject): PendingUnknownEffectRecovery? {
        requireCurrentEnvelope(request)
        val command = request.getValue("command").jsonObject
        if (command["type"]?.jsonPrimitive?.content !in setOf("resolve_unknown_effect_recovery", "resolve_owned_task_recovery")) return null
        val decision = Json.decodeFromJsonElement<UnknownEffectRecoveryDecision>(
            command.getValue("arguments").jsonObject.getValue("input"))
        decision.validate()
        return PendingUnknownEffectRecovery(request.getValue("request_id").jsonPrimitive.content, decision)
    }

    private fun requireCurrentEnvelope(request: JsonObject) {
        require(request["protocol_version"] == kotlinx.serialization.json.JsonPrimitive(
            AgentCoreNative.nativeApplicationProtocolVersion())) {
            "Pending Application command uses an incompatible protocol; the original intent was preserved"
        }
        require(request["client_id"] == kotlinx.serialization.json.JsonPrimitive(submissionJournal.clientId)) {
            "Pending Application command belongs to another client"
        }
    }

    fun inspectUnknownEffectRecovery(conversationId: String, runId: String, assistantMessageId: String): UnknownEffectRecoveryInspection? {
        val result = executeQuery(buildJsonObject {
            put("type", "inspect_unknown_effect_recovery"); put("conversation_id", conversationId)
            put("expected_run_id", runId); put("assistant_message_id", assistantMessageId)
        })
        return if (result == JsonNull) null else Json.decodeFromJsonElement<UnknownEffectRecoveryInspection>(result)
            .also { it.validate(conversationId, runId, assistantMessageId) }
    }

    fun submitUnknownEffectDecision(decision: UnknownEffectRecoveryDecision, requestId: String): UnknownEffectRecoveryReply {
        decision.validate()
        val result = submitMutation(buildJsonObject {
            put("type", "resolve_unknown_effect_recovery"); put("input", Json.encodeToJsonElement(decision))
        }, requestId)
        return Json.decodeFromJsonElement<UnknownEffectRecoveryReply>(result).also { it.validate(decision) }
    }

    fun pendingConversationCommand(conversationId: String): String? = durableTransaction {
        submissionJournal.pending().firstOrNull {
            val scope = ApplicationCommandScope.envelope(it)
            scope == ApplicationCommandScope.Global || scope == ApplicationCommandScope.Conversation(conversationId)
        }?.getValue("request_id")?.jsonPrimitive?.content
    }

    fun pendingOwnedTaskDecision(owner: JsonObject): PendingUnknownEffectRecovery? = durableTransaction {
        submissionJournal.pending().singleOrNull {
            val command = it.getValue("command").jsonObject
            command["type"] == JsonPrimitive("resolve_owned_task_recovery") &&
                command.getValue("arguments").jsonObject["owner"] == owner
        }?.let(::unknownEffectIntent)
    }

    fun pendingUnknownEffectDecision(conversationId: String): PendingUnknownEffectRecovery? = durableTransaction {
        submissionJournal.pending().mapNotNull(::unknownEffectIntent).singleOrNull { it.decision.conversationId == conversationId }
    }

    fun recoverPendingUnknownEffectDecision(expected: PendingUnknownEffectRecovery): UnknownEffectRecoveryReply = durableTransaction {
        val request = submissionJournal.pending().singleOrNull { it["request_id"] == JsonPrimitive(expected.requestId) }
            ?: throw PendingMutation(expected.requestId)
        require(unknownEffectIntent(request) == expected) { "Recovery intent owner changed" }
        val result = terminalResult(acceptSubmission(request, recoverEnvelope(request)))
        Json.decodeFromJsonElement<UnknownEffectRecoveryReply>(result).also { it.validate(expected.decision) }
    }

    private fun terminalResult(receipt: JsonObject): JsonElement = when (receipt.getValue("status").jsonPrimitive.content) {
        "committed" -> receipt.getValue("result")
        "rejected" -> throw RejectedMutation(receipt.getValue("error").jsonObject["message"]?.jsonPrimitive?.content ?: "Command rejected")
        "conflict" -> {
            val conflict = receipt.getValue("conflict").jsonObject
            throw RevisionConflict(conflict.getValue("scope_id").jsonPrimitive.content,
                conflict.getValue("expected_revision").jsonPrimitive.content.toLong(),
                conflict.getValue("actual_revision").jsonPrimitive.content.toLong())
        }
        else -> error("Invalid terminal command receipt")
    }

    fun putProductSetting(key: String, valueJson: String, updatedAtMs: Long): CanonicalProductSetting =
        submitMutation(buildJsonObject {
            put("type", "put_product_setting")
            put("key", key)
            put("value_json", valueJson)
            put("updated_at_ms", updatedAtMs)
        }).jsonObject.toCanonicalProductSetting()

    fun productSetting(key: String): CanonicalProductSetting? {
        val result = execute(buildJsonObject {
            put("type", "get_product_setting")
            put("key", key)
        })
        return result.takeUnless { it.toString() == "null" }?.jsonObject?.toCanonicalProductSetting()
    }

    fun deleteProductSetting(key: String): Boolean = submitMutation(buildJsonObject {
        put("type", "delete_product_setting")
        put("key", key)
    }).jsonPrimitive.content.toBoolean()

    fun upsertModelProfile(profile: CanonicalModelProfile) {
        execute(buildJsonObject {
            put("type", "model")
            put("command", buildJsonObject {
                put("type", "upsert_profile")
                put("input", json.encodeToJsonElement(CanonicalModelProfile.serializer(), profile))
            })
        })
    }

    fun modelProfile(id: String): CanonicalModelProfile? {
        val result = execute(buildJsonObject {
            put("type", "model")
            put("command", buildJsonObject {
                put("type", "get_profile")
                put("id", id)
            })
        })
        return result.takeUnless { it.toString() == "null" }
            ?.let { json.decodeFromJsonElement(CanonicalModelProfile.serializer(), it) }
    }

    fun resetModelParameters(profileIds: List<String>, parametersJson: String, updatedAtMs: Long) {
        execute(buildJsonObject {
            put("type", "model")
            put("command", buildJsonObject {
                put("type", "reset_parameters")
                put("profile_ids", JsonArray(profileIds.map { kotlinx.serialization.json.JsonPrimitive(it) }))
                put("parameters_json", parametersJson)
                put("updated_at_ms", updatedAtMs)
            })
        })
    }

    class RevisionConflict(val scope: String, val expected: Long, val actual: Long) :
        IllegalStateException("Revision conflict for $scope: expected $expected, actual $actual")

    fun configureModelParameters(
        profile: CanonicalModelProfile,
        expectedRevision: Long,
        requestId: String? = null,
    ): Long = durableTransaction {
        require(expectedRevision >= 0)
        val command = buildJsonObject {
            put("type", "model")
            put("command", buildJsonObject {
                put("type", "configure_parameters")
                put("input", json.encodeToJsonElement(CanonicalModelProfile.serializer(), profile))
            })
        }
        val alreadyPending = submissionJournal.pending().isNotEmpty()
        val request = prepareSubmission(command, requestId, expectedRevision)
        val receipt = acceptSubmission(request,
            if (alreadyPending) recoverEnvelope(request) else transmitEnvelope(request))
        terminalResult(receipt)
        receipt.getValue("committed_revision").jsonPrimitive.content.toLong().also {
            check(it >= 0) { "Invalid committed revision" }
        }
    }

    fun replaceModelProfiles(
        provider: String, runtime: String, profiles: List<CanonicalModelProfile>,
        expectedRevisions: Map<String, Long>, updatedAtMs: Long,
    ) {
        execute(buildJsonObject {
            put("type", "model")
            put("command", buildJsonObject {
                put("type", "replace_profiles")
                put("provider", provider)
                put("runtime", runtime)
                put("expected_revisions", JsonObject(expectedRevisions.mapValues { (_, revision) ->
                    kotlinx.serialization.json.JsonPrimitive(revision)
                }))
                put("profiles", JsonArray(profiles.map {
                    json.encodeToJsonElement(CanonicalModelProfile.serializer(), it)
                }))
                put("updated_at_ms", updatedAtMs)
            })
        })
    }

    fun listModelProfiles(): List<CanonicalModelProfile> = execute(buildJsonObject {
        put("type", "model")
        put("command", buildJsonObject { put("type", "list_profiles") })
    }).jsonArray.map { json.decodeFromJsonElement(CanonicalModelProfile.serializer(), it) }

    fun deleteModelProfile(id: String, deletedAtMs: Long): Boolean = execute(buildJsonObject {
        put("type", "model")
        put("command", buildJsonObject {
            put("type", "delete_profile")
            put("id", id)
            put("deleted_at_ms", deletedAtMs)
        })
    }).jsonPrimitive.content.toBoolean()

    fun pendingSyncChanges(limit: Int = 100): List<CanonicalSyncChange> = execute(buildJsonObject {
        put("type", "pending_sync_changes")
        put("limit", limit.coerceAtLeast(1))
    }).jsonArray.map { value -> value.jsonObject.toCanonicalSyncChange() }

    fun resolveSyncChange(operationId: String, status: String, remoteCursor: Long?, resolvedAtMs: Long): Boolean =
        execute(buildJsonObject {
            put("type", "resolve_sync_change")
            put("operation_id", operationId)
            put("status", status)
            put("remote_cursor", remoteCursor)
            put("resolved_at_ms", resolvedAtMs)
        }).jsonPrimitive.content.toBoolean()

    fun syncCursor(replicaId: String): CanonicalSyncCursor? {
        val result = execute(buildJsonObject {
            put("type", "get_sync_cursor")
            put("replica_id", replicaId)
        })
        return result.takeUnless { it.toString() == "null" }?.jsonObject?.toCanonicalSyncCursor()
    }

    fun saveSyncCursor(replicaId: String, cursor: Long, updatedAtMs: Long): CanonicalSyncCursor =
        execute(buildJsonObject {
            put("type", "save_sync_cursor")
            put("replica_id", replicaId)
            put("cursor", cursor)
            put("updated_at_ms", updatedAtMs)
        }).jsonObject.toCanonicalSyncCursor()

    fun applyRemoteSyncPage(
        replicaId: String,
        changes: List<CanonicalRemoteSyncChange>,
        nextCursor: Long,
        appliedAtMs: Long,
    ): CanonicalRemoteSyncPageResult = execute(buildJsonObject {
        put("type", "apply_remote_sync_page")
        put("input", buildJsonObject {
            put("replica_id", replicaId)
            put("changes", JsonArray(changes.map { change ->
                buildJsonObject {
                    put("cursor", change.cursor)
                    put("operation_id", change.operationId)
                    put("entity_type", change.entityType)
                    put("entity_id", change.entityId)
                    put("revision", change.revision)
                    put("deleted", change.deleted)
                    put("payload_json", change.payloadJson)
                    put("created_at_ms", change.createdAtMs)
                }
            }))
            put("next_cursor", nextCursor)
            put("applied_at_ms", appliedAtMs)
        })
    }).jsonObject.toCanonicalRemoteSyncPageResult()

    private val dataSources by lazy {
        AndroidCanonicalDataSources(CanonicalDataSourceClient(::execute),
            File(databaseFile.parentFile, "data-source-content"))
    }

    fun saveDataSource(source: CustomDataSourceConfig) = dataSources.save(source)
    suspend fun refreshDataSource(source: CustomDataSourceConfig, read: suspend () -> CustomDataSourceConfig): CustomDataSourceConfig =
        dataSources.refresh(source, read)
    fun listDataSources(): List<CustomDataSourceConfig> = dataSources.list()
    fun deleteDataSource(id: String) = dataSources.delete(id)
    fun setDataSourceEnabled(id: String, enabled: Boolean) = dataSources.setEnabled(id, enabled)

    fun appendDataSourceUsage(record: DataSourceUsageRecord) {
        execute(buildJsonObject {
            put("type", "data_source")
            put("command", buildJsonObject {
                put("type", "record_usage")
                put("input", buildJsonObject {
                put("id", record.id)
                put("source_id", record.sourceId)
                put("source_title", record.sourceTitle)
                put("consumer", record.consumer)
                put("purpose", record.purpose)
                put("conversation_id", record.conversationId)
                put("disclosed_to_cloud", record.disclosedToCloud)
                put("metadata", buildJsonObject {})
                    put("used_at_ms", record.usedAt)
                })
            })
        })
    }

    fun listDataSourceUsage(limit: Int = 200): List<DataSourceUsageRecord> = execute(
        buildJsonObject {
            put("type", "data_source")
            put("command", buildJsonObject {
                put("type", "list_usage")
                put("limit", limit)
            })
        }
    ).jsonArray.map { value ->
        val record = value.jsonObject
        DataSourceUsageRecord(
                id = record.getValue("id").jsonPrimitive.content,
                sourceId = record.getValue("source_id").jsonPrimitive.content,
                sourceTitle = record.getValue("source_title").jsonPrimitive.content,
                consumer = record.getValue("consumer").jsonPrimitive.content,
                purpose = record.getValue("purpose").jsonPrimitive.content,
                conversationId = record["conversation_id"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content,
                disclosedToCloud = record["disclosed_to_cloud"]?.takeUnless { it == JsonNull }
                    ?.jsonPrimitive?.content?.toBooleanStrict() ?: false,
                usedAt = record.getValue("used_at_ms").jsonPrimitive.content.toLong(),
        )
    }

    fun recallProfileMemories(query: String, limit: Int): List<StructuredMemoryRecord> =
        listOf("personal", "long_term", "user_preferences").flatMap { namespace ->
            execute(buildJsonObject {
                put("type", "memory")
                put("command", buildJsonObject {
                    put("type", "recall")
                    put("input", buildJsonObject {
                        put("binding", buildJsonObject { put("profile_id", "local") })
                        put("namespace", buildJsonObject {
                            put("kind", if (namespace == "personal") "personal" else "custom")
                            if (namespace != "personal") put("name", namespace)
                        })
                        put("scope", "global")
                        put("query", query)
                        put("kind", JsonNull)
                        put("now_iso", java.time.Instant.now().toString())
                        put("limit", limit.coerceIn(1, 50))
                    })
                })
            }).jsonArray.map(::decodeMemory)
        }

    private fun memoryCommitCommand(record: StructuredMemoryRecord): JsonObject {
        val intent = when (record.provenanceKind) {
            "explicit_user" -> "explicit_user"
            "imported" -> "imported"
            "synced" -> "synced"
            else -> error("Automatic memory requires a Core quality assessment")
        }
        return buildJsonObject {
            put("type", "memory")
            put("command", buildJsonObject {
                put("type", "commit")
                put("intent", intent)
                put("input", buildJsonObject {
                    put("proposed_at_ms", record.updatedAt)
                    put("quality", JsonNull)
                    put("record", buildJsonObject {
                        put("id", record.id)
                        put("owner", record.owner)
                        put("namespace", buildJsonObject {
                            if (record.namespace in setOf("personal", "workspace", "conversation")) {
                                put("kind", record.namespace)
                            } else {
                                put("kind", "custom")
                                put("name", record.namespace)
                            }
                        })
                        put("scope", record.scope)
                        put("kind", record.kind)
                        put("content", record.content)
                        put("tags", json.parseToJsonElement(json.encodeToString(record.tags)))
                        put("confidence", record.confidence)
                        put("importance", record.confidence)
                        record.sensitivity?.let { put("sensitivity", it) }
                        put("supersedes", json.parseToJsonElement(json.encodeToString(record.supersedes)))
                        put("retention", buildJsonObject {
                            put("mode", record.retention)
                            if (record.retention == "expires_at") {
                                put("expires_at", java.time.Instant.ofEpochMilli(requireNotNull(record.expiresAt)).toString())
                            }
                        })
                        put("provenance", buildJsonObject {
                            put("kind", record.provenanceKind)
                            put("source", requireNotNull(record.sourceReference) { "Memory source is required" })
                            put("observed_at", record.observedAt?.let { java.time.Instant.ofEpochMilli(it).toString() })
                        })
                    })
                })
            })
        }
    }

    fun upsertMemory(record: StructuredMemoryRecord): StructuredMemoryRecord =
        decodeMemory(submitMutation(memoryCommitCommand(record)))

    data class MemorySnapshot(
        val canonical: JsonArray,
        val records: List<StructuredMemoryRecord>
    )

    fun memorySnapshot(limit: Int = 1000): MemorySnapshot {
        val canonical = execute(buildJsonObject {
            put("type", "memory")
            put("command", buildJsonObject {
                put("type", "list")
                put("limit", limit)
            })
        }).jsonArray
        return MemorySnapshot(canonical, canonical.map(::decodeMemory))
    }

    fun listMemories(limit: Int = 1000): List<StructuredMemoryRecord> = memorySnapshot(limit).records

    fun replaceMemories(expected: JsonArray, records: List<StructuredMemoryRecord>): MemorySnapshot {
        val existing = expected.associateBy { it.jsonObject.getValue("record").jsonObject.getValue("id").jsonPrimitive.content }
        val candidates = records.map { record ->
            val original = existing[record.id]
            if (original != null && decodeMemory(original) == record) {
                buildJsonObject {
                    put("record", original.jsonObject.getValue("record"))
                    put("quality", JsonNull)
                    put("proposed_at_ms", record.updatedAt)
                }
            } else {
                memoryCommitCommand(record).getValue("command").jsonObject.getValue("input")
            }
        }
        val committed = submitMutation(buildJsonObject {
            put("type", "memory")
            put("command", buildJsonObject {
                put("type", "replace")
                put("expected", expected)
                put("candidates", JsonArray(candidates))
                put("intent", "explicit_user")
                put("deleted_at_ms", System.currentTimeMillis())
            })
        }).jsonArray
        return MemorySnapshot(committed, committed.map(::decodeMemory))
    }

    private fun decodeMemory(value: JsonElement): StructuredMemoryRecord {
        val envelope = value.jsonObject
        val record = envelope.getValue("record").jsonObject
        val namespace = record.getValue("namespace").jsonObject
        val provenance = record.getValue("provenance").jsonObject
        val retention = record.getValue("retention").jsonObject
        fun instant(value: JsonElement?): Long? = value?.takeUnless { it == JsonNull }?.let {
            java.time.Instant.parse(it.jsonPrimitive.content).toEpochMilli()
        }
        return StructuredMemoryRecord(
            id = record.getValue("id").jsonPrimitive.content,
            owner = record.getValue("owner").jsonObject,
            namespace = if (namespace.getValue("kind").jsonPrimitive.content == "custom")
                namespace.getValue("name").jsonPrimitive.content else namespace.getValue("kind").jsonPrimitive.content,
            scope = record.getValue("scope").jsonPrimitive.content,
            kind = record.getValue("kind").jsonPrimitive.content,
            content = record.getValue("content").jsonPrimitive.content,
            confidence = record.getValue("confidence").jsonPrimitive.content.toDouble(),
            sensitivity = record["sensitivity"]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content,
            provenanceKind = provenance.getValue("kind").jsonPrimitive.content,
            sourceReference = provenance.getValue("source").jsonPrimitive.content,
            observedAt = instant(provenance["observed_at"]),
            createdAt = envelope.getValue("created_at_ms").jsonPrimitive.content.toLong(),
            updatedAt = envelope.getValue("updated_at_ms").jsonPrimitive.content.toLong(),
            retention = retention.getValue("mode").jsonPrimitive.content,
            expiresAt = instant(retention["expires_at"]),
            tags = record.getValue("tags").jsonArray.map { it.jsonPrimitive.content },
            supersedes = record.getValue("supersedes").jsonArray.map { it.jsonPrimitive.content },
        )
    }

    fun deleteMemory(id: String, deletedAtMs: Long = System.currentTimeMillis()) {
        submitMutation(buildJsonObject {
            put("type", "memory")
            put("command", buildJsonObject {
                put("type", "delete")
                put("id", id)
                put("deleted_at_ms", deletedAtMs)
            })
        })
    }

    data class SkillEditorSnapshot(val record: JsonObject, val revision: Long)

    fun skillEditor(id: String): SkillEditorSnapshot? {
        val value = execute(buildJsonObject {
            put("type", "skill")
            put("command", buildJsonObject { put("type", "get_editor"); put("id", id) })
        })
        return if (value == JsonNull) null else decodeSkillEditor(value)
    }

    private fun decodeSkillEditor(value: kotlinx.serialization.json.JsonElement): SkillEditorSnapshot {
        val envelope = value.jsonObject
        return SkillEditorSnapshot(
            envelope.getValue("record").jsonObject,
            envelope.getValue("revision").jsonPrimitive.content.toLong().also { require(it > 0) }
        )
    }

    fun configureUserSkill(original: SkillEditorSnapshot, name: String, content: String): SkillEditorSnapshot {
        require(name.isNotBlank() && content.isNotBlank())
        val digest = MessageDigest.getInstance("SHA-256").digest(content.encodeToByteArray())
            .joinToString("") { "%02x".format(it) }
        // Preserve the complete canonical record, including non-builtin tool references.
        val input = JsonObject(original.record.toMutableMap().apply {
            remove("deleted_at_ms")
            put("display_name", kotlinx.serialization.json.JsonPrimitive(name))
            put("instructions_text", kotlinx.serialization.json.JsonPrimitive(content))
            put("instructions_digest", kotlinx.serialization.json.JsonPrimitive(digest))
            if (original.record.getValue("instructions_text").jsonPrimitive.content != content) {
                put("version", kotlinx.serialization.json.JsonPrimitive(UUID.randomUUID().toString()))
            }
            put("updated_at_ms", kotlinx.serialization.json.JsonPrimitive(System.currentTimeMillis()))
        })
        return decodeSkillEditor(submitMutation(buildJsonObject {
            put("type", "skill")
            put("command", buildJsonObject {
                put("type", "configure_user"); put("input", input)
                put("expected_revision", original.revision)
            })
        }))
    }

    fun setSkillEnabled(original: SkillEditorSnapshot, enabled: Boolean): SkillEditorSnapshot =
        decodeSkillEditor(submitMutation(buildJsonObject {
            put("type", "skill")
            put("command", buildJsonObject {
                put("type", "set_enabled_checked")
                put("id", original.record.getValue("id"))
                put("enabled", enabled); put("expected_revision", original.revision)
                put("updated_at_ms", System.currentTimeMillis())
            })
        }))

    fun deleteUserSkill(original: SkillEditorSnapshot) {
        submitMutation(buildJsonObject {
            put("type", "skill")
            put("command", buildJsonObject {
                put("type", "delete_user_checked")
                put("id", original.record.getValue("id"))
                put("expected_revision", original.revision)
                put("deleted_at_ms", System.currentTimeMillis())
            })
        })
    }

    fun createUserSkill(skill: UploadedContent): SkillEditorSnapshot =
        decodeSkillEditor(writeNewSkill(skill))

    private fun writeNewSkill(skill: UploadedContent): kotlinx.serialization.json.JsonElement {
        val digest = MessageDigest.getInstance("SHA-256").digest(skill.content.encodeToByteArray())
            .joinToString("") { "%02x".format(it) }
        val id = skill.resourceId ?: UUID.randomUUID().toString()
        val now = System.currentTimeMillis()
        return submitMutation(buildJsonObject {
            put("type", "skill")
            put("command", buildJsonObject {
                put("type", "create_user")
                put("input", buildJsonObject {
                put("id", id)
                put("source", "user")
                put("version", "${skill.version ?: "user"}-${digest.take(16)}")
                put("display_name", skill.name)
                put("summary", skill.description)
                put("category", "user")
                put("instructions_ref", "nola://android/skills/$id")
                put("instructions_digest", digest)
                put("instructions_text", skill.content)
                put("required_tools", JsonArray(skill.requiredTools.map { toolName ->
                    buildJsonObject {
                        put("provider", "builtin")
                        put("tool_name", toolName)
                    }
                }))
                put("default_mode", JsonNull)
                put("enabled", skill.enabled)
                put("created_at_ms", now)
                    put("updated_at_ms", now)
                })
            })
        })
    }

    fun listSkills(): List<UploadedContent> = decodeAndroidUserSkills(execute(
        buildJsonObject {
            put("type", "skill")
            put("command", buildJsonObject { put("type", "list_editors") })
        }
    ))

    fun createMcpServer(server: McpServerConfig): McpEditorSnapshot =
        decodeMcpSnapshot(submitMutation(buildJsonObject {
            put("type", "mcp")
            put("command", buildJsonObject {
                put("type", "create")
                put("id", server.serverId)
                put("display_name", server.name)
                put("enabled", server.enabled)
                put("configuration_json", json.encodeToString(server))
                put("created_at_ms", System.currentTimeMillis())
            })
        }))

    data class McpEditorSnapshot(val server: McpServerConfig, val revision: Long)

    fun listMcpServers(): List<McpServerConfig> = mcpEditorSnapshots().map { it.server }

    fun mcpEditorSnapshots(): List<McpEditorSnapshot> = execute(
        buildJsonObject {
            put("type", "mcp")
            put("command", buildJsonObject { put("type", "list") })
        }
    ).jsonArray.map(::decodeMcpSnapshot)

    private fun decodeMcpSnapshot(value: JsonElement): McpEditorSnapshot {
        val record = value.jsonObject
        val configuration = record.getValue("configuration_json").jsonPrimitive.content
        val server = json.decodeFromString<McpServerConfig>(configuration)
        return McpEditorSnapshot(
            server.copy(
                canonicalConfiguration = json.parseToJsonElement(configuration).jsonObject,
                canonicalRevision = record.getValue("revision").jsonPrimitive.content.toLong(),
                serverId = record.getValue("id").jsonPrimitive.content,
                name = record.getValue("display_name").jsonPrimitive.content,
                enabled = record.getValue("enabled").jsonPrimitive.content.toBooleanStrict()
            ),
            record.getValue("revision").jsonPrimitive.content.toLong()
        )
    }

    fun configureMcpServer(snapshot: McpEditorSnapshot, edited: McpServerConfig): McpEditorSnapshot {
        require(edited.serverId == snapshot.server.serverId) { "MCP editor cannot change server identity" }
        val configuration = requireNotNull(snapshot.server.canonicalConfiguration) {
            "Reload the MCP server before editing"
        }.toMutableMap()
        val before = json.parseToJsonElement(json.encodeToString(snapshot.server)).jsonObject
        val after = json.parseToJsonElement(json.encodeToString(edited)).jsonObject
        for (key in before.keys + after.keys) {
            if (before[key] != after[key]) {
                val value = after[key]
                if (value == null) configuration.remove(key) else configuration[key] = value
            }
        }
        return decodeMcpSnapshot(submitMutation(buildJsonObject {
            put("type", "mcp")
            put("command", buildJsonObject {
                put("type", "configure")
                put("id", snapshot.server.serverId)
                put("expected_revision", snapshot.revision)
                put("display_name", edited.name)
                put("enabled", edited.enabled)
                put("configuration_json", JsonObject(configuration).toString())
                put("updated_at_ms", System.currentTimeMillis())
            })
        }))
    }

    fun deleteMcpServer(snapshot: McpEditorSnapshot) {
        submitMutation(buildJsonObject {
            put("type", "mcp")
            put("command", buildJsonObject {
                put("type", "delete_checked")
                put("id", snapshot.server.serverId)
                put("expected_revision", snapshot.revision)
                put("deleted_at_ms", System.currentTimeMillis())
            })
        })
    }

    fun enqueueAutomationRun(
        automationId: String,
        definitionRevision: Long,
        occurrenceKey: String,
        trigger: String,
        scheduledForMs: Long,
    ): CanonicalAutomationRun {
        require(definitionRevision > 0)
        val occurrenceId = automationOccurrenceId(automationId, occurrenceKey)
        loadAutomationRun(occurrenceId)?.let {
            require(it.automationId == automationId && it.occurrenceKey == occurrenceKey)
            return it
        }
        val now = System.currentTimeMillis()
        return execute(buildJsonObject {
            put("type", "automation")
            put("command", automationRunCommand("enqueue", buildJsonObject {
                put("input", buildJsonObject {
                put("id", occurrenceId)
                put("automation_id", automationId)
                put("definition_revision", definitionRevision)
                put("occurrence_key", occurrenceKey)
                put("trigger", trigger)
                put("status", "queued")
                put("attempt", 0)
                put("worker_id", null as String?)
                put("lease_token", null as String?)
                put("lease_until_ms", null as String?)
                put("scheduled_for_ms", scheduledForMs)
                put("started_at_ms", null as String?)
                put("finished_at_ms", null as String?)
                put("next_retry_at_ms", null as String?)
                put("result_json", null as String?)
                put("error_json", null as String?)
                put("created_at_ms", now)
                put("updated_at_ms", now)
                })
            }))
        }).jsonObject.toCanonicalAutomationRun()
    }

    fun findAutomationOccurrence(automationId: String, occurrenceKey: String): CanonicalAutomationRun? =
        loadAutomationRun(automationOccurrenceId(automationId, occurrenceKey))?.also {
            require(it.automationId == automationId && it.occurrenceKey == occurrenceKey)
        }

    private fun automationOccurrenceId(automationId: String, occurrenceKey: String): String = "automation-run-" +
        java.security.MessageDigest.getInstance("SHA-256")
            .digest("$automationId\u0000$occurrenceKey".toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it.toInt() and 255) }

    fun claimAutomationRun(
        id: String,
        workerId: String,
        leaseToken: String,
        nowMs: Long,
        leaseUntilMs: Long,
    ): Boolean = execute(buildJsonObject {
        put("type", "automation")
        put("command", automationRunCommand("claim", buildJsonObject {
            put("id", id); put("worker_id", workerId); put("lease_token", leaseToken)
            put("now_ms", nowMs); put("lease_until_ms", leaseUntilMs)
        }))
    }).jsonPrimitive.content.toBoolean()

    fun heartbeatAutomationRun(
        id: String,
        leaseToken: String,
        nowMs: Long,
        leaseUntilMs: Long,
    ): Boolean = execute(buildJsonObject {
        put("type", "automation")
        put("command", automationRunCommand("heartbeat", buildJsonObject {
            put("id", id); put("lease_token", leaseToken)
            put("now_ms", nowMs); put("lease_until_ms", leaseUntilMs)
        }))
    }).jsonPrimitive.content.toBoolean()

    fun retryAutomationRun(
        id: String,
        leaseToken: String,
        nextRetryAtMs: Long,
        errorJson: String,
        updatedAtMs: Long,
    ): Boolean = execute(buildJsonObject {
        put("type", "automation")
        put("command", automationRunCommand("retry", buildJsonObject {
            put("id", id); put("lease_token", leaseToken); put("next_retry_at_ms", nextRetryAtMs)
            put("error_json", errorJson); put("updated_at_ms", updatedAtMs)
        }))
    }).jsonPrimitive.content.toBoolean()

    fun loadAutomationDefinition(id: String): AndroidAutomationDefinition? {
        val row = automationDefinitionCommand(buildJsonObject { put("type", "get"); put("id", id) })
        return if (row == kotlinx.serialization.json.JsonNull) null else decodeAndroidAutomationDefinition(row.jsonObject)
    }

    fun saveAutomationDefinition(id: String, name: String, enabled: Boolean, metadata: JsonObject,
        spec: AndroidAutomationSpec): AndroidAutomationDefinition {
        val previous = automationDefinitionCommand(buildJsonObject { put("type", "get"); put("id", id) })
            .takeUnless { it == kotlinx.serialization.json.JsonNull }?.jsonObject
        val revision = previous?.getValue("revision")?.jsonPrimitive?.content?.toLong() ?: 0L
        val now = System.currentTimeMillis()
        val input = buildJsonObject {
            put("id", id); put("name", name); put("enabled", enabled)
            put("schedule_format", "nola.android.scheduled_action.v1"); put("revision", Math.addExact(revision, 1L))
            put("execution_spec", buildJsonObject { put("kind", "agent"); put("spec", spec.toJson()) })
            put("metadata_json", metadata.toString())
            put("created_at_ms", previous?.getValue("created_at_ms") ?: JsonPrimitive(now))
            put("updated_at_ms", now); put("deleted_at_ms", kotlinx.serialization.json.JsonNull)
        }
        val result = automationDefinitionCommand(buildJsonObject {
            put("type", if (previous == null) "upsert" else "update_if_revision")
            put("input", input)
            if (previous != null) put("expected_revision", revision)
        })
        // The returned canonical revision, never a client default, owns enqueue.
        return decodeAndroidAutomationDefinition(result.jsonObject)
    }

    fun deleteAutomationDefinition(id: String) {
        automationDefinitionCommand(buildJsonObject { put("type", "delete"); put("id", id); put("deleted_at_ms", System.currentTimeMillis()) })
    }

    private fun automationDefinitionCommand(command: JsonObject) = execute(buildJsonObject {
        put("type", "automation"); put("command", buildJsonObject { put("type", "definition"); put("command", command) })
    })

    fun observeAutomationExecution(occurrenceId: String): JsonObject = execute(buildJsonObject {
        put("type", "observe_automation_execution"); put("occurrence_id", occurrenceId)
    }).jsonObject

    fun reconcileAutomationExecution(occurrenceId: String): JsonObject? = execute(buildJsonObject {
        put("type", "reconcile_automation_execution"); put("occurrence_id", occurrenceId)
        put("reconciled_at_ms", System.currentTimeMillis())
    }).takeUnless { it == kotlinx.serialization.json.JsonNull }?.jsonObject

    fun observeAutomationWorker(lease: AndroidAutomationLease, detail: String) {
        execute(buildJsonObject {
            put("type", "observe_automation_worker"); put("lease", lease.toJson())
            put("observation", buildJsonObject { put("kind", "disconnected"); put("detail", detail) })
            put("observed_at_ms", System.currentTimeMillis())
        })
    }

    fun acknowledgeAutomationNotification(occurrenceId: String, notificationId: String) {
        val acknowledged = execute(buildJsonObject {
            put("type", "ack_automation_notification"); put("occurrence_id", occurrenceId)
            put("notification_id", notificationId); put("delivered_at_ms", System.currentTimeMillis())
        })
        check(acknowledged.jsonPrimitive.content == "true") { "Automation notification receipt was not acknowledged" }
    }

    fun loadAutomationRun(id: String): CanonicalAutomationRun? {
        val value = execute(buildJsonObject {
            put("type", "automation")
            put("command", automationRunCommand("load", buildJsonObject { put("id", id) }))
        })
        return if (value.toString() == "null") null else value.jsonObject.toCanonicalAutomationRun()
    }

    private fun automationRunCommand(type: String, fields: JsonObject): JsonObject = buildJsonObject {
        put("type", "run")
        put("command", buildJsonObject {
            put("type", type)
            fields.forEach { (key, value) -> put(key, value) }
        })
    }

    fun createConversation(
        id: String,
        title: String,
        createdAtMs: Long,
        updatedAtMs: Long,
        settingsJson: String?,
    ) {
        val receipt = execute(buildJsonObject {
            put("type", "create_conversation")
            put("input", buildJsonObject {
                put("id", id)
                put("title", title)
                put("settings_json", settingsJson)
                put("created_at_ms", createdAtMs)
                put("updated_at_ms", updatedAtMs)
            })
        })
        requireCreatedConversation(receipt)
    }

    fun prepareRemoteSubmission(
        sourceHostId: String,
        clientId: String,
        conversationId: String,
        conversationTitle: String,
        submissionId: String,
        userText: String,
        userContentJson: String?,
        createdAtMs: Long,
        userMessageId: String = submissionId,
    ): com.nola.data.remote.api.RemoteSubmissionIntent = com.nola.data.remote.api.decodeRemoteSubmissionIntent(
        execute(buildJsonObject {
            put("type", "prepare_remote_submission")
            put("input", buildJsonObject {
                put("source_host_id", sourceHostId)
                put("client_id", clientId)
                put("conversation_id", conversationId)
                put("conversation_title", conversationTitle)
                put("submission_id", submissionId)
                put("user_message_id", userMessageId)
                put("user_part_id", "$userMessageId:text")
                put("user_text", userText)
                put("user_content_json", userContentJson)
                put("assistant_message_id", "$submissionId:assistant")
                put("assistant_part_id", "$submissionId:assistant:text")
                put("context_checkpoint_id", null as String?)
                put("created_at_ms", createdAtMs)
            })
        }).jsonObject)

    fun bindRemoteSubmissionRequest(identity: com.nola.data.remote.api.RemoteSubmissionIdentity, request: JsonObject) =
        com.nola.data.remote.api.decodeRemoteSubmissionIntent(execute(buildJsonObject {
            put("type", "bind_remote_submission_request"); put("identity", identity.wire()); put("request", request)
        }).jsonObject)

    fun readRemoteSubmissionIntent(conversationId: String): com.nola.data.remote.api.RemoteSubmissionIntent? =
        execute(buildJsonObject { put("type", "read_remote_submission_intent"); put("conversation_id", conversationId) })
            .takeUnless { it == JsonNull }?.jsonObject?.let { com.nola.data.remote.api.decodeRemoteSubmissionIntent(it) }

    fun abandonPreparedRemoteSubmission(identity: com.nola.data.remote.api.RemoteSubmissionIdentity) =
        com.nola.data.remote.api.decodeRemoteSubmissionIntent(execute(buildJsonObject {
            put("type", "abandon_prepared_remote_submission"); put("identity", identity.wire())
        }).jsonObject)

    fun readRemoteSubmissionRequest(identity: com.nola.data.remote.api.RemoteSubmissionIdentity, digest: String): JsonObject =
        execute(buildJsonObject { put("type", "read_remote_submission_request"); put("identity", identity.wire()); put("request_digest", digest) }).jsonObject

    fun acceptRemoteSubmissionReceipt(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt) =
        com.nola.data.remote.api.decodeRemoteSubmissionIntent(execute(buildJsonObject {
            put("type", "accept_remote_submission_receipt"); put("receipt", receipt.wire)
        }).jsonObject)

    fun adoptObservedRemoteRun(receipt: com.nola.data.remote.api.RemoteSubmissionReceipt) {
        execute(buildJsonObject { put("type", "adopt_observed_remote_run"); put("receipt", receipt.wire) })
    }


    fun loadConversation(id: String): CanonicalConversation? {
        val value = execute(buildJsonObject {
            put("type", "load_conversation")
            put("conversation_id", id)
        })
        if (value.toString() == "null") return null
        return value.jsonObject.toCanonicalConversation()
    }

    fun loadConversationProjectionV1(id: String): ConversationProjectionV1 =
        ConversationProjectionV1Decoder.decode(execute(buildJsonObject {
            put("type", "load_conversation_projection_v1")
            put("conversation_id", id)
        }))

    fun loadConversationProjectionWindowV1(
        id: String,
        beforeOrdinal: Long?,
        limit: Int,
        contentByteLimit: Int,
    ): ConversationProjectionWindowV1 = ConversationProjectionV1Decoder.decodeWindow(
        execute(buildJsonObject {
            put("type", "load_conversation_projection_window_v1")
            put("conversation_id", id)
            beforeOrdinal?.let { put("before_ordinal", it) }
            put("limit", limit)
            put("content_byte_limit", contentByteLimit)
        }),
    )

    fun listConversations(): List<CanonicalConversationSummary> {
        val result = mutableListOf<CanonicalConversationSummary>()
        while (true) {
            val page = execute(buildJsonObject {
                put("type", "list_conversations")
                put("query", buildJsonObject {
                    put("project_root", JsonNull)
                    put("non_empty", false)
                    put("limit", 1024)
                    put("offset", result.size)
                })
            }).jsonArray
            result += page.map { value ->
                val record = value.jsonObject
                CanonicalConversationSummary(
                    id = record.getValue("id").jsonPrimitive.content,
                    title = record.getValue("title").jsonPrimitive.content,
                    createdAtMs = record.getValue("created_at_ms").jsonPrimitive.content.toLong(),
                    updatedAtMs = record.getValue("updated_at_ms").jsonPrimitive.content.toLong(),
                )
            }
            if (page.size < 1024) return result
        }
    }

    fun renameConversation(id: String, title: String, updatedAtMs: Long) {
        execute(buildJsonObject {
            put("type", "rename_conversation")
            put("conversation_id", id)
            put("title", title)
            put("updated_at_ms", updatedAtMs)
        })
    }

    fun prepareContextCompaction(id: String): JsonObject = execute(buildJsonObject {
        put("type", "context")
        put("command", buildJsonObject { put("type", "prepare_compaction"); put("conversation_id", id) })
    }).jsonObject

    fun replaceContextSummary(source: JsonObject, summary: String) {
        check(execute(buildJsonObject {
            put("type", "context")
            put("command", buildJsonObject {
                put("type", "replace_summary"); put("source", source); put("summary", summary)
                put("committed_at_ms", System.currentTimeMillis())
            })
        }).jsonPrimitive.content.toBoolean())
    }


    fun selectConversationExecution(id: String, expectedRevision: Long, selection: JsonObject): ConfigurationCommitResult =
        decodeConfigurationCommit(execute(buildJsonObject {
            put("type", "select_conversation_execution")
            put("input", buildJsonObject {
                put("conversation_id", id)
                put("expected_revision", expectedRevision)
                put("selection", selection)
                put("updated_at_ms", System.currentTimeMillis())
            })
        }))

    fun setConversationPermission(id: String, expectedRevision: Long, expectedRunId: String?,
                                  expectedSourceHostId: String?, profile: String): ConfigurationCommitResult =
        decodeConfigurationCommit(execute(buildJsonObject {
            put("type", "set_conversation_permission")
            put("conversation_id", id)
            put("expected_revision", expectedRevision)
            put("expected_run_id", expectedRunId?.let(::JsonPrimitive) ?: JsonNull)
            put("expected_source_host_id", expectedSourceHostId?.let(::JsonPrimitive) ?: JsonNull)
            put("profile", profile)
            put("updated_at_ms", System.currentTimeMillis())
        }))

    fun patchConversationConfiguration(id: String, expectedRevision: Long, patchJson: String): ConfigurationCommitResult {
        val result = execute(buildJsonObject {
            put("type", "patch_conversation_configuration")
            put("input", buildJsonObject {
                put("conversation_id", id)
                put("expected_revision", expectedRevision)
                put("patch_json", patchJson)
                put("updated_at_ms", System.currentTimeMillis())
            })
        })
        return decodeConfigurationCommit(result)
    }

    private fun decodeConfigurationCommit(result: JsonElement): ConfigurationCommitResult {
        val value = result.jsonObject
        return when (value.getValue("status").jsonPrimitive.content) {
            "committed" -> ConfigurationCommitResult.Committed(value.getValue("snapshot").jsonObject)
            "conflict" -> ConfigurationCommitResult.Conflict(value.getValue("expected_revision").jsonPrimitive.content.toLong(),
                value.getValue("actual_revision").jsonPrimitive.content.toLong())
            "not_found" -> ConfigurationCommitResult.NotFound(value.getValue("conversation_id").jsonPrimitive.content)
            else -> error("Invalid configuration commit result")
        }
    }

    fun deleteConversation(id: String) {
        execute(buildJsonObject {
            put("type", "delete_conversation")
            put("conversation_id", id)
        })
    }

    private fun canonicalMessages(messages: List<CanonicalConversationMessage>, firstOrdinal: Int) =
        kotlinx.serialization.json.buildJsonArray {
            messages.forEachIndexed { index, message ->
                add(buildJsonObject {
                    put("id", message.id)
                    put("run_id", null as String?)
                    put("role", message.role)
                    put("status", "completed")
                    put("ordinal", firstOrdinal + index)
                    put("created_at_ms", message.createdAtMs)
                    put("completed_at_ms", message.createdAtMs)
                    put("parts", kotlinx.serialization.json.buildJsonArray {
                        add(buildJsonObject {
                            put("id", "${message.id}:text")
                            put("kind", "text")
                            put("ordinal", 1)
                            put("state", "completed")
                            put("content_text", message.text)
                            put("content_json", message.metadataJson)
                            put("content_digest", null as String?)
                            put("created_at_ms", message.createdAtMs)
                            put("updated_at_ms", message.createdAtMs)
                        })
                    })
                })
            }
        }

    override fun close() = synchronized(lock) {
        if (handle != 0L) {
            AgentCoreNative.nativeApplicationFree(handle)
            handle = 0L
        }
    }
}

data class CanonicalConversationMessage(
    val id: String,
    val role: String,
    val text: String,
    val metadataJson: String?,
    val createdAtMs: Long,
)

data class CanonicalConversation(
    val id: String,
    val title: String,
    val revision: Long,
    val createdAtMs: Long,
    val updatedAtMs: Long,
    val settingsJson: String?,
    val messages: List<CanonicalConversationMessage>,
    val configurationRevision: Long? = null,
)

sealed interface ConfigurationCommitResult {
    data class Committed(val snapshot: JsonObject) : ConfigurationCommitResult
    data class Conflict(val expectedRevision: Long, val actualRevision: Long) : ConfigurationCommitResult
    data class NotFound(val conversationId: String) : ConfigurationCommitResult
    fun requireCommitted(): JsonObject = when (this) {
        is Committed -> snapshot
        is Conflict -> throw ConcurrentModificationException("Conversation configuration changed; your edit is retained")
        is NotFound -> error("Conversation is missing: $conversationId")
    }
}

data class CanonicalConversationSummary(
    val id: String,
    val title: String,
    val createdAtMs: Long,
    val updatedAtMs: Long,
)

data class CanonicalProductSetting(
    val key: String,
    val valueJson: String,
    val revision: Long,
    val updatedAtMs: Long,
)

@Serializable
data class CanonicalModelProfile(
    val id: String,
    @kotlinx.serialization.SerialName("display_name") val displayName: String,
    val provider: String,
    @kotlinx.serialization.SerialName("model_name") val modelName: String,
    val runtime: String,
    @kotlinx.serialization.SerialName("source_json") val sourceJson: String,
    @kotlinx.serialization.SerialName("parameters_json") val parametersJson: String,
    val selected: Boolean,
    val enabled: Boolean,
    val revision: Long,
    @kotlinx.serialization.SerialName("created_at_ms") val createdAtMs: Long,
    @kotlinx.serialization.SerialName("updated_at_ms") val updatedAtMs: Long,
    @kotlinx.serialization.SerialName("deleted_at_ms") val deletedAtMs: Long? = null,
)

data class CanonicalSyncChange(
    val localSequence: Long,
    val operationId: String,
    val entityType: String,
    val entityId: String,
    val revision: Long,
    val deleted: Boolean,
    val payloadJson: String,
    val status: String,
    val remoteCursor: Long?,
    val createdAtMs: Long,
    val resolvedAtMs: Long?,
)

data class CanonicalSyncCursor(val replicaId: String, val cursor: Long, val updatedAtMs: Long)

data class CanonicalRemoteSyncChange(
    val cursor: Long,
    val operationId: String,
    val entityType: String,
    val entityId: String,
    val revision: Long,
    val deleted: Boolean,
    val payloadJson: String,
    val createdAtMs: Long,
)

data class CanonicalRemoteSyncPageResult(
    val applied: Int,
    val skipped: Int,
    val cursor: Long,
)

private fun JsonObject.toCanonicalSyncChange() = CanonicalSyncChange(
    localSequence = getValue("local_sequence").jsonPrimitive.content.toLong(),
    operationId = getValue("operation_id").jsonPrimitive.content,
    entityType = getValue("entity_type").jsonPrimitive.content,
    entityId = getValue("entity_id").jsonPrimitive.content,
    revision = getValue("revision").jsonPrimitive.content.toLong(),
    deleted = getValue("deleted").jsonPrimitive.content.toBoolean(),
    payloadJson = getValue("payload_json").jsonPrimitive.content,
    status = getValue("status").jsonPrimitive.content,
    remoteCursor = get("remote_cursor")?.takeUnless { it.toString() == "null" }?.jsonPrimitive?.content?.toLong(),
    createdAtMs = getValue("created_at_ms").jsonPrimitive.content.toLong(),
    resolvedAtMs = get("resolved_at_ms")?.takeUnless { it.toString() == "null" }?.jsonPrimitive?.content?.toLong(),
)

private fun JsonObject.toCanonicalSyncCursor() = CanonicalSyncCursor(
    replicaId = getValue("replica_id").jsonPrimitive.content,
    cursor = getValue("cursor").jsonPrimitive.content.toLong(),
    updatedAtMs = getValue("updated_at_ms").jsonPrimitive.content.toLong(),
)

private fun JsonObject.toCanonicalRemoteSyncPageResult() = CanonicalRemoteSyncPageResult(
    applied = getValue("applied").jsonPrimitive.content.toInt(),
    skipped = getValue("skipped").jsonPrimitive.content.toInt(),
    cursor = getValue("cursor").jsonPrimitive.content.toLong(),
)

private fun JsonObject.toCanonicalProductSetting() = CanonicalProductSetting(
    key = getValue("key").jsonPrimitive.content,
    valueJson = getValue("value_json").jsonPrimitive.content,
    revision = getValue("revision").jsonPrimitive.content.toLong(),
    updatedAtMs = getValue("updated_at_ms").jsonPrimitive.content.toLong(),
)

data class CanonicalAutomationRun(
    val id: String,
    val definitionRevision: Long,
    val automationId: String,
    val occurrenceKey: String,
    val trigger: String,
    val status: String,
    val attempt: Long,
    val scheduledForMs: Long,
    val nextRetryAtMs: Long?,
    val resultJson: String?,
    val errorJson: String?,
)

private fun JsonObject.toCanonicalAutomationRun() = CanonicalAutomationRun(
    id = getValue("id").jsonPrimitive.content,
    definitionRevision = getValue("definition_revision").jsonPrimitive.content.toLong(),
    automationId = getValue("automation_id").jsonPrimitive.content,
    occurrenceKey = getValue("occurrence_key").jsonPrimitive.content,
    trigger = getValue("trigger").jsonPrimitive.content,
    status = getValue("status").jsonPrimitive.content,
    attempt = getValue("attempt").jsonPrimitive.content.toLong(),
    scheduledForMs = getValue("scheduled_for_ms").jsonPrimitive.content.toLong(),
    nextRetryAtMs = get("next_retry_at_ms")?.takeUnless { it.toString() == "null" }
        ?.jsonPrimitive?.content?.toLong(),
    resultJson = get("result_json")?.takeUnless { it.toString() == "null" }?.jsonPrimitive?.content,
    errorJson = get("error_json")?.takeUnless { it.toString() == "null" }?.jsonPrimitive?.content,
)

private fun JsonObject.toCanonicalConversation(): CanonicalConversation = CanonicalConversation(
    id = getValue("id").jsonPrimitive.content,
    title = getValue("title").jsonPrimitive.content,
    revision = getValue("revision").jsonPrimitive.content.toLong(),
    configurationRevision = getValue("configuration_revision").jsonPrimitive.content.toLong(),
    createdAtMs = getValue("created_at_ms").jsonPrimitive.content.toLong(),
    updatedAtMs = getValue("updated_at_ms").jsonPrimitive.content.toLong(),
    settingsJson = get("settings_json")?.let { value ->
        if (value.toString() == "null") null else value.jsonPrimitive.content
    },
    messages = getValue("messages").jsonArray.map { messageValue ->
        val message = messageValue.jsonObject
        val textPart = message.getValue("parts").jsonArray.map { it.jsonObject }
            .firstOrNull { it["kind"]?.jsonPrimitive?.content == "text" }
        CanonicalConversationMessage(
            id = message.getValue("id").jsonPrimitive.content,
            role = message.getValue("role").jsonPrimitive.content,
            text = textPart?.get("content_text")?.jsonPrimitive?.content.orEmpty(),
            metadataJson = textPart?.get("content_json")?.let { value ->
                if (value.toString() == "null") null else value.jsonPrimitive.content
            },
            createdAtMs = message.getValue("created_at_ms").jsonPrimitive.content.toLong(),
        )
    },
)
