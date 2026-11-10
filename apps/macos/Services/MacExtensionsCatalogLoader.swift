// SPDX-License-Identifier: Apache-2.0
import Combine
import Foundation

/// Owns publication of the latest catalog request, including refreshes while a
/// previous source query is still awaiting I/O.
@MainActor
final class MacExtensionsCatalogLoader: ObservableObject {
    @Published private(set) var items: [MacExtensionItem] = []
    @Published private(set) var isLoading = false
    @Published var errorMessage: String?
    @Published private(set) var hasRemovedBuiltInSkills = false
    private var generation = UUID()
    private let local: (URL?) async throws -> [MacExtensionItem]
    private let host: () async throws -> [MacExtensionItem]
    private let removedSkills: () throws -> Bool

    init(local: @escaping (URL?) async throws -> [MacExtensionItem],
         host: @escaping () async throws -> [MacExtensionItem],
         removedSkills: @escaping () throws -> Bool) {
        self.local = local
        self.host = host
        self.removedSkills = removedSkills
    }

    func invalidate() {
        generation = UUID()
        isLoading = false
    }

    func load(includeHost: Bool, workspaceURL: URL?) async {
        let request = UUID()
        generation = request
        isLoading = true
        defer { if generation == request { isLoading = false } }
        do {
            var merged = try await local(workspaceURL)
            guard generation == request, !Task.isCancelled else { return }
            if includeHost { merged.append(contentsOf: try await host()) }
            guard generation == request, !Task.isCancelled else { return }
            let removed = try removedSkills()
            items = merged
            hasRemovedBuiltInSkills = removed
            errorMessage = nil
        } catch {
            guard generation == request, !Task.isCancelled else { return }
            errorMessage = error.localizedDescription
        }
    }
}
