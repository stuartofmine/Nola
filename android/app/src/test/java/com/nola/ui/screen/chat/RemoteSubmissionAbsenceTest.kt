package com.nola.ui.screen.chat

import com.nola.domain.model.ProcessingState
import org.junit.Assert.*
import org.junit.Test

class RemoteSubmissionAbsenceTest {
    @Test fun noRunAfterCommittedRejectionPreservesOutcome() {
        val prior=ChatUiState(error="dispatch_not_admitted",activeResponseId="pending",processingState=ProcessingState.BuildingContext)
        val next=prior.withRemoteOwnerAbsent(true)
        assertEquals("dispatch_not_admitted",next.error)
        assertNull(next.activeResponseId)
        assertEquals(ProcessingState.Idle,next.processingState)
    }
    @Test fun authoritativeAbsenceClearsOnlyTransientObservationError() {
        val next=ChatUiState(error="transport unavailable").withRemoteOwnerAbsent(false)
        assertNull(next.error)
        assertEquals(ProcessingState.Idle,next.processingState)
    }
}
