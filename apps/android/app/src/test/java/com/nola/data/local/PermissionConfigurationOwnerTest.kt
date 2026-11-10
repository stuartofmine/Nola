package com.nola.data.local

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class PermissionConfigurationOwnerTest {
    @Test fun failedHydrationRetriesOnlyAtOwnerBoundaryAndStaleReplyCannotCompleteRetry() {
        val identity = com.nola.data.remote.api.RemoteSubmissionIdentity("host", "client", "conversation", "run", "user", "assistant")
        val hydration = PermissionOwnerHydration()
        val first = requireNotNull(hydration.begin(identity))
        assertNull(hydration.begin(identity))
        assertTrue(hydration.finish(identity, first, succeeded = false))
        val retry = requireNotNull(hydration.begin(identity))
        assertFalse(hydration.finish(identity, first, succeeded = true))
        assertTrue(hydration.finish(identity, retry, succeeded = true))
        assertNull(hydration.begin(identity))
        // A source configuration commit can precede its active-policy ACK.
        // Explicit reconnect must re-read even after a prior confirmed save.
        hydration.confirm(identity)
        assertNull(hydration.begin(identity))
        hydration.invalidate(forgetSuccess = true)
        assertNotNull(hydration.begin(identity))
    }

    @Test fun ownerUsesOneCoreSnapshotWithUnresolvedObservation() {
        var reads = 0
        val owner = readPermissionConfigurationOwner("conversation", "host", "client", "run", true) { input ->
            reads++
            assertEquals("read_permission_configuration", input.journalString("type"))
            assertEquals(JsonArray(listOf(JsonPrimitive("run"))), input["unresolved_run_ids"])
            assertEquals(JsonPrimitive(true), input["source_unconfirmed"])
            Json.parseToJsonElement("""{"conversation_id":"conversation","owner":{"kind":"remote","identity":{
                "source_host_id":"host","client_id":"client","conversation_id":"conversation",
                "submission_id":"run","user_message_id":"user","assistant_message_id":"assistant"}}}""")
        }
        assertEquals(1, reads)
        assertEquals("run", (owner as PermissionConfigurationOwner.Remote).identity.submissionId)
    }

    @Test fun failedOrForeignSnapshotNeverBecomesInactive() {
        assertThrows(IllegalStateException::class.java) {
            readPermissionConfigurationOwner("conversation", null, null, null, false) { error("owner changed") }
        }
        assertThrows(IllegalArgumentException::class.java) {
            readPermissionConfigurationOwner("conversation", null, null, null, false) {
                Json.parseToJsonElement("""{"conversation_id":"foreign","owner":{"kind":"inactive"}}""")
            }
        }
    }
}
