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
            guard !isUpdatingReveal else { return }
            revealRevision &+= 1
        }
    }
    private var isUpdatingReveal = false
    @Published private(set) var revealRevision = 0
    @Published private(set) var revealedKnowledge: MacInspectorKnowledgePane?
    var revealID: String { revealedKnowledge.map { "knowledge-" + $0.rawValue } ?? selectedTab.rawValue }
    @Published var knowledgePane: MacInspectorKnowledgePane {
        didSet {
            expandedKnowledge.insert(knowledgePane)
            isUpdatingReveal = true
            selectedTab = .context
            isUpdatingReveal = false
            revealedKnowledge = knowledgePane
            revealRevision &+= 1
        }
    }
    let goalDrafts = MacInspectorGoalDraftStore()
    @Published var expandedKnowledge: Set<MacInspectorKnowledgePane> = []

    func setKnowledgeExpanded(_ pane: MacInspectorKnowledgePane, _ expanded: Bool) {
        if expanded {
            expandedKnowledge.insert(pane)
            isUpdatingReveal = true
            selectedTab = .context
            isUpdatingReveal = false
        } else {
            expandedKnowledge.remove(pane)
        }
    }

    func select(_ tab: MacChatInspector.InspectorTab, isVisible: Bool) -> Bool {
        let remainsVisible = !isVisible || selectedTab != tab
        selectedTab = tab
        return remainsVisible
    }

    init(
        selectedTab: MacChatInspector.InspectorTab = .task,
        revealedKnowledge: MacInspectorKnowledgePane? = nil
    ) {
        if let pane = revealedKnowledge {
            self.selectedTab = .context
            self.knowledgePane = pane
            self.expandedKnowledge.insert(pane)
            self.revealedKnowledge = pane
        } else {
            self.selectedTab = selectedTab
            self.knowledgePane = .points
            self.revealedKnowledge = nil
        }
    }

    convenience init(captureTab: String?) {
        if let rawValue = captureTab, let pane = MacInspectorKnowledgePane(rawValue: rawValue) {
            self.init(selectedTab: .context, revealedKnowledge: pane)
        } else if let rawValue = captureTab, let tab = MacChatInspector.InspectorTab(rawValue: rawValue) {
            self.init(selectedTab: tab)
        } else {
            self.init()
        }
    }
}

struct MacChatInspector: View {
    enum InspectorTab: String, CaseIterable, Identifiable {
        case task, artifacts, context
        var id: String { rawValue }
        var area: InspectorArea {
            switch self {
            case .task: .task
            case .artifacts: .artifacts
            case .context: .context
            }
        }
        var title: String { area.title }
        var iconAsset: String { area.icon }
        var symbol: String {
            switch self {
            case .task: "checklist"
            case .artifacts: "doc.on.doc"
            case .context: "tray.full"
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
            switch self { case .task: [.task]; case .artifacts: [.artifacts]; case .context: [.context] }
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
        _navigation = StateObject(wrappedValue: navigation ?? MacInspectorNavigationState())
    }

    var body: some View {
        VStack(spacing: 0) {
            inspectorHeader
                .padding(.horizontal, 12)
                .frame(height: 44)
                .background(MacDesignColor.inspector)
                .fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
                .overlay(alignment: .bottom) {
                    Rectangle()
                        .fill(MacDesignColor.borderSubtle)
                        .frame(height: 1)
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
        ) { _ in chrome.refresh(); selectedTab = .context }
    }

    private func inspectorRegion(_ area: InspectorArea) -> some View {
            ScrollViewReader { reader in
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        inspectorContent(area.sections[0])
                            .id(area.rawValue)
                    }
                    .padding(14)
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
        case .task:
            MacInspectorProgressView(model: model, mode: .progress)
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
            MacInspectorProgressView(model: model, mode: .goal, draftStore: navigation.goalDrafts)
            if !participants.isEmpty {
                Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
                MacInspectorParticipantsView(participants: participants)
            }
            expertHistory
        case .artifacts:
            MacInspectorArtifactsView(model: model, canReadLocalFiles: canReadLocalFiles, inspectFile: inspectFile)
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
            MacInspectorReviewView(model: model, openReview: openReview)
        case .context:
            MacInspectorWorkingContextView(model: model)
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
            MacInspectorContextSummaryView(model: model)
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
            MacInspectorKnowledgeView(model: model, navigation: navigation)
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
            MacInspectorDataSourcesView(model: model, dataRegistry: dataRegistry,
                platformData: platformData, webSources: webSources)
            if chrome.snapshot.errorMessage != nil || !chrome.snapshot.isExtensionConnected {
                Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 1)
                MacInspectorBrowserView(model: model, chrome: chrome)
            }
        }
    }

    private var inspectorHeader: some View {
        HStack(spacing: 8) {
            Image(selectedTab.iconAsset)
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .frame(width: 15, height: 15)
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
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
