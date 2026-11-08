// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Mirrors Core capability-refresh and Skill runtime protocol objects for the native macOS Host.
// - Safety: Correlation uses Core request/effect/lease identities and monotonic cursors; user text is never inspected.

import Foundation

nonisolated indirect enum MacCoreCapabilityJSONValue: Codable, Equatable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: MacCoreCapabilityJSONValue])
    case array([MacCoreCapabilityJSONValue])
    case null

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null }
        else if let value = try? container.decode(Bool.self) { self = .bool(value) }
        else if let value = try? container.decode(Double.self) { self = .number(value) }
        else if let value = try? container.decode(String.self) { self = .string(value) }
        else if let value = try? container.decode([String: MacCoreCapabilityJSONValue].self) {
            self = .object(value)
        } else {
            self = .array(try container.decode([MacCoreCapabilityJSONValue].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

nonisolated enum MacCoreCapabilityLookupKind: String, Codable, Equatable, Sendable {
    case server, tool, resource, prompt
}

nonisolated enum MacCoreCapabilityRecovery: String, Codable, Equatable, Sendable {
    case refreshCatalogThenRetry = "refresh_catalog_then_retry"
}

nonisolated struct MacCoreRecoverableCapabilityError: Codable, Equatable, Sendable {
    let code: String
    let kind: MacCoreCapabilityLookupKind
    let requestedID: String
    let serverID: String?
    let catalogRevision: UInt64
    let availableIDs: [String]
    let recovery: MacCoreCapabilityRecovery

    enum CodingKeys: String, CodingKey {
        case code, kind, recovery
        case requestedID = "requested_id"
        case serverID = "server_id"
        case catalogRevision = "catalog_revision"
        case availableIDs = "available_ids"
    }
}

nonisolated struct MacCoreMCPCapabilityCatalog: Codable, Equatable, Sendable {
    let revision: UInt64
    let servers: [String: MacCoreCapabilityJSONValue]

    init(revision: UInt64 = 0, servers: [String: MacCoreCapabilityJSONValue] = [:]) {
        self.revision = revision
        self.servers = servers
    }
}

nonisolated struct MacCoreCapabilityCatalog: Codable, Equatable, Sendable {
    let revision: UInt64
    let builtinTools: Set<String>
    let mcp: MacCoreMCPCapabilityCatalog

    init(
        revision: UInt64,
        builtinTools: Set<String>,
        mcp: MacCoreMCPCapabilityCatalog = .init()
    ) {
        self.revision = revision
        self.builtinTools = builtinTools
        self.mcp = mcp
    }

    enum CodingKeys: String, CodingKey {
        case revision, mcp
        case builtinTools = "builtin_tools"
    }
}

nonisolated struct MacCoreRefreshCapabilityCatalogPayload: Codable, Equatable, Sendable {
    let runID: String
    let catalogRevision: UInt64
    let errors: [MacCoreRecoverableCapabilityError]

    enum CodingKeys: String, CodingKey {
        case type
        case runID = "run_id"
        case catalogRevision = "catalog_revision"
        case errors
    }

    init(runID: String, catalogRevision: UInt64, errors: [MacCoreRecoverableCapabilityError]) {
        self.runID = runID
        self.catalogRevision = catalogRevision
        self.errors = errors
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        guard try container.decode(String.self, forKey: .type) == "refresh_capability_catalog" else {
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "Expected Core refresh_capability_catalog request"
            )
        }
        runID = try container.decode(String.self, forKey: .runID)
        catalogRevision = try container.decode(UInt64.self, forKey: .catalogRevision)
        errors = try container.decode([MacCoreRecoverableCapabilityError].self, forKey: .errors)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("refresh_capability_catalog", forKey: .type)
        try container.encode(runID, forKey: .runID)
        try container.encode(catalogRevision, forKey: .catalogRevision)
        try container.encode(errors, forKey: .errors)
    }
}

nonisolated struct MacCoreRefreshCapabilityCatalogHostRequest: Codable, Equatable, Sendable {
    let requestID: String
    let effectID: String
    let hostID: String
    let leaseID: String?
    let sessionID: String
    let turnID: String?
    let catalogRevision: UInt64
    let availabilityEpoch: UInt64
    let createdAtMilliseconds: UInt64
    let deadlineMilliseconds: UInt64?
    let resumeAfterSequence: UInt64?
    let payload: MacCoreRefreshCapabilityCatalogPayload

    enum CodingKeys: String, CodingKey {
        case payload
        case requestID = "request_id"
        case effectID = "effect_id"
        case hostID = "host_id"
        case leaseID = "lease_id"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case catalogRevision = "catalog_revision"
        case availabilityEpoch = "availability_epoch"
        case createdAtMilliseconds = "created_at_ms"
        case deadlineMilliseconds = "deadline_ms"
        case resumeAfterSequence = "resume_after_sequence"
    }
}

nonisolated struct MacCoreCapabilityCatalogRefreshedPayload: Codable, Equatable, Sendable {
    let catalog: MacCoreCapabilityCatalog

    enum CodingKeys: String, CodingKey { case type, catalog }

    init(catalog: MacCoreCapabilityCatalog) { self.catalog = catalog }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        guard try container.decode(String.self, forKey: .type) == "capability_catalog_refreshed" else {
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "Expected Core capability_catalog_refreshed response"
            )
        }
        catalog = try container.decode(MacCoreCapabilityCatalog.self, forKey: .catalog)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode("capability_catalog_refreshed", forKey: .type)
        try container.encode(catalog, forKey: .catalog)
    }
}

nonisolated struct MacCoreCapabilityCatalogRefreshedHostResponse: Codable, Equatable, Sendable {
    let requestID: String
    let effectID: String
    let hostID: String
    let leaseID: String?
    let sessionID: String
    let turnID: String?
    let catalogRevision: UInt64
    let availabilityEpoch: UInt64
    let sequence: UInt64
    let payload: MacCoreCapabilityCatalogRefreshedPayload

    enum CodingKeys: String, CodingKey {
        case payload, sequence
        case requestID = "request_id"
        case effectID = "effect_id"
        case hostID = "host_id"
        case leaseID = "lease_id"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case catalogRevision = "catalog_revision"
        case availabilityEpoch = "availability_epoch"
    }

    static func responding(
        to request: MacCoreRefreshCapabilityCatalogHostRequest,
        with catalog: MacCoreCapabilityCatalog
    ) throws -> Self {
        guard !request.requestID.isEmpty,
              !request.effectID.isEmpty,
              !request.hostID.isEmpty,
              !request.sessionID.isEmpty,
              !request.payload.runID.isEmpty,
              catalog.revision >= request.payload.catalogRevision else {
            throw MacCoreCapabilityRefreshContractError.invalidRequest
        }
        return Self(
            requestID: request.requestID,
            effectID: request.effectID,
            hostID: request.hostID,
            leaseID: request.leaseID,
            sessionID: request.sessionID,
            turnID: request.turnID,
            catalogRevision: request.catalogRevision,
            availabilityEpoch: request.availabilityEpoch,
            sequence: request.resumeAfterSequence.map { $0 &+ 1 } ?? 0,
            payload: .init(catalog: catalog)
        )
    }

    func isCorrelated(to request: MacCoreRefreshCapabilityCatalogHostRequest) -> Bool {
        requestID == request.requestID
            && effectID == request.effectID
            && hostID == request.hostID
            && leaseID == request.leaseID
            && sessionID == request.sessionID
            && turnID == request.turnID
            && catalogRevision == request.catalogRevision
            && availabilityEpoch == request.availabilityEpoch
            && sequence == (request.resumeAfterSequence.map { $0 &+ 1 } ?? 0)
    }
}

nonisolated enum MacCoreCapabilityRefreshContractError: Error, Equatable, Sendable {
    case invalidRequest
}

nonisolated enum MacCoreCapabilityToolReference: Codable, Equatable, Hashable, Sendable {
    case builtin(toolName: String)
    case mcp(serverID: String, toolName: String)

    enum CodingKeys: String, CodingKey {
        case provider
        case toolName = "tool_name"
        case serverID = "server_id"
    }

    init(from decoder: Decoder) throws {
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

    func encode(to encoder: Encoder) throws {
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

nonisolated enum MacCoreSkillExecutionMode: String, Codable, Equatable, Hashable, Sendable {
    case inline, fork
}

nonisolated enum MacCoreSkillRunStatus: String, Codable, Equatable, Hashable, Sendable {
    case running
    case waitingForCatalog = "waiting_for_catalog"
    case completed, failed, cancelled

    var isTerminal: Bool { self == .completed || self == .failed || self == .cancelled }
}

nonisolated struct MacCoreSkillDescriptor: Codable, Equatable, Hashable, Sendable {
    let skillID: String
    let version: String
    let displayName: String
    let instructionsRef: String
    let instructionsDigest: String
    let requiredTools: [MacCoreCapabilityToolReference]
    let defaultMode: MacCoreSkillExecutionMode?
    let executionContract: MacCoreSkillExecutionContract

    enum CodingKeys: String, CodingKey {
        case version
        case skillID = "skill_id"
        case displayName = "display_name"
        case instructionsRef = "instructions_ref"
        case instructionsDigest = "instructions_digest"
        case requiredTools = "required_tools"
        case defaultMode = "default_mode"
        case executionContract = "execution_contract"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        skillID = try values.decode(String.self, forKey: .skillID)
        version = try values.decode(String.self, forKey: .version)
        displayName = try values.decode(String.self, forKey: .displayName)
        instructionsRef = try values.decode(String.self, forKey: .instructionsRef)
        instructionsDigest = try values.decode(String.self, forKey: .instructionsDigest)
        requiredTools = try values.decode([MacCoreCapabilityToolReference].self, forKey: .requiredTools)
        defaultMode = try values.decodeIfPresent(MacCoreSkillExecutionMode.self, forKey: .defaultMode)
        executionContract = try values.decode(MacCoreSkillExecutionContract.self, forKey: .executionContract)
        guard instructionsDigest.utf8.count == 64,
              instructionsDigest.allSatisfy({ "0123456789abcdef".contains($0) }),
              instructionsRef.hasSuffix("#sha256=\(instructionsDigest)"),
              executionContract.isValid else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath,
                debugDescription: "Skill descriptor lacks a valid immutable content identity or execution contract."))
        }
    }
}

nonisolated struct MacCoreSkillExecutionContract: Codable, Equatable, Hashable, Sendable {
    let schemaVersion: UInt32
    let progressSteps: [ProgressStep]
    let artifacts: [Artifact]
    let verifiers: [Verifier]
    let resumePoints: [String]

    struct ProgressStep: Codable, Equatable, Hashable, Sendable {
        let id: String
        let title: String
        let required: Bool
    }
    struct Artifact: Codable, Equatable, Hashable, Sendable {
        let id: String
        let mediaType: String
        let required: Bool
        let description: String
    }
    struct Verifier: Codable, Equatable, Hashable, Sendable {
        let id: String
        let description: String
        let method: MacCoreSkillVerificationEvidence.Kind
        let required: Bool
    }

    var isValid: Bool {
        func nonempty(_ value: String) -> Bool {
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
        return schemaVersion == 1
            && Set(progressSteps.map(\.id)).count == progressSteps.count
            && progressSteps.allSatisfy { nonempty($0.id) && nonempty($0.title) }
            && Set(artifacts.map(\.id)).count == artifacts.count
            && artifacts.allSatisfy { nonempty($0.id) && nonempty($0.mediaType) }
            && Set(verifiers.map(\.id)).count == verifiers.count
            && verifiers.allSatisfy { nonempty($0.id) && nonempty($0.description) }
            && Set(resumePoints).count == resumePoints.count
            && resumePoints.allSatisfy(nonempty)
    }
}

nonisolated struct MacCoreSkillRunBudget: Codable, Equatable, Hashable, Sendable {
    let maxModelTurns: UInt32
    let maxToolCalls: UInt32
    let maxElapsedMilliseconds: UInt64?

    enum CodingKeys: String, CodingKey {
        case maxModelTurns = "max_model_turns"
        case maxToolCalls = "max_tool_calls"
        case maxElapsedMilliseconds = "max_elapsed_millis"
    }
}

nonisolated struct MacCoreSkillRunUsage: Codable, Equatable, Hashable, Sendable {
    let modelTurns: UInt32
    let toolCalls: UInt32
    let elapsedMilliseconds: UInt64

    enum CodingKeys: String, CodingKey {
        case modelTurns = "model_turns"
        case toolCalls = "tool_calls"
        case elapsedMilliseconds = "elapsed_millis"
    }

    func isMonotonic(after old: Self) -> Bool {
        modelTurns >= old.modelTurns
            && toolCalls >= old.toolCalls
            && elapsedMilliseconds >= old.elapsedMilliseconds
    }
}

nonisolated struct MacCoreSkillEvidenceSource: Codable, Equatable, Hashable, Sendable {
    enum Kind: String, Codable, Hashable, Sendable { case executionEvidence = "execution_evidence", toolCall = "tool_call" }
    let kind: Kind
    let evidenceID: String?
    let callID: String?
    enum CodingKeys: String, CodingKey { case kind, evidenceID = "evidence_id", callID = "call_id" }
    var isValid: Bool {
        switch kind {
        case .executionEvidence: evidenceID?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false && callID == nil
        case .toolCall: callID?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false && evidenceID == nil
        }
    }
}

nonisolated struct MacCoreSkillVerificationEvidence: Codable, Equatable, Hashable, Sendable {
    enum Kind: String, Codable, Hashable, Sendable { case modelJudgment = "model_judgment", hostCheck = "host_check" }
    enum CheckKind: String, Codable, Hashable, Sendable { case processExit = "process_exit", postcondition }
    let kind: Kind
    let sources: [MacCoreSkillEvidenceSource]?
    let source: MacCoreSkillEvidenceSource?
    let resourceURI: String?
    let version: String?
    let checkKind: CheckKind?
    let predicate: String?
    enum CodingKeys: String, CodingKey {
        case kind, sources, source, version, predicate
        case resourceURI = "resource_uri", checkKind = "check_kind"
    }
    var isValid: Bool {
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

nonisolated struct MacCoreSkillVerificationResult: Codable, Equatable, Hashable, Sendable {
    let verifierID: String
    let passed: Bool
    let evidence: MacCoreSkillVerificationEvidence
    let replacesEvidence: MacCoreSkillVerificationEvidence?
    let summary: String

    enum CodingKeys: String, CodingKey {
        case passed, summary, evidence, replacesEvidence
        case verifierID = "verifierId"
    }
}

nonisolated struct MacCoreSkillRunOwner: Codable, Equatable, Hashable, Sendable {
    let sessionID: String
    let parentTurnID: String
    let childTurnID: String?

    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id"
        case parentTurnID = "parent_turn_id"
        case childTurnID = "child_turn_id"
    }
}

nonisolated struct MacCoreSkillRun: Codable, Equatable, Hashable, Sendable {
    let runID: String
    let owner: MacCoreSkillRunOwner
    let descriptor: MacCoreSkillDescriptor
    let mode: MacCoreSkillExecutionMode
    var status: MacCoreSkillRunStatus
    let budget: MacCoreSkillRunBudget
    var usage: MacCoreSkillRunUsage
    var missingTools: [MacCoreCapabilityToolReference]
    var catalogRevision: UInt64
    var resultRef: String?
    var failure: String?
    var currentProgressStep: String?
    var completedProgressSteps: Set<String>
    var artifacts: [String: String]
    var verifications: [String: MacCoreSkillVerificationResult]
    var resumePoint: String?

    enum CodingKeys: String, CodingKey {
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

nonisolated enum MacCoreSkillBudgetDimension: String, Codable, Equatable, Hashable, Sendable {
    case modelTurns = "model_turns"
    case toolCalls = "tool_calls"
    case elapsedMilliseconds = "elapsed_millis"
}

nonisolated enum MacCoreSkillRuntimeEvent: Codable, Equatable, Sendable {
    case catalogReplaced(revision: UInt64)
    case started(run: MacCoreSkillRun)
    case capabilityRefreshRequested(runID: String, errors: [MacCoreRecoverableCapabilityError])
    case resumed(runID: String, catalogRevision: UInt64)
    case usageUpdated(runID: String, usage: MacCoreSkillRunUsage)
    case progressUpdated(runID: String, stepID: String, completed: Bool)
    case artifactRegistered(runID: String, artifactID: String, artifactRef: String)
    case verificationRecorded(runID: String, result: MacCoreSkillVerificationResult)
    case resumePointUpdated(runID: String, resumePoint: String)
    case budgetExceeded(runID: String, dimension: MacCoreSkillBudgetDimension)
    case completed(runID: String, resultRef: String?)
    case failed(runID: String, error: String)
    case cancelled(runID: String, reason: String?)

    enum CodingKeys: String, CodingKey {
        case type, revision, run, errors, usage, completed, result, dimension, error, reason
        case runID = "run_id"
        case catalogRevision = "catalog_revision"
        case stepID = "step_id"
        case artifactID = "artifact_id"
        case artifactRef = "artifact_ref"
        case resumePoint = "resume_point"
        case resultRef = "result_ref"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "catalog_replaced":
            self = .catalogReplaced(revision: try container.decode(UInt64.self, forKey: .revision))
        case "started":
            self = .started(run: try container.decode(MacCoreSkillRun.self, forKey: .run))
        case "capability_refresh_requested":
            self = .capabilityRefreshRequested(
                runID: try container.decode(String.self, forKey: .runID),
                errors: try container.decode([MacCoreRecoverableCapabilityError].self, forKey: .errors)
            )
        case "resumed":
            self = .resumed(
                runID: try container.decode(String.self, forKey: .runID),
                catalogRevision: try container.decode(UInt64.self, forKey: .catalogRevision)
            )
        case "usage_updated":
            self = .usageUpdated(
                runID: try container.decode(String.self, forKey: .runID),
                usage: try container.decode(MacCoreSkillRunUsage.self, forKey: .usage)
            )
        case "progress_updated":
            self = .progressUpdated(
                runID: try container.decode(String.self, forKey: .runID),
                stepID: try container.decode(String.self, forKey: .stepID),
                completed: try container.decode(Bool.self, forKey: .completed)
            )
        case "artifact_registered":
            self = .artifactRegistered(
                runID: try container.decode(String.self, forKey: .runID),
                artifactID: try container.decode(String.self, forKey: .artifactID),
                artifactRef: try container.decode(String.self, forKey: .artifactRef)
            )
        case "verification_recorded":
            self = .verificationRecorded(
                runID: try container.decode(String.self, forKey: .runID),
                result: try container.decode(MacCoreSkillVerificationResult.self, forKey: .result)
            )
        case "resume_point_updated":
            self = .resumePointUpdated(
                runID: try container.decode(String.self, forKey: .runID),
                resumePoint: try container.decode(String.self, forKey: .resumePoint)
            )
        case "budget_exceeded":
            self = .budgetExceeded(
                runID: try container.decode(String.self, forKey: .runID),
                dimension: try container.decode(MacCoreSkillBudgetDimension.self, forKey: .dimension)
            )
        case "completed":
            self = .completed(
                runID: try container.decode(String.self, forKey: .runID),
                resultRef: try container.decodeIfPresent(String.self, forKey: .resultRef)
            )
        case "failed":
            self = .failed(
                runID: try container.decode(String.self, forKey: .runID),
                error: try container.decode(String.self, forKey: .error)
            )
        case "cancelled":
            self = .cancelled(
                runID: try container.decode(String.self, forKey: .runID),
                reason: try container.decodeIfPresent(String.self, forKey: .reason)
            )
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "Unknown Core Skill runtime event"
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .catalogReplaced(let revision):
            try container.encode("catalog_replaced", forKey: .type)
            try container.encode(revision, forKey: .revision)
        case .started(let run):
            try container.encode("started", forKey: .type)
            try container.encode(run, forKey: .run)
        case .capabilityRefreshRequested(let runID, let errors):
            try container.encode("capability_refresh_requested", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(errors, forKey: .errors)
        case .resumed(let runID, let catalogRevision):
            try container.encode("resumed", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(catalogRevision, forKey: .catalogRevision)
        case .usageUpdated(let runID, let usage):
            try container.encode("usage_updated", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(usage, forKey: .usage)
        case .progressUpdated(let runID, let stepID, let completed):
            try container.encode("progress_updated", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(stepID, forKey: .stepID)
            try container.encode(completed, forKey: .completed)
        case .artifactRegistered(let runID, let artifactID, let artifactRef):
            try container.encode("artifact_registered", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(artifactID, forKey: .artifactID)
            try container.encode(artifactRef, forKey: .artifactRef)
        case .verificationRecorded(let runID, let result):
            try container.encode("verification_recorded", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(result, forKey: .result)
        case .resumePointUpdated(let runID, let resumePoint):
            try container.encode("resume_point_updated", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(resumePoint, forKey: .resumePoint)
        case .budgetExceeded(let runID, let dimension):
            try container.encode("budget_exceeded", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(dimension, forKey: .dimension)
        case .completed(let runID, let resultRef):
            try container.encode("completed", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encodeIfPresent(resultRef, forKey: .resultRef)
        case .failed(let runID, let error):
            try container.encode("failed", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encode(error, forKey: .error)
        case .cancelled(let runID, let reason):
            try container.encode("cancelled", forKey: .type)
            try container.encode(runID, forKey: .runID)
            try container.encodeIfPresent(reason, forKey: .reason)
        }
    }
}

nonisolated struct MacCoreSkillRuntimeUpdateEnvelope: Codable, Equatable, Sendable {
    let protocolVersion: UInt32
    let type: String
    let sessionID: String
    let turnID: String?
    let cursor: UInt64
    let revision: UInt64
    let skillRuntimeRevision: UInt64
    let update: MacCoreSkillRuntimeEvent

    enum CodingKeys: String, CodingKey {
        case type, cursor, revision, update
        case protocolVersion = "protocol_version"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case skillRuntimeRevision = "skill_runtime_revision"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        protocolVersion = try values.decode(UInt32.self, forKey: .protocolVersion)
        type = try values.decode(String.self, forKey: .type)
        sessionID = try values.decode(String.self, forKey: .sessionID)
        turnID = try values.decodeIfPresent(String.self, forKey: .turnID)
        cursor = try values.decode(UInt64.self, forKey: .cursor)
        revision = try values.decode(UInt64.self, forKey: .revision)
        skillRuntimeRevision = try values.decode(UInt64.self, forKey: .skillRuntimeRevision)
        update = try values.decode(MacCoreSkillRuntimeEvent.self, forKey: .update)

        guard protocolVersion == 2,
              type == "core_skill_runtime_updated",
              !sessionID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              cursor > 0,
              revision > 0,
              skillRuntimeRevision > 0,
              Self.isValid(update, sessionID: sessionID) else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: values.codingPath,
                debugDescription: "Skill runtime update does not match its Core session envelope."
            ))
        }
    }

    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(protocolVersion, forKey: .protocolVersion)
        try values.encode(type, forKey: .type)
        try values.encode(sessionID, forKey: .sessionID)
        try values.encodeIfPresent(turnID, forKey: .turnID)
        try values.encode(cursor, forKey: .cursor)
        try values.encode(revision, forKey: .revision)
        try values.encode(skillRuntimeRevision, forKey: .skillRuntimeRevision)
        try values.encode(update, forKey: .update)
    }

    private static func isValid(
        _ update: MacCoreSkillRuntimeEvent,
        sessionID: String
    ) -> Bool {
        func validID(_ value: String) -> Bool {
            !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }

        switch update {
        case .catalogReplaced:
            return true
        case .started(let run):
            return validID(run.runID)
                && run.owner.sessionID == sessionID
                && validID(run.owner.parentTurnID)
                && validID(run.descriptor.skillID)
                && validID(run.descriptor.version)
        case let .capabilityRefreshRequested(runID, errors):
            return validID(runID)
                && errors.allSatisfy { validID($0.requestedID) }
        case let .resumed(runID, _),
             let .usageUpdated(runID, _),
             let .resumePointUpdated(runID, _),
             let .budgetExceeded(runID, _),
             let .completed(runID, _),
             let .failed(runID, _),
             let .cancelled(runID, _):
            return validID(runID)
        case let .progressUpdated(runID, stepID, _):
            return validID(runID) && validID(stepID)
        case let .artifactRegistered(runID, artifactID, artifactRef):
            return validID(runID) && validID(artifactID) && validID(artifactRef)
        case let .verificationRecorded(runID, result):
            return validID(runID)
                && validID(result.verifierID)
                && result.evidence.isValid
                && (result.replacesEvidence?.isValid ?? true)
        }
    }
}

nonisolated struct MacCoreCapabilityActivationEnvelope: Codable, Equatable, Sendable {
    let protocolVersion: UInt32
    let type: String
    let sessionID: String
    let turnID: String?
    let cursor: UInt64
    let revision: UInt64
    let source: String
    let capabilityIDs: [String]

    enum CodingKeys: String, CodingKey {
        case type, cursor, revision, source
        case protocolVersion = "protocol_version"
        case sessionID = "session_id"
        case turnID = "turn_id"
        case capabilityIDs = "capability_ids"
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        protocolVersion = try values.decode(UInt32.self, forKey: .protocolVersion)
        type = try values.decode(String.self, forKey: .type)
        sessionID = try values.decode(String.self, forKey: .sessionID)
        turnID = try values.decodeIfPresent(String.self, forKey: .turnID)
        cursor = try values.decode(UInt64.self, forKey: .cursor)
        revision = try values.decode(UInt64.self, forKey: .revision)
        source = try values.decode(String.self, forKey: .source)
        capabilityIDs = try values.decode([String].self, forKey: .capabilityIDs)

        let normalizedCapabilities = capabilityIDs.map {
            $0.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        guard protocolVersion == 2,
              type == "core_capabilities_activated",
              !sessionID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              turnID.map({ !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) ?? true,
              cursor > 0,
              revision > 0,
              !source.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !normalizedCapabilities.isEmpty,
              normalizedCapabilities.allSatisfy({ !$0.isEmpty }),
              Set(normalizedCapabilities).count == normalizedCapabilities.count else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: values.codingPath,
                debugDescription: "Capability activation does not satisfy the Core envelope contract."
            ))
        }
    }

    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(protocolVersion, forKey: .protocolVersion)
        try values.encode(type, forKey: .type)
        try values.encode(sessionID, forKey: .sessionID)
        try values.encodeIfPresent(turnID, forKey: .turnID)
        try values.encode(cursor, forKey: .cursor)
        try values.encode(revision, forKey: .revision)
        try values.encode(source, forKey: .source)
        try values.encode(capabilityIDs, forKey: .capabilityIDs)
    }
}

nonisolated struct MacCoreSkillRuntimeProjection: Codable, Equatable, Sendable {
    var revision: UInt64 = 0
    var lastCursor: UInt64?
    var lastEventRevision: UInt64?
    var catalogRevision: UInt64 = 0
    var order: [String] = []
    var runs: [String: MacCoreSkillRun] = [:]

    var orderedRuns: [MacCoreSkillRun] { order.compactMap { runs[$0] } }
}

nonisolated struct MacCoreSkillProcessProjection: Equatable, Sendable {
    let externalID: String
    let title: String
    let detail: String
    let state: MacAssistantTrace.State
    let result: String?

    static func project(_ run: MacCoreSkillRun) -> Self {
        let state: MacAssistantTrace.State
        switch run.status {
        case .running:
            state = .running
        case .waitingForCatalog:
            state = .waiting
        case .completed:
            state = .completed
        case .failed:
            state = .failed
        case .cancelled:
            state = .cancelled
        }
        return Self(
            externalID: "core-skill:\(run.runID)",
            title: run.descriptor.displayName,
            // Execution mode, counters and protocol status are inspectable
            // runtime metadata, not a user-facing sentence. The chronological
            // tool Parts already expose actual work, while this row communicates
            // only the typed Skill lifecycle in the current locale.
            detail: lifecycleDetail(for: run),
            state: state,
            result: run.failure ?? run.resultRef
        )
    }

    private static func lifecycleDetail(for run: MacCoreSkillRun) -> String {
        switch run.status {
        case .running:
            return String(localized: "Running")
        case .waitingForCatalog:
            return String(localized: "Preparing required tools")
        case .completed:
            return String(localized: "Completed")
        case .failed:
            return run.failure?.trimmingCharacters(in: .whitespacesAndNewlines)
                ?? String(localized: "Failed")
        case .cancelled:
            return String(localized: "Cancelled")
        }
    }
}
