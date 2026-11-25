// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns the native macOS chat composer, attachment input, and submission controls.

import AppKit
import SwiftUI
import UniformTypeIdentifiers

final class MacAttachmentPasteTextView: NSTextView {
    // Draft undo belongs to this editor, never the window's other fields.
    private let draftUndoManager = UndoManager()
    override var undoManager: UndoManager? { draftUndoManager }

    // Draw the hint through TextKit with the same container geometry as input.
    // A SwiftUI overlay has different font metrics and ignores native padding.
    var submitsReturn = true
    var pastedText: ((String) -> Void)?
    var placeholder: String = "" { didSet { needsDisplay = true } }

    override func didChangeText() {
        super.didChangeText()
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !hasMarkedText(), !placeholder.isEmpty,
              let container = textContainer else { return }
        let storage = NSTextStorage(string: placeholder, attributes: [
            .font: font ?? NSFont.systemFont(ofSize: 16),
            .foregroundColor: NSColor.placeholderTextColor,
            .paragraphStyle: defaultParagraphStyle ?? NSParagraphStyle.default,
        ])
        let layout = NSLayoutManager()
        let hintContainer = NSTextContainer(size: container.containerSize)
        hintContainer.lineFragmentPadding = container.lineFragmentPadding
        hintContainer.maximumNumberOfLines = 1
        storage.addLayoutManager(layout)
        layout.addTextContainer(hintContainer)
        layout.drawGlyphs(forGlyphRange: layout.glyphRange(for: hintContainer), at: textContainerOrigin)
    }

    func configureComposerTypography() {
        let inputFont = NSFont.systemFont(ofSize: 16)
        font = inputFont
        let naturalLineHeight = NSLayoutManager().defaultLineHeight(for: inputFont)
        textContainerInset = NSSize(width: 0, height: max(5, (36 - naturalLineHeight) / 2))
        let paragraph = NSMutableParagraphStyle()
        defaultParagraphStyle = paragraph
        typingAttributes[.paragraphStyle] = paragraph
    }

    var attachmentPasteHandler: (() -> Bool)?
    var attachmentDropHandler: ((NSPasteboard) -> Bool)?
    var submitHandler: (() -> Void)?
    var mentionHandler: ((String) -> Bool)?
    var focusChangeHandler: ((Bool) -> Void)?

    override func becomeFirstResponder() -> Bool {
        let accepted = super.becomeFirstResponder()
        if accepted { focusChangeHandler?(true) }
        return accepted
    }

    override func resignFirstResponder() -> Bool {
        let accepted = super.resignFirstResponder()
        if accepted { focusChangeHandler?(false) }
        return accepted
    }

    override func paste(_ sender: Any?) {
        if attachmentPasteHandler?() == true {
            return
        }
        super.paste(sender)
        pastedText?(string)
    }

    override func draggingEntered(_ sender: any NSDraggingInfo) -> NSDragOperation {
        canImportAttachments(from: sender.draggingPasteboard) ? .copy : super.draggingEntered(sender)
    }

    override func performDragOperation(_ sender: any NSDraggingInfo) -> Bool {
        if attachmentDropHandler?(sender.draggingPasteboard) == true {
            return true
        }
        return super.performDragOperation(sender)
    }

    func canImportAttachments(from pasteboard: NSPasteboard) -> Bool {
        pasteboard.canReadObject(
            forClasses: [NSURL.self, NSImage.self],
            options: [.urlReadingFileURLsOnly: true]
        )
            || pasteboard.availableType(from: [.png, .tiff]) != nil
    }

    override func keyDown(with event: NSEvent) {
        let isReturn = event.keyCode == 36 || event.keyCode == 76
        // Do not submit while an input method (IME) is composing marked text.
        // With CJK input methods, Return confirms the candidate composition and
        // must not be treated as a message send.
        let editingModifiers = event.modifierFlags.intersection([.shift, .option, .control, .command])
        // Holding Return must not accept another candidate or send another turn.
        // Marked text still belongs to the input method, including its repeats.
        if isReturn, editingModifiers.isEmpty, !hasMarkedText(), event.isARepeat { return }
        if editingModifiers.isEmpty, !hasMarkedText(),
           let command = [126: "up", 125: "down", 53: "dismiss", 48: "accept", 36: "accept", 76: "accept"][Int(event.keyCode)],
           mentionHandler?(command) == true { return }
        if submitsReturn, isReturn, editingModifiers.isEmpty, !hasMarkedText() {
            submitHandler?()
            return
        }
        super.keyDown(with: event)
    }
}

/// Size negotiation must not resize or lay out the live NSTextView: AppKit may
/// reenter its enclosing SwiftUI layout while a proposal is being evaluated.
@MainActor
enum MacComposerTextMeasurement {
    static func size(for textView: NSTextView, width: CGFloat, maxHeight: CGFloat = 132) -> CGSize {
        let storage = NSTextStorage(attributedString: textView.attributedString())
        let manager = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: width, height: .greatestFiniteMagnitude))
        container.lineFragmentPadding = textView.textContainer?.lineFragmentPadding ?? 0
        storage.addLayoutManager(manager)
        manager.addTextContainer(container)
        manager.ensureLayout(for: container)
        let insets = textView.textContainerInset.height * 2
        let lineHeight = max(
            textView.defaultParagraphStyle?.minimumLineHeight ?? 0,
            manager.defaultLineHeight(for: textView.font ?? NSFont.systemFont(ofSize: 16))
        )
        let height = max(lineHeight, manager.usedRect(for: container).height) + insets
        return CGSize(width: width, height: min(maxHeight, max(30, height)))
    }
}

private struct MacComposerEditorPlaceholderKey: EnvironmentKey {
    static let defaultValue: String? = nil
}

private struct MacComposerEditorMaxHeightKey: EnvironmentKey {
    static let defaultValue: CGFloat? = nil
}

extension EnvironmentValues {
    var macComposerEditorPlaceholder: String? {
        get { self[MacComposerEditorPlaceholderKey.self] }
        set { self[MacComposerEditorPlaceholderKey.self] = newValue }
    }

    var macComposerEditorMaxHeight: CGFloat? {
        get { self[MacComposerEditorMaxHeightKey.self] }
        set { self[MacComposerEditorMaxHeightKey.self] = newValue }
    }
}

struct MacComposerTextEditor: NSViewRepresentable {
    @Binding var text: String
    let conversationID: String
    let focused: FocusState<Bool>.Binding
    let onPasteAttachments: () -> Bool
    let onDropAttachments: (NSPasteboard) -> Bool
    let onSubmit: () -> Void
    var submitsReturn = true
    var mentionCommand: (String) -> Bool = { _ in false }
    var mentionEditor = AppleMentionEditor()
    var editorAccessibilityIdentifier: String?
    var placeholder: String? = nil
    var maxHeight: CGFloat? = nil
    @Environment(\.macComposerEditorPlaceholder) private var environmentPlaceholder
    @Environment(\.macComposerEditorMaxHeight) private var environmentMaxHeight

    private var resolvedPlaceholder: String {
        placeholder ?? environmentPlaceholder ?? String(localized: "Message Nola")
    }

    private var resolvedMaxHeight: CGFloat {
        maxHeight ?? environmentMaxHeight ?? 132
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true

        let textView = MacAttachmentPasteTextView()
        if let editorAccessibilityIdentifier {
            textView.setAccessibilityIdentifier(editorAccessibilityIdentifier)
        }
        textView.registerForDraggedTypes([.fileURL, .png, .tiff])
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.importsGraphics = false
        textView.drawsBackground = false
        textView.insertionPointColor = NSColor(named: "AccentColor") ?? .textColor
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = true
        textView.minSize = .zero
        textView.maxSize = NSSize(
            width: CGFloat.greatestFiniteMagnitude,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.textContainer?.lineFragmentPadding = 0
        textView.placeholder = resolvedPlaceholder
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.containerSize = NSSize(
            width: 0,
            height: CGFloat.greatestFiniteMagnitude
        )
        textView.configureComposerTypography()
        textView.string = text
        context.coordinator.conversationID = conversationID
        textView.attachmentPasteHandler = onPasteAttachments
        textView.attachmentDropHandler = onDropAttachments
        textView.submitsReturn = submitsReturn
        textView.submitHandler = onSubmit
        textView.mentionHandler = mentionCommand
        textView.pastedText = mentionEditor.pasted
        textView.focusChangeHandler = { [weak coordinator = context.coordinator] in
            coordinator?.publishFocus($0)
        }
        scrollView.documentView = textView
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? MacAttachmentPasteTextView else { return }
        context.coordinator.parent = self
        if context.coordinator.conversationID != conversationID {
            context.coordinator.conversationID = conversationID
            context.coordinator.invalidateFocusPublication()
            context.coordinator.isReplacingConversation = true
            textView.unmarkText()
            textView.undoManager?.removeAllActions()
            context.coordinator.undoHistory = AppleMentionUndoHistory()
            textView.string = text
            textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
            textView.needsDisplay = true
            context.coordinator.isReplacingConversation = false
        }
        if textView.placeholder != resolvedPlaceholder {
            textView.placeholder = resolvedPlaceholder
        }
        if !focused.wrappedValue { context.coordinator.nativeFocusLostAwaitingPublication = false }
        textView.attachmentPasteHandler = onPasteAttachments
        textView.attachmentDropHandler = onDropAttachments
        textView.submitsReturn = submitsReturn
        textView.submitHandler = onSubmit
        textView.mentionHandler = mentionCommand
        textView.pastedText = mentionEditor.pasted
        textView.focusChangeHandler = { [weak coordinator = context.coordinator] in
            coordinator?.publishFocus($0)
        }
        if textView.string != text, !textView.hasMarkedText() {
            textView.string = text
            textView.needsDisplay = true
        }
        if !textView.hasMarkedText() {
            if let request = mentionEditor.caretRequest, context.coordinator.lastCaret != request.revision {
                context.coordinator.lastCaret = request.revision
                if let previous = request.before {
                    let current = AppleMentionEditor.Snapshot(text: text, mentions: mentionEditor.mentions, caret: request.location)
                    context.coordinator.undoHistory.registerCompletion(textView.undoManager, before: previous, after: current) { [weak textView, weak coordinator = context.coordinator] expected, restored in
                        guard let textView, let coordinator, textView.string == expected.text else { return false }
                        coordinator.parent.mentionEditor.restore(restored.text, restored.mentions)
                        textView.string = restored.text
                        let range = NSRange(location: min(restored.caret, restored.text.utf16.count), length: 0)
                        textView.setSelectedRange(range)
                        textView.scrollRangeToVisible(range)
                        return true
                    }
                }
                textView.setSelectedRange(NSRange(location: min(request.location, textView.string.utf16.count), length: 0))
                textView.scrollRangeToVisible(textView.selectedRange())
            }
            if let layout = textView.layoutManager, context.coordinator.styledText != textView.string || context.coordinator.styledMentions != mentionEditor.mentions {
                context.coordinator.styledText = textView.string
                context.coordinator.styledMentions = mentionEditor.mentions
                layout.removeTemporaryAttribute(.foregroundColor, forCharacterRange: NSRange(location: 0, length: textView.string.utf16.count))
                let mentionAccent = NSColor(named: "AccentColor") ?? .controlAccentColor
                for mention in mentionEditor.mentions where NSMaxRange(mention.range) <= textView.string.utf16.count {
                    layout.addTemporaryAttributes([.foregroundColor: mentionAccent], forCharacterRange: mention.range)
                }
            }
        }
        if focused.wrappedValue, !context.coordinator.nativeFocusLostAwaitingPublication,
           textView.window?.firstResponder !== textView {
            textView.window?.makeFirstResponder(textView)
        }
    }

    func sizeThatFits(
        _ proposal: ProposedViewSize,
        nsView scrollView: NSScrollView,
        context: Context
    ) -> CGSize? {
        guard let textView = scrollView.documentView as? NSTextView else { return nil }
        let width = max(120, proposal.width ?? scrollView.bounds.width)
        let effectiveMaxHeight = proposal.height.map { min(resolvedMaxHeight, max(30, $0)) } ?? resolvedMaxHeight
        return MacComposerTextMeasurement.size(for: textView, width: width, maxHeight: effectiveMaxHeight)
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: MacComposerTextEditor
        var conversationID: String?
        var isReplacingConversation = false

        init(parent: MacComposerTextEditor) {
            self.parent = parent
        }

        var previousSelection: NSRange?
        var lastCaret: UUID?
        var styledText: String?
        var styledMentions: [AppleDraftMention] = []
        var undoHistory = AppleMentionUndoHistory()
        var pendingEdit: (String, NSRange, String)?
        func textView(_ textView: NSTextView, shouldChangeTextIn affectedCharRange: NSRange, replacementString: String?) -> Bool {
            guard !isReplacingConversation, let replacementString else { return true }
            let range = parent.mentionEditor.deletionRange(affectedCharRange, replacement: replacementString)
            if range != affectedCharRange, !textView.hasMarkedText() {
                textView.insertText("", replacementRange: range)
                return false
            }
            pendingEdit = (textView.string, affectedCharRange, replacementString)
            return true
        }
        func textViewDidChangeSelection(_ notification: Notification) {
            guard !isReplacingConversation, let view = notification.object as? NSTextView else { return }
            let selection = view.selectedRange(), composing = view.hasMarkedText()
            let next = parent.mentionEditor.caretRange(selection, previous: previousSelection)
            previousSelection = next
            if !composing, next != selection, NSMaxRange(next) <= view.string.utf16.count { view.setSelectedRange(next); return }
            Task { @MainActor [weak self, weak view] in
                guard let view, view.selectedRange() == selection else { return }
                self?.parent.mentionEditor.selectionChanged(selection, composing)
            }
        }
        func textDidChange(_ notification: Notification) {
            guard !isReplacingConversation, let textView = notification.object as? NSTextView else { return }
            // Delegate edits originate from the native event, not a SwiftUI
            // update. Commit before Return can submit; deferred writes can
            // otherwise restore a stale draft after send clears it.
            // The binding getter may still read the previous display snapshot.
            // Do not use it to suppress a native edit: insert then delete can
            // return to that old value before SwiftUI renders either change.
            if let (old, range, replacement) = pendingEdit,
               (old as NSString).replacingCharacters(in: range, with: replacement) == textView.string {
                pendingEdit = nil
                if undoHistory.restore(before: old, after: textView.string, undo: textView.undoManager, editor: parent.mentionEditor) { return }
                if parent.mentionEditor.applyEdit(old, range, replacement) {
                    let original = parent.mentionEditor.mentions
                    undoHistory.record(before: old, after: textView.string, original: original,
                        current: AppleDraftMention.replacing(original, range: range, with: replacement))
                    return
                }
            }
            pendingEdit = nil
            parent.text = textView.string
        }

        private var focusRevision = 0
        var nativeFocusLostAwaitingPublication = false

        func invalidateFocusPublication() { focusRevision &+= 1 }

        func publishFocus(_ isFocused: Bool) {
            focusRevision &+= 1
            let revision = focusRevision
            nativeFocusLostAwaitingPublication = !isFocused
            Task { @MainActor [weak self] in
                await Task.yield()
                guard let self, self.focusRevision == revision else { return }
                self.parent.focused.wrappedValue = isFocused
                // Keep the native loss authoritative until SwiftUI observes the
                // false binding; streaming updates must never reclaim focus.
            }
        }

    }
}

private struct MacCircularComposerButtonStyle: ButtonStyle {
    var baseFill: Color = Color.primary.opacity(0.055)
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        MacCircularComposerButtonBody(
            label: configuration.label,
            baseFill: baseFill,
            isPressed: configuration.isPressed,
            isEnabled: isEnabled,
            reduceMotion: reduceMotion
        )
    }
}

private struct MacCircularComposerButtonBody<Label: View>: View {
    let label: Label
    let baseFill: Color
    let isPressed: Bool
    let isEnabled: Bool
    let reduceMotion: Bool
    @State private var isHovering = false

    var body: some View {
        label
            .frame(width: 30, height: 30)
            .background {
                Circle()
                    .fill(baseFill)
                    .overlay {
                        if isEnabled && isPressed {
                            Circle().fill(Color.primary.opacity(0.12))
                        } else if isEnabled && isHovering {
                            Circle().fill(Color.primary.opacity(0.06))
                        }
                    }
            }
            .contentShape(Circle())
            .opacity(isEnabled ? (isPressed ? 0.86 : 1) : 0.45)
            .onHover { isHovering = isEnabled && $0 }
            .onChange(of: isEnabled) { _, enabled in if !enabled { isHovering = false } }
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isHovering)
            .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.hover), value: isPressed)
    }
}

struct MacComposerView: View {
    let state: MacComposerDisplayState
    let actions: MacComposerActionSink
    var composerFocused: FocusState<Bool>.Binding
    var isModelConfigured = true
    var leadingControls: AnyView = AnyView(EmptyView())
    var trailingControls: AnyView = AnyView(EmptyView())
    var mentionCommand: (String) -> Bool = { _ in false }
    var mentionEditor = AppleMentionEditor()
    var localCommand: () -> Bool = { false }
    @State private var fileImporterPresented = false
    @State private var fileImporterConversationID: String?
    @State private var fileImportFailure: (conversationID: String, message: String)?
    @State private var dropTargeted = false
    @State private var voiceInput = MacVoiceInputService()
    @State private var isInputHovered = false
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast
    @State private var dictationDraft = MacDictationDraft()
    @State private var dictationActive = false
    @State private var dictationConversationID: String?
    @State private var attachmentThumbnails: [MacAttachment.ID: NSImage] = [:]
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 6) {
            HStack(spacing: 6) {
                leadingControls
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 6)
            .opacity(0.78)

            inputSurface
            if let notice = state.attachmentImportNotice {
                MacInlineNotice(
                    message: notice,
                    kind: .information,
                    dismiss: {
                        actions.perform(.dismissAttachmentImportNotice(conversationID: state.conversationID))
                    }
                )
                .accessibilityIdentifier("chat.attachments.import-notice")
            }
            if let failure = fileImportFailure, failure.conversationID == state.conversationID {
                MacInlineNotice(message: failure.message, kind: .failure, dismiss: { fileImportFailure = nil })
                    .accessibilityIdentifier("chat.attachments.import-error")
            }
            if let error = voiceInput.errorMessage {
                MacInlineNotice(message: error, kind: .failure, dismiss: { voiceInput.errorMessage = nil })
                    .accessibilityIdentifier("chat.dictation.error")
            }
            if state.permissionConfigurationState == .failed {
                HStack(spacing: 8) {
                    Text(state.permissionConfigurationState.title)
                        .font(.caption)
                        .foregroundStyle(MacDesignColor.warning)
                        .accessibilityIdentifier("mac-composer-permission-status")
                    Button(String(localized: "Refresh", table: "Chat")) {
                        actions.perform(.reloadPermissionConfiguration(conversationID: state.conversationID))
                    }
                    .controlSize(.small)
                    .accessibilityIdentifier("mac-composer-permission-refresh")
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 4)
            }
            if let reason = state.submissionBlockReason {
                Text(reason)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("chat.composer.submission-block-reason")
            }
        }
    }

    private var inputSurface: some View {
        VStack(spacing: 0) {
            attachmentTray
            inputRow
            bottomToolbar
        }
        .background(
            MacDesignColor.panel,
            in: RoundedRectangle(cornerRadius: 18, style: .continuous)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(composerStrokeColor, lineWidth: dropTargeted ? 1.5 : 1)
                .allowsHitTesting(false)
        }
        .shadow(
            color: Color.black.opacity(colorScheme == .dark ? 0.36 : 0.05),
            radius: colorScheme == .dark ? 16 : 8,
            y: colorScheme == .dark ? 6 : 2
        )
        .onHover { isInputHovered = $0 }
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: dropTargeted)
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: composerFocused.wrappedValue)
        .animation(reduceMotion ? nil : MacDesignTokens.Motion.responsive, value: isInputHovered)
        .fileImporter(
            isPresented: $fileImporterPresented,
            allowedContentTypes: [.item],
            allowsMultipleSelection: true
        ) { result in
            // The picker belongs to the draft that opened it, even if a
            // keyboard/menu navigation changes the selected conversation.
            guard let conversationID = fileImporterConversationID else { return }
            fileImporterConversationID = nil
            switch result {
            case .success(let urls):
                Task { @MainActor in
                    await Task.yield()
                    guard !Task.isCancelled else { return }
                    actions.perform(.importFiles(conversationID: conversationID, urls: urls))
                }
            case .failure(let error):
                let failure = error as NSError
                guard failure.domain != NSCocoaErrorDomain || failure.code != NSUserCancelledError else { return }
                fileImportFailure = (conversationID, error.localizedDescription)
            }
        }
        .onDrop(
            of: [UTType.fileURL.identifier, UTType.image.identifier],
            isTargeted: $dropTargeted
        ) { providers in
            actions.perform(.importDroppedAttachments(
                conversationID: state.conversationID,
                providers: providers
            ))
        }
        .onPasteCommand(of: [.fileURL, .png, .tiff, .image]) { _ in
            actions.perform(.importPasteboardAttachments(
                conversationID: state.conversationID,
                pasteboard: .general
            ))
        }
        .task(id: voiceInput.transcript) {
            let transcript = voiceInput.transcript
            guard !transcript.isEmpty else { return }
            await Task.yield()
            guard !Task.isCancelled, dictationActive,
                  dictationConversationID == state.conversationID else { return }
            guard let updated = dictationDraft.applying(transcript, currentDraft: state.draft, conversationID: state.conversationID) else {
                dictationActive = false
                voiceInput.stop()
                return
            }
            actions.perform(.updateDraft(conversationID: state.conversationID, value: updated))
        }
        .onChange(of: state.conversationID) { _, _ in
            dictationActive = false
            dictationConversationID = nil
            voiceInput.stop()
        }
        .onDisappear {
            dictationActive = false
            voiceInput.stop()
        }
    }

    @ViewBuilder
    private var attachmentTray: some View {
        if !state.attachments.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 7) {
                    ForEach(state.attachments) { attachment in
                        attachmentChip(attachment)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.top, 10)
            }
            .task(id: state.attachments.map(\.id)) {
                let currentIDs = Set(state.attachments.map(\.id))
                attachmentThumbnails = attachmentThumbnails.filter { currentIDs.contains($0.key) }
                let pending = state.attachments.filter { attachmentThumbnails[$0.id] == nil }
                guard !pending.isEmpty else { return }
                let targets = pending.map { (id: $0.id, url: $0.url) }
                let loaded: [(MacAttachment.ID, NSImage)] = await Task.detached(priority: .utility) {
                    targets.compactMap { item in
                        let accessed = item.url.startAccessingSecurityScopedResource()
                        defer { if accessed { item.url.stopAccessingSecurityScopedResource() } }
                        guard let image = NSImage(contentsOf: item.url) else { return nil }
                        return (item.id, image)
                    }
                }.value
                guard !Task.isCancelled else { return }
                for (id, image) in loaded where currentIDs.contains(id) {
                    attachmentThumbnails[id] = image
                }
            }
        }
    }

    private func attachmentChip(_ attachment: MacAttachment) -> some View {
        HStack(spacing: 7) {
            Button {
                MacQuickLookController.shared.preview([attachment.url])
            } label: {
                HStack(spacing: 7) {
                    if let image = attachmentThumbnails[attachment.id] {
                        Image(nsImage: image)
                            .resizable()
                            .scaledToFill()
                            .frame(width: 30, height: 30)
                            .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                            .overlay {
                                RoundedRectangle(cornerRadius: 6, style: .continuous)
                                    .stroke(Color.primary.opacity(0.10), lineWidth: 0.5)
                            }
                    } else {
                        Text(attachment.fileType)
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(Color.accentColor)
                            .frame(width: 30, height: 30)
                            .background(
                                Color.accentColor.opacity(0.08),
                                in: RoundedRectangle(cornerRadius: 6, style: .continuous)
                            )
                    }
                    Text(attachment.name)
                        .font(.system(size: 11.5, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    attachmentStatus(attachment)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(String(format: String(localized: "Quick Look %@"), attachment.name))
            Button {
                actions.perform(.removeAttachment(
                    conversationID: state.conversationID,
                    attachmentID: attachment.id
                ))
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .semibold))
            }
            .buttonStyle(MacInlineIconButtonStyle(size: 22))
            .help(String(format: String(localized: "Remove %@"), attachment.name))
            .accessibilityLabel(
                String(
                    format: String(localized: "Remove %@"),
                    attachment.name
                )
            )
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
        .frame(maxWidth: 180, alignment: .leading)
        .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(attachment.name), \(attachmentAccessibilityStatus(attachment))")
        .contextMenu {
            Button(String(localized: "Quick Look")) { MacQuickLookController.shared.preview([attachment.url]) }
            Button(String(localized: "Reveal in Finder")) { NSWorkspace.shared.activateFileViewerSelecting([attachment.url]) }
            Button(String(localized: "Open with Default App")) { NSWorkspace.shared.open(attachment.url) }
            Button(String(localized: "Copy Path")) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(attachment.url.path, forType: .string)
            }
            Divider()
            Button(String(localized: "Remove")) {
                actions.perform(.removeAttachment(
                    conversationID: state.conversationID,
                    attachmentID: attachment.id
                ))
            }
        }
    }

    @ViewBuilder
    private func attachmentStatus(_ attachment: MacAttachment) -> some View {
        switch attachment.status {
        case .extracting:
            ProgressView().controlSize(.mini)
        case .ready:
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 9))
                .foregroundStyle(MacDesignColor.success)
        case .failed:
            Image(systemName: "exclamationmark.circle.fill")
                .font(.system(size: 9))
                .foregroundStyle(MacDesignColor.warning)
                .help(attachment.errorMessage ?? String(localized: "Extraction failed"))
        }
    }

    private var inputRow: some View {
        HStack(alignment: .bottom, spacing: 9) {
            ZStack(alignment: .leading) {
                MacComposerTextEditor(
                    text: draftBinding,
                    conversationID: state.conversationID,
                    focused: composerFocused,
                    onPasteAttachments: {
                        actions.perform(.importPasteboardAttachments(
                            conversationID: state.conversationID,
                            pasteboard: .general
                        ))
                    },
                    onDropAttachments: {
                        actions.perform(.importPasteboardAttachments(
                            conversationID: state.conversationID,
                            pasteboard: $0
                        ))
                    },
                    onSubmit: {
                        if localCommand() { return }
                        guard isModelConfigured else { return }
                        // A native edit can commit and Return can arrive before
                        // SwiftUI renders the next display snapshot. The action
                        // sink validates current draft and conversation authority.
                        guard actions.perform(.submit(conversationID: state.conversationID)) else { return }
                        dictationActive = false
                        voiceInput.stop()
                    },
                    mentionCommand: mentionCommand, mentionEditor: mentionEditor
                )
                .focused(composerFocused)
                .accessibilityLabel(String(localized: "Message Nola"))
            }
            .frame(minHeight: 30, maxHeight: 132, alignment: .bottom)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.top, 12)
        .padding(.bottom, 8)
    }

    private var bottomToolbar: some View {
        HStack(alignment: .center, spacing: 6) {
            // Left: Attach button + Plan/Build mode toggle
            Button {
                fileImportFailure = nil
                fileImporterConversationID = state.conversationID
                fileImporterPresented = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 13.5, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(MacCircularComposerButtonStyle(
                baseFill: colorScheme == .dark ? Color.white.opacity(0.06) : Color.primary.opacity(0.055)
            ))
            .help("Attach Files")
            .accessibilityLabel(String(localized: "Attach Files"))

            planModeToggle

            Spacer(minLength: 6)

            // Right: Execution (Model picker, Reasoning effort, Permission mode, Mic, Send/Stop)
            HStack(alignment: .center, spacing: 4) {
                trailingControls
                reasoningMenu
                permissionModeMenu
                micButton
                sendOrStopButton
            }
        }
        .padding(.horizontal, 10)
        .padding(.bottom, 9)
        .padding(.top, 2)
    }

    private var micButton: some View {
        Button {
            if voiceInput.isRecording || voiceInput.isStarting {
                dictationActive = false
                voiceInput.stop()
            } else {
                dictationConversationID = state.conversationID
                dictationDraft = MacDictationDraft(draft: state.draft, conversationID: state.conversationID)
                dictationActive = true
                Task { await voiceInput.start() }
            }
        } label: {
            Image(systemName: voiceInput.isRecording ? "waveform" : voiceInput.isStarting ? "hourglass" : "mic")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(voiceInput.isRecording ? MacDesignColor.danger : Color.secondary)
        }
        .buttonStyle(MacCircularComposerButtonStyle(
            baseFill: voiceInput.isRecording ? MacDesignColor.danger.opacity(0.12) : Color.primary.opacity(0.045)
        ))
        .help(
            (voiceInput.isRecording || voiceInput.isStarting)
                ? String(localized: "Stop Dictation")
                : String(localized: "Start Dictation")
        )
        .accessibilityLabel(
            (voiceInput.isRecording || voiceInput.isStarting)
                ? String(localized: "Stop Dictation")
                : String(localized: "Start Dictation")
        )
    }

    @ViewBuilder
    private var sendOrStopButton: some View {
        let showsStopAction = state.isVisiblyRunning && !state.draft.hasPrefix("/")
        let isActiveAction = (state.canSend && isModelConfigured) || state.isVisiblyRunning
        Button {
            if localCommand() { return }
            if state.isVisiblyRunning {
                actions.perform(.stop(conversationID: state.conversationID))
            } else if state.canSend && isModelConfigured {
                dictationActive = false
                voiceInput.stop()
                actions.perform(.submit(conversationID: state.conversationID))
            }
        } label: {
            Image(systemName: showsStopAction ? "stop.fill" : "arrow.up")
                .contentTransition(reduceMotion ? .identity : .symbolEffect(.replace))
                .animation(reduceMotion ? nil : .easeOut(duration: MacDesignTokens.Motion.selection), value: showsStopAction)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(isActiveAction ? MacDesignColor.onPrimaryAction : Color.secondary)
        }
        .buttonStyle(MacCircularComposerButtonStyle(
            baseFill: isActiveAction ? MacDesignColor.primaryAction : Color.primary.opacity(0.07)
        ))
        .disabled(!state.draft.hasPrefix("/") && (state.isPrimaryActionDisabled || (!isModelConfigured && !state.isVisiblyRunning)))
        .accessibilityIdentifier(showsStopAction ? "mac-composer-stop" : "mac-composer-send")
        .help(
            showsStopAction
                ? String(localized: "Stop")
                : String(localized: "Send")
        )
        .accessibilityLabel(
            showsStopAction
                ? String(localized: "Stop")
                : String(localized: "Send")
        )
    }

    private var planModeToggle: some View {
        Button {
            actions.perform(.setPlanMode(conversationID: state.conversationID, !state.isPlanMode))
        } label: {
            HStack(spacing: 5) {
                Image(systemName: state.isPlanMode ? "map.fill" : "hammer")
                    .font(.system(size: 11, weight: .semibold))
                Text(verbatim: state.isPlanMode ? "PLAN" : "Build")
                    .font(.system(size: 11, weight: state.isPlanMode ? .bold : .medium, design: state.isPlanMode ? .rounded : .default))
                    .tracking(state.isPlanMode ? 0.4 : 0)
            }
            .foregroundStyle(state.isPlanMode ? MacDesignColor.brand : Color.secondary)
            .padding(.horizontal, 9)
            .frame(height: 28)
            .macComposerPill(isActive: state.isPlanMode, tint: MacDesignColor.brand)
        }
        .buttonStyle(.plain)
        .fixedSize(horizontal: true, vertical: false)
        .disabled(state.isRunning || state.isSubmissionBlocked)
        .help(
            state.isPlanMode
                ? "Plan mode (read-only): inspects files and designs a plan without modifying your workspace. Click to switch to Build mode."
                : "Build mode: full editing and execution tools enabled with automatic turn checkpoints. Click to switch to read-only Plan mode."
        )
        .accessibilityLabel(state.isPlanMode ? "Plan Mode (Read-Only)" : "Build Mode")
        .accessibilityValue(state.isPlanMode ? "Plan" : "Build")
        .accessibilityIdentifier("mac-composer-plan-mode")
    }

    private var permissionModeMenu: some View {
        Menu {
            ForEach(MacPermissionMode.allCases) { mode in
                Button {
                    actions.perform(.selectPermissionMode(conversationID: state.conversationID, mode))
                } label: {
                    Label(mode.title, systemImage: state.permissionMode == mode ? "checkmark" : "shield")
                }
                .help(mode.detail)
            }
        } label: {
            Label(state.permissionMode.title, systemImage: "shield.lefthalf.filled")
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(state.permissionMode == .allowAll ? MacDesignColor.warning : Color.secondary)
        }
        .menuStyle(MacPillMenuStyle(height: 28))
        .fixedSize(horizontal: true, vertical: false)
        .help(state.activePermissionMode.map { "\(state.permissionMode.detail) (\(String(localized: "Permissions active", table: "Permissions")): \($0.title))" }
            ?? state.permissionMode.detail)
        .accessibilityLabel(String(localized: "Permission Mode"))
        .accessibilityIdentifier("mac-composer-permission-mode")
        .disabled(state.permissionConfigurationState == .saving)
    }

    @ViewBuilder
    private var reasoningMenu: some View {
        if !state.reasoningOptions.isEmpty {
            Menu {
                ForEach(["automatic"] + state.reasoningOptions, id: \.self) { preference in
                    Button {
                        actions.perform(.selectReasoningPreference(conversationID: state.conversationID, preference))
                    } label: {
                        if state.reasoningPreference == preference {
                            Label { Text(verbatim: reasoningTitle(preference)) } icon: { Image(systemName: "checkmark") }
                        } else { Text(verbatim: reasoningTitle(preference)) }
                    }
                }
            } label: {
                Label {
                    Text(verbatim: reasoningTitle(state.reasoningPreference))
                } icon: {
                    Image(systemName: "brain")
                }
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(.secondary)
            }
            .menuStyle(MacPillMenuStyle(height: 28))
            .fixedSize(horizontal: true, vertical: false)
            .accessibilityLabel(String(localized: "Reasoning", table: "ModelCenter"))
            .accessibilityIdentifier("mac-composer-reasoning")
            .disabled(state.isRunning || state.isSubmissionBlocked)
        }
    }

    private func reasoningTitle(_ preference: String) -> String {
        if Int(preference) != nil {
            return preference + " " + String(localized: "Tokens")
        }
        return NSLocalizedString("reasoning.effort." + preference, tableName: "Reasoning", value: preference, comment: "Thinking effort")
    }

    private var draftBinding: Binding<String> {
        Binding(
            get: { state.draft },
            set: {
                actions.perform(.updateDraft(conversationID: state.conversationID, value: $0))
            }
        )
    }

    private var composerStrokeColor: Color {
        if dropTargeted { return MacDesignColor.brand }
        if state.isPlanMode { return MacDesignColor.brand.opacity(colorScheme == .dark ? 0.55 : 0.45) }
        if colorScheme == .dark {
            if contrast == .increased { return Color.white.opacity(composerFocused.wrappedValue ? 0.50 : 0.28) }
            if composerFocused.wrappedValue { return Color.white.opacity(0.22) }
            return isInputHovered ? Color.white.opacity(0.15) : Color.white.opacity(0.09)
        }
        if composerFocused.wrappedValue { return Color.primary.opacity(0.28) }
        return contrast == .increased || isInputHovered ? MacDesignColor.controlOutline : MacDesignColor.borderSubtle
    }

    private func attachmentAccessibilityStatus(_ attachment: MacAttachment) -> String {
        switch attachment.status {
        case .extracting: return String(localized: "Processing")
        case .ready: return String(localized: "Ready")
        case .failed: return attachment.errorMessage ?? String(localized: "Failed")
        }
    }
}

/// A recognition session owns only its appended range. User edits revoke that
/// ownership rather than being overwritten by a later partial recognition.
nonisolated struct MacDictationDraft {
    private let original: String
    private let conversationID: String
    private(set) var lastApplied: String

    init(draft: String = "", conversationID: String = "") {
        self.conversationID = conversationID
        original = draft
        lastApplied = draft
    }

    mutating func applying(_ transcript: String, currentDraft: String, conversationID: String = "") -> String? {
        guard self.conversationID == conversationID, currentDraft == lastApplied else { return nil }
        let separator = original.isEmpty || original.last?.isWhitespace == true ? "" : " "
        lastApplied = original + separator + transcript
        return lastApplied
    }
}
