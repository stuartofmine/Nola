package com.nola.ui.screen.chat

/** Returns only a true appended suffix; history prepends/replacements never replay entrances. */
internal object ChatTimelineMotionPolicy {
    data class ItemKey(val conversationId: String?, val messageId: String) : java.io.Serializable

    fun itemKey(conversationId: String?, messageId: String) = ItemKey(conversationId, messageId)

    fun appendedMessageIds(previous: List<String>?, current: List<String>): List<String> {
        if (previous == null || current.size <= previous.size) return emptyList()
        if (current.take(previous.size) != previous) return emptyList()
        return current.drop(previous.size)
    }
}
