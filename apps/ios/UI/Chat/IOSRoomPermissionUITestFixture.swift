// SPDX-License-Identifier: Apache-2.0
// Isolated UI automation only; no model, HTTP client or tool executor is installed.
#if DEBUG
import SwiftUI
import Combine

@MainActor
final class IOSRoomPermissionUITestState: ObservableObject {
    let container: ServiceContainer
    let workspace = IOSAgentRoomWorkspace()
    let submission: IOSRoomSubmissionWorkspace
    let service: IOSAgentRoomService
    let roomA: IOSAgentRoomSnapshot
    let roomB: IOSAgentRoomSnapshot
    let executionA: ChatManager
    let executionB: ChatManager
    let ownerA: UUID
    let ownerB: UUID
    @Published var status = "ready"
    private var acknowledgement: CheckedContinuation<Void, Never>?
    private var sendsA = 0
    private var sendsB = 0
    private var allowed = 0
    private var started = false

    init(container: ServiceContainer) throws {
        precondition(ProcessInfo.processInfo.arguments.contains("-ui-testing"))
        self.container = container
        service = IOSAgentRoomService(runtime: container.applicationRuntime)
        let directory = try service.createAgent(name: "Fixture assistant", description: "No tool execution",
            defaults: .object([:]), directory: service.directory())
        guard let agent = directory.agents.first else { throw CocoaError(.coderInvalidValue) }
        roomA = try service.join(agent: agent, room: service.create(title: "Fixture A"))
        roomB = try service.join(agent: agent, room: service.create(title: "Fixture B"))
        guard let a = roomA.room.participants.first, let b = roomB.room.participants.first,
              let ownerA = UUID(uuidString: a.contextConversationID),
              let ownerB = UUID(uuidString: b.contextConversationID),
              let executionA = container.chatManager.ensureExecutionForConversation(ownerA),
              let executionB = container.chatManager.ensureExecutionForConversation(ownerB) else {
            throw CocoaError(.coderInvalidValue)
        }
        self.ownerA = ownerA; self.ownerB = ownerB
        self.executionA = executionA; self.executionB = executionB
        submission = IOSRoomSubmissionWorkspace(service: service)
        try workspace.open(roomA, container: container)
    }

    func start() {
        guard !started else { return }
        started = true
        func request(_ nonce: String) -> HostAgentPermissionRequest {
            .init(id: "same-request", allowConversation: false, toolName: "write_file",
                argsPreview: "path: fixture.txt; no execution", match: "fixture.txt",
                reason: "Isolated permission interaction", message: "No tool will execute.", nonce: nonce)
        }
        executionA.startHostPermissionInteraction(request("nonce-a"), conversationId: ownerA,
            isCurrent: { true }, send: { [weak self] payload in
                guard let self else { return }
                self.sendsA += 1
                let resolution = payload["resolution"] as? [String: Any]
                guard payload["nonce"] as? String == "nonce-a",
                      payload["request_id"] as? String == "same-request",
                      resolution?["type"] as? String == "authorization",
                      resolution?["decision"] as? String == "deny" else {
                    self.status = "invalid-payload"; return
                }
                await withCheckedContinuation { continuation in
                    self.acknowledgement = continuation
                    self.status = "deny-sent-awaiting-ack"
                }
            }, onSubmitted: { [weak self] decision in
                guard let self else { return }
                if decision.isAllowed { self.allowed += 1 }
                let isolated = self.sendsA == 1 && self.sendsB == 0 && self.allowed == 0
                    && decision.wireValue == "deny"
                    && self.executionA.pendingHostPermissionRequest == nil
                    && self.executionA.pendingHostPermissionConversationId == nil
                    && self.executionB.pendingHostPermissionRequest?.nonce == "nonce-b"
                    && self.executionB.pendingHostPermissionConversationId == self.ownerB
                    && self.workspace.room?.room.id == self.roomB.room.id
                    && (try? self.service.submissions(roomID: self.roomA.room.id).isEmpty) == true
                    && (try? self.service.submissions(roomID: self.roomB.room.id).isEmpty) == true
                self.status = isolated ? "deny-confirmed-owner-isolated-no-submission" : "isolation-failed"
            })
        executionB.startHostPermissionInteraction(request("nonce-b"), conversationId: ownerB,
            isCurrent: { true }, send: { [weak self] _ in self?.sendsB += 1 })
    }

    func switchRoom() {
        do { try workspace.open(roomB, container: container); status = "room-b-awaiting-ack" }
        catch { status = "switch-failed" }
    }

    func acknowledge() { acknowledgement?.resume(); acknowledgement = nil }

    func close() {
        acknowledge()
        executionA.cancelHostPermissionInteraction()
        executionB.cancelHostPermissionInteraction()
        workspace.close()
    }
}

struct IOSRoomPermissionUITestFixture: View {
    @StateObject private var state: IOSRoomPermissionUITestState
    init(container: ServiceContainer) {
        _state = StateObject(wrappedValue: Self.prepare(container))
    }
    private static func prepare(_ container: ServiceContainer) -> IOSRoomPermissionUITestState {
        do { return try IOSRoomPermissionUITestState(container: container) }
        catch { fatalError("Unable to prepare isolated room permission fixture") }
    }
    var body: some View {
        NavigationStack {
            IOSRoomConversationView(workspace: state.workspace, submission: state.submission)
                .environmentObject(state.container)
                .navigationTitle("Room permission fixture")
                .safeAreaInset(edge: .top) {
                    VStack {
                        Text(state.status).font(.caption).accessibilityIdentifier("fixture.permission.status")
                        HStack {
                            Button("Switch to B") { state.switchRoom() }.accessibilityIdentifier("fixture.switch")
                            Button("Acknowledge A") { state.acknowledge() }.accessibilityIdentifier("fixture.ack")
                        }
                    }.padding(8)
                }
        }.task { state.start() }.onDisappear { state.close() }
    }
}
#endif
