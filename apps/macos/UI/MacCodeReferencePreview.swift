// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Routes Markdown file references into a native, line-focused code inspector.

import AppKit
import SwiftUI
import Quartz
import UniformTypeIdentifiers
import PDFKit

enum MacFilePreviewPolicy {
    nonisolated static func isLocalFileURL(_ url: URL) -> Bool {
        url.isFileURL && (url.host == nil || url.host == "" || url.host == "localhost")
    }
    nonisolated static func allowsLocalRead(sourceOwner: AppleConversationSourceOwner?, isLocalExecutionHost: Bool, url: URL) -> Bool {
        sourceOwner == .local && isLocalExecutionHost && isLocalFileURL(url)
    }
    nonisolated static func usesDocumentPreview(_ url: URL) -> Bool {
        guard let type = UTType(filenameExtension: url.pathExtension) else { return false }
        return !type.conforms(to: .text) && !type.conforms(to: .sourceCode)
    }
}

struct MacNativeDocumentPreview: NSViewRepresentable {
    let url: URL
    @Environment(\.macDetailScrollPositions) private var positions
    @Environment(\.macDetailScrollScope) private var scope

    @MainActor final class Coordinator {
        private weak var view: PDFView?
        private weak var positions: MacDetailScrollPositions?
        private var key = ""
        private var observers: [NSObjectProtocol] = []
        private(set) var generation: UInt64 = 0

        func capture() {
            guard let view, let document = view.document,
                  let destination = view.currentDestination,
                  let page = destination.page else { return }
            let pageIndex = document.index(for: page)
            guard pageIndex != NSNotFound else { return }
            positions?.savePDF(.init(pageIndex: pageIndex, point: destination.point,
                scaleFactor: view.scaleFactor, autoScales: view.autoScales), for: key)
        }

        func detach() {
            generation &+= 1
            detachCurrentOwner()
        }

        private func detachCurrentOwner() {
            capture()
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
            observers = []; view = nil
        }

        func requestAttachment(_ view: PDFView, key: String, replacingDocument: Bool) -> UInt64 {
            generation &+= 1
            // Capture before the caller changes PDFDocument or the owner key.
            if self.view !== view || self.key != key || replacingDocument { detachCurrentOwner() }
            return generation
        }

        func attach(_ view: PDFView, key: String, positions: MacDetailScrollPositions?, request: UInt64) {
            guard request == generation else { return }
            guard self.view !== view || self.key != key else { return }
            self.view = view; self.key = key; self.positions = positions
            view.layoutSubtreeIfNeeded()
            if let saved = positions?.pdfPosition(for: key), let document = view.document,
               let page = document.page(at: min(saved.pageIndex, max(0, document.pageCount - 1))) {
                view.autoScales = saved.autoScales
                if !saved.autoScales { view.scaleFactor = saved.scaleFactor }
                view.go(to: PDFDestination(page: page, at: saved.point))
            } else if let page = view.document?.page(at: 0) {
                // A reused PDFView can retain an old document's scroll origin.
                // A new owner without a saved viewport starts at its first page.
                view.autoScales = true
                view.go(to: PDFDestination(page: page, at: NSPoint(x: 0, y: page.bounds(for: .cropBox).maxY)))
            }
            for name in [Notification.Name.PDFViewPageChanged, .PDFViewScaleChanged] {
                observers.append(NotificationCenter.default.addObserver(forName: name, object: view, queue: .main) { [weak self] _ in
                    Task { @MainActor in self?.capture() }
                })
            }
            if let clip = view.documentView?.enclosingScrollView?.contentView {
                clip.postsBoundsChangedNotifications = true
                observers.append(NotificationCenter.default.addObserver(forName: NSView.boundsDidChangeNotification, object: clip, queue: .main) { [weak self] _ in
                    Task { @MainActor in self?.capture() }
                })
            }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }
    static func makePreview(for url: URL) -> NSView {
        if UTType(filenameExtension: url.pathExtension)?.conforms(to: .pdf) == true {
            guard let document = PDFDocument(url: url) else { return failureView() }
            guard !document.isLocked else {
                // Quick Look supplies the native password interaction for protected PDFs.
                return quickLook(for: url)
            }
            let view = PDFView()
            view.autoScales = true
            view.displayMode = .singlePageContinuous
            view.document = document
            return view
        }
        return quickLook(for: url)
    }
    private static func failureView() -> NSView {
        let view = NSTextField(wrappingLabelWithString: String(localized: "Unable to Preview File"))
        view.alignment = .center
        view.textColor = .secondaryLabelColor
        view.setAccessibilityIdentifier("mac-document-preview-error")
        return view
    }
    private static func quickLook(for url: URL) -> NSView {
        guard let view = QLPreviewView(frame: .zero, style: .normal) else { return failureView() }
        view.autostarts = true
        view.previewItem = url as NSURL
        return view
    }
    func makeNSView(context: Context) -> NSView { Self.makePreview(for: url) }
    func updateNSView(_ view: NSView, context: Context) {
        if let view = view as? PDFView {
            let key = scope + "/pdf/" + url.standardizedFileURL.absoluteString
            let coordinator = context.coordinator
            let replacingDocument = view.document?.documentURL != url
            let request = coordinator.requestAttachment(view, key: key, replacingDocument: replacingDocument)
            if replacingDocument { view.document = PDFDocument(url: url) }
            // PDFKit's document view and scroll clip are ready after layout.
            DispatchQueue.main.async { [weak view, weak positions] in
                guard let view else { return }
                coordinator.attach(view, key: key, positions: positions, request: request)
            }
        } else if let view = view as? QLPreviewView,
                  (view.previewItem as? NSURL) != url as NSURL { view.previewItem = url as NSURL }
    }
    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        coordinator.detach()
        (view as? QLPreviewView)?.close()
    }
}

struct MacCodeReference: Hashable, Sendable {
    let url: URL
    let line: Int?
    let column: Int?

    var targetDescription: String? {
        guard let line else { return nil }
        if let column {
            return "L\(line):\(column)"
        }
        return "L\(line)"
    }
}

enum MacCodeReferenceParser {
    nonisolated static func parse(_ url: URL) -> MacCodeReference? {
        parse(url, relativeTo: nil)
    }

    nonisolated static func parse(_ url: URL, relativeTo workspaceURL: URL?) -> MacCodeReference? {
        if let scheme = url.scheme?.lowercased(),
           !scheme.isEmpty,
           scheme != "file" {
            return nil
        }

        // URL.path is already decoded. Decoding it again changes literal %xx
        // filenames into a different resource.
        var rawPath = url.path
        guard !rawPath.isEmpty, url.host == nil || url.host == "" || url.host == "localhost" else { return nil }

        var line: Int?
        var column: Int?
        if let fragmentLocation = parseFragment(url.fragment) {
            line = fragmentLocation.line
            column = fragmentLocation.column
        }

        if let suffixLocation = parseLineSuffix(rawPath) {
            rawPath = suffixLocation.path
            line = line ?? suffixLocation.line
            column = column ?? suffixLocation.column
        }

        rawPath = (rawPath as NSString).expandingTildeInPath
        if !rawPath.hasPrefix("/") {
            guard let workspaceURL, workspaceURL.isFileURL else { return nil }
            rawPath = workspaceURL.appendingPathComponent(rawPath).path
        }

        return MacCodeReference(
            url: URL(fileURLWithPath: rawPath).standardizedFileURL,
            line: line,
            column: column
        )
    }

    private nonisolated static func parseFragment(
        _ fragment: String?
    ) -> (line: Int, column: Int?)? {
        guard let fragment, !fragment.isEmpty else { return nil }
        let pattern = #"^(?:L|line=)?([1-9]\d*)(?:C|:)?([1-9]\d*)?(?:-L[1-9]\d*)?$"#
        guard let match = firstMatch(pattern: pattern, value: fragment),
              let line = integer(match, group: 1, in: fragment) else {
            return nil
        }
        return (line, integer(match, group: 2, in: fragment))
    }

    private nonisolated static func parseLineSuffix(
        _ path: String
    ) -> (path: String, line: Int, column: Int?)? {
        let patterns = [
            #"^(.*):([1-9]\d*):([1-9]\d*)$"#,
            #"^(.*):([1-9]\d*)$"#,
        ]
        for pattern in patterns {
            guard let match = firstMatch(pattern: pattern, value: path),
                  let pathRange = Range(match.range(at: 1), in: path),
                  let line = integer(match, group: 2, in: path) else {
                continue
            }
            return (
                String(path[pathRange]),
                line,
                integer(match, group: 3, in: path)
            )
        }
        return nil
    }

    private nonisolated static func firstMatch(
        pattern: String,
        value: String
    ) -> NSTextCheckingResult? {
        guard let expression = try? NSRegularExpression(pattern: pattern) else { return nil }
        return expression.firstMatch(
            in: value,
            range: NSRange(value.startIndex..., in: value)
        )
    }

    private nonisolated static func integer(
        _ match: NSTextCheckingResult,
        group: Int,
        in value: String
    ) -> Int? {
        guard group < match.numberOfRanges else { return nil }
        let range = match.range(at: group)
        guard range.location != NSNotFound,
              let swiftRange = Range(range, in: value) else {
            return nil
        }
        return Int(value[swiftRange])
    }
}

struct MacOpenFileReferenceAction {
    private let handler: (MacCodeReference) -> Void

    init(_ handler: @escaping (MacCodeReference) -> Void) {
        self.handler = handler
    }

    func callAsFunction(_ reference: MacCodeReference) {
        handler(reference)
    }
}

private struct MacOpenFileReferenceKey: EnvironmentKey {
    static let defaultValue = MacOpenFileReferenceAction { _ in }
}

private struct MacFileReferenceBaseURLKey: EnvironmentKey {
    static let defaultValue: URL? = nil
}

extension EnvironmentValues {
    var macFileReferenceBaseURL: URL? {
        get { self[MacFileReferenceBaseURLKey.self] }
        set { self[MacFileReferenceBaseURLKey.self] = newValue }
    }
    var macOpenFileReference: MacOpenFileReferenceAction {
        get { self[MacOpenFileReferenceKey.self] }
        set { self[MacOpenFileReferenceKey.self] = newValue }
    }
}

private struct MacFileReferenceLinkModifier: ViewModifier {
    @Environment(\.macOpenFileReference) private var openFileReference
    @Environment(\.macFileReferenceBaseURL) private var workspaceURL

    func body(content: Content) -> some View {
        content.environment(
            \.openURL,
            OpenURLAction { url in
                guard let reference = MacCodeReferenceParser.parse(url, relativeTo: workspaceURL) else {
                    return .systemAction
                }
                openFileReference(reference)
                return .handled
            }
        )
    }
}

extension View {
    func macFileReferenceLinks() -> some View {
        modifier(MacFileReferenceLinkModifier())
    }
}

struct MacCodeReferenceDocument: Equatable, Sendable {
    nonisolated static let maximumPreviewBytes = 8 * 1_024 * 1_024

    let url: URL
    let lines: [String]
    let byteCount: Int
    let encodingName: String

    var lineCount: Int {
        lines.count
    }

    var text: String {
        lines.joined(separator: "\n")
    }
}

enum MacCodeReferenceLoadError: LocalizedError {
    case missing
    case directory
    case tooLarge(Int)
    case binary
    case unreadable

    var errorDescription: String? {
        switch self {
        case .missing:
            return String(localized: "The referenced file no longer exists.")
        case .directory:
            return String(localized: "The reference points to a folder, not a file.")
        case .tooLarge(let bytes):
            return String(
                localized: "This file is too large to preview (\(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)))."
            )
        case .binary:
            return String(localized: "This file does not contain previewable text.")
        case .unreadable:
            return String(localized: "Nola could not read this file.")
        }
    }
}

enum MacCodeReferenceLoader {
    nonisolated static func load(_ reference: MacCodeReference) throws -> MacCodeReferenceDocument {
        guard MacFilePreviewPolicy.isLocalFileURL(reference.url) else { throw MacCodeReferenceLoadError.unreadable }
        let fileManager = FileManager.default
        var isDirectory: ObjCBool = false
        guard fileManager.fileExists(atPath: reference.url.path, isDirectory: &isDirectory) else {
            throw MacCodeReferenceLoadError.missing
        }
        guard !isDirectory.boolValue else {
            throw MacCodeReferenceLoadError.directory
        }

        let attributes = try? fileManager.attributesOfItem(atPath: reference.url.path)
        let byteCount = (attributes?[.size] as? NSNumber)?.intValue ?? 0
        guard byteCount <= MacCodeReferenceDocument.maximumPreviewBytes else {
            throw MacCodeReferenceLoadError.tooLarge(byteCount)
        }
        guard let data = try? Data(contentsOf: reference.url, options: [.mappedIfSafe]) else {
            throw MacCodeReferenceLoadError.unreadable
        }

        let decoded = try decode(data)
        let normalized = decoded.text
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        return MacCodeReferenceDocument(
            url: reference.url,
            lines: normalized.components(separatedBy: "\n"),
            byteCount: data.count,
            encodingName: decoded.encodingName
        )
    }

    private nonisolated static func decode(_ data: Data) throws -> (text: String, encodingName: String) {
        if data.starts(with: [0xFF, 0xFE]),
           let text = String(data: data, encoding: .utf16LittleEndian) {
            return (text, "UTF-16 LE")
        }
        if data.starts(with: [0xFE, 0xFF]),
           let text = String(data: data, encoding: .utf16BigEndian) {
            return (text, "UTF-16 BE")
        }
        if let text = String(data: data, encoding: .utf8) {
            return (text, "UTF-8")
        }
        if data.prefix(4_096).contains(0) {
            throw MacCodeReferenceLoadError.binary
        }
        if let text = String(data: data, encoding: .isoLatin1) {
            return (text, "ISO-8859-1")
        }
        throw MacCodeReferenceLoadError.binary
    }
}

enum MacCodeSyntaxHighlighter {
    private struct Styles {
        let prefix: [NSAttributedString.Key: Any]
        let targetPrefix: [NSAttributedString.Key: Any]
        let body: [NSAttributedString.Key: Any]
        let keywordFont: NSFont
        let keyword: NSColor
        let literal: NSColor
        let string: NSColor
        let comment: NSColor
        let targetBackground: NSColor

        init(resolveColors: Bool) {
            func color(_ value: NSColor) -> NSColor {
                resolveColors ? (value.usingColorSpace(.deviceRGB) ?? value) : value
            }
            prefix = [.font: NSFont.monospacedSystemFont(ofSize: 10.5, weight: .regular),
                      .foregroundColor: color(.secondaryLabelColor.withAlphaComponent(0.62))]
            targetPrefix = [.font: NSFont.monospacedSystemFont(ofSize: 10.5, weight: .medium),
                            .foregroundColor: color(.controlAccentColor)]
            body = [.font: NSFont.monospacedSystemFont(ofSize: 11.5, weight: .regular),
                    .foregroundColor: color(.labelColor.withAlphaComponent(0.92))]
            keywordFont = NSFont.monospacedSystemFont(ofSize: 11.5, weight: .semibold)
            keyword = color(.systemPurple)
            literal = color(.systemBlue)
            string = color(.systemRed)
            comment = color(.systemGreen)
            targetBackground = color(.controlAccentColor.withAlphaComponent(0.075))
        }
    }

    private struct Rules {
        let keywords: NSRegularExpression?
        let literals: NSRegularExpression?
        let strings: NSRegularExpression?
        let comments: NSRegularExpression?

        init(fileExtension: String) {
            keywords = try? NSRegularExpression(pattern: "\\b(?:\(keywordPattern(for: fileExtension)))\\b")
            literals = try? NSRegularExpression(pattern: #"\b(?:true|false|null|nil|None|True|False|\d+(?:\.\d+)?)\b"#)
            strings = try? NSRegularExpression(pattern: #"(?:\"(?:\\.|[^\"\\])*\"|'(?:\\.|[^'\\])*')"#)
            comments = try? NSRegularExpression(pattern: commentPattern(for: fileExtension))
        }
    }

    static func attributedText(
        for document: MacCodeReferenceDocument,
        targetLine: Int?,
        appearance: NSAppearance? = nil
    ) -> NSAttributedString {
        if let appearance {
            // SwiftUI's colorScheme need not match AppKit's ambient appearance.
            // Resolve semantic colors before bridging the attributed text.
            var resolved = NSAttributedString(string: "")
            appearance.performAsCurrentDrawingAppearance {
                // Resolve the finite palette once, not once for every token
                // run followed by a second full attributed-string copy.
                resolved = render(document, targetLine: targetLine, styles: Styles(resolveColors: true))
            }
            return resolved
        }
        return render(document, targetLine: targetLine, styles: Styles(resolveColors: false))
    }

    private static func render(_ document: MacCodeReferenceDocument, targetLine: Int?,
                               styles: Styles) -> NSAttributedString {
        let result = NSMutableAttributedString()
        // Compile once per document, not four times for each source line.
        // Rules contain no source text and need no unbounded content cache.
        let rules = Rules(fileExtension: document.url.pathExtension.lowercased())
        let lineNumberWidth = max(3, String(max(1, document.lineCount)).count)
        let target = targetLine.map {
            min(max(1, $0), max(1, document.lineCount))
        }

        for (index, source) in document.lines.enumerated() {
            let lineNumber = index + 1
            let prefix = String(
                format: "%\(lineNumberWidth)d  ",
                lineNumber
            )
            let prefixText = NSAttributedString(
                string: prefix,
                attributes: lineNumber == target ? styles.targetPrefix : styles.prefix
            )
            result.append(prefixText)

            let highlighted = highlightedLine(
                source.isEmpty ? " " : source,
                rules: rules, styles: styles
            )
            result.append(highlighted)
            if index < document.lines.count - 1 {
                result.append(NSAttributedString(string: "\n"))
            }

            if lineNumber == target {
                let lineRange = (result.string as NSString).lineRange(
                    for: NSRange(
                        location: max(0, result.length - highlighted.length - prefixText.length),
                        length: highlighted.length + prefixText.length
                    )
                )
                result.addAttribute(
                    .backgroundColor,
                    value: styles.targetBackground,
                    range: NSRange(
                        location: lineRange.location,
                        length: min(lineRange.length, result.length - lineRange.location)
                    )
                )
            }
        }
        return result
    }

    private static func highlightedLine(
        _ source: String,
        rules: Rules,
        styles: Styles
    ) -> NSAttributedString {
        let text = NSMutableAttributedString(
            string: source,
            attributes: styles.body
        )
        let fullRange = NSRange(location: 0, length: text.length)
        apply(
            rules.keywords,
            to: text,
            range: fullRange,
            color: styles.keyword,
            font: styles.keywordFont
        )
        apply(
            rules.literals,
            to: text,
            range: fullRange,
            color: styles.literal
        )
        apply(
            rules.strings,
            to: text,
            range: fullRange,
            color: styles.string
        )
        apply(
            rules.comments,
            to: text,
            range: fullRange,
            color: styles.comment
        )
        return text
    }

    private static func apply(
        _ expression: NSRegularExpression?,
        to text: NSMutableAttributedString,
        range: NSRange,
        color: NSColor,
        font: NSFont? = nil
    ) {
        guard let expression else {
            return
        }
        for match in expression.matches(in: text.string, range: range) {
            text.addAttribute(.foregroundColor, value: color, range: match.range)
            if let font {
                text.addAttribute(.font, value: font, range: match.range)
            }
        }
    }

    private static func keywordPattern(for fileExtension: String) -> String {
        switch fileExtension {
        case "py":
            return "and|as|assert|async|await|break|class|continue|def|del|elif|else|except|finally|for|from|global|if|import|in|is|lambda|nonlocal|not|or|pass|raise|return|try|while|with|yield"
        case "swift":
            return "actor|as|associatedtype|async|await|break|case|catch|class|continue|default|defer|deinit|do|else|enum|extension|fallthrough|false|fileprivate|for|func|guard|if|import|in|init|inout|internal|is|let|nil|nonisolated|open|operator|private|protocol|public|repeat|required|rethrows|return|self|some|static|struct|subscript|super|switch|throw|throws|true|try|typealias|var|where|while"
        case "js", "jsx", "ts", "tsx":
            return "async|await|break|case|catch|class|const|continue|debugger|default|delete|do|else|enum|export|extends|false|finally|for|from|function|get|if|implements|import|in|instanceof|interface|let|new|null|of|package|private|protected|public|return|set|static|super|switch|this|throw|true|try|type|typeof|undefined|var|void|while|with|yield"
        default:
            return "break|case|catch|class|const|continue|def|do|else|enum|false|for|func|function|if|import|in|let|nil|null|private|public|return|static|struct|switch|throw|true|try|var|while"
        }
    }

    private static func commentPattern(for fileExtension: String) -> String {
        switch fileExtension {
        case "py", "rb", "sh", "bash", "zsh", "yaml", "yml", "toml":
            return #"#.*$"#
        default:
            return #"//.*$"#
        }
    }
}

struct MacCodeReferenceInspector: View {
    let reference: MacCodeReference
    let canReadLocally: Bool
    let close: () -> Void
    var openInTab: (() -> Void)? = nil

    @State private var document: MacCodeReferenceDocument?
    @State private var errorMessage: String?
    @State private var isLoading = true
    @State private var copied = false
    @State private var manualCopy = ""
    @State private var showsManualCopy = false
    @State private var scopedURL: URL?
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        VStack(spacing: 0) {
            if canReadLocally {
                header
                Divider().opacity(0.65)
                content
            } else {
                ContentUnavailableView(String(localized: "Unable to Preview File"), systemImage: "network.slash",
                    description: Text("This artifact requires its source Host to provide a preview."))
                Button("Close", action: close).padding(16)
            }
        }
        .background {
            if reduceTransparency {
                MacDesignColor.inspector
            } else {
                Rectangle()
                    .fill(.ultraThinMaterial)
                    .overlay(MacDesignColor.inspectorGlass)
            }
        }
        .task(id: "\(reference)/\(canReadLocally)") {
            guard canReadLocally, MacFilePreviewPolicy.isLocalFileURL(reference.url) else {
                document = nil
                scopedURL?.stopAccessingSecurityScopedResource(); scopedURL = nil
                return
            }
            await load()
        }
        .onDisappear { scopedURL?.stopAccessingSecurityScopedResource(); scopedURL = nil }
        .sheet(isPresented: $showsManualCopy) {
            VStack(alignment: .leading, spacing: 16) {
                Text("Copy").font(.headline)
                ScrollView { Text(manualCopy).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                HStack { Spacer(); Button("Done") { showsManualCopy = false }.keyboardShortcut(.defaultAction) }
            }.padding(24).frame(width: 560, height: 420)
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: fileSymbol)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(fileColor.opacity(0.78))
                .frame(width: 20, height: 20)

            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 7) {
                    Text(reference.url.lastPathComponent)
                        .font(.system(size: 12.5, weight: .semibold))
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                        .help(reference.url.lastPathComponent)
                    if let targetDescription = reference.targetDescription {
                        Text(targetDescription)
                            .font(.system(size: 11, weight: .medium, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                }
                Label(
                    reference.url.deletingLastPathComponent().lastPathComponent,
                    systemImage: "folder"
                )
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .help(reference.url.path)
            }

            Spacer(minLength: 10)
            if let document {
                Button {
                    NSPasteboard.general.clearContents()
                    guard NSPasteboard.general.setString(document.text, forType: .string) else {
                        manualCopy = document.text; showsManualCopy = true
                        return
                    }
                    copied = true
                    Task { @MainActor in
                        try? await Task.sleep(for: .seconds(1.2))
                        guard !Task.isCancelled else { return }
                        copied = false
                    }
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                }
                .macToolbarIcon(isActive: copied)
                .help(copied ? String(localized: "Copied") : String(localized: "Copy All"))
                .accessibilityLabel(copied ? String(localized: "Copied") : String(localized: "Copy All"))
            }
            Menu {
                if let openInTab {
                    Button(String(localized: "Open in Tab"), systemImage: "rectangle.stack") { openInTab() }
                    Divider()
                }
                Button(String(localized: "Copy Path"), systemImage: "doc.on.doc") {
                    NSPasteboard.general.clearContents()
                    if !NSPasteboard.general.setString(reference.url.path, forType: .string) {
                        manualCopy = reference.url.path; showsManualCopy = true
                    }
                }
                Button(String(localized: "Open File"), systemImage: "arrow.up.forward.app") {
                    NSWorkspace.shared.open(reference.url)
                }
                Divider()
                Button(String(localized: "Reveal in Finder"), systemImage: "folder") {
                    NSWorkspace.shared.activateFileViewerSelecting([reference.url])
                }
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 11, weight: .medium))
                    .frame(width: 26, height: 26)
                    .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .macToolbarIcon()
            .help(String(localized: "More"))
            .accessibilityLabel(String(localized: "More"))

            headerButton(
                symbol: "xmark",
                help: String(localized: "Close")
            ) {
                close()
            }
        }
        .padding(.horizontal, MacDesignTokens.Spacing.md)
        .padding(.vertical, MacDesignTokens.Spacing.sm)
        .frame(minHeight: MacDesignTokens.Control.pageHeader)
        .background(MacDesignColor.chrome.opacity(0.96))
    }

    @ViewBuilder
    private var content: some View {
        if isLoading {
            MacLoadingState(String(localized: "Loading"))
        } else if errorMessage == nil, MacFilePreviewPolicy.usesDocumentPreview(reference.url) {
            MacNativeDocumentPreview(url: reference.url)
                .id(reference.url)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let document {
            MacCodeReferenceDocumentView(
                document: document,
                targetLine: reference.line
            )
        } else {
            ContentUnavailableView(
                String(localized: "Unable to Preview File"),
                systemImage: "doc.badge.ellipsis",
                description: Text(errorMessage ?? String(localized: "The file could not be read."))
            )
        }
    }

    private func headerButton(
        symbol: String,
        help: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 10.5, weight: .medium))
        }
        .macToolbarIcon()
        .help(help)
        .accessibilityLabel(help)
    }

    private func load() async {
        guard canReadLocally, MacFilePreviewPolicy.isLocalFileURL(reference.url) else { return }
        scopedURL?.stopAccessingSecurityScopedResource()
        scopedURL = reference.url.startAccessingSecurityScopedResource() ? reference.url : nil
        isLoading = true
        document = nil
        errorMessage = nil
        if MacFilePreviewPolicy.usesDocumentPreview(reference.url) {
            if !FileManager.default.isReadableFile(atPath: reference.url.path) {
                errorMessage = MacCodeReferenceLoadError.unreadable.localizedDescription
            }
            isLoading = false
            return
        }
        do {
            let value = try await Task.detached(priority: .userInitiated) {
                try MacCodeReferenceLoader.load(reference)
            }.value
            guard !Task.isCancelled else { return }
            document = value
        } catch {
            guard !Task.isCancelled else { return }
            errorMessage = error.localizedDescription
        }
        isLoading = false
    }

    private var fileSymbol: String {
        switch reference.url.pathExtension.lowercased() {
        case "swift": return "swift"
        case "py": return "chevron.left.forwardslash.chevron.right"
        case "js", "ts", "jsx", "tsx": return "curlybraces"
        case "json", "yaml", "yml", "toml": return "text.alignleft"
        case "md": return "text.document"
        default: return "doc.text"
        }
    }

    private var fileColor: Color {
        switch reference.url.pathExtension.lowercased() {
        case "swift": return .orange
        case "py": return .blue
        case "js", "ts", "jsx", "tsx": return .yellow
        case "json", "yaml", "yml", "toml": return .green
        default: return .accentColor
        }
    }
}

private struct MacCodeReferenceDocumentView: View {
    let document: MacCodeReferenceDocument
    let targetLine: Int?

    var body: some View {
        VStack(spacing: 0) {
            MacSelectableCodeView(
                document: document,
                targetLine: targetLine
            )
            .background(Color(nsColor: .textBackgroundColor).opacity(0.28))

            HStack(spacing: 7) {
                Text("\(document.lineCount.formatted()) lines")
                Text("•")
                Text(document.encodingName)
                Text("•")
                Text(ByteCountFormatter.string(fromByteCount: Int64(document.byteCount), countStyle: .file))
                Spacer()
            }
            .font(.system(size: 10.5, weight: .regular, design: .monospaced))
            .foregroundStyle(.tertiary)
            .padding(.horizontal, 13)
            .frame(height: 28)
            .background(.bar)
        }
    }
}

private struct MacSelectableCodeView: NSViewRepresentable {
    let document: MacCodeReferenceDocument
    let targetLine: Int?
    @Environment(\.macDetailScrollPositions) private var positions
    @Environment(\.macDetailScrollScope) private var scope

    @MainActor final class Coordinator {
        var identity = ""
        var generation: UInt64 = 0
        var observation: NSObjectProtocol?
        func observe(_ scroll: NSScrollView, key: String, positions: MacDetailScrollPositions?) {
            if let observation { NotificationCenter.default.removeObserver(observation) }
            scroll.contentView.postsBoundsChangedNotifications = true
            observation = NotificationCenter.default.addObserver(forName: NSView.boundsDidChangeNotification,
                object: scroll.contentView, queue: .main) { [weak scroll, weak positions] _ in
                    MainActor.assumeIsolated {
                        guard let scroll else { return }
                        positions?.save(scroll.contentView.bounds.origin, for: key)
                    }
                }
        }
        func detach() {
            generation &+= 1
            if let observation { NotificationCenter.default.removeObserver(observation) }
            observation = nil
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder

        let textView = NSTextView()
        textView.isEditable = false
        textView.isSelectable = true
        textView.allowsUndo = false
        textView.drawsBackground = false
        textView.isRichText = true
        textView.importsGraphics = false
        textView.usesFindPanel = true
        textView.isIncrementalSearchingEnabled = true
        textView.textContainerInset = NSSize(width: 12, height: 10)
        textView.minSize = .zero
        textView.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = true
        textView.textContainer?.widthTracksTextView = false
        textView.textContainer?.containerSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        scrollView.documentView = textView
        return scrollView
    }

    static func dismantleNSView(_ scrollView: NSScrollView, coordinator: Coordinator) { coordinator.detach() }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? NSTextView else {
            return
        }
        let identity = [
            document.url.path,
            String(document.byteCount),
            String(targetLine ?? 0),
        ].joined(separator: ":")
        guard context.coordinator.identity != identity else { return }
        context.coordinator.identity = identity
        let viewportKey = scope + "/file/" + identity
        let savedPosition = positions?.position(for: viewportKey)
        context.coordinator.detach()

        textView.textStorage?.setAttributedString(
            MacCodeSyntaxHighlighter.attributedText(
                for: document,
                targetLine: targetLine
            )
        )
        textView.setSelectedRange(NSRange(location: 0, length: 0))

        if let savedPosition {
            let coordinator = context.coordinator
            let generation = coordinator.generation
            DispatchQueue.main.async { [weak scrollView, weak textView, weak coordinator, weak positions = positions] in
                guard let scrollView, let textView, let coordinator, coordinator.identity == identity,
                      coordinator.generation == generation else { return }
                if let container = textView.textContainer { textView.layoutManager?.ensureLayout(for: container) }
                scrollView.contentView.scroll(to: scrollView.contentView.constrainBoundsRect(NSRect(origin: savedPosition,
                    size: scrollView.contentView.bounds.size)).origin)
                scrollView.reflectScrolledClipView(scrollView.contentView)
                coordinator.observe(scrollView, key: viewportKey, positions: positions)
            }
            return
        }
        context.coordinator.observe(scrollView, key: viewportKey, positions: positions)
        guard let targetLine else {
            textView.scrollToBeginningOfDocument(nil)
            return
        }
        let clampedTarget = min(
            max(1, targetLine),
            max(1, document.lineCount)
        )
        let lineStart = document.lines
            .prefix(clampedTarget - 1)
            .reduce(0) { partial, line in
                partial + line.utf16.count + 1
            }
        let lineNumberWidth = max(3, String(max(1, document.lineCount)).count)
        let prefixLength = lineNumberWidth + 2
        let precedingPrefixes = (clampedTarget - 1) * prefixLength
        let location = min(
            max(0, lineStart + precedingPrefixes),
            max(0, textView.string.utf16.count - 1)
        )
        DispatchQueue.main.async {
            textView.scrollRangeToVisible(
                NSRange(location: location, length: 0)
            )
        }
    }
}
