package com.nola.domain.projection

import com.nola.domain.model.ChatRole
import com.nola.domain.model.ConversationMessage
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

class ConversationProjectionV1DecoderTests {
    @Test fun childOwnershipIsPreservedAndForeignChildRejected() {
        val source = fixture("tool").replace("\"kind\":\"user\"",
            "\"kind\":\"task_child\",\"parent_conversation_id\":\"parent\",\"parent_run_id\":\"run\",\"task_id\":\"task\",\"child_conversation_id\":\"conversation-1\"")
        val id = Json.parseToJsonElement(source).jsonObject.getValue("conversation_id").jsonPrimitive.content
        val valid = source.replace("conversation-1", id)
        assertEquals("parent", ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(valid)).taskChildOwner?.parentConversationId)
        assertFailsWith<IllegalArgumentException> {
            ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(valid.replace("\"child_conversation_id\":\"$id\"", "\"child_conversation_id\":\"foreign\"")))
        }
    }

    @Test
    fun surfacePartPreservesIndependentRevisionAndRejectsWrongMessage() {
        val input = Json.parseToJsonElement("""{
            "kind":"ui_surface","part_id":"ui-surface:hash","revision":90,
            "surface":{"identity":{"owner":{"authority_id":"host","conversation_id":"conversation",
            "participant_id":"agent","context_id":"context","originating_run_id":"run",
            "message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1},"revision":2},
            "summary":"Form"
        }""")
        val part = ConversationProjectionV1Decoder.decodeDisplayPart(input, "message") as ConversationProjectionV1.Part.UiSurface
        assertEquals(90L, part.revision)
        assertEquals(2L, part.surface.revision)
        assertEquals("tool-part", part.surface.identity.owner.part_id)
        assertFailsWith<IllegalArgumentException> { ConversationProjectionV1Decoder.decodeDisplayPart(input, "other") }
        assertFailsWith<IllegalArgumentException> {
            ConversationProjectionV1Decoder.decodeDisplayPart(Json.parseToJsonElement(input.toString().replace("\"revision\":2", "\"revision\":0")), "message")
        }
    }

    @Test
    fun everyPublicActivityStateDecodesWithoutChangingItsMeaning() {
        val states = listOf("pending", "running", "waiting_for_permission", "waiting_for_input", "completed", "sealed", "cancelled", "failed")
        for ((index, state) in states.withIndex()) {
            val source = fixture("tool").replace("\"state\":\"running\"", "\"state\":\"$state\"")
            val projection = ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(source))
            val tool = projection.messages.single().parts.last() as ConversationProjectionV1.Part.Tool
            assertEquals(ConversationProjectionV1.ActivityState.entries[index], tool.state)
        }
    }

    @Test
    fun waitingForInputRemainsDistinctFromPermissionAndCompletion() {
        val source = fixture("tool").replace("\"state\":\"running\"", "\"state\":\"waiting_for_input\"")
        val projection = ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(source))
        val tool = projection.messages.single().parts.last() as ConversationProjectionV1.Part.Tool
        assertEquals(ConversationProjectionV1.ActivityState.WaitingForInput, tool.state)
    }

    @Test
    fun unloadedHistoricalPartKeepsBudgetReasonAndDescriptiveCanonicalPreview() {
        val fixture = checkNotNull(javaClass.classLoader?.getResourceAsStream("historical_unloaded_conversation_projection_v1.json"))
            .bufferedReader().use { it.readText() }
        val projection = ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(fixture))
        val historical = (projection.messages.single().parts[0] as ConversationProjectionV1.Part.Historical).projection
        val opaque = (projection.messages.single().parts[4] as ConversationProjectionV1.Part.Historical).projection
        assertEquals(ConversationProjectionV1.HistoricalStatus.NotLoaded, historical.status)
        assertEquals("selection_budget", historical.notLoadedReason)
        assertEquals(null, historical.decodedText)
        assertEquals(null, historical.timelineText)
        assertEquals("opaque original evidence", opaque.timelineText)
        kotlin.test.assertTrue(historical.fields.isEmpty())
        kotlin.test.assertTrue(opaque.statusLabel.contains("opaque original evidence"))
    }

    @Test
    fun historicalFieldsRemainReadOnlyAndPreserveAvailability() {
        val fixture = checkNotNull(javaClass.classLoader?.getResourceAsStream("historical_conversation_projection_v1.json"))
            .bufferedReader().use { it.readText() }
        val projection = ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(fixture))
        val parts = projection.messages.single().parts
        assertEquals(5, parts.size)
        assertEquals(5, parts.filterIsInstance<ConversationProjectionV1.Part.Historical>().size)
        assertEquals(0, parts.filterIsInstance<ConversationProjectionV1.Part.Tool>().size)
        val text = parts[0] as ConversationProjectionV1.Part.Historical
        assertEquals("  原文\\n🙂  ", text.projection.decodedText)
        val deferred = parts[3] as ConversationProjectionV1.Part.Historical
        assertEquals(null, deferred.projection.decodedText)
        kotlin.test.assertTrue(deferred.projection.fields["markdown"] is ConversationProjectionV1.HistoricalField.Deferred)
        val presented = CanonicalConversationProjector.project(projection).single()
        kotlin.test.assertTrue(presented.text.contains("  原文\\n🙂  "))
        kotlin.test.assertFalse(presented.text.contains("supported"))
        kotlin.test.assertFalse(presented.text.contains("deferred"))
        kotlin.test.assertFalse(presented.text.contains("unknown_kind"))
        kotlin.test.assertFalse(presented.text.contains("must not run"))
    }

    @Test
    fun sharedHostFixtureDecodesWithoutPlatformSpecificRepair() {
        val fixture = checkNotNull(
            javaClass.classLoader?.getResourceAsStream(
                "conversation_projection_v1.json",
            ),
        ) { "shared Host projection fixture is missing from test resources" }
            .bufferedReader()
            .use { it.readText() }
        val envelope = Json.parseToJsonElement(fixture).jsonObject
        val projection = ConversationProjectionV1Decoder.decode(
            envelope,
        )

        assertEquals(2, projection.messages.size)
        assertEquals(3, projection.messages[1].parts.size)
        assertEquals(
            "Inspect the shared projection.",
            (projection.messages[1].parts[0] as ConversationProjectionV1.Part.Reasoning).summary,
        )
        assertEquals(
            "file.read",
            (projection.messages[1].parts[1] as ConversationProjectionV1.Part.Tool).capabilityId,
        )
        assertEquals(
            "The projection is the durable UI source.",
            (projection.messages[1].parts[2] as ConversationProjectionV1.Part.Text).markdown,
        )
    }

    @Test
    fun decodesCanonicalTextReasoningToolAndContextFromOneProjection() {
        val projection = ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(fixture("tool")))

        assertEquals("conversation-1", projection.conversationId)
        assertEquals(120L, projection.contextUsage.usedTokens)
        assertEquals(3, projection.messages.single().parts.size)
        assertEquals("turn-1:assistant", projection.surface.liveMessageId)
        assertEquals("tool-1", projection.surface.currentActivityPartId)
        assertEquals(false, projection.surface.showAssistantPlaceholder)
        assertEquals(
            "file.read",
            (projection.messages.single().parts.last() as ConversationProjectionV1.Part.Tool).capabilityId,
        )
    }

    @Test
    fun modelAttemptFailureKeepsFailedAndFallbackIdentity() {
        val projection = ConversationProjectionV1Decoder.decode(
            Json.parseToJsonElement(
                """
                {
                  "execution_ownership":{"kind":"user"},"contract_version":1,"conversation_id":"fallback","revision":3,"title":"Fallback",
                  "messages":[{"message_id":"assistant","role":"assistant","state":"completed","revision":3,"created_at_ms":1,"parts":[
                    {"kind":"model_attempt","part_id":"model-attempt:one","revision":3,"state":"failed","attempt_id":"one","provider":"OpenRouter","model":"free-model","failure":{"code":"rate_limited","message":"The provider rate limit was reached.","retryable":true},"fallback_attempt_id":"two","fallback_provider":"DeepSeek","fallback_model":"deepseek-chat"},
                    {"kind":"text","part_id":"answer","revision":3,"markdown":"Recovered answer","is_final":true}
                  ]}],
                  "active_run":null,
                  "surface":{"revision":3,"committed_message_ids":["assistant"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":["model-attempt:one"],"show_assistant_placeholder":false},
                  "context_usage":{"used_tokens":0,"context_window_tokens":16384,"prompt_token_limit":12288,"estimated":false,"measured_at_revision":3}
                }
                """.trimIndent(),
            ),
        )
        val attempt = projection.messages.single().parts.first() as
            ConversationProjectionV1.Part.ModelAttempt

        assertEquals("OpenRouter", attempt.provider)
        assertEquals("free-model", attempt.model)
        assertEquals("rate_limited", attempt.failure.code)
        assertEquals("DeepSeek", attempt.fallbackProvider)
        assertEquals("deepseek-chat", attempt.fallbackModel)
    }

    @Test
    fun rejectsUnknownProviderDeltaPartInsteadOfGuessing() {
        assertFailsWith<IllegalStateException> {
            ConversationProjectionV1Decoder.decode(Json.parseToJsonElement(fixture("provider_delta")))
        }
    }

    @Test
    fun compactionSourceWindowPreservesIdentityAndStopsAtHistoryEnd() {
        val fingerprint = "a".repeat(64)
        val window = ConversationProjectionV1Decoder.decodeWindow(
            Json.parseToJsonElement(
                """
                {
                  "projection":{
                    "execution_ownership":{"kind":"user"},"contract_version":1,"conversation_id":"paged","revision":4,"title":"Paged",
                    "messages":[
                      {"message_id":"source-message","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"source-part","revision":1,"markdown":"Canonical original","is_final":true}]},
                      {"message_id":"summary-message","role":"assistant","state":"completed","revision":4,"created_at_ms":2,"parts":[{"kind":"compaction","part_id":"compaction-part","revision":4,"summary":"Summary","source_item_ids":["source-part"],"source_fingerprint":"$fingerprint","original_token_count":100,"compacted_token_count":20}]}
                    ],
                    "active_run":null,
                    "surface":{"revision":4,"committed_message_ids":["source-message","summary-message"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":false},
                    "context_usage":{"used_tokens":20,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":4}
                  },
                  "event_sequence":9,"total_message_count":2,"before_ordinal":null,"has_more":false
                }
                """.trimIndent(),
            ),
        )
        val messages = CanonicalConversationProjector.project(window.projection)
        val compaction = window.projection.messages.last().parts.single() as
            ConversationProjectionV1.Part.Compaction
        val request = CompactionSourceNavigator.Request(
            conversationId = "paged",
            compactionPartId = compaction.partId,
            sourceItemIds = compaction.sourceItemIds,
            sourceFingerprint = compaction.sourceFingerprint,
        )

        assertEquals(fingerprint, compaction.sourceFingerprint)
        assertEquals(9, window.eventSequence)
        assertEquals(true, CompactionSourceNavigator.belongsToSnapshot(window, "paged", 4, 9))
        assertEquals(false, CompactionSourceNavigator.belongsToSnapshot(window, "other", 4, 9))
        assertEquals(false, CompactionSourceNavigator.belongsToSnapshot(window, "paged", 5, 9))
        assertEquals(false, CompactionSourceNavigator.belongsToSnapshot(window, "paged", 4, 10))
        assertEquals(listOf("source-part"), compaction.sourceItemIds)
        assertEquals(true, CompactionSourceNavigator.accepts(request, "paged", window.projection))
        val loadedWindow = listOf(messages.last())
        val olderWindow = listOf(messages.first())
        assertEquals(true, CompactionSourceNavigator.accepts(request, "paged", loadedWindow))
        assertEquals(false, CompactionSourceNavigator.accepts(request, "other", loadedWindow))
        assertEquals(
            false,
            CompactionSourceNavigator.accepts(
                request.copy(sourceFingerprint = "b".repeat(64)),
                "paged",
                loadedWindow,
            ),
        )
        assertEquals(
            false,
            CompactionSourceNavigator.accepts(
                request.copy(compactionPartId = "different-compaction-part"),
                "paged",
                loadedWindow,
            ),
        )
        assertEquals(
            false,
            CompactionSourceNavigator.accepts(
                request.copy(sourceItemIds = listOf("different-source-part")),
                "paged",
                loadedWindow,
            ),
        )
        assertEquals(
            null,
            CompactionSourceNavigator.targetMessageId(compaction.sourceItemIds, loadedWindow),
        )
        val merged = CompactionSourceNavigator.mergeOlder(olderWindow, loadedWindow)
        assertEquals(
            "source-message",
            CompactionSourceNavigator.targetMessageId(compaction.sourceItemIds, merged),
        )
        assertEquals("source-message", messages.first().meta?.get("nola.canonicalMessageID"))
        assertEquals(
            listOf("source-message", "summary-message"),
            merged.map { it.id },
        )
        assertEquals(false, CompactionSourceNavigator.canContinue(3, window.beforeOrdinal, window.hasMore))
        assertEquals(false, CompactionSourceNavigator.canContinue(3, 3, true))
        assertEquals(false, CompactionSourceNavigator.canContinue(3, 4, true))
        assertEquals(true, CompactionSourceNavigator.canContinue(3, 2, true))
    }

    @Test
    fun compactionLookupRejectsLatePageAfterConversationReloadOrSwitch() {
        assertEquals(true, CompactionSourceNavigator.isCurrentLookup("a", "a", 7, 7))
        assertEquals(false, CompactionSourceNavigator.isCurrentLookup("a", "b", 7, 7))
        assertEquals(false, CompactionSourceNavigator.isCurrentLookup("a", "a", 7, 8))
    }

    @Test
    fun compactionPagesCommitOldestFirstAndPreserveConcurrentCurrentSuffix() {
        fun message(id: String) = ConversationMessage(
            id = id,
            conversationId = "conversation",
            role = ChatRole.assistant,
            text = id,
            meta = mapOf("nola.canonicalMessageID" to id),
        )
        val initial = message("initial")
        val concurrentSuffix = message("streaming-suffix")
        val merged = CompactionSourceNavigator.mergeFetchedOlderPages(
            pagesNewestToOldest = listOf(
                listOf(message("nearer-page")),
                listOf(message("oldest-page")),
            ),
            current = listOf(initial, concurrentSuffix),
        )

        assertEquals(
            listOf("oldest-page", "nearer-page", "initial", "streaming-suffix"),
            merged.map { it.id },
        )
    }

    private fun fixture(toolKind: String) = """
        {
          "execution_ownership":{"kind":"user"},"contract_version": 1,
          "conversation_id": "conversation-1",
          "revision": 3,
          "title": "Contract",
          "messages": [{
            "message_id": "turn-1:assistant",
            "role": "assistant",
            "state": "running",
            "revision": 3,
            "created_at_ms": 1,
            "parts": [
              {"kind":"text","part_id":"text-1","revision":3,"markdown":"hello","is_final":false},
              {"kind":"reasoning","part_id":"reasoning-1","revision":3,"state":"completed","title":"Inspect","summary":"checked"},
              {"kind":"$toolKind","execution_source":{"kind":"model"},"part_id":"tool-1","revision":3,"tool_call_id":"call-1","capability_id":"file.read","state":"running","display_key":null,"title":"Read","origin":{"kind":"builtin"},"artifacts":[],"input":{"path":"a.txt"},"output":null,"affected_resources":[]}
            ]
          }],
          "active_run":{"run_id":"turn-1","state":"running","active_part_id":"tool-1","display_key":null,"progress":null},
          "context_usage":{"used_tokens":120,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":3}
        }
    """.trimIndent()
}
