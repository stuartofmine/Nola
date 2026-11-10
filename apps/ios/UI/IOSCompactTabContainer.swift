// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit

/// The compact shell owns its bottom navigation. Retain each visited root
/// without also creating a hidden system tab bar and its selection animations.
struct IOSCompactTabContainer<Tab: Hashable>: UIViewControllerRepresentable {
    let selection: Tab
    let content: (Tab) -> AnyView

    func makeUIViewController(context: Context) -> Controller { Controller() }

    func updateUIViewController(_ controller: Controller, context: Context) {
        controller.select(selection, content: content(selection))
    }

    final class Controller: UIViewController {
        private var roots: [Tab: UIHostingController<AnyView>] = [:]
        private var selected: Tab?

        override func loadView() {
            view = UIView()
            view.backgroundColor = .clear
        }

        func select(_ tab: Tab, content: AnyView) {
            let host: UIHostingController<AnyView>
            if let retained = roots[tab] {
                host = retained
                host.rootView = content
            } else {
                host = UIHostingController(rootView: content)
                host.view.backgroundColor = .clear
                // The surrounding SwiftUI shell already follows the keyboard.
                host.safeAreaRegions = .container
                roots[tab] = host
            }
            guard selected != tab else { return }
            loadViewIfNeeded()
            view.endEditing(true)
            for previous in children {
                previous.willMove(toParent: nil)
                previous.view.removeFromSuperview()
                previous.removeFromParent()
            }
            selected = tab
            addChild(host)
            host.view.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(host.view)
            NSLayoutConstraint.activate([
                host.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
                host.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
                host.view.topAnchor.constraint(equalTo: view.topAnchor),
                host.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            ])
            host.didMove(toParent: self)
        }
    }
}
