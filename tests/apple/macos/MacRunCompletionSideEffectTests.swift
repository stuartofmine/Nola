import Foundation
import XCTest
@testable import Nola

private actor CompletionDeliveryRecorder {
    private var values: [MacRunNotificationService.AgentCompletionNotification] = []
    func append(_ value: MacRunNotificationService.AgentCompletionNotification) { values.append(value) }
    func snapshot() -> [MacRunNotificationService.AgentCompletionNotification] { values }
}

@MainActor
final class MacRunCompletionSideEffectTests: XCTestCase {
    func testActivityPersistsAllSixCanonicalOutcomesWithoutCollapsingCancellation() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = MacActivityStore(baseDirectory: directory)
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            store.recordConversationRun(title: status.rawValue, conversationID: status.rawValue,
                execution: .thisMac, status: status, startedAt: Date(timeIntervalSince1970: 1), output: "retained partial")
        }
        XCTAssertNil(store.lastError)
        let reopened = MacActivityStore(baseDirectory: directory)
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            let entry = try XCTUnwrap(reopened.entries.first { $0.conversationID == status.rawValue })
            XCTAssertEqual(entry.status, .init(result: status))
            XCTAssertEqual(entry.outputPreview, "retained partial")
        }
        XCTAssertEqual(reopened.entries.count, 6)
        XCTAssertEqual(reopened.entries.first { $0.status == .cancelled }?.detail, String(localized: "Cancelled"))
    }

    func testNotificationPreservesFiveBusinessResultsButNeverCancellation() async {
        let recorder = CompletionDeliveryRecorder()
        let service = MacRunNotificationService(completionNotificationsEnabled: { true },
            deliverAgentCompletion: { await recorder.append($0) })
        for status in [MacRunCompletionStatus.answered, .actionCompleted, .incomplete, .blocked, .failed, .cancelled] {
            await service.notifyAgentCompletion(conversationTitle: status.rawValue,
                conversationID: status.rawValue, status: status)
        }
        let deliveries = await recorder.snapshot()
        XCTAssertEqual(deliveries.map(\.conversationID), ["answered", "action_completed", "incomplete", "blocked", "failed"])
        XCTAssertEqual(deliveries.map(\.title), [String(localized: "Nola answered"), String(localized: "Nola completed the action"), String(localized: "Nola has unfinished work"), String(localized: "Nola is blocked"), String(localized: "Nola encountered an error")])
        XCTAssertEqual(deliveries.map(\.body), ["answered", "action_completed", "incomplete", "blocked", "failed"])
    }

    func testDisabledNotificationsDoNotInvokeDelivery() async {
        let recorder = CompletionDeliveryRecorder()
        let service = MacRunNotificationService(completionNotificationsEnabled: { false },
            deliverAgentCompletion: { await recorder.append($0) })
        await service.notifyAgentCompletion(conversationTitle: "Done", conversationID: "c", status: .answered)
        let deliveries = await recorder.snapshot()
        XCTAssertTrue(deliveries.isEmpty)
    }

    func testAttentionIsNotCancellationOrExecutionSettlement() {
        for status in [MacActivityEntry.Status.answered, .actionCompleted, .delegationAccepted, .cancelled, .executionFinished] {
            XCTAssertFalse(status.needsAttention)
        }
        for status in [MacActivityEntry.Status.incomplete, .blocked, .failed] {
            XCTAssertTrue(status.needsAttention)
        }
        XCTAssertNotEqual(MacRunCompletionStatus.answered.title, MacRunCompletionStatus.actionCompleted.title)
    }

    func testSuccessIndicatorRequiresCanonicalBusinessSuccess() {
        for status in [MacActivityEntry.Status.answered, .actionCompleted] {
            XCTAssertTrue(status.isSuccessful, status.rawValue)
        }
        for status in [MacActivityEntry.Status.incomplete, .blocked, .failed, .delegationAccepted, .cancelled, .executionFinished] {
            XCTAssertFalse(status.isSuccessful, status.rawValue)
        }
        XCTAssertFalse(MacActivityEntry.Status.cancelled.needsAttention)
        XCTAssertFalse(MacActivityEntry.Status.executionFinished.needsAttention)
        // Lack of an attention warning is not proof of successful execution.
        let missing: MacActivityEntry.Status? = nil
        XCTAssertFalse(missing?.isSuccessful == true)
    }
}
