package com.nola.ui.screen.chat

import com.nola.agent.context.resolveCoreArchiveIndices
import com.nola.core.AgentCoreContextArchiveRequest
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CoreContextPreparationTest {
    private val activeIds = listOf("m0", "m1", "m2", "m3")

    @Test
    fun acceptsOnlyMatchingIndicesAndIds() {
        val result = resolveCoreArchiveIndices(
            activeMessageIds = activeIds,
            request = AgentCoreContextArchiveRequest(
                archiveId = "archive-1",
                archiveDigest = "a".repeat(64),
                messageIndices = listOf(0, 1),
                messageIds = listOf("m0", "m1"),
                messages = emptyList(),
                targetSummaryTokens = 128,
            ),
        )

        assertEquals(listOf(0, 1), result)
    }

    @Test
    fun rejectsMismatchedOrUnorderedArchiveRequests() {
        assertNull(
            resolveCoreArchiveIndices(
                activeMessageIds = activeIds,
                request = AgentCoreContextArchiveRequest(
                    archiveId = "archive-2",
                    archiveDigest = "b".repeat(64),
                    messageIndices = listOf(0, 2),
                    messageIds = listOf("m0", "m1"),
                    messages = emptyList(),
                    targetSummaryTokens = 128,
                ),
            )
        )
        assertNull(
            resolveCoreArchiveIndices(
                activeMessageIds = activeIds,
                request = AgentCoreContextArchiveRequest(
                    archiveId = "archive-3",
                    archiveDigest = "c".repeat(64),
                    messageIndices = listOf(1, 0),
                    messageIds = listOf("m1", "m0"),
                    messages = emptyList(),
                    targetSummaryTokens = 128,
                ),
            )
        )
    }

    @Test
    fun preparedHistoryIsNotCappedAndKeepsLatestUserLast() {
        val activeIds = (0 until 30).map { "m$it" }
        val prepared = (0 until 29).map { index ->
            buildJsonObject { put("id", "m$index") }
        }

        val result = resolveCorePreparedIndices(
            activeMessageIds = activeIds,
            preparedMessages = prepared,
            latestUserId = "m29",
        )

        assertEquals(30, result.size)
        assertEquals(29, result.last())
    }

    @Test
    fun preparedHistoryRejectsMessagesThatCannotBeProjected() {
        val result = resolveCorePreparedIndices(
            activeMessageIds = listOf("m0", "m1", "m2"),
            preparedMessages = listOf(buildJsonObject { put("content", "no id") }),
            latestUserId = "m2",
        )

        assertEquals(emptyList<Int>(), result)
    }
}
