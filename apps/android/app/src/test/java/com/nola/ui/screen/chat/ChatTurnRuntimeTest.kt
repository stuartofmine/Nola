package com.nola.ui.screen.chat

import com.nola.core.AgentCoreSnapshot
import com.nola.core.AgentCoreTaskProjection
import com.nola.core.AgentCoreSkillRuntimeProjection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

class ChatTurnRuntimeTest {
    @Test
    fun newCoreIdentityClearsOldTaskAndSkillViewsWithoutChangingConversationRoute() {
        val state = ChatUiState(
            conversationId = "route",
            coreTaskProjection = AgentCoreTaskProjection(sessionId = "old-core", revision = 8),
            coreSkillProjection = AgentCoreSkillRuntimeProjection(sessionId = "old-core"),
        )
        val runtime = ChatTurnRuntime()
        val snapshot = AgentCoreSnapshot(platform = "android", phase = "running", sessionId = "new-core")
        val updated = runtime.applySnapshot(state, snapshot)
        assertEquals("route", updated.conversationId)
        assertEquals("new-core", updated.coreTaskProjection.sessionId)
        assertEquals(0L, updated.coreTaskProjection.revision)
        assertEquals("new-core", updated.coreSkillProjection.sessionId)
        val repeated = runtime.applySnapshot(updated, snapshot)
        assertSame(updated.coreTaskProjection, repeated.coreTaskProjection)
        assertSame(updated.coreSkillProjection, repeated.coreSkillProjection)
    }
}
