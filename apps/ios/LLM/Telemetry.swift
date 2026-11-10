// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements Telemetry logic in the iOS LLM integration layer.
// - Interfaces:
//   - Telemetry
// - APIs:
//   - none
//
//
//  Telemetry.swift
//  Nola
//
//  Small telemetry helper to record inference metrics.
//

import Foundation

public enum Telemetry {
    public static func recordInference(provider: String, duration: TimeInterval, estimatedTokens: Int, success: Bool, error: Error?) {
        var msg = "inference:\(provider) duration=\(String(format: "%.3f", duration))s tokens=\(estimatedTokens) success=\(success)"
        if let e = error {
            msg += " error=\(String(describing: e))"
        }
        Logger.shared.log(msg, level: success ? .info : .error)
    }

    public static func estimateTokens(from text: String) -> Int {
        // rough heuristic: 4 chars ~ 1 token
        let chars = text.count
        return max(1, chars / 4)
    }
}
