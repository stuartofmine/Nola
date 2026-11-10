import XCTest
@testable import Nola

@MainActor
final class MacProjectionObservationCoordinatorTests: XCTestCase {
    func testCancelledPresentationReadDoesNotStartWork() async {
        let read = Task { @MainActor in
            try await MacConversationReadTask.run {
                XCTFail("A cancelled page must not start storage work")
                return 1
            }
        }
        read.cancel()
        do {
            _ = try await read.value
            XCTFail("Cancelled read returned a value")
        } catch is CancellationError {
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testCancellingPresentationReadStopsFollowingStagesAndDrainsCurrentCall() async {
        let started = expectation(description: "First storage call entered")
        let release = DispatchSemaphore(value: 0)
        defer { release.signal() }
        let read = Task {
            try await MacConversationReadTask.run {
                XCTAssertFalse(Thread.isMainThread)
                started.fulfill()
                guard release.wait(timeout: .now() + 5) == .success else {
                    throw NSError(domain: "Read gate timed out", code: 1)
                }
                try Task.checkCancellation()
                XCTFail("Cancellation must skip subsequent pages and token estimation")
                return 1
            }
        }
        await fulfillment(of: [started], timeout: 5)
        read.cancel()
        release.signal()
        do {
            _ = try await read.value
            XCTFail("Cancelled read published a value")
        } catch is CancellationError {
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
        let current = try? await MacConversationReadTask.run { 2 }
        XCTAssertEqual(current, 2, "A cancelled page must not cancel a replacement read")
    }

    private final class Gate {
        private var continuation: CheckedContinuation<Void, Never>?
        private var opened = false

        func wait() async {
            if opened { return }
            await withCheckedContinuation { continuation = $0 }
        }

        func open() {
            opened = true
            continuation?.resume()
            continuation = nil
        }
    }

    func testReplacementDrainsOldObserverBeforeStartingNewOne() async {
        let coordinator = MacProjectionObservationCoordinator()
        let oldStarted = Gate()
        let oldRelease = Gate()
        let newStarted = Gate()
        let newRelease = Gate()
        var events: [String] = []
        let old = coordinator.start(conversationID: "conversation") { _ in
            events.append("old started")
            oldStarted.open()
            await oldRelease.wait()
            events.append("old finished")
        }
        await oldStarted.wait()
        let current = coordinator.start(conversationID: "conversation") { _ in
            events.append("new started")
            newStarted.open()
            await newRelease.wait()
        }
        XCTAssertFalse(coordinator.isCurrent(old))
        XCTAssertTrue(coordinator.isCurrent(current))
        await Task.yield()
        XCTAssertEqual(events, ["old started"])
        oldRelease.open()
        await newStarted.wait()
        XCTAssertEqual(events, ["old started", "old finished", "new started"])
        XCTAssertTrue(coordinator.isCurrent(current), "Old completion must not release its successor")
        newRelease.open()
        await coordinator.cancelAndDrain(conversationID: "conversation")
        XCTAssertFalse(coordinator.isCurrent(current))
    }

    func testCancelledObserverRemainsDrainableByReplacement() async {
        let coordinator = MacProjectionObservationCoordinator()
        let started = Gate()
        let release = Gate()
        let replacementStarted = Gate()
        let old = coordinator.start(conversationID: "conversation") { _ in
            started.open()
            await release.wait()
        }
        await started.wait()
        coordinator.cancel(conversationID: "conversation")
        XCTAssertFalse(coordinator.isCurrent(old))
        var replacementHasStarted = false
        coordinator.start(conversationID: "conversation") { _ in
            replacementHasStarted = true
            replacementStarted.open()
        }
        await Task.yield()
        XCTAssertFalse(replacementHasStarted)
        release.open()
        await replacementStarted.wait()
        await coordinator.cancelAndDrain(conversationID: "conversation")
    }

    func testCancellingQueuedReplacementDoesNotInvokeItsOperation() async {
        let coordinator = MacProjectionObservationCoordinator()
        let started = Gate()
        let release = Gate()
        coordinator.start(conversationID: "conversation") { _ in
            started.open()
            await release.wait()
        }
        await started.wait()
        var invoked = false
        coordinator.start(conversationID: "conversation") { _ in invoked = true }
        coordinator.cancel(conversationID: "conversation")
        release.open()
        await coordinator.cancelAndDrain(conversationID: "conversation")
        XCTAssertFalse(invoked)
    }

    func testDifferentConversationsDoNotWaitForEachOther() async {
        let coordinator = MacProjectionObservationCoordinator()
        let started = Gate()
        let release = Gate()
        let otherStarted = Gate()
        coordinator.start(conversationID: "first") { _ in
            started.open()
            await release.wait()
        }
        await started.wait()
        coordinator.start(conversationID: "second") { _ in otherStarted.open() }
        await otherStarted.wait()
        await coordinator.cancelAndDrain(conversationID: "second")
        release.open()
        await coordinator.cancelAndDrain(conversationID: "first")
    }

    func testActiveObservationRemainsReusableWhileOtherConversationIsCancelled() async {
        let coordinator = MacProjectionObservationCoordinator()
        let release = Gate()
        let active = coordinator.start(conversationID: "active") { _ in await release.wait() }
        let other = coordinator.start(conversationID: "other") { _ in await release.wait() }
        XCTAssertTrue(coordinator.isObserving(conversationID: "active"))
        coordinator.cancel(conversationID: "other")
        XCTAssertFalse(coordinator.isObserving(conversationID: "other"))
        XCTAssertTrue(coordinator.isObserving(conversationID: "active"))
        XCTAssertTrue(coordinator.isCurrent(active))
        XCTAssertFalse(coordinator.isCurrent(other))
        release.open()
        await coordinator.cancelAllAndDrain()
        XCTAssertFalse(coordinator.isObserving(conversationID: "active"))
    }

    func testDetachCanPreserveOnlyTheUnfinishedRunObserver() async {
        let coordinator = MacProjectionObservationCoordinator()
        let activeRelease = Gate()
        let idleRelease = Gate()
        let active = coordinator.start(conversationID: "active") { _ in await activeRelease.wait() }
        let idle = coordinator.start(conversationID: "idle") { _ in await idleRelease.wait() }
        coordinator.cancelAll(except: "active")
        XCTAssertTrue(coordinator.isCurrent(active))
        XCTAssertFalse(coordinator.isCurrent(idle))
        activeRelease.open()
        idleRelease.open()
        await coordinator.cancelAllAndDrain()
        XCTAssertFalse(coordinator.isCurrent(active))
    }

    func testDrainingDetachedLeasesCannotCancelReattachedObserver() async {
        let coordinator = MacProjectionObservationCoordinator()
        let oldStarted = Gate()
        let oldRelease = Gate()
        let newStarted = Gate()
        let newRelease = Gate()
        let old = coordinator.start(conversationID: "conversation") { _ in
            oldStarted.open()
            await oldRelease.wait()
        }
        await oldStarted.wait()
        let drain = Task { @MainActor in await coordinator.cancelAllAndDrain() }
        while coordinator.isCurrent(old) { await Task.yield() }
        let current = coordinator.start(conversationID: "conversation") { _ in
            newStarted.open()
            await newRelease.wait()
        }
        oldRelease.open()
        await drain.value
        await newStarted.wait()
        XCTAssertTrue(coordinator.isCurrent(current))
        newRelease.open()
        await coordinator.cancelAllAndDrain()
    }
}
