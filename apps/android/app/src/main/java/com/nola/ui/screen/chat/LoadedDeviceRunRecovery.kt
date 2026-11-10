// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1.ActivityState

/** Restore the exact durable owner before presenting a locally controllable active Run. */
internal fun shouldResumeLoadedDeviceRun(
    state: ActivityState?,
    inspectionConfirmed: Boolean,
    hasPendingCommand: Boolean,
    hasUnknownEffects: Boolean,
): Boolean = inspectionConfirmed && !hasPendingCommand && !hasUnknownEffects &&
    state in setOf(ActivityState.Pending, ActivityState.Running, ActivityState.WaitingForPermission)
