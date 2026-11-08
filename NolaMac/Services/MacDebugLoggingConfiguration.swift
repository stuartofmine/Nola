// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns the device-local runtime logging preference for Nola on macOS.

import Foundation

nonisolated enum MacDebugLoggingConfiguration {
    static let enabledKey = "nola.mac.debugLogging.enabled"
    static let levelKey = "nola.mac.debugLogging.level"

    static func isEnabled(defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: enabledKey) as? Bool ?? false
    }

    static func level(defaults: UserDefaults = .standard) -> Logger.Level {
        level(from: defaults.string(forKey: levelKey))
    }

    @MainActor
    static func apply(
        defaults: UserDefaults = .standard,
        logger: Logger
    ) {
        let enabled = isEnabled(defaults: defaults)
        let configuredLevel = level(defaults: defaults)
        let currentState = logger.runtimeLoggingState
        guard currentState.isDetailedLoggingEnabled != enabled
                || currentState.minimumLevel != configuredLevel else {
            return
        }
        logger.configureRuntimeLogging(
            detailedLoggingEnabled: enabled,
            minimumLevel: configuredLevel
        )
    }

    @MainActor
    static func apply(defaults: UserDefaults = .standard) {
        apply(defaults: defaults, logger: .shared)
    }

    static func apply(
        to environment: inout [String: String],
        defaults: UserDefaults = .standard,
        logStore: LogFileStore = .shared
    ) {
        if isEnabled(defaults: defaults) {
            environment["NOLA_CLI_DEBUG"] = "1"
            environment["NOLA_CLI_DEBUG_LOG_PATH"] = logStore.logFileURL().path
        } else {
            environment.removeValue(forKey: "NOLA_CLI_DEBUG")
            environment.removeValue(forKey: "NOLA_CLI_DEBUG_LOG_PATH")
        }
    }

    @MainActor
    static func reset(
        defaults: UserDefaults = .standard,
        logger: Logger
    ) {
        defaults.removeObject(forKey: enabledKey)
        defaults.removeObject(forKey: levelKey)
        apply(defaults: defaults, logger: logger)
    }

    @MainActor
    static func reset(defaults: UserDefaults = .standard) {
        reset(defaults: defaults, logger: .shared)
    }

    static func level(from rawValue: String?) -> Logger.Level {
        switch rawValue?.lowercased() {
        case "error": return .error
        case "warn", "warning": return .warn
        case "info": return .info
        case "trace": return .trace
        default: return .debug
        }
    }
}
