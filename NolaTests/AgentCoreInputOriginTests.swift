import XCTest
@testable import Nola

final class AgentCoreInputOriginTests: XCTestCase {
    func testInteractiveInputUsesUserSubmissionByDefault() throws {
        let value = try AgentCoreInputSubmission.make(
            input: "Hello",
            messageId: "message-1",
            submissionId: "submission-1"
        )
        guard case .object(let object) = value else {
            return XCTFail("Expected object submission")
        }
        XCTAssertEqual(object["type"], .string("user_input"))
        XCTAssertEqual(object["delivery"], .string("current_turn"))
        XCTAssertNil(object["origin"])
        XCTAssertEqual(object["submission_id"], .string("submission-1"))
        XCTAssertEqual(messageContent(in: object), "Hello")
    }

    func testScheduledActionUsesTypedAutomatedSubmission() throws {
        let component = AgentCoreContextComponent(
            kind: .document,
            id: "reference-1",
            content: .string("{\"value\":1}"),
            source: .init(platform: .ios)
        )
        let value = try AgentCoreInputSubmission.make(
            input: "Run the scheduled summary",
            origin: .scheduledAction,
            contextComponents: [component],
            messageId: "message-2",
            submissionId: "submission-2"
        )
        guard case .object(let object) = value else {
            return XCTFail("Expected object submission")
        }
        XCTAssertEqual(object["type"], .string("automated_input"))
        XCTAssertEqual(object["origin"], .string("scheduled_action"))
        XCTAssertNil(object["delivery"])
        XCTAssertEqual(object["turn_context_platform"], .string("ios"))
        guard case .array(let components)? = object["turn_context_components"] else {
            return XCTFail("Expected typed turn context components")
        }
        XCTAssertEqual(components.count, 1)
        XCTAssertEqual(object["submission_id"], .string("submission-2"))
        XCTAssertEqual(messageContent(in: object), "Run the scheduled summary")
    }

    func testUnknownOriginCannotStartATurn() {
        XCTAssertThrowsError(try AgentCoreInputSubmission.make(input: "Unknown", origin: .unknown)) { error in
            XCTAssertEqual(error as? AgentCoreInputSubmissionError, .unsupportedOrigin(.unknown))
        }
    }

    private func messageContent(in object: [String: AgentCoreJSONValue]) -> String? {
        guard case .array(let items)? = object["items"],
              case .object(let message)? = items.first,
              case .string(let content)? = message["content"] else { return nil }
        return content
    }
}
