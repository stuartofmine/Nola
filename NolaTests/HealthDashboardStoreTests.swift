// SPDX-License-Identifier: Apache-2.0

import XCTest
import HealthKit
@testable import Nola

@MainActor
final class HealthDashboardStoreTests: XCTestCase {
    func testInitialHealthAuthorizationWaitIsNotPresentedAsRefreshFailure() async {
        let store = HealthDashboardStore {
            throw NSError(domain: HKErrorDomain, code: HKError.Code.errorAuthorizationNotDetermined.rawValue)
        }
        await store.load()
        XCTAssertNil(store.snapshot)
        XCTAssertNil(store.errorMessage)
        XCTAssertFalse(store.isLoading)
    }

    func testDeniedHealthAuthorizationStillSurfacesFailure() async {
        let store = HealthDashboardStore {
            throw NSError(domain: HKErrorDomain, code: HKError.Code.errorAuthorizationDenied.rawValue)
        }
        await store.load()
        XCTAssertNil(store.snapshot)
        XCTAssertNotNil(store.errorMessage)
        XCTAssertFalse(store.isLoading)
    }

    func testConcurrentLoadsWaitForTheSameSnapshotRequest() async {
        let gate = HealthDashboardTestGate()
        let completion = HealthDashboardTestFlag()
        var loadCount = 0
        let store = HealthDashboardStore {
            loadCount += 1
            await gate.wait()
            throw HealthDashboardTestError.failed
        }

        let first = Task { await store.load() }
        while loadCount == 0 {
            await Task.yield()
        }

        let second = Task {
            await store.load()
            await completion.set()
        }
        await Task.yield()
        await Task.yield()

        let completedBeforeRelease = await completion.value
        XCTAssertTrue(store.isLoading)
        XCTAssertFalse(completedBeforeRelease)
        XCTAssertEqual(loadCount, 1)

        await gate.open()
        await first.value
        await second.value

        let completedAfterRelease = await completion.value
        XCTAssertFalse(store.isLoading)
        XCTAssertTrue(completedAfterRelease)
        XCTAssertEqual(loadCount, 1)
        XCTAssertNotNil(store.errorMessage)
    }

    func testMissingAuthorizedSnapshotIsAnErrorRatherThanPermissionPrompt() {
        let state = DataSourceStatusStore.resolveHealthState(
            snapshot: nil,
            errorMessage: nil,
            reportedPermission: .authorized,
            cached: nil
        )

        XCTAssertEqual(state.phase, .error)
        XCTAssertEqual(state.permission, .authorized)
    }
}

private enum HealthDashboardTestError: LocalizedError {
    case failed

    var errorDescription: String? { "Expected test failure" }
}

private actor HealthDashboardTestFlag {
    private(set) var value = false

    func set() {
        value = true
    }
}

private actor HealthDashboardTestGate {
    private var isOpen = false
    private var continuations: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        guard !isOpen else { return }
        await withCheckedContinuation { continuation in
            continuations.append(continuation)
        }
    }

    func open() {
        isOpen = true
        let pending = continuations
        continuations.removeAll()
        pending.forEach { $0.resume() }
    }
}
