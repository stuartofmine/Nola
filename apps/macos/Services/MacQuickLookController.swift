// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides native Quick Look previews for chat attachments and generated files.

import AppKit
import Foundation
import Quartz
import SwiftUI

@MainActor
final class MacQuickLookController: NSObject, NSWindowDelegate {
    static let shared = MacQuickLookController()

    private var previewURLs: [URL] = []
    private var currentIndex = 0
    private var window: NSWindow?
    private var scopedURLs: [URL] = []

    func preview(_ urls: [URL], startingAt index: Int = 0) {
        let requestedURL = urls.indices.contains(index) ? urls[index] : nil
        window?.close()
        scopedURLs = urls.filter { $0.startAccessingSecurityScopedResource() }
        let readableURLs = urls.filter { url in
            url.isFileURL && FileManager.default.isReadableFile(atPath: url.path)
        }
        guard !readableURLs.isEmpty else {
            releaseAccess()
            let alert = NSAlert()
            alert.messageText = String(localized: "Unable to Preview File")
            alert.informativeText = String(localized: "The file could not be read.")
            if let host = NSApp.keyWindow { alert.beginSheetModal(for: host) }
            else { alert.runModal() }
            return
        }
        previewURLs = readableURLs
        currentIndex = requestedURL.flatMap { readableURLs.firstIndex(of: $0) } ?? 0
        let preview = NSHostingView(rootView: MacDocumentPreviewGallery(urls: readableURLs, index: currentIndex) { [weak self] index in
            self?.currentIndex = index
            self?.window?.title = readableURLs[index].lastPathComponent
        })
        preview.frame = NSRect(x: 0, y: 0, width: 800, height: 680)
        let panel = NSWindow(contentRect: preview.frame, styleMask: [.titled, .closable, .resizable, .miniaturizable], backing: .buffered, defer: false)
        panel.isReleasedWhenClosed = false
        panel.title = readableURLs[currentIndex].lastPathComponent
        panel.contentView = preview
        panel.delegate = self
        window = panel
        panel.center()
        panel.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        (notification.object as? NSWindow)?.contentView = nil
        window = nil
        releaseAccess()
    }

    private func releaseAccess() {
        scopedURLs.forEach { $0.stopAccessingSecurityScopedResource() }
        scopedURLs = []
        previewURLs.removeAll(keepingCapacity: false)
        currentIndex = 0
    }
}

private struct MacDocumentPreviewGallery: View {
    let urls: [URL]
    @State var index: Int
    let onSelectionChanged: (Int) -> Void
    @State private var copiedPath = false
    @State private var copyFeedbackTask: Task<Void, Never>?

    private var currentURL: URL { urls[index] }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                if urls.count > 1 {
                    HStack(spacing: 4) {
                        Button { index -= 1 } label: { Image(systemName: "chevron.left") }
                            .buttonStyle(.borderless)
                            .disabled(index == 0)
                            .accessibilityLabel(String(localized: "Previous File"))
                            .keyboardShortcut(.leftArrow, modifiers: .command)
                        Text("\(index + 1) / \(urls.count)")
                            .font(.system(size: 11, weight: .medium, design: .monospaced))
                            .foregroundStyle(.secondary)
                        Button { index += 1 } label: { Image(systemName: "chevron.right") }
                            .buttonStyle(.borderless)
                            .disabled(index == urls.count - 1)
                            .accessibilityLabel(String(localized: "Next File"))
                            .keyboardShortcut(.rightArrow, modifiers: .command)
                    }
                    Divider()
                        .frame(height: 14)
                }

                Text(currentURL.lastPathComponent)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(currentURL.path)

                Spacer(minLength: 12)

                HStack(spacing: 4) {
                    Button {
                        NSWorkspace.shared.activateFileViewerSelecting([currentURL])
                    } label: {
                        Label(String(localized: "Reveal in Finder"), systemImage: "folder")
                            .font(.system(size: 11.5, weight: .medium))
                    }
                    .buttonStyle(.borderless)
                    .help(String(localized: "Reveal in Finder"))

                    Divider()
                        .frame(height: 12)

                    Button {
                        NSWorkspace.shared.open(currentURL)
                    } label: {
                        Label(String(localized: "Open"), systemImage: "arrow.up.forward.app")
                            .font(.system(size: 11.5, weight: .medium))
                    }
                    .buttonStyle(.borderless)
                    .help(String(localized: "Open with Default App"))

                    Divider()
                        .frame(height: 12)

                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(currentURL.path, forType: .string)
                        copiedPath = true
                        copyFeedbackTask?.cancel()
                        copyFeedbackTask = Task { @MainActor in
                            try? await Task.sleep(for: .seconds(1.5))
                            guard !Task.isCancelled else { return }
                            copiedPath = false
                        }
                    } label: {
                        Label(
                            copiedPath ? String(localized: "Copied") : String(localized: "Copy Path"),
                            systemImage: copiedPath ? "checkmark" : "doc.on.doc"
                        )
                        .font(.system(size: 11.5, weight: .medium))
                    }
                    .buttonStyle(.borderless)
                    .help(String(localized: "Copy Path"))
                }
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(MacDesignColor.canvas)

            Divider()

            MacNativeDocumentPreview(url: currentURL).id(currentURL)
        }
        .onChange(of: index) { _, value in
            copiedPath = false
            onSelectionChanged(value)
        }
        .onDisappear {
            copyFeedbackTask?.cancel()
        }
    }
}
