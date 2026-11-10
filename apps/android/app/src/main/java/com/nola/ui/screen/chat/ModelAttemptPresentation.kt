// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.domain.projection.ConversationProjectionV1

/** Presentation only. A successor proves request succession, never model degradation. */
internal data class ModelAttemptPresentation(
    val kind: Kind,
    val modelName: String,
    val successorModelName: String?,
) {
    enum class Kind { Pending, Running, Waiting, Completed, Cancelled, Failed, Restarted, RouteChanged }
    val isFailure: Boolean get() = kind == Kind.Failed
}

internal fun modelAttemptPresentation(attempt: ConversationProjectionV1.Part.ModelAttempt): ModelAttemptPresentation {
    fun normalized(value: String?) = value?.trim()?.takeIf(String::isNotEmpty)
    val model = normalized(attempt.model)
    val nextModel = normalized(attempt.fallbackModel)
    val provider = normalized(attempt.provider)
    val nextProvider = normalized(attempt.fallbackProvider)
    val successor = normalized(attempt.fallbackAttemptId) != null
    val kind = if (successor) {
        val routeChanged = (model != null && nextModel != null && model != nextModel) ||
            (provider != null && nextProvider != null && provider != nextProvider)
        if (routeChanged) ModelAttemptPresentation.Kind.RouteChanged else ModelAttemptPresentation.Kind.Restarted
    } else when (attempt.state) {
        ConversationProjectionV1.ActivityState.Pending -> ModelAttemptPresentation.Kind.Pending
        ConversationProjectionV1.ActivityState.Running -> ModelAttemptPresentation.Kind.Running
        ConversationProjectionV1.ActivityState.WaitingForPermission,
        ConversationProjectionV1.ActivityState.WaitingForInput -> ModelAttemptPresentation.Kind.Waiting
        ConversationProjectionV1.ActivityState.Completed,
        ConversationProjectionV1.ActivityState.Sealed -> ModelAttemptPresentation.Kind.Completed
        ConversationProjectionV1.ActivityState.Cancelled -> ModelAttemptPresentation.Kind.Cancelled
        ConversationProjectionV1.ActivityState.Failed -> ModelAttemptPresentation.Kind.Failed
    }
    return ModelAttemptPresentation(kind, model ?: "—", nextModel.takeIf { successor && it != model })
}
