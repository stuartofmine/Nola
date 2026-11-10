import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacCanonicalAutomationRunEndToEndTests: XCTestCase {
    func testLeaseOwnershipAndUnboundExitCannotManufactureResultAfterColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-automation-run-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let databaseURL = root.appendingPathComponent("nola.db")
        let now: Int64 = 1_800_000_000_000
        let input = MacCanonicalAutomationRun(
            id: "run-first",
            automationID: "automation-first",
            definitionRevision: 1,
            occurrenceKey: "scheduled:1800000000",
            trigger: "schedule",
            status: "queued",
            attempt: 0,
            workerID: nil,
            leaseToken: nil,
            leaseUntilMs: nil,
            scheduledForMs: now,
            startedAtMs: nil,
            finishedAtMs: nil,
            nextRetryAtMs: nil,
            resultJSON: nil,
            errorJSON: nil,
            createdAtMs: now,
            updatedAtMs: now
        )
        let encoded = try JSONEncoder().encode(input)
        XCTAssertEqual(try JSONDecoder().decode(MacCanonicalAutomationRun.self, from: encoded), input)
        var missingRevision = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        missingRevision.removeValue(forKey: "definition_revision")
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalAutomationRun.self,
            from: JSONSerialization.data(withJSONObject: missingRevision)))

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            let definition = try store.upsertAutomation(.init(id: "automation-first", name: "Fixture", scheduleFormat: "nola.mac.automation.v1", enabled: true, revision: 0,
                executionSpec: .agent(.init(prompt: "Fixture", executionScope: .localWorkspace(root: root.path), permissionProfile: "cautious", modelSelection: "use_current_default_at_admission", contextRefs: [], skillRefs: [])),
                metadataJSON: "{}", createdAtMs: now, updatedAtMs: now, deletedAtMs: nil))
            let first = try store.enqueueAutomationRun(input)
            XCTAssertEqual(first.definitionRevision, definition.revision)
            let duplicate = try store.enqueueAutomationRun(MacCanonicalAutomationRun(
                id: "run-duplicate",
                automationID: input.automationID,
                definitionRevision: input.definitionRevision,
                occurrenceKey: input.occurrenceKey,
                trigger: input.trigger,
                status: input.status,
                attempt: 0,
                workerID: nil,
                leaseToken: nil,
                leaseUntilMs: nil,
                scheduledForMs: now,
                startedAtMs: nil,
                finishedAtMs: nil,
                nextRetryAtMs: nil,
                resultJSON: nil,
                errorJSON: nil,
                createdAtMs: now,
                updatedAtMs: now
            ))
            XCTAssertEqual(first.id, duplicate.id)
            XCTAssertTrue(try store.claimAutomationRun(
                id: first.id,
                workerID: "mac-test-a",
                leaseToken: "lease-a",
                nowMs: now,
                leaseUntilMs: now + 60_000
            ))
            XCTAssertFalse(try store.claimAutomationRun(
                id: first.id,
                workerID: "mac-test-b",
                leaseToken: "lease-b",
                nowMs: now + 1,
                leaseUntilMs: now + 60_001
            ))
            let observed = try store.observeAutomationExecution(occurrenceID: first.id)
            XCTAssertNil(observed.binding)
            XCTAssertNil(observed.terminal)
            XCTAssertTrue(try store.observeAutomationWorker(lease: .init(occurrenceID: first.id, definitionRevision: definition.revision, workerID: "mac-test-a", leaseToken: "lease-a"), observation: .exited(exitCode: 0), observedAtMs: now + 2))
            XCTAssertNil(try store.reconcileAutomationExecution(occurrenceID: first.id, reconciledAtMs: now + 2),
                "A claimed occurrence without a bound Run cannot manufacture a successful result")
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let persisted = try XCTUnwrap(reopened.loadAutomationRun(id: "run-first"))
        XCTAssertNil(persisted.resultJSON)
        XCTAssertEqual(persisted.definitionRevision, input.definitionRevision)
        let observation = try reopened.observeAutomationExecution(occurrenceID: persisted.id)
        XCTAssertNil(observation.receipt)
        XCTAssertNil(observation.terminal)
        XCTAssertFalse(try reopened.claimAutomationRun(id: persisted.id, workerID: "replacement", leaseToken: "replacement",
            nowMs: now + 70_000, leaseUntilMs: now + 90_000), "Lease expiry alone cannot replay an uncertain occurrence")
        let failure = MacAutomationWorkerObservation.launchFailed(detail: "Fixture dependency platform:reminders failed before admission")
        XCTAssertTrue(try reopened.observeAutomationWorker(lease: .init(occurrenceID: persisted.id, definitionRevision: 1,
            workerID: "mac-test-a", leaseToken: "lease-a"), observation: failure, observedAtMs: now + 70_001))
        let failed = try reopened.observeAutomationExecution(occurrenceID: persisted.id)
        XCTAssertEqual(failed.phase, .launchFailed)
        XCTAssertEqual(failed.latestWorkerObservation, failure)
        XCTAssertNil(failed.binding)
        XCTAssertNil(failed.receipt, "Startup failure does not fabricate an admitted Run seal")
        XCTAssertEqual(try reopened.loadAutomationRun(id: persisted.id)?.status, "launch_failed")
        XCTAssertEqual(try reopened.listAutomationRuns(automationID: "automation-first").count, 1)
    }
}
