// SPDX-License-Identifier: Apache-2.0
import Foundation
import CryptoKit
import Network
import XCTest
@testable import Nola

@MainActor
final class MacRemoteCancellationReceiptTests: XCTestCase {
    func testColdRoomHTTPRejectionObservesCapturedSourceWithoutResubmitting() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 409, delayed: false, mode: "room_rejected")
        defer { server.terminate() }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("receiver.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let target = try service.captureSubmissionTarget()
        let bound = try port.prepareFixtureRemoteSubmission(.init(conversationID: "conversation",
            conversationTitle: "Room", submissionID: "original", sourceHostID: "fixture-host", clientID: target.clientID,
            userMessageID: "original:user", userPartID: "original:user:text", userText: "fixture", userContentJSON: nil,
            assistantMessageID: "original:assistant", assistantPartID: "original:assistant:text", contextCheckpointID: nil, createdAtMs: 1))
        // The private Room owner must keep its captured source even if settings
        // change while this immutable execution remains pending.
        try configuration.save(url: "https://changed.invalid", token: "changed-token", hostID: "changed-host")
        var rejected = 0
        let digest = try XCTUnwrap(bound.request_digest)
        let stream = await service.runBoundRoomRequest(identity: bound.identity,
            digest: digest, target: target)
        for try await event in stream {
            if event.remoteSubmissionIntent?.phase == .rejected { rejected += 1 }
        }
        XCTAssertEqual(rejected, 1)
        XCTAssertNil(try port.remoteSubmissionIntent(conversationID: "conversation"))
        XCTAssertNil(try port.activeRunIdentity(conversationID: "conversation"))
        XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "original"))
        let counts = try await requestCounts(baseURL)
        XCTAssertEqual(counts["streams"], 1)
    }

    private func fixturePermissionEvidence() throws -> MacRemotePermissionToolEvidence {
        try XCTUnwrap(MacRemotePermissionToolEvidence.decode([
            "catalog_revision": 2, "tool": ["tool_id": "web_search", "route": "agent_host",
                "schema": ["type": "function", "function": ["name": "web_search", "parameters": ["type": "object"]]]]
        ], toolName: "web_search"))
    }

    func testColdHTTPObservationImportsAdmissionAndOriginalNonceThenActualTerminalWithoutSubmit() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 202, delayed: false, mode: "cold_observe")
        defer { server.terminate() }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("receiver.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let target = try service.captureSubmissionTarget()
        let first = try await service.observeRun(conversationID: "conversation", expectedRunID: nil,
            assistantMessageID: nil, target: target)
        XCTAssertEqual(first.observation?.run_id, "original")
        let permission = try XCTUnwrap(first.events.compactMap(\.permissionRequest).first)
        XCTAssertEqual(permission.nonce, "original-cold-nonce")
        XCTAssertTrue(first.events.allSatisfy { $0.trace == nil })
        XCTAssertTrue(permission.reason.contains("Review this search"))
        // Configuration edits cannot retarget this admitted Run or nonce.
        try configuration.save(url: "https://changed.invalid", token: "changed-token", hostID: "changed-host")
        // No stream receipt exists. The real decision path must acquire a
        // original Run catalog without consulting current settings or inventing a challenge.
        try await service.resolvePermission(request: permission, allow: false)
        do {
            _ = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
                assistantMessageID: "foreign-root", target: target)
            XCTFail("Observation cannot replace the pinned assistant owner")
        } catch MacRemoteHostError.invalidResponse {}
        let second = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
            assistantMessageID: "original:assistant", target: target)
        XCTAssertEqual(second.observation?.recovery, .terminal)
        XCTAssertNotNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "original"))
        let cursor = try XCTUnwrap(second.events.compactMap(\.assistantJournalCursor).last)
        let text = try await MacJournalTextRead.markdown(.init(databaseURL: port.databaseURL, cursor: cursor), historicalFallback: "")
        XCTAssertEqual(text, "Cold final body")
        let committedUsage = try XCTUnwrap(second.observation?.context_usage)
        XCTAssertEqual(committedUsage.usage.used_tokens, "7500")
        XCTAssertFalse(committedUsage.usage.estimated)
        let reopened = try MacApplicationPort(databaseURL: port.databaseURL)
        let repository = MacCanonicalConversationRepository(store: reopened)
        let restored = MacWorkspaceModel(initialConversationID: "conversation", canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        restored.remoteObservationForegroundChanged(false)
        await restored.loadSelectedConversationIfNeeded()
        XCTAssertEqual(restored.contextUsed, 7_500)
        XCTAssertEqual(restored.contextWindowTokens, 32_768)
        XCTAssertEqual(restored.contextPromptLimit, 28_672)
        XCTAssertFalse(restored.contextUsageIsEstimated)
        let (data, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("counts"))
        let counts = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Int])
        XCTAssertEqual(counts["streams"], 0)
        XCTAssertEqual(counts["settings"], 0)
        XCTAssertEqual(counts["catalog_reads"], 1)
        let (requestData, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("requests"))
        let envelopes = try XCTUnwrap(JSONSerialization.jsonObject(with: requestData) as? [[String: Any]])
        let decision = try decryptFixtureEnvelope(XCTUnwrap(envelopes.first))
        XCTAssertEqual(decision["nonce"] as? String, "original-cold-nonce")
    }
    func testHTTPRunSendsExplicitMemoryBooleans() async throws {
        for enabled in [false, true] {
            try await exerciseRunningProjection(mode: "partial", expectedBodies: ["Running body"], memoryEnabled: enabled)
        }
    }

    func testRunningHTTPProjectionYieldsWithoutRetiringPermissionReceipt() async throws {
        try await exerciseRunningProjection(mode: "partial", expectedBodies: ["Running body"])
    }

    func testWrongRunAndSessionHTTPProjectionsNeverYield() async throws {
        for mode in ["partial_wrong_run", "partial_wrong_session"] {
            try await exerciseRunningProjection(mode: mode, expectedBodies: [])
        }
    }

    func testWrongHostHTTPProjectionRejectsStreamWithoutRetiringReceipt() async throws {
        try await exerciseRunningProjection(mode: "partial_wrong_host", expectedBodies: [])
    }

    func testTerminalThenLateHTTPPartialNeverYieldsLateBody() async throws {
        try await exerciseRunningProjection(mode: "partial_terminal_late", expectedBodies: ["Running body"])
    }

    func testVerifiedTerminalClosesExecutionBeforeForeignTrailingFrame() async throws {
        try await exerciseRunningProjection(mode: "partial_terminal_foreign_late", expectedBodies: ["Running body"])
    }

    private func exerciseRunningProjection(mode: String, expectedBodies: [String], memoryEnabled: Bool = false) async throws {
        let expectsTerminal = ["partial_terminal_late", "partial_terminal_foreign_late"].contains(mode)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 202, delayed: false, mode: mode)
        defer { if server.isRunning { server.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "original", generationID: 1))
        try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
        let stream = await service.run(conversationID: "conversation", submissionID: "original",
            userMessageID: "user", assistantMessageID: "original:assistant", prompt: "fixture",
            modelID: "fixture", modelProviderID: nil, modelIntent: nil, memoryEnabled: memoryEnabled, executionBackend: "codex",
            externalSessionID: nil, submission: .init(target: try service.captureSubmissionTarget(), ticket: ticket,
                beforeDispatch: {
                    try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
                    _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Fixture",
                        submissionID: "original", sourceHostID: "fixture-host", clientID: try configuration.clientID(), userMessageID: "user",
                        userPartID: "user:text", userText: "fixture", userContentJSON: nil,
                        assistantMessageID: "original:assistant", assistantPartID: "original:assistant:text",
                        contextCheckpointID: nil, createdAtMs: 1))
                }))
        var bodies: [String] = []
        var terminals = 0
        do {
            for try await event in stream {
                if let cursor = event.assistantJournalCursor {
                    bodies.append(try await MacJournalTextRead.markdown(
                        .init(databaseURL: directory.appendingPathComponent("test.db"), cursor: cursor),
                        historicalFallback: ""))
                }
                if event.assistantRunTerminalSealDigest != nil { terminals += 1 }
            }
            XCTAssertTrue(expectsTerminal, "Nonterminal EOF must not become completion")
        } catch MacRemoteHostError.transportInterrupted {
            XCTAssertFalse(expectsTerminal)
            XCTAssertNotEqual(mode, "partial_wrong_host")
        } catch MacRemoteHostError.invalidResponse {
            XCTAssertTrue(mode.hasPrefix("partial_wrong_"))
        }
        XCTAssertEqual(bodies, expectedBodies)
        let (recorded, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("requests"))
        let requests = try XCTUnwrap(JSONSerialization.jsonObject(with: recorded) as? [[String: Any]])
        let sent = try decryptFixtureEnvelope(XCTUnwrap(requests.first))
        XCTAssertEqual(sent["memory_enabled"] as? Bool, memoryEnabled)
        XCTAssertEqual(terminals, expectsTerminal ? 1 : 0)
        let request = MacRemotePermissionRequest(id: "request", allowConversation: true, protocolVersion: MacRemotePermissionRequest.supportedProtocolVersion,
            nonce: "nonce", hostID: "fixture-host", clientID: try configuration.clientID(), sessionID: "conversation",
            toolName: "web_search", arguments: "{}", callID: "call", reason: "fixture", message: "fixture", expiresAt: nil,
            toolCatalogEvidence: try fixturePermissionEvidence())
        if expectsTerminal {
            do {
                try await service.resolvePermission(request: request, allow: false)
                XCTFail("Only the matching terminal retires the receipt")
            } catch MacRemoteHostError.invalidResponse {
                let counts = try await requestCounts(baseURL)
                XCTAssertEqual(counts["permissions"], 0, "A terminal Run cannot send permission decisions")
            }
        } else {
            // Goes through the retained production receipt and real HTTP path.
            try await service.resolvePermission(request: request, allow: false)
            let counts = try await requestCounts(baseURL)
            XCTAssertEqual(counts["permissions"], 1)
            XCTAssertEqual(counts["catalog_reads"], 1)
        }
    }

    func testCancellationDuringSettingsFetchNeverDispatchesOrCommitsMirror() async throws {
        try await exerciseSubmissionBoundary(failMirror: false)
    }

    func testFailedMirrorCommitNeverSendsHTTPRequest() async throws {
        try await exerciseSubmissionBoundary(failMirror: true)
    }

    func testUnfinishedRunRejectsReplacementAndPreservesExistingPermissionReceipt() async throws {
        try await exerciseSubmissionBoundary(failMirror: false, preserveReceipt: true)
    }

    private func exerciseSubmissionBoundary(failMirror: Bool, preserveReceipt: Bool = false) async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: 202, delayed: false,
            mode: preserveReceipt ? "settings_gate_after_first" : "settings_gate")
        defer { if server.isRunning { server.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        if preserveReceipt { try await establishReceipt(service, port: port) }
        let submissionID = preserveReceipt ? "replacement" : "original"
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: submissionID, generationID: 1))
        var mirrorCommits = 0
        let control = MacRemoteSubmissionControl(target: try service.captureSubmissionTarget(), ticket: ticket) {
            mirrorCommits += 1
            throw MacRemoteHostError.invalidResponse
        }
        try port.ensureFixtureConversation(id: "conversation", title: "Receipt")
        let stream = await service.run(conversationID: "conversation", submissionID: submissionID,
            userMessageID: "user", assistantMessageID: "assistant", prompt: "fixture", contextComponents: [],
            modelID: "fixture", modelProviderID: nil, modelIntent: nil, memoryEnabled: false, executionBackend: "codex",
            externalSessionID: nil, submission: control)
        // A running original has no terminal frontier and must reject a new
        // submission before contacting settings. The other cases exercise
        // cancellation and mirror failure after preparation actually suspends.
        if !preserveReceipt {
            try await waitForObservation(baseURL)
            if !failMirror { XCTAssertTrue(ticket.cancelPreparation()) }
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("release"))
        }
        do {
            for try await _ in stream {}
            XCTFail("Submission should fail before transport")
        } catch {
            XCTAssertEqual(ticket.phase, (failMirror || preserveReceipt) ? .rejectedBeforeDispatch : .abandoned)
        }
        XCTAssertEqual(mirrorCommits, failMirror ? 1 : 0)
        let counts = try await requestCounts(baseURL)
        XCTAssertEqual(counts["streams"], preserveReceipt ? 1 : 0)
        XCTAssertEqual(counts["cancels"], 0)
        if preserveReceipt {
            XCTAssertEqual(counts["settings"], 1, "No settings request after an unfinished Run frontier")
            let request = MacRemotePermissionRequest(id: "request", allowConversation: true, protocolVersion: MacRemotePermissionRequest.supportedProtocolVersion,
                nonce: "nonce", hostID: "fixture-host", clientID: try configuration.clientID(),
                sessionID: "conversation", toolName: "web_search", arguments: "{}",
                callID: "call", reason: "fixture", message: "fixture", expiresAt: nil,
                toolCatalogEvidence: try fixturePermissionEvidence())
            try await service.resolvePermission(request: request, allow: false)
            let counts = try await requestCounts(baseURL)
            XCTAssertEqual(counts["permissions"], 1)
        }
    }

    func testFrozenTargetAlsoOwnsSubsequentCancellationAfterDefaultChanges() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (first, firstURL) = try startServer(status: 202, delayed: false)
        let (second, secondURL) = try startServer(status: 202, delayed: false)
        defer { if first.isRunning { first.terminate() }; if second.isRunning { second.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: firstURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        try port.ensureFixtureConversation(id: "conversation", title: "Frozen")
        let target = try service.captureSubmissionTarget()
        try configuration.save(url: secondURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let ticket = MacRunSubmissionTicket(identity: .init(conversationID: "conversation", submissionID: "original", generationID: 1))
        let stream = await service.run(conversationID: "conversation", submissionID: "original",
            userMessageID: "user", assistantMessageID: "assistant", prompt: "fixture", contextComponents: [],
            modelID: "fixture", modelProviderID: nil, modelIntent: nil, memoryEnabled: false, executionBackend: "codex",
            externalSessionID: nil, submission: .init(target: target, ticket: ticket, beforeDispatch: {
                try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
                _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Frozen",
                    submissionID: "original", sourceHostID: "fixture-host", clientID: target.clientID,
                    userMessageID: "user", userPartID: "user:text", userText: "fixture", userContentJSON: nil,
                    assistantMessageID: "assistant", assistantPartID: "assistant:text", contextCheckpointID: nil, createdAtMs: 1))
            }))
        do { for try await _ in stream {} } catch MacRemoteHostError.transportInterrupted {}
        XCTAssertEqual(ticket.phase, .accepted, "Only the source admission receipt accepts this submission")
        let intent = try XCTUnwrap(port.remoteSubmissionIntent(conversationID: "conversation"))
        try await service.cancelRun(identity: intent.identity, requestDigest: XCTUnwrap(intent.request_digest))
        let old = try await requestCounts(firstURL)
        let new = try await requestCounts(secondURL)
        XCTAssertEqual(old["streams"], 1)
        XCTAssertEqual(old["cancels"], 1)
        XCTAssertEqual(new["settings"], 0)
        XCTAssertEqual(new["streams"], 0)
        XCTAssertEqual(new["cancels"], 0)
    }

    private func requestCounts(_ baseURL: URL) async throws -> [String: Int] {
        let (data, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("counts"))
        return try JSONDecoder().decode([String: Int].self, from: data)
    }

    func testRejectedCancellationPreservesPermissionReceipt() async throws {
        try await exercise(status: 503, delayed: false)
    }

    func testAcceptedCancellationIsNotTerminalEvidence() async throws {
        try await exercise(status: 202, delayed: false)
    }

    func testLateCancellationResponseCannotRetireReplacementRunReceipt() async throws {
        try await exercise(status: 202, delayed: true)
    }

    func testReadOnlyObservationCatchesUpRealJournalAndPreservesReceipt() async throws {
        try await exercise(status: 202, delayed: false, mode: "observe")
    }

    func testMatchingTerminalProjectionRetiresReceipt() async throws {
        try await exercise(status: 202, delayed: false, mode: "terminal")
    }

    func testLateOldStreamTerminalCannotRetireReplacementReceipt() async throws {
        try await exercise(status: 202, delayed: false, mode: "late_terminal")
    }

    private func exercise(status: Int, delayed: Bool, mode: String = "cancel") async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("nola-cancel-receipt-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let (server, baseURL) = try startServer(status: status, delayed: delayed, mode: mode)
        defer { if server.isRunning { server.terminate() } }
        let port = try MacApplicationPort(databaseURL: directory.appendingPathComponent("test.db"))
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: "fixture-token", hostID: "fixture-host")
        let service = MacRemoteHostService(configurationStore: configuration)
        if mode == "late_terminal" {
            let original = Task { try await establishReceipt(service, port: port, expectTerminal: true, allowRejectedTerminal: true) }
            try await waitForObservation(baseURL)
            // The source really completed while its execution response was
            // delayed. Import that receipt/seal before admitting a successor.
            _ = try await service.refreshSubmission(conversationID: "conversation", target: service.captureSubmissionTarget(),
                cancel: false, expectedIdentity: nil)
            try await establishReceipt(service, port: port, submissionID: "replacement")
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("release"))
            try await original.value
        } else {
            try await establishReceipt(service, port: port, expectTerminal: mode == "terminal")
        }
        if mode == "observe" {
            let batch = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
                assistantMessageID: "original:assistant", target: service.captureSubmissionTarget())
            XCTAssertEqual(batch.observation?.recovery, .resume_required)
            let cursor = try XCTUnwrap(batch.events.first?.assistantJournalCursor)
            let text = try await MacJournalTextRead.markdown(
                .init(databaseURL: port.databaseURL, cursor: cursor), historicalFallback: "")
            XCTAssertEqual(text, "Running body")
            XCTAssertNil(try port.canonicalRunTerminal(conversationID: "conversation", runID: "original"))
        }
        if mode == "cancel" {
        let intent = try XCTUnwrap(port.remoteSubmissionIntent(conversationID: "conversation"))
        let digest = try XCTUnwrap(intent.request_digest)
        let cancellation = Task { try await service.cancelRun(identity: intent.identity, requestDigest: digest) }
        if delayed {
            try await waitForObservation(baseURL)
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("finish-original"))
            _ = try await service.observeRun(conversationID: "conversation", expectedRunID: "original",
                assistantMessageID: "original:assistant", target: service.captureSubmissionTarget())
            try await establishReceipt(service, port: port, submissionID: "replacement")
            _ = try await URLSession.shared.data(from: baseURL.appendingPathComponent("release"))
        }
        do {
            try await cancellation.value
            XCTAssertEqual(status, 202)
        } catch {
            XCTAssertEqual(status, 503, "Unexpected cancellation error: \(error)")
        }
        }
        // A denied decision traverses the actual receipt check and HTTP path;
        // no test setter, synthetic grant, or private dictionary inspection.
        let request = MacRemotePermissionRequest(id: "request", allowConversation: true, protocolVersion: MacRemotePermissionRequest.supportedProtocolVersion,
            nonce: "nonce", hostID: "fixture-host", clientID: try configuration.clientID(),
            sessionID: "conversation", toolName: "web_search", arguments: "{}",
            callID: "call", reason: "fixture", message: "fixture", expiresAt: nil,
            toolCatalogEvidence: try fixturePermissionEvidence())
        if mode == "terminal" {
            do {
                try await service.resolvePermission(request: request, allow: false)
                XCTFail("A terminal Run must retire its receipt")
            } catch MacRemoteHostError.invalidResponse {
                let counts = try await requestCounts(baseURL)
                XCTAssertEqual(counts["permissions"], 0, "A terminal Run cannot send permission decisions")
            }
        } else {
            try await service.resolvePermission(request: request, allow: false)
            let counts = try await requestCounts(baseURL)
            XCTAssertEqual(counts["permissions"], 1)
            XCTAssertEqual(counts["catalog_reads"], 1)
        }
    }

    private func waitForObservation(_ baseURL: URL) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while ContinuousClock.now < deadline {
            let (data, _) = try await URLSession.shared.data(from: baseURL.appendingPathComponent("observed"))
            if String(decoding: data, as: UTF8.self) == "yes" { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTFail("Expected an in-flight request before replacement")
        throw MacRemoteHostError.invalidResponse
    }

    private func establishReceipt(_ service: MacRemoteHostService, port: MacApplicationPort, submissionID: String = "original",
                                  expectTerminal: Bool = false,
                                  allowRejectedTerminal: Bool = false) async throws {
        try port.ensureFixtureConversation(id: "conversation", title: "Receipt")
        let stream = await service.run(conversationID: "conversation", submissionID: submissionID,
            userMessageID: "\(submissionID):user", assistantMessageID: "\(submissionID):assistant",
            prompt: "fixture", contextComponents: [], modelID: "fixture", modelProviderID: nil,
            modelIntent: nil, memoryEnabled: false, executionBackend: "codex", externalSessionID: nil,
            submission: .init(target: try service.captureSubmissionTarget(),
                ticket: .init(identity: .init(conversationID: "conversation", submissionID: submissionID, generationID: 1)),
                beforeDispatch: {
                    if try port.remoteSubmissionIntent(conversationID: "conversation") == nil {
                        try port.ensureFixtureConversation(id: "conversation", title: "Fixture")
                        _ = try port.prepareRemoteSubmission(.init(conversationID: "conversation",
                            conversationTitle: "Receipt", submissionID: submissionID, sourceHostID: "fixture-host", clientID: try service.captureSubmissionTarget().clientID,
                            userMessageID: "\(submissionID):user", userPartID: "\(submissionID):user:text",
                            userText: "fixture", userContentJSON: nil, assistantMessageID: "\(submissionID):assistant",
                            assistantPartID: "\(submissionID):assistant:text", contextCheckpointID: nil, createdAtMs: 1))
                    }
                }))
        do {
            for try await _ in stream {}
            XCTAssertTrue(expectTerminal, "Fixture deliberately disconnects without a Run terminal")
        } catch MacRemoteHostError.transportInterrupted {
            // A superseded stream may intentionally ignore its old seal and
            // reach EOF without completing the replacement Run. The caller's
            // subsequent permission request verifies that replacement ownership.
            XCTAssertTrue(!expectTerminal || allowRejectedTerminal)
            // The real start path already retained its authenticated tool plan.
        } catch {
            // A late stream with no admitted current Run cannot complete or
            // retire the replacement receipt, even when its source has a seal.
            XCTAssertTrue(allowRejectedTerminal, "Unexpected transport failure: \(error)")
        }
    }

    private func decryptFixtureEnvelope(_ envelope: [String: Any]) throws -> [String: Any] {
        XCTAssertEqual(envelope["v"] as? Int, 1)
        XCTAssertEqual(envelope["alg"] as? String, "AES-256-GCM")
        func decode(_ field: String) throws -> Data {
            let text = try XCTUnwrap(envelope[field] as? String)
            var base64 = text.replacingOccurrences(of: "-", with: "+")
                .replacingOccurrences(of: "_", with: "/")
            base64 += String(repeating: "=", count: (4 - base64.count % 4) % 4)
            return try XCTUnwrap(Data(base64Encoded: base64))
        }
        let nonce = try decode("nonce")
        XCTAssertEqual(nonce.count, 12)
        let tag = try decode("tag")
        XCTAssertEqual(tag.count, 16)
        let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: nonce),
            ciphertext: decode("ciphertext"), tag: tag)
        let key = SymmetricKey(data: Data(SHA256.hash(data: Data("fixture-token".utf8))))
        let plaintext = try AES.GCM.open(box, using: key,
            authenticating: Data("nola-mobile-bridge-v1".utf8))
        return try XCTUnwrap(JSONSerialization.jsonObject(with: plaintext) as? [String: Any])
    }

    private func startServer(status: Int, delayed: Bool, mode: String = "cancel") throws -> (CancellationJournalHTTPServer, URL) {
        let server = try CancellationJournalHTTPServer(status: status, delayed: delayed, mode: mode)
        return (server, try server.start())
    }
}

// Actual source Application owns all journal/admission/seal reads. HTTP gates
// control only delivery timing; they never manufacture completion evidence.
nonisolated final class CancellationJournalHTTPServer: @unchecked Sendable {
    private let listener: NWListener
    private let queue = DispatchQueue(label: "nola.cancellation-journal-http")
    private let status: Int
    private let delayed: Bool
    private let mode: String
    private var streams = 0, cancels = 0, settings = 0, observations = 0, permissionDecisions = 0
    private var observed = false, released = false
    private var pending: [() -> Void] = []
    private var requests: [[String: Any]] = []
    private var decodedStreamRequests: [[String: Any]] = []
    private var modelSettings: [String: Any]?
    private var selectionRevisions: [String: Int64] = [:]
    private var selections: [String: String] = [:]
    private var fixtures: [String: MacAssistantJournalTestFixture] = [:]
    private var preparedPlans: [String: [String: Any]] = [:]
    private var runPlans: [String: [String: Any]] = [:]
    private let rejectionDirectory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    private var rejectionSource: MacApplicationServiceClient?
    private var catalogReads = 0
    private var running = true
    private let key = SymmetricKey(data: Data(SHA256.hash(data: Data("fixture-token".utf8))))
    init(status: Int, delayed: Bool, mode: String) throws {
        self.status = status; self.delayed = delayed; self.mode = mode
        listener = try NWListener(using: .tcp, on: .any)
    }
    var isRunning: Bool { queue.sync { running } }
    func replaceModelSettings(_ value: [String: Any]) { queue.sync { modelSettings = value } }
    func terminate() {
        queue.sync { running = false; listener.cancel(); pending.removeAll(); rejectionSource = nil }
        if FileManager.default.fileExists(atPath: rejectionDirectory.path) {
            try? FileManager.default.removeItem(at: rejectionDirectory)
        }
    }
    func start() throws -> URL {
        let ready = DispatchSemaphore(value: 0)
        listener.stateUpdateHandler = { state in
            if case .ready = state { ready.signal() }
            if case .failed = state { ready.signal() }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { connection.cancel(); return }
            connection.start(queue: self.queue)
            self.read(connection, buffer: Data())
        }
        listener.start(queue: queue)
        guard ready.wait(timeout: .now() + 5) == .success, let port = listener.port,
              let url = URL(string: "http://127.0.0.1:\(port.rawValue)") else {
            listener.cancel(); throw MacRemoteHostError.invalidResponse
        }
        return url
    }
    private func read(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, done, _ in
            guard let self else { connection.cancel(); return }
            var buffer = buffer
            if let data { buffer.append(data) }
            guard buffer.count <= 2 * 1024 * 1024 else { connection.cancel(); return }
            if let split = buffer.range(of: Data("\r\n\r\n".utf8)),
               let header = String(data: buffer[..<split.lowerBound], encoding: .utf8) {
                let lines = header.components(separatedBy: "\r\n")
                let requestLine = lines[0].split(separator: " ")
                let fields = Dictionary(lines.dropFirst().compactMap { line -> (String, String)? in
                    guard let colon = line.firstIndex(of: ":") else { return nil }
                    return (String(line[..<colon]).lowercased(),
                            String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces))
                }, uniquingKeysWith: { _, last in last })
                let count = Int(fields["content-length"] ?? "0") ?? 0
                if requestLine.count >= 2, count >= 0, buffer.count >= split.upperBound + count {
                    self.route(connection, method: String(requestLine[0]), path: String(requestLine[1]),
                               headers: fields, body: buffer.subdata(in: split.upperBound..<(split.upperBound + count)))
                    return
                }
            }
            if done { connection.cancel() } else { self.read(connection, buffer: buffer) }
        }
    }
    private func gate(_ action: @escaping () -> Void) {
        observed = true
        if released { action() } else { pending.append(action) }
    }
    private func route(_ connection: NWConnection, method: String, path: String, headers: [String: String], body: Data) {
        do {
            if method == "GET" {
                switch path {
                case "/requests": try json(connection, value: requests)
                case "/decoded-stream-requests": try json(connection, value: decodedStreamRequests)
                case "/counts": try json(connection, value: ["streams": streams, "cancels": cancels, "settings": settings, "catalog_reads": catalogReads, "permissions": permissionDecisions])
                case "/observed": send(connection, body: Data((observed ? "yes" : "no").utf8))
                case "/finish-original":
                    _ = try fixtures["original"]?.finish()
                    try json(connection, value: [:] as [String: String])
                case "/release":
                    released = true
                    let actions = pending; pending.removeAll()
                    actions.forEach { $0() }
                    try json(connection, value: [:] as [String: String])
                case "/api/mobile/agent/settings":
                    settings += 1
                    let response: () -> Void = { [self] in
                        try? json(connection, value: modelSettings ?? ["host_identity": ["id": "fixture-host"],
                            "network_policy": "allow", "enable_local_dev_tools": false] as [String: Any])
                    }
                    if mode == "settings_gate" || (mode == "settings_gate_after_first" && streams > 0) { gate(response) }
                    else { response() }
                default: send(connection, status: 404, body: Data())
                }
                return
            }
            guard headers["x-nola-mobile-auth"] != nil else { send(connection, status: 401, body: Data()); return }
            let envelope = try JSONSerialization.jsonObject(with: body) as! [String: Any]
            let input = try decrypt(envelope)
            if path.hasSuffix("/submission-frontier") {
                guard input["expected_host_id"] as? String == "fixture-host",
                      let client = input["client_id"] as? String, !client.isEmpty,
                      let conversation = input["conversation_id"] as? String else { throw MacRemoteHostError.invalidResponse }
                // Script a stable source frontier for these transport races.
                // Parent-change/CAS policy has separate paired-source tests;
                // every admission and terminal below still comes from Core.
                let parent = input["assistant_message_id"] as? String
                try encrypted(connection, result: ["protocol_version": 1, "source_host_id": "fixture-host",
                    "client_id": client, "conversation_id": conversation,
                    "configuration_revision": selectionRevisions[conversation] as Any? ?? (parent == nil ? NSNull() : 1 as Any),
                    "parent_message_id": parent as Any? ?? NSNull()])
            } else if path.hasSuffix("/execution-selection") {
                guard input["expected_host_id"] as? String == "fixture-host",
                      let conversation = input["conversation_id"] as? String,
                      let action = input["action"] as? [String: Any], let type = action["type"] as? String else {
                    throw MacRemoteHostError.invalidResponse
                }
                var result: [String: Any]
                if type == "read" {
                    result = selectionRevisions[conversation].map { ["status": "read", "configuration_revision": $0,
                        "model_selection_id": selections[conversation] as Any? ?? NSNull()] } ?? ["status": "not_found"]
                } else {
                    guard ["create", "select"].contains(type), let selection = action["selection"] as? [String: Any] else {
                        throw MacRemoteHostError.invalidResponse
                    }
                    let current = selectionRevisions[conversation]
                    guard type == "create" ? current == nil : (action["expected_revision"] as? NSNumber)?.int64Value == current else {
                        send(connection, status: 409, body: Data()); return
                    }
                    if selection["kind"] as? String == "model" {
                        guard let profile = selection["profile_id"] as? String,
                              let revision = selection["profile_revision"] as? NSNumber,
                              (modelSettings?["model_profiles"] as? [[String: Any]])?.contains(where: {
                                  $0["profile_id"] as? String == profile && ($0["revision"] as? NSNumber) == revision
                              }) == true else { throw MacRemoteHostError.invalidResponse }
                        selections[conversation] = "configured:" + profile
                    } else {
                        guard selection["kind"] as? String == "inherit" else { throw MacRemoteHostError.invalidResponse }
                        selections[conversation] = nil
                    }
                    let next = (current ?? 0) + 1; selectionRevisions[conversation] = next
                    result = ["status": type == "create" ? "created" : "committed", "configuration_revision": next,
                        "model_selection_id": selections[conversation] as Any? ?? NSNull()]
                }
                try encrypted(connection, result: ["source_host_id": "fixture-host", "conversation_id": conversation, "result": result], wrapResult: false)
            } else if path.hasSuffix("/tool-plan/prepare") {
                guard input["expected_host_id"] as? String == "fixture-host", let client = input["client_id"] as? String,
                      let session = input["session_id"] as? String else { throw MacRemoteHostError.invalidResponse }
                let plan = toolPlan(clientID: client, sessionID: session)
                let reference = plan["reference"] as! [String: Any]
                preparedPlans[reference["manifest_id"] as! String] = plan
                try encrypted(connection, result: plan)
            } else if path.hasSuffix("/tool-plan/read") {
                guard input["expected_host_id"] as? String == "fixture-host", input["session_id"] as? String == "conversation",
                      let run = input["expected_run_id"] as? String, let source = fixtures[run],
                      input["assistant_message_id"] as? String == source.assistantMessageID, let plan = runPlans[run],
                      (plan["reference"] as? [String: Any])?["client_id"] as? String == input["client_id"] as? String else {
                    throw MacRemoteHostError.invalidResponse
                }
                catalogReads += 1
                try encrypted(connection, result: plan)
            } else if path.hasSuffix("/stream") {
                streams += 1; requests.append(envelope)
                decodedStreamRequests.append(input)
                let run = input["submission_id"] as? String ?? "original"
                if mode == "room_rejected" {
                    let source = try MacApplicationServiceClient(databaseURL: rejectionDirectory.appendingPathComponent("source.db"))
                    rejectionSource = source
                    let identity: [String: Any] = ["source_host_id": input["expected_host_id"]!, "client_id": input["client_id"]!,
                        "conversation_id": input["conversationId"]!, "submission_id": run,
                        "user_message_id": input["user_message_id"]!, "assistant_message_id": input["assistant_message_id"]!]
                    var semantic = input; semantic.removeValue(forKey: "request_digest")
                    _ = try source.execute(commandData: JSONSerialization.data(withJSONObject: [
                        "type": "register_source_submission", "identity": identity, "request": semantic]))
                    _ = try source.execute(commandData: JSONSerialization.data(withJSONObject: [
                        "type": "reject_source_submission", "identity": identity, "request_digest": input["request_digest"]!,
                        "rejection_code": "dispatch_not_admitted"]))
                    send(connection, status: 409, body: Data("{\"code\":\"host_tool_plan_rejected\"}".utf8))
                    return
                }
                guard let ref = input["tool_plan_ref"] as? [String: Any], let manifest = ref["manifest_id"] as? String,
                      let plan = preparedPlans[manifest], NSDictionary(dictionary: ref).isEqual(to: plan["reference"] as! [String: Any]) else {
                    throw MacRemoteHostError.invalidResponse
                }
                runPlans[run] = plan
                var semantic = input; semantic.removeValue(forKey: "request_digest")
                let source = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: run,
                    assistantMessageID: input["assistant_message_id"] as? String ?? "\(run):assistant",
                    userText: "fixture", userMessageID: input["user_message_id"] as? String ?? "\(run):user",
                    clientID: input["client_id"] as? String ?? "", semanticRequest: semantic)
                guard input["request_digest"] as? String == source.requestDigest else { throw MacRemoteHostError.invalidResponse }
                fixtures[run] = source
                let receiptFrame: [String: Any] = ["type":"submission_receipt", "receipt":try object(source.submissionReceipt())]
                if mode.hasPrefix("partial") || mode == "observe" || mode == "terminal" || (mode == "late_terminal" && streams == 1) {
                    let cursor = try source.append("Running body")
                    var identity = try object(cursor.identity)
                    var live: [String: Any] = ["type": "assistant_journal_committed_v1",
                        "session_id": "conversation", "source_host_id": "fixture-host", "through": try object(cursor)]
                    if mode == "partial_wrong_run" { identity["run_id"] = "other-run" }
                    if mode == "partial_wrong_session" {
                        identity["conversation_id"] = "other-conversation"; live["session_id"] = "other-conversation"
                    }
                    var through = try object(cursor); through["identity"] = identity; live["through"] = through
                    if mode == "partial_wrong_host" { live["source_host_id"] = "wrong-host" }
                    var frames: [[String: Any]] = [receiptFrame] + (mode.hasPrefix("partial") ? [live] : [])
                    if ["partial_terminal_late", "partial_terminal_foreign_late", "terminal", "late_terminal"].contains(mode) {
                        let seal = try source.finish()
                        frames.append(["type": "assistant_run_terminal_committed_v1", "session_id": "conversation",
                            "source_host_id": "fixture-host", "run_id": run, "seal_digest": seal.seal_digest])
                        if mode == "partial_terminal_late" { frames.append(live) }
                        if mode == "partial_terminal_foreign_late" {
                            var foreign = live
                            foreign["source_host_id"] = "wrong-host"
                            frames.append(foreign)
                        }
                    }
                    let bytes = try frames.reduce(into: Data()) { output, frame in
                        output.append(Data("data: ".utf8))
                        output.append(try JSONSerialization.data(withJSONObject: frame))
                        output.append(Data("\n\n".utf8))
                    }
                    let response: () -> Void = { [self] in send(connection, body: bytes, content: "text/event-stream") }
                    if mode == "late_terminal" && streams == 1 { gate(response) } else { response() }
                } else {
                    var bytes = Data("data: ".utf8)
                    bytes.append(try JSONSerialization.data(withJSONObject: receiptFrame)); bytes.append(Data("\n\n".utf8))
                    send(connection, body: bytes, content: "text/event-stream")
                }
            } else if path.hasSuffix("/journal") {
                var command = input
                let operation = command.removeValue(forKey: "operation") as? String ?? ""
                let id = (command["through"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? (command["at"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? ((command["snapshot"] as? [String: Any])?["at"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? (command["seal"] as? [String: Any])?["identity"] as? [String: Any]
                    ?? command["identity"] as? [String: Any]
                let run = command["run_id"] as? String ?? id?["run_id"] as? String ?? "original"
                guard let source = fixtures[run] else {
                    if operation == "terminal_seal" { try encrypted(connection, result: NSNull()); return }
                    throw MacRemoteHostError.invalidResponse
                }
                let operations = ["admission_head": "local_run_message_admission_head", "admissions": "read_run_message_admissions",
                    "context_usage": "local_run_context_usage",
                    "read": "read_assistant_journal", "heads": "local_run_assistant_journal_heads",
                    "open_snapshot": "open_assistant_journal_snapshot", "record_chunk": "read_assistant_journal_record_chunk",
                    "release_snapshot": "release_assistant_journal_snapshot", "terminal_seal": "local_assistant_run_terminal_seal",
                    "terminal_manifest": "read_local_assistant_run_terminal_manifest"]
                guard let type = operations[operation] else { throw MacRemoteHostError.invalidResponse }
                command["type"] = type
                let result = try JSONSerialization.jsonObject(with: source.source.execute(
                    commandData: JSONSerialization.data(withJSONObject: command)), options: [.fragmentsAllowed])
                try encrypted(connection, result: result)
            } else if path.hasSuffix("/submission") {
                if mode == "room_rejected", let source = rejectionSource {
                    let receipt = try JSONSerialization.jsonObject(with: source.execute(commandData: JSONSerialization.data(withJSONObject: [
                        "type": "read_source_submission_receipt", "identity": input["identity"]!,
                        "request_digest": input["request_digest"]!])), options: [.fragmentsAllowed])
                    try encrypted(connection, result: receipt)
                    return
                }
                let identity = input["identity"] as? [String: Any]
                guard let run = identity?["submission_id"] as? String, let source = fixtures[run] else {
                    try encrypted(connection, result: NSNull()); return
                }
                guard input["request_digest"] as? String == source.requestDigest else { throw MacRemoteHostError.invalidResponse }
                try encrypted(connection, result: object(source.submissionReceipt()))
            } else if path.hasSuffix("/observe") {
                observations += 1
                let run = input["expected_run_id"] as? String ?? "original"
                if mode == "cold_observe", fixtures[run] == nil {
                    fixtures[run] = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: run,
                        assistantMessageID: "\(run):assistant", clientID: input["client_id"] as? String ?? "",
                        contextUsage: .init(used: 7_500, window: 32_768, promptLimit: 28_672))
                    runPlans[run] = toolPlan(clientID: input["client_id"] as? String ?? "", sessionID: "conversation")
                }
                guard input["expected_host_id"] as? String == "fixture-host", let source = fixtures[run] else {
                    send(connection, status: 409, body: Data()); return
                }
                let admission = try JSONSerialization.jsonObject(with: source.source.execute(commandData:
                    JSONSerialization.data(withJSONObject: ["type":"local_run_message_admission_head",
                        "conversation_id":"conversation", "run_id":run]))) as! [String: Any]
                let identity = admission["identity"] as! [String: Any]
                if mode == "cold_observe" && observations > 1 { _ = try source.finish("Cold final body") }
                let seal = try JSONDecoder().decode(MacAssistantRunTerminalSeal?.self, from: source.source.execute(commandData:
                    JSONSerialization.data(withJSONObject: ["type":"local_assistant_run_terminal_seal",
                        "conversation_id":"conversation", "run_id":run])))
                let permissions: [[String: Any]] = mode == "cold_observe" && seal == nil ? [[
                    "type":"permission_request", "allow_conversation": true, "protocol_version": 1, "id":"cold-request", "request_id":"cold-request",
                    "nonce":"original-cold-nonce", "host_id":"fixture-host", "client_id":input["client_id"]!,
                    "session_id":"conversation", "canonical_run_id":run, "call_id":"call", "tool_name":"web_search",
                    "expires_at":Int64(Date().timeIntervalSince1970) + 300, "safe_review_text":"Review this search",
                    "tool_catalog_evidence": ["catalog_revision": 2, "tool": ["tool_id": "web_search", "route": "agent_host",
                        "schema": ["type": "function", "function": ["name": "web_search", "parameters": ["type": "object"]]]]],
                    "permission_instructions":[]]] : []
                let encodedSeal: Any = try seal.map { try object($0) as Any } ?? NSNull()
                let contextUsage: Any = try source.committedContextUsage().map { try object($0) as Any } ?? NSNull()
                let generation: Any = permissions.isEmpty ? NSNull() : "1" as Any
                try encrypted(connection, result: ["protocol_version":1, "conversation_id":"conversation",
                    "client_id":input["client_id"]!, "run_id":run,
                    "assistant_message_id":identity["assistant_message_id"]!, "status":seal == nil ? "running" : "sealed",
                    "worker":["state":permissions.isEmpty ? "absent" : "running", "generation":generation],
                    "recovery":seal != nil ? "terminal" : permissions.isEmpty ? "resume_required" : "observe",
                    "admission":admission, "terminal_seal":encodedSeal,
                    "context_usage":contextUsage,
                    "permissions":permissions, "can_submit":seal != nil,
                    "submission_receipt":try object(source.submissionReceipt())] as [String: Any])
            } else if path.hasSuffix("/cancel") {
                guard let identity = input["identity"] as? [String: Any],
                      let run = identity["submission_id"] as? String, let source = fixtures[run],
                      input["request_digest"] as? String == source.requestDigest,
                      identity["assistant_message_id"] as? String == source.assistantMessageID else {
                    send(connection, status: 409, body: Data()); return
                }
                cancels += 1; observed = true
                let response: () -> Void = { [self] in send(connection, status: status, body: Data("{}".utf8)) }
                if delayed { gate(response) } else { response() }
            } else if path.hasSuffix("/permission") { permissionDecisions += 1; requests.append(envelope); try json(connection, value: [:] as [String: String]) }
            else { send(connection, status: 404, body: Data()) }
        } catch { send(connection, status: 500, body: Data(String(describing: error).utf8)) }
    }
    private func toolPlan(clientID: String, sessionID: String) -> [String: Any] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        return ["schema_version": 1,
                "reference": ["manifest_id": UUID().uuidString, "host_id": "fixture-host", "client_id": clientID,
                              "session_id": sessionID, "catalog_revision": String(repeating: "a", count: 64)],
                "issued_at_ms": now, "expires_at_ms": now + 60_000, "tools": []]
    }

    private func object<T: Encodable>(_ value: T) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as! [String: Any]
    }
    private func decrypt(_ envelope: [String: Any]) throws -> [String: Any] {
        let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: decode(envelope["nonce"] as! String)),
            ciphertext: decode(envelope["ciphertext"] as! String), tag: decode(envelope["tag"] as! String))
        return try JSONSerialization.jsonObject(with: AES.GCM.open(box, using: key,
            authenticating: Data("nola-mobile-bridge-v1".utf8))) as! [String: Any]
    }
    private func encrypted(_ connection: NWConnection, result: Any, wrapResult: Bool = true) throws {
        let value: Any = wrapResult ? ["source_host_id": "fixture-host", "result": result] : result
        let sealed = try AES.GCM.seal(JSONSerialization.data(withJSONObject: value), using: key,
            authenticating: Data("nola-mobile-bridge-v1".utf8))
        try json(connection, value: ["type": "encrypted", "v": 1, "alg": "AES-256-GCM",
            "nonce": encode(Data(sealed.nonce)), "ciphertext": encode(sealed.ciphertext), "tag": encode(sealed.tag)])
    }
    private func encode(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
    private func decode(_ string: String) throws -> Data {
        let base64 = string.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        guard let data = Data(base64Encoded: base64 + String(repeating: "=", count: (4-base64.count%4)%4)) else {
            throw MacRemoteHostError.invalidResponse
        }
        return data
    }
    private func json(_ connection: NWConnection, value: Any) throws {
        send(connection, body: try JSONSerialization.data(withJSONObject: value))
    }
    private func send(_ connection: NWConnection, status: Int = 200, body: Data, content: String = "application/json") {
        var response = Data("HTTP/1.1 \(status) Fixture\r\nContent-Type: \(content)\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n".utf8)
        response.append(body)
        connection.send(content: response, completion: .contentProcessed { _ in connection.cancel() })
    }
}
