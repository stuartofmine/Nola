import XCTest
@testable import Nola

final class ConversationKnowledgeTests: XCTestCase {
    func testAutosaveSnapshotKeepsItsOriginalConversationAndValues() {
        let conversationID = UUID()
        var knowledgePoints = "Knowledge A"
        var wiki = "Wiki A"
        let snapshot = ConversationKnowledgeSaveSnapshot(
            conversationID: conversationID,
            knowledgePoints: knowledgePoints,
            wiki: wiki
        )

        knowledgePoints = "Knowledge B"
        wiki = "Wiki B"

        XCTAssertEqual(snapshot.conversationID, conversationID)
        XCTAssertEqual(snapshot.knowledgePoints, "Knowledge A")
        XCTAssertEqual(snapshot.wiki, "Wiki A")
    }

    func testKnowledgeAndWikiRoundTrip() throws {
        let conversation = Conversation(
            title: "Knowledge",
            sessionMemory: "Current task",
            knowledgePoints: "- Preserve original evidence",
            wiki: "## Architecture\nLocal-first"
        )
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601

        let encoded = try encoder.encode(conversation)
        let decoded = try decoder.decode(Conversation.self, from: encoded)
        XCTAssertEqual(decoded.knowledgePoints, "- Preserve original evidence")
        XCTAssertEqual(decoded.wiki, "## Architecture\nLocal-first")

    }
}
