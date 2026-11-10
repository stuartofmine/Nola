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
        (window?.contentView as? QLPreviewView)?.close()
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

    var body: some View {
        VStack(spacing: 0) {
            if urls.count > 1 {
                HStack {
                    Button { index -= 1 } label: { Image(systemName: "chevron.left") }
                        .disabled(index == 0)
                        .accessibilityLabel(String(localized: "Previous File"))
                        .keyboardShortcut(.leftArrow, modifiers: .command)
                    Text(urls[index].lastPathComponent).lineLimit(1)
                    Spacer()
                    Text("\(index + 1) / \(urls.count)").foregroundStyle(.secondary)
                    Button { index += 1 } label: { Image(systemName: "chevron.right") }
                        .disabled(index == urls.count - 1)
                        .accessibilityLabel(String(localized: "Next File"))
                        .keyboardShortcut(.rightArrow, modifiers: .command)
                }.padding(10)
                Divider()
            }
            MacNativeDocumentPreview(url: urls[index]).id(urls[index])
        }
        .onChange(of: index) { _, value in onSelectionChanged(value) }
    }
}
