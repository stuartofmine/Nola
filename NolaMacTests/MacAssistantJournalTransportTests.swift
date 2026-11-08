// SPDX-License-Identifier: Apache-2.0
import XCTest
import Observation
import SwiftUI
import AppKit
@testable import Nola

final class MacAssistantJournalTransportTests: XCTestCase {
    func testThinkingFlowSurvivesToolsUntilPublicProseArrives() {
        func part(_ id: String, _ metadata: MacAssistantJournalMetadata, bytes: String = "0") -> MacAssistantJournalPart {
            .init(identity: .init(source_store_epoch: "epoch", conversation_id: "c", run_id: "r", assistant_message_id: "a"),
                part_id: id, generation: "1", revision: "1", metadata_revision: "1",
                order_key: .init(section: "0", position: "0"), metadata: metadata,
                bodies: [.init(field: "markdown", content_generation: "1", utf8_bytes: bytes, chunk_count: "1", content_root: "root")])
        }
        let settled = part("first", .reasoning(.init(title: .inline("Thinking"), state: "completed")))
        let next = part("next", .reasoning(.init(title: .inline("Thinking"), state: "running")))
        let placeholder = part("answer", .text(.init(is_final: false, visibility: "public", review: nil)))
        let prose = part("answer", .text(.init(is_final: false, visibility: "public", review: nil)), bytes: "12")
        XCTAssertEqual(MacJournalPartFlow.liveReasoning([settled, placeholder], delivery: .running).map(\.part_id), [])
        XCTAssertEqual(MacJournalPartFlow.liveReasoning([settled, placeholder, next], delivery: .running).map(\.part_id), ["next"])
        XCTAssertEqual(MacJournalPartFlow.liveReasoning([settled, prose, next], delivery: .running).map(\.part_id), ["next"])
        let identity = AppleUISurfaceIdentity(owner: .init(authority_id: "store", conversation_id: "c", participant_id: "agent",
            context_id: "ctx", originating_run_id: "r", message_id: "a", part_id: "tool"), surface_id: "form", generation: 1)
        let surface = part("surface", .uiSurface(.init(surface: .init(identity: identity, revision: 1), summary: "Form")))
        let tool = part("tool", .tool(.init(execution_source: .model, tool_call_id: "call", capability_id: "render_ui",
            title: .inline("Render"), summary: nil, origin: .init(kind: "local", provider_id: nil, provider_title: nil, namespace: nil),
            permission: nil, state: "completed", display_key: nil, affected_resources: .inline([]), artifacts: "0",
            full_output: nil, started_at_ms: nil, finished_at_ms: nil)))
        let mixed = MacJournalPartFlow.segments([prose, settled, tool, surface, next])
        XCTAssertEqual(mixed.map { $0.parts.map(\.part_id) }, [["answer"], ["first", "tool"], ["surface"], ["next"]])
        XCTAssertEqual(MacJournalPartFlow.liveReasoning([next, surface, prose], delivery: .running).map(\.part_id), ["next"])
        // Earlier commentary is not the current answer.
        XCTAssertFalse(MacJournalPartFlow.liveReasoning([prose, settled, placeholder, next], delivery: .running).isEmpty)
    }

    func testReferencesKeepTheirCanonicalPositionsAsHardFlowBoundaries() {
        func part(_ id: String, _ metadata: MacAssistantJournalMetadata) -> MacAssistantJournalPart {
            .init(identity: .init(source_store_epoch: "epoch", conversation_id: "c", run_id: "r", assistant_message_id: "a"),
                part_id: id, generation: "1", revision: "1", metadata_revision: "1",
                order_key: .init(section: "0", position: "0"), metadata: metadata, bodies: [])
        }
        let parts = (0..<11).flatMap { i in
            [part("source-\(i)", .source(.init(source_id: "s\(i)", title: .inline("Source"), uri: .inline("https://example.com/\(i)"), mime_type: nil, evidence: nil))),
             part("citation-\(i)", .citation(.init(citation_id: "c\(i)", source_id: "s\(i)", title: .inline("Source"), uri: .inline("https://example.com/\(i)"), mime_type: nil, origin_tool_call_id: "call", location: nil, byte_range: nil)))]
        }
        let flow = MacJournalPartFlow.segments(parts)
        XCTAssertEqual(flow.count, 22)
        XCTAssertTrue(flow.allSatisfy { $0.kind == .content && $0.parts.count == 1 })
        XCTAssertEqual(flow.flatMap(\.parts).map(\.part_id), parts.map(\.part_id))
    }





    func testSkillChromeDependsOnlyOnVisibleMetadata() {
        func skill(state: String = "running", calls: String = "2") -> MacAssistantJournalMetadata.Skill {
            .init(run_id: "run", skill_id: "research", display_name: .inline("Research"),
                  mode: "inline", state: state, model_turns: "1", tool_calls: calls, elapsed_millis: "100")
        }
        let initial = MacJournalSkillHeaderValue(skill: skill())
        // No journal cursor, body content or unrelated context estimate enters this value.
        XCTAssertEqual(initial, MacJournalSkillHeaderValue(skill: skill()))
        XCTAssertNotEqual(initial, MacJournalSkillHeaderValue(skill: skill(state: "completed")))
        XCTAssertNotEqual(initial, MacJournalSkillHeaderValue(skill: skill(calls: "3")))
    }

    func testSealedDeliveryDoesNotImplySuccessfulOutcome() {
        XCTAssertEqual(MacJournalRunStatus.attentionState("incomplete"), "incomplete")
        XCTAssertEqual(MacJournalRunStatus.attentionState("blocked"), "blocked")
        XCTAssertNil(MacJournalRunStatus.attentionState("action_completed"))
        XCTAssertEqual(MacJournalProcessStatus.primary(delivery: .running, attention: nil), "running")
    }

    func testProcessDisclosureUsesRunDeliveryAndPreservesExplicitChoice() {
        XCTAssertEqual(MacJournalProcessStatus.primary(delivery: .completed, attention: "failed"), "completed")
        XCTAssertEqual(MacJournalProcessStatus.primary(delivery: .running, attention: "failed"), "running")
        XCTAssertEqual(MacJournalProcessStatus.primary(delivery: .running, attention: "waiting_for_permission"), "waiting_for_permission")
        XCTAssertEqual(MacJournalProcessStatus.primary(delivery: .cancelled, attention: "waiting_for_permission"), "cancelled")
        XCTAssertFalse(AppleMessagePartFlow.isExpanded(choices: [Bool](), defaultExpanded: false))
        XCTAssertTrue(AppleMessagePartFlow.isExpanded(choices: [Bool](), defaultExpanded: true))
        XCTAssertTrue(AppleMessagePartFlow.isExpanded(choices: [true], defaultExpanded: false))
        XCTAssertFalse(AppleMessagePartFlow.isExpanded(choices: [true, false], defaultExpanded: true))

    }



    func testFixtureDeclaresOnlyItsActualModelTransportAndToolSurface() throws {
        for tool in [nil, "fixture_read"] as [String?] {
            let profile = MacAssistantJournalTestFixture.modelCapabilityProfile(
                toolCapabilityID: tool, contextWindow: 32_768, maxOutput: 4_096)
            XCTAssertEqual(profile["profile_id"] as? String, "mac-journal-fixture")
            XCTAssertEqual(profile["revision"] as? Int, 1)
            XCTAssertEqual(profile["native_tool_calls"] as? Bool, tool != nil)
            XCTAssertEqual(profile["stream_tool_calls"] as? Bool, tool != nil)
            XCTAssertEqual(profile["direct_tool_capability_ids"] as? [String], tool.map { [$0] } ?? [])
            XCTAssertEqual(profile["parallel_tool_calls"] as? Bool, false)
            XCTAssertEqual(profile["vision_input"] as? Bool, false)
            XCTAssertEqual(profile["context_window_tokens"] as? Int, 32_768)
            XCTAssertEqual(profile["max_output_tokens"] as? Int, 4_096)
        }
        let fixture = try MacAssistantJournalTestFixture(conversationID: "profile-contract",
            runID: "profile-run", assistantMessageID: "profile-assistant")
        XCTAssertEqual(try fixture.admittedHead().sequence, "0",
            "real Application admission accepts the declared fixture transport without publishing an answer")
        XCTAssertTrue(try fixture.heads().isEmpty)
    }

    @MainActor
    func testUnchangedJournalCoverageDoesNotInvalidateMessageContainer() async throws {
        let identity = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        func reference(_ sequence: String) -> MacJournalMessageReference {
            .init(databaseURL: URL(fileURLWithPath: "/fixture/coverage.db"),
                cursor: .init(identity: identity, sequence: sequence, record_digest: sequence))
        }
        let owner = MacJournalPresentationOwner { _, _, _ in [] }
        defer { owner.cancel() }
        owner.request(reference("1"))
        for _ in 0..<100 where owner.loadedReference != reference("1") {
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(owner.loadedReference, reference("1"))
        let redundantChange = expectation(description: "unchanged handover must not publish")
        redundantChange.isInverted = true
        withObservationTracking {
            _ = owner.activityCoverage
            _ = owner.referenceCoverage
        } onChange: { redundantChange.fulfill() }
        owner.request(reference("2"))
        for _ in 0..<100 where owner.loadedReference != reference("2") {
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertEqual(owner.loadedReference, reference("2"), "The journal cursor must still advance")
        await fulfillment(of: [redundantChange], timeout: 0.05)
    }

    @MainActor
    func testEmptyPartDoesNotConsumePreviewReadBudgetOrJoinAnotherPart() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "empty-continuation-preview",
            runID: "run", assistantMessageID: "assistant")
        _ = try fixture.append("PREFIX-")
        _ = try fixture.continueAfterOutputLimit()
        _ = try fixture.append("END")
        _ = try fixture.finish()
        let reference = MacJournalMessageReference(databaseURL: fixture.databaseURL,
            cursor: try XCTUnwrap(fixture.heads().last))
        let parts = try await MacJournalTextRead.parts(reference).filter {
            if case .text = $0.metadata { return true }; return false
        }
        var segments = try parts.map { part in
            MacJournalBodyState.Segment(partID: part.part_id,
                content: try XCTUnwrap(part.bodies.first { $0.field == "markdown" }))
        }
        // Model the empty metadata field observed after real-provider retry.
        // The two nonempty fields still use actual Application range reads;
        // the fixture-only empty fields must never cause a body query.
        let empty = MacJournalBodyState.Segment(partID: "empty-metadata-fixture",
            content: .init(field: "markdown", content_generation: "1", utf8_bytes: "0",
                chunk_count: "0", content_root: String(repeating: "0", count: 64)))
        segments.insert(empty, at: 1)
        segments.append(empty)
        let first = try XCTUnwrap(segments.first)
        let state = MacJournalBodyState()
        defer { state.cancel() }
        state.request(.init(reference: reference, partID: first.partID, content: first.content,
            isFinal: true, presentation: .markdown))
        for _ in 0..<200 where state.text.isEmpty { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertNil(state.error)
        XCTAssertEqual(state.text, "PREFIX-", "A visual reader remains bound to its own Part")
        XCTAssertTrue(state.loadState.isComplete)
        XCTAssertFalse(state.loadState.showsFullReader)
        let emptyState = MacJournalBodyState()
        defer { emptyState.cancel() }
        emptyState.request(.init(reference: reference, partID: empty.partID, content: empty.content,
            isFinal: true, presentation: .markdown))
        for _ in 0..<200 where emptyState.loadState == .loading { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertNil(emptyState.error)
        XCTAssertEqual(emptyState.text, "")
        XCTAssertTrue(emptyState.loadState.isComplete)
    }

    @MainActor
    func testSeparateTextPartsRetainIndividualReadBudgetsAndCopyAggregatesOnlyOnExplicitRequest() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "bounded-segment-preview",
            runID: "run", assistantMessageID: "assistant")
        _ = try fixture.append("FIRST-")
        _ = try fixture.continueAfterOutputLimit()
        let suffix = String(repeating: "S", count: 100_000)
        _ = try fixture.append(suffix)
        let reference = MacJournalMessageReference(databaseURL: fixture.databaseURL,
            cursor: try XCTUnwrap(fixture.heads().last))
        let parts = try await MacJournalTextRead.parts(reference).filter {
            if case .text = $0.metadata { return true }; return false
        }
        XCTAssertEqual(parts.count, 2)
        let first = try XCTUnwrap(parts.first)
        let flow = MacJournalPartFlow.segments(parts)
        XCTAssertEqual(flow.map { $0.parts.map(\.part_id) }, parts.map { [$0.part_id] })
        let segments = try parts.map { part in
            MacJournalBodyState.Segment(partID: part.part_id,
                content: try XCTUnwrap(part.bodies.first { $0.field == "markdown" }))
        }
        let state = MacJournalBodyState()
        defer { state.cancel() }
        state.request(.init(reference: reference, partID: first.part_id,
            content: segments[0].content, isFinal: false, presentation: .markdown))
        for _ in 0..<200 where state.text.isEmpty { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertNil(state.error)
        XCTAssertEqual(state.text, "FIRST-", "The first visual Part never consumes a later Part's prose")
        XCTAssertFalse(state.loadState.showsFullReader)
        let secondState = MacJournalBodyState()
        defer { secondState.cancel() }
        secondState.request(.init(reference: reference, partID: segments[1].partID,
            content: segments[1].content, isFinal: false, presentation: .markdown))
        for _ in 0..<200 where secondState.text.isEmpty { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertNil(secondState.error)
        XCTAssertEqual(secondState.text, String(suffix.prefix(65_536)), "Each Part retains its bounded two-range preview budget")
        XCTAssertTrue(secondState.loadState.showsFullReader, "The later Part retains its own full reader")
        let canonical = try await MacJournalTextRead.markdown(reference, historicalFallback: "")
        XCTAssertEqual(canonical, "FIRST-" + suffix)

        let boundary = MacAssistantJournalPart(identity: first.identity, part_id: "tool-boundary",
            generation: "1", revision: "1", metadata_revision: "1", order_key: first.order_key,
            metadata: .tool(.init(execution_source: .model, tool_call_id: "tool", capability_id: "bash", title: .inline("Run"), summary: nil,
                origin: .init(kind: "builtin", provider_id: nil, provider_title: nil, namespace: nil),
                permission: nil, state: "completed", display_key: nil, affected_resources: .inline([]),
                artifacts: "tool_artifacts", full_output: nil, started_at_ms: nil, finished_at_ms: nil)), bodies: [])
        let separated = MacJournalPartFlow.segments([parts[0], boundary, parts[1]])
        XCTAssertEqual(separated.map { $0.parts.map(\.part_id) }, [[parts[0].part_id], [boundary.part_id], [parts[1].part_id]])
    }

    func testCompletedToolGroupingPreservesOrderAndNeverHidesNonSuccessStates() {
        func part(_ id: String, _ state: String?) -> MacAssistantJournalPart {
            let metadata: MacAssistantJournalMetadata = if let state {
                .tool(.init(execution_source: .model, tool_call_id: id, capability_id: "bash", title: .inline("Run"), summary: nil,
                    origin: .init(kind: "builtin", provider_id: nil, provider_title: nil, namespace: nil),
                    permission: nil, state: state, display_key: nil, affected_resources: .inline([]),
                    artifacts: "tool_artifacts", full_output: nil, started_at_ms: nil, finished_at_ms: nil))
            } else { .text(.init(is_final: true, visibility: "public", review: nil)) }
            return .init(identity: .init(source_store_epoch: "test", conversation_id: "conversation",
                run_id: "run", assistant_message_id: "assistant"), part_id: id, generation: "1",
                revision: "1", metadata_revision: "1", order_key: .init(section: "parts", position: id),
                metadata: metadata, bodies: [])
        }
        for boundary in [nil, "failed", "cancelled", "pending", "running", "waiting_for_permission", "unknown"] as [String?] {
            let parts = [part("a", "completed"), part("b", "completed"), part("c", "completed"),
                         part("boundary", boundary), part("d", "completed"), part("e", "completed")]
            let grouping = MacJournalCompletedToolGrouping(parts)
            let process = MacJournalProcessProjection(parts)
            XCTAssertEqual(process.activities.filter { if case .tool = $0.metadata { return true }; return false }.map(\.part_id),
                parts.filter { if case .tool = $0.metadata { return true }; return false }.map(\.part_id))
            XCTAssertEqual(process.requiresAttention,
                ["failed", "pending", "waiting_for_permission", "waiting_for_input"].contains(boundary ?? ""))
            XCTAssertEqual(grouping.groups["a"]?.map(\.part_id), ["a", "b", "c"])
            XCTAssertEqual(grouping.hiddenIDs, ["b", "c"])
            let flattened = parts.flatMap { part -> [String] in
                if let group = grouping.groups[part.part_id] { return group.map(\.part_id) }
                return grouping.hiddenIDs.contains(part.part_id) ? [] : [part.part_id]
            }
            XCTAssertEqual(flattened, parts.map(\.part_id))
        }
    }

    func testModelAttemptUsesSharedModelNameAndLocalizedRestartTitle() throws {
        let appBundle = Bundle(for: MacJournalPresentationOwner.self)
        let chinese = try XCTUnwrap(Bundle(path: XCTUnwrap(appBundle.path(forResource: "zh-Hans", ofType: "lproj"))))
        let attempt = AppleModelAttemptPresentation(state: "failed", provider: "device_local", model: " qwen ",
            successorAttemptID: "actual-successor", successorProvider: "device_local", successorModel: "qwen")
        XCTAssertEqual(attempt.modelName, "qwen")
        XCTAssertEqual(attempt.kind, .restarted)
        XCTAssertEqual(attempt.title(bundle: chinese), "请求已重新开始")
        let switched = AppleModelAttemptPresentation(state: "failed", provider: "a", model: "qwen",
            successorAttemptID: "actual-successor", successorProvider: "b", successorModel: "qwen")
        XCTAssertEqual(switched.title(bundle: chinese), "模型调用路线已变更")
    }

    func testModelFailurePresentationUsesCommittedCodesAndPreservesUnknownDiagnostics() throws {
        let appBundle = Bundle(for: MacJournalPresentationOwner.self)
        let simplified = try XCTUnwrap(appBundle.path(forResource: "zh-Hans", ofType: "lproj"))
        let chinese = try XCTUnwrap(Bundle(path: simplified))
        let codes = ["responses_incomplete", "responses_timeout", "responses_transport_error",
                     "responses_tool_transaction_incomplete", "responses_provider_failed",
                     "responses_protocol_error", "provider_failed", "content_filtered", "authentication_failed",
                     "quota_exhausted", "rate_limited", "model_unavailable", "context_length_exceeded"]
        for code in codes {
            let message = try XCTUnwrap(MacJournalFailurePresentation.message(for: code, bundle: chinese))
            XCTAssertTrue(message.contains("模型") || message.contains("会话"), "Missing translation: \(code)")
            XCTAssertFalse(message.contains(code))
        }
        XCTAssertNil(MacJournalFailurePresentation.message(for: "unknown_failure"))
        XCTAssertEqual(MacJournalFailurePresentation.message(for: "model_resampled", bundle: chinese),
                       "已重新发起模型请求。")
        XCTAssertEqual(MacJournalFailurePresentation.message(for: "invalid_compaction_summary", bundle: chinese),
                       "模型返回的会话摘要不可用。")
        XCTAssertEqual(chinese.localizedString(forKey: "Waiting for permission", value: nil, table: nil), "等待授权")
        XCTAssertNil(MacJournalFailurePresentation.message(for: "provider says responses_timeout"),
                     "Provider text must not be interpreted as a committed failure code")
    }

    @MainActor
    func testBodyReadFailureIsScopedToSnapshotAndNewCursorRecoversWithoutRetry() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "snapshot-recovery", runID: "run", assistantMessageID: "assistant")
        let first = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: try fixture.append("Before"))
        let second = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: try fixture.append(" after"))
        let parts = try await MacJournalTextRead.parts(second)
        let part = try XCTUnwrap(parts.first { if case .text = $0.metadata { return true }; return false })
        let content = try XCTUnwrap(part.bodies.first { $0.field == "markdown" })
        let stale = MacJournalBodyState.Request(reference: first, partID: part.part_id,
            content: content, isFinal: false, presentation: .markdown)
        let current = MacJournalBodyState.Request(reference: second, partID: part.part_id,
            content: content, isFinal: false, presentation: .markdown)
        XCTAssertNotEqual(stale, current, "A cursor repair must trigger the view's onChange even if the body reference is unchanged")
        let state = MacJournalBodyState()
        defer { state.cancel() }
        state.request(stale)
        for _ in 0..<200 where state.error == nil { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertTrue(try XCTUnwrap(state.error).contains("content reference not reachable"),
            "The store must still reject content from a different snapshot")
        state.request(current)
        for _ in 0..<200 where state.text != "Before after" { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(state.text, "Before after")
        XCTAssertNil(state.error)
        XCTAssertTrue(state.loadState.isComplete)
    }

    @MainActor
    func testSupersededBodyFailureDoesNotBlockPendingCurrentSnapshot() async throws {
        let fixture = try MacAssistantJournalTestFixture(conversationID: "pending-snapshot", runID: "run", assistantMessageID: "assistant")
        let first = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: try fixture.append("Partial"))
        let second = MacJournalMessageReference(databaseURL: fixture.databaseURL, cursor: try fixture.append(" complete"))
        let parts = try await MacJournalTextRead.parts(second)
        let part = try XCTUnwrap(parts.first { if case .text = $0.metadata { return true }; return false })
        let content = try XCTUnwrap(part.bodies.first { $0.field == "markdown" })
        let state = MacJournalBodyState()
        defer { state.cancel() }
        state.request(.init(reference: first, partID: part.part_id, content: content, isFinal: false, presentation: .markdown))
        await Task.yield()
        state.request(.init(reference: second, partID: part.part_id, content: content, isFinal: false, presentation: .markdown))
        for _ in 0..<200 where state.text != "Partial complete" { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(state.text, "Partial complete")
        XCTAssertNil(state.error)
    }

    @MainActor
    func testCompletedToolDisclosurePreservesExplicitChoicesWhenGroupsGrow() {
        let owner = MacJournalPresentationOwner { _, _, _ in [] }
        let parts = ["tool-1", "tool-2", "tool-3"]
        XCTAssertFalse(owner.completedToolsExpanded(partIDs: parts), "The compact default must not change")
        var expansion = MacJournalActivityExpansion()
        expansion.setExpanded(true)
        owner.expansions["tool-2"] = expansion
        XCTAssertTrue(owner.completedToolsExpanded(partIDs: parts), "Grouping must not hide a tool the reader opened")
        owner.completedToolExpansions["tool-1"] = false
        XCTAssertFalse(owner.completedToolsExpanded(partIDs: parts + ["tool-4"]), "A later tool cannot undo explicit group collapse")
        owner.completedToolExpansions["tool-1"] = true
        XCTAssertTrue(owner.completedToolsExpanded(partIDs: parts + ["tool-4"]), "A growing group keeps the existing disclosure identity")

        let technicalOwner = MacJournalPresentationOwner { _, _, _ in [] }
        technicalOwner.toolTechnicalExpansions["tool-2"] = true
        XCTAssertTrue(technicalOwner.completedToolsExpanded(partIDs: parts))
        expansion.setExpanded(false)
        technicalOwner.expansions["tool-2"] = expansion
        XCTAssertFalse(technicalOwner.completedToolsExpanded(partIDs: parts), "Explicitly closing a tool takes precedence over its retained technical detail choice")
    }

    @MainActor
    func testToolDisclosureStateAdvancesWithCursorButDoesNotCrossJournalOwners() {
        func reference(_ run: String, _ sequence: String) -> MacJournalMessageReference {
            .init(databaseURL: URL(fileURLWithPath: "/fixture/disclosure.db"), cursor: .init(
                identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
                    run_id: run, assistant_message_id: "assistant"), sequence: sequence, record_digest: "fixture"))
        }
        let owner = MacJournalPresentationOwner { _, _, _ in [] }
        owner.request(reference("first", "1"))
        owner.toolTechnicalExpansions["tool"] = true
        owner.completedToolExpansions["tool"] = true
        owner.request(reference("first", "2"))
        XCTAssertEqual(owner.toolTechnicalExpansions["tool"], true)
        XCTAssertEqual(owner.completedToolExpansions["tool"], true)
        owner.request(reference("second", "1"))
        XCTAssertTrue(owner.toolTechnicalExpansions.isEmpty)
        XCTAssertTrue(owner.completedToolExpansions.isEmpty)
        XCTAssertFalse(owner.completedToolsExpanded(partIDs: ["tool"]))
        owner.cancel()
    }

    @MainActor
    func testPresentationResidencyNeverEvictsMountedStateWhenCapacityIsExceeded() throws {
        let pool = MacJournalPresentationResidency(capacity: 2)
        func reference(_ index: Int) -> MacJournalMessageReference {
            .init(databaseURL: URL(fileURLWithPath: "/fixture/residency.db"), cursor: .init(
                identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
                    run_id: "run-\(index)", assistant_message_id: "assistant-\(index)"),
                sequence: "1", record_digest: "fixture"))
        }
        let mounted = (0..<24).map { pool.owner(for: reference($0)) }
        mounted[0].toolTechnicalExpansions["tool"] = true
        mounted[0].completedToolExpansions["tool"] = true
        XCTAssertEqual(pool.count, 2, "Only offscreen strong residency is capacity-bounded")
        for index in 0..<24 {
            XCTAssertTrue(pool.owner(for: reference(index)) === mounted[index], "Mounted rows must not be replaced by empty owners")
        }
        XCTAssertEqual(pool.owner(for: reference(0)).toolTechnicalExpansions["tool"], true)
        XCTAssertEqual(pool.owner(for: reference(0)).completedToolExpansions["tool"], true)
        let fields = (0..<24).map { mounted[0].body(partID: "part-\($0)", field: "markdown") }
        let revealedNode = UUID()
        fields[0].revealedNodeIDs = [revealedNode]
        for index in 0..<24 { XCTAssertTrue(mounted[0].body(partID: "part-\(index)", field: "markdown") === fields[index]) }
        XCTAssertEqual(mounted[0].body(partID: "part-0", field: "markdown").revealedNodeIDs, [revealedNode])
        pool.retain([reference(23).ownerKey])
        XCTAssertEqual(pool.count, 1)
        pool.removeAll()
        XCTAssertEqual(pool.count, 0)
    }

    @MainActor
    func testPresentationOwnerCoalescesNotificationBurstWithoutCancellingFirstRead() async throws {
        let identity = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        func reference(_ sequence: Int) -> MacJournalMessageReference {
            .init(databaseURL: URL(fileURLWithPath: "/fixture/serial.db"),
                cursor: .init(identity: identity, sequence: String(sequence), record_digest: "fixture-\(sequence)"))
        }
        var reads: [String] = []
        var completions: [String] = []
        let owner = MacJournalPresentationOwner { reference, _, _ in
            reads.append(reference.cursor.sequence)
            try await Task.sleep(for: .milliseconds(20))
            completions.append(reference.cursor.sequence)
            return []
        }
        owner.request(reference(1))
        for _ in 0..<100 where reads.isEmpty { try await Task.sleep(for: .milliseconds(1)) }
        XCTAssertEqual(reads, ["1"])
        for sequence in 2...50 { owner.request(reference(sequence)) }
        for _ in 0..<200 where owner.loadedReference != reference(50) { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(owner.loadedReference, reference(50))
        XCTAssertEqual(reads, ["1", "50"])
        XCTAssertEqual(completions, reads, "Notification bursts must not cancel the useful first frame")
        owner.request(reference(3))
        XCTAssertEqual(owner.loadedReference, reference(50), "Stale notifications cannot rewind the visible snapshot")
        owner.cancel()
    }

    @MainActor
    func testPresentationOwnerRejectsInFlightForeignOwnerAfterReuse() async throws {
        func reference(_ run: String) -> MacJournalMessageReference {
            .init(databaseURL: URL(fileURLWithPath: "/fixture/foreign.db"), cursor: .init(
                identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
                    run_id: run, assistant_message_id: run), sequence: "1", record_digest: run))
        }
        let owner = MacJournalPresentationOwner { _, _, _ in
            // A noncooperative read must still be fenced by the owner's generation.
            try? await Task.sleep(for: .milliseconds(20))
            return []
        }
        owner.request(reference("old"))
        await Task.yield()
        owner.request(reference("new"))
        for _ in 0..<200 where owner.loadedReference != reference("new") { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(owner.loadedReference, reference("new"))
        XCTAssertNil(owner.projection(matching: reference("old")),
            "A reused row must reject old conversation content even before its onChange executes")
        XCTAssertNotNil(owner.projection(matching: reference("new")))
        owner.cancel()
    }

    @MainActor
    func testCompletedJournalOnlyMessageRemainsVisibleWithoutLegacyText() {
        let reference = MacJournalMessageReference(databaseURL: URL(fileURLWithPath: "/fixture/visible.db"), cursor: .init(
            identity: .init(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant"),
            sequence: "1", record_digest: "fixture"))
        var message = MacChatMessage(role: .assistant, text: "", createdAt: .now)
        message.journalReference = reference
        message.deliveryState = .completed
        XCTAssertTrue(macChatMessageIsVisible(message, isActive: false))
        XCTAssertEqual(MacTranscriptWindowPolicy.project([message], isRunning: false, visibleMessageLimit: 20).messages.map(\.id), [message.id])
    }

    @MainActor
    func testReusedJournalRowCannotPaintPreviousConversationBeforeRequestChanges() async throws {
        func reference(_ conversation: String) -> MacJournalMessageReference {
            .init(databaseURL: URL(fileURLWithPath: "/fixture/switch.db"), cursor: .init(
                identity: .init(source_store_epoch: "epoch", conversation_id: conversation,
                    run_id: "same-run-id", assistant_message_id: "same-message-id"),
                sequence: "1", record_digest: conversation))
        }
        let owner = MacJournalPresentationOwner { _, _, _ in [] }
        owner.request(reference("first"))
        for _ in 0..<100 where owner.loadedReference == nil { await Task.yield() }
        XCTAssertNotNil(owner.projection(matching: reference("first")))
        // Body can run with its new input before the onChange handler calls
        // request(second). Even identical Run/message IDs must not share text.
        XCTAssertNil(owner.projection(matching: reference("second")))
        owner.cancel()
    }

    @MainActor
    func testBodyHydrationRemainsBoundedAndSwitchingOwnerCannotPublishOldText() async throws {
        let first = try MacAssistantJournalTestFixture(conversationID: "body-first", runID: "first", assistantMessageID: "assistant")
        let firstReference = MacJournalMessageReference(databaseURL: first.databaseURL, cursor: try first.append(String(repeating: "A", count: 100_000)))
        let firstParts = try await MacJournalTextRead.parts(firstReference)
        let firstPart = try XCTUnwrap(firstParts.first {
            if case .text = $0.metadata { return true }; return false
        })
        let firstContent = try XCTUnwrap(firstPart.bodies.first { $0.field == "markdown" })
        let state = MacJournalBodyState()
        state.request(.init(reference: firstReference, partID: firstPart.part_id,
            content: firstContent, isFinal: false, presentation: .markdown))
        for _ in 0..<200 where state.text.isEmpty { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(state.text.utf8.count, 65_536)
        XCTAssertTrue(state.loadState.showsFullReader)
        let second = try MacAssistantJournalTestFixture(conversationID: "body-second", runID: "second", assistantMessageID: "assistant")
        let secondReference = MacJournalMessageReference(databaseURL: second.databaseURL, cursor: try second.append("Only the new owner"))
        let secondParts = try await MacJournalTextRead.parts(secondReference)
        let secondPart = try XCTUnwrap(secondParts.first {
            if case .text = $0.metadata { return true }; return false
        })
        let secondContent = try XCTUnwrap(secondPart.bodies.first { $0.field == "markdown" })
        state.request(.init(reference: firstReference, partID: firstPart.part_id,
            content: firstContent, isFinal: true, presentation: .markdown))
        await Task.yield()
        state.request(.init(reference: secondReference, partID: secondPart.part_id,
            content: secondContent, isFinal: false, presentation: .markdown))
        XCTAssertTrue(state.text.isEmpty, "A foreign owner must not display the old owner's cached text")
        for _ in 0..<200 where state.text != "Only the new owner" { try await Task.sleep(for: .milliseconds(5)) }
        XCTAssertEqual(state.text, "Only the new owner")
        XCTAssertTrue(state.loadState.isComplete)
        state.cancel()
    }

    func testJournalDocumentWaitsForTableDividerWithoutLosingSourceStructure() {
        var projection = MacJournalMarkdownProjection(identitySeed: "journal:table")
        let prefix = "Intro.\n\n"
        let header = "| Model | State |\n"
        projection.update(prefix + header, isFinal: false)
        XCTAssertEqual(projection.renderedSource, prefix)
        projection.update(prefix + header + "| --- | --", isFinal: false)
        XCTAssertEqual(projection.renderedSource, prefix)
        let source = prefix + header + "| --- | --- |\n| Local | Ready |\n"
        projection.update(source, isFinal: false)
        XCTAssertEqual(projection.renderedSource, source)
        projection.update(source, isFinal: true)
        XCTAssertEqual(projection.renderedSource, source)

        var prose = MacJournalMarkdownProjection(identitySeed: "journal:not-table")
        prose.update(header, isFinal: false)
        XCTAssertEqual(prose.renderedSource, "")
        prose.update(header, isFinal: true)
        XCTAssertEqual(prose.renderedSource, header, "Completion must not discard ordinary pipe text")
    }

    func testJournalMarkdownRevealRetainsNodesThroughAppendAndCompletion() {
        var projection = MacJournalMarkdownProjection(identitySeed: "journal:answer")
        projection.update("水面上的文字逐渐清晰。", isFinal: false)
        let first = projection.presentation.renderedTextNodes
        XCTAssertFalse(first.isEmpty)
        XCTAssertTrue(projection.hasStreamed)
        projection.update("水面上的文字逐渐清晰。接着出现第二句话。", isFinal: false)
        XCTAssertEqual(projection.presentation.renderedTextNodes.first?.id, first.first?.id)
        let beforeCompletion = projection.presentation.renderedTextNodes
        projection.update(projection.text, isFinal: true)
        XCTAssertEqual(projection.presentation.renderedTextNodes.map(\.id), beforeCompletion.map(\.id))
        XCTAssertEqual(projection.presentation.renderedTextNodes.map(\.text).joined(), projection.text)
        XCTAssertTrue(projection.presentation.isComplete)
        XCTAssertTrue(projection.hasStreamed, "Completion must retain live reveal eligibility for late body hydration")
        projection.update(projection.text, isFinal: true)
        XCTAssertEqual(projection.presentation.renderedTextNodes.map(\.id), beforeCompletion.map(\.id),
            "Metadata-only terminal publications must not duplicate or replace the visible nodes")
    }

    func testJournalMarkdownHistoryLoadsWithoutRevealAndReplacementsDoNotDuplicateText() {
        var projection = MacJournalMarkdownProjection(identitySeed: "journal:history")
        projection.update("已经完成的历史回答。", isFinal: true)
        XCTAssertFalse(projection.hasStreamed, "Opening history must not replay streaming animation")
        XCTAssertEqual(projection.presentation.renderedTextNodes.map(\.text).joined(), projection.text)
        projection.update("修订后的回答。", isFinal: true)
        XCTAssertEqual(projection.presentation.renderedTextNodes.map(\.text).joined(), "修订后的回答。")
    }

    func testJournalFinalityBeforeFinalBodyDoesNotSplitMarkdownStructure() {
        let prefix = "# A growing"
        let final = "# A growing heading\n\nA completed paragraph."
        var raced = MacJournalMarkdownProjection(identitySeed: "journal:race")
        raced.update(prefix, isFinal: false)
        raced.update(prefix, isFinal: true)
        raced.update(final, isFinal: true)
        var authoritative = MacJournalMarkdownProjection(identitySeed: "journal:race")
        authoritative.update(final, isFinal: true)
        XCTAssertEqual(raced.presentation.renderedTextNodes, authoritative.presentation.renderedTextNodes,
            "Late body hydration must not append to an already sealed Markdown block")
        XCTAssertTrue(raced.hasStreamed, "A late final body remains live content, unlike opening completed history")
        XCTAssertFalse(authoritative.hasStreamed)
        raced.update(final, isFinal: true)
        XCTAssertEqual(raced.presentation.renderedTextNodes, authoritative.presentation.renderedTextNodes)
    }

    func testLiveReasoningDoesNotCollapseAtCompletionButHistoryStartsCompact() {
        var live = MacJournalActivityExpansion()
        live.observe(isRunning: true)
        live.observe(isRunning: false)
        XCTAssertTrue(live.isExpanded(isRunning: false), "Completion must not remove expanded content height")
        live.toggle(isRunning: false)
        live.observe(isRunning: true)
        XCTAssertFalse(live.isExpanded(isRunning: true), "Explicit user choice wins over streaming defaults")
        var history = MacJournalActivityExpansion()
        history.observe(isRunning: false)
        XCTAssertFalse(history.isExpanded(isRunning: false))
    }

    func testReasoningStreamsExpandedAndPreservesUserDisclosureChoice() {
        var expansion = MacJournalActivityExpansion()
        XCTAssertTrue(expansion.isExpanded(isRunning: true), "Live reasoning must be visible without a click")
        XCTAssertFalse(expansion.isExpanded(isRunning: false), "Restored finished reasoning starts compact")
        expansion.toggle(isRunning: true)
        XCTAssertFalse(expansion.isExpanded(isRunning: true), "Stream updates must not undo manual collapse")
        XCTAssertFalse(expansion.isExpanded(isRunning: false))
        expansion.toggle(isRunning: false)
        XCTAssertTrue(expansion.isExpanded(isRunning: false), "Completed reasoning remains expandable")
        XCTAssertTrue(expansion.isExpanded(isRunning: true), "Explicit expansion survives state changes")
    }

    func testProductionFlowPreservesCanonicalOrderInsteadOfMovingLateReasoningBeforeAnswer() {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        func part(_ id: String, _ position: String, _ metadata: MacAssistantJournalMetadata) -> MacAssistantJournalPart {
            .init(identity: owner, part_id: id, generation: "1", revision: "1", metadata_revision: "1",
                order_key: .init(section: "transcript", position: position), metadata: metadata, bodies: [])
        }
        let answer = part("answer", "1", .text(.init(is_final: true, visibility: "public", review: nil)))
        let first = part("reasoning-1", "2", .reasoning(.init(title: .inline("reasoning"), state: "completed")))
        let step = part("step", "3", .step(.init(title: .inline("Inspect"), state: "completed")))
        let second = part("reasoning-2", "4", .reasoning(.init(title: .inline("reasoning"), state: "running")))
        let flow = MacJournalPartFlow.segments([answer, first, step, second])
        XCTAssertEqual(flow.map { $0.parts.map(\.part_id) }, [["answer"], ["reasoning-1"], ["step"], ["reasoning-2"]])
        XCTAssertEqual(flow.flatMap(\.parts).map(\.part_id), ["answer", "reasoning-1", "step", "reasoning-2"])
        XCTAssertEqual(answer.order_key.position, "1", "Presentation never rewrites journal chronology")
    }

    func testReasoningUsesLocalizedTypedTitleRatherThanWireTitle() {
        XCTAssertEqual(MacJournalActivityTitle.symbol(state: "completed"), "checkmark.circle")
        XCTAssertEqual(MacJournalActivityTitle.symbol(state: "failed"), "exclamationmark.circle")
        XCTAssertEqual(MacJournalActivityTitle.symbol(state: "cancelled"), "xmark.circle")
        for state in ["pending", "waiting_for_permission", "waiting_for_input", "sealed", "future_state"] {
            XCTAssertNotEqual(MacJournalActivityTitle.symbol(state: state), "checkmark.circle",
                "Non-running does not imply successful completion: \(state)")
        }
        let body = MacAssistantJournalContentReference(field: "title", content_generation: "1",
            utf8_bytes: "9007199254740993", chunk_count: "1", content_root: "root")
        XCTAssertTrue(MacJournalActivityTitle.hasDetails(title: .field("title"), bodies: [body]),
            "A field-backed reasoning body must survive before its text is hydrated")
        XCTAssertFalse(MacJournalActivityTitle.hasDetails(title: .field("other"), bodies: [body]))
        let empty = MacAssistantJournalContentReference(field: "title", content_generation: "1",
            utf8_bytes: "0", chunk_count: "0", content_root: "empty")
        XCTAssertFalse(MacJournalActivityTitle.hasDetails(title: .field("title"), bodies: [empty]))
        for state in ["completed", "sealed", "cancelled"] {
            XCTAssertFalse(MacJournalActivityTitle.showsReasoning(state: state, hasDetails: false))
            XCTAssertTrue(MacJournalActivityTitle.showsReasoning(state: state, hasDetails: true))
        }
        for state in ["running", "failed", "unknown"] {
            XCTAssertTrue(MacJournalActivityTitle.showsReasoning(state: state, hasDetails: false))
        }
        for title in ["reasoning", "thinking", "provider-specific title", ""] {
            XCTAssertEqual(MacJournalActivityTitle.label(isReasoning: true, inlineTitle: title), String(localized: "Thinking Process"))
            XCTAssertEqual(MacJournalActivityTitle.label(isReasoning: true, isRunning: true, inlineTitle: title), String(localized: "Thinking"))
        }
        XCTAssertEqual(MacJournalActivityTitle.label(isReasoning: false, inlineTitle: "Inspect syntax"), "Inspect syntax")
        XCTAssertEqual(MacJournalActivityTitle.label(isReasoning: false, inlineTitle: nil), String(localized: "Step"))
    }

    func testInitialPreviewLoadDoesNotFlashFullReaderLink() {
        XCTAssertFalse(MacJournalPreviewLoadState.loading.showsFullReader)
        XCTAssertFalse(MacJournalPreviewLoadState.loading.isComplete)
        XCTAssertFalse(MacJournalPreviewLoadState.loaded(complete: true).showsFullReader)
        XCTAssertTrue(MacJournalPreviewLoadState.loaded(complete: true).isComplete)
        XCTAssertTrue(MacJournalPreviewLoadState.loaded(complete: false).showsFullReader)
    }

    func testTerminalManifestTransportPreservesExactCountersAndIndependentCoreIdentity() throws {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        let digest = String(repeating: "a", count: 64)
        let result = try JSONDecoder().decode(MacRunResult.self,
            from: JSONSerialization.data(withJSONObject: macRunResultFixture("answered")))
        let seal = MacAssistantRunTerminalSeal(protocol_version: 2,
            admission: .init(identity: owner, sequence: "3", record_digest: digest), identity: owner,
            core_session_id: "different-core-session", transition_id: "terminal-transition",
            checkpoint_revision: "18446744073709551615", core_revision: "9007199254740992", event_cursor: "9007199254740993",
            result: result, result_digest: digest, completed_at_ms: 1000, entry_count: "2", heads_count: "1",
            child_receipt_count: "1", manifest_digest: digest, seal_digest: digest)
        let entries: [MacAssistantRunTerminalManifestEntry] = [
            .assistant(through: .init(identity: owner, sequence: "9007199254740993", record_digest: digest),
                ordinal: 2, createdAtMs: 999, state: "completed", completedAtMs: 1000),
            .childReceipt(taskID: "task", receiptID: "receipt", parentTurnID: "turn",
                childSessionID: "child", childRunID: "child-run", inputWatermark: 7,
                completionWatermark: 8, status: "sealed", receiptDigest: digest, resultDigest: digest)
        ]
        let page = MacAssistantRunTerminalManifestPage(seal: seal, after_entry: "0", next_entry: "2",
            complete: true, entries: entries)
        let decoded = try JSONDecoder().decode(MacAssistantRunTerminalManifestPage.self,
            from: JSONEncoder().encode(page))
        XCTAssertEqual(decoded.seal, seal)
        guard case .assistant(let through, let ordinal, _, let state, let completedAtMs) = decoded.entries[0] else {
            return XCTFail("assistant frontier lost")
        }
        XCTAssertEqual(through.sequence, "9007199254740993")
        XCTAssertEqual(ordinal, 2)
        XCTAssertEqual(state, "completed")
        XCTAssertEqual(completedAtMs, 1000)
        let request = try JSONSerialization.jsonObject(with: JSONEncoder().encode(
            MacAssistantJournalReadRequest.terminalManifest(seal, afterEntry: "0"))) as! [String: Any]
        XCTAssertEqual(request["operation"] as? String, "terminal_manifest")
        XCTAssertEqual(request["after_entry"] as? String, "0")
        XCTAssertEqual(request["max_records"] as? Int, 64)
        XCTAssertEqual(Set(request.keys), ["operation", "seal", "after_entry", "max_records", "max_encoded_bytes"])
        let malformed = Data(#"{"kind":"trace_done"}"#.utf8)
        XCTAssertThrowsError(try JSONDecoder().decode(MacAssistantRunTerminalManifestEntry.self, from: malformed))
    }

    func testJournalProgressAlwaysRequiresTheObservedRun() {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "c",
            run_id: "old-run", assistant_message_id: "a")
        let prior = MacAssistantJournalCursor(identity: owner, sequence: "7", record_digest: "prior")
        let later = MacAssistantJournalCursor(identity: owner, sequence: "8", record_digest: "later")
        XCTAssertTrue(later.advances(prior, conversationID: "c", observedRunID: "old-run"))
        XCTAssertFalse(later.advances(prior, conversationID: "c", observedRunID: "new-run"),
            "a cached matching owner must not admit late historical notifications")
        XCTAssertFalse(later.advances(nil, conversationID: "c", observedRunID: "new-run"))
        XCTAssertFalse(later.advances(prior, conversationID: "other", observedRunID: "old-run"))
        XCTAssertFalse(prior.advances(prior, conversationID: "c", observedRunID: "old-run"))
        XCTAssertFalse(prior.advances(later, conversationID: "c", observedRunID: "old-run"))
        let invalid = MacAssistantJournalCursor(identity: owner, sequence: "invalid", record_digest: "bad")
        XCTAssertFalse(invalid.advances(prior, conversationID: "c", observedRunID: "old-run"))
    }

    func testCancelledJournalExportDoesNotOpenItsDatabase() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let reference = MacJournalMessageReference(databaseURL: root.appendingPathComponent("not-opened.db"),
            cursor: .init(identity: .init(source_store_epoch: "epoch", conversation_id: "c", run_id: "r",
                assistant_message_id: "a"), sequence: "1", record_digest: "digest"))
        let result = await Task {
            withUnsafeCurrentTask { $0?.cancel() }
            do {
                _ = try await MacJournalTextRead.markdown(reference, historicalFallback: "stale")
                return false
            } catch is CancellationError { return true }
            catch { return false }
        }.value
        XCTAssertTrue(result)
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
    }

    func testAttachmentHandoverRequiresExactIdentityAndDoesNotResurrectRetiredRows() throws {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "c", run_id: "r", assistant_message_id: "a")
        let other = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "c", run_id: "other", assistant_message_id: "a")
        var coverage = MacJournalReferenceCoverage()
        coverage.adopt(identity: owner, sourceIDs: ["same"], citationIDs: [])
        XCTAssertFalse(coverage.containsAttachment(identity: owner, attachmentID: "same"))
        coverage.adopt(identity: owner, sourceIDs: [], citationIDs: [], attachmentIDs: ["same"])
        XCTAssertTrue(coverage.containsAttachment(identity: owner, attachmentID: "same"))
        XCTAssertFalse(coverage.containsAttachment(identity: other, attachmentID: "same"))
        coverage.adopt(identity: owner, sourceIDs: [], citationIDs: [])
        XCTAssertTrue(coverage.containsAttachment(identity: owner, attachmentID: "same"))
        coverage.adopt(identity: other, sourceIDs: [], citationIDs: [])
        XCTAssertFalse(coverage.containsAttachment(identity: other, attachmentID: "same"))
        let message = MacChatMessage(role: .assistant, text: "", createdAt: Date(),
            attachmentNames: ["report.pdf"], attachmentURLs: [URL(fileURLWithPath: "/tmp/report.pdf")],
            attachmentPartIDs: ["same"])
        let encoded = try JSONEncoder().encode(message)
        XCTAssertNil(try JSONDecoder().decode(MacChatMessage.self, from: encoded).attachmentPartIDs,
            "imported message data cannot provide journal handover identity")
    }

    func testJournalOwnerKeyIgnoresProgressButSeparatesStoresAndOwners() {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        let first = MacJournalMessageReference(databaseURL: URL(fileURLWithPath: "/fixture/a.db"),
            cursor: .init(identity: owner, sequence: "1", record_digest: "first"))
        let progressed = MacJournalMessageReference(databaseURL: first.databaseURL,
            cursor: .init(identity: owner, sequence: "100", record_digest: "later"))
        XCTAssertEqual(first.ownerKey, progressed.ownerKey)
        XCTAssertNotEqual(first.ownerKey, MacJournalMessageReference(databaseURL: URL(fileURLWithPath: "/fixture/b.db"),
            cursor: first.cursor).ownerKey)
        for other in [
            MacAssistantJournalIdentity(source_store_epoch: "new", conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant"),
            MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "other", run_id: "run", assistant_message_id: "assistant"),
            MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "other", assistant_message_id: "assistant"),
            MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation", run_id: "run", assistant_message_id: "other")
        ] {
            XCTAssertNotEqual(first.ownerKey, MacJournalOwnerKey(databaseURL: first.databaseURL, identity: other))
        }
    }

    func testTaskControlsRequireExplicitVersionedCapabilities() throws {
        let task: [String: Any] = ["task_id": "task", "display_key": "task.background",
            "state": "running", "cancel_requested": false]
        let old = try JSONDecoder().decode(MacAssistantJournalMetadata.self,
            from: JSONSerialization.data(withJSONObject: ["kind": "task", "data": task]))
        guard case .task(let historical) = old else { return XCTFail("task metadata") }
        XCTAssertFalse(historical.supportsGuidance)
        XCTAssertFalse(historical.supportsCancellation)
        for supportsGuidance in [false, true] {
            let data = try JSONSerialization.data(withJSONObject: ["kind": "task_control", "data": [
                "task": task, "supports_guidance": supportsGuidance, "supports_cancellation": true
            ]])
            let decoded = try JSONDecoder().decode(MacAssistantJournalMetadata.self, from: data)
            guard case .task(let current) = decoded else { return XCTFail("controlled task") }
            XCTAssertEqual(current.supportsGuidance, supportsGuidance)
            XCTAssertTrue(current.supportsCancellation)
        }
    }

    func testJournalTaskInteractionFailsClosedForUnknownAndCancellingState() throws {
        for state in ["pending", "running", "waiting_for_permission", "completed", "sealed", "failed", "cancelled", "future_state"] {
            for cancelling in [false, true] {
                let data = try JSONSerialization.data(withJSONObject: [
                    "task_id": "task", "display_key": "task.background", "state": state,
                    "cancel_requested": cancelling
                ])
                let task = try JSONDecoder().decode(MacAssistantJournalMetadata.Task.self, from: data)
                XCTAssertEqual(task.acceptsInteraction,
                    !cancelling && ["pending", "running", "waiting_for_permission"].contains(state))
                XCTAssertEqual(task.isTerminal, ["completed", "sealed", "failed", "cancelled"].contains(state))
            }
        }
    }

    func testAttachmentMetadataKeepsBoundedFieldsAndDoesNotAuthorizeLocalPaths() throws {
        let wire = Data(#"{"attachment_id":"pdf-1","name":{"kind":"inline","value":"报告.pdf"},"mime_type":{"kind":"inline","value":"application/pdf"},"uri":{"kind":"field","value":"attachment_uri"},"operation":{"kind":"inline","value":"created"}}"#.utf8)
        let attachment = try JSONDecoder().decode(MacAssistantJournalMetadata.Attachment.self, from: wire)
        XCTAssertEqual(attachment.attachment_id, "pdf-1")
        guard case .inline(let name) = attachment.name,
              case .field(let field) = attachment.uri else {
            return XCTFail("attachment metadata must preserve field references")
        }
        XCTAssertEqual(name, "报告.pdf")
        XCTAssertEqual(field, "attachment_uri")
        XCTAssertNil(MacJournalArtifactDestination.resolve(field, localJournal: true))
        XCTAssertNil(MacJournalArtifactDestination.resolve("file:///tmp/report.pdf", localJournal: false))
    }

    func testChangeWireRequiresExplicitRetirementAndPageTermination() throws {
        let decoder = JSONDecoder()
        let retired = try decoder.decode(MacAssistantJournalPartChangePage.Change.self,
            from: Data(#"{"part_id":"gone","head":null}"#.utf8))
        XCTAssertEqual(retired.part_id, "gone")
        XCTAssertNil(retired.head)
        for value in [#"{"part_id":"gone"}"#, #"{"part_id":"","head":null}"#] {
            XCTAssertThrowsError(try decoder.decode(MacAssistantJournalPartChangePage.Change.self, from: Data(value.utf8)))
        }
        let cursor = MacAssistantJournalCursor(identity: .init(source_store_epoch: "epoch", conversation_id: "c",
            run_id: "r", assistant_message_id: "a"), sequence: "1", record_digest: "digest")
        let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(cursor))
        var page: [String: Any] = ["base": encoded, "at": encoded, "changes": []]
        XCTAssertThrowsError(try decoder.decode(MacAssistantJournalPartChangePage.self,
            from: JSONSerialization.data(withJSONObject: page)))
        page["next_part_id"] = NSNull()
        XCTAssertNil(try decoder.decode(MacAssistantJournalPartChangePage.self,
            from: JSONSerialization.data(withJSONObject: page)).next_part_id)
    }

    func testFieldReadIdentityDoesNotReuseAcrossOwnersOrContentVersions() {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "c",
            run_id: "r", assistant_message_id: "a")
        let content = MacAssistantJournalContentReference(field: "uri", content_generation: "1",
            utf8_bytes: "10", chunk_count: "1", content_root: "root")
        let database = URL(fileURLWithPath: "/fixture/store.db")
        let key = MacJournalFieldReadKey(databaseURL: database, identity: owner, partID: "source", content: content)
        XCTAssertEqual(key, .init(databaseURL: database, identity: owner, partID: "source", content: content))
        XCTAssertNotEqual(key, .init(databaseURL: URL(fileURLWithPath: "/fixture/other.db"),
            identity: owner, partID: "source", content: content))
        XCTAssertNotEqual(key, .init(databaseURL: database, identity: owner, partID: "other", content: content))
        XCTAssertNotEqual(key, .init(databaseURL: database,
            identity: .init(source_store_epoch: "other", conversation_id: "c", run_id: "r", assistant_message_id: "a"),
            partID: "source", content: content))
        XCTAssertNotEqual(key, .init(databaseURL: database, identity: owner, partID: "source",
            content: .init(field: "uri", content_generation: "2", utf8_bytes: "10", chunk_count: "1", content_root: "root")))
        XCTAssertNotEqual(key, .init(databaseURL: database, identity: owner, partID: "source",
            content: .init(field: "uri", content_generation: "1", utf8_bytes: "10", chunk_count: "1", content_root: "new-root")))
    }

    func testNavigationFieldBudgetRejectsOversizeBeforeOpeningStore() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let reference = MacJournalMessageReference(databaseURL: directory.appendingPathComponent("unopened.db"),
            cursor: .init(identity: .init(source_store_epoch: "epoch", conversation_id: "c",
                run_id: "r", assistant_message_id: "a"), sequence: "1", record_digest: "digest"))
        let oversized = MacAssistantJournalContentReference(field: "uri", content_generation: "1",
            utf8_bytes: "32769", chunk_count: "1", content_root: "root")
        let value = try await MacJournalTextRead.completeSmallText(reference, partID: "source", content: oversized)
        XCTAssertNil(value, "oversized navigation data must not become a truncated link")
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.path))
        let malformed = MacAssistantJournalContentReference(field: "uri", content_generation: "1",
            utf8_bytes: "-1", chunk_count: "1", content_root: "root")
        do {
            _ = try await MacJournalTextRead.completeSmallText(reference, partID: "source", content: malformed)
            XCTFail("invalid byte length must fail closed")
        } catch {}
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.path))
    }

    func testReferenceHandoverSeparatesSourceCitationAndOwnerIdentities() {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "c",
            run_id: "r", assistant_message_id: "a")
        let other = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "c",
            run_id: "next", assistant_message_id: "a")
        var coverage = MacJournalReferenceCoverage()
        coverage.adopt(identity: owner, sourceIDs: ["same-id"], citationIDs: [])
        XCTAssertTrue(coverage.contains(identity: owner, sourceID: "same-id", citationID: nil))
        XCTAssertFalse(coverage.contains(identity: owner, sourceID: "same-id", citationID: "same-id"))
        coverage.adopt(identity: owner, sourceIDs: [], citationIDs: ["citation"])
        XCTAssertTrue(coverage.contains(identity: owner, sourceID: "same-id", citationID: nil))
        XCTAssertTrue(coverage.contains(identity: owner, sourceID: "same-id", citationID: "citation"))
        XCTAssertFalse(coverage.contains(identity: other, sourceID: "same-id", citationID: "citation"))
        coverage.adopt(identity: other, sourceIDs: [], citationIDs: [])
        XCTAssertFalse(coverage.contains(identity: other, sourceID: "same-id", citationID: nil))
        XCTAssertFalse(coverage.contains(identity: owner, sourceID: "same-id", citationID: "citation"))
    }

    func testArtifactDestinationsDoNotInterpretRemotePathsAsLocalFiles() {
        let local = "file:///tmp/report.pdf"
        XCTAssertNil(MacJournalArtifactDestination.resolve(local, localJournal: false))
        XCTAssertEqual(MacJournalArtifactDestination.resolve(local, localJournal: true),
            .localFile(URL(string: local)!))
        for uri in ["file://foreign-host/tmp/report.pdf", "report.pdf", "javascript:alert(1)", "data:text/plain,secret", "custom-app://run"] {
            XCTAssertNil(MacJournalArtifactDestination.resolve(uri, localJournal: true))
        }
        let web = "https://example.com/report.pdf"
        XCTAssertEqual(MacJournalArtifactDestination.resolve(web, localJournal: false), .web(URL(string: web)!))
    }

    func testStructuredJournalFieldsDisplayOnlyCompleteVersionedValues() throws {
        let input = MacJournalBodyPresentation.toolField("tool_input")
        let wire = #"{"protocol_version":1,"value":{"id":9007199254740993,"max":18446744073709551615}}"#
        XCTAssertEqual(try input.display(String(wire.prefix(20)), complete: false), "",
            "a partial structured envelope is not a user-visible result")
        let displayed = try input.display(wire, complete: true)
        XCTAssertFalse(displayed.contains("protocol_version"))
        XCTAssertTrue(displayed.contains("9007199254740993"))
        XCTAssertTrue(displayed.contains("18446744073709551615"))
        XCTAssertThrowsError(try input.display(#"{"protocol_version":2,"value":{}}"#, complete: true))
        XCTAssertThrowsError(try input.display(#"{"protocol_version":1,"resources":[]}"#, complete: true))
        XCTAssertThrowsError(try input.display("{", complete: true))
        let resources = try MacJournalBodyPresentation.toolField("tool_resources")
            .display(#"{"protocol_version":1,"resources":[]}"#, complete: true)
        XCTAssertEqual(try JSONDecoder().decode(MacCanonicalJSONValue.self, from: Data(resources.utf8)), .array([]))
        XCTAssertEqual(try MacJournalBodyPresentation.toolField("tool_output_text")
            .display("**literal tool output**", complete: false), "**literal tool output**")
    }

    func testStructuredToolAndMirrorValuesPreserveCoreIntegerDomain() throws {
        let wire = Data(#"{"value":{"id":9007199254740993,"min":-9223372036854775808,"max":18446744073709551615,"small":42,"fraction":0.5,"flag":true},"protocol_version":1}"#.utf8)
        let direct = try JSONDecoder().decode(MacCanonicalJSONValue.self, from: wire)
        let bridged = try MacCanonicalJSONValue(foundationValue: JSONSerialization.jsonObject(with: wire))
        XCTAssertEqual(direct, bridged, "SSE Foundation decoding and direct field decoding must agree")
        guard case .object(let envelope) = direct, let nested = envelope["value"],
              case .object(let value) = nested else {
            return XCTFail("Expected a structured tool payload")
        }
        XCTAssertEqual(value["id"], .signedInteger(9_007_199_254_740_993))
        XCTAssertEqual(value["min"], .signedInteger(Int64.min))
        XCTAssertEqual(value["max"], .unsignedInteger(UInt64.max))
        XCTAssertEqual(value["small"], .number(42))
        XCTAssertEqual(value["fraction"], .number(0.5))
        XCTAssertEqual(value["flag"], .bool(true))
        let encoded = try JSONEncoder().encode(direct)
        let text = String(decoding: encoded, as: UTF8.self)
        XCTAssertTrue(text.contains("9007199254740993"))
        XCTAssertTrue(text.contains("18446744073709551615"))
        XCTAssertEqual(try JSONDecoder().decode(MacCanonicalJSONValue.self, from: encoded), direct)
    }

    func testActivityHandoverOnlyRetiresExactAdoptedPartsAndOwner() {
        let owner = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "assistant")
        let successor = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "next", assistant_message_id: "assistant")
        var coverage = MacJournalActivityCoverage()
        XCTAssertFalse(coverage.contains(identity: owner, partID: "reasoning"))
        coverage.adopt(identity: owner, partIDs: ["reasoning", "step"], taskIDs: ["task-1"])
        XCTAssertTrue(coverage.containsTask(identity: owner, taskID: "task-1"))
        XCTAssertFalse(coverage.containsTask(identity: owner, taskID: "task-2"))
        XCTAssertFalse(coverage.contains(identity: owner, partID: "task-1"), "Task IDs are not Part IDs")
        XCTAssertTrue(coverage.contains(identity: owner, partID: "step"))
        XCTAssertFalse(coverage.contains(identity: owner, partID: "unmigrated-tool"))
        coverage.adopt(identity: owner, partIDs: [])
        XCTAssertTrue(coverage.containsTask(identity: owner, taskID: "task-1"), "retired task cards cannot return from a stale snapshot")
        XCTAssertFalse(coverage.containsTask(identity: successor, taskID: "task-1"))
        XCTAssertTrue(coverage.contains(identity: owner, partID: "reasoning"), "removed Parts cannot return through stale traces")
        XCTAssertFalse(coverage.contains(identity: successor, partID: "reasoning"))
        coverage.adopt(identity: successor, partIDs: ["new-step"])
        XCTAssertFalse(coverage.containsTask(identity: successor, taskID: "task-1"))
        XCTAssertFalse(coverage.containsTask(identity: owner, taskID: "task-1"))
        XCTAssertFalse(coverage.contains(identity: owner, partID: "step"))
        XCTAssertFalse(coverage.contains(identity: successor, partID: "reasoning"))
        XCTAssertTrue(coverage.contains(identity: successor, partID: "new-step"))
    }

    func testJournalDatabaseBindingCannotBeImportedThroughMessageJSON() throws {
        var message = MacChatMessage(role: .assistant, text: "answer", createdAt: .distantPast)
        message.journalReference = .init(databaseURL: URL(fileURLWithPath: "/fixture/private.sqlite"),
            cursor: .init(identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
                run_id: "run", assistant_message_id: "assistant"), sequence: "1", record_digest: "digest"))
        message.canonicalPresentation = .init(message: .init(messageID: "canonical", role: .assistant,
            state: .completed, revision: 7, createdAtMs: 1, parts: []))
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(message)) as? [String: Any])
        XCTAssertNil(json["journalReference"])
        XCTAssertNil(json["canonicalPresentation"])
        json["journalReference"] = ["databaseURL": "file:///foreign.sqlite"]
        let imported = try JSONDecoder().decode(MacChatMessage.self, from: JSONSerialization.data(withJSONObject: json))
        XCTAssertNil(imported.journalReference)
        XCTAssertNil(imported.canonicalPresentation)
        XCTAssertEqual(imported.text, "answer")
    }

    func testEveryPublicMetadataKindKeepsTypedReferencesAndExactCounters() throws {
        let title: [String: Any] = ["kind": "field", "value": "title"]
        let fixtures: [(String, [String: Any])] = [
            ("text", ["is_final": false, "visibility": "withheld", "review": "pending"]),
            ("reasoning", ["title": title, "state": "running"]),
            ("step", ["title": title, "state": "completed"]),
            ("tool", ["execution_source": ["kind": "model"], "tool_call_id": "call", "capability_id": "read", "title": title,
                "origin": ["kind": "builtin"], "permission": ["request_id": "request",
                    "permission_domain": title, "rationale": title], "state": "waiting_for_permission",
                "affected_resources": ["kind": "field", "value": "tool_resources"], "artifacts": "tool_artifacts",
                "full_output": ["message_id": "message", "part_id": "part", "tool_call_id": "call"],
                "started_at_ms": "9007199254740993"]),
            ("source", ["source_id": "source", "title": title, "uri": title, "evidence": "source_evidence"]),
            ("citation", ["citation_id": "citation", "source_id": "source", "title": title, "uri": title,
                "origin_tool_call_id": "call", "location": ["page": "9007199254740993", "section": title],
                "byte_range": ["start": "0", "end": "9007199254740993"]]),
            ("attachment", ["attachment_id": "attachment", "name": title, "uri": title]),
            ("task", ["task_id": "task", "display_key": "task", "state": "running", "cancel_requested": false,
                "progress": ["fraction_bits": String(0.5.bitPattern)]]),
            ("skill", ["run_id": "skill-run", "skill_id": "skill", "display_name": title, "mode": "fork",
                "state": "completed", "model_turns": "9007199254740993", "tool_calls": "2", "elapsed_millis": "3"]),
            ("model_attempt", ["attempt_id": "attempt", "state": "failed", "failure_code": "timeout", "retryable": true,
                "http_status": "429", "fallback_attempt_id": "successor", "fallback_provider": "provider", "fallback_model": "model"]),
            ("error", ["code": "failure", "display_key": "failure", "recoverable": true]),
            ("historical", ["schema": "agent_part", "canonical_kind": "text", "canonical_revision": "9007199254740993"]),
        ]
        var originalOrder: [MacAssistantJournalPart] = []
        for (kind, payload) in fixtures {
            let metadata = try JSONDecoder().decode(MacAssistantJournalMetadata.self,
                from: JSONSerialization.data(withJSONObject: ["kind": kind, "data": payload]))
            originalOrder.append(.init(identity: .init(source_store_epoch:"e",conversation_id:"c",run_id:"r",assistant_message_id:"a"),
                part_id:kind,generation:"1",revision:"1",metadata_revision:"1",order_key:.init(section:"transcript",position:"1"),metadata:metadata,bodies:[]))
            switch metadata {
            case .tool(let tool):
                XCTAssertEqual(tool.started_at_ms, "9007199254740993")
                guard case .field("title") = tool.title,
                      case .field("tool_resources") = tool.affected_resources else {
                    return XCTFail("References must remain references, not guessed text or hydrated JSON")
                }
                XCTAssertEqual(tool.full_output?.part_id, "part")
                XCTAssertEqual(tool.permission?.request_id, "request")
            case .task(let task): XCTAssertEqual(task.progress?.fraction, 0.5)
            case .skill(let skill): XCTAssertEqual(skill.model_turns, "9007199254740993")
            case .modelAttempt(let attempt):
                XCTAssertEqual(attempt.attempt_id, "attempt")
                XCTAssertEqual(attempt.fallback_attempt_id, "successor")
                XCTAssertEqual(attempt.http_status, "429")
                XCTAssertTrue(attempt.retryable)
            case .text(let text): XCTAssertEqual(text.visibility, "withheld")
            case .reasoning(let activity):
                XCTAssertTrue(activity.isRunning)
                guard case .field("title") = activity.title else { return XCTFail("Reasoning title must retain its Part field reference") }
            case .step(let activity): XCTAssertFalse(activity.isRunning)
            default: break
            }
        }
        XCTAssertEqual(MacJournalPartFlow.segments(originalOrder).flatMap(\.parts).map(\.part_id), originalOrder.map(\.part_id))
        XCTAssertTrue(MacJournalPartFlow.segments(originalOrder).allSatisfy { $0.parts.count == 1 },
            "Every non-R/T kind in this canonical sequence remains an individual anchor")
        XCTAssertThrowsError(try JSONDecoder().decode(MacAssistantJournalMetadata.self,
            from: Data(#"{"kind":"future_kind","data":{}}"#.utf8)), "Unknown kinds must fail, not disappear from the read model")
    }

    func testRecordChunkUsesCoreOwnedPrefixAndPreservesDecimalIntegers() throws {
        let identity = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
                                                   run_id: "run", assistant_message_id: "assistant")
        let cursor = MacAssistantJournalCursor(identity: identity, sequence: "9007199254740993", record_digest: "digest")
        let snapshot = MacAssistantJournalSnapshot(snapshot_id: "snapshot", at: cursor)
        let need = try JSONDecoder().decode(MacAssistantJournalContentNeed.self, from: Data(#"{"part_id":"part","content":{"field":"markdown","content_generation":"1","utf8_bytes":"9007199254740995","chunk_count":"9007199254740994","content_root":"final"},"expected_prefix":{"field":"markdown","content_generation":"1","utf8_bytes":"9007199254740994","chunk_count":"9007199254740993","content_root":"prepared"}}"#.utf8))
        let encoded = try JSONEncoder().encode(MacAssistantJournalReadRequest.chunk(snapshot, need))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        XCTAssertEqual(object["operation"] as? String, "record_chunk")
        XCTAssertEqual(object["chunk_index"] as? String, "9007199254740993")
        XCTAssertNil(object["source_host_id"], "The transport target, not request JSON, supplies Host authority")
        XCTAssertEqual(Set(object.keys), ["operation", "snapshot", "part_id", "content", "chunk_index"])
        let decoded = try JSONDecoder().decode(MacAssistantJournalSnapshot.self, from: JSONEncoder().encode(snapshot))
        XCTAssertEqual(decoded.at, cursor)
    }

    func testSnapshotRequestsHaveSeparateExactOperations() throws {
        let identity = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
                                                   run_id: "run", assistant_message_id: "assistant")
        let cursor = MacAssistantJournalCursor(identity: identity, sequence: "1", record_digest: "digest")
        let requests: [(MacAssistantJournalReadRequest, String, Set<String>)] = [
            (.open(cursor), "open_snapshot", ["operation", "at"]),
            (.release(.init(snapshot_id: "snapshot", at: cursor)), "release_snapshot", ["operation", "snapshot"]),
        ]
        for (request, operation, keys) in requests {
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
            XCTAssertEqual(object["operation"] as? String, operation)
            XCTAssertEqual(Set(object.keys), keys)
        }
    }

    func testRecoveryDiscoversExactRunWithoutSupplyingSourceEpochOrContent() throws {
        let request = MacAssistantJournalReadRequest.heads(conversationID: "conversation", runID: "run", afterMessageID: "message")
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual(object["operation"] as? String, "heads")
        XCTAssertEqual(object["conversation_id"] as? String, "conversation")
        XCTAssertEqual(object["run_id"] as? String, "run")
        XCTAssertEqual(object["after_message_id"] as? String, "message")
        XCTAssertEqual(object["limit"] as? Int, 64)
        XCTAssertEqual(Set(object.keys), ["operation", "conversation_id", "run_id", "after_message_id", "limit"])
    }
}
