// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.repository.AgentRoomSnapshot
import com.nola.data.repository.RoomParticipant
import com.nola.domain.model.ConversationMessage
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Display-only aggregation. Never use the merged timeline as model input. Main-thread owned. */
internal class AgentRoomPresentation {
    data class State(
        val room: AgentRoomSnapshot? = null,
        val selectedParticipantId: String? = null,
        val inspectedParticipantId: String? = null,
        val contexts: Map<String, ChatUiState> = emptyMap(),
    ) {
        val recipient: RoomParticipant? get() = room?.room?.participants?.singleOrNull {
            it.id == selectedParticipantId && it.active
        }

        fun rows(): List<Row> {
            data class OrderedRow(val row: Row, val time: Long, val ordinal: Int)
            return room?.room?.participants.orEmpty().flatMap { participant ->
                contexts[participant.contextConversationId]?.let { context ->
                    var time = Long.MIN_VALUE
                    context.timelineMessages().mapIndexed { ordinal, message ->
                        // Timestamps may tie or move backwards across projection sources.
                        // The canonical context order always wins over a random message ID.
                        time = maxOf(time, message.createdAt)
                        OrderedRow(Row(participant, message, context), time, ordinal)
                    }
                }.orEmpty()
            }.sortedWith(compareBy<OrderedRow> { it.time }
                .thenBy { it.row.participant.contextConversationId }.thenBy { it.ordinal }).map { it.row }
        }
    }

    data class Row(
        val participant: RoomParticipant,
        val message: ConversationMessage,
        val context: ChatUiState,
    ) {
        val key: String get() = "${participant.contextConversationId}/${message.id}"
    }

    private val mutable = MutableStateFlow(State())
    val state = mutable.asStateFlow()
    private var loadGeneration = 0L

    fun beginLoad(): Long = ++loadGeneration
    fun acceptsLoad(generation: Long): Boolean = generation == loadGeneration

    fun open(snapshot: AgentRoomSnapshot) {
        val previous = mutable.value
        val sameRoom = previous.room?.room?.id == snapshot.room.id
        if (sameRoom && snapshot.revision < requireNotNull(previous.room).revision) return
        val owners = snapshot.room.participants.map { it.contextConversationId }.toSet()
        mutable.value = State(
            room = snapshot,
            selectedParticipantId = previous.selectedParticipantId.takeIf { selected ->
                sameRoom && snapshot.room.participants.any { it.id == selected && it.active }
            },
            inspectedParticipantId = previous.inspectedParticipantId.takeIf { inspected ->
                sameRoom && snapshot.room.participants.any { it.id == inspected }
            },
            contexts = if (sameRoom) previous.contexts.filterKeys { it in owners } else emptyMap(),
        )
    }

    fun close() { loadGeneration++; mutable.value = State() }

    fun select(participantId: String): String {
        val participant = requireNotNull(mutable.value.room).room.participants.single {
            it.id == participantId && it.active
        }
        mutable.value = mutable.value.copy(selectedParticipantId = participant.id, inspectedParticipantId = participant.id)
        return participant.contextConversationId
    }

    fun inspect(participantId: String): String {
        val participant = requireNotNull(mutable.value.room).room.participants.single { it.id == participantId }
        mutable.value = mutable.value.copy(inspectedParticipantId = participant.id)
        return participant.contextConversationId
    }

    fun publish(snapshot: ChatUiState) {
        val contextId = snapshot.conversationId ?: return
        val current = mutable.value
        if (current.room?.room?.participants?.none { it.contextConversationId == contextId } != false) return
        require(snapshot.messages.all { it.conversationId == contextId }) { "Mixed context projection" }
        require(snapshot.journalResponse?.conversationId?.let { it == contextId } != false)
        mutable.value = current.copy(contexts = current.contexts + (contextId to snapshot))
    }

    /** Captured immediately before dispatch; a pending recipient switch must not send to the old owner. */
    fun requireRecipient(displayedContextId: String?): RoomParticipant =
        requireNotNull(mutable.value.recipient).also {
            check(it.contextConversationId == displayedContextId) { "Recipient context is not loaded" }
        }
}
