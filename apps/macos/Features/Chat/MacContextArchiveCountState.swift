// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns stale-result rejection for context archive count requests.

struct MacContextArchiveCountState {
    private(set) var conversationID: String?
    private(set) var request: UInt64 = 0
    private(set) var count: Int?
    private(set) var failed = false

    mutating func begin(conversationID: String) -> UInt64 {
        request &+= 1
        if self.conversationID != conversationID {
            count = nil
        }
        self.conversationID = conversationID
        failed = false
        return request
    }

    @discardableResult
    mutating func finish(conversationID: String, request: UInt64, count: Int?) -> Bool {
        guard self.conversationID == conversationID, self.request == request else {
            return false
        }
        self.count = count.flatMap { $0 >= 0 ? $0 : nil }
        failed = self.count == nil
        return true
    }
}
