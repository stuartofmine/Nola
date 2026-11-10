// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Binds installed iOS handlers and live platform state to a Core v2 plan receipt.
// - Security: Untrusted host events never select capability, permission, authorization, or route.

import CoreLocation
import EventKit
import Foundation
import UIKit

@MainActor
public final class IOSAgentScopedGrantStore {
    public static let shared = IOSAgentScopedGrantStore()
    private var grants: [AgentCoreScopedGrant] = []

    public var activeGrants: [AgentCoreScopedGrant] {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        grants.removeAll { $0.expiresAtMs <= now }
        return grants
    }

    public func register(capability: AgentCoreCapability, scope: String, lifetime: TimeInterval = 1_800) {
        let expires = Int64((Date().timeIntervalSince1970 + lifetime) * 1_000)
        grants.removeAll { $0.capability == capability.rawValue && $0.scope == scope }
        grants.append(.init(capability: capability.rawValue, route: "device", scope: scope, expiresAtMs: expires))
    }
}

public extension IOSAgentHostExecutor {
    nonisolated static let installedToolCapabilities: [String: AgentCoreCapability] = [
        "skill_invoke": .none,
        // Executed by the Core-bound publisher in DeviceAgentCoordinator.
        "render_ui": .none,
        "memory_search": .none,
        "device_read_file": .filesRead,
        "device_write_file": .filesWrite,
        "device_clipboard_read": .clipboard,
        "device_clipboard_write": .clipboard,
        "device_get_location": .location,
        "device_create_calendar_event": .calendar,
        "device_search_calendar_events": .calendar,
    ]

    nonisolated static var installedToolIDs: Set<String> {
        Set(installedToolCapabilities.keys)
    }

    func execute(
        event: AgentCoreEvent,
        using plan: AgentCorePreparedToolPlan
    ) async -> IOSAgentHostCallResult? {
        guard await AgentCoreToolCatalog.isCurrent(plan),
              let callId = event.callId,
              let toolId = event.toolName,
              let resolved = plan.resolvedTool(
                id: toolId,
                route: "device",
                hostId: plan.expectedIdentity.hostId,
                clientId: plan.expectedIdentity.clientId,
                sessionId: plan.expectedIdentity.sessionId
              ),
              let capability = AgentCoreCapability(rawValue: resolved.capability),
              Self.installedToolCapabilities[toolId] == capability,
              event.capability == nil || event.capability == capability else {
            return IOSAgentHostCallResult(
                callId: event.callId ?? "invalid",
                output: .object(["error": .string("Core tool plan validation failed.")]),
                isError: true
            )
        }
        return await executeBridge(
            callId: callId,
            toolName: toolId,
            capability: capability,
            arguments: event.arguments ?? .object([:]),
            grant: event.grant,
            sessionId: plan.expectedIdentity.sessionId
        )
    }

    func executeBridge(
        callId: String,
        arguments: AgentCoreJSONValue,
        grant: AgentCoreJSONValue?,
        resolvedTool: AgentCoreResolvedTool,
        plan: AgentCorePreparedToolPlan,
        executionPermit: IOSRunExecutionPermit? = nil
    ) async -> IOSAgentHostCallResult {
        guard await AgentCoreToolCatalog.isCurrent(plan),
              let trusted = plan.resolvedTool(
                id: resolvedTool.toolId,
                route: "device",
                hostId: plan.expectedIdentity.hostId,
                clientId: plan.expectedIdentity.clientId,
                sessionId: plan.expectedIdentity.sessionId
              ),
              trusted == resolvedTool,
              let capability = AgentCoreCapability(rawValue: trusted.capability),
              Self.installedToolCapabilities[trusted.toolId] == capability else {
            return IOSAgentHostCallResult(
                callId: callId,
                output: .object(["error": .string("The device tool plan is stale or does not match an installed handler.")]),
                isError: true
            )
        }
        return await executeBridge(
            callId: callId,
            toolName: trusted.toolId,
            capability: capability,
            arguments: arguments,
            grant: grant,
            sessionId: plan.expectedIdentity.sessionId,
            executionPermit: executionPermit
        )
    }
}

nonisolated public struct IOSAgentResultEgressAuthorization: Equatable, Sendable {
    let identity: AgentCoreExpectedIdentity
    let issuedAtMs: Int64
    let expiresAtMs: Int64

    static func explicitUserApproval(
        identity: AgentCoreExpectedIdentity,
        userApproved: Bool,
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) -> Self? {
        guard userApproved,
              let hostId = identity.hostId, !hostId.isEmpty,
              let clientId = identity.clientId, !clientId.isEmpty,
              let sessionId = identity.sessionId, !sessionId.isEmpty else {
            return nil
        }
        return .init(identity: identity, issuedAtMs: nowMs, expiresAtMs: nowMs + 30_000)
    }
}

public extension AgentCoreToolCatalog {
    static func prepareBoundPlan(
        for mode: AgentCoreExecutionMode,
        expectedIdentity: AgentCoreExpectedIdentity,
        isForeground: Bool,
        allowedTools: Set<String> = [],
        deniedTools: Set<String> = [],
        allowedCapabilities: Set<String> = [],
        deniedCapabilities: Set<String> = [],
        resultEgressAuthorization: IOSAgentResultEgressAuthorization? = nil,
        scopedGrants: [AgentCoreScopedGrant] = [],
        nowMs: Int64 = Int64(Date().timeIntervalSince1970 * 1_000)
    ) throws -> AgentCorePreparedToolPlan {
        let route = mode == .hostRemote ? "agent_host" : "device"
        var candidates = mode == .hostRemote ? [] : deviceTools
        if mode != .hostRemote, let application = IOSApplicationRuntime.shared {
            let contract = try application.executeAgentCommand(.object(["type": .string("read_ui_capability")]))
            guard case .object(let fields) = contract, let schema = fields["model_tool"] else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            candidates.append(schema)
        }
        let manifests = candidates.compactMap { schema -> AgentCoreRuntimeToolManifest? in
            guard let toolId = functionName(in: schema),
                  IOSAgentHostExecutor.installedToolIDs.contains(toolId),
                  route == "device" else { return nil }
            return .init(
                toolId: toolId,
                handlerId: toolId,
                schema: strippingClientMetadata(from: schema),
                route: route,
                available: true
            )
        }
        let authorizations = authorizationSnapshot(isForeground: isForeground)
        let manifestId = "nola.ios.native-handlers.\(UUID().uuidString.lowercased())"
        let manifestExpiresAtMs = nowMs + 60_000
        let approvedAuthorization: IOSAgentResultEgressAuthorization?
        if let authorization = resultEgressAuthorization,
           authorization.identity == expectedIdentity,
           authorization.issuedAtMs <= nowMs + 5_000,
           authorization.expiresAtMs > nowMs,
           authorization.expiresAtMs <= manifestExpiresAtMs,
           route == "device" {
            approvedAuthorization = authorization
        } else {
            approvedAuthorization = nil
        }
        let receipts: [AgentCoreRuntimeResultEgressReceipt] = approvedAuthorization.map { authorization in
            guard let hostId = expectedIdentity.hostId,
                  let clientId = expectedIdentity.clientId,
                  let sessionId = expectedIdentity.sessionId else { return [] }
            return manifests.map {
                .init(
                    toolId: $0.toolId,
                    route: route,
                    manifestId: manifestId,
                    hostId: hostId,
                    clientId: clientId,
                    sessionId: sessionId,
                    issuedAtMs: authorization.issuedAtMs,
                    expiresAtMs: authorization.expiresAtMs,
                    allowResultEgress: true
                )
            }
        } ?? []
        let egressGrants = receipts.map {
            AgentCoreResultEgressGrant(
                toolId: $0.toolId,
                route: $0.route,
                manifestId: $0.manifestId,
                hostId: $0.hostId,
                clientId: $0.clientId,
                sessionId: $0.sessionId,
                expiresAtMs: $0.expiresAtMs
            )
        }
        let input = AgentCoreToolPolicyInput(
            platform: "ios",
            nowMs: nowMs,
            expectedIdentity: expectedIdentity,
            runtime: .init(
                manifestId: manifestId,
                manifestVersion: 2,
                issuedAtMs: nowMs,
                expiresAtMs: manifestExpiresAtMs,
                hostId: expectedIdentity.hostId,
                clientId: expectedIdentity.clientId,
                sessionId: expectedIdentity.sessionId,
                routes: manifests.isEmpty ? [] : [route],
                capabilities: Set(IOSAgentHostExecutor.installedToolCapabilities.values.map(\.rawValue)).sorted(),
                authorizations: authorizations,
                resultEgressReceipts: receipts,
                tools: manifests
            ),
            conversation: .init(
                allowedTools: allowedTools.sorted(),
                deniedTools: deniedTools.sorted(),
                allowedCapabilities: allowedCapabilities.sorted(),
                deniedCapabilities: deniedCapabilities.sorted()
            ),
            userPolicy: .init(allowResultEgressRoutes: receipts.isEmpty ? [] : [route]),
            scopedGrants: scopedGrants,
            resultEgressGrants: egressGrants
        )
        let output = try AgentCoreToolPolicyResolver.resolveToolPolicy(input)
        guard output.schemaVersion == 1 else {
            throw AgentCoreToolPolicyError.invalidPlan("Core returned a non-v2 tool plan.")
        }
        return .init(
            output: output,
            expectedIdentity: expectedIdentity,
            authorizationSnapshot: authorizations,
            isForeground: isForeground,
            scopedGrants: scopedGrants,
            resultEgressGrants: egressGrants,
            originalIntent: input
        )
    }

    @MainActor
    static func isCurrent(_ plan: AgentCorePreparedToolPlan) -> Bool {
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        let foreground = UIApplication.shared.applicationState == .active
        return now < plan.output.manifestValidUntilMs
            && foreground == plan.isForeground
            && authorizationSnapshot(isForeground: foreground) == plan.authorizationSnapshot
            && plan.scopedGrants.allSatisfy { $0.expiresAtMs > now }
            && plan.resultEgressGrants.allSatisfy { $0.expiresAtMs > now }
    }

    @MainActor
    static func authorizationSnapshot(isForeground: Bool) -> [String: String] {
        var result: [String: String] = [
            AgentCoreCapability.none.rawValue: "granted",
            AgentCoreCapability.filesRead.rawValue: "ask",
            AgentCoreCapability.filesWrite.rawValue: "ask",
            AgentCoreCapability.clipboard.rawValue: isForeground ? "granted" : "unavailable",
        ]
        switch CLLocationManager().authorizationStatus {
        case .authorizedAlways, .authorizedWhenInUse:
            result[AgentCoreCapability.location.rawValue] = isForeground ? "granted" : "unavailable"
        case .notDetermined:
            result[AgentCoreCapability.location.rawValue] = isForeground ? "ask" : "unavailable"
        case .denied, .restricted:
            result[AgentCoreCapability.location.rawValue] = "denied"
        @unknown default:
            result[AgentCoreCapability.location.rawValue] = "unavailable"
        }
        let calendarStatus = EKEventStore.authorizationStatus(for: .event)
        switch calendarStatus {
        case .fullAccess, .writeOnly, .authorized:
            result[AgentCoreCapability.calendar.rawValue] = isForeground ? "granted" : "unavailable"
        case .notDetermined:
            result[AgentCoreCapability.calendar.rawValue] = isForeground ? "ask" : "unavailable"
        case .denied, .restricted:
            result[AgentCoreCapability.calendar.rawValue] = "denied"
        @unknown default:
            result[AgentCoreCapability.calendar.rawValue] = "unavailable"
        }
        return result
    }

    private static func functionName(in schema: AgentCoreJSONValue) -> String? {
        guard case .object(let root) = schema,
              case .object(let function)? = root["function"],
              case .string(let name)? = function["name"] else { return nil }
        return name
    }

    private static func strippingClientMetadata(from schema: AgentCoreJSONValue) -> AgentCoreJSONValue {
        guard case .object(var root) = schema else { return schema }
        let permissionContract: AgentCoreJSONValue?
        if case .object(let metadata)? = root["x_nola"] {
            permissionContract = metadata["permission_contract"]
        } else {
            permissionContract = nil
        }
        root.removeValue(forKey: "x_nola")
        // This input comes only from the compiled local deviceTools catalog.
        // Keep its execution contract in the bound, digested manifest; display
        // metadata is not authority and remains excluded.
        if let permissionContract {
            root["x_nola"] = .object(["permission_contract": permissionContract])
        }
        return .object(root)
    }
}
