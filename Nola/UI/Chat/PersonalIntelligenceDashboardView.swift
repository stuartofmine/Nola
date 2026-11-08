// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UniformTypeIdentifiers

struct PersonalIntelligenceDashboardView: View {
    var showsCloseButton = false
    var embedded = false
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @EnvironmentObject private var container: ServiceContainer
    @State private var snapshot: [String: AgentCoreJSONValue] = [:]
    @State private var contextProfile: ChatMemoryProfile = .empty
    @State private var selectedFilter: MemoryFilter = .all
    @State private var visibleMemoryCount = 24
    @State private var showClearConfirmation = false
    @State private var hasAppeared = false
    @State private var loadError: String?

    private let accent = DesignTokens.primaryColor
    private let secondaryAccent = DesignTokens.primaryColor

    private func text(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(
            key,
            value: fallback,
            comment: "Personal intelligence dashboard"
        )
    }

    private var eventObjects: [[String: AgentCoreJSONValue]] {
        (snapshot["events"]?.jsonArray ?? []).compactMap(\.jsonObject)
    }
    private var memories: [[String: AgentCoreJSONValue]] {
        (snapshot["memories"]?.jsonArray ?? []).compactMap(\.jsonObject)
    }
    private var activeMemories: [[String: AgentCoreJSONValue]] {
        memories.filter { $0["status"]?.jsonString == "active" }
    }
    private var filteredMemories: [[String: AgentCoreJSONValue]] {
        guard let kind = selectedFilter.kind else { return activeMemories }
        return activeMemories.filter { $0["kind"]?.jsonString == kind }
    }
    private var strategies: [[String: AgentCoreJSONValue]] {
        (snapshot["strategies"]?.jsonArray ?? []).compactMap(\.jsonObject)
    }
    private var sourceNames: [String] {
        Array(Set(eventObjects.compactMap { $0["source"]?.jsonString })).sorted()
    }
    private var averageConfidence: Int {
        guard !activeMemories.isEmpty else { return 0 }
        let total = activeMemories.reduce(0.0) { $0 + ($1["confidence"]?.jsonNumber ?? 0) }
        return Int((total / Double(activeMemories.count)) * 100)
    }

    var body: some View {
        NolaPage {
            if let loadError {
                VStack(alignment: .leading, spacing: 8) {
                    Text(loadError).foregroundStyle(.red)
                        .accessibilityIdentifier("intelligence.loadError")
                    Button("common.retry") { reload() }
                }
            }
            if !embedded {
                intelligenceHeader

                hero
                    .opacity(hasAppeared ? 1 : 0)
                    .offset(y: hasAppeared ? 0 : 10)

                capabilitiesGateway
                    .opacity(hasAppeared ? 1 : 0)
                    .offset(y: hasAppeared ? 0 : 12)

            }

            memorySection
                .opacity(hasAppeared ? 1 : 0)
                .offset(y: hasAppeared ? 0 : 14)

            if embedded {
                capabilitiesGateway
            }

            if !strategies.isEmpty {
                executionSection
                    .opacity(hasAppeared ? 1 : 0)
                    .offset(y: hasAppeared ? 0 : 16)
            }

            howItWorks
                .opacity(hasAppeared ? 1 : 0)

            clearSection
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("screen.intelligence")
        .refreshable { reload() }
        .navigationTitle(embedded ? text("information.memory", "Long-term memory") : (showsCloseButton ? text("personal.intelligence.title", "Personal intelligence") : ""))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(showsCloseButton || embedded ? .visible : .hidden, for: .navigationBar)
        .toolbar {
            if showsCloseButton {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.close) { dismiss() }
                }
            }
        }
        .task {
            reload()
            if reduceMotion {
                hasAppeared = true
            } else {
                withAnimation(DesignTokens.revealAnimation) { hasAppeared = true }
            }
        }
        .confirmationDialog(
            text("personal.intelligence.clear.confirm.title", "Clear all learned data?"),
            isPresented: $showClearConfirmation,
            titleVisibility: .visible
        ) {
            Button(text("common.clear", "Clear"), role: .destructive) {
                if PersonalIntelligenceCore.clear() {
                    reload()
                } else {
                    loadError = text("personal.intelligence.clear.failed", "Could not clear learned data. Retry when local storage is ready.")
                }
            }
        } message: {
            Text(text("personal.intelligence.clear.confirm.message", "Chats stay intact, but observations, memories, and route learning cannot be restored."))
        }
    }

    private var intelligenceHeader: some View {
        HStack(alignment: .center, spacing: DesignTokens.spacing_sm) {
            NolaPageHeader(
                icon: "brain.head.profile",
                title: text("personal.intelligence.title", "Personal intelligence"),
                subtitle: text(
                    "personal.intelligence.page.subtitle",
                    "Long-term memory, reusable capabilities, and learning evidence."
                ),
                tint: accent
            )
            Spacer(minLength: 0)
            Button(action: reload) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 15, weight: .semibold))
                    .frame(width: 38, height: 38)
                    .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            }
            .buttonStyle(NolaPressableStyle())
            .accessibilityLabel(text("common.refresh", "Refresh"))
        }
    }

    private var hero: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 14) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(text("personal.intelligence.hero.title", "Your Nola is forming its own understanding"))
                        .font(.title3.weight(.bold))
                        .tracking(-0.35)
                    Text(text("personal.intelligence.hero.subtitle", "It turns conversations, files, and changing data into durable memory, then adapts from real outcomes."))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 9) {
                    metricCard(text("personal.intelligence.metric.observed", "Observed"), value: eventObjects.count, icon: "waveform.path.ecg")
                    metricCard(text("personal.intelligence.metric.memories", "Memories"), value: activeMemories.count, icon: "brain")
                    metricCard(text("personal.intelligence.metric.confidence", "Confidence"), value: averageConfidence, suffix: "%", icon: "checkmark.seal")
                }
                VStack(spacing: 9) {
                    metricCard(text("personal.intelligence.metric.observed", "Observed"), value: eventObjects.count, icon: "waveform.path.ecg")
                    metricCard(text("personal.intelligence.metric.memories", "Memories"), value: activeMemories.count, icon: "brain")
                    metricCard(text("personal.intelligence.metric.confidence", "Confidence"), value: averageConfidence, suffix: "%", icon: "checkmark.seal")
                }
            }

            if !sourceNames.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text(text("personal.intelligence.sources.title", "Learning from these sources"))
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 7) {
                            ForEach(sourceNames, id: \.self) { source in
                                Label(sourceLabel(source), systemImage: sourceIcon(source))
                                    .font(.caption.weight(.medium))
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 6)
                                    .background(Color.primary.opacity(0.055), in: Capsule())
                            }
                        }
                    }
                }
            }
        }
        .padding(.vertical, DesignTokens.spacing_sm)
    }

    private var capabilitiesGateway: some View {
        NavigationLink {
            PersonalCapabilitiesLibraryView()
        } label: {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 14) {
                    capabilityGatewayIcon
                    capabilityGatewayCopy
                    Spacer(minLength: 8)
                    capabilityGatewaySummary
                    capabilityGatewayChevron
                }

                VStack(alignment: .leading, spacing: 13) {
                    HStack(spacing: 12) {
                        capabilityGatewayIcon
                        capabilityGatewayCopy
                        Spacer(minLength: 4)
                        capabilityGatewayChevron
                    }
                    capabilityGatewaySummary
                }
            }
            .padding(16)
            .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(DesignTokens.borderColor, lineWidth: 1)
            }
        }
        .buttonStyle(NolaActionButtonStyle())
        .accessibilityIdentifier("intelligence.capabilities.open")
    }

    private var capabilityGatewayIcon: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(secondaryAccent.opacity(0.11))
            Image(systemName: "shippingbox.and.arrow.backward")
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(secondaryAccent)
        }
        .frame(width: 46, height: 46)
    }

    private var capabilityGatewayCopy: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(piText("personal.capabilities.title", "Skills & MCP"))
                .font(.headline)
                .foregroundStyle(.primary)
            Text(piText("personal.capabilities.gateway.detail", "Manage reusable workflows and external tool connections."))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .layoutPriority(1)
    }

    private var capabilityGatewaySummary: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(
                String(
                    format: piText("personal.capabilities.gateway.summary", "%d Skills · %d MCP"),
                    contextProfile.uploadedSkills.filter(\.enabled).count,
                    contextProfile.mcpServers.filter(\.enabled).count
                )
            )
            .font(.subheadline.monospacedDigit().weight(.semibold))
            .foregroundStyle(secondaryAccent)
            Text(piText("personal.capabilities.gateway.count", "enabled"))
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
    }

    private var capabilityGatewayChevron: some View {
        Image(systemName: "chevron.right")
            .font(.caption.weight(.semibold))
            .foregroundStyle(.tertiary)
            .frame(width: 20, height: 44)
    }

    private func metricCard(_ title: String, value: Int, suffix: String = "", icon: String) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Image(systemName: icon)
                .font(.caption.weight(.semibold))
                .foregroundStyle(accent)
            Text("\(value.formatted())\(suffix)")
                .font(.title3.monospacedDigit().weight(.bold))
                .contentTransition(.numericText())
            Text(title)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)

    }

    private var memorySection: some View {
        VStack(alignment: .leading, spacing: 13) {
            sectionHeader(
                text("personal.intelligence.memories.title", "Managed memories"),
                subtitle: text("personal.intelligence.memories.subtitle", "Only active understanding backed by evidence")
            )

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(MemoryFilter.allCases) { filter in
                        filterButton(filter)
                    }
                }
            }

            if filteredMemories.isEmpty && loadError == nil {
                emptyMemoryState
            } else {
                ForEach(Array(filteredMemories.prefix(visibleMemoryCount).enumerated()), id: \.offset) { index, memory in
                    memoryCard(memory)
                        .transition(.opacity.combined(with: .move(edge: .bottom)))
                        .animation(
                            reduceMotion
                                ? nil
                                : DesignTokens.revealAnimation,
                            value: selectedFilter
                        )
                }
                if filteredMemories.count > visibleMemoryCount {
                    Button(text("personal.intelligence.memories.load_more", "Load more memories")) {
                        visibleMemoryCount = min(visibleMemoryCount + 24, filteredMemories.count)
                    }.frame(minHeight: 44)
                }
            }
        }
    }

    private func filterButton(_ filter: MemoryFilter) -> some View {
        let selected = selectedFilter == filter
        return Button {
            visibleMemoryCount = 24
            if reduceMotion {
                selectedFilter = filter
            } else {
                withAnimation(DesignTokens.disclosureAnimation) { selectedFilter = filter }
            }
        } label: {
            Text(filterTitle(filter))
                .font(.subheadline.weight(.medium))
                .foregroundStyle(Color.primary)
                .padding(.horizontal, 13)
                .padding(.vertical, 8)
                .frame(minHeight: 44)
                .background(selected ? DesignTokens.accentSubtleColor : .clear, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(DesignTokens.borderColor, lineWidth: 1))
        }
        .buttonStyle(NolaActionButtonStyle())
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("intelligence.filter.\(filter.rawValue)")
    }

    private var emptyMemoryState: some View {
        VStack(spacing: 12) {
            Image(systemName: selectedFilter == .all ? "sparkles" : "line.3.horizontal.decrease.circle")
                .font(.system(size: 24, weight: .medium))
                .foregroundStyle(accent)
            Text(selectedFilter == .all
                 ? text("personal.intelligence.memories.empty.all", "No durable memories yet")
                 : text("personal.intelligence.memories.empty.category", "Nothing in this category yet"))
                .font(.headline)
            Text(text("personal.intelligence.memories.empty.hint", "Keep using Nola naturally. It organizes what matters without asking you to maintain another library."))
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
        .padding(.vertical, 30)

    }

    private func memoryCard(_ memory: [String: AgentCoreJSONValue]) -> some View {
        let rawKind = memory["kind"]?.jsonString
        let confidence = min(max(memory["confidence"]?.jsonNumber ?? 0, 0), 1)
        let evidenceCount = memory["source_event_ids"]?.jsonArray?.count ?? 0
        let color = memoryColor(rawKind)

        return VStack(alignment: .leading, spacing: 13) {
            HStack(spacing: 9) {
                Label(memoryKind(rawKind), systemImage: memoryIcon(rawKind))
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(color)
                    .padding(.horizontal, 9)
                    .padding(.vertical, 5)
                    .background(color.opacity(0.10), in: Capsule())
                Spacer()
                Label(String(format: text("personal.intelligence.evidence.format", "%d evidence"), evidenceCount), systemImage: "link")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Text(memory["statement"]?.jsonString ?? "")
                .font(.body)
                .foregroundStyle(.primary)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)

            VStack(spacing: 6) {
                HStack {
                    Text(text("personal.intelligence.confidence", "Confidence"))
                    Spacer()
                    Text("\(Int(confidence * 100))%")
                        .monospacedDigit()
                        .contentTransition(.numericText())
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                ProgressView(value: confidence)
                    .tint(color)
                    .scaleEffect(y: 0.65)
            }
        }
        .padding(16)
        .background(DesignTokens.surfaceColor, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.primary.opacity(0.055), lineWidth: 1)
        }
    }

    private var executionSection: some View {
        VStack(alignment: .leading, spacing: 13) {
            sectionHeader(
                text("personal.intelligence.routes.title", "Learning how to work better"),
                subtitle: text("personal.intelligence.routes.subtitle", "Adapting model and route choices from real outcomes")
            )

            ForEach(Array(strategies.sorted(by: strategySort).prefix(10).enumerated()), id: \.offset) { _, strategy in
                strategyCard(strategy)
            }
        }
    }

    private func strategyCard(_ strategy: [String: AgentCoreJSONValue]) -> some View {
        let score = min(max(strategy["score"]?.jsonNumber ?? 0, 0), 1)
        let model = strategy["model"]?.jsonString
        let route = strategy["route"]?.jsonString ?? "—"

        return HStack(spacing: 14) {
            ZStack {
                Circle().fill(secondaryAccent.opacity(0.10))
                Image(systemName: "point.3.connected.trianglepath.dotted")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(secondaryAccent)
            }
            .frame(width: 40, height: 40)

            VStack(alignment: .leading, spacing: 4) {
                Text(model?.isEmpty == false ? model! : route)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Text("\(strategy["task_type"]?.jsonString ?? text("personal.intelligence.source.conversations", "Conversations")) · \(Int(strategy["attempts"]?.jsonNumber ?? 0)) \(text("personal.intelligence.outcomes", "outcomes"))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 3) {
                Text("\(Int(score * 100))")
                    .font(.title3.monospacedDigit().weight(.bold))
                    .foregroundStyle(secondaryAccent)
                Text(text("personal.intelligence.fit", "fit"))
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(15)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 19, style: .continuous))
    }

    private var howItWorks: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionHeader(
                text("personal.intelligence.explanation.title", "How understanding forms"),
                subtitle: text("personal.intelligence.explanation.subtitle", "Not a bookmark folder, and no manual filing")
            )
            explanationRow("waveform.path", text("personal.intelligence.explanation.observe.title", "Observe change"), text("personal.intelligence.explanation.observe.detail", "Notice meaningful changes across conversations, files, and data sources."))
            explanationRow("link", text("personal.intelligence.explanation.evidence.title", "Keep evidence"), text("personal.intelligence.explanation.evidence.detail", "Every memory keeps its sources and can evolve or be replaced by newer evidence."))
            explanationRow("arrow.triangle.branch", text("personal.intelligence.explanation.outcomes.title", "Learn from outcomes"), text("personal.intelligence.explanation.outcomes.detail", "Success, failure, and correction shape future model and route choices."))
        }
        .padding(18)

    }

    private func explanationRow(_ icon: String, _ title: String, _ detail: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(accent)
                .frame(width: 28, height: 28)
                .background(accent.opacity(0.10), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var clearSection: some View {
        Button(role: .destructive) { showClearConfirmation = true } label: {
            Text(text("personal.intelligence.clear.action", "Clear personal intelligence"))
                .font(.footnote.weight(.medium))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 10)
        }
        .buttonStyle(NolaActionButtonStyle())
        .foregroundStyle(.red)
        .disabled(snapshot.isEmpty)
        .opacity(snapshot.isEmpty ? 0.35 : 0.8)
    }

    private func sectionHeader(_ title: String, subtitle: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.title3.weight(.bold))
                .tracking(-0.2)
            Text(subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func filterTitle(_ filter: MemoryFilter) -> String {
        switch filter {
        case .all: return text("personal.intelligence.filter.all", "All")
        case .facts: return text("personal.intelligence.filter.facts", "Facts")
        case .preferences: return text("personal.intelligence.filter.preferences", "Preferences")
        case .episodes: return text("personal.intelligence.filter.episodes", "Episodes")
        }
    }

    private func memoryKind(_ raw: String?) -> String {
        switch raw {
        case "procedural": return text("personal.intelligence.memory.preference", "Preference")
        case "episodic": return text("personal.intelligence.memory.episode", "Episode")
        default: return text("personal.intelligence.memory.fact", "Fact")
        }
    }

    private func memoryIcon(_ raw: String?) -> String {
        switch raw {
        case "procedural": return "slider.horizontal.3"
        case "episodic": return "clock.arrow.circlepath"
        default: return "checkmark.seal"
        }
    }

    private func memoryColor(_ raw: String?) -> Color {
        switch raw {
        case "procedural": return Color(red: 0.88, green: 0.48, blue: 0.12)
        case "episodic": return secondaryAccent
        default: return accent
        }
    }

    private func sourceLabel(_ source: String) -> String {
        switch source {
        case "conversation": return text("personal.intelligence.source.conversations", "Conversations")
        case "health": return text("personal.intelligence.source.health", "Health")
        case "attachment": return text("personal.intelligence.source.files", "Files")
        default: return source.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    private func sourceIcon(_ source: String) -> String {
        switch source {
        case "conversation": return "bubble.left.and.bubble.right"
        case "health": return "heart.text.square"
        case "attachment": return "doc.text"
        default: return "circle.hexagongrid"
        }
    }

    private func strategySort(_ lhs: [String: AgentCoreJSONValue], _ rhs: [String: AgentCoreJSONValue]) -> Bool {
        (lhs["score"]?.jsonNumber ?? 0) > (rhs["score"]?.jsonNumber ?? 0)
    }

    private func reload() {
        contextProfile = container.chatMemoryProfileStore.loadProfile()
        guard let updated = PersonalIntelligenceCore.snapshot() else {
            loadError = text("personal.intelligence.load.failed", "Could not load learned data. Previous results are retained. Try refreshing.")
            return
        }
        loadError = nil
        if reduceMotion {
            snapshot = updated
        } else {
            withAnimation(.smooth(duration: 0.3)) {
                snapshot = updated
            }
        }
    }
}

private enum PersonalCapabilityKind: String, CaseIterable, Identifiable {
    case skills
    case mcp

    var id: String { rawValue }
}

private enum PersonalCapabilityEditorItem: Identifiable {
    case skill(Int?, UserUploadedContextItem)
    case mcp(Int?, UserMCPServer)

    var id: String {
        switch self {
        case .skill(let index, let item): return "skill-\(index.map(String.init) ?? "new")-\(item.id)"
        case .mcp(let index, let item): return "mcp-\(index.map(String.init) ?? "new")-\(item.id)"
        }
    }
}

private enum PersonalCapabilityDeleteTarget: Identifiable {
    case skill(IOSCanonicalSkillEditor, String)
    case mcp(IOSCanonicalMCPServer, String)

    var id: String {
        switch self {
        case .skill(let snapshot, _): return "skill-\(snapshot.record.id)"
        case .mcp(let snapshot, _): return "mcp-\(snapshot.id)"
        }
    }

    var name: String {
        switch self {
        case .skill(_, let name), .mcp(_, let name): return name
        }
    }
}

struct PersonalCapabilitiesLibraryView: View {
    var hostSkills: [UserUploadedContextItem] = []
    var onUseSkill: ((String) -> Void)? = nil
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @EnvironmentObject private var container: ServiceContainer
    @State private var profile: ChatCapabilityLibrarySnapshot = .empty
    @State private var libraryLoaded = false
    @State private var libraryReadFailed = false
    @State private var selectedKind: PersonalCapabilityKind = .skills
    @State private var query = ""
    @State private var editor: PersonalCapabilityEditorItem?
    @State private var skillEditorSnapshot: IOSCanonicalSkillEditor?
    @State private var mcpEditorSnapshot: IOSCanonicalMCPServer?
    @State private var showSkillImporter = false
    @State private var deleteTarget: PersonalCapabilityDeleteTarget?
    @State private var notice: String?
    @State private var errorMessage: String?

    private let columns = [GridItem(.adaptive(minimum: 250), spacing: 12)]

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 5) {
                    Group {
                        if selectedKind == .mcp { Text("mcp.mobile.http_execution") }
                        else { Text(piText("personal.capabilities.subtitle", "Reusable workflows and connected tools available across conversations.")) }
                    }
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                Picker(piText("personal.capabilities.type", "Capability type"), selection: $selectedKind) {
                    Text("\(piText("personal.capabilities.skills", "Skills")) \(profile.uploadedSkills.filter(\.enabled).count)/\(profile.uploadedSkills.count)")
                        .tag(PersonalCapabilityKind.skills)
                    Text("MCP \(profile.mcpServers.filter(\.enabled).count)/\(profile.mcpServers.count)")
                        .tag(PersonalCapabilityKind.mcp)
                }
                .pickerStyle(.segmented)

                if horizontalSizeClass == .compact {
                    VStack(spacing: 10) {
                        capabilitySearchField
                        addCapabilityButton
                    }
                } else {
                    HStack(spacing: 10) {
                        capabilitySearchField
                        addCapabilityButton
                    }
                }

                if let notice {
                    Label(notice, systemImage: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                        .transition(.opacity)
                }

                if libraryReadFailed {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(piText("personal.capabilities.load_failed", "Could not load capabilities. Previous results are retained; reload before editing."))
                            .foregroundStyle(.red)
                        Button(piText("personal.capabilities.retry", "Retry")) { reload() }
                            .accessibilityIdentifier("intelligence.capabilities.retry")
                    }
                }
                if libraryLoaded {
                    capabilityGallery
                        .disabled(libraryReadFailed)
                        .animation(.easeInOut(duration: 0.18), value: selectedKind)
                } else if !libraryReadFailed {
                    ProgressView()
                }

                if selectedKind == .skills {
                    Button {
                        showSkillImporter = true
                    } label: {
                        Label(piText("chat.context.skills.upload_file", "Import Skill file"), systemImage: "square.and.arrow.down")
                    }
                    .accessibilityIdentifier("intelligence.capabilities.importSkill")
                    .disabled(!libraryLoaded || libraryReadFailed)

                    if let onUseSkill {
                        ForEach(profile.uploadedSkills.filter(\.enabled)) { skill in
                            Button(skill.name) { onUseSkill(skill.name) }
                        }
                    }
                    ForEach(hostSkills) { skill in
                        DisclosureGroup(skill.name) {
                            Text(skill.description)
                                .foregroundStyle(.secondary)
                            Text(skill.content)
                                .textSelection(.enabled)
                            Text(piText("chat.more_settings.skills.host_managed", "Provided by the host; manage it in the Mac skill library."))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            if let onUseSkill, skill.enabled {
                                Button(skill.name) { onUseSkill(skill.name) }
                            }
                        }
                        .disclosureGroupStyle(NolaRowDisclosureStyle())
                    }
                }
            }
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 18)
            .padding(.vertical, 16)
        }
        .background(DesignTokens.backgroundColor)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("intelligence.capabilities.screen")
        .navigationTitle(piText("personal.capabilities.title", "Skills & MCP"))
        .navigationBarTitleDisplayMode(.inline)
        .task { reload() }
        .refreshable { reload() }
        .onChange(of: selectedKind) { _, _ in query = "" }
        .fileImporter(isPresented: $showSkillImporter, allowedContentTypes: [.data]) { result in
            do {
                let url = try result.get()
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let handle = try FileHandle(forReadingFrom: url)
                defer { try? handle.close() }
                let data = try handle.read(upToCount: 192_001) ?? Data()
                guard data.count <= 192_000,
                      let text = String(data: data, encoding: .utf8),
                      !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                      text.count <= 48_000 else {
                    throw IOSApplicationRuntimeError.rejected(piText("personal.capabilities.invalid_import", "Choose a nonempty UTF-8 Skill file containing at most 48,000 characters."))
                }
                errorMessage = nil
                skillEditorSnapshot = nil
                mcpEditorSnapshot = nil
                editor = .skill(nil, UserUploadedContextItem(name: url.lastPathComponent, content: text))
            } catch { errorMessage = error.localizedDescription }
        }
        .sheet(item: $editor, onDismiss: { errorMessage = nil; skillEditorSnapshot = nil; mcpEditorSnapshot = nil }) { item in
            PersonalCapabilityEditorView(item: item, errorMessage: errorMessage) { name, detail, primary in
                saveEditor(item, name: name, detail: detail, primary: primary)
            }
        }
        .confirmationDialog(
            piText("personal.capabilities.delete.title", "Delete this capability?"),
            isPresented: Binding(
                get: { deleteTarget != nil },
                set: { if !$0 { deleteTarget = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(piText("personal.capabilities.delete", "Delete"), role: .destructive) {
                deletePendingCapability()
            }
        } message: {
            Text(deleteTarget?.name ?? "")
        }
        .alert(
            piText("personal.capabilities.error", "Could not save changes"),
            isPresented: Binding(
                get: { errorMessage != nil && editor == nil },
                set: { if !$0 { errorMessage = nil } }
            )
        ) {
            Button(piText("personal.capabilities.ok", "OK")) { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private var capabilitySearchField: some View {
        TextField(piText("personal.capabilities.search", "Search capabilities"), text: $query)
            .textFieldStyle(.roundedBorder)
            .frame(maxWidth: .infinity)
            .accessibilityIdentifier("intelligence.capabilities.search")
    }

    private var addCapabilityButton: some View {
        Button {
            addCapability()
        } label: {
            Label(
                selectedKind == .skills
                    ? piText("personal.capabilities.add_skill", "Add Skill")
                    : piText("personal.capabilities.add_mcp", "Add MCP"),
                systemImage: "plus"
            )
            .font(.system(size: 14, weight: .semibold))
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
        .tint(DesignTokens.actionColor)
        .foregroundStyle(DesignTokens.onActionColor)
        .frame(maxWidth: horizontalSizeClass == .compact ? .infinity : nil)
        .accessibilityLabel(selectedKind == .skills
            ? piText("personal.capabilities.add_skill", "Add Skill")
            : piText("personal.capabilities.add_mcp", "Add MCP"))
        .accessibilityIdentifier("intelligence.capabilities.add")
        .disabled(!libraryLoaded || libraryReadFailed)
    }

    @ViewBuilder
    private var capabilityGallery: some View {
        switch selectedKind {
        case .skills:
            let entries = profile.uploadedSkills.enumerated().filter { _, item in
                query.isEmpty || [item.name, item.description, item.content].contains { $0.localizedCaseInsensitiveContains(query) }
            }
            if entries.isEmpty {
                emptyState(isFiltered: !query.isEmpty, icon: "nola-file")
            } else {
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(entries, id: \.offset) { index, item in
                        PersonalCapabilityCard(
                            title: item.name.isEmpty ? piText("personal.capabilities.untitled_skill", "Untitled Skill") : item.name,
                            detail: item.description.isEmpty ? skillPreview(item.content) : item.description,
                            icon: "nola-file",
                            enabled: item.enabled,
                            onToggle: { toggleSkill(item, enabled: $0) },
                            onEdit: { openSkill(item, index: index) },
                            onDelete: { prepareSkillDeletion(item) },
                            identifier: "intelligence.capabilities.skill.\(item.id.uuidString)"
                        )
                    }
                }
            }
        case .mcp:
            let entries = profile.mcpServers.enumerated().filter { _, item in
                query.isEmpty || [item.name, item.description, item.endpoint].contains { $0.localizedCaseInsensitiveContains(query) }
            }
            if entries.isEmpty {
                emptyState(isFiltered: !query.isEmpty, icon: "nola-link")
            } else {
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(entries, id: \.offset) { index, item in
                        PersonalCapabilityCard(
                            title: item.name.isEmpty ? "MCP" : item.name,
                            detail: item.description.isEmpty ? item.endpoint : item.description,
                            icon: "nola-link",
                            enabled: item.enabled,
                            onToggle: { toggleMCP(item, enabled: $0) },
                            onEdit: { openMCP(item, index: index) },
                            onDelete: { prepareMCPDeletion(item) },
                            identifier: "intelligence.capabilities.mcp.\(item.id.uuidString)"
                        )
                    }
                }
            }
        }
    }

    private func skillPreview(_ content: String) -> String {
        content
            .components(separatedBy: .newlines)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { line in
                !line.isEmpty
                    && line != "---"
                    && !line.hasPrefix("#")
                    && !line.lowercased().hasPrefix("name:")
                    && !line.lowercased().hasPrefix("description:")
            }
            .prefix(3)
            .joined(separator: " ")
    }

    private func emptyState(isFiltered: Bool, icon: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(icon).renderingMode(.template).resizable().scaledToFit()
                .frame(width: 24, height: 24).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 5) {
                Text(isFiltered
                    ? piText("personal.capabilities.no_results", "No matching capabilities")
                    : piText("personal.capabilities.empty", "No capabilities yet"))
                    .font(.subheadline.weight(.semibold))
                if isFiltered || selectedKind == .skills {
                    Text(isFiltered
                        ? piText("personal.capabilities.no_results.detail", "Try a different search term.")
                        : piText("personal.capabilities.empty.detail", "Create one to make it available across conversations."))
                        .font(.subheadline).foregroundStyle(.secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 16)
    }

    private func addCapability() {
        guard libraryLoaded, !libraryReadFailed else { return }
        notice = nil
        errorMessage = nil
        skillEditorSnapshot = nil
        mcpEditorSnapshot = nil
        switch selectedKind {
        case .skills:
            editor = .skill(nil, UserUploadedContextItem(name: "", content: "# Skill\n\n## Workflow\n\n1. ", enabled: true))
        case .mcp:
            editor = .mcp(nil, UserMCPServer(name: "", endpoint: "", enabled: true))
        }
    }

    private func saveEditor(_ item: PersonalCapabilityEditorItem, name: String, detail: String, primary: String) {
        if case .skill(let index, let original) = item {
            do {
                guard index == nil || skillEditorSnapshot != nil else {
                    throw IOSApplicationRuntimeError.rejected("Reload the Skill before editing")
                }
                let value = UserUploadedContextItem(id: original.id, name: name, content: primary, description: detail, enabled: original.enabled)
                _ = try skillStore().saveSkill(value, expected: skillEditorSnapshot)
                reload()
                notice = piText("personal.capabilities.saved", "Changes saved")
                editor = nil
            } catch { errorMessage = error.localizedDescription }
            return
        }
        if case .mcp(let index, let original) = item {
            do {
                guard index == nil || mcpEditorSnapshot != nil else {
                    throw IOSApplicationRuntimeError.rejected("Reload the MCP server before editing")
                }
                let value = UserMCPServer(id: original.id, name: name, endpoint: primary, description: detail, enabled: original.enabled)
                _ = try mcpStore().saveMCP(value, expected: mcpEditorSnapshot)
                reload()
                notice = piText("personal.capabilities.saved", "Changes saved")
                editor = nil
            } catch { errorMessage = error.localizedDescription }
        }
    }

    private func skillStore() throws -> any ChatSkillEditingStore {
        guard libraryLoaded, !libraryReadFailed else { throw IOSApplicationRuntimeError.applicationUnavailable }
        guard let store = container.chatMemoryProfileStore as? any ChatSkillEditingStore else {
            throw IOSApplicationRuntimeError.applicationUnavailable
        }
        return store
    }

    private func openSkill(_ item: UserUploadedContextItem, index: Int) {
        do {
            let snapshot = try skillStore().loadSkillEditor(id: item.id)
            guard let current = snapshot.record.uploadedContextItem else {
                throw IOSApplicationRuntimeError.invalidResponse
            }
            errorMessage = nil
            skillEditorSnapshot = snapshot
            editor = .skill(index, current)
        } catch { errorMessage = error.localizedDescription }
    }

    private func prepareSkillDeletion(_ item: UserUploadedContextItem) {
        do {
            let snapshot = try skillStore().loadSkillEditor(id: item.id)
            deleteTarget = .skill(snapshot, snapshot.record.displayName)
        } catch { errorMessage = error.localizedDescription }
    }

    private func toggleSkill(_ item: UserUploadedContextItem, enabled: Bool) {
        do {
            let store = try skillStore()
            let snapshot = try store.loadSkillEditor(id: item.id)
            _ = try store.setSkillEnabled(enabled, expected: snapshot)
            reload()
            notice = piText("personal.capabilities.saved", "Changes saved")
        } catch { errorMessage = error.localizedDescription }
    }

    private func mcpStore() throws -> any ChatMCPEditingStore {
        guard libraryLoaded, !libraryReadFailed else { throw IOSApplicationRuntimeError.applicationUnavailable }
        guard let store = container.chatMemoryProfileStore as? any ChatMCPEditingStore else {
            throw IOSApplicationRuntimeError.applicationUnavailable
        }
        return store
    }

    private func openMCP(_ item: UserMCPServer, index: Int) {
        do {
            let snapshot = try mcpStore().loadMCPEditor(id: item.id)
            guard let current = snapshot.userMCPServer else { throw IOSApplicationRuntimeError.invalidResponse }
            errorMessage = nil
            mcpEditorSnapshot = snapshot
            editor = .mcp(index, current)
        } catch { errorMessage = error.localizedDescription }
    }

    private func prepareMCPDeletion(_ item: UserMCPServer) {
        do {
            let snapshot = try mcpStore().loadMCPEditor(id: item.id)
            deleteTarget = .mcp(snapshot, snapshot.displayName)
        } catch { errorMessage = error.localizedDescription }
    }

    private func toggleMCP(_ item: UserMCPServer, enabled: Bool) {
        do {
            let store = try mcpStore()
            _ = try store.setMCPEnabled(enabled, expected: store.loadMCPEditor(id: item.id))
            reload()
            notice = piText("personal.capabilities.saved", "Changes saved")
        } catch { errorMessage = error.localizedDescription }
    }

    private func deletePendingCapability() {
        guard let deleteTarget else { return }
        if case .skill(let snapshot, _) = deleteTarget {
            do {
                _ = try skillStore().deleteSkill(expected: snapshot)
                self.deleteTarget = nil
                reload()
                notice = piText("personal.capabilities.saved", "Changes saved")
            } catch { errorMessage = error.localizedDescription }
            return
        }
        if case .mcp(let snapshot, _) = deleteTarget {
            do {
                _ = try mcpStore().deleteMCP(expected: snapshot)
                self.deleteTarget = nil
                reload()
                notice = piText("personal.capabilities.saved", "Changes saved")
            } catch { errorMessage = error.localizedDescription }
        }
    }

    private func reload() {
        do {
            guard let store = container.chatMemoryProfileStore as? any ChatCapabilityLibraryStore else {
                throw IOSApplicationRuntimeError.applicationUnavailable
            }
            let snapshot = try store.loadCapabilityLibrary()
            profile = snapshot
            libraryLoaded = true
            libraryReadFailed = false
        } catch {
            libraryReadFailed = true
            notice = nil
        }
    }
}

private struct PersonalCapabilityCard: View {
    let title: String
    let detail: String
    let icon: String
    let enabled: Bool
    let onToggle: (Bool) -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void
    let identifier: String

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Image(icon).renderingMode(.template).resizable().scaledToFit()
                    .frame(width: 18, height: 18)
                    .foregroundStyle(DesignTokens.actionColor)
                    .frame(width: 34, height: 34)
                    .background(DesignTokens.actionColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
                Spacer()
                Toggle("", isOn: Binding(get: { enabled }, set: onToggle))
                    .labelsHidden()
                    .accessibilityLabel(title)
                    .accessibilityIdentifier("\(identifier).enabled")
                    .frame(minWidth: 44, minHeight: 44)
            }

            Button(action: onEdit) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                    Text(detail.replacingOccurrences(of: "\n", with: " "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(3)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .buttonStyle(NolaActionButtonStyle())

            HStack {
                Button(action: onEdit) {
                    HStack(spacing: 5) {
                        Text(piText("personal.capabilities.edit", "Edit"))
                        Image("nola-chevron").renderingMode(.template).resizable().scaledToFit()
                            .frame(width: 12, height: 12)
                    }
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(DesignTokens.actionColor)
                }
                .buttonStyle(NolaActionButtonStyle())
                Spacer()
                Menu {
                    Button(role: .destructive, action: onDelete) {
                        Label(piText("personal.capabilities.delete", "Delete"), systemImage: "trash")
                    }
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.caption.weight(.semibold))
                        .frame(width: 44, height: 44)
                }
                .accessibilityIdentifier("\(identifier).menu")
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 154, alignment: .topLeading)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Color.primary.opacity(0.055), lineWidth: 1)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(identifier)
    }
}

private struct PersonalCapabilityEditorView: View {
    @Environment(\.dismiss) private var dismiss
    let item: PersonalCapabilityEditorItem
    let errorMessage: String?
    let onSave: (String, String, String) -> Void
    @State private var name: String
    @State private var detail: String
    @State private var primary: String
    @State private var showDiscardConfirmation = false

    private var isDirty: Bool {
        switch item {
        case .skill(_, let value):
            return name != value.name || detail != value.description || primary != value.content
        case .mcp(_, let value):
            return name != value.name || detail != value.description || primary != value.endpoint
        }
    }

    private var endpointIsValid: Bool {
        RemoteModelEndpointURL.isValid(primary)
    }

    init(item: PersonalCapabilityEditorItem, errorMessage: String?, onSave: @escaping (String, String, String) -> Void) {
        self.item = item
        self.errorMessage = errorMessage
        self.onSave = onSave
        switch item {
        case .skill(_, let value):
            _name = State(initialValue: value.name)
            _detail = State(initialValue: value.description)
            _primary = State(initialValue: value.content)
        case .mcp(_, let value):
            _name = State(initialValue: value.name)
            _detail = State(initialValue: value.description)
            _primary = State(initialValue: value.endpoint)
        }
    }

    private var isSkill: Bool {
        if case .skill = item { return true }
        return false
    }

    private var isValid: Bool {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        if isSkill { return !primary.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        return endpointIsValid
    }

    var body: some View {
        NavigationStack {
            Form {
                if let errorMessage {
                    Section {
                        Text(errorMessage).foregroundStyle(.red)
                    }
                }
                Section {
                    TextField(piText("personal.capabilities.name", "Name"), text: $name)
                        .accessibilityIdentifier("intelligence.capabilities.editor.name")
                    TextField(piText("personal.capabilities.description", "Description"), text: $detail, axis: .vertical)
                        .accessibilityIdentifier("intelligence.capabilities.editor.description")
                        .lineLimit(2...5)
                }
                Section(isSkill
                    ? piText("personal.capabilities.instructions", "Skill instructions")
                    : piText("personal.capabilities.endpoint", "Endpoint")) {
                    if isSkill {
                        TextEditor(text: $primary)
                            .accessibilityIdentifier("intelligence.capabilities.editor.instructions")
                            .font(.body.monospaced())
                            .frame(minHeight: 260)
                        Text("\(primary.count.formatted()) \(piText("personal.capabilities.characters", "characters"))")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    } else {
                        TextField("https://example.com/mcp", text: $primary)
                            .textInputAutocapitalization(.never)
                            .keyboardType(.URL)
                        if !primary.isEmpty && !endpointIsValid {
                            Text(piText("personal.capabilities.invalid_endpoint", "Enter a valid HTTP or HTTPS endpoint."))
                                .font(.caption)
                                .foregroundStyle(.red)
                        }
                    }
                }
            }
            .navigationTitle(isSkill
                ? piText("personal.capabilities.skill", "Skill")
                : "MCP")
            .navigationBarTitleDisplayMode(.inline)
            .interactiveDismissDisabled(isDirty)
            .confirmationDialog(NSLocalizedString("common.discard_changes", value: "Discard Changes?", comment: "Discard changes confirmation"), isPresented: $showDiscardConfirmation) {
                Button(NSLocalizedString("common.discard", value: "Discard", comment: "Discard changes"), role: .destructive) { dismiss() }
                Button(L10n.cancel, role: .cancel) { }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(piText("personal.capabilities.cancel", "Cancel")) {
                        if isDirty { showDiscardConfirmation = true } else { dismiss() }
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(piText("personal.capabilities.save", "Save")) {
                        onSave(
                            name.trimmingCharacters(in: .whitespacesAndNewlines),
                            detail.trimmingCharacters(in: .whitespacesAndNewlines),
                            isSkill ? primary : primary.trimmingCharacters(in: .whitespacesAndNewlines)
                        )
                    }
                    .disabled(!isValid)
                    .accessibilityIdentifier("intelligence.capabilities.editor.save")
                }
            }
        }
        .accessibilityIdentifier("intelligence.capabilities.editor")
        .presentationDetents(isSkill ? [.large] : [.medium, .large])
        .presentationDragIndicator(.visible)
    }
}

private func piText(_ key: String, _ fallback: String) -> String {
    NSLocalizedString(key, value: fallback, comment: "Personal intelligence")
}

private enum MemoryFilter: String, CaseIterable, Identifiable {
    case all
    case facts
    case preferences
    case episodes

    var id: String { rawValue }

    var kind: String? {
        switch self {
        case .all: return nil
        case .facts: return "semantic"
        case .preferences: return "procedural"
        case .episodes: return "episodic"
        }
    }
}

private extension AgentCoreJSONValue {
    var jsonString: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }

    var jsonNumber: Double? {
        guard case .number(let value) = self else { return nil }
        return value
    }

    var jsonObject: [String: AgentCoreJSONValue]? {
        guard case .object(let value) = self else { return nil }
        return value
    }

    var jsonArray: [AgentCoreJSONValue]? {
        guard case .array(let value) = self else { return nil }
        return value
    }
}
