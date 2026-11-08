// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

@MainActor
final class IOSConversationProjectionContractTests: XCTestCase {
    func testUserMessageRevisionSurvivesProjectionForExactEditAdmission() throws {
        let json = #"{"message_id":"stable-user-id","role":"user","state":"completed","revision":17,"created_at_ms":1,"parts":[{"kind":"text","part_id":"body","revision":17,"markdown":"Original message","is_final":true}]}"#
        let source = try JSONDecoder().decode(IOSConversationProjectionV1.Message.self, from: Data(json.utf8))
        let presented = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: source, activeRun: nil))
        XCTAssertEqual(presented.metadata["nola.canonicalMessageID"], "stable-user-id")
        XCTAssertEqual(presented.canonicalMessageRevision, 17)
        XCTAssertEqual(presented.content, "Original message")
    }

    func testAllPublicActivityStatesDecodeWithoutCollapsingWaitingOrSealed() throws {
        for state in ["pending", "running", "waiting_for_permission", "waiting_for_input", "completed", "sealed", "cancelled", "failed"] {
            let decoded = try JSONDecoder().decode(IOSConversationProjectionV1.ActivityState.self, from: Data("\"\(state)\"".utf8))
            XCTAssertEqual(decoded.rawValue, state)
        }
        XCTAssertThrowsError(try JSONDecoder().decode(IOSConversationProjectionV1.ActivityState.self, from: Data(#""foreign_state""#.utf8)))
    }

    func testCommittedCacheRatePreservesUnknownAndMeasuredZero() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":0,"write_tokens":1000,"hit_rate_bps":0,"is_compaction":true}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        XCTAssertEqual(usage.cacheUsage?.hit_rate_bps, 0)
        XCTAssertTrue(usage.cacheUsage?.is_compaction == true)
        let unknown = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: unknown).cacheUsage)
    }

    func testContextEvidenceUsesConversationTotalsNotLatestRequestRate() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":0,"write_tokens":null,"hit_rate_bps":0,"is_compaction":true},"conversation_cache_usage":{"input_tokens":10000,"read_tokens":7500,"write_tokens":null,"hit_rate_bps":7500,"is_compaction":false,"reported_requests":5,"unreported_requests":2}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        let evidence = try XCTUnwrap(IOSContextUsageEvidence.committed(usage))
        XCTAssertEqual(evidence.cacheHitRateBps, 7500)
        XCTAssertEqual(evidence.cachedInputTokens, 7500)
        XCTAssertEqual(evidence.cacheReportedRequests, 5)
        XCTAssertEqual(evidence.cacheUnreportedRequests, 2)
        XCTAssertEqual(evidence.usedTokens, 1000)
    }

    func testMissingConversationCacheTotalsDoNotFallBackToLatestRequest() throws {
        let data = Data(#"{"used_tokens":1000,"context_window_tokens":128000,"prompt_token_limit":120000,"estimated":false,"measured_at_revision":9,"cache_usage":{"input_tokens":1000,"read_tokens":1000,"write_tokens":null,"hit_rate_bps":10000,"is_compaction":false}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        let evidence = try XCTUnwrap(IOSContextUsageEvidence.committed(usage))
        XCTAssertNil(evidence.cacheHitRateBps)
        XCTAssertNil(evidence.cachedInputTokens)
    }

    func testNewRunWithoutOccupancyStillPresentsDurableConversationCacheTotals() throws {
        let data = Data(#"{"used_tokens":0,"context_window_tokens":0,"prompt_token_limit":0,"estimated":true,"measured_at_revision":0,"conversation_cache_usage":{"input_tokens":10000,"read_tokens":7500,"write_tokens":null,"hit_rate_bps":7500,"is_compaction":false,"reported_requests":5,"unreported_requests":2}}"#.utf8)
        let usage = try JSONDecoder().decode(IOSConversationProjectionV1.ContextUsage.self, from: data)
        let evidence = try XCTUnwrap(IOSContextUsageEvidence.committed(usage))
        XCTAssertFalse(evidence.hasCommittedTokenTotals)
        XCTAssertEqual(evidence.cacheHitRateBps, 7500)
        let presentation = ConversationContextUsage.withoutBreakdown().replacingTokenTotals(
            usedTokens: evidence.usedTokens, limitTokens: evidence.limitTokens,
            cacheHitRateBps: evidence.cacheHitRateBps, cacheReportedRequests: evidence.cacheReportedRequests,
            cacheUnreportedRequests: evidence.cacheUnreportedRequests, hasCommittedTokenTotals: evidence.hasCommittedTokenTotals)
        XCTAssertFalse(presentation.hasCommittedTokenTotals)
        XCTAssertFalse(presentation.hasEstimatedBreakdown)
        XCTAssertEqual(presentation.cacheReportedRequests, 5)
        XCTAssertEqual(presentation.cacheUnreportedRequests, 2)
        XCTAssertEqual(presentation.cacheHitRateBps, 7500)
    }

    func testStructuredNumberDisplayPreservesOrdinaryIntegerAndFractionFormatting() throws {
        for (json, expected) in [
            ("0", "0"), ("-0.0", "0"), ("42", "42"), ("-42", "-42"),
            ("42.0", "42"), ("0.125", "0.125"), ("-1.5", "-1.5"),
            ("9007199254740992", "9007199254740992"),
            ("-9223372036854775808", "-9223372036854775808"),
        ] {
            let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(json.utf8))
            XCTAssertEqual(value.displayText, expected, json)
        }
    }

    func testStructuredNumberDisplayDoesNotTrapOutsideInt64Range() throws {
        for json in ["-9223372036854777856", "1e100", "-1e100", "1e308", "-1e308"] {
            let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(json.utf8))
            let represented = try XCTUnwrap(Double(json))
            XCTAssertNil(Int64(exactly: represented), "Fixture must exercise the non-Int64 display path")
            XCTAssertEqual(value.displayText, String(represented), json)
        }
        let upperInside = Double(Int64.max).nextDown
        XCTAssertEqual(IOSConversationProjectionV1.StructuredValue.number(upperInside).displayText, "9223372036854774784")
        let lowerOutside = Double(Int64.min).nextDown
        XCTAssertEqual(IOSConversationProjectionV1.StructuredValue.number(lowerOutside).displayText, String(lowerOutside))
    }

    func testStructuredIntegerDecoderAndCoreBridgeRemainExactBeyondDoublePrecision() throws {
        for json in ["9007199254740991", "9007199254740992", "9007199254740993",
                     "-9007199254740993", "9223372036854775807", "9223372036854775808",
                     "18446744073709551615", "-9223372036854775808"] {
            let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(json.utf8))
            XCTAssertEqual(value.displayText, json)
            // StructuredValue is the Core value, not a lossy second bridge.
            let coreValue: AgentCoreJSONValue = value
            let encoded = try JSONEncoder().encode(coreValue)
            XCTAssertEqual(String(decoding: encoded, as: UTF8.self), json)
            XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: encoded), value)
        }
    }

    func testNestedHistoricalStructuredNumbersRenderWithoutNarrowing() throws {
        let value = try JSONDecoder().decode(IOSConversationProjectionV1.StructuredValue.self, from: Data(#"{"count":18446744073709551615,"values":[1e308,-9223372036854775808,0.125],"label":"evidence"}"#.utf8))
        XCTAssertEqual(value.displayText, "{count: 18446744073709551615, label: evidence, values: [1e+308, -9223372036854775808, 0.125]}")
    }

    func testUnloadedHistoricalPartAndCanonicalPreviewRemainDescriptive() throws {
        let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("core/tests/fixtures/historical_unloaded_conversation_projection_v1.json")
        let whole = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(contentsOf: fixture))
        guard case .historical(_, _, let projection) = whole.messages[0].parts[0],
              case .historical(_, _, let opaque) = whole.messages[0].parts[4] else { return XCTFail("Missing historical nodes") }
        XCTAssertEqual(projection.status.state, .notLoaded)
        XCTAssertNil(projection.source)
        XCTAssertNil(projection.binding)
        XCTAssertTrue(projection.fields.isEmpty)
        XCTAssertNil(projection.decodedText)
        XCTAssertNil(projection.timelineText)
        XCTAssertTrue(projection.needsHistoricalDetail)
        XCTAssertTrue(opaque.needsHistoricalDetail)
        XCTAssertEqual(opaque.timelineText, "opaque original evidence")
        XCTAssertTrue(projection.statusLabel.contains("selection_budget"))
        XCTAssertTrue(opaque.statusLabel.contains("opaque original evidence"))
    }

    func testCompleteCanonicalTextDoesNotRequireOpeningMetadataAsOriginal() throws {
        // Real user turns store a displayText metadata envelope rather than an
        // agent_part/message_part_view schema. Its canonical body is still text.
        let text = "完整原文🙂"
        let projection = try canonicalPreview(text: text, totalBytes: text.utf8.count)
        XCTAssertNil(projection.decodedText)
        XCTAssertEqual(projection.timelineText, text)
        XCTAssertFalse(projection.needsHistoricalDetail)
    }

    func testPartialOrInconsistentCanonicalPreviewStillOffersOriginal() throws {
        let text = "原文🙂"
        XCTAssertTrue(try canonicalPreview(text: text, totalBytes: text.utf8.count + 1).needsHistoricalDetail)
        XCTAssertTrue(try canonicalPreview(text: text, totalBytes: text.count).needsHistoricalDetail,
                      "Character count must never be mistaken for a complete UTF-8 body")
        XCTAssertTrue(try canonicalPreview(text: text, totalBytes: text.utf8.count, kind: "opaque_future_kind").needsHistoricalDetail)
    }

    private func canonicalPreview(text: String, totalBytes: Int, kind: String = "text") throws -> IOSConversationProjectionV1.HistoricalProjection {
        let object: [String: Any] = [
            "owner": ["conversation_id": "conversation", "message_id": "message", "part_id": "part"],
            "canonical_revision": 1, "canonical_kind": kind,
            "status": ["state": "unsupported", "code": "schema_missingkind"],
            "fields": [String: String](),
            "canonical_text": ["text": text, "total_utf8_bytes": totalBytes],
        ]
        return try JSONDecoder().decode(IOSConversationProjectionV1.HistoricalProjection.self,
                                        from: JSONSerialization.data(withJSONObject: object))
    }

    func testHistoricalProjectionRemainsDistinctAndNeverCreatesToolEvents() throws {
        let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("core/tests/fixtures/historical_conversation_projection_v1.json")
        let projection = try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(contentsOf: fixture))
        XCTAssertEqual(projection.messages[0].parts.count, 5)
        guard case .historical(_, _, let historical) = projection.messages[0].parts[0] else {
            return XCTFail("Historical text must retain its distinct Part type")
        }
        XCTAssertEqual(historical.decodedText, "  原文\\n🙂  ")
        XCTAssertFalse(historical.needsHistoricalDetail)
        guard case .historical(_, _, let deferred) = projection.messages[0].parts[3],
              case .deferred = deferred.fields["markdown"] else {
            return XCTFail("Deferred fields must retain the original locator")
        }
        XCTAssertNil(deferred.decodedText)
        XCTAssertTrue(deferred.needsHistoricalDetail)
        let message = try XCTUnwrap(IOSConversationProjectionPresenter.messages(from: projection).first)
        XCTAssertFalse(message.content.contains("supported"))
        XCTAssertFalse(message.content.contains("deferred"))
        XCTAssertFalse(message.content.contains("unknown_kind"))
        XCTAssertFalse(message.content.contains("must not run"))
        XCTAssertTrue(message.hostToolEvents.isEmpty)
        XCTAssertEqual(message.thinking, " reasoning 原文 ")
    }

    func testCoreJSONRoundTripPreservesCatalogRevisionBeyondDoublePrecision() throws {
        let expected: UInt64 = 675_868_731_199_239_600
        let decoded = try JSONDecoder().decode(
            AgentCoreJSONValue.self,
            from: Data(#"{"capability_catalog_revision":675868731199239600}"#.utf8)
        )
        guard case .object(let object) = decoded else {
            return XCTFail("An unsafe JSON integer must use the lossless UInt64 representation")
        }
        switch object["capability_catalog_revision"] {
        case .signedInteger(let revision):
            XCTAssertEqual(UInt64(revision), expected)
        case .unsignedInteger(let revision):
            XCTAssertEqual(revision, expected)
        default:
            XCTFail("An unsafe JSON integer must not be represented as Double")
        }

        let encoded = try JSONEncoder().encode(decoded)
        let roundTripped = try JSONDecoder().decode(
            [String: UInt64].self,
            from: encoded
        )
        XCTAssertEqual(roundTripped["capability_catalog_revision"], expected)
        XCTAssertTrue(String(decoding: encoded, as: UTF8.self).contains(String(expected)))
    }

    func testJournalNotificationsAreHintsWithoutLegacyProjectionAdapters() throws {
        let identity: [String: Any] = ["source_store_epoch": "epoch", "conversation_id": "conversation", "run_id": "run", "assistant_message_id": "assistant"]
        let event = WebAgentProvider.parseSSEObject(["type": "assistant_journal_committed_v1",
            "session_id": "conversation", "source_host_id": "paired-host",
            "through": ["identity": identity, "sequence": "9007199254740993", "record_digest": String(repeating: "a", count: 64)]])
        guard case .assistantJournalCommitted(let raw) = event else { return XCTFail("Missing journal hint") }
        let hint = try JSONDecoder().decode(IOSAssistantJournalNotification.self, from: JSONEncoder().encode(raw))
        XCTAssertEqual(hint.through.sequence, "9007199254740993")
        let terminal = WebAgentProvider.parseSSEObject(["type": "assistant_run_terminal_committed_v1",
            "session_id": "conversation", "source_host_id": "paired-host", "run_id": "run",
            "seal_digest": String(repeating: "b", count: 64)])
        guard case .assistantRunTerminalSealed = terminal else { return XCTFail("Missing terminal hint") }
    }

    func testProjectionIsTheSingleInputForRestoredTextReasoningToolAndContext() throws {
        let json = #"""
        {
          "contract_version": 1, "execution_ownership": {"kind": "user"},
          "conversation_id": "conversation-1",
          "revision": 9,
          "title": "Projection contract",
          "messages": [{
            "message_id": "run-1:assistant",
            "role": "assistant",
            "state": "running",
            "revision": 8,
            "created_at_ms": 1787356800000,
            "parts": [
              {"kind":"reasoning","part_id":"reasoning-1","revision":1,"state":"completed","title":"Inspect source","summary":"Inspecting the source"},
              {"kind":"tool","execution_source":{"kind":"model"},"part_id":"tool-1","revision":2,"tool_call_id":"call-1","capability_id":"file.read","state":"completed","title":"Read source","origin":{"kind":"local"},"artifacts":[],"display_key":"tool.file.read","input":{"path":"/tmp/source.md"},"output":{"bytes":42},"affected_resources":[{"resource_type":"file","uri":"file:///tmp/source.md","operation":"read"}]},
              {"kind":"text","part_id":"text-1","revision":5,"markdown":"# Result\n\nBody","is_final":false}
            ]
          }],
          "active_run":{"run_id":"run-1","state":"running","active_part_id":"text-1","display_key":"agent.composing","progress":null},
          "context_usage":{"used_tokens":2048,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":9}
        }
        """#

        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertEqual(messages.count, 1)
        XCTAssertEqual(messages[0].content, "# Result\n\nBody")
        XCTAssertEqual(messages[0].thinking, "Inspecting the source")
        XCTAssertEqual(messages[0].hostToolEvents.first?.name, "file.read")
        XCTAssertEqual(messages[0].tracePhase, "agent.composing")
        XCTAssertTrue(messages[0].isCanonicalLive)
        XCTAssertEqual(messages[0].liveHostToolEvents.map(\.id), [])
        XCTAssertEqual(messages[0].canonicalParts.count, 3)
        XCTAssertEqual(projection.contextUsage.usedTokens, 2048)
        XCTAssertEqual(projection.contextUsage.promptTokenLimit, 57344)
    }

    func testModelAttemptFailureKeepsFailedAndFallbackIdentity() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"fallback","revision":3,"title":"Fallback",
          "messages":[{"message_id":"assistant","role":"assistant","state":"completed","revision":3,"created_at_ms":1,"parts":[
            {"kind":"model_attempt","part_id":"model-attempt:one","revision":3,"state":"failed","attempt_id":"one","provider":"OpenRouter","model":"free-model","failure":{"code":"rate_limited","message":"The provider rate limit was reached.","retryable":true},"fallback_attempt_id":"two","fallback_provider":"DeepSeek","fallback_model":"deepseek-chat"},
            {"kind":"text","part_id":"answer","revision":3,"markdown":"Recovered answer","is_final":true}
          ]}],
          "active_run":null,
          "surface":{"revision":3,"committed_message_ids":["assistant"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":["model-attempt:one"],"show_assistant_placeholder":false},
          "context_usage":{"used_tokens":0,"context_window_tokens":16384,"prompt_token_limit":12288,"estimated":false,"measured_at_revision":3}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        guard case let .modelAttempt(attempt) = projection.messages[0].parts[0] else {
            return XCTFail("Expected model attempt Part")
        }

        XCTAssertEqual(attempt.provider, "OpenRouter")
        XCTAssertEqual(attempt.model, "free-model")
        XCTAssertEqual(attempt.failure.code, "rate_limited")
        XCTAssertEqual(attempt.fallbackProvider, "DeepSeek")
        XCTAssertEqual(attempt.fallbackModel, "deepseek-chat")
    }

    func testSurfaceOrdersCommittedHistoryBeforeLivePlaceholder() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"surface","revision":3,"title":"Surface",
          "messages":[
            {"message_id":"live","role":"assistant","state":"running","revision":3,"created_at_ms":2,"parts":[]},
            {"message_id":"history","role":"assistant","state":"completed","revision":2,"created_at_ms":1,"parts":[{"kind":"text","part_id":"text","revision":2,"markdown":"Done","is_final":true}]}
          ],
          "active_run":{"run_id":"run","state":"running","active_part_id":null,"display_key":null,"progress":null},
          "surface":{"revision":3,"committed_message_ids":["history"],"live_message_id":"live","current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":true},
          "context_usage":{"used_tokens":1,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":true,"measured_at_revision":3}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertEqual(messages.map { $0.metadata["nola.canonicalMessageID"] }, ["history", "live"])
        XCTAssertFalse(messages[0].isCanonicalLive)
        XCTAssertTrue(messages[1].isCanonicalLive)
        XCTAssertEqual(messages[1].metadata[ChatConversationSurfaceMeta.placeholderKey], "true")
    }

    func testCompactionSourceIdentityDecodesAndLocatesCanonicalMessage() throws {
        let fingerprint = String(repeating: "a", count: 64)
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"compaction-source","revision":4,"title":"Compaction",
          "messages":[
            {"message_id":"source-message","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"source-part","revision":1,"markdown":"Canonical original","is_final":true}]},
            {"message_id":"summary-message","role":"assistant","state":"completed","revision":4,"created_at_ms":2,"parts":[{"kind":"compaction","part_id":"compaction-part","revision":4,"summary":"Bounded summary","retained_item_ids":["source-part"],"source_fingerprint":"\#(fingerprint)","original_token_count":120,"compacted_token_count":30}]}
          ],
          "active_run":null,
          "surface":{"revision":4,"committed_message_ids":["source-message","summary-message"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":false},
          "context_usage":{"used_tokens":30,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":4}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        guard case let .compaction(_, _, _, sourceIDs, decodedFingerprint, _, _) =
            try XCTUnwrap(messages.last?.canonicalParts.first) else {
            return XCTFail("Expected a compaction Part")
        }
        XCTAssertEqual(sourceIDs, ["source-part"])
        XCTAssertEqual(decodedFingerprint, fingerprint)
        XCTAssertEqual(
            ChatCompactionSourceNavigator.targetMessageID(
                sourceItemIDs: sourceIDs,
                messages: messages
            ),
            messages.first?.id
        )
        let conversationID = UUID(uuidString: "11111111-2222-3333-4444-555555555555")!
        let request = IOSCompactionSourceRequest(
            conversationID: conversationID,
            compactionPartID: "compaction-part",
            sourceItemIDs: sourceIDs,
            sourceFingerprint: fingerprint
        )
        XCTAssertTrue(ChatCompactionSourceNavigator.accepts(
            request,
            conversationID: conversationID,
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            request,
            conversationID: UUID(uuidString: "22222222-3333-4444-5555-666666666666"),
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            IOSCompactionSourceRequest(
                conversationID: conversationID,
                compactionPartID: "compaction-part",
                sourceItemIDs: sourceIDs,
                sourceFingerprint: String(repeating: "b", count: 64)
            ),
            conversationID: conversationID,
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            IOSCompactionSourceRequest(
                conversationID: conversationID,
                compactionPartID: "different-compaction-part",
                sourceItemIDs: sourceIDs,
                sourceFingerprint: fingerprint
            ),
            conversationID: conversationID,
            messages: messages
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.accepts(
            IOSCompactionSourceRequest(
                conversationID: conversationID,
                compactionPartID: "compaction-part",
                sourceItemIDs: ["different-source-part"],
                sourceFingerprint: fingerprint
            ),
            conversationID: conversationID,
            messages: messages
        ))
    }

    func testCompactionSourceWindowPrependsCanonicalPageAndStopsAtHistoryEnd() throws {
        let json = #"""
        {
          "projection":{
            "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"paged-source","revision":5,"title":"Paged",
            "messages":[
              {"message_id":"older-source","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"canonical-source-part","revision":1,"markdown":"Canonical original","is_final":true}]},
              {"message_id":"older-neighbor","role":"assistant","state":"completed","revision":2,"created_at_ms":2,"parts":[{"kind":"text","part_id":"neighbor-part","revision":1,"markdown":"Neighbor","is_final":true}]}
            ],
            "active_run":null,
            "surface":{"revision":5,"committed_message_ids":["older-source","older-neighbor"],"live_message_id":null,"current_activity_part_id":null,"archived_activity_part_ids":[],"show_assistant_placeholder":false},
            "context_usage":{"used_tokens":20,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":5}
          },
          "event_sequence":9,"total_message_count":4,"before_ordinal":null,"has_more":false,"assistant_journal_heads":[]
        }
        """#
        let window = try JSONDecoder().decode(
            IOSConversationProjectionWindowV1.self,
            from: Data(json.utf8)
        )
        let older = IOSConversationProjectionPresenter.messages(from: window.projection)
        XCTAssertEqual(window.eventSequence, 9)
        XCTAssertTrue(ChatCompactionSourceNavigator.belongsToSnapshot(
            window,
            revision: 5,
            eventSequence: 9
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.belongsToSnapshot(
            window,
            revision: 6,
            eventSequence: 9
        ))
        XCTAssertFalse(ChatCompactionSourceNavigator.belongsToSnapshot(
            window,
            revision: 5,
            eventSequence: 10
        ))
        let alreadyLoaded = [
            ChatMessage(
                role: .assistant,
                content: "Summary",
                metadata: ["nola.canonicalMessageID": "summary-message"]
            )
        ]
        let merged = ChatCompactionSourceNavigator.mergingOlder(older, into: alreadyLoaded)

        XCTAssertEqual(
            merged.compactMap { $0.metadata["nola.canonicalMessageID"] },
            ["older-source", "older-neighbor", "summary-message"]
        )
        XCTAssertEqual(
            ChatCompactionSourceNavigator.targetMessageID(
                sourceItemIDs: ["canonical-source-part"],
                messages: merged
            ),
            older.first?.id
        )
        XCTAssertFalse(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: window.beforeOrdinal,
                hasMore: window.hasMore
            )
        )
        XCTAssertFalse(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: 3,
                hasMore: true
            ),
            "A stale cursor must terminate instead of repeatedly loading the same page"
        )
        XCTAssertFalse(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: 4,
                hasMore: true
            ),
            "A cursor moving toward newer history must not drive another older-page request"
        )
        XCTAssertTrue(
            ChatCompactionSourceNavigator.canContinue(
                previousCursor: 3,
                nextCursor: 2,
                hasMore: true
            )
        )
    }

    func testCompactionSourcePagesCommitOldestFirstAndPreserveConcurrentSuffix() {
        func message(_ canonicalID: String) -> ChatMessage {
            ChatMessage(
                role: .assistant,
                content: canonicalID,
                metadata: ["nola.canonicalMessageID": canonicalID]
            )
        }

        let merged = ChatCompactionSourceNavigator.mergingFetchedOlderPages(
            [
                [message("nearer-page")],
                [message("oldest-page")],
            ],
            into: [message("initial-window"), message("concurrent-streaming-suffix")]
        )

        XCTAssertEqual(
            merged.compactMap { $0.metadata["nola.canonicalMessageID"] },
            ["oldest-page", "nearer-page", "initial-window", "concurrent-streaming-suffix"]
        )
    }

    func testUnknownPartKindFailsInsteadOfFallingBackToText() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"c","revision":1,"title":"",
          "messages":[{"message_id":"m","role":"assistant","state":"running","revision":1,"created_at_ms":1,"parts":[{"kind":"provider_delta","part_id":"p","revision":1}]}],
          "active_run":null,
          "context_usage":{"used_tokens":0,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":true,"measured_at_revision":1}
        }
        """#
        XCTAssertThrowsError(
            try JSONDecoder().decode(IOSConversationProjectionV1.self, from: Data(json.utf8))
        )
    }

    func testColdProjectionRestorePreservesExactAppendOrderAndPendingAssistant() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"cold","revision":7,"title":"Cold restore",
          "messages":[
            {"message_id":"user","role":"user","state":"completed","revision":1,"created_at_ms":1,"parts":[{"kind":"text","part_id":"u","revision":1,"markdown":"Question","is_final":true}]},
            {"message_id":"assistant","role":"assistant","state":"running","revision":6,"created_at_ms":2,"parts":[
              {"kind":"text","part_id":"a1","revision":1,"markdown":"## Result\n\n","is_final":false},
              {"kind":"text","part_id":"a2","revision":2,"markdown":"First paragraph.","is_final":false},
              {"kind":"text","part_id":"a3","revision":3,"markdown":"\n\n```swift\nlet value = 42","is_final":false}
            ]}
          ],
          "active_run":{"run_id":"run","state":"running","active_part_id":"a3","display_key":"agent.composing","progress":null},
          "context_usage":{"used_tokens":42,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":7}
        }
        """#
        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertEqual(messages.map(\.role), [.user, .assistant])
        XCTAssertEqual(
            messages[1].content,
            "## Result\n\nFirst paragraph.\n\n```swift\nlet value = 42"
        )
        XCTAssertEqual(messages[1].tracePhase, "agent.composing")
        XCTAssertFalse(messages[1].thinkingFinished)
    }

    func testCompletedGoalProjectionCommitsOneAnswerAndReleasesComposer() throws {
        let json = #"""
        {
          "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"goal-completed","revision":12,"title":"Completed goal",
          "messages":[{
            "message_id":"goal-turn:assistant","role":"assistant","state":"completed","revision":11,"created_at_ms":1,
            "parts":[
              {"kind":"tool","execution_source":{"kind":"model"},"part_id":"goal-tool","revision":1,"tool_call_id":"goal-call","capability_id":"goal_complete","state":"completed","title":"Complete goal","origin":{"kind":"local"},"artifacts":[],"display_key":"tool.goal_complete","input":{"status":"completed"},"output":{"goal_status":"completed"},"affected_resources":[]},
              {"kind":"text","part_id":"goal-answer","revision":2,"markdown":"Goal completed exactly once.","is_final":true}
            ]
          }],
          "active_run":null,
          "context_usage":{"used_tokens":128,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":false,"measured_at_revision":12}
        }
        """#

        let projection = try JSONDecoder().decode(
            IOSConversationProjectionV1.self,
            from: Data(json.utf8)
        )
        let messages = IOSConversationProjectionPresenter.messages(from: projection)

        XCTAssertNil(projection.activeRun)
        XCTAssertEqual(messages.count, 1)
        XCTAssertEqual(messages[0].content, "Goal completed exactly once.")
        XCTAssertEqual(messages[0].hostToolEvents.map(\.name), ["goal_complete"])
        XCTAssertFalse(ChatProcessingState.completed.blocksInput)
    }
}
