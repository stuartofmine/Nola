// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.repository.RoomSubmissionInput
import com.nola.data.repository.RoomSubmission

internal data class RoomComposerState(
    val text: String = "",
    val participantId: String? = null,
    val pending: RoomSubmissionInput? = null,
    val failed: Boolean = false,
)

/** Only explicit @ completion opens candidates; ordinary words never select an agent. */
private val roomMentionPattern = Regex("(?<![A-Za-z0-9._%+@＠-])[@＠]([^\\s@＠]*)$")
internal fun roomMentionQuery(text: String): String? = roomMentionPattern.find(text)?.groupValues?.get(1)
internal fun removeRoomMentionQuery(text: String): String = roomMentionPattern.replace(text, "")

/** Late polling/submit receipts cannot erase messages or regress terminal state. */
internal fun mergeRoomSubmissions(roomId: String, current: List<RoomSubmission>, received: List<RoomSubmission>): List<RoomSubmission> {
    val merged = current.filter { it.input.roomId == roomId }.associateBy { it.input.submissionId }.toMutableMap()
    received.filter { it.input.roomId == roomId }.forEach { value ->
        val prior = merged[value.input.submissionId]
        if (prior == null || (prior.input == value.input && value.revision > prior.revision)) {
            merged[value.input.submissionId] = value
        }
    }
    return merged.values.sortedWith(compareBy({ it.input.atMs }, { it.input.submissionId }))
}
