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
    var onTrafficLightInsetChange: ((CGFloat) -> Void)? = nil

    func makeNSView(context: Context) -> WindowAttachmentView {
        let view = WindowAttachmentView(frame: .zero)
        view.desiredTitle = title
        view.onTrafficLightInsetChange = onTrafficLightInsetChange
        return view
    }

    func updateNSView(_ nsView: WindowAttachmentView, context: Context) {
        nsView.desiredTitle = title
        nsView.onTrafficLightInsetChange = onTrafficLightInsetChange
        nsView.applyWindowConfiguration()
    }

    final class WindowAttachmentView: NSView {
        var desiredTitle = ""
        var onTrafficLightInsetChange: ((CGFloat) -> Void)?
        private weak var configuredWindow: NSWindow?
        private var configurationScheduled = false
        private var lastReportedInset: CGFloat = -1
        private var observers: [NSObjectProtocol] = []

        deinit {
            for observer in observers {
                NotificationCenter.default.removeObserver(observer)
            }
        }

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
            if configuredWindow !== window {
                configuredWindow = window
                for observer in observers {
                    NotificationCenter.default.removeObserver(observer)
                }
                observers = [
                    NotificationCenter.default.addObserver(
                        forName: NSWindow.didEnterFullScreenNotification,
                        object: window,
                        queue: .main
                    ) { [weak self] _ in self?.reportTrafficLightInset() },
                    NotificationCenter.default.addObserver(
                        forName: NSWindow.didExitFullScreenNotification,
                        object: window,
                        queue: .main
                    ) { [weak self] _ in self?.reportTrafficLightInset() },
                    NotificationCenter.default.addObserver(
                        forName: NSWindow.didResizeNotification,
                        object: window,
                        queue: .main
                    ) { [weak self] _ in self?.reportTrafficLightInset() },
                ]
                window.titleVisibility = .hidden
                window.titlebarAppearsTransparent = true
                window.styleMask.insert(.fullSizeContentView)
                window.styleMask.formUnion([.titled, .closable, .miniaturizable, .resizable])
                window.backgroundColor = NSColor(named: "NolaCanvas") ?? MacDesignTokens.Palette.fallbackNSColor(
                    light: MacDesignTokens.Palette.Light.canvas,
                    dark: MacDesignTokens.Palette.Dark.canvas
                )
                window.isMovableByWindowBackground = false
                window.toolbar?.isVisible = false
                window.toolbar = nil
                window.titlebarSeparatorStyle = .none
            }
            reportTrafficLightInset()
        }

        private func reportTrafficLightInset() {
            guard let window else { return }
            let isFullScreen = window.styleMask.contains(.fullScreen)
            let closeButton = window.standardWindowButton(.closeButton)
            let zoomButton = window.standardWindowButton(.zoomButton)
            let buttonsHidden = closeButton?.isHidden ?? false
            let inset: CGFloat
            if isFullScreen || buttonsHidden {
                inset = 0
            } else if let zoomButton, !zoomButton.isHidden, zoomButton.frame.maxX > 0 {
                let converted = zoomButton.convert(zoomButton.bounds, to: nil)
                inset = max(76, ceil(converted.maxX) + 12)
            } else {
                inset = 76
            }
            guard abs(inset - lastReportedInset) > 0.5 else { return }
            lastReportedInset = inset
            onTrafficLightInsetChange?(inset)
        }
    }
}

struct MacWindowDragRegion: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        DragView()
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class DragView: NSView {
        override var mouseDownCanMoveWindow: Bool { true }

        override func mouseDown(with event: NSEvent) {
            window?.performDrag(with: event)
        }
    }
}

struct MacResizeCursorRectView: NSViewRepresentable {
    var cursor: NSCursor = .resizeLeftRight

    func makeNSView(context: Context) -> NSView {
        CursorView(cursor: cursor)
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        guard let view = nsView as? CursorView else { return }
        view.cursor = cursor
        view.window?.invalidateCursorRects(for: view)
    }

    private final class CursorView: NSView {
        var cursor: NSCursor

        init(cursor: NSCursor) {
            self.cursor = cursor
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("init(coder:) has not been implemented")
        }

        override func resetCursorRects() {
            addCursorRect(bounds, cursor: cursor)
        }
    }
}
