// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1.ActivityState
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LoadedDeviceRunRecoveryTest {
    @Test fun coldActiveRunsReacquireTheirOriginalOwner() {
        for (state in listOf(ActivityState.Pending, ActivityState.Running, ActivityState.WaitingForPermission)) {
            assertTrue(shouldResumeLoadedDeviceRun(state, true, false, false))
        }
    }
    @Test fun waitingInputAndTerminalHistoryNeverRestartAutomatically() {
        for (state in listOf(null, ActivityState.WaitingForInput, ActivityState.Completed,
            ActivityState.Sealed, ActivityState.Cancelled, ActivityState.Failed)) {
            assertFalse(shouldResumeLoadedDeviceRun(state, true, false, false))
        }
    }
    @Test fun unconfirmedInspectionOrDurableWorkBlocksRecovery() {
        assertFalse(shouldResumeLoadedDeviceRun(ActivityState.Running, false, false, false))
        assertFalse(shouldResumeLoadedDeviceRun(ActivityState.Running, true, true, false))
        assertFalse(shouldResumeLoadedDeviceRun(ActivityState.Running, true, false, true))
    }
}
