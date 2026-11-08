// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit

struct IOSRoomComposerField: UIViewRepresentable {
    @Binding var text: String
    @Binding var focused: Bool
    let label: String
    let mentionsVisible: Bool
    let command: (String) -> Bool
    let send: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeUIView(context: Context) -> IOSRoomComposerTextView {
        let input = IOSRoomComposerTextView()
        input.backgroundColor = .clear
        input.font = .preferredFont(forTextStyle: .body)
        input.adjustsFontForContentSizeCategory = true
        input.delegate = context.coordinator
        input.accessibilityIdentifier = "room.composer"
        return input
    }
    func updateUIView(_ input: IOSRoomComposerTextView, context: Context) {
        context.coordinator.parent = self
        input.accessibilityLabel = label
        input.mentionsVisible = mentionsVisible
        input.mentionCommand = command
        input.sendMessage = send
        if input.text != text, input.markedTextRange == nil { input.text = text }
        if focused, !input.isFirstResponder { input.becomeFirstResponder() }
    }
    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: IOSRoomComposerField
        init(_ parent: IOSRoomComposerField) { self.parent = parent }
        func textViewDidChange(_ textView: UITextView) { parent.text = textView.text }
        func textViewDidBeginEditing(_ textView: UITextView) { parent.focused = true }
        func textViewDidEndEditing(_ textView: UITextView) { parent.focused = false }
    }
}

final class IOSRoomComposerTextView: UITextView {
    var mentionsVisible = false
    var mentionCommand: (String) -> Bool = { _ in false }
    var sendMessage: () -> Void = {}
    override var keyCommands: [UIKeyCommand]? {
        guard markedTextRange == nil else { return super.keyCommands }
        var commands = [UIKeyCommand(input: "\r", modifierFlags: .command, action: #selector(sendFromKeyboard))]
        if mentionsVisible {
            for input in [UIKeyCommand.inputUpArrow, UIKeyCommand.inputDownArrow, "\r", "\t", UIKeyCommand.inputEscape] {
                let command = UIKeyCommand(input: input, modifierFlags: [], action: #selector(chooseMention(_:)))
                command.wantsPriorityOverSystemBehavior = true
                commands.append(command)
            }
        }
        return commands + (super.keyCommands ?? [])
    }
    @objc func chooseMention(_ key: UIKeyCommand) {
        guard markedTextRange == nil, mentionsVisible else { return }
        let action = key.input == UIKeyCommand.inputUpArrow ? "up"
            : key.input == UIKeyCommand.inputDownArrow ? "down"
            : key.input == UIKeyCommand.inputEscape ? "dismiss" : "accept"
        _ = mentionCommand(action)
    }
    @objc func sendFromKeyboard() { if markedTextRange == nil { sendMessage() } }
}
