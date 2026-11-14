// SPDX-License-Identifier: Apache-2.0
// Main Agent source binding for expert dispatch on the shared Task worker.
import Foundation
import UIKit

nonisolated struct IOSExpertPeerHostRequest: Sendable {
    let contextID: String
    let runID: String
    let callID: String
    let arguments: [String: AgentCoreJSONValue]
    let isLegacyRecovery: Bool

    init(request: AgentCoreJSONValue, expectedContextID: String, expectedRunID: String) throws {
        guard case .object(let envelope) = request,
              envelope["session_id"]?.stringScalar == expectedContextID,
              envelope["turn_id"]?.stringScalar == expectedRunID,
              case .object(let payload)? = envelope["payload"],
              let capability = payload["capability_id"]?.stringScalar, ["delegate_expert", "expert_exchange"].contains(capability),
              let call = payload["call_id"]?.stringScalar, !call.isEmpty,
              case .object(let arguments)? = payload["input"] else { throw IOSApplicationRuntimeError.invalidResponse }
        guard let action = arguments["action"]?.stringScalar else { throw IOSApplicationRuntimeError.invalidResponse }
        if capability == "expert_exchange", !["request", "read", "wait", "cancel"].contains(action) { throw IOSApplicationRuntimeError.invalidResponse }
        let allowed: Set<String>
        switch action {
        case "directory": allowed = ["action"]
        case "request": allowed = ["action", "target_participant_id", "text", "in_reply_to"]
        case "read", "cancel": allowed = ["action", "exchange_id"]
        case "wait": allowed = ["action", "exchange_id", "timeout_ms"]
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
        guard Set(arguments.keys).isSubset(of: allowed) else {
            throw IOSApplicationRuntimeError.rejected("Source identity is supplied by the Host, not tool arguments")
        }
        self.contextID = expectedContextID; self.runID = expectedRunID; self.callID = call; self.arguments = arguments; self.isLegacyRecovery = capability == "expert_exchange"
    }
}


nonisolated struct IOSExpertTaskPreparationError: LocalizedError, Sendable {
    let exchangeID: String
    let reason: String
    var taskMayExist: Bool = false
    var preparationAborted: Bool = false
    var errorDescription: String? { reason }
}

/// The same native configuration producer used for ordinary/Goal execution,
/// bound to the captured target expert rather than the source Task's model.
@MainActor enum IOSExpertTaskTargetFactory {
    static func isMainContext(runtime: IOSApplicationRuntime, conversationID: String) throws -> Bool {
        let value = try runtime.executeAgentCommand(.object(["type": .string("load_conversation_metadata"), "conversation_id": .string(conversationID)]))
        guard case .object(let fields) = value else { return false }
        let settings = try IOSConversationSettingsObject.decode(fields["settings_json"]?.stringScalar)
        guard let owner = settings["agentRoomOwner"] else { return true }
        _ = try ConversationAgentRoomOwner.read(owner)
        return false
    }

    static func configuration(material: AppleExpertPeerMaterial<AgentCoreJSONValue>, runtime: IOSApplicationRuntime,
                              skills: [IOSAgentSkillSnapshot], hostExecutor: IOSAgentHostExecutor) async throws -> AgentCoreJSONValue {
        let entry = material.entry, context = entry.participant.context_conversation_id
        guard material.view.target == entry.participant, material.view.exchangeID == entry.child_submission_id,
              let owner = UUID(uuidString: context), owner.uuidString == context else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let store = try LocalConversationStore(applicationRuntime: runtime)
        guard let conversation = try store.getConversation(conversationId: owner),
              conversation.agentRoomOwner?.roomID == material.view.roomID,
              conversation.agentRoomOwner?.participantID == entry.participant.participant_id,
              conversation.agentRoomOwner?.agentID == entry.participant.agent_id,
              conversation.configurationRevision == entry.context_configuration_revision else {
            throw IOSApplicationRuntimeError.invalidResponse
        }
        let prepared = try IOSPreparedRunModel.prepareExpert(entry, using: runtime)
        let invocation = try prepared.prepareHostInvocation()
        let plan = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal,
            expectedIdentity: .init(hostId: "ios-local", clientId: Bundle.main.bundleIdentifier ?? "nola-ios", sessionId: context),
            isForeground: UIApplication.shared.applicationState == .active, scopedGrants: [], applicationRuntime: runtime, includeExpertTasks: false)
        let application = IOSCanonicalAgentApplication(store: runtime, descriptor: .init(conversationID: context,
            conversationTitle: conversation.title ?? "", identity: .init(userMessageID: UUID()), userText: "", userContentJSON: nil,
            contextCheckpointID: nil))
        let client: TextAgentModelClient
        if let facts = prepared.frozenModelCapability { client = try TextAgentModelClient(client: prepared.client, frozenModelCapability: facts) }
        else { client = TextAgentModelClient(client: prepared.client) }
        var leafSettings = try prepared.mobileDelegationSettings(projectRoot: runtime.databaseURL.deletingLastPathComponent().path,
            conversationID: context, using: runtime)
        if case .object(var fields)? = leafSettings { fields["expert_leaf"] = .bool(true); leafSettings = .object(fields) }
        let coordinator = DeviceAgentCoordinator(modelClient: client,
            delegationSettings: leafSettings, hostExecutor: hostExecutor, toolPlan: plan,
            modelSupportsToolCalls: prepared.supportsToolCalls, canonicalApplication: application, preparedHostInvocation: invocation,
            hostDependencyIdentity: IOSPreparedHostInvocation.skillIdentity(skills))
        return try await coordinator.prepareHostConfiguration(sessionID: context,
            config: .init(platform: .ios, executionMode: .deviceLocal, hostId: "ios-local", model: prepared.modelName,
                maxIterations: 0, contextWindowTokens: prepared.parameters.maxContextLength,
                reservedOutputTokens: prepared.parameters.maxOutputTokens, grantedCapabilities: [], permissionProfile: conversation.permissionMode))
    }
}
