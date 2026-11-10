// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides reusable presentation primitives for macOS conversation inspector modules.

import SwiftUI

struct MacInspectorSection<Content: View>: View {
    let title: String
    let showsDivider: Bool
    private let content: Content

    init(
        title: String,
        showsDivider: Bool = true,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.showsDivider = showsDivider
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(title)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(.primary)
                .padding(.horizontal, 2)
                .accessibilityAddTraits(.isHeader)

            VStack(alignment: .leading, spacing: 6) {
                content
            }
            .padding(.horizontal, 2)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 2)
        .padding(.bottom, showsDivider ? 10 : 0)
        .overlay(alignment: .bottom) {
            if showsDivider {
                Rectangle()
                    .fill(MacDesignColor.borderSubtle.opacity(0.7))
                    .frame(height: 0.5)
            }
        }
    }
}

struct MacInspectorDocumentEditor: View {
        @Binding var text: String
        let placeholder: String
        let minHeight: CGFloat
        @FocusState private var isFocused: Bool
        @Environment(\.accessibilityReduceMotion) private var reduceMotion

        var body: some View {
            ZStack(alignment: .topLeading) {
                if text.isEmpty {
                    Text(placeholder)
                        .font(.system(size: MacDesignTokens.Typography.inspectorBody))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 9)
                        .allowsHitTesting(false)
                }

                TextEditor(text: $text)
                    .accessibilityLabel(Text(placeholder))
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody))
                    .lineSpacing(2)
                    .scrollContentBackground(.hidden)
                    .focused($isFocused)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 3)
            }
            .frame(maxWidth: .infinity, minHeight: minHeight, maxHeight: minHeight)
            .background(
                MacDesignColor.canvas,
                in: RoundedRectangle(cornerRadius: 9, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .stroke(
                        isFocused ? MacDesignColor.controlOutline : MacDesignColor.borderSubtle,
                        lineWidth: 1
                    )
            }
            .animation(reduceMotion ? nil : .easeOut(duration: 0.14), value: isFocused)
        }
    }

struct MacConversationRecallRow: View {
        let result: MacConversationRecallResult
        @State private var isExpanded = false
        @Environment(\.accessibilityReduceMotion) private var reduceMotion

        private var sourceTitle: String {
            switch result.source {
            case .originalMessage:
                switch result.role {
                case .some(.user): return String(localized: "Your message", table: "Chat")
                case .some(.assistant): return String(localized: "Nola response", table: "Chat")
                default: return String(localized: "Conversation message", table: "Chat")
                }
            case .compactionSummary:
                return String(localized: "Compaction summary", table: "Chat")
            }
        }

        private var symbol: String {
            result.source == .compactionSummary ? "archivebox" : "text.bubble"
        }

        var body: some View {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    isExpanded.toggle()
                }
            } label: {
                VStack(alignment: .leading, spacing: 7) {
                    HStack(spacing: 7) {
                        Image(systemName: symbol)
                            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .semibold))
                            .foregroundStyle(result.source == .compactionSummary ? Color.orange : Color.accentColor)
                            .frame(width: 16)
                        Text(sourceTitle)
                            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .semibold))
                        Spacer(minLength: 6)
                        Image(systemName: "chevron.down")
                            .font(.system(size: MacDesignTokens.Typography.inspectorCaption, weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .rotationEffect(.degrees(isExpanded ? 180 : 0))
                    }

                    Group {
                        if isExpanded {
                            Text(result.content)
                                .lineLimit(18)
                                .macDisclosureTransition(reduceMotion: reduceMotion)
                        } else {
                            Text(result.snippet)
                                .lineLimit(3)
                                .macDisclosureTransition(reduceMotion: reduceMotion)
                        }
                    }
                    .font(.system(size: MacDesignTokens.Typography.inspectorCaption))
                    .foregroundStyle(.secondary)
                    .lineSpacing(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                }
                .padding(10)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .background(MacDesignColor.hover.opacity(0.65), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .stroke(Color.primary.opacity(0.065), lineWidth: 1)
            }
        }
    }

struct MacInspectorToggleRow: View {
        let symbol: String
        let title: String
        let detail: String?
        @Binding var isOn: Bool

        init(
            symbol: String,
            title: String,
            detail: String? = nil,
            isOn: Binding<Bool>
        ) {
            self.symbol = symbol
            self.title = title
            self.detail = detail
            _isOn = isOn
        }

        var body: some View {
            HStack(alignment: .top, spacing: 9) {
                Image(systemName: symbol)
                    .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .semibold))
                    .frame(width: 26, height: 26)
                    .foregroundStyle(isOn ? MacDesignColor.brand : Color.secondary)
                    .background(
                        (isOn ? MacDesignColor.brand : Color.primary).opacity(isOn ? 0.085 : 0.035),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                    )

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .medium))
                        .lineLimit(2)

                    if let detail {
                        Text(detail)
                            .font(.system(size: MacDesignTokens.Typography.inspectorCaption))
                            .foregroundStyle(.secondary)
                            .lineSpacing(1)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Toggle("", isOn: $isOn)
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .controlSize(.mini)
                    .frame(width: 38, height: 20, alignment: .trailing)
                    .padding(.top, 2)
                    .accessibilityLabel(Text(title))
            }
            .padding(.horizontal, 6)
            .padding(.vertical, detail == nil ? 5 : 7)
            .frame(
                maxWidth: .infinity,
                minHeight: detail == nil ? 38 : 52,
                alignment: .leading
            )
            .contentShape(Rectangle())
            .animation(.easeOut(duration: 0.14), value: isOn)
        }
    }

struct MacInspectorRow: View {
        let symbol: String
        let title: String
        let value: String

        var body: some View {
            HStack(alignment: .center, spacing: 9) {
                Image(systemName: symbol)
                    .font(.system(size: MacDesignTokens.Typography.inspectorDetail, weight: .semibold))
                    .frame(width: 26, height: 26)
                    .foregroundStyle(.secondary)
                    .background(
                        Color.primary.opacity(0.04),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                    )

                Text(title)
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody, weight: .medium))
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .help(title)

                Spacer(minLength: 8)

                Text(value)
                    .font(.system(size: MacDesignTokens.Typography.inspectorBody))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 132, alignment: .trailing)
                    .help(value)
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, minHeight: 38)
            .contentShape(Rectangle())
        }
    }
