import Foundation
import UIKit

nonisolated struct IOSRunToolPlanObservation: Sendable {
    let nowMs: Int64
    let isForeground: Bool
    let authorizations: [String: String]
}

/// A receipt can expire; the original selection and authorization lifetimes cannot expand.
actor IOSRunToolPlanSession {
    private var receipt: AgentCorePreparedToolPlan
    private let original: AgentCoreToolPolicyInput?
    private let observe: @Sendable () async -> IOSRunToolPlanObservation
    private var inFlight: (id: UUID, task: Task<AgentCorePreparedToolPlan, Error>)?

    init(plan: AgentCorePreparedToolPlan,
         observe: @escaping @Sendable () async -> IOSRunToolPlanObservation = {
             await MainActor.run {
                 let foreground = UIApplication.shared.applicationState == .active
                 return .init(nowMs: Int64(Date().timeIntervalSince1970 * 1_000), isForeground: foreground,
                    authorizations: AgentCoreToolCatalog.authorizationSnapshot(isForeground: foreground))
             }
         }) {
        receipt = plan
        original = plan.originalIntent
        self.observe = observe
    }

    func currentPlan() async throws -> AgentCorePreparedToolPlan {
        if let inFlight { return try await inFlight.task.value }
        let id = UUID()
        let task = Task { try await self.refreshPlan() }
        inFlight = (id, task)
        do {
            let result = try await task.value
            if inFlight?.id == id { inFlight = nil }
            return result
        } catch {
            if inFlight?.id == id { inFlight = nil }
            throw error
        }
    }

    private func refreshPlan() async throws -> AgentCorePreparedToolPlan {
        let state = await observe()
        guard state.nowMs > 0, state.nowMs <= Int64.max - 60_000 else {
            throw AgentCoreToolPolicyError.invalidPlan("Invalid authorization observation time.")
        }
        if state.nowMs < receipt.output.manifestValidUntilMs,
           state.isForeground == receipt.isForeground,
           state.authorizations == receipt.authorizationSnapshot,
           receipt.scopedGrants.allSatisfy({ $0.expiresAtMs > state.nowMs }),
           receipt.resultEgressGrants.allSatisfy({ $0.expiresAtMs > state.nowMs }) { return receipt }
        guard let original, original.expectedIdentity == receipt.expectedIdentity else {
            throw AgentCoreToolPolicyError.invalidPlan("The original tool selection is unavailable for renewal.")
        }
        let manifest = original.runtime
        let manifestID = "nola.ios.native-handlers.\(UUID().uuidString.lowercased())"
        // Renewal preserves original grant timestamps. A new receipt is not new consent.
        let egressReceipts = manifest.resultEgressReceipts.filter { $0.expiresAtMs > state.nowMs }.map {
            AgentCoreRuntimeResultEgressReceipt(toolId: $0.toolId, route: $0.route, manifestId: manifestID,
                hostId: $0.hostId, clientId: $0.clientId, sessionId: $0.sessionId,
                issuedAtMs: $0.issuedAtMs, expiresAtMs: $0.expiresAtMs, allowResultEgress: $0.allowResultEgress)
        }
        let grants = original.resultEgressGrants.filter { $0.expiresAtMs > state.nowMs }.map {
            AgentCoreResultEgressGrant(toolId: $0.toolId, route: $0.route, manifestId: manifestID,
                hostId: $0.hostId, clientId: $0.clientId, sessionId: $0.sessionId, expiresAtMs: $0.expiresAtMs)
        }
        let scoped = original.scopedGrants.filter { $0.expiresAtMs > state.nowMs }
        let input = AgentCoreToolPolicyInput(schemaVersion: original.schemaVersion,
            platform: original.platform, nowMs: state.nowMs, expectedIdentity: original.expectedIdentity,
            runtime: .init(manifestId: manifestID, manifestVersion: manifest.manifestVersion,
                issuedAtMs: state.nowMs, expiresAtMs: state.nowMs + 60_000,
                hostId: manifest.hostId, clientId: manifest.clientId, sessionId: manifest.sessionId,
                routes: manifest.routes, capabilities: manifest.capabilities,
                authorizations: state.authorizations, resultEgressReceipts: egressReceipts, tools: manifest.tools),
            conversation: original.conversation, userPolicy: original.userPolicy,
            scopedGrants: scoped, resultEgressGrants: grants)
        let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(input)
        guard output.schemaVersion == 1 else { throw AgentCoreToolPolicyError.invalidResponse }
        receipt = .init(output: output, expectedIdentity: receipt.expectedIdentity,
            authorizationSnapshot: state.authorizations, isForeground: state.isForeground,
            scopedGrants: scoped, resultEgressGrants: grants, originalIntent: original)
        return receipt
    }
}
