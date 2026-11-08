// SPDX-License-Identifier: Apache-2.0
// Conversation-local composer state; asynchronous imports never target selection.
import Foundation

struct ConversationComposerDraftStore {
    nonisolated enum Owner: Hashable, Codable, Sendable {
        case conversation(UUID)
        case unsent(UUID)
    }
    nonisolated struct Draft: Sendable {
        private(set) var textRevision = UUID()
        var text = "" { didSet { if text != oldValue { textRevision = UUID() } } }
        var attachments: [ChatAttachmentDraft] = []
        var notice: String?
        var importID: UUID?
        var configuration: ModelConfiguration?
        var restored = false
    }
    nonisolated struct ModelConfiguration: Codable, Equatable, Sendable {
        enum Source: String, Codable, Sendable { case local, configured, host }
        let modelID: String
        let source: Source
        let parameters: ModelParameters
        init(modelID: String, source: Source, parameters: ModelParameters) {
            self.modelID = modelID; self.source = source; self.parameters = parameters
        }
        init(model: ModelInfo, parameters: ModelParameters) {
            self.init(modelID: model.id, source: model.isWebBackend ? .host : model.isLocal ? .local : .configured, parameters: parameters)
        }
    }
    struct Import: Equatable {
        let owner: Owner
        let id: UUID
    }
    private(set) var selected: Owner = .unsent(UUID())
    private var drafts: [Owner: Draft] = [:]
    private let repository: (any ConversationDraftRepository)?
    var repositoryIdentity: UUID? { repository?.identity }
    private var ownerErrors: [Owner: String] = [:]
    private var repositoryError: String?
    private(set) var persistenceError: String? {
        get { ownerErrors[selected] ?? repositoryError }
        set { ownerErrors[selected] = newValue }
    }

    init(repository: (any ConversationDraftRepository)? = nil) {
        self.repository = repository
        do {
            if let repository {
                selected = .unsent(try repository.pendingOwner())
                // Reopen the original canonical owner after a lost ACK. A
                // pending intent never authorizes a second submission.
                if let pending = try repository.pendingSubmissions().last(where: { $0.sourceOwner == selected }) {
                    selected = .conversation(pending.conversationID)
                }
                drafts[selected] = try repository.load(selected)
            }
        } catch { repositoryError = error.localizedDescription }
    }

    private mutating func persist(_ owner: Owner) {
        guard let draft = drafts[owner] else { return }
        do { try repository?.save(draft, owner: owner) }
        catch { ownerErrors[owner] = error.localizedDescription }
    }

    var current: Draft {
        get { drafts[selected] ?? Draft() }
        set { drafts[selected] = newValue; persist(selected) }
    }
    nonisolated struct TextSubmissionTicket: Equatable, Sendable {
        let owner: Owner
        let revision: UUID
    }
    func textSubmissionTicket() -> TextSubmissionTicket {
        .init(owner: selected, revision: current.textRevision)
    }
    /// An acknowledgement owns the original text revision, even if the user
    /// changed conversations or later typed the identical text again.
    @discardableResult
    mutating func acknowledgeSubmittedText(_ ticket: TextSubmissionTicket) -> Bool {
        guard var draft = drafts[ticket.owner], draft.textRevision == ticket.revision else { return false }
        draft.text = ""
        drafts[ticket.owner] = draft
        persist(ticket.owner)
        return true
    }
    func draft(for owner: Owner) -> Draft? { drafts[owner] }
    mutating func recordPersistenceFailure(owner: Owner, detail: String) {
        guard drafts[owner] != nil else { return }
        drafts[owner]?.notice = detail
        ownerErrors[owner] = detail
    }

    mutating func select(_ conversationID: UUID?) {
        if let conversationID { selected = .conversation(conversationID) }
        else if case .conversation = selected {
            do { selected = .unsent(try repository?.newPendingOwner() ?? UUID()) }
            catch { persistenceError = error.localizedDescription; selected = .unsent(UUID()) }
        }
        if drafts[selected] == nil {
            do { drafts[selected] = try repository?.load(selected) }
            catch { persistenceError = error.localizedDescription }
        }
    }

    mutating func beginImport() -> Import? {
        guard current.importID == nil else { return nil }
        let operation = Import(owner: selected, id: UUID())
        current.importID = operation.id
        current.notice = nil
        return operation
    }

    func accepts(_ operation: Import) -> Bool {
        drafts[operation.owner]?.importID == operation.id
    }

    @discardableResult
    mutating func finishImport(_ operation: Import, attachments: [ChatAttachmentDraft], notice: String?) -> Bool {
        guard accepts(operation), var draft = drafts[operation.owner] else { return false }
        draft.attachments = Array((draft.attachments + attachments).suffix(10))
        draft.notice = notice
        draft.importID = nil
        drafts[operation.owner] = draft
        persist(operation.owner)
        return true
    }

    func validateForSubmission() throws { try repository?.validateForSubmission(selected) }

    nonisolated struct Submission: Sendable {
        let ticket: ConversationDraftSubmissionTicket?
        let repository: (any ConversationDraftRepository)?
        func committed() throws { if let ticket { try repository?.commitSubmission(ticket) } }
    }

    mutating func prepareSubmission(to conversationID: UUID, identity: IOSCanonicalAgentTurnIdentity,
                                    configurationRevision: Int64?) throws -> Submission {
        try validateForSubmission()
        let ticket = try repository?.stageSubmission(owner: selected, conversationID: conversationID,
            userMessageID: identity.visibleUserMessageID, configurationRevision: configurationRevision, draft: current)
        drafts[selected] = nil
        ownerErrors[selected] = nil
        selected = .conversation(conversationID)
        drafts[selected] = Draft()
        persist(selected)
        return Submission(ticket: ticket, repository: repository)
    }

    @discardableResult
    mutating func submitted(to conversationID: UUID) -> Bool {
        if case .unsent = selected {
            do { try repository?.retire(selected) } catch { persistenceError = error.localizedDescription; return false }
        }
        drafts[selected] = nil
        ownerErrors[selected] = nil
        selected = .conversation(conversationID)
        drafts[selected] = Draft()
        persist(selected)
        return true
    }

    mutating func remove(_ conversationID: UUID) {
        drafts[.conversation(conversationID)] = nil
        do { try repository?.remove(.conversation(conversationID)) }
        catch { ownerErrors[.conversation(conversationID)] = error.localizedDescription }
    }

    mutating func cancelImports() {
        for owner in Array(drafts.keys) { drafts[owner]?.importID = nil }
    }
}
