// SPDX-License-Identifier: Apache-2.0
// Restore native interactive navigation when the chat uses its compact header.
import SwiftUI
import UIKit

struct ChatNavigationGestureBridge: UIViewControllerRepresentable {
    var isEnabled: Bool = true

    func makeUIViewController(context: Context) -> Controller {
        let controller = Controller()
        controller.gestureEnabled = isEnabled
        return controller
    }

    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.gestureEnabled = isEnabled
    }

    final class Controller: UIViewController, UIGestureRecognizerDelegate {
        var gestureEnabled: Bool = true {
            didSet {
                attachIfNeeded()
                recognizer?.isEnabled = gestureEnabled
            }
        }
        private weak var recognizer: UIGestureRecognizer?
        private weak var originalDelegate: (any UIGestureRecognizerDelegate)?
        private var originallyEnabled = false

        override func loadView() {
            view = UIView()
            view.isUserInteractionEnabled = false
        }

        override func didMove(toParent parent: UIViewController?) {
            super.didMove(toParent: parent)
            if parent != nil {
                attachIfNeeded()
            }
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            attachIfNeeded()
        }

        override func viewDidDisappear(_ animated: Bool) {
            super.viewDidDisappear(animated)
            restoreGesture()
        }

        private func attachIfNeeded() {
            guard let gesture = navigationController?.interactivePopGestureRecognizer else { return }
            if gesture.delegate !== self {
                recognizer = gesture
                originalDelegate = gesture.delegate
                originallyEnabled = gesture.isEnabled
                gesture.delegate = self
            }
            gesture.isEnabled = gestureEnabled
        }

        private func restoreGesture() {
            guard let recognizer, recognizer.delegate === self else { return }
            recognizer.delegate = originalDelegate
            recognizer.isEnabled = originallyEnabled
            self.recognizer = nil
        }

        func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
            guard gestureEnabled, let navigationController else { return false }
            return navigationController.viewControllers.count > 1
                && navigationController.transitionCoordinator == nil
                && navigationController.presentedViewController == nil
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldBeRequiredToFailBy otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            gestureEnabled && gestureRecognizer === recognizer
        }
    }
}
