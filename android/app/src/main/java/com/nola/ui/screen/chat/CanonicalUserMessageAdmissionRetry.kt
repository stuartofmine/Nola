package com.nola.ui.screen.chat

import java.util.UUID

/** Frozen request identity reused until canonical admission and expert fan-out are confirmed. */
internal data class PendingCanonicalUserMessageAdmission(
    val conversationId: String,
    val messageId: String,
    val text: String,
    val createdAtMs: Long,
    val selectedAgentIds: Set<String>,
)

internal class CanonicalUserMessageAdmissionRetryState(
    private val newMessageId: () -> String = { UUID.randomUUID().toString().lowercase() },
    private val nowMs: () -> Long = System::currentTimeMillis,
) {
    private val pending = mutableMapOf<String, PendingCanonicalUserMessageAdmission>()

    @Synchronized
    fun find(draftOwner: String, conversationId: String?): PendingCanonicalUserMessageAdmission? =
        pending[draftOwner] ?: conversationId?.let { id -> pending.values.firstOrNull { it.conversationId == id } }

    @Synchronized
    fun beginOrResume(
        draftOwner: String,
        conversationId: String,
        text: String,
        selectedAgentIds: Set<String>,
    ): PendingCanonicalUserMessageAdmission {
        require(draftOwner.isNotBlank() && conversationId.isNotBlank() && text.isNotBlank() && selectedAgentIds.isNotEmpty())
        (pending[draftOwner] ?: pending.values.firstOrNull { it.conversationId == conversationId })?.let { previous ->
            require(previous.conversationId == conversationId) {
                "A pending message admission belongs to another conversation. Retry or resolve it first."
            }
            require(previous.text == text) {
                "A pending message admission has different text. Restore the original draft to retry it."
            }
            pending[draftOwner] = previous
            return previous
        }
        val createdAt = nowMs().also { require(it >= 0) }
        return PendingCanonicalUserMessageAdmission(conversationId, newMessageId(), text, createdAt, selectedAgentIds.toSet()).also {
            pending[draftOwner] = it
        }
    }

    @Synchronized
    fun restore(draftOwner: String, input: com.nola.data.local.AndroidAdmitExpertMessage): PendingCanonicalUserMessageAdmission {
        val original = PendingCanonicalUserMessageAdmission(input.conversationId, input.userMessageId, input.text,
            input.createdAtMs, input.selectedParticipants.map { it.agentId }.toSet())
        require(input.submissionId == input.userMessageId && original.selectedAgentIds.isNotEmpty())
        find(draftOwner, input.conversationId)?.let { require(it == original) { "The original expert intent changed" } }
        pending[draftOwner] = original
        return original
    }

    @Synchronized
    fun confirm(draftOwner: String, admission: PendingCanonicalUserMessageAdmission) {
        if (pending[draftOwner] == admission) pending.remove(draftOwner)
        pending.entries.removeAll { it.value == admission }
    }

}
