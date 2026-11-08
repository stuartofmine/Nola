import Foundation

/// One message's presentation cursor. Accepted text is not necessarily visible:
/// segmented text remains owned here across suspended or cancelled reveal tasks.
@MainActor
final class MacAssistantPresentationSession {
    struct Lease: Equatable { fileprivate let revision: UInt64 }
    struct Frame {
        let text: String
        let nodes: [MacAssistantTextNode]
        let mutableTail: MacAssistantTextNode?
        let replacesText: Bool
        let isComplete: Bool
    }

    private(set) var acceptedText: String
    private(set) var visibleText: String
    private(set) var assembler: MacSemanticStreamAssembler
    private var pacing = MacPacedStreamBuffer()
    private var pendingSegments: [String] = []
    private var nextSegmentIndex = 0
    private var revision: UInt64 = 0
    private var completed = false
    private let identitySeed: String

    init(visibleText: String = "", assembler: MacSemanticStreamAssembler? = nil,
         identitySeed: String) {
        self.visibleText = visibleText
        acceptedText = visibleText
        self.identitySeed = identitySeed
        self.assembler = assembler ?? MacSemanticStreamAssembler(existingText: visibleText, identitySeed: identitySeed)
    }

    var pendingSegmentCount: Int { pendingSegments.count - nextSegmentIndex }

    func accept(authoritative text: String) -> Frame? {
        switch AuthoritativeTextReconciliation.reconcile(authoritative: text, accepted: acceptedText) {
        case .unchanged: return nil
        case let .append(delta):
            acceptedText = text
            completed = false
            pendingSegments.append(contentsOf: pacing.append(delta))
            return nil
        case let .replace(text):
            revision &+= 1
            acceptedText = text
            visibleText = text
            completed = false
            pacing = MacPacedStreamBuffer()
            pendingSegments = []
            nextSegmentIndex = 0
            assembler = MacSemanticStreamAssembler(identitySeed: identitySeed)
            let nodes = assembler.append(text) + assembler.finish()
            return Frame(text: text, nodes: nodes, mutableTail: nil, replacesText: true, isComplete: false)
        }
    }

    /// Replacing a reveal owner invalidates its lease, not its pending text.
    func beginReveal() -> Lease {
        revision &+= 1
        return Lease(revision: revision)
    }

    func nextFrame(lease: Lease) -> Frame? {
        guard lease.revision == revision, nextSegmentIndex < pendingSegments.count else { return nil }
        let segment = pendingSegments[nextSegmentIndex]
        nextSegmentIndex += 1
        if nextSegmentIndex == pendingSegments.count {
            pendingSegments.removeAll(keepingCapacity: true)
            nextSegmentIndex = 0
        }
        visibleText += segment
        let nodes = assembler.append(segment)
        return Frame(text: visibleText, nodes: nodes, mutableTail: assembler.mutableTail,
            replacesText: false, isComplete: false)
    }

    func flushPending() { pendingSegments.append(contentsOf: pacing.finish()) }

    func finishFrame(lease: Lease) -> Frame? {
        guard lease.revision == revision, pendingSegmentCount == 0,
              pacing.bufferedText.isEmpty, !completed else { return nil }
        completed = true
        return Frame(text: visibleText, nodes: assembler.finish(), mutableTail: nil,
            replacesText: false, isComplete: true)
    }

    /// Synchronous adapter: append to the accepted cursor and reveal all owned
    /// pending text, never to a potentially lagging visible prefix.
    func appendImmediately(_ delta: String) -> Frame {
        let replacement = accept(authoritative: acceptedText + delta)
        flushPending()
        let lease = beginReveal()
        var nodes = replacement?.nodes ?? []
        while let frame = nextFrame(lease: lease) { nodes += frame.nodes }
        return Frame(text: visibleText, nodes: nodes, mutableTail: assembler.mutableTail,
            replacesText: replacement?.replacesText ?? false, isComplete: false)
    }
}
