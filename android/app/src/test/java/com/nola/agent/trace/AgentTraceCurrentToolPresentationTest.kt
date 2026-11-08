// SPDX-License-Identifier: Apache-2.0

package com.nola.agent.trace

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class AgentTraceCurrentToolPresentationTest {
    @Test
    fun `live focus exposes only latest active tool without dropping history`() {
        val completed = AgentTraceToolProjection(
            traceId = "trace",
            spanId = "completed",
            callId = "completed",
            toolName = "read_file",
            status = "succeeded",
        )
        val running = AgentTraceToolProjection(
            traceId = "trace",
            spanId = "running",
            callId = "running",
            toolName = "code_search",
            status = "running",
        )
        val projection = AgentTraceProjection(tools = listOf(completed, running))

        assertEquals(2, projection.tools.size)
        assertEquals("running", projection.currentTool?.callId)
    }

    @Test
    fun `terminal tool history does not remain in live focus`() {
        val projection = AgentTraceProjection(
            tools = listOf(
                AgentTraceToolProjection(
                    traceId = "trace",
                    spanId = "completed",
                    callId = "completed",
                    toolName = "read_file",
                    status = "succeeded",
                ),
            ),
        )

        assertNull(projection.currentTool)
        assertEquals(1, projection.tools.size)
    }
}
