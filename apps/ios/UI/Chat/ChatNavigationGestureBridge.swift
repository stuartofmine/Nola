// SPDX-License-Identifier: Apache-2.0
// Restore native interactive navigation when the chat uses its compact header.
import SwiftUI
import UIKit

struct ChatNavigationGestureBridge: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> Controller { Controller() }
    func updateUIViewController(_ controller: Controller, context: Context) {}

    final class Controller: UIViewController, UIGestureRecognizerDelegate {
        private weak var recognizer: UIGestureRecognizer?
        private weak var originalDelegate: (any UIGestureRecognizerDelegate)?
        private var originallyEnabled = false

        override func loadView() {
            view = UIView()
            view.isUserInteractionEnabled = false
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            guard let gesture = navigationController?.interactivePopGestureRecognizer,
                  gesture.delegate !== self else { return }
            recognizer = gesture
            originalDelegate = gesture.delegate
            originallyEnabled = gesture.isEnabled
            gesture.delegate = self
            gesture.isEnabled = true
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            restoreGesture()
        }

        private func restoreGesture() {
            guard let recognizer, recognizer.delegate === self else { return }
            recognizer.delegate = originalDelegate
            recognizer.isEnabled = originallyEnabled
            self.recognizer = nil
        }

        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard let navigationController else { return false }
            return navigationController.viewControllers.count > 1
                && navigationController.transitionCoordinator == nil
                && navigationController.presentedViewController == nil
        }
    }
}
