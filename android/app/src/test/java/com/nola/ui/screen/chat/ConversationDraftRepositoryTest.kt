package com.nola.ui.screen.chat

import java.nio.file.Files
import org.junit.Assert.*
import org.junit.Test
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import org.junit.Before
import org.junit.After

class ConversationDraftRepositoryTest {
    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    @Before fun installDispatcher() { Dispatchers.setMain(UnconfinedTestDispatcher()) }
    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    @After fun removeDispatcher() { Dispatchers.resetMain() }
    @Test fun preparingConversationKeepsExactDraftAndRestoresBindingWithoutCopyingInput() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-bind").toFile()
        val drafts = ConversationComposerDrafts(ConversationDraftRepository(root, "fixture"))
        try {
            drafts.initialize()
            val draft = drafts.awaitReady(null)
            val owner = drafts.ownerKey(null)
            draft.text.value = "not sent"
            drafts.bindConversation("prepared", owner)
            assertSame(draft, drafts.forConversation("prepared"))
            assertEquals("conversation:prepared", drafts.ownerKey("prepared"))
            // An importer already holding this object keeps writing to its owner.
            draft.text.value += " plus late input"
            drafts.flush()
            val restored = ConversationComposerDrafts(ConversationDraftRepository(root, "fixture"))
            try {
                restored.initialize()
                assertNotEquals(owner, restored.ownerKey(null))
                assertEquals("conversation:prepared", restored.ownerKey("prepared"))
                assertEquals("not sent plus late input", restored.awaitReady("prepared").text.value)
                assertEquals("", restored.awaitReady(null).text.value)
                val ticket = restored.prepareSubmission(restored.captureSubmission(restored.ownerKey("prepared")))
                restored.forConversation("prepared").text.value = "successor edit"
                assertFalse(restored.submitted(ticket))
                assertEquals("successor edit", restored.forConversation("prepared").text.value)
                restored.flush()
            } finally { restored.close() }
        } finally { drafts.close(); root.deleteRecursively() }
    }

    @Test fun draftCannotBeBoundToTwoConversationsOrOverwriteAnotherOwner() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-bind-conflict").toFile()
        try {
            val repository = ConversationDraftRepository(root, "fixture")
            val owner = "draft:${repository.pendingOwner()}"
            repository.load(owner)
            repository.bindConversation("first", owner)
            try { repository.bindConversation("second", owner); fail("double owner") } catch (_: IllegalArgumentException) { }
            assertEquals(mapOf("first" to owner), repository.conversationOwners())
        } finally { root.deleteRecursively() }
    }

    @Test fun asynchronousRestorePreservesInputAndExplicitClearingDuringLoad() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-loading-edit").toFile()
        val drafts = ConversationComposerDrafts(ConversationDraftRepository(root, "fixture"))
        try {
            val writer = ConversationDraftRepository(root, "fixture")
            writer.conditionalSave("conversation:a", ConversationDraftRepository.Snapshot("old disk text"))
            writer.conditionalSave("conversation:b", ConversationDraftRepository.Snapshot("must stay cleared"))
            drafts.initialize()
            val a = drafts.forConversation("a")
            a.text.value = "new typing"
            assertSame(a, drafts.awaitReady("a"))
            assertEquals("new typing", a.text.value)
            val b = drafts.forConversation("b")
            b.text.value = ""
            drafts.awaitReady("b")
            assertEquals("", b.text.value)
            assertEquals("new typing", writer.load("conversation:a")?.text)
            assertEquals("", writer.load("conversation:b")?.text)
        } finally { drafts.close(); root.deleteRecursively() }
    }

    @Test fun staleSubmissionCannotRetireAnotherWindowsText() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-retire-cas").toFile()
        try {
            val first = ConversationDraftRepository(root, "fixture")
            val stale = ConversationDraftRepository(root, "fixture")
            val drafts = ConversationComposerDrafts(stale)
            drafts.initialize()
            val local = drafts.awaitReady(null)
            val owner = drafts.ownerKey(null)
            val capture = drafts.captureSubmission(owner)
            val ticket = drafts.prepareSubmission(capture)
            first.load(owner)
            first.conditionalSave(owner, ConversationDraftRepository.Snapshot("newer other window"))
            first.load(owner)
            try { drafts.prepareSubmission(capture); fail("Expected conflict") } catch (_: ConversationDraftRepository.Conflict) { }
            assertFalse(drafts.submitted(ticket))
            assertSame(local, drafts.forConversation(null))
            assertEquals(owner, drafts.ownerKey(null))
            assertEquals("newer other window", first.load(owner)?.text)
            drafts.close()
        } finally { root.deleteRecursively() }
    }
    @Test fun restartRepairsRetiredPendingIndexAndRemovalDoesNotReplaceAnotherWindowOwner() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-pending-recovery").toFile()
        try {
            val first = ConversationDraftRepository(root, "fixture")
            val retired = first.pendingOwner()
            first.remove("draft:$retired")
            assertNotEquals(retired, first.pendingOwner())
            // Simulate process death after tombstone commit but before index replacement.
            java.io.File(root, "pending-owner.json").writeText("\"$retired\"")
            val restarted = ConversationDraftRepository(root, "fixture")
            val recovered = restarted.pendingOwner()
            assertNotEquals(retired, recovered)
            restarted.conditionalSave("draft:$recovered", ConversationDraftRepository.Snapshot("recoverable"))
            assertEquals("recoverable", restarted.load("draft:$recovered")?.text)
            val anotherWindow = restarted.newPendingOwner()
            first.remove("draft:$recovered")
            assertEquals(anotherWindow, restarted.pendingOwner())
        } finally { root.deleteRecursively() }
    }
    @Test fun pendingDefaultsDoNotReplaceRecentConversationButExplicitSettingsDo() {
        val drafts = ConversationComposerDrafts()
        val owner = drafts.ownerKey(null)
        val automatic = ConversationDraftRepository.Configuration(null,
            com.nola.domain.model.ModelParameters(), userConfigured = false)
        drafts.savePendingConfiguration(owner, automatic)
        assertFalse(drafts.shouldRestorePending())
        drafts.savePendingConfiguration(owner, automatic.copy(userConfigured = true))
        assertTrue(drafts.shouldRestorePending())
        drafts.beginNewDraft()
        drafts.forConversation(null).text.value = "unsubmitted"
        assertTrue(drafts.shouldRestorePending())
    }

    @Test fun directoryReadCannotApplyDefaultsAfterPendingOwnerOrSelectionChanges() {
        val drafts = ConversationComposerDrafts()
        val read = drafts.captureRead(null, 1)
        assertTrue(drafts.ownsRead(read, null, 1))
        assertFalse(drafts.ownsRead(read, null, 2))
        assertFalse(drafts.ownsRead(read, "existing", 1))
        drafts.beginNewDraft()
        assertFalse(drafts.ownsRead(read, null, 1))
    }
    @Test fun attachmentContentVersionsCannotChangeAnotherManifest() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-blobs").toFile()
        try {
            val repository = ConversationDraftRepository(root, "fixture")
            val attachment = AndroidChatAttachment(name = "note", typeLabel = "text", sizeBytes = 3,
                content = "old", extractionLabel = "text", contentDigest = "old",
                uri = "content://fixture/chat_attachments/note", mimeType = "text/plain")
            repository.conditionalSave("conversation:a", ConversationDraftRepository.Snapshot(attachments = listOf(attachment)))
            repository.conditionalSave("conversation:b", ConversationDraftRepository.Snapshot(attachments = listOf(attachment.copy(content = "new"))))
            assertEquals("old", repository.load("conversation:a")?.attachments?.single()?.content)
            assertEquals("new", repository.load("conversation:b")?.attachments?.single()?.content)
        } finally { root.deleteRecursively() }
    }
    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    @Test fun composerKeepsConflictingLocalInputAndReportsFailure() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val root = Files.createTempDirectory("nola-draft-ui-conflict").toFile()
        try {
            val first = ConversationDraftRepository(root, "fixture")
            val stale = ConversationDraftRepository(root, "fixture")
            val drafts = ConversationComposerDrafts(stale)
            drafts.initialize()
            val local = drafts.awaitReady("a")
            first.conditionalSave("conversation:a", ConversationDraftRepository.Snapshot("other window"))
            assertEquals("other window", first.load("conversation:a")?.text)
            local.text.value = "keep this local input"
            drafts.flush()
            assertEquals("keep this local input", local.text.value)
            assertNotNull(local.persistenceError.value)
            assertEquals("other window", first.load("conversation:a")?.text)
        } finally { Dispatchers.resetMain(); root.deleteRecursively() }
    }
    @Test fun staleInstancesCannotOverwriteOrResurrectDeletedDraft() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-cas").toFile()
        try {
            val first = ConversationDraftRepository(root, "fixture")
            val stale = ConversationDraftRepository(root, "fixture")
            assertNull(first.load("draft:a")); assertNull(stale.load("draft:a"))
            first.conditionalSave("draft:a", ConversationDraftRepository.Snapshot("winner"))
            assertEquals("winner", first.load("draft:a")?.text)
            val failure = java.util.concurrent.atomic.AtomicReference<String?>()
            failure.set(runCatching { stale.conditionalSave("draft:a", ConversationDraftRepository.Snapshot("local text")) }.exceptionOrNull()?.message)
            assertEquals("winner", first.load("draft:a")?.text)
            assertNotNull(failure.get())
            first.remove("draft:a")
            val reopened = ConversationDraftRepository(root, "fixture")
            assertNull(reopened.load("draft:a"))
            failure.set(null)
            failure.set(runCatching { reopened.conditionalSave("draft:a", ConversationDraftRepository.Snapshot("resurrection")) }.exceptionOrNull()?.message)
            assertNull(first.load("draft:a")); assertNotNull(failure.get())
        } finally { root.deleteRecursively() }
    }

    @Test fun unsentConfigurationRestoresButCanonicalOwnerRejectsIt() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-config").toFile()
        try {
            val repository = ConversationDraftRepository(root, "fixture")
            val config = ConversationDraftRepository.Configuration(
                com.nola.domain.model.ModelInfo("wire-model", "Display", profileId = "profile-owner"),
                com.nola.domain.model.ModelParameters(temperature = 0.3f, maxCompletionTokens = 2048))
            repository.conditionalSave("draft:pending", ConversationDraftRepository.Snapshot("text", configuration = config))
            assertEquals(config, ConversationDraftRepository(root, "fixture").load("draft:pending")?.configuration)
            val failure = java.util.concurrent.atomic.AtomicReference<String?>()
            failure.set(runCatching { repository.conditionalSave("conversation:existing", ConversationDraftRepository.Snapshot(configuration = config)) }.exceptionOrNull()?.message)
            assertNull(repository.load("conversation:existing")); assertNotNull(failure.get())
            val persisted = root.listFiles()!!.filter { it.extension == "json" }.joinToString { it.readText() }
            assertFalse(persisted.contains("apiKey")); assertFalse(persisted.contains("credential"))
        } finally { root.deleteRecursively() }
    }

    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    @Test fun submittedConversationCanComposeAgainWithoutRevivingDeletedOwner() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val root = Files.createTempDirectory("nola-draft-submission").toFile()
        try {
            val repository = ConversationDraftRepository(root, "fixture")
            val drafts = ConversationComposerDrafts(repository)
            drafts.initialize()
            drafts.awaitReady("a").text.value = "sent"
            val ticket = drafts.prepareSubmission(drafts.captureSubmission(drafts.ownerKey("a")))
            assertTrue(drafts.submitted(ticket))
            drafts.awaitReady("a").text.value = "next input"
            drafts.flush()
            assertEquals("next input", repository.load("conversation:a")?.text)
            val pendingOwner = drafts.ownerKey(null)
            drafts.beginNewDraft()
            drafts.savePendingConfiguration(pendingOwner, ConversationDraftRepository.Configuration(null, com.nola.domain.model.ModelParameters()))
            drafts.flush()
            assertNull(drafts.pendingConfiguration())
            drafts.close()
            advanceUntilIdle()
        } finally { Dispatchers.resetMain(); root.deleteRecursively() }
    }
    @Test fun independentDraftsSurviveRepositoryRecreationAndDeleteIsOrderedAfterWrites() = runBlocking {
        val root = Files.createTempDirectory("nola-drafts-test").toFile()
        try {
            val first = ConversationDraftRepository(root, "fixture")
            val owner = first.pendingOwner()
            first.conditionalSave("conversation:a", ConversationDraftRepository.Snapshot("A"))
            first.conditionalSave("conversation:b", ConversationDraftRepository.Snapshot("B"))
            val reopened = ConversationDraftRepository(root, "fixture")
            assertEquals(owner, reopened.pendingOwner())
            assertEquals("A", reopened.load("conversation:a")?.text)
            assertEquals("B", reopened.load("conversation:b")?.text)
            first.conditionalSave("conversation:a", ConversationDraftRepository.Snapshot("late queued write"))
            first.remove("conversation:a")
            assertNull(reopened.load("conversation:a"))
            assertNotEquals(owner, first.newPendingOwner())
        } finally { root.deleteRecursively() }
    }

    @Test fun deletedPickerOwnerCannotBeRecreatedByLateResult() = runBlocking {
        val store = ConversationComposerDrafts()
        store.beginPicker("a")
        store.remove("a")
        assertNull(store.consumePicker())
        val original = store.ownerKey(null)
        store.forConversation(null)
        assertTrue(store.submitted(store.prepareSubmission(store.captureSubmission(original))))
        assertNotEquals(original, store.ownerKey(null))
    }

    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    @Test fun admissionKeepsNewInputWrittenWhileOriginalSaveWasAwaiting() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val root = Files.createTempDirectory("nola-draft-admission-edit").toFile()
        val drafts = ConversationComposerDrafts(ConversationDraftRepository(root, "fixture"))
        try {
            drafts.initialize()
            for (id in listOf(null, "a")) {
                val local = drafts.awaitReady(id)
                local.text.value = "submitted input"
                val owner = drafts.ownerKey(id)
                val capture = drafts.captureSubmission(owner)
                // The frozen save has not run yet. A subsequent edit belongs to the next submission.
                local.text.value = "next input"
                val ticket = drafts.prepareSubmission(capture)
                drafts.flush()
                assertFalse(drafts.submitted(ticket))
                assertSame(local, drafts.forConversation(id))
                assertEquals("next input", local.text.value)
                assertEquals("next input", ConversationDraftRepository(root, "fixture").load(owner)?.text)
                assertFalse(drafts.submitted(ticket))
            }
        } finally { drafts.close(); advanceUntilIdle(); Dispatchers.resetMain(); root.deleteRecursively() }
    }

    @Test fun pendingRetirementDoesNotTombstoneAnEditQueuedBehindAcknowledgement() = runBlocking {
        val root = Files.createTempDirectory("nola-draft-ack-queued-edit").toFile()
        try {
            val repository = ConversationDraftRepository(root, "fixture")
            val owner = "draft:${repository.pendingOwner()}"
            val frozen = repository.conditionalSave(owner, ConversationDraftRepository.Snapshot("sent"))
            assertTrue(repository.retireForSubmission(frozen))
            repository.conditionalSave(owner, ConversationDraftRepository.Snapshot("typed during acknowledgement"))
            assertEquals(owner, "draft:${repository.pendingOwner()}")
            assertEquals("typed during acknowledgement", ConversationDraftRepository(root, "fixture").load(owner)?.text)
        } finally { root.deleteRecursively() }
    }

    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
    @Test fun admissionDoesNotRetireNewEditEvenWhenTextReturnsToSubmittedValue() = runTest {
        Dispatchers.setMain(StandardTestDispatcher(testScheduler))
        val root = Files.createTempDirectory("nola-draft-admission-revision").toFile()
        val drafts = ConversationComposerDrafts(ConversationDraftRepository(root, "fixture"))
        try {
            drafts.initialize()
            val local = drafts.awaitReady("a")
            local.text.value = "same text"
            val capture = drafts.captureSubmission(drafts.ownerKey("a"))
            local.text.value = "different text"
            local.text.value = "same text"
            val ticket = drafts.prepareSubmission(capture)
            drafts.flush()
            assertFalse(drafts.submitted(ticket))
            assertEquals("same text", ConversationDraftRepository(root, "fixture").load("conversation:a")?.text)
        } finally { drafts.close(); advanceUntilIdle(); Dispatchers.resetMain(); root.deleteRecursively() }
    }
}
