import Foundation
import XCTest
@testable import Nola

final class IOSRunToolPlanSessionTests: XCTestCase {
    @MainActor
    func testExpiredReceiptRenewsOriginalIntentWithoutExpandingSelection() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true,
            allowedTools: ["memory_search"], deniedTools: ["device_clipboard_read"], nowMs: now - 61_000)
        let original = try XCTUnwrap(plan.originalIntent)
        let session = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now, isForeground: true, authorizations: plan.authorizationSnapshot)
        })
        let fresh = try await session.currentPlan()
        XCTAssertGreaterThan(fresh.output.manifestValidUntilMs, now)
        XCTAssertEqual(fresh.originalIntent, original)
        XCTAssertEqual(fresh.expectedIdentity, plan.expectedIdentity)
        XCTAssertTrue(fresh.output.enabledTools.allSatisfy { $0.toolId == "memory_search" })
        XCTAssertFalse(fresh.output.enabledTools.contains { $0.toolId == "device_clipboard_read" })
        let again = try await session.currentPlan()
        XCTAssertEqual(again, fresh)
    }

    @MainActor
    func testExpiredOriginalScopedGrantIsRemovedNotRenewed() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let grant = AgentCoreScopedGrant(capability: AgentCoreCapability.filesRead.rawValue,
            route: "device", scope: "file:///isolated", expiresAtMs: now - 1)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true, scopedGrants: [grant], nowMs: now - 1000)
        let session = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now, isForeground: true, authorizations: plan.authorizationSnapshot)
        })
        let refreshed = try await session.currentPlan()
        XCTAssertTrue(refreshed.scopedGrants.isEmpty)
        XCTAssertEqual(refreshed.originalIntent?.scopedGrants, [grant])
    }

    @MainActor
    func testConcurrentRefreshSharesObservationAndNextCallRechecksRevocation() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: identity, isForeground: true, nowMs: now - 61_000)
        var revoked = plan.authorizationSnapshot
        revoked[AgentCoreCapability.clipboard.rawValue] = "denied"
        let latest = IOSRunToolPlanObservation(nowMs: now, isForeground: true, authorizations: revoked)
        let gate = ObservationGate(latest: latest)
        let session = IOSRunToolPlanSession(plan: plan, observe: { await gate.observe() })
        let old = Task { try await session.currentPlan() }
        await gate.waitUntilSuspended()
        let second = Task { try await session.currentPlan() }
        await gate.release(latest)
        let returned = try await old.value
        let fresh = try await second.value
        XCTAssertEqual(fresh, returned)
        let subsequent = try await session.currentPlan()
        XCTAssertEqual(subsequent.authorizationSnapshot, revoked)
        XCTAssertEqual(returned.authorizationSnapshot, revoked)
        XCTAssertFalse(returned.output.enabledTools.contains { $0.toolId == "device_clipboard_read" })
    }

    @MainActor
    func testForegroundRefreshDoesNotExtendOriginalEgressConsent() async throws {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let authorization = try XCTUnwrap(IOSAgentResultEgressAuthorization.explicitUserApproval(
            identity: identity, userApproved: true, nowMs: now))
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .hybrid,
            expectedIdentity: identity, isForeground: true, resultEgressAuthorization: authorization, nowMs: now)
        let session = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now + 1, isForeground: false, authorizations: plan.authorizationSnapshot)
        })
        let fresh = try await session.currentPlan()
        XCTAssertFalse(fresh.resultEgressGrants.isEmpty)
        XCTAssertEqual(Set(fresh.resultEgressGrants.map(\.expiresAtMs)), Set(plan.resultEgressGrants.map(\.expiresAtMs)))
        let expiredSession = IOSRunToolPlanSession(plan: plan, observe: {
            .init(nowMs: now + 61_000, isForeground: false, authorizations: plan.authorizationSnapshot)
        })
        let expired = try await expiredSession.currentPlan()
        XCTAssertTrue(expired.resultEgressGrants.isEmpty)
    }

    private var identity: AgentCoreExpectedIdentity {
        .init(hostId: "test-host", clientId: "test-client", sessionId: "test-session")
    }

    private actor ObservationGate {
        let latest: IOSRunToolPlanObservation
        var suspended: CheckedContinuation<IOSRunToolPlanObservation, Never>?
        var waiter: CheckedContinuation<Void, Never>?
        var started = false
        init(latest: IOSRunToolPlanObservation) { self.latest = latest }
        func observe() async -> IOSRunToolPlanObservation {
            guard !started else { return latest }
            started = true
            return await withCheckedContinuation { continuation in
                suspended = continuation
                waiter?.resume(); waiter = nil
            }
        }
        func waitUntilSuspended() async {
            if suspended != nil { return }
            await withCheckedContinuation { waiter = $0 }
        }
        func release(_ value: IOSRunToolPlanObservation) { suspended?.resume(returning: value); suspended = nil }
    }
}
