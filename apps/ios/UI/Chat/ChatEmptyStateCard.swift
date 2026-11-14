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
    @State private var hasAppeared = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    private var suggestions: [(String, String)] {
        [
            ("waveform.path.ecg", NSLocalizedString("chat.suggest.brainstorm", value: "总结我最近数据中的重要变化", comment: "Review recent personal signals")),
            ("brain.head.profile", NSLocalizedString("chat.suggest.summary", value: "根据你对我的记忆，我接下来该关注什么？", comment: "Continue from personal memory")),
            ("calendar.badge.clock", NSLocalizedString("chat.suggest.schedule", value: "结合我的日程与优先级，规划今天", comment: "Plan today from priorities")),
            ("doc.text.magnifyingglass", NSLocalizedString("chat.suggest.translate", value: "分析我上传的文件，提炼事实与影响", comment: "Analyze an uploaded file"))
        ]
    }

    var body: some View {
        VStack(spacing: 20) {
            VStack(spacing: 8) {
                Text(NSLocalizedString("chat.empty.greeting", value: "我们从哪里开始？", comment: "Chat empty greeting"))
                    .font(.title2.weight(.semibold))
                    .foregroundColor(.primary)

                Text(emptyStateDescription)
                    .font(.body.weight(.medium))
                    .foregroundStyle(Color(uiColor: .label))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 18)
                    .accessibilityIdentifier("chat.empty.description")
            }
            .opacity(hasAppeared ? 1 : 0)
            .offset(y: reduceMotion || hasAppeared ? 0 : 6)

            if hasModels && modelIsReady {
                LazyVGrid(
                    columns: suggestionColumns,
                    spacing: 10
                ) {
                    ForEach(suggestions, id: \.1) { item in
                        Button(action: { onSuggestion(item.1) }) {
                            suggestionLabel(icon: item.0, title: item.1)
                            .padding(horizontalSizeClass == .compact ? 11 : 13)
                            .frame(
                                maxWidth: .infinity,
                                minHeight: 44,
                                alignment: horizontalSizeClass == .compact ? .leading : .topLeading
                            )
                            .overlay(alignment: .bottom) { DesignTokens.borderColor.frame(height: 0.5) }
                            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                        }
                        .buttonStyle(NolaPressableStyle())
                    }
                }
                .opacity(hasAppeared ? 1 : 0)
                .offset(y: reduceMotion || hasAppeared ? 0 : 8)
            } else if selectedModel?.isWebBackend == true,
                      hostConnectionPhase == .checking || hostConnectionPhase == .notApplicable {
                HStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.small)
                    Text(NSLocalizedString("chat.empty.host_checking", value: "正在检查连接", comment: "Checking host action"))
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("chat.empty.host_checking")
                .opacity(hasAppeared ? 1 : 0)
            } else {
                Button(action: onPrimaryAction) {
                    Text(primaryActionTitle)
                        .font(.callout.weight(.semibold))
                        .foregroundColor(Color(.systemBackground))
                        .padding(.horizontal, 22)
                        .padding(.vertical, 12)
                        .background(RoundedRectangle(cornerRadius: DesignTokens.radius_md).fill(DesignTokens.actionColor))
                }
                .buttonStyle(NolaPressableStyle())
                .opacity(hasAppeared ? 1 : 0)
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 32)
        .frame(maxWidth: 600)
        .task {
            guard !hasAppeared else { return }
            guard !reduceMotion else {
                hasAppeared = true
                return
            }
            await Task.yield()
            withAnimation(DesignTokens.revealAnimation) {
                hasAppeared = true
            }
        }
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
                return NSLocalizedString("chat.empty.mode.host_checking", value: "正在连接宿主机并读取可用能力…", comment: "Checking host empty chat description")
            case .unavailable:
                return NSLocalizedString("chat.empty.mode.host_unavailable", value: "宿主机暂时不可用。请确认 Mac 版 Nola 正在运行并检查网络。", comment: "Unavailable host empty chat description")
            case .unconfigured:
                return NSLocalizedString("chat.empty.mode.host_unconfigured", value: "宿主机尚未连接。完成配对后，可使用项目工具并处理长任务。", comment: "Unconfigured host empty chat description")
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
                return NSLocalizedString("chat.empty.retry_host", value: "重试连接", comment: "Retry host from empty chat")
            }
            if !modelIsReady {
                return NSLocalizedString("chat.empty.configure_host", value: "配置宿主机", comment: "Configure host from empty chat")
            }
        }
        return NSLocalizedString("model.picker.title", comment: "Select model action")
    }

}
