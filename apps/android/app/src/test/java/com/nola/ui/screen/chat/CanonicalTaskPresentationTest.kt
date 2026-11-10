package com.nola.ui.screen.chat

import com.nola.core.AgentCoreTaskHandle
import com.nola.core.AgentCoreTaskKind
import com.nola.core.AgentCoreTaskState
import com.nola.domain.projection.ConversationProjectionV1
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class CanonicalTaskPresentationTest {
    private fun part(id: String) = ConversationProjectionV1.Part.Task(
        partId = "part-$id", revision = 1, taskId = id, displayKey = id,
        state = ConversationProjectionV1.ActivityState.Completed, progress = null,
        currentStep = null, resultRef = null, cancelRequested = false, parentRunId = "source-run",
    )
    private fun task(id: String, owner: String = "core-session-distinct") = AgentCoreTaskHandle(
        id = id, ownerSessionId = owner, ownerTurnId = "source-core-turn",
        kind = AgentCoreTaskKind.Agent, state = AgentCoreTaskState.Sealed,
    )
    @Test fun coldMessageUsesCanonicalTaskPublicationWithoutLegacyTurnMetadata() {
        val handle = task("one")
        assertEquals(listOf(handle), canonicalTaskHandles("core-session-distinct", listOf(part("one")), mapOf("one" to handle)))
    }
    @Test fun doesNotBroadcastTasksToOtherMessagesOrConversations() {
        val handles = mapOf("one" to task("one"), "foreign" to task("foreign", "other"))
        assertTrue(canonicalTaskHandles("core-session-distinct", emptyList(), handles).isEmpty())
        assertTrue(canonicalTaskHandles("core-session-distinct", listOf(part("missing"), part("foreign")), handles).isEmpty())
        assertEquals(1, canonicalTaskHandles("core-session-distinct", listOf(part("one"), part("one")), handles).size)
    }
}
