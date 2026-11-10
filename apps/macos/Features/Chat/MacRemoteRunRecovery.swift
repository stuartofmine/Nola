// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Stable identity used while recovering a remote run.

import Foundation

struct MacRemoteRunRecovery: Equatable {
    let conversationID: String
    let expectedRunID: String?
    let assistantMessageID: UUID
    let generationID: UInt64
    let observedTraceID: String?
    let afterTraceSequence: UInt64

    init(conversationID: String, expectedRunID: String?, assistantMessageID: UUID,
         generationID: UInt64, observedTraceID: String? = nil, afterTraceSequence: UInt64 = 0) {
        self.conversationID = conversationID
        self.expectedRunID = expectedRunID
        self.assistantMessageID = assistantMessageID
        self.generationID = generationID
        self.observedTraceID = observedTraceID
        self.afterTraceSequence = afterTraceSequence
    }
}
