import XCTest
@testable import Nola

@MainActor final class AgentCorePartProjectionTests: XCTestCase {
    func testJournalReplacementDoesNotDuplicatePartIdentity() async throws {
        let before = try await readJournalPresentation([text("text-1", "Hello", final: false)])
        let after = try await readJournalPresentation([text("text-1", "Hello world", final: true)], sequence: 2)
        XCTAssertEqual(before.text, "Hello")
        XCTAssertEqual(after.text, "Hello world")
        XCTAssertEqual(after.parts.map(\.partID), ["text-1"])
    }

    func testRetiredPriorRoundDoesNotResurfaceInNewIncompleteAnswer() async throws {
        let before = try await readJournalPresentation([text("old", "Old answer", final: true)])
        let after = try await readJournalPresentation([text("new", "New", final: false)], sequence: 2)
        XCTAssertEqual(before.text, "Old answer")
        XCTAssertEqual(after.text, "New")
        XCTAssertEqual(after.parts.map(\.partID), ["new"])
    }

    func testNewReasoningRoundHasIndependentIdentityAndCompletionState() async throws {
        let content = try await readJournalPresentation([["kind":"reasoning","part_id":"reason-new","revision":2,
            "state":"running","title":"Thinking","summary":"New reasoning"]])
        XCTAssertEqual(content.reasoning, "New reasoning")
        guard case .reasoning(let reasoning)? = content.parts.first else { return XCTFail("Missing reasoning") }
        XCTAssertEqual(reasoning.state, .running)
    }

    func testReasoningAndToolJournalStateRemainIndependent() async throws {
        let content = try await readJournalPresentation([
            ["kind":"reasoning","part_id":"reason","revision":1,"state":"completed","title":"Thinking","summary":"Inspecting"],
            tool("completed", output:"2 files")
        ])
        XCTAssertEqual(content.reasoning, "Inspecting")
        guard case .tool(let projected) = content.parts[1] else { return XCTFail("Missing tool") }
        XCTAssertEqual(projected.toolCallID, "call-1")
        XCTAssertEqual(projected.state, .completed)
        XCTAssertEqual(projected.title, "Read file")
        XCTAssertEqual(projected.summary, "Read README.md")
        XCTAssertEqual(projected.outputPreview, "2 files")
    }

    func testFailedAndCancelledJournalToolsRemainDistinct() async throws {
        for (state, expected) in [("failed", IOSConversationProjectionV1.ActivityState.failed), ("cancelled", .cancelled)] {
            let content = try await readJournalPresentation([tool(state, output:state)])
            guard case .tool(let projected)? = content.parts.first else { return XCTFail("Missing tool") }
            XCTAssertEqual(projected.state, expected)
        }
    }

    private func text(_ id:String,_ value:String,final:Bool)->[String:Any] {
        ["kind":"text","part_id":id,"revision":1,"markdown":value,"is_final":final]
    }
    private func tool(_ state:String,output:String)->[String:Any] {
        ["kind":"tool","execution_source":["kind":"model"],"part_id":"tool-1","revision":1,"tool_call_id":"call-1","capability_id":"read_file",
         "title":"Read file","summary":"Read README.md","state":state,"input":["path":"README.md"],
         "origin":["kind":"builtin"],"artifacts":[],"affected_resources":[],"output_preview":output]
    }

    func testWebProviderParsesJournalReceiptWithoutTraceOrInlineBody() throws {
        let object: [String: Any] = ["type":"assistant_journal_committed_v1", "session_id":"conversation",
            "source_host_id":"host", "through":["identity":["source_store_epoch":"epoch",
                "conversation_id":"conversation", "run_id":"run", "assistant_message_id":"assistant"],
                "sequence":"7", "record_digest":String(repeating: "a", count: 64)]]
        guard case .assistantJournalCommitted(let raw)? = WebAgentProvider.parseSSEObject(object) else {
            return XCTFail("Expected committed journal notification")
        }
        let notification = try JSONDecoder().decode(IOSAssistantJournalNotification.self, from: JSONEncoder().encode(raw))
        XCTAssertEqual(notification.through.identity.run_id, "run")
        XCTAssertEqual(notification.through.sequence, "7")
        XCTAssertNil(object["part"])
        XCTAssertNil(object["trace"])
    }

    func testWebProviderParsesCanonicalCoreTurnTerminalWithoutTraceRequirement() throws {
        let object: [String: Any] = [
            "protocol_version": 1,
            "type": "turn_failed",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "cursor": 9,
            "revision": 9,
            "reason": "model request failed",
        ]
        guard case .coreTurnTerminal(let terminal, let trace)? = WebAgentProvider.parseSSEObject(object) else {
            return XCTFail("Expected Core Turn terminal")
        }

        XCTAssertNil(trace)
        XCTAssertEqual(terminal.status, .failed)
        XCTAssertEqual(terminal.reason, "model request failed")
    }

    func testWebProviderRejectsNestedCoreTurnTerminalOnHostTransport() {
        let object: [String: Any] = [
            "session_id": "session-1",
            "turn_id": "turn-1",
            "cursor": ["sequence": 9],
            "revision": 9,
            "event": [
                "type": "turn_completed",
                "turn_id": "turn-1",
            ],
        ]

        guard case .error(let message)? = WebAgentProvider.parseSSEObject(object) else {
            return XCTFail("Expected strict Host protocol error")
        }
        XCTAssertEqual(message, "Agent Host protocol violation: event is missing Core Trace")
    }

}
