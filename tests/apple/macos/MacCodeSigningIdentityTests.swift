// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies that permission diagnostics distinguish stable app identities from rebuild-specific ad-hoc identities.

import XCTest
@testable import Nola

final class MacCodeSigningIdentityTests: XCTestCase {
    func testMacTargetPinsStableDevelopmentSigningForTCCPermissions() throws {
        let project = try projectSource()
        let macConfigurationStart = try XCTUnwrap(
            project.range(of: "B10000000000000000000008 /* Debug */")
        )
        let macConfigurationEnd = try XCTUnwrap(
            project.range(
                of: "D50000000000000000000008 /* Debug */",
                range: macConfigurationStart.upperBound..<project.endIndex
            )
        )
        let macConfigurations = project[macConfigurationStart.lowerBound..<macConfigurationEnd.lowerBound]

        XCTAssertEqual(macConfigurations.components(separatedBy: "CODE_SIGN_IDENTITY = \"Apple Development\";").count - 1, 2)
        XCTAssertEqual(macConfigurations.components(separatedBy: "DEVELOPMENT_TEAM = 65G58B4R2Q;").count - 1, 2)
        XCTAssertEqual(macConfigurations.components(separatedBy: "PRODUCT_BUNDLE_IDENTIFIER = com.gaojc.Nola.mac;").count - 1, 2)
    }

    func testEmbeddedAgentToolsUseAdHocSigningWithoutASecondKeychainIdentity() throws {
        let project = try projectSource()
        let phaseStart = try XCTUnwrap(
            project.range(of: "F1000000000000000000000C /* Embed Nola Runtime Products */")
        )
        let phaseEnd = try XCTUnwrap(
            project.range(
                of: "/* End PBXShellScriptBuildPhase section */",
                range: phaseStart.upperBound..<project.endIndex
            )
        )
        let phase = project[phaseStart.lowerBound..<phaseEnd.lowerBound]

        XCTAssertTrue(phase.contains("${SRCROOT}/scripts/embed_mac_runtime.sh"))
        XCTAssertTrue(phase.contains("$(SRCROOT)/scripts/lib/install_mac_executable.sh"))
        XCTAssertFalse(phase.contains("EXPANDED_CODE_SIGN_IDENTITY"))
        let embed = try repositorySource("scripts/embed_mac_runtime.sh")
        let installer = try repositorySource("scripts/lib/install_mac_executable.sh")
        XCTAssertTrue(embed.contains(". \"${SRCROOT}/scripts/lib/install_mac_executable.sh\""))
        XCTAssertTrue(embed.contains("MACOS=\"${TARGET_BUILD_DIR}/${CONTENTS_FOLDER_PATH}/MacOS\""))
        for executable in ["nola-agent-host", "nola-service", "nola-mlx-runtime"] {
            XCTAssertTrue(embed.contains("\"$MACOS/\(executable)\" \"${CODE_SIGNING_ALLOWED:-NO}\""))
        }
        XCTAssertTrue(installer.contains("if [[ \"$sign\" == YES ]]; then"))
        let sign = try XCTUnwrap(installer.range(of: "/usr/bin/codesign --force --sign - --timestamp=none \"$staged\" || exit 1"))
        let verify = try XCTUnwrap(installer.range(of: "/usr/bin/codesign --verify \"$staged\" || exit 1"))
        let install = try XCTUnwrap(installer.range(of: "mv -f \"$staged\" \"$destination\" || exit 1"))
        XCTAssertLessThan(sign.lowerBound, verify.lowerBound)
        XCTAssertLessThan(verify.lowerBound, install.lowerBound)
        for source in [embed, installer] {
            XCTAssertFalse(source.contains("EXPANDED_CODE_SIGN_IDENTITY"))
            XCTAssertFalse(source.contains("Products/Debug"))
            XCTAssertFalse(source.contains("security unlock-keychain"))
        }
    }

    func testDefaultMacTestBuildCannotTouchSigningKeychain() throws {
        let project = try projectSource()
        let appTestStart = try XCTUnwrap(
            project.range(of: "E50000000000000000000001 /* Test */")
        )
        let testBundleStart = try XCTUnwrap(
            project.range(
                of: "E50000000000000000000002 /* Test */",
                range: appTestStart.upperBound..<project.endIndex
            )
        )
        let testBundleEnd = try XCTUnwrap(
            project.range(
                of: "E50000000000000000000003 /* Test */",
                range: testBundleStart.upperBound..<project.endIndex
            )
        )

        let appTestConfiguration = project[
            appTestStart.lowerBound..<testBundleStart.lowerBound
        ]
        let testBundleConfiguration = project[
            testBundleStart.lowerBound..<testBundleEnd.lowerBound
        ]
        XCTAssertTrue(appTestConfiguration.contains("CODE_SIGNING_ALLOWED = NO;"))
        XCTAssertTrue(testBundleConfiguration.contains("CODE_SIGNING_ALLOWED = NO;"))
    }

    func testStableDeveloperIdentityCanReuseTCCGrantAcrossBuilds() {
        let identity = MacCodeSigningIdentitySnapshot(
            bundleIdentifier: "com.gaojc.Nola.mac",
            signingIdentifier: "com.gaojc.Nola.mac",
            teamIdentifier: "65G58B4R2Q",
            designatedRequirement: "identifier com.gaojc.Nola.mac and anchor apple generic"
        )
        XCTAssertTrue(identity.isStable)
    }

    func testAdHocCDHashIdentityCannotReuseVisibleOlderTCCGrant() {
        let identity = MacCodeSigningIdentitySnapshot(
            bundleIdentifier: "com.gaojc.Nola.mac",
            signingIdentifier: "Nola",
            teamIdentifier: nil,
            designatedRequirement: "cdhash H\"0123456789abcdef\""
        )
        XCTAssertFalse(identity.isStable)
    }

    func testMismatchedSigningIdentifierIsNotStable() {
        let identity = MacCodeSigningIdentitySnapshot(
            bundleIdentifier: "com.gaojc.Nola.mac",
            signingIdentifier: "com.gaojc.Nola.debug",
            teamIdentifier: "65G58B4R2Q",
            designatedRequirement: "identifier com.gaojc.Nola.debug and anchor apple generic"
        )
        XCTAssertFalse(identity.isStable)
    }

    private func projectSource() throws -> String {
        let projectURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "NolaProject.pbxproj",
                withExtension: "txt"
            )
        )
        return try String(contentsOf: projectURL, encoding: .utf8)
    }

    private func repositorySource(_ path: String) throws -> String {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        return try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
    }
}
