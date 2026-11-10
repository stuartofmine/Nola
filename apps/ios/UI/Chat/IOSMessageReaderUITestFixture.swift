// SPDX-License-Identifier: Apache-2.0
// Display-only GUI fixture: no model, Run, permission or tool is executed.
#if DEBUG
import Combine
import SwiftUI
import UIKit

@MainActor
private final class IOSMessageReaderDisplayInputs: ObservableObject {
    enum Mutation { case prepend, owner }
    @Published var conversationID = UUID()
    @Published var message: ChatMessage
    @Published var status = "ready"
    private weak var window: UIWindow?
    private weak var root: UIViewController?
    private var epoch = UUID()
    private var mutation: Task<Void, Never>?

    init() throws { message = try Self.message(id: UUID(), output: "SNAPSHOT-A", prepend: false) }
    deinit { mutation?.cancel() }

    func attach(_ window: UIWindow) {
        self.window = window
        root = window.rootViewController
    }

    func cancel(detach: Bool) {
        epoch = UUID()
        mutation?.cancel(); mutation = nil
        if detach { root = nil; window = nil }
    }

    func arm(_ action: Mutation) {
        cancel(detach: false)
        guard let window, let root, window.rootViewController === root,
              presented(from: root).isEmpty else { status = "failed:arm-owner-or-cover"; return }
        let ticket = epoch, conversation = conversationID, messageID = message.id
        status = action == .prepend ? "armed-prepend" : "armed-owner"
        mutation = Task { @MainActor [weak self, weak window, weak root] in
            let deadline = Date().addingTimeInterval(6)
            while Date() < deadline {
                guard let self, let window, let root, !Task.isCancelled, self.epoch == ticket,
                      self.window === window, self.root === root, window.rootViewController === root,
                      self.conversationID == conversation, self.message.id == messageID else { return }
                let presentations = self.presented(from: root)
                if presentations.count > 1 { self.status = "failed:ambiguous-cover"; return }
                if let pair = presentations.first,
                   pair.cover.viewIfLoaded?.window === window, !pair.cover.isBeingPresented {
                    await self.mutate(action, presenter: pair.presenter, cover: pair.cover,
                        window: window, root: root, ticket: ticket, deadline: deadline)
                    return
                }
                try? await Task.sleep(for: .milliseconds(50))
            }
            if let self, self.epoch == ticket { self.status = "failed:no-presented-reader" }
        }
    }

    private func mutate(_ action: Mutation, presenter: UIViewController, cover: UIViewController,
                        window: UIWindow, root: UIViewController, ticket: UUID, deadline: Date) async {
        do {
            switch action {
            case .prepend:
                message = try Self.message(id: message.id, output: "SNAPSHOT-B", prepend: true)
                // Sample the real presented UIKit object while SwiftUI destroys
                // the original process:firstPart view. No binding is invoked.
                for _ in 0..<8 {
                    try await Task.sleep(for: .milliseconds(50))
                    guard !Task.isCancelled, epoch == ticket, self.window === window,
                          window.rootViewController === root else { return }
                    guard presenter.presentedViewController === cover,
                          cover.viewIfLoaded?.window === window, !cover.isBeingDismissed else {
                        status = "failed:reader-dismissed-on-regroup"; return
                    }
                }
                status = "prepend-verified-same-controller"
            case .owner:
                conversationID = UUID()
                message = try Self.message(id: UUID(), output: "OTHER-OWNER", prepend: false)
                while Date() < deadline {
                    guard !Task.isCancelled, epoch == ticket, self.window === window,
                          window.rootViewController === root else { return }
                    if presenter.presentedViewController == nil {
                        status = "owner-change-dismissed"; return
                    }
                    guard presenter.presentedViewController === cover else {
                        status = "failed:unexpected-replacement-cover"; return
                    }
                    try await Task.sleep(for: .milliseconds(50))
                }
                status = "failed:owner-retained-reader"
            }
        } catch {
            if !Task.isCancelled, epoch == ticket { status = "failed:display-input" }
        }
    }

    // Search only the fixture's own captured root hierarchy. No application-
    // global window selection, private SwiftUI types, or controller injection.
    private func presented(from controller: UIViewController) -> [(presenter: UIViewController, cover: UIViewController)] {
        if let cover = controller.presentedViewController { return [(controller, cover)] }
        return controller.children.flatMap { presented(from: $0) }
    }

    private static func message(id: UUID, output: String, prepend: Bool) throws -> ChatMessage {
        let tool: [String: Any] = ["kind": "tool", "execution_source": ["kind": "model"],
            "part_id": "reader-tool", "revision": prepend ? 2 : 1,
            "tool_call_id": "reader-call", "capability_id": "read_file", "state": "completed",
            "title": "Read fixture", "origin": ["kind": "builtin"], "input": ["path": "fixture.txt"],
            "output": output + "\n" + String(repeating: "Display-only long Tool result.\n", count: 70),
            "artifacts": [], "affected_resources": []]
        var values: [[String: Any]] = [tool]
        if prepend {
            values.insert(["kind": "reasoning", "part_id": "earlier-reasoning", "revision": 1,
                "state": "completed", "title": "Thinking", "summary": "Earlier display segment."], at: 0)
        }
        values.append(["kind": "text", "part_id": "answer", "revision": 1,
            "markdown": "Display fixture answer", "is_final": true])
        let parts = try JSONDecoder().decode([IOSConversationProjectionV1.Part].self,
            from: JSONSerialization.data(withJSONObject: values))
        return ChatMessage(id: id, role: .assistant, content: "Display fixture answer",
            canonicalParts: parts, metadata: ["nola.canonicalMessageID": id.uuidString])
    }
}

struct IOSMessageReaderUITestFixture: View {
    let container: ServiceContainer
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var inputs: IOSMessageReaderDisplayInputs

    init(container: ServiceContainer) {
        precondition(ProcessInfo.processInfo.arguments.contains("-ui-testing"))
        self.container = container
        do {
            let inputs = try IOSMessageReaderDisplayInputs()
            _inputs = StateObject(wrappedValue: inputs)
        }
        catch { preconditionFailure("Invalid typed reader display fixture") }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(inputs.status).accessibilityIdentifier("fixture.reader.status")
            Text(inputs.conversationID.uuidString).accessibilityIdentifier("fixture.reader.owner")
            // These are UI-test protocol identifiers and controls, not product copy.
            let partID = inputs.message.canonicalParts.first?.partID ?? "empty"
            Text(verbatim: partID).accessibilityIdentifier("fixture.reader.first-part")
            HStack {
                Button { inputs.arm(.prepend) } label: { Text(verbatim: "Arm prepend") }.accessibilityIdentifier("fixture.reader.arm-prepend")
                Button { inputs.arm(.owner) } label: { Text(verbatim: "Arm owner change") }.accessibilityIdentifier("fixture.reader.arm-owner")
            }
            ScrollView {
                ChatBubble(conversationID: inputs.conversationID, message: inputs.message,
                    isLatestAssistant: true, isStreaming: false, isRetrieving: false,
                    pendingHostPermissionRequest: nil, onHostPermissionDecision: { _, _ in })
                    .padding(16)
            }
        }
        .padding(.top, 12)
        .environmentObject(container)
        .background(ReaderWindowProbe(attach: inputs.attach, dismantle: { inputs.cancel(detach: true) }))
        .onChange(of: scenePhase) { _, phase in
            if phase != .active { inputs.cancel(detach: false) }
        }
    }
}

private struct ReaderWindowProbe: UIViewRepresentable {
    let attach: (UIWindow) -> Void
    let dismantle: () -> Void
    func makeUIView(context: Context) -> Probe {
        let view = Probe()
        view.attach = attach; view.dismantle = dismantle
        view.isUserInteractionEnabled = false
        return view
    }
    func updateUIView(_ view: Probe, context: Context) { view.attach = attach; view.dismantle = dismantle }
    static func dismantleUIView(_ view: Probe, coordinator: ()) { view.dismantle?() }
    final class Probe: UIView {
        var attach: ((UIWindow) -> Void)?
        var dismantle: (() -> Void)?
        override func didMoveToWindow() {
            super.didMoveToWindow()
            // A full-screen cover may temporarily detach the presenting view;
            // only genuine SwiftUI dismantle cancels the fixture operation.
            if let window { attach?(window) }
        }
    }
}
#endif
