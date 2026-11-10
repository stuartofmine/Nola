// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1
import org.junit.Assert.*
import org.junit.Test

class ModelAttemptPresentationTest {
    private fun attempt(state: ConversationProjectionV1.ActivityState = ConversationProjectionV1.ActivityState.Failed,
        provider: String? = "device_local", model: String? = "qwen", successor: String? = null,
        nextProvider: String? = null, nextModel: String? = null) = ConversationProjectionV1.Part.ModelAttempt(
        "attempt", 1, state, "original", provider, model,
        ConversationProjectionV1.Part.ModelAttemptFailure("fixture", "bounded failure", false), successor, nextProvider, nextModel)

    @Test fun lifecycleIsNeutralUnlessItIsFailedWithoutSuccessor() {
        for (state in ConversationProjectionV1.ActivityState.entries) {
            val result = modelAttemptPresentation(attempt(state))
            assertEquals(state == ConversationProjectionV1.ActivityState.Failed, result.isFailure)
        }
    }
    @Test fun nextModelWithoutSuccessorIdCannotInventFallback() {
        val result = modelAttemptPresentation(attempt(nextModel = "other-model", nextProvider = "other-provider"))
        assertEquals(ModelAttemptPresentation.Kind.Failed, result.kind)
        assertNull(result.successorModelName)
    }
    @Test fun sameRouteSuccessorIsNeutralRequestRestartAndDoesNotRepeatModelName() {
        val result = modelAttemptPresentation(attempt(successor = "next", nextProvider = "device_local", nextModel = "qwen"))
        assertEquals(ModelAttemptPresentation.Kind.Restarted, result.kind)
        assertFalse(result.isFailure)
        assertEquals("qwen", result.modelName)
        assertNull(result.successorModelName)
    }
    @Test fun knownProviderOrModelChangeIsNeutralRouteChange() {
        for (record in listOf(attempt(successor = "next", nextProvider = "another-provider"),
            attempt(successor = "next", nextModel = "another-model"))) {
            val result = modelAttemptPresentation(record)
            assertEquals(ModelAttemptPresentation.Kind.RouteChanged, result.kind)
            assertFalse(result.isFailure)
        }
    }
    @Test fun unknownOriginalRouteDoesNotProveRouteChanged() {
        val result = modelAttemptPresentation(attempt(provider = null, model = null, successor = "next", nextProvider = "known", nextModel = "qwen"))
        assertEquals(ModelAttemptPresentation.Kind.Restarted, result.kind)
        assertEquals("—", result.modelName)
        assertEquals("qwen", result.successorModelName)
    }
    @Test fun providerNamespaceNeverBecomesModelNameAndBlankSuccessorIsNotAuthority() {
        val result = modelAttemptPresentation(attempt(provider = "device_local", model = " qwen ", successor = " ", nextModel = "other"))
        assertEquals("qwen", result.modelName)
        assertEquals(ModelAttemptPresentation.Kind.Failed, result.kind)
        assertNull(result.successorModelName)
    }
}
