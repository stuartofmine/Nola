package com.nola.data.local

import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import android.content.Context
import java.io.File
import java.util.UUID
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ApplicationSubmissionOwnerIsolationTest {
    @Test fun unconfirmedContextAIntentDoesNotBlockCanonicalContextBAndRecoversExactly() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val directory = File(context.cacheDir, "submission-owners-${UUID.randomUUID()}").also { it.mkdirs() }
        java.nio.file.Files.setPosixFilePermissions(directory.toPath(), java.nio.file.attribute.PosixFilePermissions.fromString("rwx------"))
        val database = File(directory, "isolated.db")
        val a = "owner-a-${UUID.randomUUID()}"
        val b = "owner-b-${UUID.randomUUID()}"
        val journal = ApplicationSubmissionJournal(File(database.path + ".pending-commands.json"))
        lateinit var pending: JsonObject
        AndroidApplicationRuntime(context, database).use { runtime ->
            runtime.createConversation(a, "A", 1, 1, null)
            // Durable transport intent whose delivery/ACK has not happened yet.
            pending = journal.prepare(buildJsonObject {
                put("type", "rename_conversation"); put("conversation_id", a)
                put("title", "A recovered"); put("updated_at_ms", 2)
            }, com.nola.core.AgentCoreNative.nativeApplicationProtocolVersion(), "pending-a", null)
            runtime.createConversation(b, "B", 2, 2, null)
            assertEquals(listOf(pending), journal.pending())
            try {
                runtime.renameConversation(a, "Must wait", 3)
                fail("Same Context must retain its causal barrier")
            } catch (failure: AndroidApplicationRuntime.PendingMutation) {
                assertEquals("pending-a", failure.requestId)
            }
            assertNotNull(runtime.loadConversation(b))
        }
        AndroidApplicationRuntime(context, database).use { runtime ->
            assertTrue(journal.pending().isEmpty())
            assertEquals("A recovered", runtime.loadConversation(a)?.title)
            assertEquals("B", runtime.loadConversation(b)?.title)
        }
    }
}
