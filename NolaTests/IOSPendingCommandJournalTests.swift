import XCTest
import Darwin
@testable import Nola

final class IOSPendingCommandJournalTests: XCTestCase {
    func testExpertHostNamespaceDoesNotConsumeRendererIntentsAndScopesIndependentOwners() throws {
        let database = try directory().appendingPathComponent("product.db")
        func operation(parent: String, child: String) -> [String: Any] {
            ["kind": "admit", "input": ["conversation_id": parent, "selected_participants": [["context_conversation_id": child]]],
             "context": ["actor": ["kind": "local_user", "id": "actor"], "request_host_id": NSNull()]]
        }
        func hostEnvelope(_ id: String, operation: [String: Any]) -> [String: Any] {
            ["request_id": id, "client_id": "native-expert-host", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
             "command": ["type": "host_expert_template_operation", "arguments": ["operation": operation]]]
        }
        let ordinary = ApplePendingCommandJournal(databaseURL: database, namespace: .application,
            validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let source = try ordinary.beginTransaction()
        try source.save(["ordinary": envelope("ordinary")]); source.close()
        let host = ApplePendingCommandJournal(databaseURL: database, namespace: .expertHost,
            validateOwners: AppleExpertHostIntentScope.validateJournalOwners)
        let privateScope = try host.beginTransaction()
        XCTAssertTrue(try privateScope.load().isEmpty, "A renderer intent can never become a trusted Host operation")
        let a = operation(parent: "parent-a", child: "child-a"), b = operation(parent: "parent-b", child: "child-b")
        try privateScope.save(["a": hostEnvelope("a", operation: a), "b": hostEnvelope("b", operation: b)])
        XCTAssertEqual(try privateScope.load().count, 2)
        XCTAssertTrue(AppleExpertHostIntentScope.intersects(a, operation(parent: "other-parent", child: "child-a")))
        XCTAssertFalse(AppleExpertHostIntentScope.intersects(a, b))
        XCTAssertTrue(AppleExpertHostIntentScope.intersects(a, ["kind": "unknown"]))
        XCTAssertThrowsError(try privateScope.save(["a": hostEnvelope("a", operation: a),
            "overlap": hostEnvelope("overlap", operation: operation(parent: "other-parent", child: "child-a"))]))
        XCTAssertEqual(try privateScope.load().count, 2, "A rejected overlap must not rewrite prior intents")
        privateScope.close()
        let ordinaryAgain = try ordinary.beginTransaction()
        defer { ordinaryAgain.close() }
        XCTAssertNotNil(try ordinaryAgain.load()["ordinary"])
    }

    private func directory() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true,
                                               attributes: [.posixPermissions: 0o700])
        addTeardownBlock { try FileManager.default.removeItem(at: root) }
        return root
    }
    private func envelope(_ id: String = "original", content: String = "original payload") -> [String: Any] {
        ["request_id": id, "client_id": "ios-application", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
         "expected_revision": 7, "command": ["type": "fixture", "arguments": ["content": content]]]
    }

    private func contextEnvelope(_ id: String, owner: String, type: String = "reduce_and_accept_host_response") -> [String: Any] {
        ["request_id": id, "client_id": "ios-application", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
         "command": ["type": type, "arguments": ["input": ["conversation_id": owner]]]]
    }

    func testIndependentContextsSurviveReopenAndOnlyExactReceiptOwnerIsRetired() throws {
        let database = try directory().appendingPathComponent("product.db")
        let a = contextEnvelope("a", owner: "context-a")
        let b = contextEnvelope("b", owner: "context-b")
        let initial = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        try initial.save(["a": a, "b": b])
        initial.close()
        let reopened = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { reopened.close() }
        var pending = try reopened.load()
        XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("c", owner: "context-c"), pending: pending, resolve: { $0 }))
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("a-next", owner: "context-a"), pending: pending, resolve: { $0 }), "a")
        pending.removeValue(forKey: "b")
        try reopened.save(pending)
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: reopened.load()["a"]!, options: [.sortedKeys]),
                       try JSONSerialization.data(withJSONObject: a, options: [.sortedKeys]))
    }

    func testTypedOwnershipRejectsMalformedKnownCommandsAndKeepsUnknownCommandsGlobal() throws {
        let a = contextEnvelope("a", owner: "a")
        XCTAssertEqual(try IOSPendingCommandOwner.decode(contextEnvelope("x", owner: "a", type: "future_conversation_command")), .global)
        var malformed = a
        malformed["command"] = ["type": "reduce_and_accept_host_response", "arguments": ["conversation_id": "a"]]
        XCTAssertThrowsError(try IOSPendingCommandOwner.decode(malformed))
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: envelope("global"), pending: ["a": a], resolve: { $0 }), "a")
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: a, pending: ["global": envelope("global")], resolve: { $0 }), "global")
    }

    func testJournalSnapshotLeasesAreContextWritesNotGlobalHistoryBarriers() throws {
        let cursor: [String: Any] = ["identity": ["conversation_id": "context-b", "run_id": "run-b",
            "assistant_message_id": "assistant-b", "source_store_epoch": "epoch"],
            "sequence": "1", "record_digest": String(repeating: "a", count: 64)]
        let commands: [(String, [String: Any])] = [
            ("open_assistant_journal_snapshot", ["at": cursor]),
            ("release_assistant_journal_snapshot", ["snapshot": ["snapshot_id": "snapshot-b", "at": cursor]])
        ]
        for (type, arguments) in commands {
            let request: [String: Any] = ["request_id": "snapshot", "client_id": "ios", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
                "command": ["type": type, "arguments": arguments]]
            XCTAssertFalse(IOSApplicationRuntime.isReadOnlyCommand(type: type, arguments: arguments))
            XCTAssertEqual(try IOSPendingCommandOwner.decode(request), .conversation("context-b"))
            XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: request,
                pending: ["a": contextEnvelope("a", owner: "context-a")], resolve: { $0 }))
            XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: request,
                pending: ["b": contextEnvelope("b", owner: "context-b")], resolve: { $0 }), "b")
        }
    }

    func testQueuedAdmissionsSerializeWithTheirConversationOnly() throws {
        for type in ["begin_queued_user_turn", "promote_queued_assistant_turn"] {
            let queued = contextEnvelope("queued", owner: "context-a", type: type)
            XCTAssertEqual(try IOSPendingCommandOwner.decode(queued), .conversation("context-a"))
            XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(
                for: contextEnvelope("b", owner: "context-b"), pending: ["queued": queued], resolve: { $0 }))
            XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(
                for: contextEnvelope("a", owner: "context-a"), pending: ["queued": queued], resolve: { $0 }), "queued")
        }
    }

    func testAutomationAdmissionUsesItsConversationOwnerWithoutBlockingAnotherContext() throws {
        let automation = contextEnvelope("automation", owner: "context-a", type: "begin_automation_assistant_turn")
        XCTAssertEqual(try IOSPendingCommandOwner.decode(automation), .conversation("context-a"))
        XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(
            for: contextEnvelope("b", owner: "context-b"), pending: ["automation": automation], resolve: { $0 }))
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(
            for: contextEnvelope("a", owner: "context-a"), pending: ["automation": automation], resolve: { $0 }), "automation")
    }

    func testRemoteJournalConsumerWritesRetainExactContextOrdering() throws {
        let identity: [String: Any] = ["conversation_id": "remote-a"]
        let cursor: [String: Any] = ["identity": identity]
        let record: [String: Any] = ["header": cursor]
        let cases: [(String, [String: Any])] = [
            ("accept_remote_run_context_usage", ["record": cursor]),
            ("receive_run_message_admissions", ["page": ["through": cursor]]),
            ("receive_assistant_journal_record", ["record": record]),
            ("stage_assistant_journal_content", ["input": ["record": record]]),
            ("begin_receive_assistant_run_terminal", ["seal": cursor]),
            ("stage_assistant_run_terminal_manifest", ["page": ["seal": cursor]]),
            ("commit_received_assistant_run_terminal", ["seal": cursor])
        ]
        for (type, arguments) in cases {
            let pending: [String: Any] = ["request_id": "remote", "client_id": "ios", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
                "command": ["type": type, "arguments": arguments]]
            XCTAssertEqual(try IOSPendingCommandOwner.decode(pending), .conversation("remote-a"), type)
            XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("b", owner: "remote-b"),
                pending: ["remote": pending], resolve: { $0 }), type)
            XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: contextEnvelope("a", owner: "remote-a"),
                pending: ["remote": pending], resolve: { $0 }), "remote", type)
        }
    }

    func testRunOnlyCommandUsesCanonicalConversationOwnerRatherThanIndependentRunLane() throws {
        let response = contextEnvelope("response", owner: "context-a")
        let claim: [String: Any] = ["request_id": "claim", "client_id": "ios", "protocol_version": IOSApplicationRuntime.expectedServiceProtocol,
            "command": ["type": "claim_committed_host_requests", "arguments": ["input": ["run_id": "run-a"]]]]
        XCTAssertEqual(try IOSPendingCommandOwner.decode(claim), .run("run-a"))
        let resolve: (IOSPendingCommandOwner) throws -> IOSPendingCommandOwner = {
            $0 == .run("run-a") ? .conversation("context-a") : $0
        }
        XCTAssertEqual(try IOSPendingCommandOwner.blockingRequest(for: claim, pending: ["response": response], resolve: resolve), "response")
        XCTAssertNil(try IOSPendingCommandOwner.blockingRequest(for: claim,
            pending: ["other": contextEnvelope("other", owner: "context-b")], resolve: resolve))
    }

    func testReopenPreservesCompleteEnvelopeAndExplicitRetirement() throws {
        let database = try directory().appendingPathComponent("product.db")
        let original = envelope(content: "中文 / quote \" backslash \\ newline\n")
        let first = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        try first.save(["original": original])
        first.close()
        let reopened = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        let pending = try reopened.load()
        XCTAssertEqual(try JSONSerialization.data(withJSONObject: pending["original"]!, options: [.sortedKeys]),
                       try JSONSerialization.data(withJSONObject: original, options: [.sortedKeys]))
        try reopened.save([:]) // Runtime may do this only after a validated terminal receipt.
        reopened.close()
        let retired = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { retired.close() }
        XCTAssertTrue(try retired.load().isEmpty)
        var info = stat()
        XCTAssertEqual(lstat(database.appendingPathExtension("pending-commands.json").path, &info), 0)
        XCTAssertEqual(info.st_mode & 0o777, 0o600)
    }

    func testBoundUsesSerializedFrameAndRejectsForeignMapIdentity() throws {
        let database = try directory().appendingPathComponent("product.db")
        let transaction = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        defer { transaction.close() }
        // Near the legal frame ceiling must not be truncated to an arbitrary
        // small intent limit, including commands carrying source/artifact data.
        let large = envelope(content: String(repeating: "x", count: ApplePendingCommandJournal.maximumEnvelopeBytes - 1024))
        try transaction.save(["original": large])
        XCTAssertEqual(try transaction.load().count, 1)
        XCTAssertThrowsError(try transaction.save(["foreign": envelope()]))
        XCTAssertThrowsError(try transaction.save(["original": envelope(), "second": envelope("second")]))
        XCTAssertThrowsError(try transaction.save(["original": envelope(content:
            String(repeating: "x", count: ApplePendingCommandJournal.maximumEnvelopeBytes))]))
        XCTAssertEqual(try transaction.load().count, 1, "invalid replacement leaves the previous intent intact")
    }

    func testRejectsSymlinkNonPrivateAndMalformedJournalWithoutTreatingAsEmpty() throws {
        for kind in ["symlink", "public", "null", "empty"] {
            let root = try directory()
            let database = root.appendingPathComponent("product.db")
            let journal = database.appendingPathExtension("pending-commands.json")
            if kind == "symlink" {
                let target = root.appendingPathComponent("target")
                try Data("{}".utf8).write(to: target)
                try FileManager.default.createSymbolicLink(at: journal, withDestinationURL: target)
            } else {
                let content = kind == "null" ? "null" : kind == "empty" ? "" : "{}"
                try Data(content.utf8).write(to: journal)
                try FileManager.default.setAttributes([.posixPermissions: kind == "public" ? 0o644 : 0o600],
                                                     ofItemAtPath: journal.path)
            }
            XCTAssertThrowsError(try {
                let scope = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
                defer { scope.close() }
                _ = try scope.load()
            }(), kind)
        }
    }

    func testSeparateInstancesCannotReadBetweenLoadSendAndSave() throws {
        let database = try directory().appendingPathComponent("product.db")
        let first = try ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners).beginTransaction()
        let secondJournal = ApplePendingCommandJournal(databaseURL: database, namespace: .application, validateOwners: IOSPendingCommandOwner.validateJournalOwners)
        let attempted = DispatchSemaphore(value: 0)
        let acquired = DispatchSemaphore(value: 0)
        let finished = expectation(description: "second transaction observes original intent")
        DispatchQueue.global().async {
            attempted.signal()
            do {
                let second = try secondJournal.beginTransaction()
                defer { second.close() }
                acquired.signal()
                XCTAssertEqual(try second.load().keys.sorted(), ["original"])
            } catch { XCTFail("Journal transaction failed: \(error)") }
            finished.fulfill()
        }
        XCTAssertEqual(attempted.wait(timeout: .now() + 1), .success)
        XCTAssertEqual(acquired.wait(timeout: .now() + 0.05), .timedOut)
        try first.save(["original": envelope()])
        first.close()
        wait(for: [finished], timeout: 2)
    }
}
