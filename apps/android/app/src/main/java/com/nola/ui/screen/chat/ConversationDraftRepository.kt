package com.nola.ui.screen.chat

import java.io.File
import java.util.UUID
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.withContext

/** Device-local store scope is the app's private canonical-store container.
 * Conversation IDs are local canonical IDs, never unscoped remote Run IDs. */
internal class ConversationDraftRepository(private val root: File, private val attachmentAuthority: String) {
    companion object {
        private val disk = java.util.concurrent.Executors.newSingleThreadExecutor { task ->
            Thread(task, "conversation-drafts-disk").apply { isDaemon = true }
        }
        private val dispatcher = disk.asCoroutineDispatcher()
    }
    @Serializable data class Configuration(val model: com.nola.domain.model.ModelInfo?, val parameters: com.nola.domain.model.ModelParameters,
        val userConfigured: Boolean = true, val hostBinding: com.nola.domain.model.ExecutionHostBinding? = null)
    @Serializable data class Snapshot(val text: String = "", val attachments: List<AndroidChatAttachment> = emptyList(), val missingAttachments: Int = 0, val configuration: Configuration? = null)
    data class SubmissionRevision(val owner: String, val revision: Long?)
    @Serializable private data class AttachmentRef(val id: String, val digest: String)
    @Serializable private data class Manifest(val owner: String, val revision: Long, val deleted: Boolean = false,
        val text: String = "", val attachments: List<AttachmentRef> = emptyList(), val configuration: Configuration? = null)
    class Conflict : IllegalStateException("Draft changed in another window or was deleted. Your local input is preserved; copy it before reopening.")
    private val revisions = mutableMapOf<String, Long?>()
    private val json = Json { ignoreUnknownKeys = false }
    private fun digest(value: String): String = java.security.MessageDigest.getInstance("SHA-256").digest(value.toByteArray())
            .joinToString("") { "%02x".format(it) }
    private fun file(key: String): File = File(root, "${digest(key)}.json")
    private fun atomicWrite(file: File, text: String) {
        check(root.isDirectory || root.mkdirs())
        val temporary = File.createTempFile("draft-", ".tmp", root)
        try {
            temporary.outputStream().use { it.write(text.toByteArray()); it.fd.sync() }
            java.nio.file.Files.move(temporary.toPath(), file.toPath(),
                java.nio.file.StandardCopyOption.REPLACE_EXISTING, java.nio.file.StandardCopyOption.ATOMIC_MOVE)
        } finally { temporary.delete() }
    }
    private fun <T> locked(body: () -> T): T {
        check(root.isDirectory || root.mkdirs())
        return java.io.RandomAccessFile(File(root, ".lock"), "rw").use { handle ->
            handle.channel.lock().use { body() }
        }
    }
    // UI ownership only: a prepared empty conversation keeps the exact same
    // draft object/files, including import callbacks and CAS revisions.
    private fun bindings(): Map<String, String> = File(root, "conversation-owners.json").takeIf { it.exists() }
        ?.let { json.decodeFromString<Map<String, String>>(it.readText()) }.orEmpty().also { values ->
            require(values.keys.all { it.isNotBlank() } && values.values.all { it.startsWith("draft:") })
            require(values.values.distinct().size == values.size)
        }
    suspend fun conversationOwners(): Map<String, String> = withContext(dispatcher) { locked { bindings() } }
    suspend fun bindConversation(conversationId: String, owner: String) = withContext(dispatcher) { locked {
        require(conversationId.isNotBlank() && owner.startsWith("draft:"))
        val current = bindings()
        current[conversationId]?.let { require(it == owner); return@locked }
        require(owner !in current.values && manifest("conversation:$conversationId") == null)
        validateRevision(owner)
        atomicWrite(File(root, "conversation-owners.json"), json.encodeToString(current + (conversationId to owner)))
    } }
    suspend fun pendingOwner(): String = withContext(dispatcher) { locked {
        val index = File(root, "pending-owner.json")
        val owner = if (index.exists()) json.decodeFromString<String>(index.readText()).also { UUID.fromString(it) } else null
        if (owner == null || manifest("draft:$owner")?.deleted == true || "draft:$owner" in bindings().values) createPendingOwner() else owner
    } }
    private fun createPendingOwner(): String = UUID.randomUUID().toString().also {
        atomicWrite(File(root, "pending-owner.json"), json.encodeToString(it))
    }
    suspend fun newPendingOwner(): String = withContext(dispatcher) { locked { createPendingOwner() } }
    private fun manifest(key: String): Manifest? = file(key).takeIf { it.exists() }?.let {
        json.decodeFromString<Manifest>(it.readText()).also { value -> require(value.owner == key && value.revision > 0) }
    }
    suspend fun load(key: String): Snapshot? = withContext(dispatcher) { locked { loadFromDisk(key) } }
    private fun loadFromDisk(key: String): Snapshot? {
        val manifest = manifest(key)
        revisions[key] = manifest?.revision
        if (manifest == null || manifest.deleted) return null
        var missing = 0
        val attachments = manifest.attachments.mapNotNull { reference ->
            try {
            val id = reference.id
            UUID.fromString(id)
            val encoded = file("attachment:$id:${reference.digest}").readText()
            require(digest(encoded) == reference.digest)
            json.decodeFromString<AndroidChatAttachment>(encoded).also { attachment ->
                require(attachment.id == id && attachment.uri.startsWith("content://$attachmentAuthority/chat_attachments/"))
            }
            } catch (_: Exception) { missing++; null }
        }
        return Snapshot(manifest.text, attachments, missing, manifest.configuration.takeIf { key.startsWith("draft:") })
    }
    suspend fun conditionalSave(key: String, value: Snapshot) = withContext(dispatcher) {
        locked { saveToDisk(key, value) }
    }
    private fun saveToDisk(key: String, value: Snapshot): SubmissionRevision {
        // Imported files must be app-owned content references, never external
        // provider grants whose lifetime ends at process restart.
        require(value.attachments.all { it.uri.startsWith("content://$attachmentAuthority/chat_attachments/") })
        require(value.text.toByteArray().size <= 4_000_000 && value.attachments.size <= 100)
        require(json.encodeToString(value).toByteArray().size <= 8_000_000)
        require(value.configuration == null || key.startsWith("draft:"))
        value.configuration?.hostBinding?.let { com.nola.data.repository.validateExecutionHostBinding(it) }
        val previous = manifest(key)
        if (previous?.deleted == true || previous?.revision != revisions[key]) throw Conflict()
        val revision = Math.addExact(previous?.revision ?: 0L, 1L)
        val references = value.attachments.map { attachment ->
            UUID.fromString(attachment.id)
            val encoded = json.encodeToString(attachment)
            val reference = AttachmentRef(attachment.id, digest(encoded))
            val resource = file("attachment:${reference.id}:${reference.digest}")
            if (!resource.exists()) atomicWrite(resource, encoded)
            else require(resource.readText() == encoded)
            reference
        }
        atomicWrite(file(key), json.encodeToString(Manifest(key, revision, text = value.text,
            attachments = references, configuration = value.configuration)))
        revisions[key] = revision
        return SubmissionRevision(key, revision)
    }
    suspend fun remove(key: String) = withContext(dispatcher) { locked { removeFromDisk(key) } }
    private fun validateRevision(key: String) {
        val current = manifest(key)
        if (current?.deleted == true || current?.revision != revisions[key]) throw Conflict()
    }
    suspend fun submissionRevision(key: String): SubmissionRevision = withContext(dispatcher) { locked {
        validateRevision(key)
        SubmissionRevision(key, revisions[key])
    } }
    suspend fun validateForSubmission(frozen: SubmissionRevision) = withContext(dispatcher) { locked {
        val current = manifest(frozen.owner)
        // Later accepted edits by this repository may coexist with the frozen
        // send. A foreign window or tombstone still invalidates the read.
        if (current?.deleted == true || (current?.revision != frozen.revision && current?.revision != revisions[frozen.owner])) throw Conflict()
    } }
    suspend fun retireForSubmission(frozen: SubmissionRevision): Boolean = withContext(dispatcher) { locked {
        val current = manifest(frozen.owner)
        if (current?.deleted == true || current?.revision != frozen.revision) return@locked false
        // Submission clears exactly this version, not the owner. A new edit
        // queued while this IO runs must remain writable, including pending drafts.
        // Explicit deletion alone creates a tombstone.
        saveToDisk(frozen.owner, Snapshot())
        true
    } }
    private fun removeFromDisk(key: String) {
            val revision = Math.addExact(manifest(key)?.revision ?: 0L, 1L)
            atomicWrite(file(key), json.encodeToString(Manifest(key, revision, deleted = true)))
            revisions[key] = revision
            if (key.startsWith("draft:")) {
                val index = File(root, "pending-owner.json")
                val current = if (index.exists()) json.decodeFromString<String>(index.readText()) else null
                if (key == "draft:$current") createPendingOwner()
            }
    }
}
