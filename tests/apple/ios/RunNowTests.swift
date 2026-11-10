// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements RunNowTests logic in the project module.
// - Interfaces:
//   - RunNowTests
// - APIs:
//   - none
//
//
//  RunNowTests.swift
//  NolaTests
//
//  Tests for the client-side "Run Now" flow:
//    1. ScheduledActionRunRecord field serialization
//    2. LocalJSONStorage — persist and reload records with new fields
//    3. Run record status semantics — submitted (partial) vs failure (failed)
//

import XCTest
@testable import Nola
import Foundation

final class RunNowTests: XCTestCase {

    @MainActor
    func testCanonicalAndBuiltInRunFiltersUseTheSameClassificationForRowsAndCounts() {
        typealias Status = LocalJSONStorage.ScheduledActionRunRecord.Status
        let groups: [(ScheduledRunFilter, [Status])] = [
            (.success, [.answered, .actionCompleted, .success]),
            (.issues, [.incomplete, .blocked, .failed, .partial]),
            (.skipped, [.skipped]),
            (.cancelled, [.cancelled]),
        ]
        let all = groups.flatMap { $0.1 }
        XCTAssertEqual(all.filter { ScheduledRunFilter.all.matches($0) }.count, 9)
        for (filter, expected) in groups {
            let visible = all.filter { filter.matches($0) }
            XCTAssertEqual(visible.map(\.rawValue), expected.map(\.rawValue))
            XCTAssertEqual(visible.count, expected.count)
        }
        XCTAssertFalse(ScheduledRunFilter.success.matches(.blocked))
        XCTAssertFalse(ScheduledRunFilter.success.matches(.cancelled))
        XCTAssertFalse(ScheduledRunFilter.issues.matches(.answered))
    }

    @MainActor
    func testCanonicalRunStatusPresentationsRemainDistinct() {
        typealias Status = LocalJSONStorage.ScheduledActionRunRecord.Status
        typealias Tone = ScheduledActionRunText.StatusTone
        let cases: [(Status, String, Tone)] = [
            (.answered, "text.bubble", .neutral),
            (.actionCompleted, "checkmark.circle.fill", .success),
            (.incomplete, "exclamationmark.circle", .attention),
            (.blocked, "hand.raised", .attention),
            (.cancelled, "stop.circle", .neutral),
            (.failed, "xmark.circle.fill", .failure),
        ]
        var titles = Set<String>()
        for (status, symbol, tone) in cases {
            let presentation = ScheduledActionRunText.statusPresentation(status)
            XCTAssertEqual(presentation.symbol, symbol, status.rawValue)
            XCTAssertEqual(presentation.tone, tone, status.rawValue)
            XCTAssertFalse(presentation.title.isEmpty)
            titles.insert(presentation.title)
        }
        XCTAssertEqual(titles.count, cases.count, "Do not collapse answer, cancellation, or blocked into success/failure")
    }

    // MARK: - Helpers

    private func makeRecord(
        status: LocalJSONStorage.ScheduledActionRunRecord.Status = .success,
        trigger: String? = nil,
        executionPath: String? = nil,
        serverResultId: String? = nil,
        fetchedContentBytes: Int? = nil,
        eventsCount: Int? = nil,
        tokensUsed: Int? = nil,
        reason: String? = nil,
        errorMessage: String? = nil,
        actionId: String = UUID().uuidString
    ) -> LocalJSONStorage.ScheduledActionRunRecord {
        LocalJSONStorage.ScheduledActionRunRecord(
            startedAt: Date(timeIntervalSinceReferenceDate: 0),
            finishedAt: Date(timeIntervalSinceReferenceDate: 5),
            isBackground: false,
            actionType: "scheduled_url_report",
            actionId: actionId,
            actionName: "Test Source",
            actionUrl: "https://example.com",
            dedupeKey: "test:\(UUID().uuidString)",
            status: status,
            reason: reason,
            failureReason: nil,
            resultEventId: nil,
            notificationSent: nil,
            notificationError: nil,
            modelUsed: nil,
            tokensUsed: tokensUsed,
            processingDurationSec: nil,
            details: nil,
            errorMessage: errorMessage,
            eventsCount: eventsCount,
            resultJournalId: nil,
            nextEligibleAt: nil,
            shouldRetry: nil,
            retryCount: nil,
            serverResultId: serverResultId,
            trigger: trigger,
            executionPath: executionPath,
            fetchedContentBytes: fetchedContentBytes
        )
    }

    // MARK: - 1. ScheduledActionRunRecord Codable round-trip

    func testNewFieldsRoundTrip() throws {
        let original = makeRecord(
            status: .success,
            trigger: "manual",
            executionPath: "server",
            serverResultId: "abc-123",
            fetchedContentBytes: 42_000,
            eventsCount: 3,
            tokensUsed: 512
        )

        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(original)

        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let decoded = try decoder.decode(LocalJSONStorage.ScheduledActionRunRecord.self, from: data)

        XCTAssertEqual(decoded.trigger, "manual")
        XCTAssertEqual(decoded.executionPath, "server")
        XCTAssertEqual(decoded.serverResultId, "abc-123")
        XCTAssertEqual(decoded.fetchedContentBytes, 42_000)
        XCTAssertEqual(decoded.eventsCount, 3)
        XCTAssertEqual(decoded.tokensUsed, 512)
        XCTAssertEqual(decoded.status, .success)
    }

    func testNilNewFieldsRoundTrip() throws {
        // All new fields absent — should decode as nil without errors.
        let original = makeRecord(
            status: .failed,
            trigger: nil,
            executionPath: nil,
            serverResultId: nil,
            fetchedContentBytes: nil
        )

        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        let data = try encoder.encode(original)

        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let decoded = try decoder.decode(LocalJSONStorage.ScheduledActionRunRecord.self, from: data)

        XCTAssertNil(decoded.trigger)
        XCTAssertNil(decoded.executionPath)
        XCTAssertNil(decoded.serverResultId)
        XCTAssertNil(decoded.fetchedContentBytes)
    }

    // MARK: - 2. Run record status semantics

    func testPartialStatusSemantics() {
        // .partial means "submitted but awaiting push" — should NOT be surfaced as error.
        let record = makeRecord(
            status: .partial,
            reason: "server_processing",
            errorMessage: "任务已提交到服务器，执行完毕后将通过推送通知告知结果"
        )
        XCTAssertEqual(record.status, .partial)
        XCTAssertEqual(record.reason, "server_processing")
        // Must have an explanatory message, not an error message.
        XCTAssertFalse(record.errorMessage?.isEmpty ?? true)
    }

    func testFailedStatusSemantics() {
        // .failed means a real error that the user should act on.
        let record = makeRecord(
            status: .failed,
            reason: "server_network_lost",
            errorMessage: "网络连接中断，请确认网络后重试"
        )
        XCTAssertEqual(record.status, .failed)
        XCTAssertEqual(record.reason, "server_network_lost")
    }

    // MARK: - 3. Trigger and executionPath label correctness

    func testTriggerLabels() {
        let manualRecord = makeRecord(trigger: "manual", executionPath: "server")
        let pushRecord = makeRecord(trigger: "push", executionPath: "server")
        let bgRecord = makeRecord(trigger: "background", executionPath: "local")

        XCTAssertEqual(manualRecord.trigger, "manual")
        XCTAssertEqual(pushRecord.trigger, "push")
        XCTAssertEqual(bgRecord.trigger, "background")
        XCTAssertEqual(bgRecord.executionPath, "local")
    }
}
