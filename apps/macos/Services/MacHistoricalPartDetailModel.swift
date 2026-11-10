// SPDX-License-Identifier: Apache-2.0
// Read-only, view-local historical details. No canonical or live stream mutation.
import Foundation
import Observation

nonisolated struct MacHistoricalPartSelection: Codable, Hashable, Sendable, Identifiable {
    let owner: MacCanonicalPartOwner
    let canonicalRevision: Int64
    let windowSequence: UInt64
    var id: String { owner.partID }
}

extension MacCanonicalConversationProjectionV1.HistoricalProjection {
    nonisolated func matches(owner expected: MacCanonicalPartOwner, revision: Int64) -> Bool {
        canonicalRevision == revision && owner == .object([
            "conversation_id": .string(expected.conversationID),
            "message_id": .string(expected.messageID), "part_id": .string(expected.partID)
        ])
    }
}

@MainActor
@Observable
final class MacHistoricalPartDetailModel {
    typealias Projection = MacCanonicalConversationProjectionV1.HistoricalProjection
    typealias Reader = @Sendable (MacHistoricalPartSelection) async throws -> Projection?
    enum State: Equatable {
        case idle, loading, absent, failed
        case loaded(Projection)
    }
    private(set) var state: State = .idle
    private(set) var selection: MacHistoricalPartSelection?
    private var generation: UInt64 = 0
    @ObservationIgnored private let reader: Reader

    init(reader: @escaping Reader = { selection in
        let task = Task.detached(priority: .userInitiated) {
            try Task.checkCancellation()
            return try MacApplicationPort.shared.readHistoricalPart(selection.owner,
                expectedConversationRevision: selection.canonicalRevision)
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
    }) {
        self.reader = reader
    }

    func invalidate() {
        generation &+= 1
        selection = nil
        state = .idle
    }

    func load(_ requested: MacHistoricalPartSelection) async {
        generation &+= 1
        let expectedGeneration = generation
        selection = requested
        state = .loading
        do {
            let response = try await reader(requested)
            guard !Task.isCancelled, generation == expectedGeneration, selection == requested else { return }
            if let response {
                guard response.matches(owner: requested.owner, revision: requested.canonicalRevision) else {
                    state = .failed
                    return
                }
                state = .loaded(response)
            } else {
                state = .absent
            }
        } catch {
            guard !Task.isCancelled, generation == expectedGeneration, selection == requested else { return }
            state = .failed
        }
    }
}
