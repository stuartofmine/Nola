package com.nola.core

import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class AndroidChildPermissionsTest {
    private fun request(id: String) = AndroidAgentPermissionRequest(false, id, id,
        "read_file", "files_read", "ask", "Read the selected file", null)

    @Test fun exactChildDecisionAndCancellationDoNotResolveAnotherOwner() = runBlocking {
        val first = async(start = CoroutineStart.UNDISPATCHED) {
            AndroidChildPermissions.await("parent-a", "child-a", request("request-a"))
        }
        val second = async(start = CoroutineStart.UNDISPATCHED) {
            AndroidChildPermissions.await("parent-b", "child-b", request("request-b"))
        }
        try {
            assertEquals(2, AndroidChildPermissions.pending.value.size)
            assertFalse(AndroidChildPermissions.resolve(AndroidAgentPermissionDecision("foreign",
                AndroidAgentPermissionUserDecision.AllowOnce)))
            first.cancelAndJoin()
            assertFalse(AndroidChildPermissions.owns("request-a"))
            assertFalse(second.isCompleted)
            val deny = AndroidAgentPermissionDecision("request-b", AndroidAgentPermissionUserDecision.Deny)
            assertTrue(AndroidChildPermissions.resolve(deny))
            assertEquals(deny, second.await())
            assertFalse(AndroidChildPermissions.owns("request-b"))
        } finally { first.cancelAndJoin(); second.cancelAndJoin() }
    }
}
