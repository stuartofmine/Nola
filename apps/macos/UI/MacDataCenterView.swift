// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents source health, policies, validation contracts, and provenance for macOS.

import AppKit
import Combine
import SwiftUI

private func macDataCenterText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    if keyed != key {
        return keyed
    }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

final class MacDataCenterNavigationState: ObservableObject {
    enum Route: Equatable {
        case overview
        case webSources
        case health
    }

    @Published var route: Route = .overview
    @Published var section: MacDataCenterSection = .sources

    func showOverview() { route = .overview }
    func showWebSources() { route = .webSources }
    func showHealth() { route = .health }
}

enum MacDataCenterSection: String, CaseIterable, Identifiable {
    case overview
    case sources
    case memory
    case plans
    case usage

    var id: String { rawValue }
    var title: String {
        switch self {
        case .overview: return macDataCenterText("mac.data_center.section.overview", "Overview")
        case .sources: return macDataCenterText("mac.data_center.section.sources", "Sources")
        case .plans: return String(localized: "workspace.plans")
        case .memory: return macDataCenterText("mac.data_center.section.memory", "Memory")
        case .usage: return macDataCenterText("mac.data_center.section.usage", "Usage")
        }
    }
}

struct MacDataCenterView: View {
    @State private var service: MacPlatformDataService
    @State private var registry: MacDataSourceRegistry
    @State private var webSources: MacWebSourceStore
    private var section: MacDataCenterSection {
        get { navigation.section }
        nonmutating set { navigation.section = newValue }
    }
    @StateObject private var navigation: MacDataCenterNavigationState
    let navigationAction: (() -> Void)?
    let automationNavigation: MacAutomationCenterNavigationState

    init(
        navigationAction: (() -> Void)? = nil,
        navigation: MacDataCenterNavigationState = MacDataCenterNavigationState(),
        service: MacPlatformDataService? = nil,
        registry: MacDataSourceRegistry? = nil,
        webSources: MacWebSourceStore? = nil,
        initialSection: String? = nil,
        automationNavigation: MacAutomationCenterNavigationState = MacAutomationCenterNavigationState()
    ) {
        _service = State(initialValue: service ?? .shared)
        _registry = State(initialValue: registry ?? .shared)
        _webSources = State(initialValue: webSources ?? .shared)
        if let initialSection, let section = MacDataCenterSection(rawValue: initialSection) {
            navigation.section = section
        }
        self.automationNavigation = automationNavigation
        self.navigationAction = navigationAction
        _navigation = StateObject(wrappedValue: navigation)
    }

    var body: some View {
        Group {
            switch navigation.route {
            case .overview: center
            case .webSources: MacWebSourcesView(navigationAction: navigation.showOverview)
            case .health: MacHealthSummaryView(navigationAction: navigation.showOverview)
            }
        }
    }

    private var center: some View {
        VStack(spacing: 0) {
            MacPageHeader(LocalizedStringKey("Data Center"), symbol: "square.stack.3d.up", navigationAction: navigationAction) {
                Text(macDataCenterText("mac.data_center.subtitle", "Control what Nola can perceive and how conversations may use it."))
            } actions: {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) {
                        manageSourcesButton(compact: false)
                        refreshButton(compact: false)
                    }
                    HStack(spacing: 6) {
                        manageSourcesButton(compact: true)
                        refreshButton(compact: true)
                    }
                }
            }

            Picker(macDataCenterText("mac.data_center.title", "Data Center"), selection: $navigation.section) {
                ForEach(MacDataCenterSection.allCases.filter { $0 != .overview }) { item in
                    Text(item.title).tag(item)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 420)
            .padding(.vertical, 12)

            Divider()

            if section == .plans {
                MacAutomationCenterView(navigation: automationNavigation)
            } else {
            ScrollView {
                Group {
                    switch section {
                    case .plans: EmptyView()
                    case .overview: overview
                    case .sources: sources
                    case .memory: MacMemoryCenterView()
                    case .usage: usage
                    }
                }
                .padding(.horizontal, MacDesignTokens.Spacing.xxl)
                .padding(.vertical, MacDesignTokens.Spacing.xl)
                .frame(maxWidth: MacDesignTokens.Layout.contentWidth)
                .frame(maxWidth: .infinity)
            }
            }
        }
        .background(MacPageBackground())
        .task { await refreshAll() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await service.refresh(synchronizeAutomations: false) }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSUbiquitousKeyValueStore.didChangeExternallyNotification)) { _ in
            Task { await service.refresh() }
        }
    }

    private var overview: some View {
        LazyVStack(alignment: .leading, spacing: 13) {
            MacDataCenterHealthCard(
                ready: readyCount,
                total: totalSourceCount,
                attention: attentionCount,
                defaultCount: defaultEnabledCount,
                isRefreshing: isRefreshing
            )

            if totalSourceCount == 0 {
                MacEmptyState(
                    LocalizedStringKey(macDataCenterText("mac.data_center.no_sources", "No data sources configured")),
                    description: LocalizedStringKey(macDataCenterText("mac.data_center.no_sources_detail", "Connect a source, then run a real read test before using it in a conversation.")),
                    symbol: "square.stack.3d.up.slash"
                ) {}
                .frame(minHeight: 220)
            } else {
                HStack(alignment: .firstTextBaseline) {
                    MacSectionLabel(LocalizedStringKey(macDataCenterText("mac.data_center.needs_attention", "Needs Attention")))
                    Spacer()
                    Button {
                        section = .sources
                    } label: {
                        HStack(spacing: 5) {
                            Text(macDataCenterText("mac.data_center.view_all_sources", "View All Sources"))
                        }
                    }
                    .buttonStyle(MacActionButtonStyle())
                    .controlSize(.small)
                    .foregroundStyle(.secondary)
                }
                .padding(.top, 5)

                if attentionSnapshots.isEmpty && attentionWebSources.isEmpty {
                    HStack(spacing: 10) {
                        Image(systemName: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                        Text(macDataCenterText("mac.data_center.health.ready", "All connected sources passed a real read test."))
                            .font(.system(size: 12, weight: .medium))
                        Spacer()
                    }
                    .padding(.vertical, 9)
                }

                ForEach(attentionSnapshots) { snapshot in
                    sourceCard(snapshot)
                }
                ForEach(attentionWebSources) { source in
                    webSourceCard(source)
                }
            }

            HStack(alignment: .firstTextBaseline) {
                MacSectionLabel(LocalizedStringKey(macDataCenterText("mac.data_center.recent_usage", "Recent Usage")))
                Spacer()
                if registry.usageRecords.count > 4 {
                    Button {
                        section = .usage
                    } label: {
                        HStack(spacing: 5) {
                            Text(macDataCenterText("mac.data_center.view_all_usage", "View All Usage"))
                        }
                    }
                    .buttonStyle(MacActionButtonStyle())
                    .controlSize(.small)
                    .foregroundStyle(.secondary)
                }
            }
            .padding(.top, 5)
            usagePreview
        }
    }

    private var sources: some View {
        LazyVStack(alignment: .leading, spacing: 12) {
            MacSectionLabel(LocalizedStringKey(macDataCenterText("mac.data_center.section.sources", "Sources")))
            if sourceSnapshots.isEmpty && webSources.sources.isEmpty {
                MacEmptyState(
                    LocalizedStringKey(macDataCenterText("mac.data_center.no_sources", "No data sources configured")),
                    description: LocalizedStringKey(macDataCenterText("mac.data_center.no_sources_detail", "Connect a source, then run a real read test before using it in a conversation.")),
                    symbol: "square.stack.3d.up.slash"
                ) {}
                .frame(minHeight: 280)
            } else {
                ForEach(MacDataSourceAccessPolicy.allCases) { policy in
                    let platformSources = sourceSnapshots.filter { sourcePolicy(for: $0) == policy }
                    let matchingWebSources = webSources.sources.filter { webSourcePolicy(for: $0) == policy }
                    if !platformSources.isEmpty || !matchingWebSources.isEmpty {
                        HStack(spacing: 8) {
                            Text(policy.title)
                                .font(.system(size: 11.5, weight: .semibold))
                            Text("\(platformSources.count + matchingWebSources.count)")
                                .font(.system(size: 9.5, weight: .semibold, design: .rounded))
                                .foregroundStyle(.secondary)
                                .padding(.horizontal, 7)
                                .frame(height: 20)
                                .background(Color.primary.opacity(0.045), in: Capsule())
                            Spacer()
                        }
                        .padding(.top, policy == .defaultEnabled ? 0 : 6)

                        ForEach(platformSources) { snapshot in
                            sourceCard(snapshot)
                        }
                        ForEach(matchingWebSources) { source in
                            webSourceCard(source)
                        }
                    }
                }
            }
        }
    }

    private var usage: some View {
        LazyVStack(alignment: .leading, spacing: 10) {
            MacSectionLabel(LocalizedStringKey(macDataCenterText("mac.data_center.recent_usage", "Recent Usage")))
            if registry.usageRecords.isEmpty {
                MacEmptyState(
                    LocalizedStringKey(macDataCenterText("mac.data_center.no_usage", "No source usage yet")),
                    description: LocalizedStringKey(macDataCenterText("mac.data_center.no_usage_detail", "Usage appears after a conversation or automation reads a source.")),
                    symbol: "clock.arrow.circlepath"
                ) {}
                .frame(minHeight: 280)
            } else {
                ForEach(registry.usageRecords.prefix(100)) { record in
                    MacDataSourceUsageRow(record: record)
                }
            }
        }
    }

    private var usagePreview: some View {
        Group {
            if registry.usageRecords.isEmpty {
                Text(macDataCenterText("mac.data_center.no_usage", "No source usage yet"))
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 64, alignment: .center)
                    .macPanel()
            } else {
                VStack(spacing: 0) {
                    ForEach(registry.usageRecords.prefix(4)) { record in
                        MacDataSourceUsageRow(record: record)
                        if record.id != registry.usageRecords.prefix(4).last?.id { Divider() }
                    }
                }
                .macPanel()
            }
        }
    }

    private func sourceCard(_ snapshot: MacDataSourceSnapshot) -> some View {
        let contract = registry.contract(for: snapshot.sourceID)
        let policy = sourcePolicy(for: snapshot)
        let healthAction: (() -> Void)? = snapshot.kind == .health
            ? { navigation.showHealth() }
            : nil
        return MacDataSourceCard(
            snapshot: snapshot,
            contract: contract,
            policy: policy,
            isTesting: service.isRefreshing,
            isRequestingPermission: service.requestingAccess.contains(snapshot.kind),
            permissionError: service.permissionRequestErrors[snapshot.kind],
            requestAccess: { Task { await service.requestAccess(for: snapshot.kind) } },
            testConnection: { Task { await service.validate(snapshot.kind) } },
            setPolicy: { service.setAccessPolicy($0, for: snapshot.kind) },
            openHealth: healthAction
        )
    }

    private func webSourceCard(_ source: MacWebSource) -> some View {
        let contract = registry.contract(for: source.dataSourceID)
        return MacWebDataSourceCard(
            source: source,
            contract: contract,
            policy: webSourcePolicy(for: source),
            isTesting: webSources.refreshingIDs.contains(source.id),
            testConnection: { Task { await webSources.refresh(source) } },
            setPolicy: { registry.setPolicy($0, for: source.dataSourceID) },
            openEditor: navigation.showWebSources
        )
    }

    private func sourcePolicy(for snapshot: MacDataSourceSnapshot) -> MacDataSourceAccessPolicy {
        let fallback: MacDataSourceAccessPolicy = snapshot.kind.supportsChatContext
            ? .conversationOptIn
            : .blocked
        return registry.policy(for: snapshot.sourceID, fallback: fallback)
    }

    private func webSourcePolicy(for source: MacWebSource) -> MacDataSourceAccessPolicy {
        registry.policy(
            for: source.dataSourceID,
            fallback: source.isEnabled ? .defaultEnabled : .conversationOptIn
        )
    }

    private var sourceSnapshots: [MacDataSourceSnapshot] {
        service.snapshots.filter { $0.kind != .notifications }
    }

    private var readyCount: Int {
        let platformReady = sourceSnapshots.filter {
            registry.contract(for: $0.sourceID)?.isUsable == true
                && $0.authorization == .available
        }.count
        let webReady = webSources.sources.filter {
            registry.contract(for: $0.dataSourceID)?.isUsable == true && $0.isEnabled
        }.count
        return platformReady + webReady
    }

    private var attentionCount: Int {
        attentionSnapshots.count + attentionWebSources.count
    }

    private var attentionSnapshots: [MacDataSourceSnapshot] {
        sourceSnapshots.filter {
            $0.authorization != .available
                || registry.contract(for: $0.sourceID)?.isUsable != true
        }
    }

    private var attentionWebSources: [MacWebSource] {
        webSources.sources.filter {
            $0.isEnabled && registry.contract(for: $0.dataSourceID)?.isUsable != true
        }
    }

    private var totalSourceCount: Int {
        sourceSnapshots.count + webSources.sources.count
    }

    private var defaultEnabledCount: Int {
        let platformDefaults = sourceSnapshots.filter {
            registry.policy(for: $0.sourceID) == .defaultEnabled
        }.count
        let webDefaults = webSources.sources.filter {
            registry.policy(
                for: $0.dataSourceID,
                fallback: $0.isEnabled ? .defaultEnabled : .conversationOptIn
            ) == .defaultEnabled
        }.count
        return platformDefaults + webDefaults
    }

    private var isRefreshing: Bool {
        service.isRefreshing || !webSources.refreshingIDs.isEmpty
    }

    private func refreshAll() async {
        async let platformRefresh: Void = service.refresh()
        async let webRefresh: Void = webSources.refreshAll()
        _ = await (platformRefresh, webRefresh)
    }

    @ViewBuilder
    private func manageSourcesButton(compact: Bool) -> some View {
        Button(action: navigation.showWebSources) {
            if compact {
                Image(systemName: "plus")
                    .frame(width: 18, height: 18)
            } else {
                Label(macDataCenterText("mac.data_center.manage_sources", "Manage Web Sources"), systemImage: "plus")
            }
        }
        .accessibilityLabel(macDataCenterText("mac.data_center.manage_sources", "Manage Web Sources"))
        .help(macDataCenterText("mac.data_center.manage_sources", "Manage Web Sources"))
    }

    @ViewBuilder
    private func refreshButton(compact: Bool) -> some View {
        Button {
            Task { await refreshAll() }
        } label: {
            HStack(spacing: 5) {
                ZStack {
                    Image(systemName: "arrow.clockwise")
                        .opacity(isRefreshing ? 0 : 1)
                    if isRefreshing { ProgressView().controlSize(.small) }
                }
                .frame(width: 18, height: 18)
                if !compact {
                    Text(macDataCenterText("mac.data_center.refresh", "Refresh"))
                }
            }
        }
        .disabled(isRefreshing)
        .accessibilityLabel(
            isRefreshing
                ? macDataCenterText("mac.data_center.refreshing", "Refreshing sources")
                : macDataCenterText("mac.data_center.refresh", "Refresh")
        )
        .help(macDataCenterText("mac.data_center.refresh", "Refresh"))
    }

}

private struct MacDataCenterHealthCard: View {
    let ready: Int
    let total: Int
    let attention: Int
    let defaultCount: Int
    let isRefreshing: Bool

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 15) {
                statusBlock
                    .layoutPriority(1)
                Spacer(minLength: 12)
                countBlock
            }
            VStack(alignment: .leading, spacing: 13) {
                statusBlock
                Divider()
                HStack {
                    Spacer(minLength: 0)
                    countBlock
                }
            }
        }
        .padding(.vertical, 5)
    }

    private var statusBlock: some View {
        HStack(spacing: 15) {
            Group {
                if isRefreshing { ProgressView().controlSize(.small) }
                else { Image(systemName: symbol).font(.system(size: 14, weight: .medium)).foregroundStyle(tint) }
            }
            .frame(width: 20, height: 20)
            VStack(alignment: .leading, spacing: 3) {
                Text(macDataCenterText("mac.data_center.health.title", "Source Health")).font(.system(size: 15, weight: .semibold))
                Text(summary)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var countBlock: some View {
        VStack(alignment: .trailing, spacing: 5) {
            Text("\(ready)/\(total)")
                .font(.system(size: 16, weight: .semibold, design: .rounded))
                .contentTransition(.numericText())
            Text(
                String(
                    format: macDataCenterText("mac.data_center.health.default_count", "%d available by default"),
                    defaultCount
                )
            )
            .font(.system(size: 10, weight: .medium))
            .foregroundStyle(.secondary)
        }
    }

    private var tint: Color { attention == 0 ? .green : .orange }
    private var symbol: String { attention == 0 ? "checkmark.shield.fill" : "exclamationmark.shield.fill" }
    private var summary: String {
        if total == 0 {
            return macDataCenterText("mac.data_center.health.empty", "Connect a source to begin building trusted context.")
        }
        return attention == 0
            ? macDataCenterText("mac.data_center.health.ready", "All connected sources passed a real read test.")
            : String(format: macDataCenterText("mac.data_center.health.attention", "%d sources need attention"), attention)
    }
}

private struct MacWebDataSourceCard: View {
    let source: MacWebSource
    let contract: MacDataSourceContract?
    let policy: MacDataSourceAccessPolicy
    let isTesting: Bool
    let testConnection: () -> Void
    let setPolicy: (MacDataSourceAccessPolicy) -> Void
    let openEditor: () -> Void
    @State private var isExpanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            MacDataSourceCardHeader(
                symbol: "network",
                tint: statusColor,
                title: source.name,
                subtitle: source.url,
                status: statusTitle,
                diagnostic: source.lastError ?? contract?.errorMessage,
                diagnosticTint: .red
            ) {
                HStack(spacing: 6) {
                Menu {
                    ForEach(MacDataSourceAccessPolicy.allCases) { option in
                        Button {
                            setPolicy(option)
                        } label: {
                            if option == policy { Label(option.title, systemImage: "checkmark") }
                            else { Text(option.title) }
                        }
                    }
                } label: {
                    HStack(spacing: 5) {
                        Text(policy.title)
                    }
                    .font(.system(size: 12, weight: .medium))
                    .padding(.horizontal, 10)
                    .frame(height: 32)
                    .background(Color.primary.opacity(0.045), in: Capsule())
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .accessibilityLabel(macDataCenterText("mac.data_center.policy.label", "Default access policy"))
                Button(action: openEditor) {
                    Image(systemName: "square.and.pencil")
                        .font(.system(size: 11, weight: .medium))
                }
                .macToolbarIcon()
                .help(macDataCenterText("mac.data_center.contract.edit", "Edit Source"))
                .accessibilityLabel(macDataCenterText("mac.data_center.contract.edit", "Edit Source"))
                if hasAdditionalDetails {
                    Button {
                        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) { isExpanded.toggle() }
                    } label: {
                        Image(systemName: "chevron.down")
                            .font(.system(size: 10, weight: .semibold))
                            .rotationEffect(.degrees(isExpanded ? 180 : 0))
                    }
                    .macToolbarIcon()
                    .help(macDataCenterText("mac.data_center.contract.details", "Details"))
                    .accessibilityLabel(macDataCenterText("mac.data_center.contract.details", "Details"))
                    .accessibilityValue(
                        Text(
                            isExpanded
                                ? macDataCenterText("mac.data_center.contract.expanded", "Expanded")
                                : macDataCenterText("mac.data_center.contract.collapsed", "Collapsed")
                        )
                    )
                }
            }
            }
        .padding(.vertical, 10)

            MacDataSourceContractSummary(
                contract: contract,
                isTesting: isTesting,
                testConnection: testConnection
            )

            if isExpanded {
                VStack(spacing: 0) {
                    Divider().padding(.horizontal, 14)
                    VStack(alignment: .leading, spacing: 12) {
                        if let contract {
                            if !contract.redactedSample.isEmpty {
                                MacDataContractLine(title: macDataCenterText("mac.data_center.contract.sample", "Redacted Sample"), value: contract.redactedSample)
                            }
                            if let error = contract.errorMessage {
                                Label(error, systemImage: "exclamationmark.triangle.fill")
                                    .font(.system(size: 10.5))
                                    .foregroundStyle(.orange)
                            }
                            if let sourceError = source.lastError,
                               sourceError != contract.errorMessage {
                                Label(sourceError, systemImage: "exclamationmark.triangle.fill")
                                    .font(.system(size: 10.5))
                                    .foregroundStyle(.red)
                                    .fixedSize(horizontal: false, vertical: true)
                                    .textSelection(.enabled)
                            }
                        } else {
                            Text(macDataCenterText("mac.data_center.contract.missing", "No validated contract. Test the connection before using this source."))
                                .font(.system(size: 10.5))
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(14)
                }
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .padding(.vertical, 2)
        .overlay(alignment: .bottom) {
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 0.5)
        }
    }

    private var hasAdditionalDetails: Bool {
        guard let contract else { return source.lastError != nil }
        return !contract.redactedSample.isEmpty
            || contract.errorMessage != nil
            || source.lastError != nil
    }

    private var statusTitle: String {
        if source.lastError != nil || contract?.errorMessage != nil {
            return macDataCenterText("mac.data_center.status.failed", "Validation failed")
        }
        if let contract { return contract.validationState.title }
        return macDataCenterText("mac.data_center.status.never_tested", "Never Tested")
    }

    private var statusColor: Color {
        if contract?.validationState == .failed || source.lastError != nil { return .red }
        return contract?.isUsable == true ? .green : .secondary
    }
}

private struct MacDataSourceCard: View {
    let snapshot: MacDataSourceSnapshot
    let contract: MacDataSourceContract?
    let policy: MacDataSourceAccessPolicy
    let isTesting: Bool
    let isRequestingPermission: Bool
    let permissionError: String?
    let requestAccess: () -> Void
    let testConnection: () -> Void
    let setPolicy: (MacDataSourceAccessPolicy) -> Void
    let openHealth: (() -> Void)?
    @State private var isExpanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            MacDataSourceCardHeader(
                symbol: snapshot.symbol,
                tint: statusColor,
                title: snapshot.title,
                subtitle: snapshot.subtitle,
                status: statusTitle
            ) {
                HStack(spacing: 6) {
                if snapshot.kind.supportsChatContext {
                    Menu {
                        ForEach(MacDataSourceAccessPolicy.allCases) { option in
                            Button {
                                setPolicy(option)
                            } label: {
                                if option == policy { Label(option.title, systemImage: "checkmark") }
                                else { Text(option.title) }
                            }
                        }
                    } label: {
                        HStack(spacing: 5) {
                            Text(policy.title)
                        }
                        .font(.system(size: 12, weight: .medium))
                        .padding(.horizontal, 10).frame(height: 32)
                        .background(Color.primary.opacity(0.045), in: Capsule())
                    }
                    .menuStyle(.borderlessButton)
                    .fixedSize()
                    .accessibilityLabel(macDataCenterText("mac.data_center.policy.label", "Default access policy"))
                }
                if hasAdditionalDetails {
                    Button {
                        withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) { isExpanded.toggle() }
                    } label: {
                        Image(systemName: "chevron.down")
                            .font(.system(size: 10, weight: .semibold))
                            .rotationEffect(.degrees(isExpanded ? 180 : 0))
                    }
                    .macToolbarIcon()
                    .help(macDataCenterText("mac.data_center.contract.details", "Details"))
                    .accessibilityLabel(macDataCenterText("mac.data_center.contract.details", "Details"))
                    .accessibilityValue(
                        Text(
                            isExpanded
                                ? macDataCenterText("mac.data_center.contract.expanded", "Expanded")
                                : macDataCenterText("mac.data_center.contract.collapsed", "Collapsed")
                        )
                    )
                }
            }
            }
            .padding(.vertical, 10)

            if snapshot.authorization != .available {
                if snapshot.authorization != .unavailable { authorizationAction }
            } else {
                MacDataSourceContractSummary(
                    contract: contract,
                    isTesting: isTesting,
                    testConnection: testConnection
                )
            }

            if let permissionError {
                VStack(alignment: .leading, spacing: 5) {
                    Label(macDataCenterText("mac.data_center.permission.failed", "Access could not be requested. Try again or review Nola in System Settings."), systemImage: "exclamationmark.triangle")
                    Text(permissionError).textSelection(.enabled)
                }
                .font(.callout)
                .foregroundStyle(.orange)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16).padding(.bottom, 12)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("data-source-permission-error-\(snapshot.kind.rawValue)")
            }

            if isExpanded {
                VStack(spacing: 0) {
                    Divider().padding(.horizontal, 14)
                    VStack(alignment: .leading, spacing: 12) {
                        if let contract {
                            if !contract.redactedSample.isEmpty {
                                VStack(alignment: .leading, spacing: 5) {
                                    Text(macDataCenterText("mac.data_center.contract.sample", "Redacted Sample")).font(.system(size: 9.5, weight: .semibold)).foregroundStyle(.secondary)
                                    Text(contract.redactedSample)
                                        .font(.system(size: 10, design: .monospaced))
                                        .foregroundStyle(.secondary)
                                        .textSelection(.enabled)
                                        .padding(9)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8))
                                }
                            }
                            if let error = contract.errorMessage {
                                Label(error, systemImage: "exclamationmark.triangle.fill")
                                    .font(.system(size: 10.5)).foregroundStyle(.orange)
                            }
                        } else {
                            Text(macDataCenterText("mac.data_center.contract.missing", "No validated contract. Test the connection before using this source."))
                                .font(.system(size: 10.5)).foregroundStyle(.secondary)
                        }

                        if let openHealth {
                            Button(macDataCenterText("mac.data_center.health.open", "Open health details"), action: openHealth)
                                .controlSize(.small)
                        }
                    }
                    .padding(14)
                }
                .macDisclosureTransition(reduceMotion: reduceMotion)
            }
        }
        .padding(.vertical, 2)
        .overlay(alignment: .bottom) {
            Rectangle().fill(MacDesignColor.borderSubtle).frame(height: 0.5)
        }
    }

    private var hasAdditionalDetails: Bool {
        guard let contract else { return openHealth != nil }
        return !contract.redactedSample.isEmpty
            || contract.errorMessage != nil
            || openHealth != nil
    }

    @ViewBuilder
    private var authorizationAction: some View {
        VStack(alignment: .leading, spacing: 8) {
            if snapshot.authorization == .denied || snapshot.authorization == .restricted {
                Text(macDataCenterText("mac.data_center.permission.recovery", "Review Nola’s access in System Settings, then return here to refresh its status. Restrictions may be managed by your organization."))
                    .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 10) {
                Spacer(minLength: 10)
                if snapshot.authorization == .notDetermined {
                    Button(action: requestAccess) {
                        if isRequestingPermission {
                            ProgressView().controlSize(.small)
                                .accessibilityLabel(macDataCenterText("mac.data_center.permission.requesting", "Requesting access…"))
                        } else {
                            Text(macDataCenterText("mac.data_center.permission.continue", "Continue"))
                        }
                    }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        .disabled(isRequestingPermission)
                } else if snapshot.authorization == .denied || snapshot.authorization == .restricted {
                    Button(macDataCenterText("mac.data_center.permission.settings", "System Settings")) {
                        if let url = URL(string: snapshot.kind.permissionSettingsURL) {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .controlSize(.small)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 12)
    }

    private var statusTitle: String {
        if let contract, contract.validationState == .failed { return contract.validationState.title }
        if snapshot.authorization != .available { return snapshot.authorization.title }
        return contract?.validationState.title ?? macDataCenterText("mac.data_center.status.never_tested", "Never Tested")
    }

    private var statusColor: Color {
        if contract?.validationState == .failed { return .red }
        switch snapshot.authorization {
        case .available: return contract?.isUsable == true ? .green : .accentColor
        case .notDetermined: return .accentColor
        case .denied, .restricted: return .orange
        case .unavailable: return .secondary
        }
    }
}

extension MacDataSourceSnapshot.Kind {
    var permissionSettingsURL: String {
        switch self {
        case .contacts: return "x-apple.systempreferences:com.apple.preference.security?Privacy_Contacts"
        case .calendar: return "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars"
        case .reminders: return "x-apple.systempreferences:com.apple.preference.security?Privacy_Reminders"
        case .notifications: return "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
        case .health: return "x-apple.systempreferences:com.apple.preference.security?Privacy"
        }
    }
}

private struct MacDataSourceCardHeader<Trailing: View>: View {
    let symbol: String
    let tint: Color
    let title: String
    let subtitle: String
    let status: String
    let diagnostic: String?
    let diagnosticTint: Color
    let trailing: Trailing

    init(
        symbol: String,
        tint: Color,
        title: String,
        subtitle: String,
        status: String,
        diagnostic: String? = nil,
        diagnosticTint: Color = .orange,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.symbol = symbol
        self.tint = tint
        self.title = title
        self.subtitle = subtitle
        self.status = status
        self.diagnostic = diagnostic
        self.diagnosticTint = diagnosticTint
        self.trailing = trailing()
    }

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 12) {
                identity
                Spacer(minLength: 12)
                trailing
            }
            VStack(alignment: .leading, spacing: 12) {
                identity
                trailing
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
    }

    private var identity: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(tint)
                .frame(width: 20, height: 20, alignment: .center)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.system(size: 14, weight: .semibold))
                    .lineLimit(2)
                Text(subtitle)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                HStack(spacing: 5) {
                    Circle()
                        .fill(tint)
                        .frame(width: 5, height: 5)
                        .accessibilityHidden(true)
                    Text(status)
                }
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(tint)
                if let diagnostic, !diagnostic.isEmpty {
                    Text(diagnostic)
                        .font(.system(size: 12))
                        .foregroundStyle(diagnosticTint)
                        .lineLimit(2)
                }
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct MacDataSourceContractSummary: View {
    let contract: MacDataSourceContract?
    let isTesting: Bool
    let testConnection: (() -> Void)?

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .center, spacing: 16) {
                contractFact(
                    symbol: "arrow.down.doc",
                    title: macDataCenterText("mac.data_center.contract.reader", "Reader"),
                    value: contract?.readerDescription ?? macDataCenterText("mac.data_center.status.never_tested", "Never Tested")
                )
                contractFact(
                    symbol: "curlybraces.square",
                    title: macDataCenterText("mac.data_center.contract.format", "Data Format"),
                    value: formatSummary
                )
                contractFact(
                    symbol: "clock",
                    title: macDataCenterText("mac.data_center.contract.validated", "Validated"),
                    value: validationSummary
                )
                Spacer(minLength: 4)
                testButton
            }
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .top, spacing: 12) {
                    contractFact(
                        symbol: "arrow.down.doc",
                        title: macDataCenterText("mac.data_center.contract.reader", "Reader"),
                        value: contract?.readerDescription ?? macDataCenterText("mac.data_center.status.never_tested", "Never Tested")
                    )
                    contractFact(
                        symbol: "curlybraces.square",
                        title: macDataCenterText("mac.data_center.contract.format", "Data Format"),
                        value: formatSummary
                    )
                }
                HStack(spacing: 12) {
                    contractFact(
                        symbol: "clock",
                        title: macDataCenterText("mac.data_center.contract.validated", "Validated"),
                        value: validationSummary
                    )
                    Spacer(minLength: 4)
                    testButton
                }
            }
        }
        .padding(.leading, 28)
        .padding(.trailing, 4)
        .padding(.vertical, 8)
    }

    private func contractFact(symbol: String, title: String, value: String) -> some View {
        HStack(alignment: .top, spacing: 7) {
            Image(systemName: symbol)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(contract?.isUsable == true ? Color.green : Color.secondary)
                .frame(width: 14, height: 14)
            VStack(alignment: .leading, spacing: 2) {
                Text(title.uppercased())
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(value)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .frame(minWidth: 104, maxWidth: 190, alignment: .leading)
    }

    @ViewBuilder
    private var testButton: some View {
        if let testConnection {
            if contract == nil {
                Button(action: testConnection) {
                    if isTesting {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 16, height: 16)
                    } else {
                        Label(
                            macDataCenterText("mac.data_center.contract.test", "Test Connection"),
                            systemImage: "waveform.path.ecg"
                        )
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .disabled(isTesting)
                .help(macDataCenterText("mac.data_center.contract.test", "Test Connection"))
                .accessibilityLabel(macDataCenterText("mac.data_center.contract.test", "Test Connection"))
            } else {
                Button(action: testConnection) {
                    if isTesting {
                        ProgressView()
                            .controlSize(.small)
                            .frame(width: 16, height: 16)
                    } else {
                        Image(systemName: "arrow.clockwise")
                    }
                }
                .macToolbarIcon()
                .disabled(isTesting)
                .help(macDataCenterText("mac.data_center.contract.test", "Test Connection"))
                .accessibilityLabel(macDataCenterText("mac.data_center.contract.test", "Test Connection"))
            }
        }
    }

    private var formatSummary: String {
        guard let contract else {
            return macDataCenterText("mac.data_center.contract.no_fields", "No fields detected")
        }
        return contract.schemaFields.isEmpty
            ? macDataCenterText("mac.data_center.contract.no_fields", "No fields detected")
            : contract.schemaFields.prefix(3).joined(separator: " · ")
    }

    private var validationSummary: String {
        guard let contract else {
            return macDataCenterText("mac.data_center.status.never_tested", "Never Tested")
        }
        return String(
            format: macDataCenterText("mac.data_center.contract.records", "%d records · %@"),
            contract.recordCount,
            contract.validatedAt.formatted(date: .abbreviated, time: .shortened)
        )
    }
}

private struct MacDataContractLine: View {
    let title: String
    let value: String

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(title)
                    .font(.system(size: 9.5, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 96, alignment: .leading)
                Text(value)
                    .font(.system(size: 10.5))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.system(size: 9.5, weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(value)
                    .font(.system(size: 10.5))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

private struct MacDataSourceUsageRow: View {
    let record: MacDataSourceUsageRecord

    private var localizedConsumer: String {
        switch record.consumer.lowercased() {
        case "chat": return macDataCenterText("mac.data_center.consumer.chat", "Chat")
        case "automation": return macDataCenterText("mac.data_center.consumer.automation", "Automation")
        default: return record.consumer
        }
    }

    private var localizedPurpose: String {
        switch record.purpose.lowercased() {
        case "context": return macDataCenterText("mac.data_center.purpose.context", "Conversation context")
        case "scheduled context refresh": return macDataCenterText("mac.data_center.purpose.refresh", "Scheduled context refresh")
        case "answer": return macDataCenterText("mac.data_center.purpose.answer", "Answer")
        case "scheduled summary": return macDataCenterText("mac.data_center.purpose.summary", "Scheduled summary")
        default: return record.purpose
        }
    }

    var body: some View {
        HStack(spacing: 11) {
            Image(systemName: record.disclosedToCloud ? "cloud" : "macbook")
                .font(.system(size: 11, weight: .medium)).foregroundStyle(record.disclosedToCloud ? Color.blue : Color.green)
                .frame(width: 28, height: 28).background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 2) {
                Text(record.sourceTitle).font(.system(size: 11.5, weight: .semibold))
                Text("\(localizedConsumer) · \(localizedPurpose)").font(.system(size: 9.5)).foregroundStyle(.secondary)
            }
            Spacer()
            Text(record.usedAt, style: .relative).font(.system(size: 9.5)).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 12).padding(.vertical, 9)
    }
}
