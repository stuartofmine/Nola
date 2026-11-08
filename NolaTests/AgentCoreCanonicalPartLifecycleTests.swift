// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class AgentCoreCanonicalPartLifecycleTests: XCTestCase {
    func testRealApplicationLifecyclePublishesJournalToProductionReader() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("journal-lifecycle-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        let source = try IOSCanonicalChatJournalFixture(databaseURL: directory.appendingPathComponent("source.sqlite"), orderingMarker: "verified")
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let run = try source.begin(["conversationId":"journal-lifecycle","submission_id":identity.submissionID,
            "user_message_id":identity.userMessageID,"assistant_message_id":identity.assistantMessageID,
            "client_id":"fixture-client","expected_host_id":IOSCanonicalChatJournalFixture.sourceHostID,
            "messages":[["role":"user","content":"Read the project and answer"]]])
        let reader = IOSAssistantJournalReader { command in
            var fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(command)) as? [String:Any])
            let type = try XCTUnwrap(fields.removeValue(forKey:"type") as? String)
            let operations = ["open_assistant_journal_snapshot":"open_snapshot",
                "read_assistant_journal_parts":"parts","read_assistant_journal_part_changes":"changes",
                "read_assistant_journal_range":"range","release_assistant_journal_snapshot":"release_snapshot"]
            fields["operation"] = try XCTUnwrap(operations[type])
            return try source.journalResult(fields)
        }
        var latest: IOSJournalMessageContent?
        var sawRunningTool = false, sawCompletedTool = false, sawReasoning = false, sawTerminal = false
        for stage in IOSCanonicalChatJournalFixture.Stage.allCases {
            for event in try source.advance(run, stage:stage) {
                guard case .object(let fields) = event, case .string(let type)? = fields["type"] else { continue }
                if type == "assistant_run_terminal_committed_v1" { sawTerminal = true; continue }
                guard type == "assistant_journal_committed_v1", let through = fields["through"] else { continue }
                let cursor = try JSONDecoder().decode(IOSAssistantJournalCursor.self, from: JSONEncoder().encode(through))
                let content = try await reader.read(at:cursor)
                XCTAssertEqual(content.cursor.identity.assistant_message_id, identity.assistantMessageID)
                let duplicate = try await reader.read(at:cursor)
                XCTAssertEqual(duplicate, content)
                sawReasoning = sawReasoning || !content.reasoning.isEmpty
                for case .tool(let tool) in content.parts {
                    sawRunningTool = sawRunningTool || tool.state == .running
                    sawCompletedTool = sawCompletedTool || tool.state == .completed
                }
                latest = content
            }
        }
        XCTAssertTrue(sawReasoning); XCTAssertTrue(sawRunningTool); XCTAssertTrue(sawCompletedTool); XCTAssertTrue(sawTerminal)
        XCTAssertEqual(latest?.text, run.answer)
        XCTAssertEqual(Set(latest?.parts.map(\.partID) ?? []).count, latest?.parts.count)
    }

    // This fixture records kernel events, not a supported client transport.
    // Preserve the independent ownership/order/late-response regression while
    // the test above exercises the only production publication path.
    func testKernelLifecycleFixtureRetainsExactOwnersAndRejectsLateResponse() throws {
        let url = try XCTUnwrap(Bundle(for:Self.self).url(forResource:"canonical_part_lifecycle_v1",withExtension:"json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with:Data(contentsOf:url)) as? [String:Any])
        let envelopes = try XCTUnwrap(fixture["event_envelopes"] as? [[String:Any]])
        var order:[String] = [], revisions:[String:Int] = [:], terminalTypes=Set<String>()
        var activeTurn:String?
        var sawCompleted = false, sawCancelledPartial = false, sawSucceededTool = false
        for envelope in envelopes {
            let event = try XCTUnwrap(envelope["event"] as? [String:Any])
            let type = try XCTUnwrap(event["type"] as? String)
            if type == "turn_started" { activeTurn = event["turn_id"] as? String }
            if type == "part_updated" {
                XCTAssertEqual(event["turn_id"] as? String, activeTurn)
                let part = try XCTUnwrap(event["part"] as? [String:Any])
                let id = try XCTUnwrap(part["id"] as? String)
                if !order.contains(id) { order.append(id) }
                revisions[id] = event["projection_revision"] as? Int
                sawCompleted = sawCompleted || (part["text"] as? String == "## Verified Result\n\nThe evidence confirms that Core owns stable Part identity, lifecycle, and citations.")
                sawCancelledPartial = sawCancelledPartial || (part["text"] as? String == "This partial answer must remain attached to the cancelled Turn.")
                sawSucceededTool = sawSucceededTool || (part["kind"] as? String == "tool" && part["state"] as? String == "completed")
            }
            if ["turn_completed","turn_failed","turn_cancelled"].contains(type) {
                let raw = try JSONDecoder().decode(AgentCoreJSONValue.self,from:JSONSerialization.data(withJSONObject:envelope))
                let terminal = try XCTUnwrap(AgentCoreTurnTerminalEnvelope.decodeCore(from:raw))
                XCTAssertEqual(terminal.turnId,activeTurn); terminalTypes.insert(type)
            }
        }
        XCTAssertEqual(order,fixture["part_order"] as? [String])
        XCTAssertEqual(revisions,fixture["latest_projection_revision_by_part"] as? [String:Int])
        XCTAssertTrue(sawCompleted); XCTAssertTrue(sawCancelledPartial); XCTAssertTrue(sawSucceededTool)
        XCTAssertTrue(terminalTypes.isSuperset(of:["turn_completed","turn_cancelled"]))
        let probe = try XCTUnwrap(fixture["late_response_probe"] as? [String:Any])
        let outcome = try XCTUnwrap(probe["outcome"] as? [String:Any])
        XCTAssertEqual(outcome["emitted_event_count"] as? Int,0)
        XCTAssertEqual(probe["snapshot_unchanged"] as? Bool,true)
        XCTAssertFalse(String(decoding:try JSONSerialization.data(withJSONObject:envelopes),as:UTF8.self).contains("LATE CONTENT MUST NOT APPEAR"))
    }
}
