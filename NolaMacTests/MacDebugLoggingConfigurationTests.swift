import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacDebugLoggingConfigurationTests: XCTestCase {
    func testDefaultsDisableDetailedLoggingAtDebugLevel() throws {
        let defaults = try makeDefaults()
        defer { clear(defaults) }
        let logger = Logger()

        MacDebugLoggingConfiguration.apply(defaults: defaults, logger: logger)

        XCTAssertFalse(logger.runtimeLoggingState.isDetailedLoggingEnabled)
        XCTAssertEqual(logger.runtimeLoggingState.minimumLevel, .debug)
        XCTAssertTrue(logger.shouldRecord(.error))
        XCTAssertTrue(logger.shouldRecord(.warn))
        XCTAssertFalse(logger.shouldRecord(.info))
        XCTAssertFalse(logger.shouldRecord(.debug))
    }

    func testPersistedLevelControlsRuntimeFiltering() throws {
        let defaults = try makeDefaults()
        defer { clear(defaults) }
        let logger = Logger()
        defaults.set(true, forKey: MacDebugLoggingConfiguration.enabledKey)
        defaults.set("debug", forKey: MacDebugLoggingConfiguration.levelKey)

        MacDebugLoggingConfiguration.apply(defaults: defaults, logger: logger)

        XCTAssertTrue(logger.runtimeLoggingState.isDetailedLoggingEnabled)
        XCTAssertTrue(logger.shouldRecord(.info))
        XCTAssertTrue(logger.shouldRecord(.debug))
        XCTAssertFalse(logger.shouldRecord(.trace))

        defaults.set("trace", forKey: MacDebugLoggingConfiguration.levelKey)
        MacDebugLoggingConfiguration.apply(defaults: defaults, logger: logger)
        XCTAssertTrue(logger.shouldRecord(.trace))
    }

    func testResetRemovesPreferencesAndDisablesDetailedLogging() throws {
        let defaults = try makeDefaults()
        defer { clear(defaults) }
        let logger = Logger()
        defaults.set(true, forKey: MacDebugLoggingConfiguration.enabledKey)
        defaults.set("trace", forKey: MacDebugLoggingConfiguration.levelKey)

        MacDebugLoggingConfiguration.reset(defaults: defaults, logger: logger)

        XCTAssertNil(defaults.object(forKey: MacDebugLoggingConfiguration.enabledKey))
        XCTAssertNil(defaults.object(forKey: MacDebugLoggingConfiguration.levelKey))
        XCTAssertFalse(logger.runtimeLoggingState.isDetailedLoggingEnabled)
        XCTAssertEqual(logger.runtimeLoggingState.minimumLevel, .debug)
    }

    func testDiagnosticSanitizerRemovesSensitiveValuesAndLocationDetails() {
        let source = "prompt=private request, token=secret-token, url=https://example.com/private?q=secret, path=/Users/person/Documents/report.pdf"

        let sanitized = Logger.sanitizeForDiagnostics(source)

        XCTAssertFalse(sanitized.contains("private request"))
        XCTAssertFalse(sanitized.contains("secret-token"))
        XCTAssertFalse(sanitized.contains("/private?q=secret"))
        XCTAssertFalse(sanitized.contains("/Users/person"))
        XCTAssertTrue(sanitized.contains("https://example.com"))
        XCTAssertTrue(sanitized.contains("<redacted>"))
        XCTAssertTrue(sanitized.contains("<path>"))
    }

    private func makeDefaults() throws -> UserDefaults {
        let suiteName = "nola.mac.debug-logging.tests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        defaults.removePersistentDomain(forName: suiteName)
        return defaults
    }

    private func clear(_ defaults: UserDefaults) {
        if let suiteName = defaults.volatileDomainNames.first(where: { $0.hasPrefix("nola.mac.debug-logging.tests.") }) {
            defaults.removePersistentDomain(forName: suiteName)
        }
        defaults.removeObject(forKey: MacDebugLoggingConfiguration.enabledKey)
        defaults.removeObject(forKey: MacDebugLoggingConfiguration.levelKey)
    }
}
