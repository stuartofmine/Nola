// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies the Mac projection boundary consumes Application ABI JSON without Host queries.

import Foundation
import Testing
import NolaCore
@testable import Nola

struct MacCanonicalSessionProjectionTests {
    private final class ResourceBundleMarker {}

    @Test
    func canonicalJSONDisplayIsStableAcrossObjectInsertionOrders() throws {
        let orders = [
            #"{"z":3,"a":{"y":2,"b":1},"list":[{"d":4,"c":3},2]}"#,
            #"{"list":[{"c":3,"d":4},2],"a":{"b":1,"y":2},"z":3}"#,
        ]
        let expected = #"{"a":{"b":1,"y":2},"list":[{"c":3,"d":4},2],"z":3}"#
        for _ in 0..<32 {
            for source in orders {
                let value = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(source.utf8))
                #expect(value.displayString == expected)
                #expect(try JSONDecoder().decode(MacCanonicalJSONValue.self,
                    from: Data(value.displayString.utf8)) == value)
            }
        }
        #expect(MacCanonicalJSONValue.string("原文\nnot JSON").displayString == "原文\nnot JSON")
    }

    @Test
    func committedModelRetryAndTerminalFailuresReplaceTransportProtocolPreviews() throws {
        for terminalState in ["completed", "failed"] {
            let isFailed = terminalState == "failed"
            let answer = isFailed ? "" : "Verified answer after the protocol retry."
            let reason = isFailed
                ? "The model repeatedly emitted pseudo tool execution or unverified command output"
                : "The previous model response was rejected before the protocol retry."
            let providerNamespace = "openai-compatible.chat-completions:" + String(repeating: "a", count: 64)
            let fallbackNamespace = "openai-compatible.responses:" + String(repeating: "b", count: 64)
            var attempt: [String: Any] = [
                "kind": "model_attempt",
                "part_id": "model-attempt:protocol-attempt",
                "revision": 3,
                "state": "failed",
                "attempt_id": "protocol-attempt",
                "provider": providerNamespace,
                "model": "deepseek-flash",
                "failure": [
                    "code": isFailed ? "invalid_model_execution_claim" : "model_resampled",
                    "message": reason,
                    "retryable": !isFailed,
                ],
            ]
            if !isFailed {
                attempt["fallback_attempt_id"] = "verified-attempt"
                attempt["fallback_provider"] = fallbackNamespace
                attempt["fallback_model"] = "deepseek-chat"
            }
            var parts = [attempt]
            if !isFailed {
                parts.append([
                    "kind": "text", "part_id": "verified-answer", "revision": 3,
                    "markdown": answer, "is_final": true,
                ])
            }
            let fixture: [String: Any] = [
                "contract_version": 1,
                "conversation_id": "protocol-conversation",
                "revision": 3,
                "title": "Protocol retry",
                "messages": [[
                    "message_id": "protocol-assistant", "role": "assistant",
                    "state": terminalState, "revision": 3, "created_at_ms": 1,
                    "parts": parts,
                ]],
                "active_run": NSNull(),
                "surface": [
                    "revision": 3, "committed_message_ids": ["protocol-assistant"],
                    "live_message_id": NSNull(), "current_activity_part_id": NSNull(),
                    "archived_activity_part_ids": ["model-attempt:protocol-attempt"],
                    "show_assistant_placeholder": false,
                ],
                "context_usage": [
                    "used_tokens": 0, "context_window_tokens": 16384,
                    "prompt_token_limit": 12288, "estimated": false,
                    "measured_at_revision": 3,
                ],
            ]
            let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.self,
                from: JSONSerialization.data(withJSONObject: fixture))
            guard case .modelAttempt(let canonicalAttempt) = projection.messages[0].parts[0] else {
                Issue.record("Expected a canonical ModelAttempt")
                continue
            }
            #expect(canonicalAttempt.provider == providerNamespace)
            #expect(canonicalAttempt.fallbackProvider == (isFailed ? nil : fallbackNamespace))
            let message = try #require(MacCanonicalConversationRepository.projectMessages(projection).first)
            var transient = MacAssistantPresentation.processing()
            for index in 1...3 {
                transient.upsertStatus(externalID: "model-protocol:event-\(index)",
                    title: "Response failed", detail: "Retrying", state: .failed)
            }
            transient.upsertStatus(externalID: "model-route:terminal:event",
                title: "Response failed", detail: "Transport duplicate", state: .failed)
            transient.finish()
            let previous = MacChatMessage(id: message.id, role: .assistant, text: answer,
                createdAt: message.createdAt, presentation: transient)
            let refreshed = try #require(MacConversationPresentationOverlay.preservingCompletedText(
                in: [message], previous: [previous]).first)
            let trace = try #require(refreshed.presentation?.traceItems.first)

            #expect(refreshed.deliveryState == (isFailed ? .failed : .completed))
            #expect(refreshed.text == answer)
            #expect(refreshed.presentation?.traceItems.count == 1)
            #expect(trace.externalID == "model-attempt:protocol-attempt")
            #expect(trace.detail.contains("deepseek-flash"))
            #expect(trace.detail.contains(reason))
            #expect(!trace.detail.contains(providerNamespace))
            #expect(!trace.detail.contains(fallbackNamespace))
            #expect(!trace.detail.contains("openai-compatible."))
            if !isFailed { #expect(trace.detail.contains("deepseek-chat")) }
            #expect(trace.state == (isFailed ? .failed : .incomplete))
        }
    }

    @Test
    func failedAssistantWithoutTextRemainsVisibleAfterColdProjection() throws {
        let data = Data(#"""
        {
          "contract_version": 1,
          "conversation_id": "failed-response",
          "revision": 2,
          "title": "Failed response",
          "messages": [{
            "message_id": "assistant-failed",
            "role": "assistant",
            "state": "failed",
            "revision": 2,
            "created_at_ms": 1,
            "parts": []
          }],
          "active_run": null,
          "surface": {
            "revision": 2,
            "committed_message_ids": ["assistant-failed"],
            "live_message_id": null,
            "current_activity_part_id": null,
            "archived_activity_part_ids": [],
            "show_assistant_placeholder": false
          },
          "context_usage": {
            "used_tokens": 0,
            "context_window_tokens": 16384,
            "prompt_token_limit": 12288,
            "estimated": false,
            "measured_at_revision": 2
          }
        }
        """#.utf8)
        let projection = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.self,
            from: data
        )

        let messages = try MacCanonicalConversationRepository.projectMessages(projection)

        #expect(messages.count == 1)
        #expect(messages[0].deliveryState == .failed)
        #expect(messages[0].text.isEmpty)
    }

    @Test
    func modelAttemptSuccessorProjectsNeutralRouteChangeAndPreservesFailureReason() throws {
        let data = Data(#"""
        {
          "contract_version": 1,
          "conversation_id": "fallback-response",
          "revision": 3,
          "title": "Fallback response",
          "messages": [{
            "message_id": "assistant-fallback",
            "role": "assistant",
            "state": "completed",
            "revision": 3,
            "created_at_ms": 1,
            "parts": [{
              "kind": "model_attempt",
              "part_id": "model-attempt:attempt-1",
              "revision": 3,
              "state": "failed",
              "attempt_id": "attempt-1",
              "provider": "OpenRouter",
              "model": "free-model",
              "failure": {
                "code": "rate_limited",
                "message": "The provider rate limit was reached.",
                "retryable": true
              },
              "fallback_attempt_id": "attempt-2",
              "fallback_provider": "DeepSeek",
              "fallback_model": "deepseek-chat"
            }, {
              "kind": "text",
              "part_id": "answer",
              "revision": 3,
              "markdown": "Recovered answer",
              "is_final": true
            }]
          }],
          "active_run": null,
          "surface": {
            "revision": 3,
            "committed_message_ids": ["assistant-fallback"],
            "live_message_id": null,
            "current_activity_part_id": null,
            "archived_activity_part_ids": ["model-attempt:attempt-1"],
            "show_assistant_placeholder": false
          },
          "context_usage": {
            "used_tokens": 0,
            "context_window_tokens": 16384,
            "prompt_token_limit": 12288,
            "estimated": false,
            "measured_at_revision": 3
          }
        }
        """#.utf8)

        let projection = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.self,
            from: data
        )
        let message = try #require(
            MacCanonicalConversationRepository.projectMessages(projection).first
        )
        let failure = try #require(message.presentation?.traceItems.first)

        #expect(failure.title == String(localized: "Model route changed"))
        #expect(failure.state == .incomplete)
        #expect(failure.detail.contains("free-model"))
        #expect(!failure.detail.contains("OpenRouter"))
        #expect(failure.detail.contains("The provider rate limit was reached."))
        #expect(failure.detail.contains("deepseek-chat"))
        #expect(!failure.detail.contains("DeepSeek"))
        #expect(message.text == "Recovered answer")
    }

    @Test
    func unloadedHistoricalPartRetainsExplicitBudgetStateAndCanonicalPreview() throws {
        let fixture = try #require(Bundle(for: ResourceBundleMarker.self).url(
            forResource: "historical_unloaded_conversation_projection_v1", withExtension: "json"
        ))
        let whole = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.self, from: Data(contentsOf: fixture))
        guard case .historical(let value) = whole.messages[0].parts[0],
              case .historical(let opaque) = whole.messages[0].parts[4] else { Issue.record("Missing historical nodes"); return }
        let projection = value.projection
        #expect(projection.status.state == .notLoaded)
        #expect(projection.source == nil && projection.binding == nil)
        #expect(projection.fields.isEmpty && projection.decodedText == nil)
        #expect(projection.statusLabel.contains("selection_budget"))
        #expect(opaque.projection.statusLabel.contains("opaque original evidence"))
    }

    @Test
    func historicalProjectionHasNoLiveToolPresentation() throws {
        let fixture = try #require(Bundle(for: ResourceBundleMarker.self).url(
            forResource: "historical_conversation_projection_v1", withExtension: "json"
        ))
        let projection = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.self, from: Data(contentsOf: fixture))
        #expect(projection.messages[0].parts.count == 5)
        guard case .historical(let text) = projection.messages[0].parts[0] else {
            Issue.record("Historical text was converted into a live Part")
            return
        }
        #expect(text.projection.decodedText == "  原文\\n🙂  ")
        guard case .historical(let deferred) = projection.messages[0].parts[3],
              case .deferred = deferred.projection.fields["markdown"] else {
            Issue.record("Deferred locator was lost")
            return
        }
        #expect(deferred.projection.decodedText == nil)
        let message = try MacCanonicalConversationRepository.projectMessage(projection.messages[0])
        #expect(message.text == "  原文\\n🙂  ")
        #expect(!message.text.contains("Historical content"))
        #expect(!message.text.contains("supported"))
        #expect(!message.text.contains("deferred"))
        #expect(!message.text.contains("unknown_kind"))
        #expect(!message.text.contains("must not run"))
        #expect(message.presentation?.traceItems.contains(where: { $0.kind == .tool }) == false)
        #expect(message.presentation?.traceItems.count == 1)
        #expect(message.presentation?.traceItems.first?.title == String(localized: "Thinking Process"))
        #expect(message.presentation?.traceItems.allSatisfy { $0.state == .completed } == true)
    }

    @Test
    func transcriptKeepsLegacyProseAndActualToolWhileDroppingHistoricalMetadataRows() throws {
        let data = Data(#"""
        {
          "contract_version": 1,
          "conversation_id": "history-display",
          "revision": 4,
          "title": "History",
          "messages": [
            {
              "message_id": "user-message",
              "role": "user",
              "state": "completed",
              "revision": 1,
              "created_at_ms": 1,
              "parts": [{
                "kind": "historical",
                "part_id": "legacy-user-text",
                "revision": 1,
                "projection": {
                  "owner": {"conversation_id": "history-display", "message_id": "user-message", "part_id": "legacy-user-text"},
                  "canonical_revision": 4,
                  "schema": "message_part_view",
                  "canonical_kind": "text",
                  "binding": "canonical",
                  "status": {"state": "unsupported", "code": "schema_missingfield"},
                  "fields": {},
                  "canonical_text": {"text": "背诵出师表", "total_utf8_bytes": 15}
                }
              }]
            },
            {
              "message_id": "metadata-only-assistant",
              "role": "assistant",
              "state": "completed",
              "revision": 2,
              "created_at_ms": 2,
              "parts": [{
                "kind": "historical",
                "part_id": "legacy-parameters",
                "revision": 1,
                "projection": {
                  "owner": {"conversation_id": "history-display", "message_id": "metadata-only-assistant", "part_id": "legacy-parameters"},
                  "canonical_revision": 4,
                  "schema": "message_part_view",
                  "canonical_kind": "tool",
                  "binding": "canonical",
                  "status": {"state": "supported"},
                  "fields": {
                    "input": {"availability": "complete", "value": {"query": "must stay in details"}},
                    "state": {"availability": "complete", "value": "completed"}
                  }
                }
              }]
            },
            {
              "message_id": "actual-tool-assistant",
              "role": "assistant",
              "state": "completed",
              "revision": 3,
              "created_at_ms": 3,
              "parts": [{
                "kind": "tool", "execution_source": {"kind": "model"},
                "part_id": "actual-tool",
                "revision": 1,
                "tool_call_id": "call-1",
                "capability_id": "read_file",
                "state": "completed",
                "display_key": "Read file",
                "title": "read_file",
                "origin": {"kind": "host"},
                "artifacts": [],
                "input": {"path": "notes.txt"},
                "output": {"text": "done"},
                "affected_resources": []
              }]
            }
          ],
          "active_run": null,
          "surface": {
            "revision": 4,
            "committed_message_ids": ["user-message", "metadata-only-assistant", "actual-tool-assistant"],
            "live_message_id": null,
            "current_activity_part_id": null,
            "archived_activity_part_ids": [],
            "show_assistant_placeholder": false
          },
          "context_usage": {
            "used_tokens": 0,
            "context_window_tokens": 65536,
            "prompt_token_limit": 61440,
            "estimated": true,
            "measured_at_revision": 4
          }
        }
        """#.utf8)
        let projection = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.self,
            from: data
        )
        let messages = try MacCanonicalConversationRepository.projectMessages(projection)

        #expect(messages.count == 2)
        #expect(messages[0].role == .user)
        #expect(messages[0].text == "背诵出师表")
        #expect(!messages[0].text.contains("Historical content"))
        #expect(!messages[0].text.contains("schema_missingfield"))
        #expect(messages[1].role == .assistant)
        #expect(messages[1].text.isEmpty)
        #expect(messages[1].presentation?.traceItems.count == 1)
        #expect(messages[1].presentation?.traceItems.first?.kind == .tool)
        #expect(messages[1].presentation?.traceItems.first?.title == "read_file")
    }

    @Test
    func swiftCallsNativeKernelWithOwnedLiveText() throws {
        let initial = Data(#"""
        {"assistant_journal_heads":[],"event_sequence":5,"total_message_count":1,"before_ordinal":null,"has_more":false,"deferred_text_parts":[],
         "stream_owners":[{"message_id":"assistant","part_id":"text","run_id":"run","execution":{"type":"model_attempt","attempt_id":"attempt"}}],
         "projection":{"execution_ownership":{"kind":"user"},"contract_version":2,"conversation_id":"native","revision":1,"title":"Native",
          "messages":[{"message_id":"assistant","role":"assistant","state":"running","revision":1,
            "created_at_ms":1,"parts":[{"kind":"text","part_id":"text","revision":1,"markdown":"Hello","is_final":false}]}],
          "active_run":null,"context_usage":{"used_tokens":0,"context_window_tokens":65536,
            "prompt_token_limit":61440,"estimated":true,"measured_at_revision":0}}}
        """#.utf8)
        let handle = try #require(initial.withUnsafeBytes { bytes in
            nola_core_projection_new(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        })
        defer { nola_core_projection_free(handle) }
        func consume(_ result: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
            let result = try #require(result)
            defer { nola_core_free_c_string(result) }
            let data = Data(String(cString: result).utf8)
            return try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        }
        func append(attempt: String) throws -> [String: Any] {
            let data = try JSONSerialization.data(withJSONObject: [[
                "identity": ["event_id": "next", "conversation_id": "native", "run_id": "run",
                    "attempt_id": attempt, "sequence": 6],
                "mutation": ["type": "append_text_preview", "message_id": "assistant",
                    "part_id": "text", "revision": 2, "delta": " world"]
            ]])
            return try consume(data.withUnsafeBytes { bytes in
                nola_core_projection_apply(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
            })
        }
        #expect(try append(attempt: "another-request")["ok"] as? Bool == false)
        #expect(try append(attempt: "attempt")["ok"] as? Bool == true)
        let duplicate = try #require(append(attempt: "attempt")["result"] as? [String: Any])
        #expect(duplicate["duplicates"] as? Int == 1)
        let snapshot = try consume(nola_core_projection_snapshot(handle))
        let result = try #require(snapshot["result"] as? [String: Any])
        #expect(result["sequence"] as? Int == 6)
        let projection = try #require(result["projection"] as? [String: Any])
        let messages = try #require(projection["messages"] as? [[String: Any]])
        let parts = try #require(messages.first?["parts"] as? [[String: Any]])
        #expect(parts.first?["markdown"] as? String == "Hello world")
    }

    @Test
    func windowPreservesUnavailableAndEmptyStreamOwnershipDistinctly() throws {
        var payload: [String: Any] = [
            "projection": [
                "contract_version": 1, "conversation_id": "window", "revision": 1,
                "title": "Window", "messages": [], "active_run": NSNull(),
                "surface": ["revision": 1, "committed_message_ids": [], "live_message_id": NSNull(),
                    "current_activity_part_id": NSNull(), "archived_activity_part_ids": [],
                    "show_assistant_placeholder": false],
                "context_usage": ["used_tokens": 0, "context_window_tokens": 65536,
                    "prompt_token_limit": 61440, "estimated": true, "measured_at_revision": 1]
            ],
            "event_sequence": 12, "total_message_count": 0,
            "before_ordinal": NSNull(), "has_more": false, "stream_owners": NSNull(),
            "assistant_journal_heads": [] as [String], "deferred_text_parts": [] as [String]
        ]
        func decode(_ value: [String: Any]) throws -> MacCanonicalConversationWindow {
            try JSONDecoder().decode(MacCanonicalConversationWindow.self,
                from: JSONSerialization.data(withJSONObject: value))
        }
        let unavailable = try decode(payload)
        #expect(unavailable.streamOwners == nil)
        #expect(unavailable.eventSequence == 12)
        payload["stream_owners"] = [] as [String]
        #expect(try decode(payload).streamOwners == [])
        payload.removeValue(forKey: "stream_owners")
        #expect(throws: (any Error).self) { try decode(payload) }

        let owner = try JSONDecoder().decode(MacCanonicalConversationWindow.StreamOwner.self,
            from: Data(#"{"message_id":"assistant","part_id":"text","run_id":"run","execution":{"type":"model_attempt","attempt_id":"attempt"}}"#.utf8))
        #expect(owner.messageID == "assistant")
        #expect(owner.partID == "text")
        #expect(owner.runID == "run")
        #expect(owner.execution == .modelAttempt(attemptID: "attempt"))
        let delegated = try JSONDecoder().decode(MacCanonicalConversationWindow.StreamOwner.self,
            from: Data(#"{"message_id":"assistant","part_id":"text","run_id":"run","execution":{"type":"delegated_invocation","invocation_id":"external"}}"#.utf8))
        #expect(delegated.execution == .delegatedInvocation(invocationID: "external"))
        #expect(throws: (any Error).self) {
            try JSONDecoder().decode(MacCanonicalConversationWindow.StreamOwner.self,
                from: Data(#"{"message_id":"assistant","part_id":"text","run_id":"run","execution":{"type":"delegated_invocation","invocation_id":"external","attempt_id":"fake"}}"#.utf8))
        }
    }

    @Test
    func delegatedProjectionEventRetainsInvocationIdentity() throws {
        let wire = #"{"scope":{"type":"conversation","conversation_id":"history"},"cursor":{"scope":{"type":"conversation","conversation_id":"history"},"sequence":1},"identity":{"event_id":"event","conversation_id":"history","run_id":"run","delegated_invocation_id":"external"},"mutation":{"type":"conversation","mutation":{"type":"invalidate_projection","revision":2}},"occurred_at":"2026-09-09T00:00:00Z"}"#
        let event = try JSONDecoder().decode(MacCanonicalConversationProjectionEvent.self, from: Data(wire.utf8))
        #expect(event.delegatedInvocationID == "external")
        #expect(event.attemptID == nil)
        let ambiguous = wire.replacingOccurrences(of: #""delegated_invocation_id":"external""#,
            with: #""delegated_invocation_id":"external","attempt_id":"fake""#)
        #expect(throws: (any Error).self) {
            try JSONDecoder().decode(MacCanonicalConversationProjectionEvent.self, from: Data(ambiguous.utf8))
        }
    }

    @Test
    func applicationOrderedContextUpdateCanResetMeasurementRevision() throws {
        let initial = #"""
        {"execution_ownership":{"kind":"user"},"contract_version":2,"conversation_id":"context-order","revision":100,
         "title":"Context order","messages":[],"active_run":null,
         "context_usage":{"used_tokens":9000,"context_window_tokens":65536,
           "prompt_token_limit":57344,"estimated":false,"measured_at_revision":900}}
        """#
        let event: [String: Any] = [
            "identity": ["event_id": "usage-reset", "conversation_id": "context-order", "sequence": 41],
            "mutation": ["type": "set_context_usage", "context_usage": [
                "used_tokens": 0, "context_window_tokens": 32768,
                "prompt_token_limit": 24576, "estimated": true, "measured_at_revision": 2]]
        ]
        let updated = try applyKernelPages(initial: initial, sequence: 40, pages: [[event], [event]])
        #expect(updated.contextUsage.usedTokens == 0)
        #expect(updated.contextUsage.measuredAtRevision == 2)
        #expect(updated.contextUsage.windowTokens == 32768)
        #expect(updated.revision == 100)
    }

    @Test
    func terminalProjectionDoesNotInventAReplayCompletionTimestamp() throws {
        for state in ["completed", "failed", "cancelled"] {
            let data = Data("""
                {"message_id":"stored-answer","role":"assistant","state":"\(state)",
                 "revision":8,"created_at_ms":1000,"parts":[
                 {"kind":"reasoning","part_id":"unfinished-reason","revision":2,
                  "state":"running","title":"Reasoning","summary":"Recorded work"}]}
                """.utf8)
            let message = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Message.self, from: data)
            let first = try MacCanonicalConversationRepository.projectMessage(message)
            let second = try MacCanonicalConversationRepository.projectMessage(message)
            #expect(first == second, "Replaying immutable state must produce identical presentation values")
            let trace = try #require(first.presentation?.traceItems.first)
            #expect(trace.finishedAt == nil, "Read time is not the execution's finish time")
            #expect(first.presentation?.isComplete == true)
        }
    }

    @Test
    func reasoningPartsPreserveIndependentIdentityAndLifecycle() throws {
        let message = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.Message.self,
            from: Data(#"""
            {
              "message_id":"assistant-reasoning",
              "role":"assistant",
              "state":"running",
              "revision":8,
              "created_at_ms":1000,
              "parts":[
                {"kind":"reasoning","part_id":"reason-1","revision":2,"state":"completed","title":"Inspect request","summary":"Same text"},
                {"kind":"reasoning","part_id":"reason-2","revision":3,"state":"running","title":"Inspect request","summary":"Same text"},
                {"kind":"reasoning","part_id":"reason-3","revision":4,"state":"cancelled","title":"Inspect request","summary":"Cancelled step"},
                {"kind":"reasoning","part_id":"reason-4","revision":5,"state":"failed","title":"Inspect request","summary":"Failed step"}
              ]
            }
            """#.utf8)
        )
        let projected = try MacCanonicalConversationRepository.projectMessage(message)
        let presentation = try #require(projected.presentation)
        #expect(presentation.traceItems.map(\.externalID) == ["reason-1", "reason-2", "reason-3", "reason-4"])
        #expect(presentation.traceItems.map(\.state) == [.completed, .running, .cancelled, .failed])
        #expect(presentation.traceItems.prefix(2).map(\.detail) == ["Same text", "Same text"])
        let replayed = try MacCanonicalConversationRepository.projectMessage(message)
        #expect(replayed.presentation?.traceItems.map(\.id) == presentation.traceItems.map(\.id))
    }

    @Test
    func canonicalSurfaceOwnsMessageOrderAndAssistantPlaceholder() throws {
        let data = Data(
            #"""
            {
              "contract_version":1,
              "conversation_id":"surface-1",
              "revision":4,
              "title":"Surface",
              "messages":[
                {
                  "message_id":"assistant-1",
                  "role":"assistant",
                  "state":"pending",
                  "revision":1,
                  "created_at_ms":2000,
                  "parts":[]
                },
                {
                  "message_id":"user-1",
                  "role":"user",
                  "state":"completed",
                  "revision":1,
                  "created_at_ms":1000,
                  "parts":[{
                    "kind":"text",
                    "part_id":"user-text-1",
                    "revision":1,
                    "markdown":"Hello",
                    "is_final":true
                  }]
                }
              ],
              "active_run":null,
              "surface":{
                "revision":4,
                "committed_message_ids":["user-1"],
                "live_message_id":"assistant-1",
                "current_activity_part_id":null,
                "archived_activity_part_ids":[],
                "show_assistant_placeholder":true
              },
              "context_usage":{
                "used_tokens":12,
                "context_window_tokens":65536,
                "prompt_token_limit":57344,
                "estimated":true,
                "measured_at_revision":4
              }
            }
            """#.utf8
        )

        let projection = try JSONDecoder().decode(
            MacCanonicalConversationProjectionV1.self,
            from: data
        )
        let messages = try MacCanonicalConversationRepository.projectMessages(projection)

        #expect(messages.map(\.role) == [.user, .assistant])
        #expect(messages.first?.text == "Hello")
        #expect(messages.last?.text.isEmpty == true)
        #expect(messages.last?.presentation != nil)
    }

    @Test
    func projectionEventsAppendPreviewThenReplaceItWithCanonicalCompletedText() throws {
        let initial = #"""
        {"execution_ownership":{"kind":"user"},"contract_version":2,"conversation_id":"stream-1","revision":1,"title":"Stream",
         "messages":[{"message_id":"assistant-1","role":"assistant","state":"running",
           "revision":1,"created_at_ms":1000,"parts":[{"kind":"text","part_id":"text-1",
             "revision":1,"markdown":"Hel","is_final":false}]}],"active_run":null,
         "context_usage":{"used_tokens":1,"context_window_tokens":65536,
           "prompt_token_limit":57344,"estimated":true,"measured_at_revision":1}}
        """#
        func event(_ sequence: Int, mutation: [String: Any]) -> [String: Any] {
            ["identity": ["event_id": "event-\(sequence)", "conversation_id": "stream-1",
                "run_id": "run-1", "attempt_id": "attempt-1", "sequence": sequence],
             "mutation": mutation]
        }
        let preview = event(8, mutation: ["type": "append_text_preview", "message_id": "assistant-1",
            "part_id": "text-1", "revision": 2, "delta": "lo"])
        let completed = event(9, mutation: ["type": "complete_text_part", "message_id": "assistant-1",
            "part_id": "text-1", "revision": 3, "markdown": "Hello, canonical world."])
        let owners: [[String: Any]] = [["message_id": "assistant-1", "part_id": "text-1",
                       "run_id": "run-1", "execution": ["type": "model_attempt", "attempt_id": "attempt-1"]]]
        let projection = try applyKernelPages(initial: initial, sequence: 7, owners: owners,
                                              pages: [[preview, completed]])
        let paged = try applyKernelPages(initial: initial, sequence: 7, owners: owners,
                                         pages: [[preview], [completed], [completed]])
        #expect(paged == projection)
        guard case let .text(text) = projection.messages.first?.parts.first else {
            Issue.record("Expected canonical text Part")
            return
        }
        #expect(text.markdown == "Hello, canonical world.")
        #expect(text.revision == 3)
        #expect(text.isFinal)
        #expect(projection.messages.first?.state == .running)
        #expect(throws: (any Error).self) {
            try applyKernelPages(initial: initial, sequence: 7, owners: owners,
                                 pages: [[completed, preview]])
        }
    }

    private func applyKernelPages(
        initial: String, sequence: Int, owners: [[String: Any]] = [],
        pages: [[[String: Any]]]
    ) throws -> MacCanonicalConversationProjectionV1 {
        let projection = try JSONSerialization.jsonObject(with: Data(initial.utf8))
        let input = try JSONSerialization.data(withJSONObject: [
            "projection": projection, "event_sequence": sequence, "stream_owners": owners,
            "total_message_count": ((projection as? [String: Any])?["messages"] as? [Any])?.count ?? 0,
            "before_ordinal": NSNull(), "has_more": false, "assistant_journal_heads": [] as [String], "deferred_text_parts": [] as [String]
        ])
        let handle = try #require(input.withUnsafeBytes { bytes in
            nola_core_projection_new(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
        })
        defer { nola_core_projection_free(handle) }
        func consume(_ pointer: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
            let pointer = try #require(pointer)
            defer { nola_core_free_c_string(pointer) }
            let value = try #require(JSONSerialization.jsonObject(
                with: Data(String(cString: pointer).utf8)) as? [String: Any])
            guard value["ok"] as? Bool == true else {
                throw NSError(domain: "KernelProjectionTest", code: 1,
                              userInfo: [NSLocalizedDescriptionKey: String(describing: value)])
            }
            return try #require(value["result"] as? [String: Any])
        }
        for page in pages {
            let data = try JSONSerialization.data(withJSONObject: page)
            _ = try consume(data.withUnsafeBytes { bytes in
                nola_core_projection_apply(handle, bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count)
            })
        }
        let result = try consume(nola_core_projection_snapshot(handle))
        return try JSONDecoder().decode(MacCanonicalConversationProjectionV1.self,
            from: JSONSerialization.data(withJSONObject: try #require(result["projection"])))
    }

    @Test
    func decodesSharedApplicationProjectionAndMapsSkillRuntime() throws {
        let data = Data(
            #"""
            {
              "conversation_id":"session-1",
              "source":"core_checkpoint",
              "source_host_id":null,
              "store_revision":17,
              "core_revision":16,
              "event_cursor":41,
              "capability_catalog_revision":8,
              "checkpoint_created_at_ms":1786900100000,
              "conversation_projection":{"revision":1,"order":[],"parts":{}},
              "task_projection":{"revision":2,"order":[],"tasks":{},"guidance":{}},
              "goal_projection":{"applied_operations":{},"revision":3,"active":null},
              "memory_projection":{"revision":4,"order":[],"records":{}},
              "memory_runtime_projection":{"revision":5,"jobs":{},"job_order":[],"usage_ledger":[],"usage_stats":{}},
              "skill_runtime_projection":{"session_id":"session-1","cursor":41,"revision":6,"catalog_revision":8,"order":[],"runs":{}},
              "permission_projection":{"revision":7,"pending":[]},
              "permission_rule_set":{},
              "context_usage":{"used_tokens":12345,"context_window_tokens":65536,"prompt_token_limit":57344,"usage_percent":18.83697509765625,"usage_is_estimated":false,"cached_input_tokens":0,"cached_input_tokens_reported":false,"cache_write_input_tokens":0,"cache_write_input_tokens_reported":false}
            }
            """#.utf8
        )

        let projection = try JSONDecoder().decode(MacCanonicalSessionProjection.self, from: data)
        #expect(projection.conversationID == "session-1")
        #expect(projection.source == "core_checkpoint")
        #expect(projection.storeRevision == 17)
        #expect(projection.contextUsage.usedTokens == 12_345)
        #expect(projection.contextUsage.contextWindowTokens == 65_536)
        #expect(projection.contextUsage.usageIsEstimated == false)
        let skill = try JSONDecoder().decode(
            MacCanonicalSkillRuntimeProjection.self,
            from: projection.skillRuntimeProjection
        )
        #expect(skill.sessionID == "session-1")
        #expect(skill.presentationProjection.revision == 6)
        #expect(skill.presentationProjection.lastCursor == 41)
        #expect(skill.presentationProjection.catalogRevision == 8)
    }

    @Test
    func temporaryApplicationPortReleasesItsOwnedService() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-service-lifetime-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let databaseURL = directory.appendingPathComponent("nola.db")
        let socketURL = MacApplicationServiceClient.serviceSocketURL(for: databaseURL)

        var port: MacApplicationPort? = try MacApplicationPort(databaseURL: databaseURL)
        #expect(MacApplicationServiceSupervisor.shared.ownsProcessForTesting(socketURL: socketURL))
        withExtendedLifetime(port) {}
        port = nil
        #expect(!MacApplicationServiceSupervisor.shared.ownsProcessForTesting(socketURL: socketURL))
    }
}
