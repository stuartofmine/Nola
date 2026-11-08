// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Projects typed Core Skill runtime events on iOS without parsing Trace or user text.
// - Safety: Session, Turn, cursor, revision, owner, usage, and terminal state are monotonic.

import Foundation

nonisolated public enum AgentCoreSkillToolReference: Codable, Equatable, Hashable, Sendable {
    case builtin(toolName: String)
    case mcp(serverID: String, toolName: String)

    private enum CodingKeys: String, CodingKey {
        case provider
        case toolName = "tool_name"
        case serverID = "server_id"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .provider) {
        case "builtin":
            self = .builtin(toolName: try container.decode(String.self, forKey: .toolName))
        case "mcp":
            self = .mcp(
                serverID: try container.decode(String.self, forKey: .serverID),
                toolName: try container.decode(String.self, forKey: .toolName)
            )
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .provider,
                in: container,
                debugDescription: "Unknown Core capability provider"
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .builtin(let toolName):
            try container.encode("builtin", forKey: .provider)
            try container.encode(toolName, forKey: .toolName)
        case .mcp(let serverID, let toolName):
            try container.encode("mcp", forKey: .provider)
            try container.encode(serverID, forKey: .serverID)
            try container.encode(toolName, forKey: .toolName)
        }
    }
}

nonisolated public enum AgentCoreSkillExecutionMode: String, Codable, Sendable {
    case inline, fork
}

nonisolated public enum AgentCoreSkillRunStatus: String, Codable, Sendable {
    case running
    case waitingForCatalog = "waiting_for_catalog"
    case completed, failed, cancelled

    public var isTerminal: Bool {
        self == .completed || self == .failed || self == .cancelled
    }
}

nonisolated public struct AgentCoreSkillDescriptor: Codable, Equatable, Sendable {
    public let skillID: String
    public let version: String
    public let displayName: String
    public let instructionsRef: String
    public let instructionsDigest: String
    public let requiredTools: [AgentCoreSkillToolReference]
    public let defaultMode: AgentCoreSkillExecutionMode?
    public let executionContract: AgentCoreSkillExecutionContract

    private enum CodingKeys: String, CodingKey {
        case version
        case skillID = "skill_id"
        case displayName = "display_name"
        case instructionsRef = "instructions_ref"
        case instructionsDigest = "instructions_digest"
        case requiredTools = "required_tools"
        case defaultMode = "default_mode"
        case executionContract = "execution_contract"
    }

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        skillID = try values.decode(String.self, forKey: .skillID)
        version = try values.decode(String.self, forKey: .version)
        displayName = try values.decode(String.self, forKey: .displayName)
        instructionsRef = try values.decode(String.self, forKey: .instructionsRef)
        instructionsDigest = try values.decode(String.self, forKey: .instructionsDigest)
        requiredTools = try values.decode([AgentCoreSkillToolReference].self, forKey: .requiredTools)
        defaultMode = try values.decodeIfPresent(AgentCoreSkillExecutionMode.self, forKey: .defaultMode)
        executionContract = try values.decode(AgentCoreSkillExecutionContract.self, forKey: .executionContract)
        guard instructionsDigest.utf8.count == 64,
              instructionsDigest.allSatisfy({ "0123456789abcdef".contains($0) }),
              instructionsRef.hasSuffix("#sha256=\(instructionsDigest)"), executionContract.isValid else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Invalid current Skill content identity or execution contract"))
        }
    }
}

nonisolated public struct AgentCoreSkillExecutionContract: Codable, Equatable, Sendable {
    public let schemaVersion: UInt32
    public let progressSteps: [ProgressStep]
    public let artifacts: [Artifact]
    public let verifiers: [Verifier]
    public let resumePoints: [String]
    public struct ProgressStep: Codable, Equatable, Sendable {
        public let id: String
        public let title: String
        public let required: Bool
    }
    public struct Artifact: Codable, Equatable, Sendable {
        public let id: String
        public let mediaType: String
        public let required: Bool
        public let description: String
    }
    public struct Verifier: Codable, Equatable, Sendable {
        public let id: String
        public let description: String
        public let method: AgentCoreSkillVerificationEvidence.Kind
        public let required: Bool
    }
    fileprivate var isValid: Bool {
        func nonempty(_ value: String) -> Bool { !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        return schemaVersion == 1
            && Set(progressSteps.map(\.id)).count == progressSteps.count
            && progressSteps.allSatisfy { nonempty($0.id) && nonempty($0.title) }
            && Set(artifacts.map(\.id)).count == artifacts.count
            && artifacts.allSatisfy { nonempty($0.id) && nonempty($0.mediaType) }
            && Set(verifiers.map(\.id)).count == verifiers.count
            && verifiers.allSatisfy { nonempty($0.id) && nonempty($0.description) }
            && Set(resumePoints).count == resumePoints.count && resumePoints.allSatisfy(nonempty)
    }
}

nonisolated public struct AgentCoreSkillEvidenceSource: Codable, Equatable, Sendable {
    public enum Kind: String, Codable, Sendable { case executionEvidence = "execution_evidence", toolCall = "tool_call" }
    public let kind: Kind
    public let evidenceID: String?
    public let callID: String?
    private enum CodingKeys: String, CodingKey { case kind, evidenceID = "evidence_id", callID = "call_id" }
    fileprivate var isValid: Bool {
        let identity = kind == .executionEvidence ? evidenceID : callID
        return identity.map { !$0.isEmpty && $0 == $0.trimmingCharacters(in: .whitespacesAndNewlines) && $0.utf8.count <= 256 } == true
            && (kind == .executionEvidence ? callID == nil : evidenceID == nil)
    }
}

nonisolated public struct AgentCoreSkillVerificationEvidence: Codable, Equatable, Sendable {
    public enum Kind: String, Codable, Sendable { case modelJudgment = "model_judgment", hostCheck = "host_check" }
    public enum CheckKind: String, Codable, Sendable { case processExit = "process_exit", postcondition }
    public let kind: Kind
    public let sources: [AgentCoreSkillEvidenceSource]?
    public let source: AgentCoreSkillEvidenceSource?
    public let resourceURI: String?
    public let version: String?
    public let checkKind: CheckKind?
    public let predicate: String?
    private enum CodingKeys: String, CodingKey {
        case kind, sources, source, version, predicate
        case resourceURI = "resource_uri", checkKind = "check_kind"
    }
    fileprivate var isValid: Bool {
        switch kind {
        case .modelJudgment:
            guard let sources, sources.count <= 16 else { return false }
            return sources.allSatisfy(\.isValid) && source == nil && resourceURI == nil && version == nil && checkKind == nil && predicate == nil
        case .hostCheck:
            return sources == nil && source?.isValid == true && checkKind != nil
                && [resourceURI, version, predicate].allSatisfy { $0?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false }
        }
    }
}

nonisolated public struct AgentCoreSkillVerificationResult: Codable, Equatable, Sendable {
    public let verifierID: String
    public let passed: Bool
    public let evidence: AgentCoreSkillVerificationEvidence
    public let replacesEvidence: AgentCoreSkillVerificationEvidence?
    public let summary: String
    private enum CodingKeys: String, CodingKey { case passed, evidence, replacesEvidence, summary, verifierID = "verifierId" }
}

nonisolated public struct AgentCoreSkillRunBudget: Codable, Equatable, Sendable {
    public let maxModelTurns: UInt32
    public let maxToolCalls: UInt32
    public let maxElapsedMilliseconds: UInt64?

    private enum CodingKeys: String, CodingKey {
        case maxModelTurns = "max_model_turns"
        case maxToolCalls = "max_tool_calls"
        case maxElapsedMilliseconds = "max_elapsed_millis"
    }
}

nonisolated public struct AgentCoreSkillRunUsage: Codable, Equatable, Sendable {
    public let modelTurns: UInt32
    public let toolCalls: UInt32
    public let elapsedMilliseconds: UInt64

    private enum CodingKeys: String, CodingKey {
        case modelTurns = "model_turns"
        case toolCalls = "tool_calls"
        case elapsedMilliseconds = "elapsed_millis"
    }

    fileprivate func isMonotonic(after old: Self) -> Bool {
        modelTurns >= old.modelTurns
            && toolCalls >= old.toolCalls
            && elapsedMilliseconds >= old.elapsedMilliseconds
    }
}

nonisolated public struct AgentCoreSkillRunOwner: Codable, Equatable, Sendable {
    public let sessionID: String
    public let parentTurnID: String
    public let childTurnID: String?

    private enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case parentTurnID = "parent_turn_id"
        case childTurnID = "child_turn_id"
    }
}

nonisolated public struct AgentCoreSkillRun: Codable, Equatable, Sendable, Identifiable {
    public let runID: String
    public let owner: AgentCoreSkillRunOwner
    public let descriptor: AgentCoreSkillDescriptor
    public let mode: AgentCoreSkillExecutionMode
    public fileprivate(set) var status: AgentCoreSkillRunStatus
    public let budget: AgentCoreSkillRunBudget
    public fileprivate(set) var usage: AgentCoreSkillRunUsage
    public fileprivate(set) var missingTools: [AgentCoreSkillToolReference]
    public fileprivate(set) var catalogRevision: UInt64
    public fileprivate(set) var resultRef: String?
    public fileprivate(set) var failure: String?
    public fileprivate(set) var currentProgressStep: String?
    public fileprivate(set) var completedProgressSteps: Set<String>
    public fileprivate(set) var artifacts: [String: String]
    public fileprivate(set) var verifications: [String: AgentCoreSkillVerificationResult]
    public fileprivate(set) var resumePoint: String?

    public var id: String { runID }

    private enum CodingKeys: String, CodingKey {
        case owner, descriptor, mode, status, budget, usage, failure
        case runID = "run_id"
        case missingTools = "missing_tools"
        case catalogRevision = "catalog_revision"
        case resultRef = "result_ref"
        case currentProgressStep = "current_progress_step"
        case completedProgressSteps = "completed_progress_steps"
        case artifacts, verifications
        case resumePoint = "resume_point"
    }
}

nonisolated public enum AgentCoreSkillRuntimeUpdate: Codable, Equatable, Sendable {
    case catalogReplaced(revision: UInt64)
    case started(run: AgentCoreSkillRun)
    case capabilityRefreshRequested(runID: String)
    case resumed(runID: String, catalogRevision: UInt64)
    case usageUpdated(runID: String, usage: AgentCoreSkillRunUsage)
    case progressUpdated(runID: String, stepID: String, completed: Bool)
    case artifactRegistered(runID: String, artifactID: String, artifactRef: String)
    case verificationRecorded(runID: String, result: AgentCoreSkillVerificationResult)
    case resumePointUpdated(runID: String, resumePoint: String)
    case budgetExceeded(runID: String)
    case completed(runID: String, resultRef: String?)
    case failed(runID: String, error: String)
    case cancelled(runID: String, reason: String?)

    private enum CodingKeys: String, CodingKey {
        case type, revision, run, usage, error, reason, completed, result
        case runID = "run_id"
        case catalogRevision = "catalog_revision"
        case resultRef = "result_ref"
        case stepID = "step_id", artifactID = "artifact_id", artifactRef = "artifact_ref", resumePoint = "resume_point"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "catalog_replaced": self = .catalogReplaced(revision: try container.decode(UInt64.self, forKey: .revision))
        case "started": self = .started(run: try container.decode(AgentCoreSkillRun.self, forKey: .run))
        case "capability_refresh_requested": self = .capabilityRefreshRequested(runID: try container.decode(String.self, forKey: .runID))
        case "resumed": self = .resumed(runID: try container.decode(String.self, forKey: .runID), catalogRevision: try container.decode(UInt64.self, forKey: .catalogRevision))
        case "usage_updated": self = .usageUpdated(runID: try container.decode(String.self, forKey: .runID), usage: try container.decode(AgentCoreSkillRunUsage.self, forKey: .usage))
        case "progress_updated": self = .progressUpdated(runID: try container.decode(String.self, forKey: .runID), stepID: try container.decode(String.self, forKey: .stepID), completed: try container.decode(Bool.self, forKey: .completed))
        case "artifact_registered": self = .artifactRegistered(runID: try container.decode(String.self, forKey: .runID), artifactID: try container.decode(String.self, forKey: .artifactID), artifactRef: try container.decode(String.self, forKey: .artifactRef))
        case "verification_recorded": self = .verificationRecorded(runID: try container.decode(String.self, forKey: .runID), result: try container.decode(AgentCoreSkillVerificationResult.self, forKey: .result))
        case "resume_point_updated": self = .resumePointUpdated(runID: try container.decode(String.self, forKey: .runID), resumePoint: try container.decode(String.self, forKey: .resumePoint))
        case "budget_exceeded": self = .budgetExceeded(runID: try container.decode(String.self, forKey: .runID))
        case "completed": self = .completed(runID: try container.decode(String.self, forKey: .runID), resultRef: try container.decodeIfPresent(String.self, forKey: .resultRef))
        case "failed": self = .failed(runID: try container.decode(String.self, forKey: .runID), error: try container.decode(String.self, forKey: .error))
        case "cancelled": self = .cancelled(runID: try container.decode(String.self, forKey: .runID), reason: try container.decodeIfPresent(String.self, forKey: .reason))
        default:
            throw DecodingError.dataCorruptedError(forKey: .type, in: container, debugDescription: "Unknown Core Skill runtime update")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .catalogReplaced(let revision):
            try container.encode("catalog_replaced", forKey: .type); try container.encode(revision, forKey: .revision)
        case .started(let run):
            try container.encode("started", forKey: .type); try container.encode(run, forKey: .run)
        case .capabilityRefreshRequested(let runID):
            try container.encode("capability_refresh_requested", forKey: .type); try container.encode(runID, forKey: .runID)
        case .resumed(let runID, let revision):
            try container.encode("resumed", forKey: .type); try container.encode(runID, forKey: .runID); try container.encode(revision, forKey: .catalogRevision)
        case .usageUpdated(let runID, let usage):
            try container.encode("usage_updated", forKey: .type); try container.encode(runID, forKey: .runID); try container.encode(usage, forKey: .usage)
        case .budgetExceeded(let runID):
            try container.encode("budget_exceeded", forKey: .type); try container.encode(runID, forKey: .runID)
        case .progressUpdated(let runID, let stepID, let completed):
            try container.encode("progress_updated", forKey: .type); try container.encode(runID, forKey: .runID)
            try container.encode(stepID, forKey: .stepID); try container.encode(completed, forKey: .completed)
        case .artifactRegistered(let runID, let artifactID, let artifactRef):
            try container.encode("artifact_registered", forKey: .type); try container.encode(runID, forKey: .runID)
            try container.encode(artifactID, forKey: .artifactID); try container.encode(artifactRef, forKey: .artifactRef)
        case .verificationRecorded(let runID, let result):
            try container.encode("verification_recorded", forKey: .type); try container.encode(runID, forKey: .runID); try container.encode(result, forKey: .result)
        case .resumePointUpdated(let runID, let resumePoint):
            try container.encode("resume_point_updated", forKey: .type); try container.encode(runID, forKey: .runID); try container.encode(resumePoint, forKey: .resumePoint)
        case .completed(let runID, let resultRef):
            try container.encode("completed", forKey: .type); try container.encode(runID, forKey: .runID); try container.encodeIfPresent(resultRef, forKey: .resultRef)
        case .failed(let runID, let error):
            try container.encode("failed", forKey: .type); try container.encode(runID, forKey: .runID); try container.encode(error, forKey: .error)
        case .cancelled(let runID, let reason):
            try container.encode("cancelled", forKey: .type); try container.encode(runID, forKey: .runID); try container.encodeIfPresent(reason, forKey: .reason)
        }
    }
}

nonisolated public struct AgentCoreSkillRuntimeUpdateEnvelope: Codable, Equatable, Sendable {
    public let protocolVersion: UInt32
    public let type: String
    public let sessionID: String
    public let turnID: String?
    public let cursor: UInt64
    public let revision: UInt64
    public let skillRuntimeRevision: UInt64
    public let update: AgentCoreSkillRuntimeUpdate

    private enum CodingKeys: String, CodingKey {
        case type, cursor, revision, update
        case protocolVersion = "protocol_version"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case skillRuntimeRevision = "skill_runtime_revision"
    }

    public static func decode(from value: AgentCoreJSONValue) -> Self? {
        guard let data = try? JSONEncoder().encode(value),
              let envelope = try? JSONDecoder().decode(Self.self, from: data),
              envelope.protocolVersion == 2,
              envelope.type == "core_skill_runtime_updated" else { return nil }
        return envelope
    }
}

nonisolated public struct AgentCoreCapabilityActivationEnvelope: Codable, Equatable, Sendable {
    public let protocolVersion: UInt32
    public let type: String
    public let sessionID: String
    public let turnID: String?
    public let cursor: UInt64
    public let revision: UInt64
    public let source: String
    public let capabilityIDs: [String]

    private enum CodingKeys: String, CodingKey {
        case type, cursor, revision, source
        case protocolVersion = "protocol_version"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case capabilityIDs = "capability_ids"
    }

    public static func decode(from value: AgentCoreJSONValue) -> Self? {
        guard let data = try? JSONEncoder().encode(value),
              let envelope = try? JSONDecoder().decode(Self.self, from: data),
              envelope.type == "core_capabilities_activated",
              envelope.protocolVersion == 2,
              !envelope.sessionID.isEmpty,
              !envelope.source.isEmpty,
              !envelope.capabilityIDs.isEmpty,
              envelope.capabilityIDs.allSatisfy({ !$0.isEmpty }) else { return nil }
        return envelope
    }

    public var hostToolEvent: HostAgentToolEvent {
        HostAgentToolEvent(
            id: "core-capability:\(turnID ?? sessionID):\(source)",
            name: source,
            status: .succeeded,
            display: capabilityIDs.joined(separator: " · "),
            text: capabilityIDs.joined(separator: "\n")
        )
    }
}

nonisolated public struct AgentCoreSkillRuntimeProjection: Codable, Equatable, Sendable {
    public fileprivate(set) var sessionID: String?
    public fileprivate(set) var turnID: String?
    public fileprivate(set) var cursor: UInt64 = 0
    public fileprivate(set) var revision: UInt64 = 0
    public fileprivate(set) var journalRevision: UInt64 = 0
    public fileprivate(set) var catalogRevision: UInt64 = 0
    public fileprivate(set) var order: [String] = []
    public fileprivate(set) var runs: [String: AgentCoreSkillRun] = [:]

    public init() {}
    public var orderedRuns: [AgentCoreSkillRun] { order.compactMap { runs[$0] } }

    public var hostToolEvents: [HostAgentToolEvent] {
        orderedRuns.map { run in
            let status: HostAgentToolEvent.Status = switch run.status {
            case .running, .waitingForCatalog: .running
            case .completed: .succeeded
            case .failed: .failed
            case .cancelled: .cancelled
            }
            return HostAgentToolEvent(
                id: "core-skill:\(run.runID)",
                name: run.descriptor.displayName,
                status: status,
                display: run.descriptor.displayName,
                text: run.resultRef ?? run.failure ?? ""
            )
        }
    }

    public func mergingHostToolEvents(_ existing: [HostAgentToolEvent]) -> [HostAgentToolEvent] {
        var merged = existing.filter { !$0.id.hasPrefix("core-skill:") }
        for event in hostToolEvents {
            if let index = merged.firstIndex(where: { $0.id == event.id }) {
                merged[index] = event
            } else {
                merged.append(event)
            }
        }
        return merged
    }
}

nonisolated public enum AgentCoreSkillRuntimeApplyResult: Equatable, Sendable {
    case applied
    case duplicate
    case rejected
}

nonisolated public struct AgentCoreSkillRuntimeProjectionReducer: Sendable {
    public private(set) var projection = AgentCoreSkillRuntimeProjection()

    public init(projection: AgentCoreSkillRuntimeProjection = .init()) {
        self.projection = projection
    }

    public mutating func apply(
        _ envelope: AgentCoreSkillRuntimeUpdateEnvelope,
        expectedSessionID: String,
        expectedTurnID: String? = nil
    ) -> AgentCoreSkillRuntimeApplyResult {
        guard envelope.type == "core_skill_runtime_updated",
              envelope.protocolVersion == 2,
              !expectedSessionID.isEmpty,
              envelope.sessionID == expectedSessionID,
              projection.sessionID == nil || projection.sessionID == envelope.sessionID,
              expectedTurnID == nil || envelope.turnID == expectedTurnID,
              projection.turnID == nil || projection.turnID == envelope.turnID,
              envelope.skillRuntimeRevision >= projection.revision,
              envelope.revision >= projection.journalRevision else { return .rejected }
        guard envelope.cursor > projection.cursor else { return .duplicate }

        var next = projection
        guard Self.reduce(envelope.update, envelope: envelope, projection: &next) else { return .rejected }
        next.sessionID = envelope.sessionID
        next.turnID = envelope.turnID
        next.cursor = envelope.cursor
        next.revision = envelope.skillRuntimeRevision
        next.journalRevision = envelope.revision
        projection = next
        return .applied
    }

    private static func reduce(
        _ update: AgentCoreSkillRuntimeUpdate,
        envelope: AgentCoreSkillRuntimeUpdateEnvelope,
        projection: inout AgentCoreSkillRuntimeProjection
    ) -> Bool {
        switch update {
        case .catalogReplaced(let revision):
            guard revision >= projection.catalogRevision else { return false }
            projection.catalogRevision = revision
        case .started(let run):
            guard run.runID.isEmpty == false,
                  run.descriptor.executionContract.schemaVersion == 1,
                  run.descriptor.instructionsDigest.count == 64,
                  run.descriptor.instructionsDigest.allSatisfy({ "0123456789abcdef".contains($0) }),
                  run.owner.sessionID == envelope.sessionID,
                  run.owner.parentTurnID == envelope.turnID,
                  projection.runs[run.runID] == nil else { return false }
            projection.runs[run.runID] = run
            projection.order.append(run.runID)
        case .capabilityRefreshRequested(let runID):
            guard var run = activeRun(runID, in: projection) else { return false }
            run.status = .waitingForCatalog
            projection.runs[runID] = run
        case .resumed(let runID, let catalogRevision):
            guard var run = activeRun(runID, in: projection), catalogRevision >= run.catalogRevision else { return false }
            run.status = .running
            run.catalogRevision = catalogRevision
            run.missingTools = []
            projection.catalogRevision = max(projection.catalogRevision, catalogRevision)
            projection.runs[runID] = run
        case .usageUpdated(let runID, let usage):
            guard var run = activeRun(runID, in: projection), usage.isMonotonic(after: run.usage) else { return false }
            run.usage = usage
            projection.runs[runID] = run
        case .progressUpdated(let runID, let stepID, let completed):
            guard var run = activeRun(runID, in: projection),
                  run.descriptor.executionContract.progressSteps.contains(where: { $0.id == stepID }) else { return false }
            if completed { run.completedProgressSteps.insert(stepID) }
            else { run.completedProgressSteps.remove(stepID) }
            run.currentProgressStep = completed
                ? run.descriptor.executionContract.progressSteps.first(where: { !run.completedProgressSteps.contains($0.id) })?.id
                : stepID
            projection.runs[runID] = run
        case .artifactRegistered(let runID, let artifactID, let artifactRef):
            guard var run = activeRun(runID, in: projection), !artifactRef.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  run.descriptor.executionContract.artifacts.contains(where: { $0.id == artifactID }),
                  run.artifacts[artifactID] == nil || run.artifacts[artifactID] == artifactRef else { return false }
            run.artifacts[artifactID] = artifactRef
            projection.runs[runID] = run
        case .verificationRecorded(let runID, let result):
            guard var run = activeRun(runID, in: projection), result.evidence.isValid,
                  result.replacesEvidence?.isValid ?? true,
                  run.descriptor.executionContract.verifiers.contains(where: { $0.id == result.verifierID && $0.method == result.evidence.kind }) else { return false }
            if let old = run.verifications[result.verifierID] {
                guard old == result || result.replacesEvidence == old.evidence else { return false }
            } else if result.replacesEvidence != nil { return false }
            run.verifications[result.verifierID] = result
            projection.runs[runID] = run
        case .resumePointUpdated(let runID, let resumePoint):
            guard var run = activeRun(runID, in: projection),
                  run.descriptor.executionContract.resumePoints.contains(resumePoint) else { return false }
            run.resumePoint = resumePoint
            projection.runs[runID] = run
        case .budgetExceeded(let runID):
            guard var run = activeRun(runID, in: projection) else { return false }
            run.status = .failed
            run.failure = "skill_budget_exceeded"
            projection.runs[runID] = run
        case .completed(let runID, let resultRef):
            guard var run = activeRun(runID, in: projection) else { return false }
            run.status = .completed
            run.resultRef = resultRef
            projection.runs[runID] = run
        case .failed(let runID, let error):
            guard var run = activeRun(runID, in: projection) else { return false }
            run.status = .failed
            run.failure = error
            projection.runs[runID] = run
        case .cancelled(let runID, let reason):
            guard var run = activeRun(runID, in: projection) else { return false }
            run.status = .cancelled
            run.failure = reason
            projection.runs[runID] = run
        }
        return true
    }

    private static func activeRun(_ runID: String, in projection: AgentCoreSkillRuntimeProjection) -> AgentCoreSkillRun? {
        guard let run = projection.runs[runID], !run.status.isTerminal else { return nil }
        return run
    }
}

nonisolated public enum AgentCoreSkillProjectionMetadata {
    public static let key = "agentCoreSkillRuntimeProjection"

    public static func encode(_ projection: AgentCoreSkillRuntimeProjection, into metadata: [String: String]) -> [String: String] {
        guard let data = try? JSONEncoder().encode(projection) else { return metadata }
        var next = metadata
        next[key] = data.base64EncodedString()
        return next
    }

    public static func decode(from metadata: [String: String]) -> AgentCoreSkillRuntimeProjection? {
        guard let encoded = metadata[key], let data = Data(base64Encoded: encoded) else { return nil }
        return try? JSONDecoder().decode(AgentCoreSkillRuntimeProjection.self, from: data)
    }
}
