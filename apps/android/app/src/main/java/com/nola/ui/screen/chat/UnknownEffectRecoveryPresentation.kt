package com.nola.ui.screen.chat

import com.nola.core.AndroidCanonicalTurnDescriptor
import com.nola.data.local.UnknownEffectRecoveryInspection
import com.nola.data.local.PendingUnknownEffectRecovery
import java.util.UUID

/** A view snapshot only; Application remains the sole decision authority. */
data class UnknownEffectRecoveryPresentation(
    val conversationId: String,
    val runId: String,
    val loadGeneration: Long,
    val descriptor: AndroidCanonicalTurnDescriptor?,
    val inspection: UnknownEffectRecoveryInspection?,
    val pending: PendingUnknownEffectRecovery?,
    val snapshotId: String = UUID.randomUUID().toString(),
) {
    fun owns(conversation: String?, generation: Long) = conversation == conversationId && generation == loadGeneration
    override fun toString(): String = "UnknownEffectRecoveryPresentation(private)"
}
