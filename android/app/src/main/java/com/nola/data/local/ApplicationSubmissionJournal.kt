package com.nola.data.local

import kotlinx.serialization.json.*
import java.io.File
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.NoSuchFileException
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption.*
import java.nio.file.attribute.PosixFilePermissions
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

/** Exact transport envelopes survive ambiguous native outcomes and process restarts. */
internal class ApplicationSubmissionJournal(
    private val file: File,
    private val privateCommandScope: ((JsonObject) -> ApplicationCommandScope)? = null,
    private val resolveRun: (String) -> String? = { null },
) {
    private val json = Json
    private val path = file.toPath().toAbsolutePath().normalize()
    private val processLock = locks.computeIfAbsent(path.toString()) { ReentrantLock() }
    private lateinit var state: JsonObject
    private var inTransaction = false
    private var storageUncertain = false

    /** Runtime holds this across load → receipt lookup/send → retirement. */
    fun <T> transaction(block: () -> T): T = processLock.withLock {
        check(!storageUncertain) { "Application intent durability is uncertain" }
        if (inTransaction) return@withLock block()
        Files.createDirectories(path.parent)
        check(!Files.isSymbolicLink(path.parent) && Files.isDirectory(path.parent, NOFOLLOW_LINKS))
        val directoryPermissions = Files.getPosixFilePermissions(path.parent, NOFOLLOW_LINKS)
        check(java.nio.file.attribute.PosixFilePermission.GROUP_WRITE !in directoryPermissions &&
            java.nio.file.attribute.PosixFilePermission.OTHERS_WRITE !in directoryPermissions)
        val lockPath = path.resolveSibling(path.fileName.toString() + ".lock")
        FileChannel.open(lockPath, setOf<java.nio.file.OpenOption>(CREATE, READ, WRITE, NOFOLLOW_LINKS),
            PosixFilePermissions.asFileAttribute(privatePermissions)).use { channel ->
            validatePrivate(lockPath)
            channel.lock().use {
                inTransaction = true
                try {
                    state = load() ?: buildJsonObject {
                        put("client_id", "android-${UUID.randomUUID()}")
                        put("pending", buildJsonObject {})
                    }.also { persistState(it) }
                    block()
                } finally { inTransaction = false }
            }
        }
    }

    val clientId: String get() = transaction { state.getValue("client_id").jsonPrimitive.content }
    fun pending(): List<JsonObject> = transaction { state.getValue("pending").jsonObject.values.map { it.jsonObject } }

    fun prepare(command: JsonObject, version: Int, requestId: String?, expectedRevision: Long?): JsonObject = transaction {
        val transport = buildJsonObject {
            put("type", command.getValue("type"))
            put("arguments", JsonObject(command.filterKeys { it != "type" }))
        }
        val existing = pending().firstOrNull { envelope ->
            if (requestId != null) envelope.getValue("request_id").jsonPrimitive.content == requestId
            else envelope["command"] == transport &&
                envelope["expected_revision"]?.jsonPrimitive?.longOrNull == expectedRevision
        }
        if (existing != null) {
            require(existing["protocol_version"]?.jsonPrimitive?.intOrNull == version && existing["command"] == transport &&
                existing["expected_revision"]?.jsonPrimitive?.longOrNull == expectedRevision) {
                "Pending request ID cannot be reused with a changed command"
            }
            return@transaction existing
        }
        val scope = privateCommandScope?.invoke(command) ?: ApplicationCommandScope.command(command, resolveRun)
        pending().firstOrNull { envelopeScope(it).conflicts(scope) }?.let {
            throw PendingSubmission(it.getValue("request_id").jsonPrimitive.content, "An earlier command must finish before another is admitted")
        }
        val request = buildJsonObject {
            put("protocol_version", version)
            put("client_id", clientId)
            put("request_id", requestId ?: UUID.randomUUID().toString())
            expectedRevision?.let { put("expected_revision", it) }
            put("command", transport)
        }
        val requests = state.getValue("pending").jsonObject.toMutableMap()
        check(requests.size < MAX_PENDING) { "Application request journal is full" }
        requests[request.getValue("request_id").jsonPrimitive.content] = request
        persist(requests)
        request
    }

    private fun envelopeScope(envelope: JsonObject): ApplicationCommandScope {
        val classifier = privateCommandScope ?: return ApplicationCommandScope.envelope(envelope, resolveRun)
        val command = envelope.getValue("command").jsonObject
        return classifier(JsonObject(command.getValue("arguments").jsonObject + ("type" to command.getValue("type"))))
    }

    fun acceptResponse(request: JsonObject, response: ByteArray?): JsonObject {
        val receipt = ApplicationReceiptValidation.decode(request, response)
        return acceptReceipt(request, receipt)
    }

    fun acceptReceipt(request: JsonObject, receipt: JsonObject, validateResult: (JsonObject) -> Unit = {}): JsonObject = transaction {
        val id = request.getValue("request_id").jsonPrimitive.content
        require(pending().singleOrNull { it["request_id"] == request["request_id"] } == request) { "Receipt has no exact pending command" }
        ApplicationReceiptValidation.validate(request, receipt)
        if (receipt.getValue("status").jsonPrimitive.content == "accepted") {
            throw PendingSubmission(id, "Command has no terminal receipt")
        }
        validateResult(receipt)
        remove(id)
        receipt
    }

    /** Observations have no write outcome to reconcile or serialize behind. */
    fun retireObservation(request: JsonObject) = transaction {
        val wire = request.getValue("command").jsonObject
        val command = JsonObject(wire.getValue("arguments").jsonObject + ("type" to wire.getValue("type")))
        require(ApplicationCommandEffect.isObservation(command))
        val id = request.getValue("request_id").jsonPrimitive.content
        require(pending().singleOrNull { it["request_id"] == request["request_id"] } == request)
        remove(id)
    }

    /** Used only by the separate native Host journal, never the renderer command journal. */
    fun acceptExpertHostResponse(request: JsonObject, response: ByteArray?, validateResult: (JsonElement) -> Unit = {}): JsonObject = transaction {
        require(request.getValue("command").jsonObject.getValue("type") == JsonPrimitive("native_expert_host_operation"))
        val id = request.getValue("request_id").jsonPrimitive.content
        require(pending().singleOrNull { it["request_id"] == JsonPrimitive(id) } == request)
        val receipt = try {
            checkNotNull(response).decodeToString(throwOnInvalidSequence = true).let { Json.parseToJsonElement(it).jsonObject }
        } catch (_: Exception) { throw PendingSubmission(id, "Native Host response is unavailable or malformed") }
        val ok = receipt["ok"]
        if (ok == JsonPrimitive(true) && receipt.keys == setOf("ok", "result", "request_id") && receipt["request_id"] == JsonPrimitive(id)) {
            try { validateResult(receipt.getValue("result")) }
            catch (_: Exception) { throw PendingSubmission(id, "Native Host result has not been validated") }
            remove(id)
            return@transaction receipt
        }
        val error = receipt["error"] as? JsonObject
        if (ok != JsonPrimitive(false) || receipt.keys != setOf("ok", "error") || error == null ||
            !setOf("code", "message", "outcome_pending", "request_id", "reopen_required").containsAll(error.keys) ||
            error["request_id"] != JsonPrimitive(id) ||
            (error["code"] as? JsonPrimitive)?.isString != true || (error["message"] as? JsonPrimitive)?.isString != true ||
            error["outcome_pending"] !in setOf(JsonPrimitive(true), JsonPrimitive(false)) ||
            ("reopen_required" in error && error["reopen_required"] !in setOf(JsonPrimitive(true), JsonPrimitive(false)))) {
            throw PendingSubmission(id, "Native Host response has no exact definitive outcome")
        }
        if (error["outcome_pending"] == JsonPrimitive(true)) return@transaction receipt
        remove(id)
        receipt
    }

    /** Retire a client-owned intent only after its transport reports a definitive outcome. */
    fun retireUiIntent(request: JsonObject) = transaction {
        require(request.getValue("command").jsonObject.getValue("type").jsonPrimitive.content == "submit_ui_interaction")
        val id = request.getValue("request_id").jsonPrimitive.content
        require(pending().singleOrNull { it["request_id"] == request["request_id"] } == request)
        remove(id)
    }

    private fun remove(id: String) {
        val requests = state.getValue("pending").jsonObject.toMutableMap()
        requests.remove(id)
        persist(requests)
    }

    private fun persist(requests: Map<String, JsonElement>) {
        val updated = buildJsonObject {
            put("client_id", clientId)
            put("pending", JsonObject(requests))
        }
        persistState(updated)
        state = updated
    }

    private fun persistState(updated: JsonObject) {
        validateState(updated)
        val bytes = updated.toString().encodeToByteArray()
        require(bytes.size <= MAX_BYTES) { "Application request journal is full" }
        var temporary: java.nio.file.Path? = null
        try {
            temporary = Files.createTempFile(path.parent, ".application-intent-", ".tmp",
                PosixFilePermissions.asFileAttribute(privatePermissions))
            FileChannel.open(temporary, WRITE, NOFOLLOW_LINKS).use { channel ->
                val buffer = ByteBuffer.wrap(bytes)
                while (buffer.hasRemaining()) channel.write(buffer)
                channel.force(true)
            }
            Files.move(temporary, path, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
            syncDirectory()
        } catch (failure: Exception) {
            storageUncertain = true
            throw failure
        } finally { temporary?.let { Files.deleteIfExists(it) } }
    }

    private fun load(): JsonObject? {
        val channel = try { FileChannel.open(path, READ, WRITE, NOFOLLOW_LINKS) }
            catch (_: NoSuchFileException) { return null }
        return channel.use {
            validatePrivate(path)
            require(channel.size() <= MAX_BYTES) { "Application request journal is too large" }
            // Confirm a previous instance's visible rename before interpreting it.
            channel.force(true)
            syncDirectory()
            val output = ByteArrayOutputStream()
            val buffer = ByteBuffer.allocate(64 * 1024)
            while (output.size() <= MAX_BYTES) {
                buffer.clear()
                buffer.limit(minOf(buffer.capacity(), MAX_BYTES + 1 - output.size()))
                val read = channel.read(buffer)
                if (read < 0) break
                if (read > 0) output.write(buffer.array(), 0, read)
            }
            require(output.size() <= MAX_BYTES) { "Application request journal is too large" }
            json.parseToJsonElement(output.toByteArray().decodeToString(throwOnInvalidSequence = true)).jsonObject
                .also(::validateState)
        }
    }

    private fun validateState(value: JsonObject) {
        require(value.keys == setOf("client_id", "pending"))
        val client = value.getValue("client_id").jsonPrimitive
        require(client.isString && client.content.isNotBlank() && client.content.length <= 512)
        val requests = value.getValue("pending").jsonObject
        require(requests.size <= MAX_PENDING) { "Application journal has too many unresolved commands" }
        requests.forEach { (id, item) ->
            val envelope = item.jsonObject
            require(id.isNotBlank() && envelope["request_id"] == JsonPrimitive(id) && envelope["client_id"] == client)
            require(envelope["command"] is JsonObject)
            require(envelope.toString().encodeToByteArray().size <= ApplicationReceiptValidation.MAX_FRAME_BYTES)
        }
    }

    private fun validatePrivate(target: java.nio.file.Path) {
        check(Files.isRegularFile(target, NOFOLLOW_LINKS) && Files.getPosixFilePermissions(target, NOFOLLOW_LINKS) == privatePermissions)
        check(Files.getOwner(target, NOFOLLOW_LINKS) == Files.getOwner(path.parent, NOFOLLOW_LINKS))
    }

    private fun syncDirectory() { FileChannel.open(path.parent, READ, NOFOLLOW_LINKS).use { it.force(true) } }

    class PendingSubmission(val requestId: String, reason: String) :
        IllegalStateException("Command outcome pending: $requestId ($reason)")

    private companion object {
        const val MAX_PENDING = 64
        // Bounded aggregate storage, regardless of the number of independent owners.
        const val MAX_BYTES = 2 * ApplicationReceiptValidation.MAX_FRAME_BYTES + 2048
        val privatePermissions = PosixFilePermissions.fromString("rw-------")
        val locks = ConcurrentHashMap<String, ReentrantLock>()
    }
}
