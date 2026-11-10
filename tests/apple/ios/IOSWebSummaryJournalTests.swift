// SPDX-License-Identifier: Apache-2.0
#if DEBUG
import XCTest
@testable import Nola

final class IOSWebSummaryJournalTests: XCTestCase {
    @MainActor
    func testSealedSummaryIncludesEverySegmentInCanonicalMessageOrder() throws {
        let root = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "root")
        let second = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "run", assistant_message_id: "segment")
        let firstCursor = IOSAssistantJournalCursor(identity: root, sequence: "3", record_digest: String(repeating: "a", count: 64))
        let secondCursor = IOSAssistantJournalCursor(identity: second, sequence: "2", record_digest: String(repeating: "b", count: 64))
        XCTAssertEqual(try WebAgentProvider.summaryAssistantHeads([(3, secondCursor), (1, firstCursor)],
            root: root, expectedCount: "2"), [firstCursor, secondCursor])
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(1, firstCursor), (3, secondCursor)],
            root: root, expectedCount: "1"))
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(1, firstCursor), (3, firstCursor)],
            root: root, expectedCount: "2"))
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(3, secondCursor)], root: root, expectedCount: "1"))
        let foreign = IOSAssistantJournalCursor(identity: .init(source_store_epoch: "epoch", conversation_id: "conversation",
            run_id: "foreign", assistant_message_id: "segment"), sequence: "2", record_digest: secondCursor.record_digest)
        XCTAssertThrowsError(try WebAgentProvider.summaryAssistantHeads([(1, firstCursor), (3, foreign)],
            root: root, expectedCount: "2"))
    }

    func testSummaryEmitsOnlyOneSealedPublicSourceResult() async throws {
        try await exercise(.sealed)
    }
    func testMissingAdmissionFrameRecoversOriginalReceiptWithoutResubmission() async throws {
        try await exercise(.lostReceipt)
    }
    func testNonterminalEOFNeverReturnsPartialSuccess() async throws {
        try await exercise(.unsealed)
    }
    func testForeignReceiptNeverBecomesSummaryContent() async throws {
        try await exercise(.foreignReceipt)
    }

    private func exercise(_ mode: SummarySource.Mode) async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let source = try SummarySource(directory: directory, mode: mode)
        let provider = await WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "fixture", expectedHostId: IOSCanonicalChatJournalFixture.sourceHostID, clientId: "summary-client")) {
            try await source.response($0)
        }
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let input = ModelInput(dateRange: .init(start: "2026-01-01", end: "2026-01-02"), candidates: [],
            userPrefs: ["message":"Exercise the canonical Host transcript", "enable_thinking":"true",
                "host_conversation_id":UUID().uuidString, "submission_id":identity.submissionID,
                "user_message_id":identity.userMessageID, "assistant_message_id":identity.assistantMessageID])
        var chunks: [String] = []
        do {
            let stream = await provider.generateSummaryStream(input: input)
            for try await chunk in stream { chunks.append(chunk) }
            XCTAssertTrue(mode == .sealed || mode == .lostReceipt)
            XCTAssertEqual(chunks.count, 1)
            XCTAssertTrue(chunks.first?.contains("canonical Host stream") == true)
            XCTAssertTrue(chunks.first?.contains("summary-source-marker") == true)
            XCTAssertFalse(chunks.first?.contains("<think>") == true)
        } catch {
            XCTAssertTrue(mode == .unsealed || mode == .foreignReceipt, "Unexpected error: \(error)")
            XCTAssertTrue(chunks.isEmpty, "No partial text is a completed String summary")
        }
        let counts = await source.counts()
        XCTAssertEqual(counts.streams, 1, "Recovery must never resubmit")
        if mode == .sealed || mode == .lostReceipt { XCTAssertGreaterThan(counts.ranges, 0) }
        if mode == .foreignReceipt { XCTAssertEqual(counts.ranges, 0) }
    }
}

private actor SummarySource {
    enum Mode: Sendable { case sealed, lostReceipt, unsealed, foreignReceipt }
    private let source: IOSCanonicalChatJournalFixture
    private let intentStore: IOSApplicationRuntime
    private let mode: Mode
    private var run: IOSCanonicalChatJournalFixture.Run?
    private var streams = 0
    private var ranges = 0
    init(directory: URL, mode: Mode) throws {
        source = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.db"),
            orderingMarker: "summary-source-marker")
        intentStore = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("hash-owner.db"))
        self.mode = mode
    }
    func counts() -> (streams: Int, ranges: Int) { (streams, ranges) }
    private func value(_ object: Any) throws -> AgentCoreJSONValue {
        try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONSerialization.data(withJSONObject: object, options: [.fragmentsAllowed]))
    }
    private func object(_ value: AgentCoreJSONValue) throws -> Any {
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(value), options: [.fragmentsAllowed])
    }
    func response(_ request: URLRequest) throws -> WebAgentTransportResponse {
        let path = request.url!.path
        if path.hasSuffix("/submission-frontier") || path.hasSuffix("/agent/settings")
            || path.hasSuffix("/agent/tool-plan/prepare") {
            return try source.response(for: request, token: "fixture")
        }
        let envelope = try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as! [String: Any]
        let payload = try WebAgentProvider.decryptMobileJSON(envelope, token: "fixture")
        if request.url!.path.hasSuffix("/submission"), payload["operation"] as? String == "prepare" {
            let identity = payload["identity"] as! [String: Any]
            let semantic = payload["request"] as! [String: Any]
            // Use Application's actual Rust request binding to compute the
            // digest. No source registration or execution occurs in prepare.
            var input = identity
            input["conversation_title"] = "Summary hash"
            input["user_part_id"] = "\(identity["user_message_id"]!):text"
            input["assistant_part_id"] = "\(identity["assistant_message_id"]!):text"
            input["user_text"] = "Exercise the canonical Host transcript"
            input["created_at_ms"] = 1
            _ = try intentStore.executeAgentCommand(value(["type":"prepare_remote_submission", "input":input]))
            let bound = try intentStore.executeAgentCommand(value(["type":"bind_remote_submission_request",
                "identity":identity, "request":semantic]))
            let fields = try object(bound) as! [String: Any]
            return try encrypted(request, result: ["identity":identity, "request_digest":fields["request_digest"]!])
        }
        if request.url!.path.hasSuffix("/stream") {
            streams += 1
            let active = try source.begin(payload); run = active
            var receipt = try object(source.submissionReceipt(active)) as! [String: Any]
            if mode == .foreignReceipt {
                var identity = receipt["identity"] as! [String: Any]
                identity["submission_id"] = "foreign-run"; receipt["identity"] = identity
            }
            var frames: [AgentCoreJSONValue] = mode == .lostReceipt ? [] : [try value(["type":"submission_receipt", "receipt":receipt])]
            for stage in IOSCanonicalChatJournalFixture.Stage.allCases {
                if mode == .unsealed && stage == .terminal { break }
                frames += try source.advance(active, stage: stage)
            }
            var bytes = Data()
            for frame in frames {
                bytes.append(Data("data: ".utf8)); bytes.append(try JSONEncoder().encode(frame)); bytes.append(Data("\n\n".utf8))
            }
            return transport(request, data: bytes, contentType: "text/event-stream")
        }
        if request.url!.path.hasSuffix("/submission") {
            return try encrypted(request, result: object(source.submissionReceipt(XCTUnwrap(run))))
        }
        XCTAssertTrue(request.url!.path.hasSuffix("/journal"), "Unexpected summary request: \(path)")
        if payload["operation"] as? String == "range" { ranges += 1 }
        return try encrypted(request, result: object(source.journalResult(payload)))
    }
    private func encrypted(_ request: URLRequest, result: Any) throws -> WebAgentTransportResponse {
        var envelope = try WebAgentProvider.encryptMobileJSON(["source_host_id":IOSCanonicalChatJournalFixture.sourceHostID,
            "result":result], token: "fixture")
        envelope["type"] = "encrypted"
        return transport(request, data: try JSONSerialization.data(withJSONObject: envelope), contentType: "application/json")
    }
    private func transport(_ request: URLRequest, data: Data, contentType: String) -> WebAgentTransportResponse {
        .init(bytes: AsyncThrowingStream { stream in data.forEach { stream.yield($0) }; stream.finish() },
            response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil,
                headerFields: ["Content-Type":contentType])!)
    }
}
#endif
