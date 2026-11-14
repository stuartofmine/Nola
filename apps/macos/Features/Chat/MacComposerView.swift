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
        if isReturn, editingModifiers.isEmpty, !hasMarkedText() {
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
    static func size(for textView: NSTextView, width: CGFloat) -> CGSize {
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
        return CGSize(width: width, height: min(132, max(30, height)))
    }
}

private struct MacComposerTextEditor: NSViewRepresentable {
    @Binding var text: String
    let conversationID: String
    let focused: FocusState<Bool>.Binding
    let onPasteAttachments: () -> Bool
    let onDropAttachments: (NSPasteboard) -> Bool
    let onSubmit: () -> Void
    var mentionCommand: (String) -> Bool = { _ in false }

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
        textView.registerForDraggedTypes([.fileURL, .png, .tiff])
        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.importsGraphics = false
        textView.drawsBackground = false
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
        textView.placeholder = String(localized: "Message Nola")
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
        textView.submitHandler = onSubmit
        textView.mentionHandler = mentionCommand
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
            textView.string = text
            textView.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
            textView.needsDisplay = true
            context.coordinator.isReplacingConversation = false
        }
        if !focused.wrappedValue { context.coordinator.nativeFocusLostAwaitingPublication = false }
        textView.attachmentPasteHandler = onPasteAttachments
        textView.attachmentDropHandler = onDropAttachments
        textView.submitHandler = onSubmit
        textView.mentionHandler = mentionCommand
        textView.focusChangeHandler = { [weak coordinator = context.coordinator] in
            coordinator?.publishFocus($0)
        }
        if textView.string != text, !textView.hasMarkedText() {
            textView.string = text
            textView.needsDisplay = true
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
        return MacComposerTextMeasurement.size(for: textView, width: width)
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: MacComposerTextEditor
        var conversationID: String?
        var isReplacingConversation = false

        init(parent: MacComposerTextEditor) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard !isReplacingConversation, let textView = notification.object as? NSTextView else { return }
            // Delegate edits originate from the native event, not a SwiftUI
            // update. Commit before Return can submit; deferred writes can
            // otherwise restore a stale draft after send clears it.
            // The binding getter may still read the previous display snapshot.
            // Do not use it to suppress a native edit: insert then delete can
            // return to that old value before SwiftUI renders either change.
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

struct MacComposerView: View {
    let state: MacComposerDisplayState
    let actions: MacComposerActionSink
    var composerFocused: FocusState<Bool>.Binding
    var isModelConfigured = true
    var mentionCommand: (String) -> Bool = { _ in false }
    var localCommand: () -> Bool = { false }
    @State private var fileImporterPresented = false
    @State private var fileImporterConversationID: String?
    @State private var fileImportFailure: (conversationID: String, message: String)?
    @State private var dropTargeted = false
    @State private var voiceInput = MacVoiceInputService()
    @State private var isSendHovering = false
    @State private var dictationDraft = MacDictationDraft()
    @State private var dictationActive = false
    @State private var dictationConversationID: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 8) {
            inputSurface
            if let failure = fileImportFailure, failure.conversationID == state.conversationID {
                MacInlineNotice(message: failure.message, kind: .failure, dismiss: { fileImportFailure = nil })
                    .accessibilityIdentifier("chat.attachments.import-error")
            }
            if let error = voiceInput.errorMessage {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("chat.dictation.error")
            }
            quickBar
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
        }
        .background(NolaSurfaceBackground(fill: MacDesignColor.surfaceRaised,
                                          border: MacDesignColor.borderSubtle,
                                          cornerRadius: MacDesignTokens.Radius.message))
        .overlay {
            RoundedRectangle(cornerRadius: MacDesignTokens.Radius.message, style: .continuous)
                .strokeBorder(composerStrokeColor, lineWidth: dropTargeted || composerFocused.wrappedValue ? 2 : 1)
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: dropTargeted)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: composerFocused.wrappedValue)
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
        }
    }

    private func attachmentChip(_ attachment: MacAttachment) -> some View {
        HStack(spacing: 7) {
            if let image = NSImage(contentsOf: attachment.url) {
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
            attachmentStatus(attachment)
            Button {
                actions.perform(.removeAttachment(
                    conversationID: state.conversationID,
                    attachmentID: attachment.id
                ))
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .semibold))
                    .frame(width: MacDesignTokens.Control.iconButton, height: MacDesignTokens.Control.iconButton)
                    .contentShape(Rectangle())
            }
            .buttonStyle(MacActionButtonStyle(contentInsets: .init()))
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
        .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(attachment.name), \(attachmentAccessibilityStatus(attachment))")
        .contextMenu {
            Button("Quick Look") { MacQuickLookController.shared.preview([attachment.url]) }
            Button("Reveal in Finder") { NSWorkspace.shared.activateFileViewerSelecting([attachment.url]) }
            Divider()
            Button("Remove") {
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
                .foregroundStyle(.green)
        case .failed:
            Image(systemName: "exclamationmark.circle.fill")
                .font(.system(size: 9))
                .foregroundStyle(.orange)
                .help(attachment.errorMessage ?? String(localized: "Extraction failed"))
        }
    }

    private var inputRow: some View {
        HStack(alignment: .center, spacing: 9) {
            Button {
                fileImportFailure = nil
                fileImporterConversationID = state.conversationID
                fileImporterPresented = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 13.5, weight: .semibold))
                    .frame(width: 30, height: 30)
            }
            .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: .init()))
            .background(Color.primary.opacity(0.055), in: Circle())
            .help("Attach Files")
            .accessibilityLabel(String(localized: "Attach Files"))

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
                    mentionCommand: mentionCommand
                )
                .focused(composerFocused)
                .accessibilityLabel(String(localized: "Message Nola"))
            }
            .frame(minHeight: 30, maxHeight: 132, alignment: .center)
            .fixedSize(horizontal: false, vertical: true)

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
                    .foregroundStyle(voiceInput.isRecording ? Color.red : Color.secondary)
                    .frame(width: 30, height: 30)
                    .background {
                        Circle().fill(voiceInput.isRecording ? Color.red.opacity(0.10) : Color.primary.opacity(0.045))
                    }
            }
            .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: .init()))
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
                Image(systemName: (state.isVisiblyRunning && !state.draft.hasPrefix("/")) ? "stop.fill" : "arrow.up")
                    .contentTransition(.opacity)
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.16), value: state.isVisiblyRunning)
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle((state.canSend && isModelConfigured) || state.isVisiblyRunning ? MacDesignColor.onPrimaryAction : Color.secondary)
                    .frame(width: 30, height: 30)
                    .background {
                        Circle().fill((state.canSend && isModelConfigured) || state.isVisiblyRunning ? MacDesignColor.primaryAction : Color.primary.opacity(0.07))
                    }
            }
            .buttonStyle(MacPressFeedbackButtonStyle(contentInsets: .init()))
            .disabled(!state.draft.hasPrefix("/") && (state.isPrimaryActionDisabled || (!isModelConfigured && !state.isVisiblyRunning)))
            .accessibilityIdentifier(state.isVisiblyRunning ? "mac-composer-stop" : "mac-composer-send")
            .opacity(isSendHovering && ((state.canSend && isModelConfigured) || state.isVisiblyRunning) ? 0.85 : 1)
            .onHover { hovering in
                guard isSendHovering != hovering else { return }
                if reduceMotion {
                    isSendHovering = hovering
                } else {
                    withAnimation(.easeOut(duration: 0.14)) {
                        isSendHovering = hovering
                    }
                }
            }
            .help(
                state.isVisiblyRunning
                    ? String(localized: "Stop")
                    : String(localized: "Send")
            )
            .accessibilityLabel(
                state.isVisiblyRunning
                    ? String(localized: "Stop")
                    : String(localized: "Send")
            )
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    private var quickBar: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) { quickBarControls; Spacer(minLength: 0) }
                .fixedSize(horizontal: true, vertical: false)
            VStack(alignment: .leading, spacing: 4) { quickBarControls }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .menuStyle(.borderlessButton)
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(.secondary)
        .padding(.horizontal, 4)
    }

    @ViewBuilder private var quickBarControls: some View {
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
            }
            .help(state.permissionMode.detail)
            .accessibilityLabel(String(localized: "Permission Mode"))
            .accessibilityIdentifier("mac-composer-permission-mode")
            .disabled(state.permissionConfigurationState == .saving)
            .frame(minHeight: 28)

            Text(state.permissionConfigurationState.title)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("mac-composer-permission-status")
                .help(state.activePermissionMode.map { "\(String(localized: "Permissions active", table: "Permissions")): \($0.title)" }
                    ?? state.permissionConfigurationState.title)
            if state.permissionConfigurationState == .failed {
                Button(String(localized: "Refresh", table: "Chat")) {
                    actions.perform(.reloadPermissionConfiguration(conversationID: state.conversationID))
                }
                .frame(minHeight: 28)
                .accessibilityIdentifier("mac-composer-permission-refresh")
            }

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
                    Label { Text(verbatim: reasoningTitle(state.reasoningPreference)) }
                    icon: { Image(systemName: "brain") }
                }
                .menuStyle(.borderlessButton)
                .accessibilityLabel(String(localized: "Reasoning", table: "ModelCenter"))
                .accessibilityIdentifier("mac-composer-reasoning")
                .disabled(state.isRunning || state.isSubmissionBlocked)
                .frame(minHeight: 28)
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
        if composerFocused.wrappedValue { return MacDesignColor.brand }
        return MacDesignColor.controlOutline
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
