// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacCoreSourceProjectionVisibilityTests: XCTestCase {
    func testLocalFilesStayInsideToolTraceInsteadOfReplySources() {
        let internalManifest = source(
            id: "manifest",
            title: "NOLA_MANIFEST.json",
            uri: "file:///Applications/Nola.app/Contents/Resources/NolaBuiltInSkills.bundle/word/NOLA_MANIFEST.json"
        )
        let internalScript = source(
            id: "script",
            title: "create_document.py",
            uri: "file:///Applications/Nola.app/Contents/Resources/NolaBuiltInSkills.bundle/word/scripts/create_document.py"
        )
        let userDocument = source(
            id: "document",
            title: "比赛资料.md",
            uri: "file:///Users/example/Documents/比赛资料.md"
        )
        let legacyAbsolutePath = source(
            id: "legacy-document",
            title: "旧会话资料.md",
            uri: "/Users/example/Documents/旧会话资料.md"
        )

        let projected = MacCoreSourceProjection.project([
            internalManifest,
            internalScript,
            userDocument,
            legacyAbsolutePath,
        ])

        XCTAssertTrue(projected.isEmpty)
    }

    func testWebEvidenceRemainsAReplyLevelSource() {
        let webPage = source(
            id: "web",
            title: "Reference",
            uri: "https://example.com/reference"
        )

        let projected = MacCoreSourceProjection.project([webPage])

        XCTAssertEqual(projected.map(\.sourceID), ["web"])
        XCTAssertEqual(projected.first?.displayTitle, "Reference")
    }

    func testResidentMemoryIsAnActionableInternalContextReference() {
        let projected = MacCoreSourceProjection.project([
            source(
                id: "memory-source",
                title: "Selected resident memory",
                uri: "nola-memory://record/memory-record-1"
            )
        ])

        guard let reference = try? XCTUnwrap(projected.first) else {
            return XCTFail("Expected a projected memory reference")
        }
        XCTAssertTrue(reference.isMemoryReference)
        guard case .memory(let recordID) = reference.destination else {
            return XCTFail("Expected a memory-center destination")
        }
        XCTAssertEqual(recordID, "memory-record-1")
        XCTAssertNotEqual(reference.displayTitle, "Selected resident memory")
    }

    private func source(id: String, title: String, uri: String) -> MacCoreEvidencePart {
        MacCoreEvidencePart(
            kind: "source",
            id: id,
            messageID: nil,
            turnID: nil,
            sourceID: id,
            citationID: nil,
            title: title,
            uri: uri,
            mimeType: nil,
            location: nil,
            range: nil,
            evidence: nil
        )
    }
}
