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

    static func gigabyteSize(_ bytes: Int64) -> String {
        let gigabytes = max(0, Double(bytes)) / 1_000_000_000.0
        if gigabytes >= 10 {
            return String(format: "%.1f GB", gigabytes)
        }
        return String(format: "%.2f GB", gigabytes)
    }

    static func transferRate(_ bytesPerSecond: Double) -> String {
        "\(fileSizeFormatter.string(fromByteCount: Int64(bytesPerSecond)))/s"
    }
}
