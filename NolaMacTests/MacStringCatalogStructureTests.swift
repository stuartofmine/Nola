// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Ensures Mac string catalogs keep every localized key inside the canonical strings dictionary.

import XCTest

final class MacStringCatalogStructureTests: XCTestCase {
    func testLocalizableCatalogHasOnlyCanonicalTopLevelFields() throws {
        let catalogURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "Localizable.xcstrings",
                withExtension: "txt"
            )
        )
        let root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(contentsOf: catalogURL)) as? [String: Any]
        )

        XCTAssertEqual(Set(root.keys), ["sourceLanguage", "strings", "version"])
        let strings = try XCTUnwrap(root["strings"] as? [String: Any])
        let signingMessage = "This Nola build has an unstable signing identity. macOS may show an older Nola entry as enabled while denying the current build. Run a normally signed build, remove the stale entry, and grant the current Nola app access again."
        let entry = try XCTUnwrap(strings[signingMessage] as? [String: Any])
        let localizations = try XCTUnwrap(entry["localizations"] as? [String: Any])
        let simplifiedChinese = try XCTUnwrap(localizations["zh-Hans"] as? [String: Any])
        let unit = try XCTUnwrap(simplifiedChinese["stringUnit"] as? [String: Any])
        XCTAssertEqual(unit["state"] as? String, "translated")
        XCTAssertFalse((unit["value"] as? String)?.isEmpty ?? true)
    }
}
