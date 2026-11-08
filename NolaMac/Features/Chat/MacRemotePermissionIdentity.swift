// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Stable identities used to correlate remote permission stream events.

struct MacRemotePermissionStreamIdentity: Hashable {
    let generationID: UInt64
    let conversationID: String
    let traceID: String
}

struct MacRemotePermissionTraceIdentity {
    let stream: MacRemotePermissionStreamIdentity
    let runID: String
    let turnID: String
    let requestID: String
    let callID: String
    let createdSequence: UInt64
}
