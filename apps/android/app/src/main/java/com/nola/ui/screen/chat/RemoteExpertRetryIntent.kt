// SPDX-License-Identifier: Apache-2.0
package com.nola.ui.screen.chat

import com.nola.data.local.AndroidAdmitExpertMessage
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.*

/** Transport retry metadata only. Canonical messages and source ownership stay in Core. */
@Serializable
internal data class RemoteExpertRetryIntent(val version: Int = 1, val hostId: String, val clientId: String,
    val storeEpoch: String, val input: AndroidAdmitExpertMessage) {
    fun validated(host: String, client: String, parent: String): RemoteExpertRetryIntent = apply {
        require(version == 1 && hostId == host && clientId == client && storeEpoch.isNotBlank() && input.conversationId == parent)
        require(input.userMessageId.isNotBlank() && input.submissionId == input.userMessageId && input.selectedParticipants.isNotEmpty())
    }
}
internal fun remoteExpertRetryKey(host: String, client: String, parent: String): String = JsonArray(listOf(host, client, parent).map(::JsonPrimitive)).toString()
