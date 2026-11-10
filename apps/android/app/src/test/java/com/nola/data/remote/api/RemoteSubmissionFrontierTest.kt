// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class RemoteSubmissionFrontierTest {
    private fun response(revision: JsonElement = JsonPrimitive(7), parent: JsonElement = JsonPrimitive("assistant-old")) = buildJsonObject {
        put("source_host_id", "host")
        put("result", buildJsonObject {
            put("protocol_version", 1); put("source_host_id", "host"); put("client_id", "client")
            put("conversation_id", "conversation"); put("configuration_revision", revision); put("parent_message_id", parent)
        })
    }
    @Test fun existingSourceUsesOneAtomicHostFrontier() {
        assertEquals(RemoteSubmissionFrontier(7, "assistant-old"),
            decodeRemoteSubmissionFrontier(response(), "host", "client", "conversation", "assistant-old"))
        val request = remoteSubmissionFrontierRequest("host", "client", "conversation",
            RemoteRunKey("conversation", "run-old", "assistant-old"))
        assertEquals(JsonPrimitive("run-old"), request["expected_run_id"])
        assertEquals(JsonPrimitive("assistant-old"), request["assistant_message_id"])
    }
    @Test fun missingConversationPreservesNullAbsenceWitnesses() {
        assertEquals(RemoteSubmissionFrontier(null, null),
            decodeRemoteSubmissionFrontier(response(JsonNull, JsonNull), "host", "client", "conversation", null))
        assertEquals(JsonNull, remoteSubmissionFrontierRequest("host", "client", "conversation", null)["expected_run_id"])
    }
    @Test fun changedParentNeverRebasesAnUnseenUserSubmission() {
        assertThrows(IllegalArgumentException::class.java) {
            decodeRemoteSubmissionFrontier(response(), "host", "client", "conversation", "older-visible-parent")
        }
    }
    @Test fun wrongOwnerAndInvalidRevisionAreRejected() {
        assertThrows(IllegalArgumentException::class.java) {
            decodeRemoteSubmissionFrontier(response(), "other-host", "client", "conversation", "assistant-old")
        }
        assertThrows(IllegalArgumentException::class.java) {
            decodeRemoteSubmissionFrontier(response(), "host", "other-client", "conversation", "assistant-old")
        }
        for (revision in listOf(JsonPrimitive(0), JsonPrimitive("7"), JsonNull)) {
            assertThrows(IllegalArgumentException::class.java) {
                decodeRemoteSubmissionFrontier(response(revision), "host", "client", "conversation", "assistant-old")
            }
        }
    }
}
