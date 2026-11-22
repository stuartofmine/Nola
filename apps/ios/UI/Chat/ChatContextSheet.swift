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
    static func importFiles(_ urls: [URL], existing: [UserUploadedContextItem]) -> (items: [UserUploadedContextItem], problems: [String]) {
        var items = existing
        var problems = [String]()
        var overLimit = [String]()
        for url in urls {
            guard items.count < 30 else { overLimit.append(url.lastPathComponent); continue }
            do {
                let text = try readText(from: url, maximumCharacters: 40_000)
                items.append(UserUploadedContextItem(name: url.lastPathComponent, content: text, description: firstNonEmptyLine(in: text)))
            } catch {
                problems.append("\(url.lastPathComponent): \(error.localizedDescription)")
            }
        }
        if !overLimit.isEmpty {
            problems.append(String(format: String(localized: "chat.context.import.limit"), overLimit.joined(separator: ", ")))
        }
        return (items, problems)
    }

    private static func firstNonEmptyLine(in text: String) -> String {
        text
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }?
            .replacingOccurrences(of: #"^#+\s*"#, with: "", options: .regularExpression)
            .prefix(160)
            .description ?? ""
    }

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
    var documentsReadable = true
    let dataSources: [URLDataSource]
    @Binding var selectedDataSourceIDs: Set<UUID>
    @Binding var selectedSystemDataSourceTypes: Set<DataSourceType>
    let onSave: () throws -> Void
    let onSaveMemoryProfile: () async throws -> Void
    var onSaveUploadedDocuments: (([UserUploadedContextItem]) throws -> [UserUploadedContextItem])? = nil
    let onSaveConversationKnowledge: (ConversationKnowledgeSaveSnapshot) async throws -> Void
    let onCompact: () async throws -> String
    let onReload: () async throws -> String?
    let onDiscard: () -> Void
    let onUseSkill: (String) -> Void
    let onSearchMemory: (String) async throws -> [ContextMemorySearchHit]
    var initialContextPane: ContextPane = .overview
    var embeddedInNavigationStack: Bool = false

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
    @State private var selectedContextPane: ContextPane
    @State private var selectedSkillID: UUID?
    @State private var selectedMCPServerID: UUID?
    @State private var pendingSkillDeletionID: UUID?
    @State private var pendingMCPDeletionID: UUID?
    @State private var initialDocuments: [UserUploadedContextItem]?
    @State private var initialMemoryText: [String]?
    @State private var showContextDiagnostics = false

    init(
        displayMode: DisplayMode,
        conversationID: UUID?,
        conversationTitle: String,
        contextUsage: ConversationContextUsage?,
        sessionMemory: Binding<String>,
        knowledgePoints: Binding<String>,
        conversationWiki: Binding<String>,
        longTermMemory: Binding<String>,
        userPreferences: Binding<String>,
        mcpServers: Binding<[UserMCPServer]>,
        uploadedSkills: Binding<[UserUploadedContextItem]>,
        hostUploadedSkills: [UserUploadedContextItem],
        hostUploadedDocuments: [UserUploadedContextItem],
        hostMCPServers: [UserMCPServer],
        uploadedDocuments: Binding<[UserUploadedContextItem]>,
        documentsReadable: Bool = true,
        dataSources: [URLDataSource],
        selectedDataSourceIDs: Binding<Set<UUID>>,
        selectedSystemDataSourceTypes: Binding<Set<DataSourceType>>,
        onSave: @escaping () throws -> Void,
        onSaveMemoryProfile: @escaping () async throws -> Void,
        onSaveUploadedDocuments: (([UserUploadedContextItem]) throws -> [UserUploadedContextItem])? = nil,
        onSaveConversationKnowledge: @escaping (ConversationKnowledgeSaveSnapshot) async throws -> Void,
        onCompact: @escaping () async throws -> String,
        onReload: @escaping () async throws -> String?,
        onDiscard: @escaping () -> Void,
        onUseSkill: @escaping (String) -> Void,
        onSearchMemory: @escaping (String) async throws -> [ContextMemorySearchHit],
        initialContextPane: ContextPane = .overview,
        embeddedInNavigationStack: Bool = false
    ) {
        self.displayMode = displayMode
        self.conversationID = conversationID
        self.conversationTitle = conversationTitle
        self.contextUsage = contextUsage
        _sessionMemory = sessionMemory
        _knowledgePoints = knowledgePoints
        _conversationWiki = conversationWiki
        _longTermMemory = longTermMemory
        _userPreferences = userPreferences
        _mcpServers = mcpServers
        _uploadedSkills = uploadedSkills
        self.hostUploadedSkills = hostUploadedSkills
        self.hostUploadedDocuments = hostUploadedDocuments
        self.hostMCPServers = hostMCPServers
        _uploadedDocuments = uploadedDocuments
        self.documentsReadable = documentsReadable
        self.dataSources = dataSources
        _selectedDataSourceIDs = selectedDataSourceIDs
        _selectedSystemDataSourceTypes = selectedSystemDataSourceTypes
        self.onSave = onSave
        self.onSaveMemoryProfile = onSaveMemoryProfile
        self.onSaveUploadedDocuments = onSaveUploadedDocuments
        self.onSaveConversationKnowledge = onSaveConversationKnowledge
        self.onCompact = onCompact
        self.onReload = onReload
        self.onDiscard = onDiscard
        self.onUseSkill = onUseSkill
        self.onSearchMemory = onSearchMemory
        self.initialContextPane = initialContextPane
        self.embeddedInNavigationStack = embeddedInNavigationStack
        _selectedContextPane = State(initialValue: initialContextPane)
        _initialDocuments = State(initialValue: uploadedDocuments.wrappedValue)
        _initialMemoryText = State(initialValue: [sessionMemory.wrappedValue, longTermMemory.wrappedValue, userPreferences.wrappedValue])
    }

    private var hasUnsavedChanges: Bool {
        let memoryChanged = initialMemoryText.map { $0 != [sessionMemory, longTermMemory, userPreferences] } ?? false
        let documentsChanged = initialDocuments.map { $0 != uploadedDocuments } ?? false
        return memoryChanged || documentsChanged
    }

    var body: some View {
        Form {
            Group {
            Section {
                ChatConversationScopeSummary(
                    symbol: displayMode == .context ? "circle.hexagongrid" : "shippingbox",
                    title: conversationTitle,
                    detail: sheetSubtitle,
                    scopeLabel: displayMode == .context
                        ? String(localized: "chat.scope.current_conversation")
                        : String(localized: "chat.scope.across_conversations")
                )
            }

            if displayMode == .context {
                Section {
                    Picker(
                        String(localized: "chat.context.pane.picker"),
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
                                Label(String(localized: "chat.context.metric.occupancy_unknown"), systemImage: "info.circle")
                                    .font(.callout)
                                    .foregroundStyle(.secondary)
                            }

                            DisclosureGroup(isExpanded: $showContextDiagnostics) {
                                LazyVGrid(
                                    columns: [GridItem(.adaptive(minimum: 112), alignment: .leading)],
                                    alignment: .leading,
                                    spacing: 12
                                ) {
                                    contextMetric(title: String(localized: "chat.context.metric.measurement"),
                                        value: contextUsage.contextIsEstimated
                                            ? String(localized: "chat.context.metric.estimated_request")
                                            : String(localized: "chat.context.metric.measured_request"))
                                    if let request = contextUsage.cacheDiagnosticRequestID {
                                        contextMetric(title: String(localized: "chat.context.metric.cache_request"), value: request)
                                    }
                                    if let attempt = contextUsage.cacheDiagnosticAttemptID {
                                        contextMetric(title: String(localized: "chat.context.metric.cache_attempt"), value: attempt)
                                    }
                                    if contextUsage.hasEstimatedBreakdown {
                                        contextMetric(title: String(localized: "chat.context.metric.active"), value: "\(contextUsage.activeMessageTokens)")
                                        contextMetric(title: String(localized: "chat.context.metric.memory"), value: "\(contextUsage.longTermMemoryTokens)")
                                        contextMetric(title: String(localized: "chat.context.metric.preferences"), value: "\(contextUsage.userPreferencesTokens)")
                                        contextMetric(title: String(localized: "chat.context.metric.mcp"), value: "\(contextUsage.mcpServersTokens)")
                                        contextMetric(title: String(localized: "chat.context.metric.skills"), value: "\(contextUsage.uploadedSkillsTokens)")
                                        contextMetric(title: String(localized: "chat.context.metric.documents"), value: "\(contextUsage.uploadedDocumentsTokens)")
                                    }
                                    if let status = contextUsage.promptCacheStatus {
                                        contextMetric(
                                            title: String(localized: "chat.context.metric.prompt_cache"),
                                            value: promptCacheStatusLabel(status)
                                        )
                                    }
                                    contextMetric(
                                        title: String(localized: "chat.context.metric.cache_hit_rate"),
                                        value: contextUsage.cacheHitRateBps.map { (Double($0) / 10_000).formatted(.percent.precision(.fractionLength(1))) }
                                            ?? String(localized: "chat.context.metric.cache_unknown")
                                    )
                                    if let value = contextUsage.cacheReportedRequests {
                                        contextMetric(title: String(localized: "chat.context.metric.cache_reported"), value: value.formatted())
                                    }
                                    if let value = contextUsage.cacheUnreportedRequests, value > 0 {
                                        contextMetric(title: String(localized: "chat.context.metric.cache_unreported"), value: value.formatted())
                                    }
                                    if let value = contextUsage.cachedInputTokens {
                                        contextMetric(
                                            title: String(localized: "chat.context.metric.cache_read"),
                                            value: formattedTokenCount(value)
                                        )
                                    }
                                    if let value = contextUsage.cacheWriteInputTokens {
                                        contextMetric(
                                            title: String(localized: "chat.context.metric.cache_write"),
                                            value: formattedTokenCount(value)
                                        )
                                    }
                                    if contextUsage.hasEstimatedBreakdown {
                                        contextMetric(title: String(localized: "chat.context.metric.supplemental"), value: "\(contextUsage.supplementalTokens)")
                                        contextMetric(title: String(localized: "chat.context.metric.archived"), value: "\(contextUsage.archivedMessageCount)")
                                    }
                                }
                                .padding(.top, 8)
                            } label: {
                                Text(String(localized: "chat.context.diagnostics.header"))
                                    .font(.subheadline.weight(.medium))
                                    .foregroundStyle(DesignTokens.secondaryContentColor)
                                    .frame(minHeight: 40)
                            }
                            .disclosureGroupStyle(NolaRowDisclosureStyle(isPlain: true))
                            .accessibilityIdentifier("chat.context.diagnostics")

                            if let lastCompactedAt = contextUsage.lastCompactedAt {
                                Text("\(String(localized: "chat.context.last_compact.prefix"))\(lastCompactedAt.formatted(date: .abbreviated, time: .shortened))")
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
                        Text(String(localized: "chat.context.usage.empty"))
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                    }
                } header: {
                    Text(String(localized: "chat.context.usage.header"))
                } footer: {
                    Text(String(localized: "chat.context.usage.footer"))
                }
            }

            if displayMode == .context && selectedContextPane == .knowledge {
                Section {
                    HStack(spacing: 8) {
                        TextField(String(localized: "chat.context.search.placeholder"), text: $memorySearchQuery)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled(true)
                        Button {
                            runMemorySearch()
                        } label: {
                            ZStack {
                                Text(String(localized: "chat.context.search.action"))
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
                        Text(String(localized: "chat.context.search.empty"))
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
                                        Text("\(String(localized: "chat.context.search.score_prefix")) \(hit.score)")
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
                    Text(String(localized: "chat.context.search.header"))
                } footer: {
                    Text(String(localized: "chat.context.search.footer"))
                }
            }

            if displayMode == .context {
                if selectedContextPane == .overview {
                    Section {
                        TextEditor(text: $sessionMemory)
                            .frame(minHeight: 140)
                            .font(.body)
                            .scrollContentBackground(.hidden)
                            .background(DesignTokens.cardBackgroundColor)
                            .accessibilityIdentifier("chat-context-session-memory")
                    } header: {
                        Text(String(localized: "chat.context.session.header"))
                    } footer: {
                        Text(String(localized: "chat.context.session.footer"))
                    }
                }

                if selectedContextPane == .knowledge {
                    Section {
                        TextEditor(text: $knowledgePoints)
                            .frame(minHeight: 140)
                            .font(.body)
                            .scrollContentBackground(.hidden)
                            .background(DesignTokens.cardBackgroundColor)
                            .accessibilityIdentifier("chat-context-knowledge")
                    } header: {
                        HStack {
                            Text(String(localized: "chat.context.knowledge.header"))
                            Spacer()
                        }
                    } footer: {
                        Text(String(localized: "chat.context.knowledge.draft_footer"))
                    }

                    Section {
                        TextEditor(text: $conversationWiki)
                            .frame(minHeight: 170)
                            .font(.body)
                            .scrollContentBackground(.hidden)
                            .background(DesignTokens.cardBackgroundColor)
                            .accessibilityIdentifier("chat-context-wiki")
                    } header: {
                        Text(String(localized: "chat.context.wiki.header"))
                    } footer: {
                        Text(String(localized: "chat.context.wiki.footer"))
                    }

                    Section {
                        TextEditor(text: $longTermMemory)
                            .frame(minHeight: 130)
                            .font(.body)
                            .scrollContentBackground(.hidden)
                            .background(DesignTokens.cardBackgroundColor)
                    } header: {
                        Text(String(localized: "chat.context.long_term.header"))
                    } footer: {
                        Text(String(localized: "chat.context.long_term.footer"))
                    }

                    Section {
                        TextEditor(text: $userPreferences)
                            .frame(minHeight: 130)
                            .font(.body)
                            .scrollContentBackground(.hidden)
                            .background(DesignTokens.cardBackgroundColor)
                    } header: {
                        Text(String(localized: "chat.context.preferences.header"))
                    } footer: {
                        Text(String(localized: "chat.context.preferences.footer"))
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
                                Text(String(localized: "chat.context.memory.save"))
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .foregroundStyle(DesignTokens.onActionColor)
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
                                Text(String(localized: "chat.context.compact.run"))
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        .disabled(isSavingMemoryProfile || isCompacting)

                        Button(String(localized: "chat.context.reload")) {
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
                        Text(String(localized: "chat.context.compact.header"))
                    } footer: {
                        Text(String(localized: "chat.context.compact.footer"))
                    }
                }
            }

            if displayMode == .context && selectedContextPane == .tools {
                Section {
                    if !hostMCPServers.isEmpty {
                        hostMCPServersList(items: hostMCPServers)
                    } else if !mcpServers.isEmpty {
                        localMCPServersReadOnlyList(items: mcpServers)
                    } else {
                        Text(String(localized: "chat.context.mcp.empty"))
                            .font(.subheadline)
                            .foregroundColor(.secondary)
                    }
                } header: {
                    Text(String(localized: "chat.context.mcp.header"))
                } footer: {
                    if !hostMCPServers.isEmpty {
                        Text(String(localized: "chat.context.mcp.host.footer"))
                    } else {
                        Text(String(localized: "chat.context.mcp.local.footer"))
                    }
                }
            }

            if displayMode == .skills {
                Section {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(String(localized: "chat.context.skills.library"))
                            .font(.title3.weight(.semibold))
                        Text(String(localized: "chat.context.skills.library.footer"))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }

                    HStack(spacing: 10) {
                        Label(
                            "\(uploadedSkills.filter(\.enabled).count + hostUploadedSkills.filter(\.enabled).count) \(String(localized: "chat.context.skills.enabled_count"))",
                            systemImage: "checkmark.circle.fill"
                        )
                        Spacer()
                        Text("\(uploadedSkills.count + hostUploadedSkills.count) \(String(localized: "chat.context.skills.total_count"))")
                    }
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                } header: {
                    Text(String(localized: "chat.context.skills.header"))
                }

                skillGallery(items: $uploadedSkills, hostItems: hostUploadedSkills)

                Section {
                    Button {
                        uploadedSkills.append(UserUploadedContextItem(
                            name: String(localized: "chat.context.skills.new_skill_name"),
                            content: String(localized: "chat.context.skills.template.body"),
                            description: String(localized: "chat.context.skills.new_skill_description"),
                            enabled: true
                        ))
                        selectedSkillID = uploadedSkills.last?.id
                        noticeMessage = String(localized: "chat.context.skills.template_created")
                    } label: {
                        Label(String(localized: "chat.context.skills.create_template"), systemImage: "plus")
                    }
                    Button {
                        skillImportOwner = ContextFileImportOwner(conversationID: conversationID)
                        showSkillImporter = true
                    } label: {
                        Label(String(localized: "chat.context.skills.upload_file"), systemImage: "square.and.arrow.down")
                    }
                } footer: {
                    Text(String(localized: "chat.context.skills.footer"))
                }
            }

            if displayMode == .context && selectedContextPane == .tools {
                Section {
                    if documentsReadable {
                        uploadedItemsList(
                            items: $uploadedDocuments,
                            emptyText: hostUploadedDocuments.isEmpty ? String(localized: "chat.context.documents.empty") : ""
                        )
                    } else {
                        Text("chat.context.documents.read_error").foregroundStyle(.secondary)
                    }
                    hostDocumentsList(items: hostUploadedDocuments)
                    Button {
                        documentImportOwner = ContextFileImportOwner(conversationID: conversationID)
                        showDocumentImporter = true
                    } label: {
                        Label(String(localized: "chat.context.documents.upload"), systemImage: "doc.badge.plus")
                    }
                    .disabled(!documentsReadable)
                } header: {
                    Text(String(localized: "chat.context.documents.header"))
                } footer: {
                    Text(String(localized: "chat.context.documents.footer"))
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
            .listRowBackground(DesignTokens.surfaceColor)
            .listRowSeparatorTint(DesignTokens.borderColor)
        }
        .nolaGroupedSurface()
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
            if !embeddedInNavigationStack {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.cancel) {
                        onDiscard()
                        dismiss()
                    }
                    .disabled(isSavingMemoryProfile || isCompacting)
                    .accessibilityIdentifier("chat.context.cancel")
                }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button(L10n.commonSave) {
                    runSaveAndDismiss()
                }
                .disabled(isSavingMemoryProfile || isCompacting)
            }
        }
        .disabled(isSavingMemoryProfile || isCompacting)
        .interactiveDismissDisabled(isSavingMemoryProfile || isCompacting || hasUnsavedChanges)
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
            String(localized: "chat.context.skills.delete_title"),
            isPresented: Binding(
                get: { pendingSkillDeletionID != nil },
                set: { if !$0 { pendingSkillDeletionID = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(String(localized: "chat.context.delete"), role: .destructive) {
                deletePendingSkill()
            }
            Button(L10n.cancel, role: .cancel) {
                pendingSkillDeletionID = nil
            }
        } message: {
            Text(String(localized: "chat.context.skills.delete_message"))
        }
        .confirmationDialog(
            String(localized: "chat.context.mcp.delete_title"),
            isPresented: Binding(
                get: { pendingMCPDeletionID != nil },
                set: { if !$0 { pendingMCPDeletionID = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(String(localized: "chat.context.delete"), role: .destructive) {
                deletePendingMCPServer()
            }
            Button(L10n.cancel, role: .cancel) {
                pendingMCPDeletionID = nil
            }
        } message: {
            Text(String(localized: "chat.context.mcp.delete_message"))
        }
    }

    private func contextPaneTitle(_ pane: ContextPane) -> String {
        switch pane {
        case .overview:
            return String(localized: "chat.context.pane.overview")
        case .knowledge:
            return String(localized: "chat.context.pane.knowledge")
        case .tools:
            return String(localized: "chat.context.pane.tools")
        }
    }

    private var sheetTitle: String {
        switch displayMode {
        case .context:
            return NSLocalizedString("chat.context.title", value: "上下文", comment: "Chat context title")
        case .skills:
            return String(localized: "chat.context.skills.header")
        }
    }

    private var sheetSubtitle: String {
        switch displayMode {
        case .context:
            return NSLocalizedString("chat.context.sheet.subtitle", value: "查看上下文占用、整理 session memory，并维护跨对话生效的长期记忆与用户偏好。", comment: "Chat context sheet subtitle")
        case .skills:
            return String(localized: "chat.context.skills.sheet_subtitle")
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
                    if let onSaveUploadedDocuments, let initialDocuments, initialDocuments != uploadedDocuments {
                        uploadedDocuments = try onSaveUploadedDocuments(uploadedDocuments)
                        self.initialDocuments = uploadedDocuments
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
        case "disabled": return String(localized: "chat.context.cache.disabled")
        case "invalid_identity": return String(localized: "chat.context.cache.invalid")
        case "first_request": return String(localized: "chat.context.cache.first_request")
        case "hit": return String(localized: "chat.context.cache.hit")
        case "write": return String(localized: "chat.context.cache.write")
        case "miss": return String(localized: "chat.context.cache.miss")
        case "stable_candidate": return String(localized: "chat.context.cache.candidate")
        default: return String(localized: "chat.context.cache.unknown")
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
                    Text("~\(suggestion.estimatedSavingsTokens) \(String(localized: "chat.context.tokens"))")
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
                    Toggle(String(localized: "chat.context.toggle.enabled"), isOn: $item.enabled)
                    TextField(String(localized: "chat.context.field.name"), text: $item.name)
                    Text("\(item.content.count) \(String(localized: "chat.context.unit.characters"))")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    TextEditor(text: $item.content)
                        .frame(minHeight: 90)
                        .font(.caption)
                        .scrollContentBackground(.hidden)
                        .background(DesignTokens.cardBackgroundColor)
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
                    String(localized: "chat.context.skills.empty_title"),
                    systemImage: "shippingbox",
                    description: Text(String(localized: "chat.context.skills.empty"))
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
                    .accessibilityLabel(String(localized: "chat.context.toggle.enabled"))
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
                        Text(String(localized: "chat.context.skills.local"))
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
                    Text(String(localized: "chat.context.skills.host"))
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
            Group {
            Section {
                HStack(spacing: 12) {
                    skillCategoryIcon(category)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(skillCategoryTitle(category))
                            .font(.subheadline.weight(.semibold))
                        Text(String(localized: "chat.context.skills.local_editable"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Toggle(String(localized: "chat.context.toggle.enabled"), isOn: item.enabled)
                        .labelsHidden()
                }
            }
            Section {
                GitHubSkillSourceLink(reference: item.wrappedValue.packageReference)
                TextField(String(localized: "chat.context.skills.name"), text: item.name)
                TextField(String(localized: "chat.context.skills.description"), text: item.description, axis: .vertical)
                    .lineLimit(2...5)
            } header: {
                Text(String(localized: "chat.context.skills.identity"))
            }
            Section {
                TextEditor(text: item.content)
                    .frame(minHeight: 260)
                    .font(.system(.caption, design: .monospaced))
                    .scrollContentBackground(.hidden)
                    .background(DesignTokens.cardBackgroundColor)
            } header: {
                HStack {
                    Text(String(localized: "chat.context.skills.instructions"))
                    Spacer()
                    Text("\(item.wrappedValue.content.count) \(String(localized: "chat.context.unit.characters"))")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Section {
                Button {
                    onUseSkill(skillInvocation(for: item.wrappedValue))
                } label: {
                    Label(String(localized: "chat.context.skills.use"), systemImage: "arrow.turn.down.left")
                }
                Button {
                    let invocation = skillInvocation(for: item.wrappedValue)
                    UIPasteboard.general.string = invocation
                    noticeMessage = String(localized: "chat.context.skills.copied")
                } label: {
                    Label(String(localized: "chat.context.copy"), systemImage: "doc.on.doc")
                }
                Button(role: .destructive) {
                    pendingSkillDeletionID = item.wrappedValue.id
                } label: {
                    Label(String(localized: "chat.context.delete"), systemImage: "trash")
                }
            }
            }
            .listRowBackground(DesignTokens.surfaceColor)
            .listRowSeparatorTint(DesignTokens.borderColor)
        }
        .nolaGroupedSurface()
        .navigationTitle(skillDisplayName(item.wrappedValue))
        .navigationBarTitleDisplayMode(.inline)
    }

    private func hostSkillDetail(
        item: UserUploadedContextItem,
        category: SkillVisualCategory
    ) -> some View {
        List {
            Group {
            Section {
                HStack(spacing: 12) {
                    skillCategoryIcon(category)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(skillDisplayName(item))
                            .font(.headline)
                        Text(String(localized: "chat.context.skills.host_managed"))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                GitHubSkillSourceLink(reference: item.packageReference)
            }
            if !item.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Section(String(localized: "chat.context.skills.purpose")) {
                    Text(item.description)
                }
            }
            Section(String(localized: "chat.context.skills.instructions")) {
                Text(item.content)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
            }
            Section {
                Button {
                    onUseSkill(skillInvocation(for: item))
                } label: {
                    Label(String(localized: "chat.context.skills.use"), systemImage: "arrow.turn.down.left")
                }
            }
            }
            .listRowBackground(DesignTokens.surfaceColor)
            .listRowSeparatorTint(DesignTokens.borderColor)
        }
        .nolaGroupedSurface()
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
        case .documents: return String(localized: "chat.context.skills.category.documents")
        case .data: return String(localized: "chat.context.skills.category.data")
        case .design: return String(localized: "chat.context.skills.category.design")
        case .communication: return String(localized: "chat.context.skills.category.communication")
        case .automation: return String(localized: "chat.context.skills.category.automation")
        case .research: return String(localized: "chat.context.skills.category.research")
        case .workspace: return String(localized: "chat.context.skills.category.workspace")
        case .custom: return String(localized: "chat.context.skills.category.custom")
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
                                Text(server.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? String(localized: "chat.context.mcp.default_name") : server.name)
                                    .font(.subheadline.weight(.semibold))
                                    .foregroundStyle(.primary)
                                Text(server.endpoint.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? String(localized: "chat.context.mcp.endpoint_required") : server.endpoint)
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
                        .accessibilityLabel(String(localized: "chat.context.toggle.enabled"))
                }

                if selectedMCPServerID == server.id {
                    Divider()
                    TextField(String(localized: "chat.context.field.name"), text: $server.name)
                    TextField("https://example.com/mcp", text: $server.endpoint)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                    TextField(String(localized: "chat.context.field.description_optional"), text: $server.description, axis: .vertical)
                        .lineLimit(1...3)

                    HStack {
                        Spacer()
                        Button(role: .destructive) {
                            pendingMCPDeletionID = server.id
                        } label: {
                            Label(String(localized: "chat.context.delete"), systemImage: "trash")
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
            ? String(localized: "chat.context.skills.untitled")
            : item.name
    }

    private func skillSummary(_ item: UserUploadedContextItem) -> String {
        let description = item.description.trimmingCharacters(in: .whitespacesAndNewlines)
        if !description.isEmpty {
            return description
        }
        return item.enabled
            ? String(localized: "chat.context.skills.enabled_desc")
            : String(localized: "chat.context.skills.disabled_desc")
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
                    Text(String(localized: "chat.context.skills.host.header"))
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    ForEach(items) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Text(item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? String(localized: "chat.host_skill.default_name") : item.name)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(item.enabled
                                     ? String(localized: "chat.context.skills.host.enabled")
                                     : String(localized: "chat.context.skills.host.disabled"))
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
                                Label("\(item.content.count) \(String(localized: "chat.context.unit.characters"))", systemImage: "desktopcomputer")
                                Spacer()
                                Button {
                                    let invocation = skillInvocation(for: item)
                                    onUseSkill(invocation)
                                } label: {
                                    Text(String(localized: "chat.context.use"))
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
                    Text(String(localized: "chat.context.documents.host.header"))
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    ForEach(items) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack {
                                Text(item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? String(localized: "chat.host_document.default_name") : item.name)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(item.enabled
                                     ? String(localized: "chat.context.documents.host.enabled")
                                     : String(localized: "chat.context.documents.host.disabled"))
                                    .font(.caption2.weight(.semibold))
                                    .foregroundColor(item.enabled ? DesignTokens.successColor : .secondary)
                            }
                            if !item.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                Text(item.description)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Label("\(item.content.count) \(String(localized: "chat.context.unit.characters"))", systemImage: "doc.text")
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

    private func localMCPServersReadOnlyList(items: [UserMCPServer]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(items) { server in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(server.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                             ? String(localized: "chat.context.mcp.default_name")
                             : server.name)
                            .font(.subheadline.weight(.semibold))
                        Spacer()
                        Text(server.enabled
                             ? String(localized: "chat.context.toggle.enabled")
                             : String(localized: "chat.context.sources.disabled"))
                            .font(.caption2.weight(.semibold))
                            .foregroundColor(server.enabled ? DesignTokens.successColor : .secondary)
                    }
                    if !server.endpoint.isEmpty {
                        Text(server.endpoint)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    if !server.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Text(server.description)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(10)
                .background(DesignTokens.primaryColor.opacity(0.06), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
        }
        .padding(.vertical, 4)
    }

    private func hostMCPServersList(items: [UserMCPServer]) -> some View {
        Group {
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "chat.context.mcp.host.header"))
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.secondary)
                    ForEach(items) { server in
                        VStack(alignment: .leading, spacing: 6) {
                            HStack(alignment: .firstTextBaseline) {
                                Text(server.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? String(localized: "chat.host_mcp.default_name") : server.name)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                                Text(server.availability == "available"
                                     ? String(localized: "chat.context.mcp.host.connected")
                                     : String(localized: "chat.context.mcp.host.unavailable"))
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
                                    format: String(localized: "chat.context.mcp.host.catalog_count"),
                                    catalogCount
                                )
                            ) {
                                hostMCPNames(String(localized: "chat.context.mcp.host.tools"), names: server.tools ?? [])
                                hostMCPNames(String(localized: "chat.context.mcp.host.resources"), names: server.resources ?? [])
                                hostMCPNames(String(localized: "chat.context.mcp.host.templates"), names: server.resourceTemplates ?? [])
                                hostMCPNames(String(localized: "chat.context.mcp.host.prompts"), names: server.prompts ?? [])
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
        let name = item.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? String(localized: "chat.context.skills.invocation.default_name") : item.name
        return String(format: String(localized: "chat.context.skills.invocation.format"), name)
    }

    private func importUploadedFiles(_ result: Result<[URL], Error>, into items: Binding<[UserUploadedContextItem]>) {
        do {
            let urls = try result.get()
            let imported = IOSContextFileImportPolicy.importFiles(urls, existing: items.wrappedValue)
            items.wrappedValue = imported.items
            if !imported.problems.isEmpty {
                errorMessage = imported.problems.joined(separator: "\n")
            }
        } catch {
            if !IOSContextFileImportPolicy.isCancellation(error) { errorMessage = error.localizedDescription }
        }
    }


}

struct GitHubSkillSourceLink: View {
    let reference: String?

    var body: some View {
        if let source = IOSGitHubSkillPackages.sourceURL(reference: reference) {
            Link(destination: source) {
                Label("GitHub · " + source.pathComponents.dropFirst().prefix(2).joined(separator: "/"), systemImage: "link")
            }
        }
    }
}
