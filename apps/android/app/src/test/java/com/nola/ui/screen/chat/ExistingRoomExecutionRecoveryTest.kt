package com.nola.ui.screen.chat

import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import kotlin.test.assertFailsWith

class ExistingRoomExecutionRecoveryTest {
    @Test fun existingCoreRunRestoresItsConversationWithoutNewAdmission() = runTest {
        val calls = mutableListOf<String>()
        assertTrue(restoreExistingRoomExecution("run", "child-context",
            { calls += "read:$it"; true }, { calls += "restore:$it" }))
        assertEquals(listOf("read:run", "restore:child-context"), calls)
    }
    @Test fun absentExecutionLeavesNormalAdmissionPathAvailable() = runTest {
        for (id in listOf(null, "not-admitted")) {
            assertFalse(restoreExistingRoomExecution(id, "child-context", { false },
                { error("must not recover an absent execution") }))
        }
    }
    @Test fun failedCoreReadDoesNotFallThroughIntoDuplicateAdmission() = runTest {
        assertFailsWith<IllegalStateException> {
            restoreExistingRoomExecution("run", "child-context", { error("Store unavailable") },
                { error("must not load without Core identity") })
        }
    }
}
