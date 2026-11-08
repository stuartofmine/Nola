// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MainActorWatchdog logic in the iOS service layer.
// - Interfaces:
//   - MainActorWatchdog
// - APIs:
//   - none
//
import Foundation
import UIKit

/// Logs when the main actor stops responding for more than a threshold.
/// This is intended for diagnosing startup/test hangs and should be lightweight.
final class MainActorWatchdog {
    private let thresholdSeconds: TimeInterval
    private let staleTickResetSeconds: TimeInterval = 60
    private let queue = DispatchQueue(label: "com.nola.main-actor-watchdog", qos: .background)
    private var timer: DispatchSourceTimer?
    private let lock = NSLock()
    private var lastMainTick = Date()
    private var lastWarn = Date.distantPast
    private var stage: String = "(unset)"
    private var isAppActive = true
    private weak var logger: Logger?

    init(thresholdSeconds: TimeInterval) {
        self.thresholdSeconds = thresholdSeconds
    }

    func start(logger: Logger) {
        lock.lock()
        defer { lock.unlock() }
        self.logger = logger
        lastMainTick = Date()
        lastWarn = .distantPast
        if timer != nil { return }

        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 1, repeating: 1)
        t.setEventHandler { [weak self] in
            self?.tick()
        }
        timer = t
        t.resume()
    }

    func setStage(_ stage: String) {
        lock.lock()
        self.stage = stage
        lock.unlock()
    }

    private func isActionableStage(_ stage: String) -> Bool {
        guard stage != "(unset)" else { return false }
        if stage.hasSuffix(":done") || stage.contains(":skipped") {
            return false
        }
        return true
    }

    private func tick() {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.lock.lock()
            self.lastMainTick = Date()
            self.isAppActive = UIApplication.shared.applicationState == .active
            self.lock.unlock()
        }

        lock.lock()
        let now = Date()
        if !isAppActive {
            lastMainTick = now
            lock.unlock()
            return
        }

        let elapsed = now.timeIntervalSince(lastMainTick)
        if elapsed > staleTickResetSeconds {
            lastMainTick = now
            lock.unlock()
            return
        }

        let canWarn = now.timeIntervalSince(lastWarn) >= max(2, thresholdSeconds)
        let stageSnapshot = stage
        let logger = self.logger
        if elapsed >= thresholdSeconds && canWarn && isActionableStage(stageSnapshot) {
            lastWarn = now
            lock.unlock()
            logger?.log(.warn, "MainActorWatchdog", "Main actor appears blocked", metadata: [
                "elapsedSeconds": String(format: "%.1f", elapsed),
                "stage": stageSnapshot
            ])
            return
        }
        lock.unlock()
    }
}
