// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatHeaderChip logic in the iOS UI layer.
// - Interfaces:
//   - ChatHeaderChip
// - APIs:
//   - none
//
import SwiftUI

struct ChatHeaderChip: View {
    let label: String
    var systemImage: String?
    let tint: Color
    let trailingText: String?
    var compact: Bool = false

    var body: some View {
        HStack(spacing: compact ? 5 : 6) {
            if let systemImage, !systemImage.isEmpty {
                Image(systemName: systemImage)
                    .font((compact ? Font.caption : Font.callout).weight(.semibold))
            }
            Text(label)
                .font((compact ? Font.caption : Font.callout).weight(.semibold))
                .lineLimit(1)
            if let trailingText, !trailingText.isEmpty {
                Text(trailingText)
                    .font(compact ? .caption : .callout)
                    .foregroundColor(.secondary)
                    .lineLimit(1)
            }
        }
        .foregroundColor(tint)
        .padding(.horizontal, compact ? 10 : 12)
        .padding(.vertical, compact ? 7 : 9)
        .background(
            Capsule()
                .fill(tint.opacity(DesignTokens.overlaySoftOpacity))
        )
        .overlay(
            Capsule()
                .stroke(tint.opacity(0.18), lineWidth: 1)
        )
    }
}
