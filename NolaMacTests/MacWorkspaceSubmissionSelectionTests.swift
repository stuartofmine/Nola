// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
private final class SubmissionBookmarkIO: MacWorkspaceBookmarkIO {
    var selected: URL?
    func chooseFolder(suggested: URL?) -> URL? { selected }
    func bookmark(_ url: URL) throws -> Data { Data(url.absoluteString.utf8) }
    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        guard let value = String(data: data, encoding: .utf8), let url = URL(string: value) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        return (url, false)
    }
    func start(_ url: URL) -> Bool { true }
    func stop(_ url: URL) {}
}

@MainActor
final class MacWorkspaceSubmissionSelectionTests: XCTestCase {
    func testLaterPrimaryAndGrantsDoNotChangeCapturedSubmissionAuthority() async throws {
        let suite = "Nola.SubmissionSelection.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = SubmissionBookmarkIO()
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let root = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
            .appendingPathComponent(UUID().uuidString)
        let original = root.appendingPathComponent("original")
        let added = root.appendingPathComponent("new-primary")
        let confirmed = root.appendingPathComponent("new-confirmed")
        io.selected = original
        let originalSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertEqual(originalSelection, original)
        let implicit = store.captureSubmissionSelection(conversationID: "conversation", selectedURL: nil)
        let explicit = store.captureSubmissionSelection(conversationID: "conversation", selectedURL: original)
        io.selected = added
        let addedSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertEqual(addedSelection, added)
        try store.confirmFolderAccess(confirmed, conversationID: "conversation")
        for snapshot in [implicit, explicit] {
            let resolved = try await store.resolveSubmissionSelection(snapshot)
            XCTAssertEqual(resolved.projectURL, original)
            XCTAssertEqual(Set(resolved.authorizedURLs), [original])
            XCTAssertFalse(resolved.authorizedURLs.contains(added))
            XCTAssertFalse(resolved.authorizedURLs.contains(confirmed))
        }
    }

    func testRevokedCapturedWorkspaceFailsInsteadOfChangingToSandboxOrNewPrimary() async throws {
        let suite = "Nola.SubmissionSelection.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let io = SubmissionBookmarkIO()
        let store = MacSecurityScopedBookmarkStore(defaults: defaults, io: io)
        defer { store.close() }
        let root = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
            .appendingPathComponent(UUID().uuidString)
        let original = root.appendingPathComponent("original")
        io.selected = original
        let originalSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertNotNil(originalSelection)
        let snapshots = [
            store.captureSubmissionSelection(conversationID: "conversation", selectedURL: nil),
            store.captureSubmissionSelection(conversationID: "conversation", selectedURL: original)
        ]
        store.removeWorkspace(conversationID: "conversation")
        io.selected = root.appendingPathComponent("replacement")
        let replacementSelection = await store.chooseWorkspace(conversationID: "conversation")
        XCTAssertNotNil(replacementSelection)
        for snapshot in snapshots {
            do {
                _ = try await store.resolveSubmissionSelection(snapshot)
                XCTFail("Revocation must reject this submission, not silently select a different directory")
            } catch {
                XCTAssertEqual((error as NSError).domain, NSCocoaErrorDomain)
                XCTAssertEqual((error as NSError).code, CocoaError.fileReadNoPermission.rawValue)
            }
        }
    }
}
