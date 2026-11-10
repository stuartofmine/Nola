// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatProcessingStatusBar logic in the iOS UI layer.
// - Interfaces:
//   - ChatProcessingStatusBar
// - APIs:
//   - none
//
import SwiftUI

struct ChatProcessingStatusBar: View {
    let retrievalInProgress: Bool
    let stateIcon: String
    let stateDescription: String

    var body: some View {
        HStack(spacing: 8) {
            if retrievalInProgress {
                ProgressView()
                    .progressViewStyle(CircularProgressViewStyle(tint: .secondary))
                    .scaleEffect(0.7)
                    .frame(width: 14, height: 14)
            } else {
                Image(systemName: stateIcon)
                    .font(.caption2)
                    .foregroundColor(.secondary)
            }

            Text(stateDescription)
                .font(.caption)
                .foregroundColor(.secondary)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
        .background(
            Capsule()
                .fill(DesignTokens.elevatedColor.opacity(0.92))
        )
        .overlay(
            Capsule()
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.top, 8)
    }
}
