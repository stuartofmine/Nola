// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import WebKit

extension EnvironmentValues {
    @Entry var desktopPluginClient = MacDesktopPluginClient()
}

struct MacDesktopPluginPageView: View {
    @Environment(\.desktopPluginClient) private var pluginClient
    let page: MacDesktopPluginPage
    @State private var document: MacDesktopPluginDocument?
    @State private var failure: String?
    @State private var retry = 0
    var body: some View {
        Group {
            if let document, document.id == page.id {
                MacDesktopPluginWebView(html: document.html).accessibilityLabel(page.title)
            } else if let failure {
                VStack(spacing: 12) {
                    Text(failure).textSelection(.enabled)
                    Button(String(localized: "Retry")) { retry += 1 }
                }.padding()
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in retry += 1 }
        .onReceive(NotificationCenter.default.publisher(for: .macDesktopPluginsChanged)) { _ in retry += 1 }
        .task(id: retry) {
            failure = nil
            do {
                let current = try await pluginClient.document(page.id)
                if !Task.isCancelled { document = current }
            } catch {
                if !Task.isCancelled { document = nil; failure = error.localizedDescription }
            }
        }
    }
}

struct MacDesktopPluginWebView: NSViewRepresentable {
    let html: String

    final class Coordinator: NSObject, WKNavigationDelegate {
        var renderedHTML: String?
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            let url = action.request.url?.absoluteString
            let internalDocument = url == "about:blank" || url == "about:srcdoc"
            decisionHandler(action.targetFrame != nil && internalDocument ? .allow : .cancel)
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        guard html != context.coordinator.renderedHTML else { return }
        context.coordinator.renderedHTML = html
        view.loadHTMLString(Self.wrapper(html), baseURL: nil)
    }

    static func wrapper(_ html: String) -> String {
        let content = html.replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
        return """
        <!doctype html><meta charset="utf-8"><meta name="color-scheme" content="light dark">
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; frame-src about:; form-action 'none'; base-uri 'none'">
        <style>html,body,iframe{width:100%;height:100%;margin:0;border:0;display:block}</style>
        <iframe sandbox="allow-scripts" referrerpolicy="no-referrer" srcdoc="\(content)"></iframe>
        """
    }
}
