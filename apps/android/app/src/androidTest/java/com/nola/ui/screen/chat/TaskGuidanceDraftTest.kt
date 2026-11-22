// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import com.nola.R
import com.nola.core.AgentCoreTaskHandle
import com.nola.core.AgentCoreTaskKind
import com.nola.core.AgentCoreTaskState
import com.nola.domain.projection.ConversationProjectionV1
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class TaskGuidanceDraftTest {
    @get:Rule val compose = createComposeRule()

    @Test fun rejectedGuidanceAndEditsMadeDuringSubmissionSurvive() {
        val task = AgentCoreTaskHandle("task", "conversation", "parent", AgentCoreTaskKind.Agent, AgentCoreTaskState.Running)
        val control = ConversationProjectionV1.Part.Task("part", 1, task.id, "agent", ConversationProjectionV1.ActivityState.Running,
            null, null, null, false, supportsGuidance = true, parentRunId = "parent")
        val receipts = mutableListOf<CompletableDeferred<Boolean>>()
        val submissions = mutableListOf<String>()
        compose.setContent {
            com.nola.ui.theme.NolaTheme {
                AgentCoreTaskCard(task, control, null, null, null, {}, { message ->
                    submissions += message
                    CompletableDeferred<Boolean>().also { receipts += it }.await()
                })
            }
        }
        compose.onNodeWithTag("chat.task.card.task").performClick()
        val field = compose.onNode(hasSetTextAction())
        val send = compose.onNodeWithContentDescription(InstrumentationRegistry.getInstrumentation().targetContext.getString(R.string.chat_task_guidance_send))
        compose.onAllNodes(hasSetTextAction() and hasAnyAncestor(hasTestTag("chat.task.card.task"))).assertCountEquals(0)
        field.performTextInput("First instruction")
        send.performClick()
        send.assertIsNotEnabled()
        compose.runOnIdle { receipts.single().complete(false) }
        field.assertTextContains("First instruction")
        send.performClick()
        field.performTextReplacement("Second instruction")
        compose.runOnIdle { receipts[1].complete(true) }
        field.assertTextContains("Second instruction")
        send.performClick()
        compose.runOnIdle { receipts[2].complete(true) }
        send.assertIsNotEnabled()
        assertEquals(listOf("First instruction", "First instruction", "Second instruction"), submissions)
    }
}
