// SPDX-License-Identifier: Apache-2.0
package com.nola.data.remote.api

import com.nola.data.local.AndroidAdmitExpertMessage
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class RemoteExpertAdmissionTest {
    private val input = AndroidAdmitExpertMessage("parent", 3, "room", 4, "submission", "user", "hello", emptyList(), 5)
    private fun evidence() = buildJsonObject {
        put("definitive_rejection", true); put("host_id", "host"); put("client_id", "client")
        put("conversation_id", "parent"); put("submission_id", "submission"); put("user_message_id", "user")
    }
    @Test fun onlyCompleteSourceOwnedDefinitiveRejectionCanReleaseTheFrozenIntent() {
        assertTrue(definitiveRemoteExpertRejection(WebHostStatusException(409, "expert_admission_rejected", evidence()), "host", "client", input))
        for (field in evidence().keys) {
            val missing = JsonObject(evidence().filterKeys { it != field })
            val foreign = JsonObject(evidence() + (field to JsonPrimitive("foreign")))
            for (value in listOf(missing, foreign)) assertFalse(definitiveRemoteExpertRejection(WebHostStatusException(409, "expert_admission_rejected", value), "host", "client", input))
        }
        assertFalse(definitiveRemoteExpertRejection(WebHostStatusException(503, "expert_admission_rejected", evidence()), "host", "client", input))
        assertFalse(definitiveRemoteExpertRejection(WebHostStatusException(409, "expert_admission_unconfirmed", evidence()), "host", "client", input))
        assertFalse(definitiveRemoteExpertRejection(IllegalStateException("rejected"), "host", "client", input))
    }
}
