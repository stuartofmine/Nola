// SPDX-License-Identifier: Apache-2.0
// Authenticated remote expert catalog/projection provenance.
import Foundation

nonisolated struct AppleExpertRoomHostResponse<JSON: Codable & Sendable>: Codable, Sendable {
    let hostID: String
    let sourceStoreEpoch: String
    let sourceMatches: Bool
    let result: JSON
    enum CodingKeys: String, CodingKey {
        case hostID = "host_id", sourceStoreEpoch = "source_store_epoch", sourceMatches = "source_matches", result
    }
    func validate(hostID: String, expectedEpoch: String?) throws {
        guard self.hostID == hostID, !sourceStoreEpoch.isEmpty, sourceMatches,
              expectedEpoch == nil || expectedEpoch == sourceStoreEpoch else { throw CocoaError(.coderReadCorrupt) }
    }
}
