// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DisplayFormatters logic in the iOS UI layer.
// - Interfaces:
//   - DisplayFormatters
// - APIs:
//   - none
//
import Foundation

enum DisplayFormatters {
    private static let fileSizeFormatter: ByteCountFormatter = {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter
    }()

    static func byteSize(_ bytes: Int64) -> String {
        fileSizeFormatter.string(fromByteCount: bytes)
    }

    static func transferRate(_ bytesPerSecond: Double) -> String {
        "\(fileSizeFormatter.string(fromByteCount: Int64(bytesPerSecond)))/s"
    }
}
