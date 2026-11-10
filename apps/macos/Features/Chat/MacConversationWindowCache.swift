// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Caches bounded history-window models by conversation identity.

import Foundation

@MainActor
final class MacConversationWindowCache {
    private let repository: MacCanonicalConversationRepository
    private var windows: [String: MacConversationWindowModel] = [:]

    init(repository: MacCanonicalConversationRepository) {
        self.repository = repository
    }

    func window(for conversationID: String) -> MacConversationWindowModel {
        if let existing = windows[conversationID] {
            return existing
        }
        let window = repository.makeWindowModel()
        windows[conversationID] = window
        return window
    }

    func existingWindow(for conversationID: String?) -> MacConversationWindowModel? {
        conversationID.flatMap { windows[$0] }
    }

    func evict(conversationID: String) {
        windows[conversationID] = nil
    }
}
