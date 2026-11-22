// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatEmptyStateCard logic in the iOS UI layer.
// - Interfaces:
//   - ChatEmptyStateCard
// - APIs:
//   - none
//
import SwiftUI

struct ChatEmptyStateCard: View {
    let hasModels: Bool
    let selectedModel: ModelInfo?
    let hybridDeviceToolsEnabled: Bool
    let hostConnectionPhase: ChatHostConnectionPhase
    let onSuggestion: (String) -> Void
    let onPrimaryAction: () -> Void
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    private var suggestions: [(String, String)] {
        [
            ("waveform.path.ecg", NSLocalizedString("chat.suggest.brainstorm", value: "Summarize important changes in my recent data", comment: "Review recent personal signals")),
            ("brain.head.profile", NSLocalizedString("chat.suggest.summary", value: "Based on what you remember about me, what should I focus on next?", comment: "Continue from personal memory")),
            ("calendar.badge.clock", NSLocalizedString("chat.suggest.schedule", value: "Plan today around my schedule and priorities", comment: "Plan today from priorities")),
            ("doc.text.magnifyingglass", NSLocalizedString("chat.suggest.translate", value: "Analyze an uploaded file and highlight key facts and impact", comment: "Analyze an uploaded file"))
        ]
    }

    var body: some View {
        VStack(spacing: 20) {
            VStack(spacing: 8) {
                Text(NSLocalizedString("chat.empty.greeting", value: "Where shall we start?", comment: "Chat empty greeting"))
                    .font(.title2.weight(.semibold))
                    .foregroundStyle(DesignTokens.contentColor)

                Text(emptyStateDescription)
                    .font(.subheadline)
                    .foregroundStyle(DesignTokens.secondaryContentColor)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 18)
                    .accessibilityIdentifier("chat.empty.description")
            }

            if hasModels && modelIsReady {
                LazyVGrid(
                    columns: suggestionColumns,
                    spacing: 10
                ) {
                    ForEach(suggestions, id: \.1) { item in
                        Button(action: { onSuggestion(item.1) }) {
                            suggestionLabel(icon: item.0, title: item.1)
                            .padding(horizontalSizeClass == .compact ? 12 : 14)
                            .frame(
                                maxWidth: .infinity,
                                minHeight: 48,
                                alignment: horizontalSizeClass == .compact ? .leading : .topLeading
                            )
                            .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: 14, style: .continuous)
                                    .stroke(DesignTokens.borderColor, lineWidth: 1)
                            )
                            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                        }
                        .buttonStyle(NolaPressableStyle())
                    }
                }
            } else if selectedModel?.isWebBackend == true,
                      hostConnectionPhase == .checking || hostConnectionPhase == .notApplicable {
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text(NSLocalizedString("chat.empty.host_checking", value: "Checking connection", comment: "Checking host action"))
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("chat.empty.host_checking")
            } else {
                Button(action: onPrimaryAction) {
                    Text(primaryActionTitle)
                        .font(.callout.weight(.semibold))
                        .padding(.horizontal, 22)
                        .padding(.vertical, 12)
                }
                .buttonStyle(NolaPrimaryButtonStyle(cornerRadius: DesignTokens.radius_md))
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 32)
        .frame(maxWidth: 600)
    }

    private var suggestionColumns: [GridItem] {
        if horizontalSizeClass == .compact {
            return [GridItem(.flexible(), spacing: 10)]
        }
        return [GridItem(.adaptive(minimum: 180), spacing: 10)]
    }

    @ViewBuilder
    private func suggestionLabel(icon: String, title: String) -> some View {
        if horizontalSizeClass == .compact {
            HStack(spacing: 11) {
                suggestionIcon(icon)
                suggestionText(title)
                Spacer(minLength: 4)
            }
        } else {
            VStack(alignment: .leading, spacing: 10) {
                suggestionIcon(icon)
                suggestionText(title)
            }
        }
    }

    private func suggestionIcon(_ icon: String) -> some View {
        Image(systemName: icon)
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(DesignTokens.primaryColor)
            .frame(width: 30, height: 30)
            .accessibilityHidden(true)
    }

    private func suggestionText(_ title: String) -> some View {
        Text(title)
            .font(.subheadline.weight(.medium))
            .foregroundColor(.primary)
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var emptyStateDescription: String {
        guard let selectedModel else { return L10n.chatEmptySubtitle }
        if selectedModel.isWebBackend {
            switch hostConnectionPhase {
            case .checking, .notApplicable:
                return NSLocalizedString("chat.empty.mode.host_checking", value: "Connecting to Nola Server and reading available capabilities…", comment: "Checking host empty chat description")
            case .unavailable:
                return NSLocalizedString("chat.empty.mode.host_unavailable", value: "Nola Server is currently unavailable. Check that Nola on your Mac is running and reachable.", comment: "Unavailable host empty chat description")
            case .unconfigured:
                return NSLocalizedString("chat.empty.mode.host_unconfigured", value: "Nola Server is not connected yet. Pair with your Mac to use project tools and longer-running tasks.", comment: "Unconfigured host empty chat description")
            case .connected:
                break
            }
            return hybridDeviceToolsEnabled
                ? NSLocalizedString("chat.empty.mode.hybrid", value: "Your Mac can coordinate work and request approved actions on this device.", comment: "Hybrid empty chat description")
                : NSLocalizedString("chat.empty.mode.host", value: "Connected to your Mac for project tools and longer-running work.", comment: "Host empty chat description")
        }
        if selectedModel.isLocal {
            return NSLocalizedString("chat.empty.mode.device", value: "Ready on this device for writing, planning, and device actions.", comment: "Local empty chat description")
        }
        return NSLocalizedString("chat.empty.mode.cloud", value: "Using the selected cloud model for conversation and analysis.", comment: "Cloud empty chat description")
    }

    private var modelIsReady: Bool {
        guard let selectedModel else { return false }
        return !selectedModel.isWebBackend || hostConnectionPhase == .connected
    }

    private var primaryActionTitle: String {
        if !hasModels { return NSLocalizedString("model.setup.title", value: "Configure model", comment: "Configure model") }
        if selectedModel?.isWebBackend == true {
            if hostConnectionPhase == .unavailable {
                return NSLocalizedString("chat.empty.retry_host", value: "Retry connection", comment: "Retry host from empty chat")
            }
            if !modelIsReady {
                return NSLocalizedString("chat.empty.configure_host", value: "Configure host", comment: "Configure host from empty chat")
            }
        }
        return NSLocalizedString("model.picker.title", comment: "Select model action")
    }

}
