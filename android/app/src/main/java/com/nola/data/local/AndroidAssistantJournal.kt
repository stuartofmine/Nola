package com.nola.data.local
import com.nola.core.hasValidRunResultStructure

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.NonCancellable
import kotlinx.serialization.json.*
import javax.inject.Inject
import javax.inject.Singleton

internal fun JsonObject.journalString(key: String): String = getValue(key).jsonPrimitive.let {
    require(it.isString && it.content.isNotBlank()) { "Invalid journal $key" }; it.content
}
internal fun JsonObject.journalNumber(key: String): ULong = journalString(key).let {
    require(it == "0" || it.first() in '1'..'9' && it.all(Char::isDigit)) { "Invalid journal counter" }
    it.toULong()
}
internal fun JsonObject.journalBoolean(key: String): Boolean = getValue(key).jsonPrimitive.boolean
internal fun journalCommand(type: String, values: JsonObjectBuilder.() -> Unit = {}): JsonObject = buildJsonObject { put("type", type); values() }
internal fun journalRequest(operation: String, values: JsonObjectBuilder.() -> Unit = {}): JsonObject = buildJsonObject { put("operation", operation); values() }
internal fun JsonObject.requireTerminalMessage() {
    require(journalString("state") in setOf("completed", "failed", "cancelled"))
    val completed = getValue("completed_at_ms").jsonPrimitive
    val created = getValue("created_at_ms").jsonPrimitive
    require(!completed.isString && !created.isString && completed.long >= created.long && created.long >= 0)
}
internal fun recordCursor(record: JsonObject): JsonObject = record.getValue("header").jsonObject.let { header -> buildJsonObject {
    put("identity", header.getValue("identity")); put("sequence", header.getValue("sequence")); put("record_digest", header.getValue("record_digest"))
} }

data class AndroidJournalTurn(val hostId: String, val conversationId: String, val submissionId: String,
    val assistantMessageId: String, var runId: String? = null)

/** Thin durable receiver: Application, not a UI cache, validates record chains,
 * content prefixes, complete manifest ownership, and the terminal transition. */
@Singleton
class AndroidAssistantJournal(private val execute: (JsonObject) -> JsonElement) {
    @Inject constructor(runtime: AndroidApplicationRuntime) : this(runtime::execute)
    fun permissionOwnerFacts(conversationId: String, hostId: String?, clientId: String?, unresolvedRunId: String?,
        sourceUnconfirmed: Boolean): PermissionConfigurationOwner = readPermissionConfigurationOwner(
            conversationId, hostId, clientId, unresolvedRunId, sourceUnconfirmed, execute)
    /** Resolve a sealed or live displayed root through Core's durable receipt.
     * Pending-intent and permission-control queries deliberately omit sealed Runs. */
    fun admittedSourceOwner(conversationId: String, hostId: String, clientId: String,
        assistantMessageId: String?): com.nola.data.remote.api.RemoteSubmissionIdentity? {
        if (assistantMessageId == null) return null
        val window = execute(journalCommand("load_conversation_projection_window_v1") {
            put("conversation_id", conversationId); put("before_ordinal", JsonNull)
            put("limit", 1); put("content_byte_limit", 1)
        }).jsonObject
        val root = window.getValue("assistant_journal_heads").jsonArray.map { it.jsonObject.getValue("identity").jsonObject }
            .singleOrNull { it.journalString("conversation_id") == conversationId &&
                it.journalString("assistant_message_id") == assistantMessageId } ?: return null
        val key = com.nola.data.remote.api.RemoteRunKey(conversationId, root.journalString("run_id"), assistantMessageId)
        val receipt = execute(journalCommand("read_admitted_remote_run_identity") {
            put("source_host_id", hostId); put("client_id", clientId); put("conversation_id", conversationId)
            put("run_id", key.runId); put("assistant_message_id", assistantMessageId)
        }).takeUnless { it == JsonNull }?.jsonObject ?: return null
        return Json.decodeFromJsonElement<com.nola.data.remote.api.RemoteSubmissionIdentity>(receipt).also {
            it.validate()
            require(it.sourceHostId == hostId && it.clientId == clientId && it.conversationId == conversationId &&
                it.submissionId == key.runId && it.assistantMessageId == assistantMessageId)
        }
    }
    private val presentation = AndroidJournalPresentation(execute)
    private val admitted = java.util.Collections.synchronizedSet(mutableSetOf<String>())
    private fun admissionKey(host: String, identity: JsonObject) = JsonArray(listOf(JsonPrimitive(host), identity)).toString()

    private suspend fun receiveAdmissions(turn: AndroidJournalTurn, remote: suspend (JsonObject) -> JsonObject,
        current: () -> Boolean, required: JsonObject? = null): JsonObject {
        currentCoroutineContext().ensureActive(); check(current())
        suspend fun query(request: JsonObject): JsonElement {
            currentCoroutineContext().ensureActive(); check(current())
            val response = remote(request); check(current())
            require(response.journalString("source_host_id") == turn.hostId)
            return response.getValue("result")
        }
        val through = required ?: query(journalRequest("admission_head") {
            put("conversation_id", turn.conversationId); put("run_id", turn.runId ?: turn.submissionId)
        }).jsonObject
        val identity = through.getValue("identity").jsonObject
        require(identity.journalString("conversation_id") == turn.conversationId && identity.journalString("assistant_message_id") == turn.assistantMessageId)
        require(turn.runId == null || identity.journalString("run_id") == turn.runId)
        turn.runId = identity.journalString("run_id")
        var after = execute(journalCommand("run_message_admission_reception_state") {
            put("source_host_id", turn.hostId); put("through", through)
        }).jsonObject
        require(after["identity"] == identity)
        // Core verifies the sealed historical digest even when its durable cursor
        // has since advanced; an immutable seal must never roll that cursor back.
        if (after.journalNumber("sequence") >= through.journalNumber("sequence")) return through
        while (after != through) {
            require(after["identity"] == identity && after.journalNumber("sequence") < through.journalNumber("sequence"))
            val page = query(journalRequest("admissions") {
                put("after", after); put("through", through); put("max_records", 32); put("max_encoded_bytes", 262144)
            }).jsonObject
            require(page["after"] == after && page["through"] == through)
            val next = page.getValue("next").jsonObject
            require(next["identity"] == identity && next.journalNumber("sequence") > after.journalNumber("sequence")
                && next.journalNumber("sequence") <= through.journalNumber("sequence"))
            require(page.getValue("records").jsonArray.size in 1..32 && page.journalBoolean("complete") == (next == through))
            check(current()); currentCoroutineContext().ensureActive()
            val received = execute(journalCommand("receive_run_message_admissions") {
                put("source_host_id", turn.hostId); put("page", page)
            }).jsonObject
            require(received == next)
            after = received
        }
        return through
    }
    fun overlay(projection: com.nola.domain.projection.ConversationProjectionV1, heads: List<JsonObject>) = presentation.overlay(projection, heads)
    fun displayParts(through: JsonObject, localRevision: Long = 0) = presentation.partsFor(through, localRevision)

    /** Tokens in a notification are never authority. Admission and the source
     * record are validated and persisted before the canonical projection is read. */
    suspend fun receiveContextUsage(turn: AndroidJournalTurn, remote: suspend (JsonObject) -> JsonObject,
        current: () -> Boolean, record: JsonObject? = null) {
        val admission = receiveAdmissions(turn, remote, current,
            record?.getValue("admission")?.jsonObject)
        val identity = admission.getValue("identity").jsonObject
        val committed = if (record != null) record else {
            currentCoroutineContext().ensureActive(); check(current())
            val response = remote(journalRequest("context_usage") { put("identity", identity) })
            currentCoroutineContext().ensureActive(); check(current())
            require(response.journalString("source_host_id") == turn.hostId)
            response.getValue("result").takeUnless { it == JsonNull }?.jsonObject ?: return
        }
        require(committed["identity"] == identity)
        require(committed["protocol_version"]?.jsonPrimitive?.content == "3") { "Unsupported committed context usage format" }
        val recordAdmission = committed.getValue("admission").jsonObject
        require(recordAdmission["identity"] == identity)
        receiveAdmissions(turn, remote, current, recordAdmission)
        currentCoroutineContext().ensureActive(); check(current())
        execute(journalCommand("accept_remote_run_context_usage") {
            put("source_host_id", turn.hostId); put("record", committed)
        })
    }

    /** Exact cold/active catch-up starts at durable Run admission, not a local
     * journal cache. No seal is needed to receive a still-running Run. */
    suspend fun catchUpRun(turn: AndroidJournalTurn, remote: suspend (JsonObject) -> JsonObject,
        current: () -> Boolean, admission: JsonObject? = null, terminalSeal: JsonObject? = null,
        contextUsage: JsonObject? = null): Pair<List<JsonObject>, Boolean> {
        val run = checkNotNull(turn.runId) { "Cold observation requires an exact durable Run" }
        val admitted = receiveAdmissions(turn, remote, current, admission)
        if (contextUsage != null) receiveContextUsage(turn, remote, current, contextUsage)
        val heads = linkedMapOf<String, JsonObject>()
        var after: String? = null
        var pages = 0
        do {
            currentCoroutineContext().ensureActive(); check(current())
            val response = remote(journalRequest("heads") {
                put("conversation_id", turn.conversationId); put("run_id", run)
                put("after_message_id", after); put("limit", 64)
            })
            check(current()); require(response.journalString("source_host_id") == turn.hostId)
            val page = response.getValue("result").jsonArray
            require(page.size <= 64 && ++pages <= 64) { "Run head window exceeded its bounded transfer" }
            for (element in page) {
                val head = element.jsonObject
                val identity = head.getValue("identity").jsonObject
                val id = identity.journalString("assistant_message_id")
                require(after == null || id > checkNotNull(after))
                require(identity["source_store_epoch"] == admitted.getValue("identity").jsonObject["source_store_epoch"])
                receive(head, turn, remote, current)
                heads[id] = head; after = id
            }
        } while (page.size == 64)
        if (terminalSeal == null) return heads.values.toList() to false
        require(terminalSeal["identity"] == admitted["identity"])
        val sealed = receiveTerminal(turn, remote, current, terminalSeal.journalString("seal_digest"), run)
            ?: error("Observed terminal seal disappeared before durable reception")
        for (head in sealed) {
            val id = head.getValue("identity").jsonObject.journalString("assistant_message_id")
            val previous = heads[id]
            if (previous == null || previous.journalNumber("sequence") < head.journalNumber("sequence")) heads[id] = head
        }
        return heads.values.toList() to true
    }

    /** Reconcile only Runs represented in the caller's visible history window.
     * Seal admits owners; current heads may advance beyond that immutable frontier. */
    suspend fun recoverVisible(hostId: String, conversationId: String, visibleHeads: List<JsonObject>,
        remote: suspend (JsonObject) -> JsonObject, current: () -> Boolean) {
        val owners = visibleHeads.map { it.getValue("identity").jsonObject }.distinctBy { it.journalString("run_id") }
        suspend fun query(request: JsonObject): JsonElement {
            currentCoroutineContext().ensureActive(); check(current())
            val response = remote(request); check(current())
            require(response.journalString("source_host_id") == hostId)
            return response.getValue("result")
        }
        for (visible in owners) {
            require(visible.journalString("conversation_id") == conversationId)
            val run = visible.journalString("run_id")
            val raw = query(journalRequest("terminal_seal") { put("conversation_id", conversationId); put("run_id", run) })
            if (raw == JsonNull) {
                val admission = query(journalRequest("admission_head") {
                    put("conversation_id", conversationId); put("run_id", run)
                }).jsonObject
                val root = admission.getValue("identity").jsonObject
                require(root["source_store_epoch"] == visible["source_store_epoch"])
                catchUpRun(AndroidJournalTurn(hostId, conversationId, run, root.journalString("assistant_message_id"), run),
                    remote, current, admission)
                continue
            }
            val identity = raw.jsonObject.getValue("identity").jsonObject
            require(identity["source_store_epoch"] == visible["source_store_epoch"])
            val turn = AndroidJournalTurn(hostId, conversationId, run, identity.journalString("assistant_message_id"), run)
            receiveTerminal(turn, remote, current, raw.jsonObject.journalString("seal_digest"))
                ?: error("Committed terminal seal disappeared")
            var after: String? = null
            do {
                val heads = query(journalRequest("heads") {
                    put("conversation_id", conversationId); put("run_id", run); put("after_message_id", after); put("limit", 64)
                }).jsonArray
                require(heads.size <= 64)
                for (element in heads) {
                    val head = element.jsonObject; val owner = head.getValue("identity").jsonObject
                    val id = owner.journalString("assistant_message_id")
                    require(owner["source_store_epoch"] == identity["source_store_epoch"] && (after == null || id > checkNotNull(after)))
                    receive(head, turn, remote, current)
                    after = id
                }
            } while (heads.size == 64)
        }
    }

    private fun owner(through: JsonObject, turn: AndroidJournalTurn, bind: Boolean) {
        through.journalNumber("sequence")
        require(through.journalString("record_digest").matches(Regex("[0-9a-f]{64}")))
        val identity = through.getValue("identity").jsonObject
        require(identity.journalString("conversation_id") == turn.conversationId)
        identity.journalString("source_store_epoch")
        val run = identity.journalString("run_id")
        if (turn.runId == null && bind) {
            require(identity.journalString("assistant_message_id") == turn.assistantMessageId)
            turn.runId = run
        }
        require(run == turn.runId) { "Journal Run owner mismatch" }
    }

    suspend fun receive(through: JsonObject, turn: AndroidJournalTurn,
        remote: suspend (JsonObject) -> JsonObject, current: () -> Boolean) {
        check(current())
        val key = admissionKey(turn.hostId, through.getValue("identity").jsonObject)
        if (key !in admitted) {
            val identity = through.getValue("identity").jsonObject
            require(identity.journalString("conversation_id") == turn.conversationId)
            require(turn.runId == null || turn.runId == identity.journalString("run_id"))
            // The candidate Run is proven by the authenticated root admission head.
            if (turn.runId == null) turn.runId = identity.journalString("run_id")
            receiveAdmissions(turn, remote, current)
        }
        owner(through, turn, bind = false)
        suspend fun query(request: JsonObject): JsonElement {
            currentCoroutineContext().ensureActive(); check(current())
            val response = remote(request)
            check(current()); require(response.journalString("source_host_id") == turn.hostId)
            return response.getValue("result")
        }
        fun state() = execute(journalCommand("assistant_journal_reception_state") {
            put("source_host_id", turn.hostId); put("through", through)
        }).jsonObject
        var reception = state()
        while (!reception.journalBoolean("complete")) {
            val after = reception.getValue("after").jsonObject
            val page = query(journalRequest("read") { put("after", after); put("through", through); put("max_records", 32); put("max_encoded_bytes", 262144) }).jsonObject
            require(page.journalString("status") == "records") { "Journal snapshot recovery is required" }
            require(page["after"] == after && page["through"] == through)
            val records = page.getValue("records").jsonArray
            require(records.isNotEmpty() && records.size <= 32)
            require(page["next"] == recordCursor(records.last().jsonObject))
            require(page.journalBoolean("complete") == (page["next"] == through))
            for (element in records) {
                val record = element.jsonObject; val at = recordCursor(record)
                owner(at, turn, bind = false)
                require(at["identity"] == through["identity"] && at.journalNumber("sequence") > after.journalNumber("sequence")
                    && at.journalNumber("sequence") <= through.journalNumber("sequence"))
                fun nextNeed() = execute(journalCommand("next_assistant_journal_content") { put("source_host_id", turn.hostId); put("record", record) })
                var need = nextNeed(); var snapshot: JsonObject? = null
                try {
                    if (need != JsonNull) {
                        snapshot = query(journalRequest("open_snapshot") { put("at", at) }).jsonObject
                        require(snapshot["at"] == at)
                    }
                    while (need != JsonNull) {
                        val value = need.jsonObject
                        val chunk = query(journalRequest("record_chunk") {
                            put("snapshot", checkNotNull(snapshot)); put("part_id", value.getValue("part_id")); put("content", value.getValue("content"))
                            put("chunk_index", value.getValue("expected_prefix").jsonObject.getValue("chunk_count"))
                        })
                        execute(journalCommand("stage_assistant_journal_content") { put("input", buildJsonObject {
                            put("source_host_id", turn.hostId); put("record", record); put("part_id", value.getValue("part_id"))
                            put("content", value.getValue("content")); put("expected_prefix", value.getValue("expected_prefix"))
                            put("chunks", JsonArray(listOf(chunk))); put("restart", false)
                        }) })
                        need = nextNeed(); require(need != value) { "Journal content did not advance" }
                    }
                    check(current()); currentCoroutineContext().ensureActive()
                    val receipt = execute(journalCommand("receive_assistant_journal_record") {
                        put("source_host_id", turn.hostId); put("record", record); put("chunks", JsonArray(emptyList()))
                    }).jsonObject
                    require(receipt["cursor"] == at)
                } finally {
                    snapshot?.let { pinned -> withContext(NonCancellable) {
                        // Cleanup remains bound to the captured transport even after UI cancellation.
                        remote(journalRequest("release_snapshot") { put("snapshot", pinned) })
                    } }
                }
            }
            reception = state()
            require(reception.journalBoolean("complete") || reception["after"] != after)
        }
        admitted += key
    }

    suspend fun receiveTerminal(turn: AndroidJournalTurn, remote: suspend (JsonObject) -> JsonObject,
        current: () -> Boolean, expectedDigest: String? = null, candidateRunId: String? = null): List<JsonObject>? {
        suspend fun query(request: JsonObject): JsonElement {
            currentCoroutineContext().ensureActive(); check(current())
            val response = remote(request); check(current())
            require(response.journalString("source_host_id") == turn.hostId)
            return response.getValue("result")
        }
        val run = turn.runId ?: candidateRunId ?: turn.submissionId
        val raw = query(journalRequest("terminal_seal") { put("conversation_id", turn.conversationId); put("run_id", run) })
        if (raw == JsonNull) return null
        val seal = raw.jsonObject; val identity = seal.getValue("identity").jsonObject
        require(seal.getValue("protocol_version").jsonPrimitive.content == "2")
        val result = seal.getValue("result").jsonObject
        require(result.journalString("conversation_id") == turn.conversationId && result.journalString("run_id") == run)
        require(result.hasValidRunResultStructure()) { "Invalid terminal business result structure" }
        require(identity.journalString("conversation_id") == turn.conversationId && identity.journalString("run_id") == run
            && identity.journalString("assistant_message_id") == turn.assistantMessageId)
        require(expectedDigest == null || seal.journalString("seal_digest") == expectedDigest)
        turn.runId = run
        val admission = seal.getValue("admission").jsonObject
        require(admission["identity"] == identity)
        receiveAdmissions(turn, remote, current, admission)
        val context = query(journalRequest("context_usage") { put("identity", identity) })
        if (context != JsonNull) receiveContextUsage(turn, remote, current, context.jsonObject)
        val total = seal.journalNumber("entry_count")
        fun transition(type: String, page: JsonObject? = null): JsonObject = execute(journalCommand(type) {
            put("source_host_id", turn.hostId)
            if (page == null) put("seal", seal) else put("page", page)
        }).jsonObject.also { require(it["seal"] == seal) }
        var reception = transition("begin_receive_assistant_run_terminal")
        suspend fun page(after: ULong): JsonObject = query(journalRequest("terminal_manifest") {
            put("seal", seal); put("after_entry", after.toString()); put("max_records", 64); put("max_encoded_bytes", 131072)
        }).jsonObject.also { value ->
            require(value["seal"] == seal && value.journalNumber("after_entry") == after)
            val next = value.journalNumber("next_entry"); val entries = value.getValue("entries").jsonArray
            require(next > after && next <= total && next - after == entries.size.toULong() && entries.size <= 64)
            require(value.journalBoolean("complete") == (next == total))
        }
        while (!reception.journalBoolean("manifest_complete")) {
            val page = page(reception.journalNumber("next_entry"))
            reception = transition("stage_assistant_run_terminal_manifest", page)
            require(reception["next_entry"] == page["next_entry"] && reception["manifest_complete"] == page["complete"])
        }
        // Stage all owners before admitting any additional assistant journals.
        var after = 0uL; val heads = mutableListOf<JsonObject>()
        while (after < total) {
            val page = page(after)
            for (element in page.getValue("entries").jsonArray) {
                val entry = element.jsonObject
                if (entry.journalString("kind") == "assistant") {
                    entry.requireTerminalMessage()
                    val through = entry.getValue("through").jsonObject
                    require(through.getValue("identity").jsonObject["source_store_epoch"] == identity["source_store_epoch"])
                    receive(through, turn, remote, current); heads += through
                }
            }
            after = page.journalNumber("next_entry")
        }
        check(current()); currentCoroutineContext().ensureActive()
        reception = transition("commit_received_assistant_run_terminal")
        require(reception.journalBoolean("committed"))
        return heads
    }
}
