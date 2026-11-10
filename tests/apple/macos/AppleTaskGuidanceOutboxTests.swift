// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

private actor GuidanceReceiptTransport {
    enum Mode { case lostResponse, absentThenRetry, unavailable, confirmed, foreign, selectiveFailure }
    var mode: Mode
    var requests: [(String, Data)] = []
    init(_ mode: Mode) { self.mode = mode }
    func send(_ path: String, _ data: Data) throws -> Data {
        requests.append((path, data))
        let input = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        let taskID = input["taskId"] as? String
        if mode == .unavailable || (mode == .selectiveFailure && taskID == "task-a") { throw URLError(.networkConnectionLost) }
        if requests.count == 1 && (mode == .lostResponse || mode == .absentThenRetry) {
            throw URLError(.networkConnectionLost)
        }
        let guidance: Any = mode == .absentThenRetry && path.hasSuffix("/receipt") ? NSNull() : [
            "task_id": input["taskId"]!, "message_id": input["messageId"]!,
            "owner_session_id": input["conversationId"]!, "content": input["message"]!,
            "submitted_at_ms": input["submittedAtMs"]!, "status": "submitted"
        ]
        return try JSONSerialization.data(withJSONObject: [
            "hostId": mode == .foreign ? "other-host" : input["hostId"]!,
            "conversationId": input["conversationId"]!, "taskId": input["taskId"]!,
            "messageId": input["messageId"]!, "guidance": guidance
        ])
    }
    func paths() -> [String] { requests.map(\.0) }
    func bodies() -> [Data] { requests.map(\.1) }
}

final class AppleTaskGuidanceOutboxTests: XCTestCase {
    func testCoreTransportChecksExactRunBeforeSubmittingFixedIntent() throws {
        let payload = try JSONSerialization.data(withJSONObject: [
            "hostId": "host", "conversationId": "conversation", "parentRunId": "run",
            "taskId": "task", "messageId": "fixed-message", "submittedAtMs": 42,
            "message": "Continue carefully"
        ])
        var commands: [[String: Any]] = []
        _ = try AppleTaskGuidanceCoreTransport.exchange(path: "chat/agent/v2/task/message",
            payload: payload, conversationId: "conversation", parentRunId: "run", parentTurnId: "turn") { data in
                commands.append(try JSONSerialization.jsonObject(with: data) as! [String: Any])
                return Data("null".utf8)
            }
        XCTAssertEqual(commands.map { $0["type"] as? String }, ["load_task_guidance_receipt", "submit_task_guidance"])
        XCTAssertEqual(commands[0]["parent_run_id"] as? String, "run")
        XCTAssertEqual(commands[1]["parent_turn_id"] as? String, "turn")
        XCTAssertEqual(commands[1]["message_id"] as? String, "fixed-message")
        XCTAssertEqual(commands[1]["submitted_at_ms"] as? Int, 42)
        var failedCommands = 0
        XCTAssertThrowsError(try AppleTaskGuidanceCoreTransport.exchange(path: "chat/agent/v2/task/message",
            payload: payload, conversationId: "conversation", parentRunId: "run", parentTurnId: "turn") { _ in
                failedCommands += 1
                throw URLError(.cannotParseResponse)
            })
        XCTAssertEqual(failedCommands, 1, "Unconfirmed source authority must never reach submission")
        commands.removeAll()
        _ = try AppleTaskGuidanceCoreTransport.exchange(path: "chat/agent/v2/task/message",
            payload: payload, conversationId: "conversation", parentRunId: "run", parentTurnId: "turn") { data in
                commands.append(try JSONSerialization.jsonObject(with: data) as! [String: Any])
                return Data("{}".utf8)
            }
        XCTAssertEqual(commands.count, 1, "A recorded intent must be returned without another submission")
    }

    func testLostResponseQueriesReceiptWithoutSecondInstruction() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let transport = GuidanceReceiptTransport(.lostResponse)
        let outbox = AppleTaskGuidanceOutbox(directory: directory)
        let receipt = try await outbox.submit(hostId: "host", conversationId: "conversation",
            parentRunId: "source-run", taskId: "task", message: "  Continue carefully  ", transport: { path, data in try await transport.send(path, data) })
        XCTAssertEqual(receipt.content, "Continue carefully")
        let paths = await transport.paths()
        XCTAssertEqual(paths, ["chat/agent/v2/task/message", "chat/agent/v2/task/message/receipt"])
        let bodies = await transport.bodies()
        XCTAssertEqual(bodies.first, bodies.last)
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
    }

    func testOnlyAuthoritativeNullPermitsSameIntentResubmission() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let transport = GuidanceReceiptTransport(.absentThenRetry)
        _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
            conversationId: "conversation", parentRunId: "run", taskId: "task", message: "指导", transport: { path, data in try await transport.send(path, data) })
        let bodies = await transport.bodies()
        XCTAssertEqual(bodies.count, 3)
        XCTAssertTrue(bodies.allSatisfy { $0 == bodies[0] })
    }

    func testColdRetryKeepsCompleteIntentAndRejectsForeignReceipt() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let unavailable = GuidanceReceiptTransport(.unavailable)
        do {
            _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
                conversationId: "conversation", parentRunId: "run", taskId: "task", message: "Keep scope", transport: { path, data in try await unavailable.send(path, data) })
            XCTFail("unknown outcome must remain pending")
        } catch {}
        let foreign = GuidanceReceiptTransport(.foreign)
        do {
            _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
                conversationId: "conversation", parentRunId: "run", taskId: "task", message: "Keep scope", transport: { path, data in try await foreign.send(path, data) })
            XCTFail("foreign owner must not clear intent")
        } catch {}
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path).count, 1)
        let confirmed = GuidanceReceiptTransport(.confirmed)
        try await AppleTaskGuidanceOutbox(directory: directory).recover(hostId: "host",
            conversationId: "conversation", parentRunId: "run", transport: { path, data in try await confirmed.send(path, data) })
        let original = await unavailable.bodies()
        let recovered = await confirmed.bodies()
        XCTAssertEqual(original.first, recovered.first)
        let paths = await confirmed.paths()
        XCTAssertEqual(paths, ["chat/agent/v2/task/message/receipt"])
    }
    func testOneUnresolvedOrMalformedIntentDoesNotStarveOtherTasks() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let unavailable = GuidanceReceiptTransport(.unavailable)
        for task in ["task-a", "task-b"] {
            do {
                _ = try await AppleTaskGuidanceOutbox(directory: directory).submit(hostId: "host",
                    conversationId: "conversation", parentRunId: "run", taskId: task, message: task,
                    transport: { path, data in try await unavailable.send(path, data) })
                XCTFail("fixture should retain pending intent")
            } catch {}
        }
        try Data("invalid".utf8).write(to: directory.appendingPathComponent("000-corrupt.json"))
        let transport = GuidanceReceiptTransport(.selectiveFailure)
        do {
            try await AppleTaskGuidanceOutbox(directory: directory).recover(hostId: "host",
                conversationId: "conversation", parentRunId: "run",
                transport: { path, data in try await transport.send(path, data) })
            XCTFail("unresolved intents must still be reported")
        } catch AppleTaskGuidanceOutbox.Failure.recoveryIncomplete(let count) {
            XCTAssertEqual(count, 2)
        }
        let bodies = await transport.bodies()
        XCTAssertTrue(try bodies.contains { data in
            (try JSONSerialization.jsonObject(with: data) as? [String: Any])?["taskId"] as? String == "task-b"
        })
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: directory.path).count, 2)
    }

}
