// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LogFileStore logic in the iOS service layer.
// - Interfaces:
//   - LogFileStore
// - APIs:
//   - none
//
//
//  LogFileStore.swift
//  Nola
//
//  Lightweight log file sink with basic rotation and tail-reading.
//

import Foundation

nonisolated public final class LogFileStore: @unchecked Sendable {
    public static let shared = LogFileStore()

    private let queue = DispatchQueue(label: "nola.logfilestore", qos: .utility)
    private let fileManager = FileManager.default

    // Keep this modest to avoid storage bloat.
    private let maxBytesPerFile: Int64 = 1_000_000
    private let maxArchives: Int = 2

    private init() {}

    public func logFileURL() -> URL {
        let environment = ProcessInfo.processInfo.environment
        if let configured = environment["NOLA_MAC_APP_SUPPORT"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
           !configured.isEmpty {
            return URL(fileURLWithPath: configured, isDirectory: true)
                .appendingPathComponent("Logs", isDirectory: true)
                .appendingPathComponent("nola.log", isDirectory: false)
        }
        if environment["XCTestConfigurationFilePath"] != nil
            || environment["XCTestBundlePath"] != nil
            || environment["SWIFT_TESTING"] != nil {
            return fileManager.temporaryDirectory
                .appendingPathComponent(
                    "Nola-Test-Runtime-\(ProcessInfo.processInfo.processIdentifier)",
                    isDirectory: true
                )
                .appendingPathComponent("Logs", isDirectory: true)
                .appendingPathComponent("nola.log", isDirectory: false)
        }
        let base = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? fileManager.temporaryDirectory
        // Keep diagnostics inside the product-owned support directory.  The
        // previous `Application Support/Logs` location was both easy to miss
        // and shared a generic namespace with unrelated applications, which
        // made the Mac diagnostics screen appear empty even while logging was
        // enabled.
        let dir = base
            .appendingPathComponent("Nola", isDirectory: true)
            .appendingPathComponent("Logs", isDirectory: true)
        return dir.appendingPathComponent("nola.log", isDirectory: false)
    }

    public func appendLine(_ line: String) {
        queue.async { [weak self] in
            guard let self else { return }
            self.appendLineSync(line)
        }
    }

    public func clear() async {
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            queue.async { [weak self] in
                defer { continuation.resume(returning: ()) }
                guard let self else { return }
                let url = self.logFileURL()
                do {
                    try self.ensureDirectoryExists(for: url)
                    if self.fileManager.fileExists(atPath: url.path) {
                        try self.fileManager.removeItem(at: url)
                    }
                } catch {
                    // Best-effort: swallow errors.
                }
            }
        }
    }

    public func readTail(maxBytes: Int = 200_000) async -> String {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(returning: "")
                    return
                }
                let url = self.logFileURL()
                do {
                    let data = try self.readTailData(from: url, maxBytes: maxBytes)
                    let text = String(data: data, encoding: .utf8) ?? ""
                    continuation.resume(returning: text)
                } catch {
                    continuation.resume(returning: "")
                }
            }
        }
    }

    /// Read only the last `maxLines` lines from the log file.
    ///
    /// This is designed for UI display to avoid loading and rendering large logs.
    public func readLastLines(maxLines: Int = 100, maxBytes: Int = 250_000) async -> String {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(returning: "")
                    return
                }

                guard maxLines > 0 else {
                    continuation.resume(returning: "")
                    return
                }

                let url = self.logFileURL()
                do {
                    let (data, startOffset) = try self.readTailDataWithOffset(from: url, maxBytes: maxBytes)
                    let sliced = Self.dataForLastLines(data, maxLines: maxLines, dropPartialFirstLine: startOffset > 0)
                    let text = String(data: sliced, encoding: .utf8) ?? ""
                    continuation.resume(returning: text)
                } catch {
                    continuation.resume(returning: "")
                }
            }
        }
    }

    public func currentFileSizeBytes() async -> Int64 {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(returning: 0)
                    return
                }
                let url = self.logFileURL()
                let size = (try? self.fileSize(url: url)) ?? 0
                continuation.resume(returning: size)
            }
        }
    }

    // MARK: - Internals

    private func appendLineSync(_ line: String) {
        let url = logFileURL()
        do {
            try ensureDirectoryExists(for: url)
            try rotateIfNeeded(url: url)

            if !fileManager.fileExists(atPath: url.path) {
                fileManager.createFile(atPath: url.path, contents: nil)
            }

            let handle = try FileHandle(forWritingTo: url)
            defer { try? handle.close() }
            try handle.seekToEnd()

            // Always terminate lines for easier tail reading.
            let lineWithNewline = line.hasSuffix("\n") ? line : (line + "\n")
            if let data = lineWithNewline.data(using: .utf8) {
                try handle.write(contentsOf: data)
            }
        } catch {
            // Best-effort: swallow errors.
        }
    }

    private func ensureDirectoryExists(for fileURL: URL) throws {
        let dir = fileURL.deletingLastPathComponent()
        if !fileManager.fileExists(atPath: dir.path) {
            try fileManager.createDirectory(at: dir, withIntermediateDirectories: true)
        }
    }

    private func fileSize(url: URL) throws -> Int64 {
        let attrs = try fileManager.attributesOfItem(atPath: url.path)
        return (attrs[.size] as? NSNumber)?.int64Value ?? 0
    }

    private func rotateIfNeeded(url: URL) throws {
        guard fileManager.fileExists(atPath: url.path) else { return }
        let size = try fileSize(url: url)
        guard size >= maxBytesPerFile else { return }

        // Rotate: nola.log -> nola.log.1 -> nola.log.2 ...
        for i in stride(from: maxArchives, through: 1, by: -1) {
            let src = url.appendingPathExtension(String(i))
            let dst = url.appendingPathExtension(String(i + 1))
            if fileManager.fileExists(atPath: src.path) {
                if i == maxArchives {
                    try? fileManager.removeItem(at: src)
                } else {
                    try? fileManager.removeItem(at: dst)
                    try fileManager.moveItem(at: src, to: dst)
                }
            }
        }

        let firstArchive = url.appendingPathExtension("1")
        try? fileManager.removeItem(at: firstArchive)
        try fileManager.moveItem(at: url, to: firstArchive)

        // Create a fresh file for new logs.
        fileManager.createFile(atPath: url.path, contents: nil)
    }

    private func readTailDataWithOffset(from url: URL, maxBytes: Int) throws -> (data: Data, startOffset: Int64) {
        guard fileManager.fileExists(atPath: url.path) else { return (Data(), 0) }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }

        let size = try fileSize(url: url)
        let maxBytesI64 = Int64(max(0, maxBytes))
        let startOffset = max(Int64(0), size - maxBytesI64)
        try handle.seek(toOffset: UInt64(startOffset))
        let data = try handle.readToEnd() ?? Data()
        return (data, startOffset)
    }

    private static func dataForLastLines(_ data: Data, maxLines: Int, dropPartialFirstLine: Bool) -> Data {
        guard maxLines > 0, !data.isEmpty else { return Data() }

        // Trim trailing newlines to avoid returning an empty last line.
        var end = data.count
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            let bytes = raw.bindMemory(to: UInt8.self)
            while end > 0 {
                let b = bytes[end - 1]
                if b == 10 || b == 13 { // \n or \r
                    end -= 1
                } else {
                    break
                }
            }
        }

        guard end > 0 else { return Data() }

        var start = 0
        var newlineCount = 0

        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            let bytes = raw.bindMemory(to: UInt8.self)
            if end == 0 { return }

            var i = end - 1
            while true {
                if bytes[i] == 10 { // \n
                    newlineCount += 1
                    if newlineCount == maxLines {
                        start = i + 1
                        break
                    }
                }
                if i == 0 { break }
                i -= 1
            }

            // If we started reading mid-file, the first line might be partial.
            // Prefer dropping the partial first line for cleaner display.
            if start == 0 && dropPartialFirstLine {
                var j = 0
                while j < end {
                    if bytes[j] == 10 {
                        start = min(j + 1, end)
                        break
                    }
                    j += 1
                }
            }
        }

        guard start < end else { return Data() }
        return data.subdata(in: start..<end)
    }

    private func readTailData(from url: URL, maxBytes: Int) throws -> Data {
        guard fileManager.fileExists(atPath: url.path) else { return Data() }
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }

        let size = try fileSize(url: url)
        let maxBytesI64 = Int64(max(0, maxBytes))
        let startOffset = max(Int64(0), size - maxBytesI64)
        try handle.seek(toOffset: UInt64(startOffset))
        return try handle.readToEnd() ?? Data()
    }
}
