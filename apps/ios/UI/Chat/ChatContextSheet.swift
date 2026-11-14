// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ChatContextSheet logic in the iOS UI layer.
// - Interfaces:
//   - ChatContextSheet
// - APIs:
//   - none
//
import SwiftUI
import UniformTypeIdentifiers
import UIKit

struct ContextFileImportOwner {
    let conversationID: UUID?
}

enum IOSContextFileImportPolicy {
    static func isCancellation(_ error: Error) -> Bool {
        let value = error as NSError
        return value.domain == NSCocoaErrorDomain && value.code == CocoaError.userCancelled.rawValue
    }

    static func readText(from url: URL, maximumCharacters: Int) throws -> String {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let data = try handle.read(upToCount: 192_001) ?? Data()
        guard data.count <= 192_000, let text = String(data: data, encoding: .utf8),
              !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              text.count <= maximumCharacters else {
            throw IOSApplicationRuntimeError.rejected(String(format: NSLocalizedString(
                "chat.context.import.invalid_text", value: "Choose a nonempty UTF-8 text file with at most %d characters (192 KB).", comment: "Invalid context text import"
            ), maximumCharacters))
        }
        return text
    }
}

struct ConversationKnowledgeSaveSnapshot: Equatable, Sendable {
    let conversationID: UUID?
    let knowledgePoints: String
    let wiki: String
}

struct ChatConversationScopeSummary: View {
    let symbol: String
    let title: String
    let detail: String
    let scopeLabel: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(DesignTokens.primaryColor)
                .frame(width: 36, height: 36)
                .background(DesignTokens.primaryColor.opacity(0.10), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.primary)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }

            Spacer(minLength: 8)

            Text(scopeLabel)
                .font(.caption.weight(.semibold))
                .foregroundStyle(DesignTokens.primaryColor)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(DesignTokens.primaryColor.opacity(0.09), in: Capsule())
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }
}

struct ChatContextSheet: View {
    enum DisplayMode {
        case context
        case skills
    }

    enum ContextPane: String, CaseIterable, Hashable {
        case overview
        case knowledge
        case tools
    }

    private enum SkillVisualCategory: String, CaseIterable, Identifiable {
        case documents
        case data
        case design
        case communication
        case automation
        case research
        case workspace
        case custom

        var id: String { rawValue }
    }

    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss

    let displayMode: DisplayMode
    let conversationID: UUID?
    let conversationTitle: String
    let contextUsage: ConversationContextUsage?
    @Binding var sessionMemory: String
    @Binding var knowledgePoints: String
    @Binding var conversationWiki: String
    @Binding var longTermMemory: String
    @Binding var userPreferences: String
    @Binding var mcpServers: [UserMCPServer]
    @Binding var uploadedSkills: [UserUploadedContextItem]
    let hostUploadedSkills: [UserUploadedContextItem]
    let hostUploadedDocuments: [UserUploadedContextItem]
    let hostMCPServers: [UserMCPServer]
    @Binding var uploadedDocuments: [UserUploadedContextItem]
    let dataSources: [URLDataSource]
    @Binding var selectedDataSourceIDs: Set<UUID>
    @Binding var selectedSystemDataSourceTypes: Set<DataSourceType>
    let onSave: () throws -> Void
    let onSaveMemoryProfile: () async throws -> Void
    let onSaveConversationKnowledge: (ConversationKnowledgeSaveSnapshot) async throws -> Void
    let onCompact: () async throws -> String
    let onReload: () async throws -> String?
    let onDiscard: () -> Void
    let onUseSkill: (String) -> Void
    let onSearchMemory: (String) async throws -> [ContextMemorySearchHit]

    @State private var errorMessage: String? = nil
    @State private var noticeMessage: String? = nil
    @State private var isSavingMemoryProfile = false
    @State private var isCompacting = false
    @State private var isSearchingMemory = false
    @State private var isReloading = false
    @State private var showSkillImporter = false
    @State private var showDocumentImporter = false
    @State private var skillImportOwner: ContextFileImportOwner?
    @State private var documentImportOwner: ContextFileImportOwner?
    @State private var memorySearchQuery = ""
    @State private var memorySearchResults: [ContextMemorySearchHit] = []
    @State private var selectedContextPane: ContextPane = .overview
    @State private var selectedSkillID: UUID?
    @State private var selectedMCPServerID: UUID?
    @State private var pendingSkillDeletionID: UUID?
    @State private var pendingMCPDeletionID: UUID?
    @State private var initialMemoryText: [String]?

    var body: some View {
        Form {
            Section {
                ChatConversationScopeSummary(
                    symbol: displayMode == .context ? "circle.hexagongrid" : "shippingbox",
                    title: conversationTitle,
                    detail: sheetSubtitle,
                    scopeLabel: displayMode == .context
                        ? tr("chat.scope.current_conversation", "当前对话", "This conversation")
                        : tr("chat.scope.across_conversations", "跨对话", "Across conversations")
                )
            }

            if displayMode == .context {
                Section {
                    Picker(
                        tr("chat.context.pane.picker", "对话详情", "Conversation details"),
                        selection: $selectedContextPane
                    ) {
                        ForEach(ContextPane.allCases, id: \.self) { pane in
                            Text(contextPaneTitle(pane))
                                .tag(pane)
                                .accessibilityIdentifier("chat-context-pane-\(pane.rawValue)")
                        }
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("chat-context-pane-picker")
                }
            }

            if displayMode == .context && selectedContextPane == .overview {
                Section {
                    if let contextUsage {
                        VStack(alignment: .leading, spacing: 12) {
                            if contextUsage.hasCommittedTokenTotals {
                                HStack {
                                    Text("\(contextUsage.percentage)%")
                                        .font(.title3.weight(.semibold))
                                        .foregroundStyle(contextUsage.percentage >= 80 ? DesignTokens.warningColor : DesignTokens.primaryColor)
                                    Spacer()
                                    Text("\(formattedTokenCount(contextUsage.totalTokens)) / \(formattedTokenCount(contextUsage.estimatedTokenLimit))")
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }

                                GeometryReader { proxy in
                                    ZStack(alignment: .leading) {
                                        Capsule()
                                            .fill(Color.secondary.opacity(0.14))
                                        Capsule()
                                            .fill(contextUsage.percentage >= 80 ? DesignTokens.warningColor : DesignTokens.primaryColor)
                                            .frame(width: proxy.size.width * CGFloat(contextUsage.percentage) / 100.0)
                                    }
                                }
                                .frame(height: 10)
                            } else {
                                Label(tr("chat.context.metric.occupancy_unknown", "暂无已提交的上下文用量", "No committed context usage yet"), systemImage: "info.circle")
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }

                            LazyVGrid(
                                columns: [GridItem(.adaptive(minimum: 112), alignment: .leading)],
                                alignment: .leading,
                                spacing: 12
                            ) {
                                contextMetric(title: tr("chat.context.metric.measurement", "上下文", "Context"),
                                    value: contextUsage.contextIsEstimated
                                        ? tr("chat.context.metric.estimated_request", "预计下次请求", "Estimated next request")
                                        : tr("chat.context.metric.measured_request", "请求实测上下文", "Measured request context"))
                                if let request = contextUsage.cacheDiagnosticRequestID {
                                    contextMetric(title: tr("chat.context.metric.cache_request", "缓存诊断请求", "Cache diagnostic request"), value: request)
                                }
                                if let attempt = contextUsage.cacheDiagnosticAttemptID {
                                    contextMetric(title: tr("chat.context.metric.cache_attempt", "物理尝试", "Physical attempt"), value: attempt)
                                }
                                if contextUsage.hasEstimatedBreakdown {
                                    contextMetric(title: tr("chat.context.metric.active", "当前消息", "Active"), value: "\(contextUsage.activeMessageTokens)")
                                    contextMetric(title: tr("chat.context.metric.memory", "记忆", "Memory"), value: "\(contextUsage.longTermMemoryTokens)")
                                    contextMetric(title: tr("chat.context.metric.preferences", "偏好", "Preferences"), value: "\(contextUsage.userPreferencesTokens)")
                                    contextMetric(title: tr("chat.context.metric.mcp", "MCP", "MCP"), value: "\(contextUsage.mcpServersTokens)")
                                    contextMetric(title: tr("chat.context.metric.skills", "Skills", "Skills"), value: "\(contextUsage.uploadedSkillsTokens)")
                                    contextMetric(title: tr("chat.context.metric.documents", "文档", "Documents"), value: "\(contextUsage.uploadedDocumentsTokens)")
                                }
                                if let status = contextUsage.promptCacheStatus {
                                    contextMetric(
                                        title: tr("chat.context.metric.prompt_cache", "提示缓存", "Prompt cache"),
                                        value: promptCacheStatusLabel(status)
                                    )
                                }
                                contextMetric(
                                    title: tr("chat.context.metric.cache_hit_rate", "会话缓存命中率", "Conversation cache hit rate"),
                                    value: contextUsage.cacheHitRateBps.map { (Double($0) / 10_000).formatted(.percent.precision(.fractionLength(1))) }
                                        ?? tr("chat.context.metric.cache_unknown", "未知", "Unknown")
                                )
                                if let value = contextUsage.cacheReportedRequests {
                                    contextMetric(title: tr("chat.context.metric.cache_reported", "已报告请求", "Reported requests"), value: value.formatted())
                                }
                                if let value = contextUsage.cacheUnreportedRequests, value > 0 {
                                    contextMetric(title: tr("chat.context.metric.cache_unreported", "缓存用量未知的请求", "Requests with unknown cache usage"), value: value.formatted())
                                }
                                if let value = contextUsage.cachedInputTokens {
                                    contextMetric(
                                        title: tr("chat.context.metric.cache_read", "缓存读取", "Cache read"),
                                        value: formattedTokenCount(value)
                                    )
                                }
                                if let value = contextUsage.cacheWriteInputTokens {
                                    contextMetric(
                                        title: tr("chat.context.metric.cache_write", "缓存写入", "Cache write"),
                                        value: formattedTokenCount(value)
                                    )
                                }
                                if contextUsage.hasEstimatedBreakdown {
                                    contextMetric(title: tr("chat.context.metric.supplemental", "附加", "Supplemental"), value: "\(contextUsage.supplementalTokens)")
                                    contextMetric(title: tr("chat.context.metric.archived", "已折叠", "Archived"), value: "\(contextUsage.archivedMessageCount)")
                                }
                            }

                            if let lastCompactedAt = contextUsage.lastCompactedAt {
                                Text("\(tr("chat.context.last_compact.prefix", "上次 compact：", "Last compact: "))\(lastCompactedAt.formatted(date: .abbreviated, time: .shortened))")
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }

                            if !contextUsage.suggestionDetails.isEmpty {
                                VStack(alignment: .leading, spacing: 8) {
                                    ForEach(contextUsage.suggestionDetails) { suggestion in
                                        contextSuggestionCard(suggestion)
                                    }
                                }
                            } else if !contextUsage.suggestions.isEmpty {
                                VStack(alignment: .leading, spacing: 6) {
                                    ForEach(contextUsage.suggestions, id: \.self) { suggestion in
                                        Text("• \(suggestion)")
                                            .font(.caption)
                                            .foregroundColor(.secondary)
                                    }
                                }
                            }
                        }
                        .padding(.vertical, 4)
                    } else {
                        Text(tr("chat.context.usage.empty", "当前还没有可用的上下文统计。", "No context usage stats are available yet."))
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                    }
                } header: {
                    Text(tr("chat.context.usage.header", "Context usage", "Context usage"))
                } footer: {
                    Text(tr("chat.context.usage.footer", "compact 会把较早消息折叠到 session memory，但本地历史仍会保留。", "Compact folds earlier messages into session memory while preserving local history."))
                }
            }

            if displayMode == .context && selectedContextPane == .knowledge {
                Section {
                    HStack(spacing: 8) {
                        TextField(tr("chat.context.search.placeholder", "输入关键词检索 compact 前原始记忆", "Search original memory before compact with keywords"), text: $memorySearchQuery)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled(true)
                        Button {
                            runMemorySearch()
                        } label: {
                            ZStack {
                                Text(tr("chat.context.search.action", "检索", "Search"))
                                    .opacity(isSearchingMemory ? 0 : 1)
                                if isSearchingMemory {
                                    ProgressView()
                                }
                            }
                            .frame(minWidth: 44)
                        }
                        .buttonStyle(.bordered)
                        .disabled(isSearchingMemory || memorySearchQuery.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }

                    if memorySearchResults.isEmpty {
                        Text(tr("chat.context.search.empty", "暂无检索结果。", "No search results yet."))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    } else {
                        ForEach(memorySearchResults) { hit in
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(alignment: .top) {
                                    Text(hit.sourceLabel)
                                        .font(.caption.weight(.semibold))
                                        .foregroundColor(.primary)
                                    Spacer()
                                    if hit.score > 0 {
                                        Text("\(tr("chat.context.search.score_prefix", "匹配分", "Score")) \(hit.score)")
                                            .font(.caption2)
                                            .foregroundColor(.secondary)
                                    }
                                }
                                Text(hit.preview)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                    .lineLimit(3)
                            }
                            .padding(.vertical, 4)
                        }
                    }
                } header: {
                    Text(tr("chat.context.search.header", "原始记忆检索", "Raw memory search"))
                } footer: {
                    Text(tr("chat.context.search.footer", "检索 compact 快照与已折叠消息，用于回查原始信息；不会自动注入模型上下文。", "Search compact snapshots and archived messages for raw recall; results are not auto-injected into model context."))
                }
            }

            if displayMode == .context {
                if selectedContextPane == .overview {
                    Section {
                        TextEditor(text: $sessionMemory)
                            .frame(minHeight: 140)
                            .font(.body)
                            .accessibilityIdentifier("chat-context-session-memory")
                    } header: {
                        Text(tr("chat.context.session.header", "Session memory", "Session memory"))
                    } footer: {
                        Text(tr("chat.context.session.footer", "仅作用于当前对话，用来记录本轮任务的阶段信息和临时约束。", "Only affects this conversation; records task phase and temporary constraints."))
                    }
                }

                if selectedContextPane == .knowledge {
                    Section {
                        TextEditor(text: $knowledgePoints)
                            .frame(minHeight: 140)
                            .font(.body)
                            .accessibilityIdentifier("chat-context-knowledge")
                    } header: {
                        HStack {
                            Text(tr("chat.context.knowledge.header", "会话知识点", "Conversation knowledge"))
                            Spacer()
                        }
                    } footer: {
                        Text(tr("chat.context.knowledge.draft_footer", "整理当前会话的事实、决定、约束和未完成事项；点击保存后生效。", "Keep this conversation’s facts, decisions, constraints, and unresolved items. Choose Save to apply changes."))
                    }

                    Section {
                        TextEditor(text: $conversationWiki)
                            .frame(minHeight: 170)
                            .font(.body)
                            .accessibilityIdentifier("chat-context-wiki")
                    } header: {
                        Text(tr("chat.context.wiki.header", "会话 Wiki", "Conversation Wiki"))
                    } footer: {
                        Text(tr("chat.context.wiki.footer", "使用 Markdown 维护术语、架构、人物、流程和参考资料；仅属于当前会话。", "Use Markdown for terminology, architecture, people, workflows, and references. Scoped to this conversation only."))
                    }

                    Section {
                        TextEditor(text: $longTermMemory)
                            .frame(minHeight: 130)
                            .font(.body)
                    } header: {
                        Text(tr("chat.context.long_term.header", "长期记忆（全局）", "Long-term memory (global)"))
                    } footer: {
                        Text(tr("chat.context.long_term.footer", "跨对话持久保留，适合放稳定事实、项目约定与长期背景。", "Persists across conversations for stable facts, project conventions, and long-term context."))
                    }

                    Section {
                        TextEditor(text: $userPreferences)
                            .frame(minHeight: 130)
                            .font(.body)
                    } header: {
                        Text(tr("chat.context.preferences.header", "用户偏好（全局）", "User preferences (global)"))
                    } footer: {
                        Text(tr("chat.context.preferences.footer", "用于记录输出语气、交互偏好和决策倾向。", "Records response tone, interaction preferences, and decision tendencies."))
                    }

                    Section {
                        Button {
                            isSavingMemoryProfile = true
                            errorMessage = nil
                            Task {
                                do {
                                    try await onSaveMemoryProfile()
                                    initialMemoryText = [sessionMemory, longTermMemory, userPreferences]
                                } catch {
                                    errorMessage = error.localizedDescription
                                }
                                isSavingMemoryProfile = false
                            }
                        } label: {
                            HStack(spacing: 8) {
                                ZStack {
                                    if isSavingMemoryProfile {
                                        ProgressView()
                                            .controlSize(.small)
                                    }
                                }
                                .frame(width: 20, height: 20)
                                Text(tr("chat.context.memory.save", "保存长期记忆与用户偏好", "Save long-term memory and user preferences"))
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(isSavingMemoryProfile || isCompacting)
                    }
                }

                if selectedContextPane == .overview {
                    Section {
                        if let errorMessage, !errorMessage.isEmpty {
                            Text(errorMessage)
                                .font(.footnote)
                                .foregroundColor(.red)
                                .accessibilityIdentifier("chat.context.maintenance.error")
                        }
                        if let noticeMessage, !noticeMessage.isEmpty {
                            Text(noticeMessage)
                                .font(.footnote)
                                .foregroundColor(.secondary)
                                .accessibilityIdentifier("chat.context.maintenance.notice")
                        }
                        Button {
                            isCompacting = true
                            errorMessage = nil
                            Task {
                                do {
                                    noticeMessage = try await onCompact()
                                } catch {
                                    errorMessage = error.localizedDescription
                                }
                                isCompacting = false
                            }
                        } label: {
                            HStack(spacing: 8) {
                                ZStack {
                                    if isCompacting {
                                        ProgressView()
                                            .controlSize(.small)
                                    }
                                }
                                .frame(width: 20, height: 20)
                                Text(tr("chat.context.compact.run", "执行 compact 压缩", "Run compact"))
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        .disabled(isSavingMemoryProfile || isCompacting)

                        Button(tr("chat.context.reload", "重新加载上下文状态", "Reload context state")) {
                            guard !isReloading else { return }
                            isReloading = true
                            errorMessage = nil
                            Task { @MainActor in
                                defer { isReloading = false }
                                do { noticeMessage = try await onReload() }
                                catch { errorMessage = error.localizedDescription }
                            }
                        }
                        .font(.footnote)
                        .disabled(isCompacting || isReloading)
                    } header: {
                        Text(tr("chat.context.compact.header", "压缩上下文", "Compact context"))
                    } footer: {
                        Text(tr("chat.context.compact.footer", "compact 会折叠旧消息，降低 token 占用；原始历史仍保留在本地。", "Compact folds older messages to reduce token usage while keeping raw history locally."))
                    }
                }
            }

            if displayMode == .context && selectedContextPane == .tools {
                Section {
                    hostMCPServersList(items: hostMCPServers)
                    if hostMCPServers.isEmpty {
                        Text(tr("chat.context.mcp.host.empty", "当前宿主机没有可用的 MCP 服务器。", "The connected host has no available MCP servers."))
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                    }
                } header: {
                    Text(tr("chat.context.mcp.header", "MCP 能力", "MCP capabilities"))
                } footer: {
                    Text(tr("chat.context.mcp.host.footer", "连接、授权和能力发现由宿主机管理；此设备显示本次会话可用的能力。", "Connection, authorization, and capability discovery are managed by the host; this device shows what is available to this conversation."))
                }
            }

            if displayMode == .skills {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(tr("chat.context.skills.library", "Skill Library", "Skill Library"))
                            .font(.title3.weight(.semibold))
                        Text(tr("chat.context.skills.library.footer", "一个 Skill 只描述一个稳定工作流。启用后会注入后续对话，也可以在输入框直接点名使用。", "Each Skill should describe one stable workflow. Enabled skills are injected into later conversations and can be invoked directly in chat."))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    HStack(spacing: 10) {
                        Label(
                            "\(uploadedSkills.filter(\.enabled).count + hostUploadedSkills.filter(\.enabled).count) \(tr("chat.context.skills.enabled_count", "已启用", "enabled"))",
                            systemImage: "checkmark.circle.fill"
                        )
                        Spacer()
                        Text("\(uploadedSkills.count + hostUploadedSkills.count) \(tr("chat.context.skills.total_count", "个 Skill", "Skills"))")
                    }
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                } header: {
                    Text(tr("chat.context.skills.header", "Skills", "Skills"))
                }

                skillGallery(items: $uploadedSkills, hostItems: hostUploadedSkills)

                Section {
                    Button {
                        uploadedSkills.append(UserUploadedContextItem(
                            name: tr("chat.context.skills.new_skill_name", "新 Skill", "New Skill"),
                            content: skillTemplate,
                            description: tr("chat.context.skills.new_skill_description", "说明这个 skill 适合什么时候使用。", "Describe when this skill should be used."),
                            enabled: true
                        ))
                        selectedSkillID = uploadedSkills.last?.id
                        noticeMessage = tr("chat.context.skills.template_created", "已创建模板 Skill。", "Template Skill created.")
                    } label: {
                        Label(tr("chat.context.skills.create_template", "新建 Skill", "Create Skill"), systemImage: "plus")
                    }
                    Button {
                        skillImportOwner = ContextFileImportOwner(conversationID: conversationID)
                        showSkillImporter = true
                    } label: {
                        Label(tr("chat.context.skills.upload_file", "导入 Skill 文件", "Import Skill file"), systemImage: "square.and.arrow.down")
                    }
                } footer: {
                    Text(tr("chat.context.skills.footer", "本机 Skill 可编辑和删除；主机 Skill 由连接的主机管理。启用项会作为全局能力用于后续对话。", "Local Skills can be edited or removed. Host Skills are managed by the connected host. Enabled Skills become available across conversations."))
                }
            }

            if displayMode == .context && selectedContextPane == .tools {
                Section {
                    uploadedItemsList(
                        items: $uploadedDocuments,
                        emptyText: hostUploadedDocuments.isEmpty ? tr("chat.context.documents.empty", "还没有上传文档。", "No documents uploaded yet.") : ""
                    )
                    hostDocumentsList(items: hostUploadedDocuments)
                    Button {
                        documentImportOwner = ContextFileImportOwner(conversationID: conversationID)
                        showDocumentImporter = true
                    } label: {
                        Label(tr("chat.context.documents.upload", "上传文档文件", "Upload document file"), systemImage: "doc.badge.plus")
                    }
                } header: {
                    Text(tr("chat.context.documents.header", "用户文档", "User documents"))
                } footer: {
                    Text(tr("chat.context.documents.footer", "文档默认只保存在本机，用于增强当前设备上的对话上下文。", "Documents are stored locally by default to enhance chat context on this device."))
                }



                Section {
                    ForEach(SystemDataSourceAccessPolicyStore.shared.configurableTypes, id: \.self) { type in
                        let policy = SystemDataSourceAccessPolicyStore.shared.policy(for: type)
                        Button {
                            if selectedSystemDataSourceTypes.contains(type) {
                                selectedSystemDataSourceTypes.remove(type)
                            } else {
                                selectedSystemDataSourceTypes.insert(type)
                            }
                        } label: {
                            HStack(spacing: 12) {
                                Image(systemName: selectedSystemDataSourceTypes.contains(type) ? "checkmark.circle.fill" : "circle")
                                    .foregroundColor(selectedSystemDataSourceTypes.contains(type) ? DesignTokens.successColor : .secondary)
                                NolaIconBadge(
                                    icon: type.iconSystemName,
                                    tint: DesignTokens.primaryColor,
                                    size: DesignTokens.iconBadgeSizeSmall
                                )
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(type.displayName)
                                        .foregroundColor(.primary)
                                    Text(type.supportedEvents)
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                        .lineLimit(1)
                                    Label(policy.displayName, systemImage: policy.systemImageName)
                                        .font(.caption2.weight(.medium))
                                        .foregroundColor(policy == .blocked ? .red : .secondary)
                                }
                                Spacer()
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(NolaActionButtonStyle())
                        .disabled(policy == .blocked)

                        if type != SystemDataSourceAccessPolicyStore.shared.configurableTypes.last || !dataSources.isEmpty {
                            Divider()
                        }
                    }

                    if dataSources.isEmpty && SystemDataSourceAccessPolicyStore.shared.configurableTypes.isEmpty {
                        Text(NSLocalizedString("chat.context.sources.empty", value: "还没有可关联的数据源。网站、搜索结果、文档或文件夹等可读取来源会显示在这里。", comment: "No data sources for context"))
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                    } else if !dataSources.isEmpty {
                        ForEach(dataSources) { source in
                            Button {
                                if selectedDataSourceIDs.contains(source.id) {
                                    selectedDataSourceIDs.remove(source.id)
                                } else {
                                    selectedDataSourceIDs.insert(source.id)
                                }
                            } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: selectedDataSourceIDs.contains(source.id) ? "checkmark.circle.fill" : "circle")
                                        .foregroundColor(selectedDataSourceIDs.contains(source.id) ? DesignTokens.successColor : .secondary)
                                    NolaIconBadge(
                                        icon: source.kind.iconSystemName,
                                        tint: DesignTokens.primaryColor,
                                        size: DesignTokens.iconBadgeSizeSmall
                                    )
                                    VStack(alignment: .leading, spacing: 4) {
                                        Text(source.name)
                                            .foregroundColor(.primary)
                                        Text(source.contextDisplaySubtitle)
                                            .font(.caption)
                                            .foregroundColor(.secondary)
                                            .lineLimit(1)
                                        HStack(spacing: 6) {
                                            Text(source.kind.displayName)
                                                .font(.caption2.weight(.medium))
                                                .foregroundColor(.secondary)
                                            Text(source.accessPolicy.displayName)
                                                .font(.caption2.weight(.medium))
                                                .foregroundColor(source.accessPolicy == .blocked ? .red : .secondary)
                                            if source.isScheduledAutomation {
                                                Text(DataSourceCapability.scheduledOperation.displayName)
                                                    .font(.caption2.weight(.medium))
                                                    .foregroundColor(DesignTokens.warningColor)
                                            }
                                        }
                                    }
                                    Spacer()
                                    if !source.enabled {
                                        Text(NSLocalizedString("chat.context.sources.disabled", value: "已停用", comment: "Disabled context source"))
                                            .font(.caption)
                                            .foregroundColor(.secondary)
                                    }
                                }
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(NolaActionButtonStyle())
                            .disabled(source.accessPolicy == .blocked)
                        }
                    }
                } header: {
                    Text(NSLocalizedString("chat.context.sources.section", value: "附加数据源", comment: "Data sources section"))
                } footer: {
                    Text(NSLocalizedString("chat.context.sources.footer", value: "会把选中的可读取来源作为当前对话上下文。数据源可以来自网站、搜索结果、文件、文件夹、系统数据或 API；默认保存在本地。", comment: "Context data sources footer"))
                }
            }

            if !(displayMode == .context && selectedContextPane == .overview),
               let errorMessage, !errorMessage.isEmpty {
                Section {
                    Text(errorMessage)
                        .font(.footnote)
                        .foregroundColor(.red)
                }
            }

            if !(displayMode == .context && selectedContextPane == .overview),
               let noticeMessage, !noticeMessage.isEmpty {
                Section {
                    Text(noticeMessage)
                        .font(.footnote)
                        .foregroundColor(.secondary)
                }
            }
        }
        .navigationTitle(sheetTitle)
        .navigationBarTitleDisplayMode(.inline)
        .fileImporter(
            isPresented: $showSkillImporter,
            allowedContentTypes: [.data],
            allowsMultipleSelection: true
        ) { result in
            guard skillImportOwner?.conversationID == conversationID, skillImportOwner != nil else { return }
            skillImportOwner = nil
            let count = uploadedSkills.count
            importUploadedFiles(result, into: $uploadedSkills)
            if uploadedSkills.count > count { selectedSkillID = uploadedSkills.last?.id }
        }
        .fileImporter(
            isPresented: $showDocumentImporter,
            allowedContentTypes: [.data],
            allowsMultipleSelection: true
        ) { result in
            guard documentImportOwner?.conversationID == conversationID, documentImportOwner != nil else { return }
            documentImportOwner = nil
            importUploadedFiles(result, into: $uploadedDocuments)
        }
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button(L10n.cancel) {
                    onDiscard()
                    dismiss()
                }
                .disabled(isSavingMemoryProfile || isCompacting)
                .accessibilityIdentifier("chat.context.cancel")
            }
            ToolbarItem(placement: .confirmationAction) {
                Button(L10n.commonSave) {
                    runSaveAndDismiss()
                }
                .disabled(isSavingMemoryProfile || isCompacting)
            }
        }
        .disabled(isSavingMemoryProfile || isCompacting)
        .interactiveDismissDisabled(true)
        .onAppear {
            if initialMemoryText == nil {
                initialMemoryText = [sessionMemory, longTermMemory, userPreferences]
            }
        }
        .onChange(of: conversationID) { _, _ in
            skillImportOwner = nil
            documentImportOwner = nil
            showSkillImporter = false
            showDocumentImporter = false
            selectedContextPane = .overview
            memorySearchQuery = ""
            memorySearchResults = []
            errorMessage = nil
            noticeMessage = nil
        }
        .confirmationDialog(
            tr("chat.context.skills.delete_title", "删除这个 Skill？", "Delete this Skill?"),
            isPresented: Binding(
                get: { pendingSkillDeletionID != nil },
                set: { if !$0 { pendingSkillDeletionID = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(tr("chat.context.delete", "删除", "Delete"), role: .destructive) {
                deletePendingSkill()
            }
            Button(L10n.cancel, role: .cancel) {
                pendingSkillDeletionID = nil
            }
        } message: {
            Text(tr("chat.context.skills.delete_message", "删除后将不再注入后续对话。", "It will no longer be injected into future conversations."))
        }
        .confirmationDialog(
            tr("chat.context.mcp.delete_title", "删除这个 MCP 接口？", "Delete this MCP endpoint?"),
            isPresented: Binding(
                get: { pendingMCPDeletionID != nil },
                set: { if !$0 { pendingMCPDeletionID = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(tr("chat.context.delete", "删除", "Delete"), role: .destructive) {
                deletePendingMCPServer()
            }
            Button(L10n.cancel, role: .cancel) {
                pendingMCPDeletionID = nil
            }
        } message: {
            Text(tr("chat.context.mcp.delete_message", "删除后将无法再使用该接口提供的工具和资源。", "Tools and resources from this endpoint will no longer be available."))
        }
    }

    private func contextPaneTitle(_ pane: ContextPane) -> String {
        switch pane {
        case .overview:
            return tr("chat.context.pane.overview", "概览", "Overview")
        case .knowledge:
            return tr("chat.context.pane.knowledge", "知识", "Knowledge")
        case .tools:
            return tr("chat.context.pane.tools", "工具", "Tools")
        }
    }

    private var sheetTitle: String {
        switch displayMode {
        case .context:
            return NSLocalizedString("chat.context.title", value: "上下文", comment: "Chat context title")
        case .skills:
            return tr("chat.context.skills.header", "Skills", "Skills")
        }
    }

    private var sheetSubtitle: String {
        switch displayMode {
        case .context:
            return NSLocalizedString("chat.context.sheet.subtitle", value: "查看上下文占用、整理 session memory，并维护跨对话生效的长期记忆与用户偏好。", comment: "Chat context sheet subtitle")
        case .skills:
            return tr("chat.context.skills.sheet_subtitle", "独立维护你的技能库：按场景拆分 skill，启用后自动进入后续对话上下文。", "Maintain your skill library independently: split skills by scenario, and enabled ones automatically join later context.")
        }
    }

    private func runSaveAndDismiss() {
        switch displayMode {
        case .context:
            let snapshot = knowledgeSaveSnapshot
            isSavingMemoryProfile = true
            errorMessage = nil
            Task { @MainActor in
                defer { isSavingMemoryProfile = false }
                var savedAnyPart = false
                do {
                    if initialMemoryText != [sessionMemory, longTermMemory, userPreferences] {
                        try await onSaveMemoryProfile()
                        initialMemoryText = [sessionMemory, longTermMemory, userPreferences]
                        savedAnyPart = true
                    }
                    try onSave()
                    savedAnyPart = true
                    try await onSaveConversationKnowledge(snapshot)
                    dismiss()
                } catch {
                    let partialNotice = savedAnyPart
                        ? NSLocalizedString("chat.context.partial_save", value: "Some changes were saved. Your remaining edits are retained; retry Save to finish.", comment: "Partial context save failure") + "\n"
                        : ""
                    errorMessage = partialNotice + error.localizedDescription
                }
            }
        case .skills:
            isSavingMemoryProfile = true
            errorMessage = nil
            Task {
                do {
                    try await onSaveMemoryProfile()
                    dismiss()
                } catch {
                    errorMessage = error.localizedDescription
                }
                isSavingMemoryProfile = false
            }
        }
    }

    private var knowledgeSaveSnapshot: ConversationKnowledgeSaveSnapshot {
        ConversationKnowledgeSaveSnapshot(
            conversationID: conversationID,
            knowledgePoints: knowledgePoints,
            wiki: conversationWiki
        )
    }

    private func runMemorySearch() {
        isSearchingMemory = true
        errorMessage = nil
        Task {
            do {
                memorySearchResults = try await onSearchMemory(memorySearchQuery)
            } catch {
                errorMessage = error.localizedDescription
            }
            isSearchingMemory = false
        }
    }

    private func contextMetric(title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption2)
                .foregroundColor(.secondary)
            Text(value)
                .font(.caption.weight(.semibold))
                .foregroundColor(.primary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func formattedTokenCount(_ value: Int) -> String {
        if value >= 1_000_000 {
            return String(format: "%.1fM", Double(value) / 1_000_000)
        }
        if value >= 1_000 {
            return String(format: "%.1fK", Double(value) / 1_000)
        }
        return String(value)
    }

    private func promptCacheStatusLabel(_ status: String) -> String {
        switch status {
        case "disabled": return tr("chat.context.cache.disabled", "已关闭", "Disabled")
        case "invalid_identity": return tr("chat.context.cache.invalid", "标识无效", "Invalid identity")
        case "first_request": return tr("chat.context.cache.first_request", "首次请求", "First request")
        case "hit": return tr("chat.context.cache.hit", "命中", "Hit")
        case "write": return tr("chat.context.cache.write", "已写入", "Stored")
        case "miss": return tr("chat.context.cache.miss", "未命中", "Miss")
        case "stable_candidate": return tr("chat.context.cache.candidate", "稳定候选", "Stable candidate")
        default: return tr("chat.context.cache.unknown", "状态未知", "Unknown")
        }
    }

    private func contextSuggestionTint(
        _ severity: ConversationContextUsage.ContextSuggestion.Severity
    ) -> Color {
        switch severity {
        case .high:
            return DesignTokens.dangerColor
        case .medium:
            return DesignTokens.warningColor
        case .low:
            return DesignTokens.primaryColor
        case .info:
            return .secondary
        }
    }

    private func contextSuggestionCard(
        _ suggestion: ConversationContextUsage.ContextSuggestion
    ) -> some View {
        let tint = contextSuggestionTint(suggestion.severity)
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 8) {
                Text(suggestion.title)
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.primary)
                Spacer()
                if suggestion.estimatedSavingsTokens > 0 {
                    Text("~\(suggestion.estimatedSavingsTokens) \(tr("chat.context.tokens", "tokens", "tokens"))")
                        .font(.caption2)
                        .foregroundColor(.secondary)
                }
            }
            if !suggestion.detail.isEmpty {
                Text(suggestion.detail)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding(10)
        .background(tint.opacity(0.10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(tint.opacity(0.35), lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: 10))
    }

    private func uploadedItemsList(items: Binding<[UserUploadedContextItem]>, emptyText: String) -> some View {
        Group {
            if items.wrappedValue.isEmpty && !emptyText.isEmpty {
                Text(emptyText)
                    .font(.subheadline)
                    .foregroundColor(.secondary)
            }
            ForEach(items) { $item in
                VStack(alignment: .leading, spacing: 8) {
                    Toggle(tr("chat.context.toggle.enabled", "启用", "Enabled"), isOn: $item.enabled)
                    TextField(tr("chat.context.field.name", "名称", "Name"), text: $item.name)
                    Text("\(item.content.count) \(tr("chat.context.unit.characters", "字符", "characters"))")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    TextEditor(text: $item.content)
                        .frame(minHeight: 90)
                        .font(.caption)
                }
                .padding(.vertical, 4)
            }
            .onDelete { offsets in
                items.wrappedValue.remove(atOffsets: offsets)
            }
        }
    }

    @ViewBuilder
    private func skillGallery(
        items: Binding<[UserUploadedContextItem]>,
        hostItems: [UserUploadedContextItem]
    ) -> some View {
        let hasSkills = !items.wrappedValue.isEmpty || !hostItems.isEmpty
        if !hasSkills {
            Section {
                ContentUnavailableView(
                    tr("chat.context.skills.empty_title", "还没有 Skill", "No Skills yet"),
                    systemImage: "shippingbox",
                    description: Text(tr("chat.context.skills.empty", "新建或导入一个 Skill，建立可重复使用的工作流。", "Create or import a Skill to build a reusable workflow."))
                )
            }
        } else {
            ForEach(SkillVisualCategory.allCases) { category in
                let localIndices = items.wrappedValue.indices.filter {
                    skillCategory(for: items.wrappedValue[$0]) == category
                }
                let remoteItems = hostItems.filter { skillCategory(for: $0) == category }
                if !localIndices.isEmpty || !remoteItems.isEmpty {
                    Section {
                        LazyVGrid(
                            columns: [GridItem(.adaptive(minimum: 148), spacing: 12)],
                            alignment: .leading,
                            spacing: 12
                        ) {
                            ForEach(localIndices, id: \.self) { index in
                                localSkillCard(item: items[index], category: category)
                            }
                            ForEach(remoteItems) { item in
                                hostSkillCard(item: item, category: category)
                            }
                        }
                        .padding(.vertical, 4)
                    } header: {
                        Label(skillCategoryTitle(category), systemImage: skillCategorySymbol(category))
                    }
                }
            }
        }
    }

    private func localSkillCard(
        item: Binding<UserUploadedContextItem>,
        category: SkillVisualCategory
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top) {
                skillCategoryIcon(category)
                Spacer(minLength: 8)
                Toggle("", isOn: item.enabled)
                    .labelsHidden()
                    .controlSize(.small)
                    .accessibilityLabel(tr("chat.context.toggle.enabled", "启用", "Enabled"))
            }

            NavigationLink {
                localSkillDetail(item: item, category: category)
            } label: {
                VStack(alignment: .leading, spacing: 5) {
                    Text(skillDisplayName(item.wrappedValue))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                    Text(skillSummary(item.wrappedValue))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                    HStack(spacing: 4) {
                        Text(tr("chat.context.skills.local", "本机", "On device"))
                        Spacer(minLength: 4)
                    }
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(NolaActionButtonStyle())
        }
        .padding(13)
        .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
        .background(Color.secondary.opacity(0.055), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.secondary.opacity(0.10), lineWidth: 1)
        )
        .accessibilityElement(children: .contain)
    }

    private func hostSkillCard(
        item: UserUploadedContextItem,
        category: SkillVisualCategory
    ) -> some View {
        NavigationLink {
            hostSkillDetail(item: item, category: category)
        } label: {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .top) {
                    skillCategoryIcon(category)
                    Spacer(minLength: 8)
                    Image(systemName: item.enabled ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(item.enabled ? DesignTokens.successColor : Color.secondary)
                }
                Text(skillDisplayName(item))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                Text(skillSummary(item))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                HStack(spacing: 4) {
                    Text(tr("chat.context.skills.host", "主机", "Host"))
                    Spacer(minLength: 4)
                }
                .font(.caption2.weight(.medium))
                .foregroundStyle(.tertiary)
            }
            .padding(13)
            .frame(maxWidth: .infinity, minHeight: 150, alignment: .topLeading)
            .background(Color.secondary.opacity(0.055), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(Color.secondary.opacity(0.10), lineWidth: 1)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(NolaActionButtonStyle())
    }

    private func localSkillDetail(
        item: Binding<UserUploadedContextItem>,
        category: SkillVisualCategory
    ) -> some View {
        Form {
            Section {
                HStack(spacing: 12) {
                    skillCategoryIcon(category)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(skillCategoryTitle(category))
                            .font(.subheadline.weight(.semibold))
                        Text(tr("chat.context.skills.local_editable", "保存在本机，可编辑", "Stored on this device and editable"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Toggle(tr("chat.context.toggle.enabled", "启用", "Enabled"), isOn: item.enabled)
                        .labelsHidden()
                }
            }
            Section {
                TextField(tr("chat.context.skills.name", "Skill 名称", "Skill name"), text: item.name)
                TextField(tr("chat.context.skills.description", "说明：什么时候使用这个 Skill", "Description: when to use this Skill"), text: item.description, axis: .vertical)
                    .lineLimit(2...5)
            } header: {
                Text(tr("chat.context.skills.identity", "名称与用途", "Identity and purpose"))
            }
            Section {
                TextEditor(text: item.content)
                    .frame(minHeight: 260)
                    .font(.system(.caption, design: .monospaced))
            } header: {
                HStack {
                    Text(tr("chat.context.skills.instructions", "Skill 内容", "Skill instructions"))
                    Spacer()
                    Text("\(item.wrappedValue.content.count) \(tr("chat.context.unit.characters", "字符", "characters"))")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Section {
                Button {
                    onUseSkill(skillInvocation(for: item.wrappedValue))
                } label: {
                    Label(tr("chat.context.skills.use", "在对话中使用", "Use in conversation"), systemImage: "arrow.turn.down.left")
                }
                Button {
                    let invocation = skillInvocation(for: item.wrappedValue)
                    UIPasteboard.general.string = invocation
                    noticeMessage = tr("chat.context.skills.copied", "已复制调用语句", "Invocation copied")
                } label: {
                    Label(tr("chat.context.copy", "复制调用语句", "Copy invocation"), systemImage: "doc.on.doc")
                }
                Button(role: .destructive) {
                    pendingSkillDeletionID = item.wrappedValue.id
                } label: {
                    Label(tr("chat.context.delete", "删除 Skill", "Delete Skill"), systemImage: "trash")
                }
            }
        }
        .navigationTitle(skillDisplayName(item.wrappedValue))
        .navigationBarTitleDisplayMode(.inline)
    }

    private func hostSkillDetail(
        item: UserUploadedContextItem,
        category: SkillVisualCategory
    ) -> some View {
        List {
            Section {
                HStack(spacing: 12) {
                    skillCategoryIcon(category)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(skillDisplayName(item))
                            .font(.headline)
                        Text(tr("chat.context.skills.host_managed", "由连接的主机提供和管理", "Provided and managed by the connected host"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            if !item.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Section(tr("chat.context.skills.purpose", "适用场景", "When to use")) {
                    Text(item.description)
                }
            }
            Section(tr("chat.context.skills.instructions", "Skill 内容", "Skill instructions")) {
                Text(item.content)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
            }
            Section {
                Button {
                    onUseSkill(skillInvocation(for: item))
                } label: {
                    Label(tr("chat.context.skills.use", "在对话中使用", "Use in conversation"), systemImage: "arrow.turn.down.left")
                }
            }
        }
        .navigationTitle(skillDisplayName(item))
        .navigationBarTitleDisplayMode(.inline)
    }

    private func skillCategory(for item: UserUploadedContextItem) -> SkillVisualCategory {
        let value = "\(item.name) \(item.description) \(item.content.prefix(500))".lowercased()
        if value.contains("word") || value.contains("excel") || value.contains("powerpoint") || value.contains("ppt") || value.contains("docx") || value.contains("xlsx") || value.contains("文档") || value.contains("表格") || value.contains("演示文稿") { return .documents }
        if value.contains("data") || value.contains("csv") || value.contains("sql") || value.contains("分析") || value.contains("统计") || value.contains("图表") { return .data }
        if value.contains("figma") || value.contains("design") || value.contains("image") || value.contains("设计") || value.contains("视觉") || value.contains("图片") { return .design }
        if value.contains("email") || value.contains("meeting") || value.contains("translate") || value.contains("邮件") || value.contains("会议") || value.contains("翻译") || value.contains("沟通") { return .communication }
        if value.contains("automation") || value.contains("schedule") || value.contains("workflow") || value.contains("自动化") || value.contains("定时") || value.contains("任务") { return .automation }
        if value.contains("research") || value.contains("search") || value.contains("arxiv") || value.contains("研究") || value.contains("检索") || value.contains("搜索") { return .research }
        if value.contains("file") || value.contains("workspace") || value.contains("git") || value.contains("code") || value.contains("文件") || value.contains("工作区") || value.contains("代码") { return .workspace }
        return .custom
    }

    private func skillCategoryTitle(_ category: SkillVisualCategory) -> String {
        switch category {
        case .documents: return tr("chat.context.skills.category.documents", "文档与表格", "Documents and sheets")
        case .data: return tr("chat.context.skills.category.data", "数据分析", "Data analysis")
        case .design: return tr("chat.context.skills.category.design", "设计与创作", "Design and creation")
        case .communication: return tr("chat.context.skills.category.communication", "沟通协作", "Communication")
        case .automation: return tr("chat.context.skills.category.automation", "计划与自动化", "Planning and automation")
        case .research: return tr("chat.context.skills.category.research", "知识与研究", "Knowledge and research")
        case .workspace: return tr("chat.context.skills.category.workspace", "工作区与代码", "Workspace and code")
        case .custom: return tr("chat.context.skills.category.custom", "其他", "Other")
        }
    }

    private func skillCategorySymbol(_ category: SkillVisualCategory) -> String {
        switch category {
        case .documents: return "doc.text.fill"
        case .data: return "chart.xyaxis.line"
        case .design: return "paintpalette.fill"
        case .communication: return "bubble.left.and.bubble.right.fill"
        case .automation: return "clock.arrow.trianglehead.counterclockwise.rotate.90"
        case .research: return "sparkle.magnifyingglass"
        case .workspace: return "folder.fill"
        case .custom: return "shippingbox.fill"
        }
    }

    private func skillCategoryTint(_ category: SkillVisualCategory) -> Color {
        switch category {
        case .documents: return .blue
        case .data: return .green
        case .design: return .pink
        case .communication: return .cyan
        case .automation: return .orange
        case .research: return .indigo
        case .workspace: return .teal
        case .custom: return .secondary
        }
    }

    private func skillCategoryIcon(_ category: SkillVisualCategory) -> some View {
        let tint = skillCategoryTint(category)
        return Image(systemName: skillCategorySymbol(category))
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(tint)
            .frame(width: 36, height: 36)
            .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private func mcpServersList(items: Binding<[UserMCPServer]>) -> some View {
        ForEach(items) { $server in
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 12) {
                    Button {
                        selectedMCPServerID = selectedMCPServerID == server.id ? nil : server.id
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: "point.3.connected.trianglepath.dotted")
                                .font(.system(size: 14, weight: .semibold))
                                .foregroundStyle(DesignTokens.primaryColor)
                                .frame(width: 34, height: 34)
                                .background(DesignTokens.primaryColor.opacity(0.10), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                            VStack(alignment: .leading, spacing: 3) {
                                Text(server.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? tr("chat.context.mcp.default_name", "MCP Server", "MCP Server") : server.name)
                                    .font(.subheadline.weight(.semibold))
                                    .foregroundStyle(.primary)
                                Text(server.endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? tr("chat.context.mcp.endpoint_required", "等待填写服务地址", "Endpoint required") : server.endpoint)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            Spacer(minLength: 8)
                            Image(systemName: selectedMCPServerID == server.id ? "chevron.up" : "chevron.right")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.tertiary)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(NolaActionButtonStyle())

                    Toggle("", isOn: $server.enabled)
                        .labelsHidden()
                        .accessibilityLabel(tr("chat.context.toggle.enabled", "启用", "Enabled"))
                }

                if selectedMCPServerID == server.id {
                    Divider()
                    TextField(tr("chat.context.field.name", "名称", "Name"), text: $server.name)
                    TextField("https://example.com/mcp", text: $server.endpoint)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                    TextField(tr("chat.context.field.description_optional", "说明（可选）", "Description (optional)"), text: $server.description, axis: .vertical)
                        .lineLimit(1...3)

                    HStack {
                        Spacer()
                        Button(role: .destructive) {
                            pendingMCPDeletionID = server.id
                        } label: {
                            Label(tr("chat.context.delete", "删除", "Delete"), systemImage: "trash")
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            .padding(12)
            .background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
    }

    private func skillDisplayName(_ item: UserUploadedContextItem) -> String {
        item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? tr("chat.context.skills.untitled", "未命名 Skill", "Untitled Skill")
            : item.name
    }

    private func skillSummary(_ item: UserUploadedContextItem) -> String {
        let description = item.description.trimmingCharacters(in: .whitespacesAndNewlines)
        if !description.isEmpty {
            return description
        }
        return item.enabled
            ? tr("chat.context.skills.enabled_desc", "已启用，会进入后续对话上下文", "Enabled; will be injected into later context")
            : tr("chat.context.skills.disabled_desc", "已停用，不占用上下文", "Disabled; does not consume context")
    }

    private func deletePendingSkill() {
        guard let id = pendingSkillDeletionID else { return }
        uploadedSkills.removeAll { $0.id == id }
        if selectedSkillID == id {
            selectedSkillID = nil
        }
        pendingSkillDeletionID = nil
    }

    private func deletePendingMCPServer() {
        guard let id = pendingMCPDeletionID else { return }
        mcpServers.removeAll { $0.id == id }
        if selectedMCPServerID == id {
            selectedMCPServerID = nil
        }
        pendingMCPDeletionID = nil
    }

    private func hostSkillsList(items: [UserUploadedContextItem]) -> some View {
        Group {
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text(tr("chat.context.skills.host.header", "Web 主机 Skills", "Web host Skills"))
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    ForEach(items) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Text(item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? tr("chat.host_skill.default_name", "Host Skill", "Host Skill") : item.name)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(item.enabled
                                     ? tr("chat.context.skills.host.enabled", "主机启用", "Host enabled")
                                     : tr("chat.context.skills.host.disabled", "主机停用", "Host disabled"))
                                    .font(.caption2.weight(.semibold))
                                    .foregroundColor(item.enabled ? DesignTokens.successColor : .secondary)
                            }
                            if !item.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                Text(item.description)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            HStack {
                                Label("\(item.content.count) \(tr("chat.context.unit.characters", "字符", "characters"))", systemImage: "desktopcomputer")
                                Spacer()
                                Button {
                                    let invocation = skillInvocation(for: item)
                                    onUseSkill(invocation)
                                } label: {
                                    Text(tr("chat.context.use", "使用", "Use"))
                                }
                                .buttonStyle(.borderless)
                            }
                            .font(.caption)
                            .foregroundColor(.secondary)
                        }
                        .padding(10)
                        .background(DesignTokens.primaryColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }

    private func hostDocumentsList(items: [UserUploadedContextItem]) -> some View {
        Group {
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text(tr("chat.context.documents.host.header", "宿主机文档", "Host documents"))
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    ForEach(items) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Text(item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? tr("chat.host_document.default_name", "Host Document", "Host Document") : item.name)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(item.enabled
                                     ? tr("chat.context.documents.host.enabled", "主机启用", "Host enabled")
                                     : tr("chat.context.documents.host.disabled", "主机停用", "Host disabled"))
                                    .font(.caption2.weight(.semibold))
                                    .foregroundColor(item.enabled ? DesignTokens.successColor : .secondary)
                            }
                            if !item.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                Text(item.description)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Label("\(item.content.count) \(tr("chat.context.unit.characters", "字符", "characters"))", systemImage: "doc.text")
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                        .padding(10)
                        .background(DesignTokens.primaryColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }

    private func hostMCPServersList(items: [UserMCPServer]) -> some View {
        Group {
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text(tr("chat.context.mcp.host.header", "宿主机 MCP", "Host MCP"))
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    ForEach(items) { server in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(alignment: .firstTextBaseline) {
                                Text(server.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? tr("chat.host_mcp.default_name", "Host MCP Server", "Host MCP Server") : server.name)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(server.availability == "available"
                                     ? tr("chat.context.mcp.host.connected", "已连接", "Connected")
                                     : tr("chat.context.mcp.host.unavailable", "不可用", "Unavailable"))
                                    .font(.caption2.weight(.semibold))
                                    .foregroundColor(server.availability == "available" ? DesignTokens.successColor : .secondary)
                            }
                            if !server.endpoint.isEmpty {
                                Text(server.endpoint)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                            }
                            if let version = server.protocolVersion, !version.isEmpty {
                                Text("MCP \(version)")
                                    .font(.caption2.monospaced())
                                    .foregroundColor(.secondary)
                            }
                            if !server.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                Text(server.description)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            let catalogCount = (server.tools?.count ?? 0)
                                + (server.resources?.count ?? 0)
                                + (server.resourceTemplates?.count ?? 0)
                                + (server.prompts?.count ?? 0)
                            DisclosureGroup(
                                String(
                                    format: tr("chat.context.mcp.host.catalog_count", "能力目录 · %d 项", "Capability catalog · %d items"),
                                    catalogCount
                                )
                            ) {
                                hostMCPNames(tr("chat.context.mcp.host.tools", "工具", "Tools"), names: server.tools ?? [])
                                hostMCPNames(tr("chat.context.mcp.host.resources", "资源", "Resources"), names: server.resources ?? [])
                                hostMCPNames(tr("chat.context.mcp.host.templates", "资源模板", "Resource templates"), names: server.resourceTemplates ?? [])
                                hostMCPNames(tr("chat.context.mcp.host.prompts", "提示词", "Prompts"), names: server.prompts ?? [])
                            }
                            .disclosureGroupStyle(NolaRowDisclosureStyle())
                            .font(.caption)
                        }
                        .padding(10)
                        .background(DesignTokens.primaryColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }

    @ViewBuilder
    private func hostMCPNames(_ title: String, names: [String]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("\(title) · \(names.count)")
                .font(.caption2.weight(.semibold))
                .foregroundColor(.secondary)
            ForEach(names.prefix(20), id: \.self) { name in
                Text(name)
                    .font(.caption2.monospaced())
                    .textSelection(.enabled)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    private func skillInvocation(for item: UserUploadedContextItem) -> String {
        let name = item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? tr("chat.context.skills.invocation.default_name", "Skill", "Skill") : item.name
        return "\(tr("chat.context.skills.invocation.prefix", "使用「", "Use \""))\(name)\(tr("chat.context.skills.invocation.suffix", "」skill：", "\" skill:"))"
    }

    private func tr(_ key: String, _ zh: String, _ en: String) -> String {
        let preferred = (Locale.preferredLanguages.first ?? "").lowercased()
        let fallback = preferred.hasPrefix("zh") ? zh : en
        return NSLocalizedString(key, value: fallback, comment: "")
    }

    private var skillTemplate: String {
        """
        # Skill: New Skill

        ## When to use
        Describe the exact scenario where this skill should be used.

        ## Workflow
        1. Clarify the user goal.
        2. Inspect relevant context.
        3. Produce a concise, actionable result.

        ## Output style
        - Keep the response structured.
        - Call out assumptions and risks.
        """
    }

    private func importUploadedFiles(_ result: Result<[URL], Error>, into items: Binding<[UserUploadedContextItem]>) {
        do {
            let urls = try result.get()
            var failures = 0
            for url in urls {
                do {
                    let text = try IOSContextFileImportPolicy.readText(from: url, maximumCharacters: 40_000)
                    items.wrappedValue.append(UserUploadedContextItem(
                        name: url.lastPathComponent, content: text,
                        description: firstNonEmptyLine(in: text)
                    ))
                } catch { failures += 1 }
            }
            if items.wrappedValue.count > 30 {
                items.wrappedValue = Array(items.wrappedValue.suffix(30))
            }
            if failures > 0 {
                errorMessage = String(format: NSLocalizedString(
                    "chat.context.import.failed_count", value: "%d files could not be imported. Use nonempty UTF-8 text files with at most 40,000 characters (192 KB).", comment: "Context import failures"
                ), failures)
            }
        } catch {
            if !IOSContextFileImportPolicy.isCancellation(error) { errorMessage = error.localizedDescription }
        }
    }

    private func firstNonEmptyLine(in text: String) -> String {
        text
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }?
            .replacingOccurrences(of: #"^#+\s*"#, with: "", options: .regularExpression)
            .prefix(160)
            .description ?? ""
    }
}
