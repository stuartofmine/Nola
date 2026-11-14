// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents independently expandable conversation tools in one inspector.

import Combine
import Observation
import SwiftUI

@MainActor
final class MacInspectorNavigationState: ObservableObject {
    @Published var selectedTab: MacChatInspector.InspectorTab {
        didSet {
            revealedKnowledge = nil
            guard !isUpdatingDisclosure else { return }
            expandedSections.insert(selectedTab)
            revealRevision &+= 1
        }
    }
    private var isUpdatingDisclosure = false
    @Published private(set) var revealRevision = 0
    @Published private(set) var revealedKnowledge: MacInspectorKnowledgePane?
    var revealID: String { revealedKnowledge.map { "knowledge-" + $0.rawValue } ?? selectedTab.rawValue }
    @Published var knowledgePane: MacInspectorKnowledgePane {
        didSet {
            expandedKnowledge.insert(knowledgePane)
            selectedTab = .context
            revealedKnowledge = knowledgePane
            revealRevision &+= 1
        }
    }
    let goalDrafts = MacInspectorGoalDraftStore()
    @Published var expandedKnowledge: Set<MacInspectorKnowledgePane> = []
    @Published var expandedSections: Set<MacChatInspector.InspectorTab> = [.overview]

    /// Pointer disclosure stays in place; only explicit navigation requests scroll.
    func setExpanded(_ section: MacChatInspector.InspectorTab, _ expanded: Bool) {
        if expanded {
            expandedSections.insert(section)
            isUpdatingDisclosure = true
            selectedTab = section
            isUpdatingDisclosure = false
        } else {
            expandedSections.remove(section)
        }
    }

    func setKnowledgeExpanded(_ pane: MacInspectorKnowledgePane, _ expanded: Bool) {
        if expanded {
            expandedKnowledge.insert(pane)
            isUpdatingDisclosure = true
            selectedTab = .context
            isUpdatingDisclosure = false
        } else {
            expandedKnowledge.remove(pane)
        }
    }

    func select(_ tab: MacChatInspector.InspectorTab, isVisible: Bool) -> Bool {
        let remainsVisible = !isVisible || selectedTab != tab || !expandedSections.contains(tab)
        selectedTab = tab
        expandedSections.insert(tab)
        return remainsVisible
    }

    init(initialPane: String? = nil) {
        let rawValue = initialPane ?? ""
        let pane = MacInspectorKnowledgePane(rawValue: rawValue)
        selectedTab = pane == nil ? (MacChatInspector.InspectorTab(rawValue: rawValue) ?? .overview) : .context
        knowledgePane = pane ?? .points
        if let pane { expandedKnowledge.insert(pane) }
        expandedSections.insert(selectedTab)
        revealedKnowledge = pane
    }
}

struct MacChatInspector: View {
    enum InspectorTab: String, CaseIterable, Identifiable {
        case overview, progress, goal, review, artifacts, context, execution, participants
        var id: String { rawValue }
        var area: InspectorArea {
            switch self {
            case .overview, .progress, .goal: .task
            case .review, .artifacts: .artifacts
            case .context, .execution, .participants: .context
            }
        }
        var title: String {
            switch self {
            case .overview: String(localized: "Overview")
            case .review: String(localized: "Review")
            case .artifacts: String(localized: "Artifacts")
            case .progress: String(localized: "inspector.progress", table: "ConversationInspector")
            case .goal: String(localized: "Goal")
            case .context: String(localized: "Context")
            case .execution: String(localized: "inspector.section.execution", table: "ConversationInspector")
            case .participants: String(localized: "Participants")
            }
        }
        var iconAsset: String {
            switch self {
            case .context: "nola-file"
            case .execution: "nola-execution"
            case .participants: "nola-person"
            default: "nola-\(rawValue)"
            }
        }
        var symbol: String {
            switch self {
            case .overview: "square.grid.2x2"
            case .review: "text.magnifyingglass"
            case .artifacts: "doc.on.doc"
            case .progress: "checklist"
            case .goal: "scope"
            case .context: "tray.full"
            case .execution: "terminal"
            case .participants: "person.2"
            }
        }
    }

    enum InspectorArea: String, CaseIterable, Identifiable {
        case task, artifacts, context
        var id: String { rawValue }
        var title: String {
            switch self { case .task: String(localized: "Task"); case .artifacts: String(localized: "Artifacts"); case .context: String(localized: "Context") }
        }
        var icon: String {
            switch self { case .task: "nola-progress"; case .artifacts: "nola-artifacts"; case .context: "nola-file" }
        }
        var sections: [InspectorTab] {
            switch self { case .task: [.overview, .progress, .goal]; case .artifacts: [.review, .artifacts]; case .context: [.context, .execution, .participants] }
        }
    }

    @Bindable var model: MacWorkspaceModel
    let hostDisplayName: String
    private let close: (() -> Void)?
    private let showsHeaderNavigation: Bool
    private let expertHistory: AnyView
    private let participants: [MacInspectorParticipant]
    private let canReadLocalFiles: Bool
    private let inspectFile: (MacCodeReference) -> Void
    private let openReview: (URL) -> Void
    @State private var dataRegistry: MacDataSourceRegistry
    @State private var platformData: MacPlatformDataService
    @State private var webSources: MacWebSourceStore
    @State private var chrome: MacInspectorChromeModel
    @Environment(\.openWindow) private var openWindow
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @StateObject private var navigation: MacInspectorNavigationState
    private var selectedTab: InspectorTab {
        get { navigation.selectedTab }
        nonmutating set { navigation.selectedTab = newValue }
    }

    init(
        model: MacWorkspaceModel,
        hostDisplayName: String,
        initialKnowledgePaneRawValue: String? = nil,
        dependencies: MacInspectorDependencies,
        navigation: MacInspectorNavigationState? = nil,
        close: (() -> Void)? = nil,
        showsHeaderNavigation: Bool = false,
        participants: [MacInspectorParticipant] = [],
        expertHistory: AnyView = AnyView(EmptyView()),
        canReadLocalFiles: Bool = false,
        inspectFile: @escaping (MacCodeReference) -> Void = { _ in },
        openReview: @escaping (URL) -> Void = { _ in }
    ) {
        self.model = model
        self.hostDisplayName = hostDisplayName
        self.close = close
        self.showsHeaderNavigation = showsHeaderNavigation
        self.expertHistory = expertHistory
        self.participants = participants
        self.canReadLocalFiles = canReadLocalFiles
        self.inspectFile = inspectFile
        self.openReview = openReview
        _dataRegistry = State(initialValue: dependencies.dataRegistry)
        _platformData = State(initialValue: dependencies.platformData)
        _webSources = State(initialValue: dependencies.webSources)
        _chrome = State(
            initialValue: MacInspectorChromeModel(service: dependencies.chromeMessaging)
        )
        _navigation = StateObject(wrappedValue: navigation ?? MacInspectorNavigationState(initialPane: initialKnowledgePaneRawValue))
    }

    var body: some View {
        VStack(spacing: 0) {
            inspectorHeader
                .padding(.horizontal, 12)
                .frame(height: 44)
                .fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
                .overlay(alignment: .bottom) {
                    Rectangle()
                        .fill(MacDesignColor.borderSubtle)
                        .frame(height: 0.5)
                }
            // Separate retained reading surfaces preserve scroll and editor
            // drafts. Only the selected area is visible and interactive.
            ZStack(alignment: .top) {
                ForEach(InspectorArea.allCases) { area in
                    inspectorRegion(area)
                        .opacity(selectedTab.area == area ? 1 : 0)
                        .allowsHitTesting(selectedTab.area == area)
                        .disabled(selectedTab.area != area)
                        .accessibilityHidden(selectedTab.area != area)
                }
            }
            .frame(minHeight: 0, maxHeight: .infinity, alignment: .top)
            .clipped()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(MacDesignColor.inspector.ignoresSafeArea())
        .onReceive(
            NotificationCenter.default.publisher(for: .macChromeBridgeStatusDidChange)
                .receive(on: RunLoop.main)
        ) { _ in chrome.refresh() }
        .onReceive(
            NotificationCenter.default.publisher(for: .macChromeSetupAssistanceRequested)
                .receive(on: RunLoop.main)
        ) { _ in chrome.refresh(); selectedTab = .execution }
    }

    private func inspectorRegion(_ area: InspectorArea) -> some View {
            ScrollViewReader { reader in
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(area.sections) { section in
                            DisclosureGroup(isExpanded: Binding(
                                get: { navigation.expandedSections.contains(section) },
                                set: { expanded in
                                    navigation.setExpanded(section, expanded)
                                }
                            )) {
                                inspectorContent(section)
                                    .padding(.top, 10).padding(.bottom, 6)
                            } label: {
                                Label(section.title, systemImage: section.symbol)
                                    .font(.system(size: 13, weight: .semibold))
                                    .frame(minHeight: 32, alignment: .leading)
                            }
                            .disclosureGroupStyle(MacRowDisclosureStyle(toggleIdentifier: "mac-inspector-section-" + section.rawValue, isPlain: true))
                            .id(section.rawValue)
                            .padding(.horizontal, 4)
                            Divider().opacity(0.5)
                        }
                    }
                    .padding(12)
                }
                .task(id: navigation.revealRevision) {
                    // Wait for newly expanded content to enter the layout tree.
                    await Task.yield()
                    guard !Task.isCancelled, selectedTab.area == area else { return }
                    withAnimation(reduceMotion ? nil : .smooth(duration: 0.24)) {
                        reader.scrollTo(navigation.revealID, anchor: .top)
                    }
                }
            }
            .accessibilityIdentifier("mac-inspector-region-" + area.rawValue)
    }

    @ViewBuilder
    func inspectorContent(_ section: InspectorTab) -> some View {
        switch section {
        case .overview: MacInspectorOverviewHeader(model: model, hostDisplayName: hostDisplayName)
        case .review: MacInspectorReviewView(model: model, openReview: openReview)
        case .artifacts: MacInspectorArtifactsView(model: model, canReadLocalFiles: canReadLocalFiles, inspectFile: inspectFile)
        case .progress: MacInspectorProgressView(model: model, mode: .progress)
        case .goal: MacInspectorProgressView(model: model, mode: .goal, draftStore: navigation.goalDrafts)
        case .execution:
            MacInspectorProgressView(model: model, mode: .execution)
            MacInspectorBrowserView(model: model, chrome: chrome)
        case .participants:
            MacInspectorParticipantsView(participants: participants)
            expertHistory
        case .context:
            MacInspectorWorkingContextView(model: model)
            MacInspectorContextSummaryView(model: model)
            MacInspectorKnowledgeView(model: model, navigation: navigation)
            MacInspectorDataSourcesView(model: model, dataRegistry: dataRegistry,
                platformData: platformData, webSources: webSources)
        }
    }

    private var inspectorHeader: some View {
        HStack(spacing: 8) {
            Text(selectedTab.area.title)
                .font(.system(size: 13, weight: .semibold))
                .accessibilityIdentifier("mac-inspector-area-title")
            Spacer()
            if showsHeaderNavigation {
                MacConversationInspectorButtons(selection: selectedTab, isVisible: true) { tab in
                    if !navigation.select(tab, isVisible: true) { close?() }
                }
            }
            if let close {
                Button(action: close) {
                    Image(systemName: "xmark")
                        .frame(width: MacDesignTokens.Control.iconButton, height: MacDesignTokens.Control.iconButton)
                }
                .macToolbarIcon()
                .help(String(localized: "Close"))
                .accessibilityLabel(String(localized: "Close"))
                .accessibilityIdentifier("mac-inspector-close")
            }
        }
    }


}
