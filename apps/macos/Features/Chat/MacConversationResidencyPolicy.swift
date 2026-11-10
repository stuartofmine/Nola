// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns the bounded in-memory conversation residency order and byte accounting.

struct MacConversationResidencyPolicy {
    private var order: [String] = []
    private let maximumConversationCount: Int
    private let byteBudget: Int

    init(maximumConversationCount: Int = 3, byteBudget: Int = 24_000_000) {
        self.maximumConversationCount = maximumConversationCount
        self.byteBudget = byteBudget
    }

    mutating func recordAccess(
        conversationID: String,
        protectedConversationIDs: Set<String>,
        byteCount: (String) -> Int
    ) -> [String] {
        order.removeAll { $0 == conversationID }
        order.append(conversationID)

        var evicted: [String] = []
        while order.count > maximumConversationCount
            || residentBytes(byteCount: byteCount) > byteBudget {
            guard let index = order.firstIndex(where: {
                !protectedConversationIDs.contains($0)
            }) else { break }
            let evictedID = order.remove(at: index)
            evicted.append(evictedID)
        }
        return evicted
    }

    mutating func remove(conversationID: String) {
        order.removeAll { $0 == conversationID }
    }

    private func residentBytes(byteCount: (String) -> Int) -> Int {
        order.reduce(into: 0) { total, conversationID in
            total += max(0, byteCount(conversationID))
        }
    }
}
