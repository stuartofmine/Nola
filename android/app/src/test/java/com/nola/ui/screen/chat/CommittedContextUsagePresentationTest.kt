package com.nola.ui.screen.chat

import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.sse.decodeSsePayload
import com.nola.domain.projection.ConversationProjectionV1
import kotlin.test.*

class CommittedContextUsagePresentationTest {
    @Test fun estimateDoesNotEraseCumulativeBillAndDiagnosticHasItsOwnRequest() {
        val json = kotlinx.serialization.json.Json
        val measurement = json.parseToJsonElement("""{"request_id":"paid","attempt_id":null,"context_generation":3}""") as kotlinx.serialization.json.JsonObject
        val diagnostic = json.parseToJsonElement("""{"status":"hit","cacheReadInputTokens":800,"cacheWriteInputTokens":null}""") as kotlinx.serialization.json.JsonObject
        val totals = json.parseToJsonElement("""{"hit_rate_bps":8000,"reported_requests":2,"unreported_requests":1}""") as kotlinx.serialization.json.JsonObject
        val paid = ChatUiState().applyCanonicalContextUsage(ConversationProjectionV1.ContextUsage(1000, 32768, 28672, false, 12,
            promptCacheMeasurement = measurement, promptCacheDiagnostic = diagnostic, conversationCacheUsage = totals))
        val next = paid.applyCanonicalContextUsage(ConversationProjectionV1.ContextUsage(2000, 32768, 28672, true, 13))
        assertEquals(8000, next.contextUsageDetail?.cacheHitRateBps)
        assertEquals(2L, next.contextUsageDetail?.cacheReportedRequests)
        assertTrue(next.contextUsageDetail?.contextIsEstimated == true)
        assertNull(next.contextUsageDetail?.cacheDiagnosticRequestId)
        assertNull(next.contextUsageDetail?.cachedInputTokens)
        assertEquals("paid", paid.contextUsageDetail?.cacheDiagnosticRequestId)
    }

    @Test fun canonicalWindowUsageDoesNotRequireAnAssistantMessageOrSummary() {
        val usage = ConversationProjectionV1.ContextUsage(7500, 32768, 28672, false, 12)
        val state = ChatUiState().applyCanonicalContextUsage(usage)
        assertTrue(state.messages.isEmpty())
        assertEquals(7500, state.contextTokens)
        assertEquals(32768, state.contextMaxTokens)
        assertEquals(7500, state.contextUsageDetail?.totalTokens)
        assertEquals(7500f / 32768f, state.contextUsage)
    }

    @Test fun liveContextIsOnlyAWakeupEvenWhenItCarriesArbitraryTokens() {
        val event = decodeSsePayload("""{"type":"context_usage","session_id":"conversation","used_tokens":999999999}""", null)
        assertIs<SseEvent.ContextUsageChanged>(event)
    }
}
