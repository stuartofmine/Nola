// SPDX-License-Identifier: Apache-2.0
import SwiftUI

nonisolated enum AppleUIReadOnlyReferenceValidation {
    static func matches<JSON: JournalJSONValue>(_ snapshot: AppleUISnapshot<JSON>, reference: AppleUISurfaceReference) -> Bool {
        snapshot.identity == reference.identity && snapshot.revision == reference.revision
            && (snapshot.phase == .readOnly || snapshot.phase == .deleted)
            && snapshot.nodes.values.allSatisfy { $0.actions.isEmpty && $0.bindings.isEmpty }
    }
}

/// Always-visible, source-owned readonly version. The Host supplies the actual
/// readonly projection; the renderer never edits phase, nodes or permissions.
struct AppleUIReadOnlyReferencedSurfaceView<JSON: JournalJSONValue>: View {
    let reference: AppleUISurfaceReference
    let summary: String
    let notice: String
    let readRevision: (AppleUISurfaceReference) async throws -> AppleUISnapshot<JSON>?
    @State private var loaded: (reference: AppleUISurfaceReference, snapshot: AppleUISnapshot<JSON>)?
    @State private var failure: (reference: AppleUISurfaceReference, message: String)?
    @State private var retry = 0
    private struct ReadKey: Hashable { let reference: AppleUISurfaceReference; let retry: Int }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(verbatim: notice).font(.callout).foregroundStyle(.secondary)
            if let loaded, loaded.reference == reference {
                AppleUISurfaceView(snapshot: loaded.snapshot,
                    submit: { _ in .rejected(notice) }, accepted: { _ in })
                    .id(reference.identity)
            } else {
                if !summary.isEmpty { Text(verbatim: summary).foregroundStyle(.secondary) }
                if let failure, failure.reference == reference {
                    Text(verbatim: failure.message).foregroundStyle(.secondary)
                    Button { retry += 1 } label: { Image(systemName: "arrow.clockwise") }
                        .accessibilityLabel(Text("Retry"))
                } else { ProgressView().controlSize(.small) }
            }
        }
        .accessibilityIdentifier("a2ui.readonly.reference." + reference.identity.accessibilityIdentifier)
        .task(id: ReadKey(reference: reference, retry: retry)) {
            failure = nil
            do {
                let result = try await readRevision(reference)
                try Task.checkCancellation()
                guard let result, AppleUIReadOnlyReferenceValidation.matches(result, reference: reference) else {
                    throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Readonly Surface version does not match its source reference"))
                }
                loaded = (reference, result)
            } catch is CancellationError { }
            catch { failure = (reference, error.localizedDescription) }
        }
    }
}
