// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Maps canonical Application message identifiers to stable SwiftUI identities.

import Foundation

/// UI identity is a projection of the canonical Application identity. It is not a
/// second message identifier and must never be persisted back into Core.
nonisolated enum MacCanonicalMessageIdentity {
    static func stableID(_ canonicalID: String?) -> UUID? {
        guard let canonicalID, !canonicalID.isEmpty else { return nil }

        // Locally submitted messages already use UUIDs. Preserve that identity
        // across the optimistic presentation overlay and the later canonical
        // Application projection; hashing it would manufacture a second UI
        // identity and render the same user message twice.
        if let canonicalUUID = UUID(uuidString: canonicalID) {
            return canonicalUUID
        }

        // Two independently seeded FNV-1a lanes provide a deterministic 128-bit
        // UI key without introducing a content-integrity or SHA dependency.
        let bytes = Array(canonicalID.utf8)
        let high = fnv1a(bytes, seed: 0xcbf29ce484222325)
        let low = fnv1a(bytes.reversed(), seed: 0x84222325cbf29ce4)
        var uuidBytes = withUnsafeBytes(of: high.bigEndian, Array.init)
            + withUnsafeBytes(of: low.bigEndian, Array.init)
        uuidBytes[6] = (uuidBytes[6] & 0x0f) | 0x50
        uuidBytes[8] = (uuidBytes[8] & 0x3f) | 0x80
        let value = uuidBytes.map { String(format: "%02x", $0) }.joined()
        return UUID(uuidString: "\(value.prefix(8))-\(value.dropFirst(8).prefix(4))-\(value.dropFirst(12).prefix(4))-\(value.dropFirst(16).prefix(4))-\(value.dropFirst(20).prefix(12))")
    }

    private static func fnv1a<S: Sequence>(_ bytes: S, seed: UInt64) -> UInt64
    where S.Element == UInt8 {
        bytes.reduce(seed) { hash, byte in
            (hash ^ UInt64(byte)) &* 0x100000001b3
        }
    }
}
