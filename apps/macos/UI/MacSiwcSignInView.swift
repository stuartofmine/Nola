// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import Combine
import AppKit

@MainActor final class MacSiwcSignInModel: ObservableObject {
    @Published var flow: MacSiwcFlow?
    @Published var error: String?
    @Published var running = false
    @Published var cancelling = false
    @Published var browserOpenFailed = false
    @Published var connectionID: String?
    @Published var discoveringModels = false
    @Published var cancellationConfirmed = false
    @Published var needsReauthorization = false
    @Published var retrySignInAllowed = false
    private var openedFlowID: String?
    private let openBrowser: (URL) -> Bool
    private let pollInterval: Duration
    private var bridge: (any MacSiwcAccountTransport)?
    private var application: MacApplicationPort?
    private let makeBridge: (MacApplicationPort) throws -> any MacSiwcAccountTransport
    private var task: Task<Void, Never>?
    private var generation = UUID()
    let accountID: String
    let revision: Int64?

    init(account: MacModelAccountSnapshot?, openBrowser: @escaping (URL) -> Bool = { NSWorkspace.shared.open($0) }, pollInterval: Duration = .seconds(1), makeBridge: @escaping (MacApplicationPort) throws -> any MacSiwcAccountTransport = { try MacSiwcAccountBridge(application: $0) }) {
        accountID = account?.id ?? UUID().uuidString.lowercased()
        revision = account?.revision
        self.makeBridge = makeBridge
        self.openBrowser = openBrowser
        self.pollInterval = pollInterval
    }
    var requiresConfirmedDismissal: Bool {
        running || cancelling || ["preparing", "awaiting_authorization", "authorized"].contains(flow?.phase ?? "")
    }

    func begin(application: MacApplicationPort, name: String) {
        guard task == nil, bridge == nil else { return }
        let generation = generation
        self.application = application
        running = true; error = nil; retrySignInAllowed = false
        task = Task {
            do {
                try Task.checkCancellation()
                let bridge = try makeBridge(application)
                self.bridge = bridge
                var value = try await bridge.start(accountID: accountID, revision: revision, name: name)
                while true {
                    try Task.checkCancellation()
                    guard self.generation == generation else { return }
                    flow = value
                    if value.phase == "awaiting_authorization", let raw = value.authorization_url,
                       MacSiwcAccountBridge.authorizationURL(raw) != nil, openedFlowID != value.flow_id {
                        openedFlowID = value.flow_id
                        openAuthorizationPage()
                    }
                    if value.phase == "authorized" {
                        // The explicit Sign In action authorizes this save. Do
                        // not ask for a second confirmation of the same intent.
                        value = try await bridge.update("finish")
                        guard ["committed", "failed"].contains(value.phase) else { throw MacSiwcBridgeFailure.unavailable }
                        try Task.checkCancellation()
                        guard self.generation == generation else { return }
                        flow = value
                    }
                    if value.phase == "failed" { error = MacSiwcBridgeFailure.sanitized(value.error).localizedDescription }
                    guard ["preparing", "awaiting_authorization"].contains(value.phase) else { break }
                    try await Task.sleep(for: pollInterval)
                    value = try await bridge.update("status")
                }
            } catch {
                if self.generation == generation, !Task.isCancelled {
                    self.error = MacSiwcBridgeFailure.message(error)
                    if flow == nil {
                        self.bridge?.close(); self.bridge = nil
                        if let failure = error as? MacSiwcBridgeFailure {
                            switch failure {
                            case .authorizationDeclined, .authorizationExpired, .authorizationNetwork: retrySignInAllowed = true
                            default: break
                            }
                        }
                    }
                }
            }
            if self.generation == generation { running = false; task = nil }
        }
    }
    func openAuthorizationPage() {
        guard !cancelling, let raw = flow?.authorization_url,
              let url = MacSiwcAccountBridge.authorizationURL(raw) else { return }
        browserOpenFailed = !openBrowser(url)
    }
    func chooseModels() {
        guard !running, let account = flow?.account, flow?.phase == "committed", let application else { return }
        running = true; discoveringModels = true; error = nil; needsReauthorization = false
        let generation = generation
        task = Task {
            do {
                // A failed exchange closes its private pipe. Each explicit
                // catalog attempt gets a fresh owner and fresh revisions.
                let catalog = try makeBridge(application)
                defer { catalog.close() }
                let id = try await catalog.discoverModels(account: account)
                try Task.checkCancellation()
                guard self.generation == generation else { return }
                connectionID = id
            } catch {
                if generation == self.generation, !Task.isCancelled {
                    self.error = MacSiwcBridgeFailure.message(error)
                    if let failure = error as? MacSiwcBridgeFailure {
                        switch failure { case .reauthorize, .consent: needsReauthorization = true; default: break }
                    }
                }
            }
            if generation == self.generation { running = false; discoveringModels = false; task = nil }
        }
    }
    func restart(application: MacApplicationPort, name: String) {
        guard !running, ["failed", "cancelled"].contains(flow?.phase ?? "failed"),
              flow?.private_session_may_be_retained != true else { return }
        close(); flow = nil; error = nil; openedFlowID = nil; browserOpenFailed = false
        begin(application: application, name: name)
    }
    func cancel() {
        guard !cancelling, flow?.phase != "committed" else { return }
        generation = UUID(); task?.cancel(); task = nil
        cancelling = true; running = true
        guard let bridge else { cancelling = false; running = false; cancellationConfirmed = true; return }
        let generation = generation
        task = Task {
            do {
                let result = try await bridge.update("cancel")
                guard generation == self.generation else { return }
                flow = result
                cancellationConfirmed = result.phase == "cancelled" && !result.private_session_may_be_retained
            } catch {
                if generation == self.generation { self.error = String(localized: "Cancellation could not be confirmed. Private authorization may remain; this does not revoke access in ChatGPT.") }
            }
            bridge.close()
            if generation == self.generation { cancelling = false; running = false; task = nil }
        }
    }
    func close() {
        generation = UUID(); task?.cancel(); task = nil
        running = false; cancelling = false; discoveringModels = false
        bridge?.close(); bridge = nil; application = nil
    }
}

struct MacSiwcSignInView: View {
    let application: MacApplicationPort
    var saved: ((String) -> Void)?
    var showAccounts: (() -> Void)?
    @StateObject private var model: MacSiwcSignInModel
    @State private var name: String
    @State private var copiedLink = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dismiss) private var dismiss

    init(application: MacApplicationPort, account: MacModelAccountSnapshot?, model: MacSiwcSignInModel? = nil, showAccounts: (() -> Void)? = nil, saved: ((String) -> Void)? = nil) {
        self.saved = saved
        self.showAccounts = showAccounts
        self.application = application
        _model = StateObject(wrappedValue: model ?? MacSiwcSignInModel(account: account))
        _name = State(initialValue: account?.account.display_name ?? "ChatGPT")
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 8) {
                Text(String(localized: "Sign in with ChatGPT", table: "ModelCenter")).font(.title2.bold())
                Text(String(localized: "Sign in in your browser, then choose the models you want to use in Nola.", table: "ModelCenter"))
                    .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            content
            if let error = model.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                if model.flow?.phase == "committed" || (!model.running && model.error != nil) || model.flow?.phase == "cancelled" {
                    Button(String(localized: "Done", table: "ModelCenter")) { model.close(); dismiss() }
                        .disabled(model.running)
                } else {
                    Button(String(localized: "Cancel", table: "ModelCenter")) {
                        if model.flow == nil, !model.running { dismiss() }
                        else { model.cancel() }
                    }.keyboardShortcut(.cancelAction)
                    .disabled(model.cancelling || model.flow?.phase == "authorized")
                }
                Spacer()
                primaryAction
            }
        }
        .padding(24).frame(width: 460)
        .tint(MacDesignColor.brand)
        .interactiveDismissDisabled(model.requiresConfirmedDismissal)
        .background(MacEditorWindowCloseGuard(
            shouldPreventClose: { model.requiresConfirmedDismissal },
            onCloseAttempt: {
                // Match the visible Cancel action. Finishing authorization and
                // catalog work must settle before this sheet can disappear.
                guard model.flow?.phase != "authorized", model.flow?.phase != "committed" else { return }
                model.cancel()
            }
        ))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: model.flow?.phase)
        .onChange(of: model.connectionID) { _, id in
            guard let id else { return }
            saved?(id); model.close(); dismiss()
        }
        .onChange(of: model.cancellationConfirmed) { _, confirmed in
            if confirmed { model.close(); dismiss() }
        }
        .onDisappear { model.close() }
    }
    @ViewBuilder private var content: some View {
        if model.flow == nil, !model.running, model.error == nil {
            Text(String(localized: "Your browser will open automatically. After you approve access, Nola saves this account on this Mac.", table: "ModelCenter"))
                .fixedSize(horizontal: false, vertical: true)
            DisclosureGroup(String(localized: "Account name", table: "ModelCenter")) {
                TextField(String(localized: "Account label", table: "ModelCenter"), text: $name)
                    .textFieldStyle(.roundedBorder).padding(.top, 8)
            }
            Text(String(localized: "This signs Nola in separately from Codex. Models are enabled only after you choose them.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        } else if model.flow?.phase == "committed" {
            Label(String(localized: "ChatGPT account connected", table: "ModelCenter"), systemImage: "checkmark.circle")
                .font(.headline)
            Text(String(localized: "Choose models to load the available list. You can review and enable them on the next screen.", table: "ModelCenter"))
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if model.discoveringModels { ProgressView(String(localized: "Loading models…", table: "ModelCenter")) }
        } else if model.flow?.phase == "awaiting_authorization", model.flow?.authorization_url != nil, model.error == nil {
            Label(String(localized: "Complete sign-in in your browser", table: "ModelCenter"), systemImage: "safari")
                .font(.headline)
            Text(String(localized: "Return here after approving access. Nola will finish connecting automatically.", table: "ModelCenter"))
                .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if model.browserOpenFailed {
                Text(String(localized: "The browser could not open. Try again, or copy the sign-in link into your browser.", table: "ModelCenter"))
                    .fixedSize(horizontal: false, vertical: true)
            }
            ViewThatFits(in: .horizontal) {
                HStack { browserActions }
                VStack(alignment: .leading) { browserActions }
            }.disabled(model.cancelling)
            ProgressView(String(localized: "Waiting for browser sign-in…", table: "ModelCenter")).controlSize(.small)
        } else if model.running {
            ProgressView(model.cancelling ? String(localized: "Cancelling Authorization…", table: "ModelCenter") :
                model.flow?.phase == "authorized" ? String(localized: "Saving account…", table: "ModelCenter") :
                String(localized: "Preparing sign-in…", table: "ModelCenter"))
        }
        if model.flow?.private_session_may_be_retained == true {
            Text(String(localized: "Private authorization may remain. This action does not revoke access in ChatGPT settings.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
        }
    }
    @ViewBuilder private var browserActions: some View {
                Button(String(localized: "Open browser again", table: "ModelCenter")) { model.openAuthorizationPage() }
                Button(copiedLink ? String(localized: "Link copied", table: "ModelCenter") : String(localized: "Copy sign-in link", table: "ModelCenter")) {
                    guard let raw = model.flow?.authorization_url, MacSiwcAccountBridge.authorizationURL(raw) != nil else { return }
                    NSPasteboard.general.clearContents()
                    copiedLink = NSPasteboard.general.setString(raw, forType: .string)
                }
    }
    @ViewBuilder private var primaryAction: some View {
        if model.flow == nil, !model.running, model.error == nil {
            Button(String(localized: "Sign in with ChatGPT", table: "ModelCenter")) {
                let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
                model.begin(application: application, name: label.isEmpty ? "ChatGPT" : label)
            }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                .accessibilityIdentifier("mac-chatgpt-sign-in")
        } else if model.flow?.phase == "committed", !model.needsReauthorization {
            Button(String(localized: "Choose models", table: "ModelCenter")) { model.chooseModels() }
                .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction).disabled(model.running)
                .accessibilityIdentifier("mac-chatgpt-choose-models")
        } else if !model.running, (model.flow?.phase == "failed" || model.retrySignInAllowed), model.flow?.private_session_may_be_retained != true {
            Button(String(localized: "Try sign-in again", table: "ModelCenter")) { model.restart(application: application, name: name) }
                .buttonStyle(.borderedProminent)
        } else if !model.running, model.error != nil || model.flow?.private_session_may_be_retained == true {
            Button(String(localized: "Check accounts", table: "ModelCenter")) { showAccounts?(); model.close(); dismiss() }
                .buttonStyle(.borderedProminent)
        }
    }
}
