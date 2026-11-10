// SPDX-License-Identifier: Apache-2.0
// Core-resolved projections only. Apple renderers never parse A2UI wire messages.
import Foundation
import CryptoKit

nonisolated struct AppleUISurfaceOwner: Codable, Hashable, Sendable {
    let authority_id: String
    let conversation_id: String
    let participant_id: String
    let context_id: String
    let originating_run_id: String
    let message_id: String
    let part_id: String
}

nonisolated struct AppleUISurfaceIdentity: Codable, Hashable, Sendable {
    let owner: AppleUISurfaceOwner
    let surface_id: String
    let generation: UInt64

    /// Compact and stable across process restarts; includes every ownership
    /// field while keeping the native accessibility identifier bounded.
    var accessibilityIdentifier: String {
        let values = [owner.authority_id, owner.conversation_id, owner.participant_id,
            owner.context_id, owner.originating_run_id, owner.message_id, owner.part_id,
            surface_id, String(generation)]
        let bytes = Data(values.map { "\($0.utf8.count):\($0)" }.joined().utf8)
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        return "a2ui.surface." + digest
    }
}

nonisolated enum AppleUISurfacePhase: String, Codable, Sendable {
    case pending, ready, deleted
    case readOnly = "read_only"
}

nonisolated struct AppleUIActionReference: Codable, Equatable, Sendable {
    let instance_id: String
    let action_id: String
}

nonisolated struct AppleUINode<JSON: JournalJSONValue>: Codable, Equatable, Sendable {
    let instance_id: String
    let component_id: String
    let component: String
    let props: [String: JSON]
    let slots: [String: [String]]
    let bindings: [String: String]
    let actions: [String: AppleUIActionReference]
    let validation_errors: [String]
}

nonisolated struct AppleUISnapshot<JSON: JournalJSONValue>: Codable, Equatable, Sendable {
    let identity: AppleUISurfaceIdentity
    let revision: UInt64
    let catalog_id: String
    let catalog_digest: String
    let phase: AppleUISurfacePhase
    let root_instance_id: String?
    let nodes: [String: AppleUINode<JSON>]
    let summary: String
}

nonisolated struct AppleUIChangeSet<JSON: JournalJSONValue>: Codable, Sendable {
    let identity: AppleUISurfaceIdentity
    let base_revision: UInt64
    let revision: UInt64
    let catalog_digest: String
    let phase: AppleUISurfacePhase
    let root_instance_id: String?
    let upserted_nodes: [AppleUINode<JSON>]
    let removed_instance_ids: [String]
    let summary: String
}

/// A nil optional cannot represent both removal and an explicit JSON null.
nonisolated enum AppleUIEdit<JSON: JournalJSONValue>: Codable, Sendable {
    case set(instanceID: String, binding: String, value: JSON)
    case delete(instanceID: String, binding: String)

    private enum CodingKeys: String, CodingKey { case type, instance_id, binding, value }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        let id = try fields.decode(String.self, forKey: .instance_id)
        let binding = try fields.decode(String.self, forKey: .binding)
        switch try fields.decode(String.self, forKey: .type) {
        case "set": self = .set(instanceID: id, binding: binding, value: try fields.decode(JSON.self, forKey: .value))
        case "delete": self = .delete(instanceID: id, binding: binding)
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: fields, debugDescription: "Unknown UI edit")
        }
    }
    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .set(instanceID, binding, value):
            try container.encode("set", forKey: .type)
            try container.encode(instanceID, forKey: .instance_id)
            try container.encode(binding, forKey: .binding)
            try container.encode(value, forKey: .value)
        case let .delete(instanceID, binding):
            try container.encode("delete", forKey: .type)
            try container.encode(instanceID, forKey: .instance_id)
            try container.encode(binding, forKey: .binding)
        }
    }
}

nonisolated struct AppleUIInteraction<JSON: JournalJSONValue>: Codable, Sendable {
    let identity: AppleUISurfaceIdentity
    let expected_surface_revision: UInt64
    let edits: [AppleUIEdit<JSON>]
    let action: AppleUIActionReference?
}

/// Retain the entire value until the existing Application receipt is resolved.
/// Repeated transport attempts must not regenerate the ID or timestamp.
nonisolated struct AppleUISubmission<JSON: JournalJSONValue>: Codable, Sendable {
    let interaction: AppleUIInteraction<JSON>
    let submission_id: String
    let submitted_at_ms: Int64
}

nonisolated struct AppleUIInteractionReceipt<JSON: JournalJSONValue>: Decodable, Sendable {
    let snapshot: AppleUISnapshot<JSON>
    let queued_submission_id: String?
}

nonisolated struct AppleUIListCommand: Encodable, Sendable {
    let type = "list_ui_surfaces"
    let conversation_id: String
}

nonisolated struct AppleUISubmitCommand<JSON: JournalJSONValue>: Encodable, Sendable {
    let type = "submit_ui_interaction"
    let input: AppleUISubmission<JSON>
}

nonisolated struct AppleUIReadRevisionCommand: Encodable, Sendable {
    let type = "read_ui_surface_revision"
    let identity: AppleUISurfaceIdentity
    let revision: UInt64
}

/// A message projection revision is independent of the Surface revision.
nonisolated struct AppleUISurfaceReference: Codable, Hashable, Sendable {
    let identity: AppleUISurfaceIdentity
    let revision: UInt64

    init(identity: AppleUISurfaceIdentity, revision: UInt64) {
        self.identity = identity
        self.revision = revision
    }
    private enum CodingKeys: String, CodingKey { case identity, revision }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        identity = try fields.decode(AppleUISurfaceIdentity.self, forKey: .identity)
        revision = try fields.decode(UInt64.self, forKey: .revision)
        let owner = identity.owner
        guard revision > 0, revision <= UInt64(Int64.max), identity.generation > 0,
              [owner.authority_id, owner.conversation_id, owner.participant_id, owner.context_id,
               owner.originating_run_id, owner.message_id, owner.part_id, identity.surface_id].allSatisfy({
                  !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0.utf8.count <= 512
              }) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Invalid Surface revision reference"))
        }
    }
}

nonisolated struct AppleUISurfacePart: Codable, Hashable, Sendable {
    let partID: String
    let revision: UInt64
    let surface: AppleUISurfaceReference
    let summary: String
    private enum CodingKeys: String, CodingKey {
        case partID = "part_id"
        case revision, surface, summary
    }
}

/// Canonical user action. The envelope is retained as data, never rendered or executed.
nonisolated struct AppleUIActionPart<JSON: JournalJSONValue>: Decodable, Equatable, Sendable {
    let partID: String
    let revision: UInt64
    let surface: AppleUISurfaceReference
    let envelope: JSON
    let summary: String
    private enum CodingKeys: String, CodingKey {
        case partID = "part_id"
        case revision, surface, envelope, summary
    }
}

nonisolated struct AppleUIReadChangesCommand: Encodable, Sendable {
    let type = "read_ui_surface_changes"
    let identity: AppleUISurfaceIdentity
    let base_revision: UInt64?
}

nonisolated enum AppleUISurfaceUpdate<JSON: JournalJSONValue>: Decodable, Sendable {
    case snapshot(AppleUISnapshot<JSON>)
    case changes(AppleUIChangeSet<JSON>)
    case unchanged(AppleUISurfaceIdentity, UInt64)
    case missing(AppleUISurfaceIdentity)
    private enum CodingKeys: String, CodingKey { case type, snapshot, changes, identity, revision }
    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        switch try fields.decode(String.self, forKey: .type) {
        case "snapshot": self = .snapshot(try fields.decode(AppleUISnapshot<JSON>.self, forKey: .snapshot))
        case "changes": self = .changes(try fields.decode(AppleUIChangeSet<JSON>.self, forKey: .changes))
        case "unchanged": self = .unchanged(try fields.decode(AppleUISurfaceIdentity.self, forKey: .identity), try fields.decode(UInt64.self, forKey: .revision))
        case "missing": self = .missing(try fields.decode(AppleUISurfaceIdentity.self, forKey: .identity))
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: fields, debugDescription: "Unknown Surface update")
        }
    }

    func applying(to current: AppleUISnapshot<JSON>?, identity: AppleUISurfaceIdentity) throws -> AppleUISnapshot<JSON>? {
        func invalid() -> DecodingError { .dataCorrupted(.init(codingPath: [], debugDescription: "Surface update does not match its base")) }
        switch self {
        case .snapshot(let value):
            guard value.identity == identity, value.revision >= (current?.revision ?? 0) else { throw invalid() }
            return value
        case .unchanged(let owner, let revision):
            guard owner == identity, let current, current.identity == identity, current.revision == revision else { throw invalid() }
            return current
        case .missing(let owner):
            guard owner == identity else { throw invalid() }
            return nil
        case .changes(let change):
            guard let current, current.identity == identity, change.identity == identity,
                  change.base_revision == current.revision, change.revision > current.revision,
                  change.catalog_digest == current.catalog_digest,
                  Set(change.removed_instance_ids).count == change.removed_instance_ids.count,
                  Set(change.upserted_nodes.map(\.instance_id)).count == change.upserted_nodes.count else { throw invalid() }
            var nodes = current.nodes
            for id in change.removed_instance_ids { nodes.removeValue(forKey: id) }
            for node in change.upserted_nodes { nodes[node.instance_id] = node }
            return AppleUISnapshot(identity: identity, revision: change.revision, catalog_id: current.catalog_id,
                catalog_digest: change.catalog_digest, phase: change.phase, root_instance_id: change.root_instance_id,
                nodes: nodes, summary: change.summary)
        }
    }
}


nonisolated struct AppleUIIntentSource: Codable, Equatable, Sendable {
    let hostID: String?
    let address: String
    let certificate: String?
    let clientID: String?
}

/// Uses the existing local transactional settings store as a transport-intent
/// namespace. No tokens, execution state or mutable Surface state are stored.
@MainActor
struct AppleUIIntentJournal<JSON: JournalJSONValue> {
    struct Record: Codable {
        let source: AppleUIIntentSource
        let input: AppleUISubmission<JSON>
    }
    let read: (String) throws -> (json: String, revision: Int64)?
    let write: (String, String, Int64?) throws -> Void

    private func key(_ identity: AppleUISurfaceIdentity) throws -> String {
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        let digest = SHA256.hash(data: try encoder.encode(identity)).map { String(format: "%02x", $0) }.joined()
        return "transport.a2ui.pending." + digest
    }
    private func decode(_ json: String) throws -> Record? {
        try JSONDecoder().decode(Record?.self, from: Data(json.utf8))
    }
    func pending(identity: AppleUISurfaceIdentity, source: AppleUIIntentSource) throws -> AppleUISubmission<JSON>? {
        guard let stored = try read(key(identity)), let record = try decode(stored.json) else { return nil }
        guard record.source == source, record.input.interaction.identity == identity else {
            throw NSError(domain: "NolaUIIntent", code: 1, userInfo: [NSLocalizedDescriptionKey: "请重新连接原 Host，以确认待处理的表单提交。"])
        }
        return record.input
    }
    func stage(_ input: AppleUISubmission<JSON>, source: AppleUIIntentSource) throws {
        let storageKey = try key(input.interaction.identity)
        let stored = try read(storageKey)
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        let record = Record(source: source, input: input)
        let encoded = try encoder.encode(record)
        guard encoded.count <= 256 * 1024 else { throw NSError(domain: "NolaUIIntent", code: 2) }
        if let stored, let existing = try decode(stored.json) {
            guard try encoder.encode(existing) == encoded else {
                throw NSError(domain: "NolaUIIntent", code: 3, userInfo: [NSLocalizedDescriptionKey: "上一条表单提交结果未确认，请先查询原提交。"])
            }
            return
        }
        try write(storageKey, String(decoding: encoded, as: UTF8.self), stored?.revision)
    }
    func retire(_ input: AppleUISubmission<JSON>, source: AppleUIIntentSource) throws {
        let storageKey = try key(input.interaction.identity)
        guard let stored = try read(storageKey), let existing = try decode(stored.json) else { return }
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        guard try encoder.encode(existing) == encoder.encode(Record(source: source, input: input)) else {
            throw NSError(domain: "NolaUIIntent", code: 4)
        }
        // Clear using CAS. An unconditional delete could remove a concurrent
        // replacement request; this bounded per-Surface slot keeps its revision.
        try write(storageKey, "null", stored.revision)
    }
}

/// Only the display summary is extracted here. BeginQueuedUserTurn consumes the
/// original Core command, including its typed action envelope, without rewriting it.
nonisolated struct AppleUIQueuedSubmission: Decodable, Sendable {
    struct Submission: Decodable, Sendable {
        struct Item: Decodable, Sendable {
            struct Action: Decodable, Sendable { let summary: String }
            let type: String
            let id: String
            let action: Action
        }
        let type: String
        let submission_id: String
        let items: [Item]
    }
    let type: String
    let submission: Submission

    func item(commandID: String) throws -> Submission.Item {
        guard type == "submission", submission.type == "user_input",
              submission.submission_id == commandID, submission.items.count == 1,
              let item = submission.items.first, item.type == "ui_action", !item.id.isEmpty else {
            throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Invalid queued UI action identity"))
        }
        return item
    }
}

nonisolated struct AppleUIPreviewInput<JSON: JournalJSONValue>: Codable, Sendable {
    let identity: AppleUISurfaceIdentity
    let expected_surface_revision: UInt64
    let edits: [AppleUIEdit<JSON>]
    let draft_generation: UInt64
}
nonisolated struct AppleUIPreviewResult<JSON: JournalJSONValue>: Decodable, Sendable {
    let identity: AppleUISurfaceIdentity
    let base_revision: UInt64
    let draft_generation: UInt64
    let snapshot: AppleUISnapshot<JSON>

    func matches(identity expectedIdentity: AppleUISurfaceIdentity, revision: UInt64, generation: UInt64) -> Bool {
        identity == expectedIdentity && base_revision == revision && draft_generation == generation
            && snapshot.identity == expectedIdentity && snapshot.revision == revision && snapshot.phase == .ready
    }
}
nonisolated struct AppleUIPreviewCommand<JSON: JournalJSONValue>: Encodable, Sendable {
    let type = "preview_ui_surface"
    let input: AppleUIPreviewInput<JSON>
}
