// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import Combine

@MainActor final class MacSiwcSignInModel: ObservableObject {
    @Published var flow: MacSiwcFlow?
    @Published var error: String?
    @Published var running = false
    @Published var cancelling = false
    private var bridge: (any MacSiwcAccountTransport)?
    private let makeBridge: (MacApplicationPort) throws -> any MacSiwcAccountTransport
    private var task: Task<Void, Never>?
    private var generation = UUID()
    let accountID: String
    let revision: Int64?

    init(account: MacModelAccountSnapshot?, makeBridge: @escaping (MacApplicationPort) throws -> any MacSiwcAccountTransport = { try MacSiwcAccountBridge(application: $0) }) {
        accountID = account?.id ?? UUID().uuidString.lowercased()
        revision = account?.revision
        self.makeBridge = makeBridge
    }
    func begin(application: MacApplicationPort, name: String) {
        guard task == nil, bridge == nil else { return }
        let generation = generation
        running = true; error = nil
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
                    if value.phase == "failed" { error = MacSiwcBridgeFailure.sanitized(value.error).localizedDescription }
                    guard ["preparing", "awaiting_authorization"].contains(value.phase) else { break }
                    try await Task.sleep(for: .seconds(1))
                    value = try await bridge.update("status")
                }
            } catch {
                if self.generation == generation, !Task.isCancelled {
                    self.error = MacSiwcBridgeFailure.message(error)
                }
            }
            if self.generation == generation { running = false; task = nil }
        }
    }
    func finish() {
        guard !running, flow?.phase == "authorized", let bridge else { return }
        running = true
        let generation = generation
        task = Task {
            do {
                let result = try await bridge.update("finish")
                guard generation == self.generation, !Task.isCancelled else { return }
                flow = result
            } catch {
                if generation == self.generation, !Task.isCancelled {
                    self.error = MacSiwcBridgeFailure.message(error)
                }
            }
            if generation == self.generation { running = false; task = nil }
        }
    }
    func cancel() {
        guard !cancelling, flow?.phase != "committed" else { return }
        generation = UUID(); task?.cancel(); task = nil
        cancelling = true; running = true
        guard let bridge else { cancelling = false; running = false; return }
        let generation = generation
        task = Task {
            do {
                let result = try await bridge.update("cancel")
                guard generation == self.generation else { return }
                flow = result
            } catch {
                if generation == self.generation { self.error = String(localized: "Cancellation could not be confirmed. Private authorization may remain; this does not revoke access in ChatGPT.") }
            }
            bridge.close()
            if generation == self.generation { cancelling = false; running = false; task = nil }
        }
    }
    func close() {
        generation = UUID(); task?.cancel(); task = nil
        bridge?.close(); bridge = nil
    }
}

struct MacSiwcSignInView: View {
    let application: MacApplicationPort
    var saved: (() -> Void)?
    @StateObject private var model: MacSiwcSignInModel
    @State private var name: String
    @State private var consent = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    init(application: MacApplicationPort, account: MacModelAccountSnapshot?, saved: (() -> Void)? = nil) {
        self.saved = saved
        self.application = application
        _model = StateObject(wrappedValue: MacSiwcSignInModel(account: account))
        _name = State(initialValue: account?.account.display_name ?? "ChatGPT")
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(String(localized: "Authorize ChatGPT for This Mac", table: "ModelCenter")).font(.title2.bold())
            HStack(spacing: 12) {
                step("1", String(localized: "Start Authorization", table: "ModelCenter"), complete: model.flow?.phase == "authorized" || model.flow?.phase == "committed")
                step("2", String(localized: "Save Account on This Mac", table: "ModelCenter"), complete: model.flow?.phase == "committed")
                step("3", String(localized: "Choose models", table: "ModelCenter"), complete: false)
            }
            .padding(12).background(MacDesignColor.chrome, in: RoundedRectangle(cornerRadius: 10))
            Text(String(localized: "This is Nola’s independent authorization, not your Codex login or a paired Host account. Credentials stay with this Mac’s private runtime.", table: "ModelCenter"))
                .fixedSize(horizontal: false, vertical: true)
            Text(String(localized: "Signing in does not run a model, discover models or enable a subscription route. Eligibility and available models are determined by the service.", table: "ModelCenter"))
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if model.flow == nil, !model.running, model.error == nil {
                TextField(String(localized: "Account label", table: "ModelCenter"), text: $name)
                Toggle(String(localized: "I want to authorize Nola for this Mac in my browser", table: "ModelCenter"), isOn: $consent)
                Button(String(localized: "Start Authorization", table: "ModelCenter")) { model.begin(application: application, name: name.trimmingCharacters(in: .whitespacesAndNewlines)) }
                    .buttonStyle(.borderedProminent)
                    .disabled(!consent || name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            if let flow = model.flow {
                Text(phaseLabel(flow.phase)).font(.headline)
                if flow.phase == "awaiting_authorization", let raw = flow.authorization_url,
                   let url = MacSiwcAccountBridge.authorizationURL(raw) {
                    Button(String(localized: "Continue in Browser", table: "ModelCenter")) { openURL(url) }
                        .buttonStyle(.borderedProminent)
                        .disabled(model.cancelling)
                    Text(String(localized: "Complete the request in your browser, then return here. Closing this sheet stops Nola’s pending flow, not access already granted at the source.", table: "ModelCenter"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                if flow.phase == "authorized" {
                    Text(String(localized: "Authorization was verified. Save it to this Mac’s account store to finish.", table: "ModelCenter"))
                    Button(String(localized: "Save Account on This Mac", table: "ModelCenter")) { model.finish() }
                        .buttonStyle(.borderedProminent).disabled(model.running)
                }
                if flow.private_session_may_be_retained {
                    Text(String(localized: "Private authorization may remain. This action does not revoke access in ChatGPT settings.", table: "ModelCenter"))
                        .font(.callout).foregroundStyle(.secondary)
                }
                if flow.phase == "committed" {
                    Text(String(localized: "Account saved. No model was called and no models were enabled.", table: "ModelCenter"))
                    Text(String(localized: "Next, discover the models available to this account and choose which to enable. No new sign-in is needed.", table: "ModelCenter"))
                        .foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    Button(String(localized: "Continue to model setup", table: "ModelCenter")) {
                        saved?(); model.close(); dismiss()
                    }.buttonStyle(.borderedProminent)
                }
            }
            if model.running { ProgressView(model.cancelling ? String(localized: "Cancelling Authorization…", table: "ModelCenter") : model.flow?.phase == "authorized" ? String(localized: "Saving account…", table: "ModelCenter") : String(localized: "Waiting for Authorization…", table: "ModelCenter")).controlSize(.small) }
            if let error = model.error { Text(error).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true) }
            HStack {
                if model.flow?.phase != "committed", model.flow != nil || model.running {
                    Button(String(localized: "Cancel Authorization", table: "ModelCenter")) { model.cancel() }.disabled(model.cancelling)
                }
                Spacer()
                Button(String(localized: "Close", table: "ModelCenter")) { model.close(); dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(24).frame(width: 480)
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: model.flow?.phase)
        .onDisappear { model.close() }
    }
    private func step(_ number: String, _ title: String, complete: Bool) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(verbatim: number).font(.caption.monospacedDigit())
                .foregroundStyle(complete ? Color.primary : Color.secondary)
            Text(verbatim: title).font(.caption).fixedSize(horizontal: false, vertical: true)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    private func phaseLabel(_ phase: String) -> String {
        switch phase {
        case "preparing": String(localized: "Preparing Authorization", table: "ModelCenter")
        case "awaiting_authorization": String(localized: "Waiting for Your Browser", table: "ModelCenter")
        case "authorized": String(localized: "Authorization Verified", table: "ModelCenter")
        case "committed": String(localized: "Account Saved", table: "ModelCenter")
        case "cancelled": String(localized: "Authorization Cancelled Locally", table: "ModelCenter")
        default: String(localized: "Authorization Was Not Completed", table: "ModelCenter")
        }
    }
}
