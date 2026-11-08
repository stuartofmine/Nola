// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises multi-artifact installation, atomic activation, and crash recovery.

import Foundation
import XCTest

final class MacOpenSourceModelInstallerTransactionTests: XCTestCase {
    private var repositoryRoot: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
    }

    private var installer: URL {
        repositoryRoot
            .appendingPathComponent("NolaMac/Resources/NolaBuiltInSkills.bundle/install-open-source-model/scripts/install_model_variant.py")
    }

    private func writeJSON(_ value: Any, to url: URL) throws {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
    }

    @discardableResult
    private func run(_ arguments: [String]) throws -> (status: Int32, output: String) {
        let process = Process()
        let pipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [installer.path] + arguments
        process.standardOutput = pipe
        process.standardError = pipe
        try process.run()
        process.waitUntilExit()
        let output = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        return (process.terminationStatus, output)
    }

    func testMultiArtifactSnapshotActivatesOnlyAfterEveryArtifactIsVerified() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-transaction-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)

        let first = temporary.appendingPathComponent("config.json")
        let second = temporary.appendingPathComponent("weights.bin")
        try Data("{\"model_type\":\"qwen\"}".utf8).write(to: first)
        try Data(repeating: 0x5A, count: 8_192).write(to: second)
        let total = try first.resourceValues(forKeys: [.fileSizeKey]).fileSize!
            + second.resourceValues(forKeys: [.fileSizeKey]).fileSize!
        let plan = temporary.appendingPathComponent("plan.json")
        try writeJSON([
            "status": "recommended",
            "model": "example/Qwen-Test",
            "selected": [
                "repo_id": "example/Qwen-Test",
                "name": "Qwen Test",
                "revision": "commit-123",
                "runtime": "mlx",
                "format": "mlx",
                "size_bytes": total,
                "context_length": 65_536,
            ],
        ], to: plan)
        let manifest = temporary.appendingPathComponent("artifacts.json")
        try writeJSON([
            "revision": "commit-123",
            "artifacts": [
                ["path": "config.json", "url": first.absoluteString, "size_bytes": Data("{\"model_type\":\"qwen\"}".utf8).count],
                ["path": "model/weights.bin", "url": second.absoluteString, "size_bytes": 8_192],
            ],
        ], to: manifest)
        let models = temporary.appendingPathComponent("models", isDirectory: true)

        let result = try run([
            "--plan", plan.path,
            "--models-root", models.path,
            "--artifacts-json", manifest.path,
            "--task-id", "transaction-test",
            "--max-retries", "0",
        ])
        XCTAssertEqual(result.status, 0, result.output)

        let catalog = try JSONSerialization.jsonObject(with: Data(contentsOf: models.appendingPathComponent("catalog.json"))) as! [String: Any]
        let record = (catalog["models"] as! [[String: Any]]).first!
        let snapshot = URL(fileURLWithPath: record["snapshotPath"] as! String)
        XCTAssertTrue(FileManager.default.fileExists(atPath: snapshot.appendingPathComponent("config.json").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: snapshot.appendingPathComponent("model/weights.bin").path))
        XCTAssertEqual(record["artifactBytes"] as? Int, total)
        XCTAssertEqual(record["state"] as? String, "configured")
    }

    func testDeadOwnerLockIsRecovered() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-stale-lock-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let source = temporary.appendingPathComponent("model.gguf")
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        try Data(repeating: 0x41, count: 32).write(to: source)
        let plan = temporary.appendingPathComponent("plan.json")
        try writeJSON([
            "status": "recommended",
            "model": "example/stale",
            "selected": ["repo_id": "example/stale", "revision": "r1", "runtime": "llama.cpp", "format": "gguf", "size_bytes": 32, "context_length": 65_536],
        ], to: plan)
        let models = temporary.appendingPathComponent("models", isDirectory: true)
        let lock = models.appendingPathComponent("example-stale/.install.lock", isDirectory: true)
        try FileManager.default.createDirectory(at: lock, withIntermediateDirectories: true)
        try writeJSON(["pid": 999_999_999, "heartbeatEpoch": Int(Date().timeIntervalSince1970)], to: lock.appendingPathComponent("owner.json"))

        let result = try run([
            "--plan", plan.path,
            "--models-root", models.path,
            "--artifact-url", source.absoluteString,
            "--filename", "model.gguf",
            "--task-id", "stale-lock-test",
            "--max-retries", "0",
        ])
        XCTAssertEqual(result.status, 0, result.output)
        XCTAssertFalse(FileManager.default.fileExists(atPath: lock.path))
    }
}
