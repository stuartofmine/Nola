// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides the macOS workspace background and native window chrome adapters.

import AppKit
import SwiftUI

struct MacWorkspaceBackground: View {
    var body: some View {
        MacDesignColor.canvas.ignoresSafeArea()
    }
}

struct MacChatWindowChromeConfigurator: NSViewRepresentable {
    let title: String

    func makeNSView(context: Context) -> WindowAttachmentView {
        let view = WindowAttachmentView(frame: .zero)
        view.desiredTitle = title
        return view
    }

    func updateNSView(_ nsView: WindowAttachmentView, context: Context) {
        nsView.desiredTitle = title
        nsView.applyWindowConfiguration()
    }

    final class WindowAttachmentView: NSView {
        var desiredTitle = ""
        private weak var configuredWindow: NSWindow?
        private var configurationScheduled = false

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            applyWindowConfiguration()
        }

        func applyWindowConfiguration() {
            guard window != nil, !configurationScheduled else { return }
            configurationScheduled = true
            // SwiftUI attaches native views while AppKit is laying out its
            // theme frame. Changing styleMask here synchronously reenters that
            // layout. Coalesce updates outside the attachment/update callback.
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.configurationScheduled = false
                self.configureCurrentWindow()
            }
        }

        private func configureCurrentWindow() {
            // Resolve at delivery, not scheduling: the view may have moved or
            // disappeared, and its desired title may have changed meanwhile.
            guard let window else { return }
            if window.title != desiredTitle { window.title = desiredTitle }
            guard configuredWindow !== window else { return }
            configuredWindow = window
            window.titleVisibility = .visible
            window.titlebarAppearsTransparent = false
            window.styleMask.remove(.fullSizeContentView)
            window.styleMask.formUnion([.titled, .closable, .miniaturizable, .resizable])
            window.backgroundColor = .windowBackgroundColor
            window.isMovableByWindowBackground = true
            window.toolbar?.isVisible = false
            window.toolbar = nil
            window.titlebarSeparatorStyle = .none
        }
    }
}
