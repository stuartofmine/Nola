// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit

/// Observe taps in this scene, including presented sheets. Leave the original
/// event with the control or scroll view that received it.
struct IOSKeyboardDismissal: UIViewRepresentable {
    func makeUIView(context: Context) -> ObserverView { ObserverView() }
    func updateUIView(_ view: ObserverView, context: Context) {}

    final class ObserverView: UIView, UIGestureRecognizerDelegate {
        private weak var observedWindow: UIWindow?
        private weak var editingView: UIView?
        private weak var touchedEditor: UIView?
        private lazy var tap: UITapGestureRecognizer = {
            let tap = UITapGestureRecognizer(target: self, action: #selector(tapped))
            tap.cancelsTouchesInView = false
            tap.delaysTouchesBegan = false
            tap.delaysTouchesEnded = false
            tap.delegate = self
            return tap
        }()

        override func didMoveToWindow() {
            super.didMoveToWindow()
            guard observedWindow !== window else { return }
            observedWindow?.removeGestureRecognizer(tap)
            NotificationCenter.default.removeObserver(self)
            observedWindow = window
            editingView = nil
            isUserInteractionEnabled = false
            guard let window else { return }
            window.addGestureRecognizer(tap)
            for name in [UITextField.textDidBeginEditingNotification, UITextView.textDidBeginEditingNotification] {
                NotificationCenter.default.addObserver(self, selector: #selector(editingBegan(_:)), name: name, object: nil)
            }
        }

        @objc private func editingBegan(_ notification: Notification) {
            guard let view = notification.object as? UIView, view.window === observedWindow else { return }
            editingView = view
        }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
            guard let editor = editingView, editor.isFirstResponder,
                  !editor.bounds.contains(touch.location(in: editor)) else { return false }
            var current = touch.view
            while let view = current {
                if view is UIControl || view is UITextInput || view === editor
                    || view is UINavigationBar || view is UITabBar || view is UIToolbar
                    || view.accessibilityTraits.contains(.button)
                    || view.accessibilityTraits.contains(.link)
                    || view.accessibilityTraits.contains(.adjustable)
                    || view.accessibilityTraits.contains(.searchField)
                    || view.accessibilityTraits.contains(.keyboardKey) {
                    return false
                }
                current = view.superview
            }
            if let window = observedWindow, let hitView = touch.view {
                let pointInWindow = touch.location(in: window)
                let screenPoint = window.convert(pointInWindow, to: nil)
                if containsInteractiveAccessibilityElement(in: hitView, at: screenPoint, depth: 0) {
                    return false
                }
            }
            touchedEditor = editor
            return true
        }

        private func containsInteractiveAccessibilityElement(in container: Any, at screenPoint: CGPoint, depth: Int) -> Bool {
            guard depth < 12 else { return false }
            let interactiveTraits: UIAccessibilityTraits = [
                .button, .link, .adjustable, .searchField, .keyboardKey, .allowsDirectInteraction
            ]
            if let element = container as? NSObject {
                if element.isAccessibilityElement {
                    let frame = element.accessibilityFrame
                    if !frame.isEmpty, frame.contains(screenPoint),
                       !element.accessibilityTraits.intersection(interactiveTraits).isEmpty {
                        return true
                    }
                }
                if let children = element.accessibilityElements {
                    for child in children {
                        if containsInteractiveAccessibilityElement(in: child, at: screenPoint, depth: depth + 1) {
                            return true
                        }
                    }
                }
            }
            return false
        }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                               shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer) -> Bool { true }

        @objc private func tapped() {
            guard let original = touchedEditor else { return }
            // Let SwiftUI deliver focus changes and button actions first.
            DispatchQueue.main.async { [weak original] in
                guard let original, original.isFirstResponder else { return }
                original.resignFirstResponder()
            }
        }
    }
}
