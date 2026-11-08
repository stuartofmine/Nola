import Foundation
import XCTest
@testable import Nola

final class IOSModelProfilePublicationCASTests: XCTestCase {
    @MainActor
    func testSourcePublicationPreservesParametersAndSelectionAndRejectsStaleWriter() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
        let initial = profile(account: "fixture-account-v1")
        let firstRevision = try runtime.upsertModelProfileCAS(initial, expectedRevision: 0, requestID: "create")
        let observed = try XCTUnwrap(runtime.modelProfile(id: initial.id))
        let updated = profile(account: "fixture-account-v2", preserving: observed)
        let secondRevision = try runtime.upsertModelProfileCAS(updated,
            expectedRevision: firstRevision, requestID: "publish-v2")
        XCTAssertGreaterThan(secondRevision, firstRevision)
        let committed = try XCTUnwrap(runtime.modelProfile(id: initial.id))
        XCTAssertEqual(committed.sourceJSON, updated.sourceJSON)
        XCTAssertEqual(committed.parametersJSON, observed.parametersJSON)
        XCTAssertEqual(committed.selected, observed.selected)
        XCTAssertEqual(committed.enabled, observed.enabled)
        XCTAssertEqual(committed.createdAtMs, observed.createdAtMs)
        XCTAssertEqual(committed.revision, Int64(secondRevision))
        do {
            _ = try runtime.upsertModelProfileCAS(profile(account: "stale-account", preserving: observed),
                expectedRevision: firstRevision, requestID: "stale-publish")
            XCTFail("Stale credential publication must conflict")
        } catch IOSApplicationRuntimeError.revisionConflict(let scope, let expected, let actual) {
            XCTAssertEqual(scope, "model_profile:credential-fixture")
            XCTAssertEqual(expected, firstRevision)
            XCTAssertEqual(actual, secondRevision)
        }
        XCTAssertEqual(try runtime.modelProfile(id: initial.id), committed)
    }

    @MainActor
    func testPublicationReceiptReplaySurvivesReopenWithoutRevertingLaterSource() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("application.db")
        let initial = profile(account: "fixture-account-v1")
        let firstRevision: UInt64
        let latest: IOSCanonicalModelProfile
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            firstRevision = try runtime.upsertModelProfileCAS(initial, expectedRevision: 0, requestID: "create")
            XCTAssertEqual(try runtime.upsertModelProfileCAS(initial, expectedRevision: 0, requestID: "create"), firstRevision)
            let observed = try XCTUnwrap(runtime.modelProfile(id: initial.id))
            _ = try runtime.upsertModelProfileCAS(profile(account: "fixture-account-v2", preserving: observed),
                expectedRevision: firstRevision, requestID: "update")
            latest = try XCTUnwrap(runtime.modelProfile(id: initial.id))
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try reopened.upsertModelProfileCAS(initial, expectedRevision: 0, requestID: "create"), firstRevision)
        XCTAssertEqual(try reopened.modelProfile(id: initial.id), latest,
            "Old receipt replay must return its own revision, not overwrite the newer source")
    }

    @MainActor
    func testExplicitRejectedPublicationPreservesOldProfile() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("application.db"))
        let revision = try runtime.upsertModelProfileCAS(profile(account: "fixture-account-v1"),
            expectedRevision: 0, requestID: "create")
        let observed = try XCTUnwrap(runtime.modelProfile(id: "credential-fixture"))
        let invalid = IOSCanonicalModelProfile(id: observed.id, displayName: observed.displayName,
            provider: "foreign-provider", modelName: observed.modelName, runtime: observed.runtime,
            sourceJSON: observed.sourceJSON, parametersJSON: observed.parametersJSON,
            selected: observed.selected, enabled: observed.enabled, revision: observed.revision,
            createdAtMs: observed.createdAtMs, updatedAtMs: 3_000, deletedAtMs: nil)
        do {
            _ = try runtime.upsertModelProfileCAS(invalid, expectedRevision: revision, requestID: "reject")
            XCTFail("A profile cannot change provider ownership")
        } catch IOSApplicationRuntimeError.rejected {
            // A terminal rejected receipt is distinct from outcomePending or I/O failure.
        }
        XCTAssertEqual(try runtime.modelProfile(id: observed.id), observed)
    }

    private func profile(account: String, preserving old: IOSCanonicalModelProfile? = nil) -> IOSCanonicalModelProfile {
        IOSCanonicalModelProfile(id: old?.id ?? "credential-fixture", displayName: old?.displayName ?? "Fixture",
            provider: old?.provider ?? "cloud", modelName: old?.modelName ?? "fixture-model",
            runtime: old?.runtime ?? "remote", sourceJSON: "{\"credentialAccount\":\"\(account)\"}",
            parametersJSON: old?.parametersJSON ?? "{\"temperature\":0.37,\"maxOutputTokens\":4096}",
            selected: old?.selected ?? true, enabled: old?.enabled ?? true, revision: old?.revision ?? 0,
            createdAtMs: old?.createdAtMs ?? 1_000, updatedAtMs: old == nil ? 1_000 : 2_000,
            deletedAtMs: old?.deletedAtMs)
    }
}
