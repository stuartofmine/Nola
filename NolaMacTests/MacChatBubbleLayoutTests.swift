import XCTest
@testable import Nola

final class MacChatBubbleLayoutTests: XCTestCase {
    func testProseHasIndependentReadingMeasureWithoutShrinkingWorkspace() {
        XCTAssertEqual(MacDesignTokens.Layout.chatProseMaxWidth, 720)
        XCTAssertLessThanOrEqual(MacDesignTokens.Layout.chatProseMaxWidth,
                          MacChatBubbleLayout.transcriptWidth(availableWidth: 1_440))
        XCTAssertGreaterThanOrEqual(MacDesignTokens.Control.iconButton, 32)
    }
    func testTranscriptWidthUsesAvailableSpaceWithoutBreakingCompactLayout() {
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 520), 472, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 900), 720, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 1_440), 720, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.transcriptWidth(availableWidth: 2_200), 720, accuracy: 0.1)
    }

    func testTranscriptColumnGrowsContinuouslyAndNeverExceedsAvailableWidth() {
        var previous: CGFloat = 0
        for width in stride(from: 0, through: 2_400, by: 1) {
            let column = MacChatBubbleLayout.transcriptWidth(availableWidth: CGFloat(width))
            XCTAssertGreaterThanOrEqual(column, previous)
            XCTAssertLessThanOrEqual(column, max(0, CGFloat(width) - 48))
            XCTAssertLessThanOrEqual(column, 720)
            previous = column
        }
    }

    func testUserBubbleRemainsVisuallyDistinctFromAssistantContent() {
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 0), 0)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 200), 200)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 472), 415.36, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 1_200), 620, accuracy: 0.1)
        XCTAssertEqual(MacChatBubbleLayout.userBubbleWidth(transcriptWidth: 2_048), 620, accuracy: 0.1)
    }
}
