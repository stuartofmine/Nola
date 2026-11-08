// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents independently expandable conversation tools in one inspector.

import Combine
import Observation
import SwiftUI

final class MacInspectorNavigationState: ObservableObject {
    @Published var selectedTab: MacChatInspector.InspectorTab {
        didSet { expandedSections.insert(selectedTab); revealRevision &+= 1 }
    }
    @Published private(set) var revealRevision = 0
    @Published var knowledgePane: MacInspectorKnowledgePane {
        didSet { expandedKnowledge.insert(knowledgePane) }
    }
    @Published var expandedKnowledge: Set<MacInspectorKnowledgePane> = [.points]
    @Published var expandedSections: Set<MacChatInspector.InspectorTab> = [.overview]

    func select(_ tab: MacChatInspector.InspectorTab, isVisible: Bool) -> Bool {
        let remainsVisible = !isVisible || selectedTab != tab
        selectedTab = tab
        expandedSections.insert(tab)
        return remainsVisible
    }

    init(initialPane: String? = nil) {
        let rawValue = initialPane ?? ""
        let pane = MacInspectorKnowledgePane(rawValue: rawValue)
        selectedTab = pane == nil ? (MacChatInspector.InspectorTab(rawValue: rawValue) ?? .overview) : .context
        knowledgePane = pane ?? .points
        expandedKnowledge.insert(knowledgePane)
        expandedSections.insert(selectedTab)
    }
}

struct MacChatInspector: View {
    enum InspectorTab: String, CaseIterable, Identifiable {
        case overview, review, artifacts, progress, goal, context, execution, participants
        var id: String { rawValue }
        var title: String {
            switch self {
            case .overview: String(localized: "Overview")
            case .review: String(localized: "Review")
            case .artifacts: String(localized: "Artifacts")
            case .progress: String(localized: "inspector.progress", table: "ConversationInspector")
            case .goal: String(localized: "Goal")
            case .context: String(localized: "Context")
            case .execution: String(localized: "Execution")
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

    @Bindable var model: MacWorkspaceModel
    let hostDisplayName: String
    private let close: (() -> Void)?
    private let showsHeaderNavigation: Bool
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
        canReadLocalFiles: Bool = false,
        inspectFile: @escaping (MacCodeReference) -> Void = { _ in },
        openReview: @escaping (URL) -> Void = { _ in }
    ) {
        self.model = model
        self.hostDisplayName = hostDisplayName
        self.close = close
        self.showsHeaderNavigation = showsHeaderNavigation
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
                .overlay(alignment: .bottom) {
                    Rectangle()
                        .fill(MacDesignColor.borderSubtle)
                        .frame(height: 0.5)
                }
            ScrollViewReader { reader in
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(InspectorTab.allCases) { section in
                            DisclosureGroup(isExpanded: Binding(
                                get: { navigation.expandedSections.contains(section) },
                                set: { expanded in
                                    if expanded { navigation.expandedSections.insert(section) }
                                    else { navigation.expandedSections.remove(section) }
                                }
                            )) {
                                inspectorContent(section)
                                    .padding(.top, 10).padding(.bottom, 6)
                            } label: {
                                Label(section.title, systemImage: section.symbol)
                                    .font(.system(size: 13, weight: .semibold))
                                    .frame(minHeight: 32, alignment: .leading)
                            }
                            .id(section)
                            .accessibilityIdentifier("mac-inspector-section-" + section.rawValue)
                            .padding(.horizontal, 4)
                            Divider().opacity(0.5)
                        }
                    }
                    .padding(12)
                }
                .onChange(of: navigation.revealRevision) { _, _ in
                    let section = selectedTab
                    navigation.expandedSections.insert(section)
                    withAnimation(reduceMotion ? nil : .smooth(duration: 0.24)) {
                        reader.scrollTo(section, anchor: .top)
                    }
                }
                .animation(reduceMotion ? nil : .smooth(duration: 0.22), value: navigation.expandedSections)
            }
        }
        .background(MacDesignColor.inspector.ignoresSafeArea())
        .onReceive(
            NotificationCenter.default.publisher(for: .macChromeBridgeStatusDidChange)
                .receive(on: RunLoop.main)
        ) { _ in
            chrome.refresh()
        }
        .onReceive(
            NotificationCenter.default.publisher(for: .macChromeSetupAssistanceRequested)
                .receive(on: RunLoop.main)
        ) { _ in
            chrome.refresh()
            selectedTab = .context
        }

    }

    @ViewBuilder
    private func inspectorContent(_ section: InspectorTab) -> some View {
        switch section {
        case .overview: MacInspectorDecisionOverview(model: model, select: { selectedTab = $0 })
        case .review: MacInspectorReviewView(model: model, openReview: openReview)
        case .artifacts: MacInspectorArtifactsView(model: model, canReadLocalFiles: canReadLocalFiles, inspectFile: inspectFile)
        case .progress: MacInspectorProgressView(model: model, mode: .progress)
        case .goal: MacInspectorProgressView(model: model, mode: .goal)
        case .execution:
            MacInspectorProgressView(model: model, mode: .execution)
            MacInspectorActivityView(model: model, chromeSnapshot: chrome.snapshot)
        case .participants: MacInspectorParticipantsView(participants: participants)
        case .context:
            MacInspectorWorkingContextView(model: model)
            MacInspectorOverviewHeader(model: model, hostDisplayName: hostDisplayName)
            MacInspectorContextSummaryView(model: model)
            MacInspectorKnowledgeView(model: model, navigation: navigation)
            MacInspectorBrowserView(model: model, chrome: chrome)
            MacInspectorDataSourcesView(model: model, dataRegistry: dataRegistry,
                platformData: platformData, webSources: webSources)
        }
    }

    private var inspectorHeader: some View {
        HStack(spacing: 8) {
            Text(String(localized: "Conversation"))
                .font(.system(size: 13, weight: .semibold))
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
