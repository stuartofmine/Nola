// SPDX-License-Identifier: Apache-2.0
import Foundation

typealias MacRemoteSubmissionIdentity = AppleRemoteSubmissionIdentity

nonisolated struct MacRemoteSubmissionIntent: Decodable, Sendable {
    enum Phase: String, Codable, Sendable { case prepared, bound, admitted, rejected, cancelled }
    let identity: MacRemoteSubmissionIdentity
    let request_digest: String?
    let phase: Phase
}

nonisolated struct MacSourceSubmissionReceipt: Codable, Sendable {
    let protocol_version: Int
    let identity: MacRemoteSubmissionIdentity
    let request_digest: String
    let phase: MacRemoteSubmissionIntent.Phase
    let admission: MacRunMessageAdmissionCursor?
    let rejection_code: String?
}
