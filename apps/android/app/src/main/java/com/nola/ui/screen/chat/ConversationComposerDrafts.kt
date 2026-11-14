package com.nola.ui.screen.chat

import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.saveable.mapSaver
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.Job
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel

/** Each async importer closes over its own draft, never the selected screen. */
private class PersistedDraftState<T>(initial: T, private val changed: () -> Unit) : MutableState<T> {
    private val state = mutableStateOf(initial)
    var edited = false
        private set
    fun restoreIfUnedited(value: T) { if (!edited) state.value = value }
    override var value: T
        get() = state.value
        set(value) { edited = true; state.value = value; changed() }
    override fun component1(): T = value
    override fun component2(): (T) -> Unit = { value = it }
}

internal class ConversationComposerDraft(
    saved: ConversationDraftRepository.Snapshot = ConversationDraftRepository.Snapshot(),
    changed: () -> Unit = {},
) {
    enum class PersistenceStatus { Loading, Ready, Conflict, Error }
    val text: MutableState<String> = PersistedDraftState(saved.text, changed)
    val attachments: MutableState<List<AndroidChatAttachment>> = PersistedDraftState(saved.attachments, changed)
    val configuration: MutableState<ConversationDraftRepository.Configuration?> = PersistedDraftState(saved.configuration, changed)
    val notice = mutableStateOf<String?>(null)
    val persistenceError = mutableStateOf<String?>(null)
    val importing = mutableStateOf(false)
    val loading = mutableStateOf(false)
    val persistenceStatus = mutableStateOf(PersistenceStatus.Ready)
}

internal class ConversationComposerDrafts(private val repository: ConversationDraftRepository? = null) {
    data class ReadOwner(val key: String, val selectionGeneration: Long)
    data class SubmissionCapture internal constructor(val owner: String, internal val draft: ConversationComposerDraft?,
        internal val editRevision: Long, internal val save: Deferred<ConversationDraftRepository.SubmissionRevision>?,
        internal val loadedRevision: ConversationDraftRepository.SubmissionRevision?)
    data class SubmissionTicket internal constructor(internal val capture: SubmissionCapture,
        internal val frozen: ConversationDraftRepository.SubmissionRevision)
    private val callbacks = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    // Accepted keystroke saves finish even when the screen's ViewModel closes.
    private val saves = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val drafts = mutableStateMapOf<String, ConversationComposerDraft>()
    private val conversationOwners = mutableMapOf<String, String>()
    private val retired = mutableSetOf<String>()
    private val loads = mutableMapOf<String, Job>()
    private val pendingSaves = mutableSetOf<Job>()
    private var ownerRotation: Job? = null
    private val editRevisions = mutableMapOf<String, Long>()
    private val lastSaves = mutableMapOf<String, Deferred<ConversationDraftRepository.SubmissionRevision>>()
    private val loadedRevisions = mutableMapOf<String, ConversationDraftRepository.SubmissionRevision>()
    private val pending = mutableStateOf(UUID.randomUUID().toString())
    private var initialized = repository == null
    val initializing = mutableStateOf(repository != null)
    val initializationError = mutableStateOf<String?>(null)
    suspend fun initialize() {
        if (initialized) return
        try {
            conversationOwners.putAll(repository?.conversationOwners().orEmpty())
            pending.value = repository?.pendingOwner() ?: pending.value
            initialized = true
            forConversation(null)
            loads[key(null)]?.join()
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            initializationError.value = error.localizedMessage
        } finally { initializing.value = false }
    }
    fun close() {
        for ((owner, job) in loads) {
            val draft = drafts[owner]
            val edited = (draft?.text as? PersistedDraftState<*>)?.edited == true ||
                (draft?.attachments as? PersistedDraftState<*>)?.edited == true ||
                (draft?.configuration as? PersistedDraftState<*>)?.edited == true
            if (!edited) job.cancel()
        }
        val pending = loads.values.toList()
        saves.launch { pending.forEach { it.join() }; callbacks.cancel() }
    }
    suspend fun awaitReady(id: String?): ConversationComposerDraft {
        val draft = forConversation(id)
        loads[key(id)]?.join()
        return draft
    }
    suspend fun flush() {
        ownerRotation?.join()
        loads.values.toList().forEach { it.join() }
        pendingSaves.toList().forEach { it.join() }
    }
    private var pickerOwner: String? = null
    private fun key(id: String?): String = id?.let { conversationOwners[it] ?: "conversation:$it" } ?: "draft:${pending.value}"
    suspend fun bindConversation(id: String, expectedOwner: String) {
        require(expectedOwner.startsWith("draft:") && drafts.containsKey(expectedOwner) && expectedOwner !in retired)
        loads[expectedOwner]?.join()
        check(drafts.getValue(expectedOwner).persistenceError.value == null) { "Draft persistence failed; your local input is retained" }
        flush()
        repository?.bindConversation(id, expectedOwner)
        conversationOwners[id] = expectedOwner
    }
    // Execution identity is canonical. Disk ownership may retain the original
    // pending draft to preserve edits/imports across metadata preparation.
    fun ownerKey(id: String?): String = id?.let { "conversation:$it" } ?: key(null)
    private fun storageOwner(owner: String): String = if (owner.startsWith("conversation:"))
        conversationOwners[owner.removePrefix("conversation:")] ?: owner else owner
    fun captureRead(id: String?, generation: Long) = ReadOwner(key(id), generation)
    fun ownsRead(read: ReadOwner, id: String?, generation: Long): Boolean =
        read.key == key(id) && read.selectionGeneration == generation
    fun shouldRestorePending(): Boolean = forConversation(null).let {
        it.text.value.isNotEmpty() || it.attachments.value.isNotEmpty() || it.configuration.value?.userConfigured == true
    }
    fun captureSubmission(owner: String): SubmissionCapture = storageOwner(owner).let { storage ->
        SubmissionCapture(storage, drafts[storage], editRevisions[storage] ?: 0, lastSaves[storage], loadedRevisions[storage])
    }
    suspend fun prepareSubmission(capture: SubmissionCapture): SubmissionTicket {
        check(initialized && capture.draft != null && !capture.draft.loading.value) { "Draft is still loading" }
        check(capture.draft.persistenceError.value == null) { "Draft persistence failed; your local input is retained" }
        val frozen = capture.save?.await() ?: capture.loadedRevision
            ?: if (repository == null) ConversationDraftRepository.SubmissionRevision(capture.owner, null)
                else error("Draft has no captured durable version")
        repository?.validateForSubmission(frozen)
        return SubmissionTicket(capture, frozen)
    }
    suspend fun submitted(ticket: SubmissionTicket): Boolean {
        val capture = ticket.capture
        val owner = capture.owner
        if (drafts[owner] !== capture.draft || (editRevisions[owner] ?: 0) != capture.editRevision) return false
        val retiredDisk = repository?.retireForSubmission(ticket.frozen) ?: true
        // An ACK for an earlier edit version cannot clear equal-text re-edits,
        // newly attached files, or a successor draft created during IO.
        if (!retiredDisk || drafts[owner] !== capture.draft || (editRevisions[owner] ?: 0) != capture.editRevision) return false
        if (owner.startsWith("draft:") && repository == null) retired += owner
        drafts.remove(owner)
        lastSaves.remove(owner); loadedRevisions.remove(owner)
        if (pickerOwner == owner) pickerOwner = null
        // A durable pending owner stays writable across an ACK racing new input.
        // Submission identity is independent; only explicit deletion tombstones it.
        if (owner == key(null)) pending.value = repository?.pendingOwner() ?: UUID.randomUUID().toString()
        return true
    }
    fun restoreBlockedSubmission(ticket: SubmissionTicket, conversationId: String): Boolean {
        val capture=ticket.capture
        if(ownerKey(conversationId)!=capture.owner || (editRevisions[capture.owner]?:0)!=capture.editRevision)return false
        val original=capture.draft?:return false
        val current=forConversation(conversationId)
        if(current.text.value.isNotEmpty()||current.attachments.value.isNotEmpty())return false
        current.text.value=original.text.value;current.attachments.value=original.attachments.value
        return true
    }
    fun forConversation(id: String?): ConversationComposerDraft {
        val owner = key(id)
        return drafts.getOrPut(owner) {
            lateinit var draft: ConversationComposerDraft
            draft = ConversationComposerDraft() {
                editRevisions[owner] = (editRevisions[owner] ?: 0) + 1
                if (repository != null && owner !in retired && drafts[owner] === draft) {
                    // A read must finish before edits can acquire the observed
                    // CAS revision. Never save an empty loading placeholder.
                    if (draft.loading.value) return@ConversationComposerDraft
                    val snapshot = ConversationDraftRepository.Snapshot(draft.text.value, draft.attachments.value,
                        configuration = draft.configuration.value.takeIf { owner.startsWith("draft:") })
                    val save = saves.async {
                        try {
                            val revision = repository.conditionalSave(owner, snapshot)
                            if (owner !in retired && drafts[owner] === draft) {
                                draft.persistenceStatus.value = ConversationComposerDraft.PersistenceStatus.Ready
                                draft.persistenceError.value = null
                            }
                            revision
                        }
                        catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
                        catch (error: Exception) {
                            if (owner !in retired && drafts[owner] === draft) {
                                draft.persistenceError.value = error.localizedMessage
                                draft.persistenceStatus.value = if (error is ConversationDraftRepository.Conflict)
                                    ConversationComposerDraft.PersistenceStatus.Conflict else ConversationComposerDraft.PersistenceStatus.Error
                            }
                            throw error
                        }
                    }
                    lastSaves[owner] = save
                    pendingSaves.add(save)
                    save.invokeOnCompletion { pendingSaves.remove(save) }
                }
            }
            draft.loading.value = repository != null
            if (repository != null) draft.persistenceStatus.value = ConversationComposerDraft.PersistenceStatus.Loading
            if (repository != null) loads[owner] = callbacks.launch { loadDraft(owner, draft) }
            draft
        }
    }

    private suspend fun loadDraft(owner: String, draft: ConversationComposerDraft) {
        if (repository == null) return
        try {
            val saved = repository.load(owner)
            var observedRevision = repository.submissionRevision(owner)
            if (owner in retired || drafts[owner] !== draft) return
            // Inputs entered while IO was pending win over the restored text.
            // They remain local until this read establishes the CAS revision.
            val text = draft.text as PersistedDraftState<String>
            val attachments = draft.attachments as PersistedDraftState<List<AndroidChatAttachment>>
            val configuration = draft.configuration as PersistedDraftState<ConversationDraftRepository.Configuration?>
            text.restoreIfUnedited(saved?.text.orEmpty())
            attachments.restoreIfUnedited(saved?.attachments.orEmpty())
            configuration.restoreIfUnedited(saved?.configuration)
            if (text.edited || attachments.edited || configuration.edited) observedRevision = repository.conditionalSave(owner,
                ConversationDraftRepository.Snapshot(text.value, attachments.value,
                    configuration = configuration.value.takeIf { owner.startsWith("draft:") }))
            loadedRevisions[owner] = observedRevision
            if ((saved?.missingAttachments ?: 0) > 0) draft.notice.value = "Some attachments could not be restored. Please attach them again."
            draft.persistenceStatus.value = ConversationComposerDraft.PersistenceStatus.Ready
        } catch (cancelled: kotlinx.coroutines.CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            draft.persistenceError.value = error.localizedMessage
            draft.persistenceStatus.value = if (error is ConversationDraftRepository.Conflict)
                ConversationComposerDraft.PersistenceStatus.Conflict else ConversationComposerDraft.PersistenceStatus.Error
        } finally {
            draft.loading.value = false
        }
    }

    fun beginNewDraft(ready: () -> Unit = {}) {
        if (repository == null) { pending.value = UUID.randomUUID().toString(); ready(); return }
        initializing.value = true
        ownerRotation = callbacks.launch {
            try { pending.value = repository.newPendingOwner(); ready() }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (error: Exception) { initializationError.value = error.localizedMessage }
            finally { initializing.value = false }
        }
    }

    fun pendingConfiguration(): ConversationDraftRepository.Configuration? = forConversation(null).configuration.value

    fun savePendingConfiguration(owner: String, configuration: ConversationDraftRepository.Configuration) {
        if (!initializing.value && owner == key(null) && owner !in retired) forConversation(null).configuration.value = configuration
    }

    fun remove(id: String) {
        val owner = key(id)
        retired += owner
        loads.remove(owner)?.cancel()
        drafts.remove(owner)
        if (pickerOwner == owner) pickerOwner = null
        if (repository != null) callbacks.launch { repository.remove(owner) }
    }

    fun beginPicker(id: String?) {
        forConversation(id)
        pickerOwner = key(id)
    }

    fun consumePicker(): ConversationComposerDraft? {
        val owner = pickerOwner ?: return null
        pickerOwner = null
        return if (owner in retired) null else drafts[owner]
    }

    companion object {
        // Production uses the ViewModel-owned repository. Nonpersistent/test
        // hosts may still preserve picker ownership across recreation.
        val Saver = mapSaver(
            save = { store: ConversationComposerDrafts ->
                store.drafts.mapValues { (_, draft) -> draft.text.value }
                    .toMutableMap().also { saved ->
                        saved["pendingOwner"] = store.pending.value
                        store.pickerOwner?.let { saved["pickerOwner"] = it }
                    }
            },
            restore = { saved -> ConversationComposerDrafts().also { store ->
                store.pending.value = saved["pendingOwner"] as? String ?: UUID.randomUUID().toString()
                store.pickerOwner = saved["pickerOwner"] as? String
                saved.forEach { (key, value) ->
                    if (key != "pickerOwner" && key != "pendingOwner") store.drafts[key] = ConversationComposerDraft().also { it.text.value = value as String }
                }
            } },
        )
    }
}
