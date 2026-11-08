import XCTest
@testable import Nola

final class ScheduledURLActionAgentRequestTests: XCTestCase {
    func testConfiguredPromptAndFetchedDataRemainSeparateTypedInputs() throws {
        let observedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let request = try ScheduledURLActionAgentRequest.make(
            actionId: "action-1",
            sourceName: "Status Feed",
            sourceURL: "https://example.com/feed.json",
            userPrompt: "Report only meaningful changes.",
            sourceDescription: "Fallback description",
            dedupeKey: "scheduled:1",
            observedAt: observedAt,
            dateRange: .init(start: "2023-11-14T00:00:00Z", end: "2023-11-15T00:00:00Z"),
            candidates: [.init(
                type: "url",
                title: "Release 2.0",
                name: nil,
                date: nil,
                start: nil,
                due: nil,
                relation: nil,
                notesTag: nil,
                durationMin: nil
            )]
        )

        XCTAssertEqual(request.prompt, "Report only meaningful changes.")
        XCTAssertEqual(request.referenceComponent.kind, .document)
        XCTAssertEqual(request.referenceComponent.id, "action-1")
        XCTAssertEqual(request.referenceComponent.source?.authorization, "available")
        XCTAssertEqual(request.referenceComponent.source?.uri, "https://example.com/feed.json")
        XCTAssertEqual(request.referenceComponent.source?.observedAt, ISO8601DateFormatter().string(from: observedAt))
        guard case .string(let content) = request.referenceComponent.content else {
            return XCTFail("Expected JSON reference content")
        }
        let decoded = try JSONDecoder().decode(ModelInput.self, from: Data(content.utf8))
        XCTAssertEqual(decoded.candidates.first?.title, "Release 2.0")
        XCTAssertFalse(content.contains("Report only meaningful changes."))
    }
}
