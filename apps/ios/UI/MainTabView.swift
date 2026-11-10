// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MainTabView logic in the iOS UI layer.
// - Interfaces:
//   - MainTabView
//   - NolaTab
//   - DeepLinkKey
//   - View
//   - EdgeBorder
// - APIs:
//   - none
//
//
//  MainTabView.swift
//  Nola
//
//  Main tab navigation shell.
//

import SwiftUI
import UIKit

struct MainTabView: View {
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @Environment(\.locale) private var locale
    @Environment(\.layoutDirection) private var layoutDirection
    @State private var chatDetailPresented = false
    @EnvironmentObject var container: ServiceContainer
    @SceneStorage("main.selectedTab") private var selectedTabRawValue = NolaTab.chat.rawValue
    @State private var managementDestination: IOSWorkspaceDestination?
    @State private var modelConfigurationFromChat = false
    @State private var modelConfigurationOwner: UUID?
    @State private var informationPath: [IOSWorkspaceDestination] = []
    @State private var informationRequests: [IOSInformationNavigationRequest] = []
    @State private var requestedConversationID: UUID?
    @State private var pendingNotifDeepLink: String? = nil

    var body: some View {
        Group {
            if horizontalSizeClass == .compact {
                IOSCompactTabContainer(selection: selectedTabBinding.wrappedValue) { tab in
                    AnyView(tabContent(tab)
                        .environmentObject(container)
                        .environment(\.colorScheme, colorScheme)
                        .environment(\.dynamicTypeSize, dynamicTypeSize)
                        .environment(\.locale, locale)
                        .environment(\.layoutDirection, layoutDirection)
                        .environment(\.symbolVariants, .none)
                        .tint(DesignTokens.actionColor))
                }
                    .safeAreaInset(edge: .bottom, spacing: 0) {
                        if selectedTabBinding.wrappedValue != .chat || !chatDetailPresented {
                            IOSKeyboardAwareBottomBar { bottomNavigation }
                        }
                    }
            } else if #available(iOS 18.0, *) {
                tabs
                    .tabViewStyle(.sidebarAdaptable)
            } else {
                tabs
            }
        }
        .environmentObject(container)
        .tint(DesignTokens.actionColor)
        .toolbarBackground(.visible, for: .tabBar)
        .toolbarBackground(DesignTokens.backgroundColor, for: .tabBar)
        .background(DesignTokens.backgroundColor.ignoresSafeArea())
        .onReceive(container.$pendingNotificationDedupeKey) { newVal in
            guard let key = newVal, !key.isEmpty else { return }
            select(.information)
            enqueueInformation(.records, notificationKey: key)
            // Consume once so repeated identical pushes still trigger.
            DispatchQueue.main.async {
                container.pendingNotificationDedupeKey = nil
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaOpenChatTab)) { note in
            requestedConversationID = note.object as? UUID
            select(.chat)
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaOpenModelTab)) { _ in
            pendingNotifDeepLink = nil
            modelConfigurationFromChat = true
            modelConfigurationOwner = container.chatManager.currentConversationId
            managementDestination = .models
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaOpenSettingsTab)) { _ in
            pendingNotifDeepLink = nil
            select(.settings)
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaOpenWorkspace)) { note in
            select(.information)
            if let destination = IOSWorkspaceDestination(page: note.object as? Int ?? -1) {
                enqueueInformation(destination)
            }
        }
        .sheet(item: $managementDestination, onDismiss: { pendingNotifDeepLink = nil; modelConfigurationFromChat = false; modelConfigurationOwner = nil }) { destination in
            IOSWorkspaceManagementSheet(destination: destination,
                notificationKey: destination == .records ? pendingNotifDeepLink : nil,
                onModelConfigured: { model in
                    guard modelConfigurationFromChat, modelConfigurationOwner == container.chatManager.currentConversationId else { return }
                    NotificationCenter.default.post(name: .nolaModelConfigured, object: model,
                        userInfo: ["owner": modelConfigurationOwner?.uuidString ?? "unsent"])
                    managementDestination = nil
                }).environmentObject(container)
        }
    }

    private var tabs: some View {
        TabView(selection: selectedTabBinding) {
            tabContent(.chat)
                .tabItem { Label(L10n.chatTitle, image: NolaTab.chat.icon).accessibilityIdentifier("tab.chat") }
                .tag(NolaTab.chat)
            tabContent(.activity)
                .tabItem { Label(String(localized: "information.activity"), image: NolaTab.activity.icon) }
                .tag(NolaTab.activity)
            tabContent(.information)
                .tabItem { Label(String(localized: "navigation.library"), image: NolaTab.information.icon).accessibilityIdentifier("tab.information") }
                .tag(NolaTab.information)
            tabContent(.settings)
                .tabItem { Label(L10n.settings, image: NolaTab.settings.icon).accessibilityIdentifier("tab.settings") }
                .tag(NolaTab.settings)
        }
        .environment(\.symbolVariants, .none)
    }

    @ViewBuilder
    private func tabContent(_ tab: NolaTab) -> some View {
        switch tab {
        case .chat:
            ChatNavigationView(requestedConversationID: $requestedConversationID, detailPresented: $chatDetailPresented)
                .accessibilityIdentifier("screen.chat")
        case .activity:
            NavigationStack {
                IOSConversationActivityView()
                    .navigationTitle(String(localized: "information.activity"))
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar {
                        ToolbarItem(placement: .topBarTrailing) {
                            NavigationLink { ScheduledActionsView(embedded: true) } label: {
                                Image("nola-calendar").renderingMode(.template)
                            }.accessibilityLabel(Text("workspace.plans"))
                        }
                    }
            }
        case .information:
            IOSInformationView(path: $informationPath, pendingRequests: $informationRequests)
        case .settings:
            SettingsView()
        }
    }

    private var bottomNavigation: some View {
        HStack(spacing: 0) {
            ForEach([NolaTab.chat, .activity, .information, .settings], id: \.self) { tab in
                Button { selectedTabBinding.wrappedValue = tab } label: {
                    VStack(spacing: 5) {
                        Image(tab.icon).renderingMode(.template).resizable().scaledToFit().frame(width: 20, height: 20)
                        Text(tab.title).font(.caption2)
                    }
                    .foregroundStyle(selectedTabBinding.wrappedValue == tab ? DesignTokens.actionColor : Color.secondary)
                    .frame(minWidth: 64, minHeight: 52)
                    .background(selectedTabBinding.wrappedValue == tab ? DesignTokens.elevatedColor : .clear,
                                in: RoundedRectangle(cornerRadius: 12))
                }
                .frame(maxWidth: .infinity)
                .accessibilityLabel(Text(tab.title))
                .accessibilityIdentifier("main.tab.\(tab.rawValue)")
                .accessibilityAddTraits(selectedTabBinding.wrappedValue == tab ? .isSelected : [])
            }
        }
        .buttonStyle(.plain).padding(.horizontal, 8).padding(.top, 6)
        .background(DesignTokens.surfaceColor.ignoresSafeArea(edges: .bottom))
        .overlay(alignment: .top) { Rectangle().fill(DesignTokens.borderColor).frame(height: 0.5) }
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isTabBar)
    }

    private var selectedTabBinding: Binding<NolaTab> {
        Binding(
            get: { NolaTab(rawValue: selectedTabRawValue) ?? .chat },
            set: { nextTab in
                guard nextTab.rawValue != selectedTabRawValue else { return }
                NolaHaptics.selection()
                selectedTabRawValue = nextTab.rawValue
            }
        )
    }

    private func enqueueInformation(_ destination: IOSWorkspaceDestination, notificationKey: String? = nil) {
        guard !informationRequests.contains(where: { $0.destination == destination && $0.notificationKey == notificationKey }) else { return }
        informationRequests.append(IOSInformationNavigationRequest(destination: destination, notificationKey: notificationKey))
    }

    private func select(_ tab: NolaTab) {
        selectedTabRawValue = tab.rawValue
    }
}

private enum NolaTab: String, Hashable {
    case activity
    case chat
    case information
    case settings

    var title: String {
        switch self {
        case .activity: String(localized: "information.activity")
        case .chat: L10n.chatTitle
        case .information: String(localized: "navigation.library")
        case .settings: L10n.settings
        }
    }

    var icon: String {
        switch self {
        case .activity: return "nola-clock"
        case .chat: return "nola-chat"
        case .information: return "nola-folder"
        case .settings: return "nola-settings"
        }
    }
}

// MARK: - Shared View Helpers

extension View {
    func border(width: CGFloat, edges: [Edge], color: Color) -> some View {
        overlay(EdgeBorder(width: width, edges: edges).foregroundColor(color))
    }
}

struct EdgeBorder: Shape {
    var width: CGFloat
    var edges: [Edge]

    func path(in rect: CGRect) -> Path {
        var path = Path()
        for edge in edges {
            switch edge {
            case .top:
                path.addRect(CGRect(x: rect.minX, y: rect.minY, width: rect.width, height: width))
            case .bottom:
                path.addRect(CGRect(x: rect.minX, y: rect.maxY - width, width: rect.width, height: width))
            case .leading:
                path.addRect(CGRect(x: rect.minX, y: rect.minY, width: width, height: rect.height))
            case .trailing:
                path.addRect(CGRect(x: rect.maxX - width, y: rect.minY, width: width, height: rect.height))
            }
        }
        return path
    }
}

#Preview {
    MainTabView()
        .environmentObject(ServiceContainer())
}

/// Navigation owns pages; ChatManager and the conversation draft store own work.
struct ChatNavigationView: View {
    @EnvironmentObject private var container: ServiceContainer
    @Binding var requestedConversationID: UUID?
    @Binding var detailPresented: Bool
    @State private var path: [UUID] = []
    @State private var readingPositions = ChatReadingPositions()
    @State private var search = ""

    var body: some View {
        NavigationStack(path: $path) {
            ChatConversationsList(search: $search) { id in
                if container.chatManager.currentConversationId != id {
                    container.chatManager.currentConversationId = id
                }
                if path.last != id { path.append(id) }
            }
            .navigationDestination(for: UUID.self) { id in
                ChatView(readingPositions: readingPositions).id(id)
                    .toolbar(.hidden, for: .tabBar)
            }
        }
        .onChange(of: path, initial: true) { _, value in detailPresented = !value.isEmpty }
        .onChange(of: requestedConversationID, initial: true) { _, id in
            guard let id else { return }
            container.chatManager.currentConversationId = id
            path = [id]
            requestedConversationID = nil
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaSearchConversations)) { note in
            search = note.object as? String ?? ""
            path = []
        }
    }
}

/// Keep keyboard-only layout state outside TabView's navigation and focus tree.
private struct IOSKeyboardAwareBottomBar<Content: View>: View {
    @State private var keyboardPresented = false
    @ViewBuilder var content: () -> Content

    var body: some View {
        content()
            .frame(height: keyboardPresented ? 0 : nil)
            .opacity(keyboardPresented ? 0 : 1)
            .clipped()
            .allowsHitTesting(!keyboardPresented)
            .accessibilityHidden(keyboardPresented)
            .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
                keyboardPresented = true
            }
            .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in
                keyboardPresented = false
            }
    }
}
