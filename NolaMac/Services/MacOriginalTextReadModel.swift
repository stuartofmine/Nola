// SPDX-License-Identifier: Apache-2.0
// Owns one view-local original-text query and its physical cancellation.
import Foundation
import Observation

@MainActor
@Observable
final class MacOriginalTextReadModel {
    nonisolated struct Request: Hashable, Sendable {
        let conversationID: String
        let reference: MacCanonicalConversationWindow.DeferredTextPart
        let offsetBytes: UInt64
    }
    typealias Reader = @Sendable (Request, MacApplicationQueryCancellation) async throws -> MacCanonicalTextRange

    private(set) var page: MacCanonicalTextRange?
    private(set) var failed = false
    private(set) var selection: Request?
    private var generation: UInt64 = 0
    @ObservationIgnored private var cancellation: MacApplicationQueryCancellation?

    func invalidate() {
        generation &+= 1
        cancellation?.cancel()
        cancellation = nil
        selection = nil
        page = nil
        failed = false
    }

    func rejectInvalidSelection() {
        invalidate()
        failed = true
    }

    func load(_ requested: Request, reader: Reader) async -> MacCanonicalTextRange? {
        guard !Task.isCancelled else { return nil }
        // Cancellation happens synchronously before the next query can start.
        invalidate()
        guard requested.reference.isFinal, requested.reference.canReadOriginal else {
            failed = true
            return nil
        }
        let expectedGeneration = generation
        let queryCancellation = MacApplicationQueryCancellation()
        cancellation = queryCancellation
        selection = requested
        do {
            let result = try await withTaskCancellationHandler {
                try await reader(requested, queryCancellation)
            } onCancel: {
                queryCancellation.cancel()
            }
            guard generation == expectedGeneration, selection == requested else { return nil }
            cancellation = nil
            guard !Task.isCancelled else { return nil }
            guard result.conversationID == requested.conversationID,
                  result.messageID == requested.reference.messageID,
                  result.partID == requested.reference.partID,
                  result.offsetBytes == requested.offsetBytes,
                  result.totalUTF8Bytes == requested.reference.totalUTF8Bytes else {
                failed = true
                return nil
            }
            page = result
            return result
        } catch is CancellationError {
            guard generation == expectedGeneration, selection == requested else { return nil }
            cancellation = nil
            return nil
        } catch {
            guard generation == expectedGeneration, selection == requested else { return nil }
            cancellation = nil
            guard !Task.isCancelled else { return nil }
            failed = true
            return nil
        }
    }
}
