// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Maps shared agent capabilities to iOS authorization mechanisms.
// - Security: System prompts run only after explicit user approval.

import AVFoundation
import BackgroundTasks
import Contacts
import CoreLocation
import EventKit
import Foundation
import HealthKit
import Photos
import UIKit
import UserNotifications

nonisolated public struct IOSChildPermissionOwner: Sendable, Equatable, Hashable {
    public let taskID: String
    public let parentConversationID: String
    public let childConversationID: String
    public let runID: String
    public let coreSessionID: String
}

nonisolated public struct IOSAgentPermissionRequest: Sendable, Equatable {
    public let childOwner: IOSChildPermissionOwner?
    public var presentationID: String {
        guard let owner = childOwner else { return requestId }
        return ([owner.taskID, owner.parentConversationID, owner.childConversationID, owner.runID, owner.coreSessionID, requestId]
            .map { "\($0.utf8.count):\($0)" }).joined()
    }
    public let allowConversation: Bool
    public let requestId: String
    public let callId: String
    public let toolId: String
    public let route: String
    public let capability: AgentCoreCapability
    public let permission: String
    public let authorization: String
    public let policyDigest: String
    public let manifestDigest: String
    public let manifestValidUntilMs: Int64
    public let rationale: String
    public let originKind: String?
    public let scope: String?
    public let permissionInstructions: [AgentPermissionInstruction]

    public var toolName: String { toolId }
    public var presentationRationale: String {
        guard let owner = childOwner else { return rationale }
        return "子任务 \(owner.taskID) 请求审批\n" + rationale
    }

    public init?(
        requestId: String,
        allowConversation: Bool,
        callId: String,
        resolvedTool: AgentCoreResolvedTool,
        plan: AgentCorePreparedToolPlan,
        rationale: String,
        scope: String? = nil,
        permissionInstructions: [AgentPermissionInstruction] = [],
        childOwner: IOSChildPermissionOwner? = nil
    ) {
        guard let capability = AgentCoreCapability(rawValue: resolvedTool.capability),
              IOSAgentHostExecutor.installedToolCapabilities[resolvedTool.toolId] == capability,
              plan.resolvedTool(
                id: resolvedTool.toolId,
                route: "device",
                hostId: plan.expectedIdentity.hostId,
                clientId: plan.expectedIdentity.clientId,
                sessionId: plan.expectedIdentity.sessionId
              ) == resolvedTool else { return nil }
        self.childOwner = childOwner
        self.requestId = requestId
        self.allowConversation = allowConversation
        self.callId = callId
        self.toolId = resolvedTool.toolId
        self.route = resolvedTool.route
        self.capability = capability
        self.permission = resolvedTool.permission
        self.authorization = resolvedTool.authorization
        self.policyDigest = plan.output.policyDigest
        self.manifestDigest = plan.output.manifestDigest
        self.manifestValidUntilMs = plan.output.manifestValidUntilMs
        self.rationale = rationale
        self.originKind = nil
        self.scope = scope
        self.permissionInstructions = permissionInstructions
    }
    /// Constructed only after Core verifies the exact started permission effect.
    init(coreRequestID: String, callID: String, capabilityID: String, allowConversation: Bool,
         rationale: String, scope: String?, originKind: String? = nil, instructions: [AgentPermissionInstruction], childOwner: IOSChildPermissionOwner?) {
        self.childOwner = childOwner
        requestId = coreRequestID; callId = callID; toolId = capabilityID
        self.allowConversation = allowConversation
        route = "core"; capability = .none; permission = "agent_approval"; authorization = "ask"
        policyDigest = ""; manifestDigest = ""; manifestValidUntilMs = 0
        self.rationale = rationale; self.scope = scope; self.originKind = originKind; permissionInstructions = instructions
    }

}

nonisolated public enum IOSAgentScopedResourceAccess: Sendable, Equatable {
    case read
    case readWrite
}

nonisolated public enum IOSAgentPermissionOutcome: Sendable, Equatable {
    case decision(requestId: String, decision: AgentCorePermissionDecision)
    case scopedResourceGranted(requestId: String, grant: AgentCoreJSONValue)
    case requiresScopedResource(requestId: String, access: IOSAgentScopedResourceAccess, suggestedScope: String?)
    case unavailable(requestId: String, reason: String)
}

@MainActor
public final class IOSAgentPermissionBroker {
    private let dataSourcePermissions: DataSourcePermissionManager
    private let locationRequester = AgentLocationPermissionRequester()

    public convenience init() {
        self.init(dataSourcePermissions: .shared)
    }

    public init(dataSourcePermissions: DataSourcePermissionManager) {
        self.dataSourcePermissions = dataSourcePermissions
    }

    public var scopedGrants: [AgentCoreScopedGrant] {
        IOSAgentScopedGrantStore.shared.activeGrants
    }

    public func resolve(
        _ request: IOSAgentPermissionRequest,
        userApproved: Bool
    ) async -> IOSAgentPermissionOutcome {
        if request.route == "core" {
            // No OS domain is not consent: this runs only after the user's
            // explicit decision and the coordinator's fresh Core proof.
            return .decision(requestId: request.requestId, decision: userApproved ? .allowOnce : .deny)
        }
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        guard now < request.manifestValidUntilMs,
              request.route == "device",
              request.authorization != "denied",
              request.authorization != "unavailable" else {
            return .unavailable(requestId: request.requestId, reason: "The Core tool plan is stale or no longer authorized.")
        }
        guard userApproved else {
            return .decision(requestId: request.requestId, decision: .deny)
        }

        switch request.permission {
        case "none":
            return allowOnce(request)
        case "agent_approval":
            return allowOnce(request)
        case "scoped_resource" where request.capability == .filesRead:
            return .requiresScopedResource(requestId: request.requestId, access: .read, suggestedScope: request.scope)
        case "scoped_resource" where request.capability == .filesWrite:
            return .requiresScopedResource(requestId: request.requestId, access: .readWrite, suggestedScope: request.scope)
        case "scoped_resource":
            return .unavailable(requestId: request.requestId, reason: "This scoped capability has no iOS resource broker.")
        case "platform_lifecycle":
            guard UIApplication.shared.applicationState == .active else {
                return .unavailable(requestId: request.requestId, reason: "This tool is only available while Nola is in the foreground.")
            }
            return allowOnce(request)
        case "platform_capability":
            return request.authorization == "granted"
                ? allowOnce(request)
                : .unavailable(requestId: request.requestId, reason: "The required iOS capability is unavailable.")
        case "unsupported":
            return .unavailable(requestId: request.requestId, reason: "This tool is not supported on iOS.")
        case "system_permission":
            return await resolveSystemPermission(request)
        default:
            return .unavailable(requestId: request.requestId, reason: "Core returned an unknown permission requirement.")
        }
    }

    private func resolveSystemPermission(_ request: IOSAgentPermissionRequest) async -> IOSAgentPermissionOutcome {
        if request.authorization == "granted" {
            return allowOnce(request)
        }
        guard request.authorization == "ask" else {
            return .unavailable(requestId: request.requestId, reason: "The system permission is unavailable.")
        }
        switch request.capability {
        case .background:
            guard UIApplication.shared.backgroundRefreshStatus == .available else {
                return .unavailable(requestId: request.requestId, reason: "Background App Refresh is disabled")
            }
            return allowOnce(request)
        case .camera:
            return await decision(request, granted: await requestCaptureAccess(for: .video))
        case .microphone:
            return await decision(request, granted: await requestMicrophoneAccess())
        case .photos:
            let status = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
            return await decision(request, granted: status == .authorized || status == .limited)
        case .location:
            return await decision(request, granted: await locationRequester.requestWhenInUse())
        case .calendar:
            return await decision(request, granted: (try? await dataSourcePermissions.requestCalendarPermission()) == true)
        case .reminders:
            return await decision(request, granted: (try? await dataSourcePermissions.requestRemindersPermission()) == true)
        case .contacts:
            return await decision(request, granted: (try? await dataSourcePermissions.requestContactsPermission()) == true)
        case .health:
            return await decision(request, granted: (try? await dataSourcePermissions.requestHealthPermission()) == true)
        case .notifications:
            let granted = (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])) == true
            return await decision(request, granted: granted)
        case .clipboard:
            return allowOnce(request)
        case .filesRead, .filesWrite, .network, .process, .computer, .none:
            return .unavailable(requestId: request.requestId, reason: "This capability has no iOS system-permission broker.")
        }
    }

    public func scopedResourceWasSelected(
        for request: IOSAgentPermissionRequest,
        url: URL?
    ) -> IOSAgentPermissionOutcome {
        guard Int64(Date().timeIntervalSince1970 * 1_000) < request.manifestValidUntilMs,
              request.route == "device", request.authorization != "denied",
              request.authorization != "unavailable" else {
            return .unavailable(requestId: request.requestId, reason: "The tool selection expired or is no longer authorized.")
        }
        guard let url, url.startAccessingSecurityScopedResource() else {
            return .decision(requestId: request.requestId, decision: .deny)
        }
        defer { url.stopAccessingSecurityScopedResource() }
        let bookmark = try? url.bookmarkData(
            options: .minimalBookmark,
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        )
        let grant: AgentCoreJSONValue = .object([
            "uri": .string(url.absoluteString),
            "bookmark": bookmark.map { .string($0.base64EncodedString()) } ?? .null,
            "access": .string(request.capability == .filesWrite ? "read_write" : "read")
        ])
        IOSAgentScopedGrantStore.shared.register(
            capability: request.capability,
            scope: url.absoluteString
        )
        return .scopedResourceGranted(requestId: request.requestId, grant: grant)
    }

    private func allowOnce(_ request: IOSAgentPermissionRequest) -> IOSAgentPermissionOutcome {
        .decision(requestId: request.requestId, decision: .allowOnce)
    }

    private func decision(
        _ request: IOSAgentPermissionRequest,
        granted: Bool
    ) async -> IOSAgentPermissionOutcome {
        .decision(requestId: request.requestId, decision: granted ? .allowOnce : .deny)
    }

    private func requestCaptureAccess(for mediaType: AVMediaType) async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: mediaType) {
        case .authorized:
            return true
        case .notDetermined:
            return await AVCaptureDevice.requestAccess(for: mediaType)
        case .denied, .restricted:
            return false
        @unknown default:
            return false
        }
    }

    private func requestMicrophoneAccess() async -> Bool {
        if #available(iOS 17.0, *) {
            return await withCheckedContinuation { continuation in
                AVAudioApplication.requestRecordPermission { granted in
                    continuation.resume(returning: granted)
                }
            }
        }
        return await requestCaptureAccess(for: .audio)
    }
}

@MainActor
private final class AgentLocationPermissionRequester: NSObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var continuation: CheckedContinuation<Bool, Never>?

    override init() {
        super.init()
        manager.delegate = self
    }

    func requestWhenInUse() async -> Bool {
        switch manager.authorizationStatus {
        case .authorizedAlways, .authorizedWhenInUse:
            return true
        case .denied, .restricted:
            return false
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                self.continuation = continuation
                manager.requestWhenInUseAuthorization()
            }
        @unknown default:
            return false
        }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard manager.authorizationStatus != .notDetermined,
              let continuation else {
            return
        }
        self.continuation = nil
        continuation.resume(
            returning: manager.authorizationStatus == .authorizedAlways
                || manager.authorizationStatus == .authorizedWhenInUse
        )
    }
}
