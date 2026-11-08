// SPDX-License-Identifier: Apache-2.0
// Expert color is a local presentation preference, never an execution capability.
import SwiftUI
import AppKit

struct MacExpertColorEditor: View {
    let agentID: String
    let name: String
    @AppStorage(AppleExpertAppearance.storageKey) private var stored = Data()
    @State private var draft = AppleExpertBubbleColor.blue
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text(String(format: String(localized: "Configure %@ · Bubble Color"), name)).font(.headline)
            Picker(String(localized: "Bubble Color"), selection: $draft) {
                ForEach(AppleExpertBubbleColor.allCases) { color in
                    HStack(spacing: 10) {
                        RoundedRectangle(cornerRadius: 4)
                            .fill(color.fill(scheme))
                            .overlay { RoundedRectangle(cornerRadius: 4).strokeBorder(MacDesignColor.borderSubtle, lineWidth: 1) }
                            .frame(width: 24, height: 16)
                            .accessibilityHidden(true)
                        Text(String(localized: String.LocalizationValue(color.localizationKey)))
                    }
                    .frame(minHeight: 32)
                    .tag(color)
                }
            }.pickerStyle(.radioGroup)
            HStack(alignment: .top, spacing: 12) {
                VStack(spacing: 6) {
                    AppleExpertAvatar(name: name, size: 32)
                    AppleExpertName(name: name, alignment: .center)
                }
                .frame(width: 48)
                .padding(.top, 3)
                Text(String(localized: "Preview"))
                    .font(.system(size: MacMarkdownTypography.bodySize))
                    .lineSpacing(MacMarkdownTypography.lineSpacing)
                    .padding(.horizontal, 18).padding(.vertical, 14)
                    .background(draft.fill(scheme), in: UnevenRoundedRectangle(
                        topLeadingRadius: 5, bottomLeadingRadius: 16,
                        bottomTrailingRadius: 16, topTrailingRadius: 16))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            HStack {
                Spacer()
                Button { dismiss() } label: {
                    Text(String(localized: "Cancel")).frame(minHeight: 32)
                }.keyboardShortcut(.cancelAction)
                Button {
                    guard let data = AppleExpertAppearance.saving(draft, agentID: agentID, in: stored) else { return }
                    stored = data; dismiss()
                } label: {
                    Text(String(localized: "Save")).frame(minHeight: 32)
                }.keyboardShortcut(.defaultAction)
            }
        }.padding(24).frame(width: 360)
            .background(MacDesignColor.canvas)
            .onAppear { draft = AppleExpertAppearance.selected(stored, agentID: agentID) }
    }
}

/// Bound the native text reader's proposal before it expands to infinity. The
/// measured prose width can grow during streaming; its leading edge stays fixed.
enum MacExpertBubbleLayout {
    static func contentWidth(text: String, maximumWidth: CGFloat, requiresFullWidth: Bool) -> CGFloat {
        let available = max(0, maximumWidth - 36)
        guard !requiresFullWidth, !text.isEmpty, text.utf8.prefix(1_025).count <= 1_024 else { return available }
        let font = NSFont.systemFont(ofSize: MacMarkdownTypography.bodySize)
        let width = text.split(separator: "\n", omittingEmptySubsequences: false).reduce(CGFloat.zero) { width, line in
            max(width, (String(line) as NSString).size(withAttributes: [.font: font]).width)
        }
        // Allow emphasis and inline-code metrics without exposing Markdown to a
        // second parser. Long prose and rich surfaces receive the available width.
        return min(available, max(32, ceil(width * 1.12) + 4))
    }
}
