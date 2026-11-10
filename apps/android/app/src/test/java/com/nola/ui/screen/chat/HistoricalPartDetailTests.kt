package com.nola.ui.screen.chat

import com.nola.domain.projection.*
import kotlinx.coroutines.*
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlin.test.*

class HistoricalPartDetailTests {
    private fun window(): ConversationProjectionV1 {
        val fixture = checkNotNull(javaClass.classLoader?.getResourceAsStream("historical_unloaded_conversation_projection_v1.json"))
            .bufferedReader().use { it.readText() }
        return ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(fixture))
    }
    @Test fun projectedDetailsAreNeverSerializedAsCanonicalMessages() {
        val message = CanonicalConversationProjector.project(window()).single()
        assertEquals(5, message.historicalParts.size)
        assertFalse(Json.encodeToString(message).contains("historicalParts"))
    }
    @Test fun unavailableMismatchAndRetryKeepExplicitNotLoaded() = runBlocking {
        val value = (window().messages.single().parts.first() as ConversationProjectionV1.Part.Historical).projection
        assertFailsWith<IllegalArgumentException> { loadHistoricalPartDetail(value, "unavailable") { null } }
        assertFailsWith<IllegalArgumentException> { loadHistoricalPartDetail(value, "changed") { value.copy(canonicalRevision = value.canonicalRevision + 1) } }
        assertEquals(ConversationProjectionV1.HistoricalStatus.NotLoaded, loadHistoricalPartDetail(value, "unavailable") { value }.status)
    }
    @Test fun cancelledNativeResponseCannotPublish() = runBlocking {
        val value = (window().messages.single().parts.first() as ConversationProjectionV1.Part.Historical).projection
        val started = CompletableDeferred<Unit>()
        val reply = CompletableDeferred<Unit>()
        var published = false
        val job = launch {
            loadHistoricalPartDetail(value, "unavailable") {
                started.complete(Unit)
                withContext(NonCancellable) { reply.await() }
                value
            }
            published = true
        }
        started.await()
        job.cancel()
        reply.complete(Unit)
        job.join()
        assertFalse(published)
    }
}
