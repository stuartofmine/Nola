// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents the conversation-scoped managed browser and Chrome connection controls.

import Observation
import SwiftUI
import WebKit

struct MacInspectorBrowserView: View {
    @Bindable var model: MacWorkspaceModel
    @Bindable var chrome: MacInspectorChromeModel
    @State private var showsExpandedBrowser = false

    var body: some View {
        browserContent
            .sheet(isPresented: $showsExpandedBrowser) {
                if let browser = model.selectedConversationBrowserSnapshot,
                   let previewURL = browser.previewURL,
                   URL(string: previewURL) != nil {
                    MacExpandedManagedBrowserView(
                        model: model,
                        onSelectPage: { index in
                            Task { await model.switchManagedBrowserPage(to: index) }
                        }
                    )
                }
            }
    }

    private var browserContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            MacInspectorSection(
                title: String(localized: "Managed Browser", table: "Chat"),
                showsDivider: false
            ) {
                HStack(spacing: 10) {
                    Image(systemName: "rectangle.connected.to.line.below")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(MacDesignColor.brand)
                        .frame(width: 30, height: 30)
                        .background(
                            MacDesignColor.brand.opacity(0.09),
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                        )
                    VStack(alignment: .leading, spacing: 3) {
                        Text(String(localized: "Playwright", table: "Chat"))
                            .font(.system(size: 12, weight: .semibold))
                        Text(String(localized: "One persistent browser session is shared by this conversation.", table: "Chat"))
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 4)
                    if let browser = model.selectedConversationBrowserSnapshot,
                       browser.pages.count > 1 {
                        Text(browser.pages.count.formatted())
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 7)
                            .frame(height: 22)
                            .background(Color.primary.opacity(0.055), in: Capsule())
                    }
                    Text(
                        model.selectedConversationHasManagedBrowser
                            ? String(localized: "Connected", table: "Chat")
                            : String(localized: "Available", table: "Chat")
                    )
                        .font(.system(size: 9.5, weight: .semibold))
                        .foregroundStyle(
                            model.selectedConversationHasManagedBrowser
                                ? Color.green
                                : Color.secondary
                        )
                }
            }

            if let browser = model.selectedConversationBrowserSnapshot {
                if !browser.pages.isEmpty {
                    browserPageTabs(browser.pages)
                }

                if !showsExpandedBrowser,
                   let previewURL = browser.previewURL,
                   let url = URL(string: previewURL) {
                    ZStack(alignment: .topTrailing) {
                        MacManagedBrowserSurface(url: url, owner: model.selectedConversationID ?? "", sessionID: browser.sessionID)
                            .frame(minHeight: 300, idealHeight: 390, maxHeight: 500)
                        Button {
                            showsExpandedBrowser = true
                        } label: {
                            Image(systemName: "arrow.up.left.and.arrow.down.right")
                                .font(.system(size: 11, weight: .semibold))
                                .frame(width: 28, height: 28)
                                .background(MacDesignColor.panel, in: RoundedRectangle(cornerRadius: 7))
                        }
                        .buttonStyle(.plain)
                        .padding(9)
                        .help(String(localized: "Expand"))
                    }
                    .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 11, style: .continuous)
                            .stroke(Color.primary.opacity(0.09), lineWidth: 1)
                    }
                }

                if browser.activePage == nil {
                    Text(
                        String.localizedStringWithFormat(
                            String(localized: "Last updated %@", table: "Chat"),
                            browser.updatedAt.formatted(date: .omitted, time: .shortened)
                        )
                    )
                    .font(.system(size: 9.5))
                    .foregroundStyle(.tertiary)
                }
            }

            MacInspectorSection(
                title: "Chrome",
                showsDivider: false
            ) {
                toolStatus(
                    String(localized: "Chrome Extension", table: "Chat"),
                    symbol: "puzzlepiece.extension",
                    available: chrome.snapshot.isExtensionConnected,
                    status: browserStatusTitle
                )
                Text(browserStatusDetail)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                ViewThatFits(in: .horizontal) {
                    chromeActions(horizontal: true)
                    chromeActions(horizontal: false)
                }
                if let error = chrome.actionError {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    @ViewBuilder private func chromeActions(horizontal: Bool) -> some View {
        let layout = horizontal
            ? AnyLayout(HStackLayout(spacing: 8))
            : AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
        layout {
            Button {
                chrome.installAndOpenExtensions()
            } label: {
                Label(String(localized: "Install Extension", table: "Chat"), systemImage: "square.and.arrow.down")
            }
            Button {
                chrome.openExtensions()
            } label: {
                Label(String(localized: "Open Chrome", table: "Chat"), systemImage: "arrow.up.right.square")
            }
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
    }

    private func browserPageTabs(_ pages: [MacConversationBrowserPage]) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(pages) { page in
                    Button {
                        guard !page.isActive else { return }
                        Task { await model.switchManagedBrowserPage(to: page.index) }
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: "globe")
                                .font(.system(size: 9.5, weight: .semibold))
                            Text(page.displayTitle)
                                .font(.system(size: 10.5, weight: page.isActive ? .semibold : .medium))
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        .foregroundStyle(page.isActive ? MacDesignColor.brand : Color.secondary)
                        .padding(.horizontal, 10)
                        .frame(maxWidth: 190)
                        .frame(height: 29)
                        .background(
                            page.isActive ? MacDesignColor.brand.opacity(0.11) : Color.primary.opacity(0.045),
                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                        )
                        .overlay {
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .stroke(
                                    page.isActive ? MacDesignColor.brand.opacity(0.22) : Color.clear,
                                    lineWidth: 1
                                )
                        }
                    }
                    .buttonStyle(.plain)
                    .help(page.url)
                }
            }
        }
    }
    private var browserStatusTitle: String {
        if model.executionLocation == .connectedHost {
            return String(localized: "Connected Host", table: "Chat")
        }
        return chrome.snapshot.isExtensionConnected
            ? NSLocalizedString("mac.extensions.status.connected", value: "Connected", comment: "Chrome connection state")
            : String(localized: "Setup Required", table: "Chat")
    }

    private var browserStatusDetail: String {
        if model.executionLocation == .connectedHost {
            return String(localized: "Browser actions are provided by the connected host.", table: "Chat")
        }
        return chrome.snapshot.isExtensionConnected
            ? NSLocalizedString("mac.extensions.chrome.detail.connected", value: "Nola can inspect and operate your active Chrome tabs.", comment: "Chrome connection detail")
            : NSLocalizedString("mac.extensions.chrome.detail.waiting", value: "Open Chrome and enable the Nola extension to finish connecting.", comment: "Chrome connection detail")
    }

    private func toolStatus(
        _ title: String,
        symbol: String,
        available: Bool,
        status: String? = nil
    ) -> some View {
        HStack(spacing: 9) {
            Image(systemName: symbol)
                .font(.system(size: 11, weight: .semibold))
                .frame(width: 26, height: 26)
                .foregroundStyle(available ? Color.accentColor : Color.secondary)
                .background(
                    (available ? Color.accentColor : Color.primary).opacity(available ? 0.075 : 0.028),
                    in: RoundedRectangle(cornerRadius: 7, style: .continuous)
                )
            Text(title)
            Spacer()
            Text(status ?? (available ? String(localized: "Enabled") : String(localized: "Disabled")))
                .font(.system(size: 9.5, weight: .medium))
                .foregroundStyle(available ? Color.green : Color.secondary)
        }
        .font(.system(size: 11.5))
        .frame(minHeight: 34)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(LocalizedStringKey(title)))
        .accessibilityValue(
            Text(
                available
                    ? String(localized: "Enabled")
                    : String(localized: "Disabled")
            )
        )
    }
}

struct MacManagedBrowserSurface: NSViewRepresentable {
    let url: URL
    let owner: String
    let sessionID: String?

    var documentIdentity: MacManagedBrowserDocumentIdentity {
        .init(owner: owner, sessionID: sessionID, endpoint: url)
    }

    final class Coordinator {
        var documentIdentity: MacManagedBrowserDocumentIdentity?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.setValue(false, forKey: "drawsBackground")
        view.load(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
        context.coordinator.documentIdentity = documentIdentity
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        // The preview document owns its frame stream. Frame timestamps never
        // navigate WebKit or reset focus, scrolling and reconnect state.
        guard context.coordinator.documentIdentity != documentIdentity else { return }
        context.coordinator.documentIdentity = documentIdentity
        view.load(URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData))
    }
}

struct MacManagedBrowserDocumentIdentity: Equatable {
    let owner: String
    let sessionID: String?
    let endpoint: URL
}

private struct MacExpandedManagedBrowserView: View {
    @Bindable var model: MacWorkspaceModel
    let onSelectPage: (Int) -> Void
    @Environment(\.dismiss) private var dismiss

    private var browser: MacConversationBrowserSnapshot? { model.selectedConversationBrowserSnapshot }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "safari")
                    .foregroundStyle(MacDesignColor.brand)
                Text(String(localized: "Managed Browser", table: "Chat"))
                    .font(.system(size: 13.5, weight: .semibold))
                Spacer()
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 28, height: 28)
                }
                .buttonStyle(.plain)
                .help(String(localized: "Close"))
            }
            .padding(.horizontal, 16)
            .frame(height: 50)

            Divider().opacity(0.55)

            if let browser, browser.pages.count > 1 {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(browser.pages) { page in
                            Button {
                                guard !page.isActive else { return }
                                onSelectPage(page.index)
                            } label: {
                                HStack(spacing: 6) {
                                    Image(systemName: "globe")
                                    Text(page.displayTitle)
                                        .lineLimit(1)
                                }
                                .font(.system(size: 11, weight: page.isActive ? .semibold : .medium))
                                .foregroundStyle(page.isActive ? MacDesignColor.brand : Color.secondary)
                                .padding(.horizontal, 11)
                                .frame(maxWidth: 240)
                                .frame(height: 31)
                                .background(
                                    page.isActive ? MacDesignColor.brand.opacity(0.11) : Color.primary.opacity(0.045),
                                    in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                                )
                            }
                            .buttonStyle(.plain)
                            .help(page.url)
                        }
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                }
            }

            if let browser, let previewURL = browser.previewURL.flatMap(URL.init(string:)) {
                MacManagedBrowserSurface(url: previewURL, owner: model.selectedConversationID ?? "", sessionID: browser.sessionID)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                    .padding([.horizontal, .bottom], 14)
            }
        }
        .frame(minWidth: 560, idealWidth: 1120, minHeight: 420, idealHeight: 760)
        .background(MacDesignColor.inspector)
    }
}
