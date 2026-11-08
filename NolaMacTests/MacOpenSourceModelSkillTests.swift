import Foundation
import XCTest
@testable import Nola

final class MacOpenSourceModelSkillTests: XCTestCase {
    func testPlannerSelectsNativeVariantAndExplainsRejections() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let script = descriptor.instructionURL.deletingLastPathComponent()
            .appendingPathComponent("scripts/plan_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let hardware = root.appendingPathComponent("hardware.json")
        let candidates = root.appendingPathComponent("candidates.json")
        let output = root.appendingPathComponent("plan.json")
        try Data(#"{"platform":"macos","architecture":"arm64","memory_bytes":17179869184,"available_disk_bytes":107374182400,"runtimes":["mlx","llama.cpp"]}"#.utf8).write(to: hardware)
        try Data(#"{"candidates":[{"repo_id":"publisher/model-mlx","revision":"abc","format":"mlx","runtime":"mlx","quantization":"4bit","parameter_billions":4,"size_bytes":3000000000,"context_length":131072,"architectures":["arm64"]},{"repo_id":"publisher/model-f16","revision":"def","format":"gguf","runtime":"llama.cpp","quantization":"f16","parameter_billions":14,"size_bytes":28000000000,"context_length":131072,"architectures":["arm64"]},{"repo_id":"publisher/model-short","revision":"ghi","format":"gguf","runtime":"llama.cpp","quantization":"q4_k_m","parameter_billions":4,"size_bytes":2500000000,"context_length":32768,"architectures":["arm64"]}]}"#.utf8).write(to: candidates)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, "--model", "publisher/model", "--hardware-json", hardware.path, "--candidates-json", candidates.path, "--minimum-context", "65536", "--output", output.path]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)

        let plan = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any])
        XCTAssertEqual(plan["status"] as? String, "recommended")
        XCTAssertEqual((plan["selected"] as? [String: Any])?["repo_id"] as? String, "publisher/model-mlx")
        XCTAssertEqual(plan["side_effects_performed"] as? Bool, false)
        let rejected = try XCTUnwrap(plan["rejected"] as? [[String: Any]])
        XCTAssertEqual(rejected.count, 2)
        XCTAssertTrue(rejected.flatMap { $0["rejection_reasons"] as? [String] ?? [] }.contains { $0.contains("context") })
        XCTAssertTrue(rejected.flatMap { $0["rejection_reasons"] as? [String] ?? [] }.contains { $0.contains("memory") || $0.contains("disk") })
    }

    func testPlannerReportsNoCompatibleVariantWithoutSideEffects() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let script = descriptor.instructionURL.deletingLastPathComponent()
            .appendingPathComponent("scripts/plan_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let hardware = root.appendingPathComponent("hardware.json")
        let candidates = root.appendingPathComponent("candidates.json")
        let output = root.appendingPathComponent("plan.json")
        try Data(#"{"platform":"linux","architecture":"x86_64","memory_bytes":4294967296,"available_disk_bytes":10737418240,"runtimes":["llama.cpp"]}"#.utf8).write(to: hardware)
        try Data(#"{"candidates":[{"repo_id":"publisher/model-mlx","revision":"abc","format":"mlx","runtime":"mlx","quantization":"4bit","parameter_billions":4,"size_bytes":3000000000,"context_length":131072,"architectures":["arm64"]}]}"#.utf8).write(to: candidates)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, "--model", "publisher/model", "--hardware-json", hardware.path, "--candidates-json", candidates.path, "--minimum-context", "65536", "--output", output.path]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 2)

        let plan = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any])
        XCTAssertEqual(plan["status"] as? String, "no_compatible_variant")
        XCTAssertNil(plan["selected"] as? [String: Any])
        XCTAssertEqual(plan["side_effects_performed"] as? Bool, false)
        XCTAssertFalse((plan["rejected"] as? [[String: Any]] ?? []).isEmpty)
    }

    func testPlannerSupportsNvidiaCUDAAndAMDROCmOrVulkan() throws {
        let nvidia = try runPlanner(
            hardware: #"{"platform":"linux","architecture":"x86_64","gpu_vendor":"nvidia","gpu_memory_bytes":25769803776,"memory_bytes":68719476736,"available_disk_bytes":214748364800,"accelerators":["cuda"],"runtimes":["llama.cpp"]}"#,
            candidateAccelerators: ["cuda", "cpu"]
        )
        XCTAssertEqual((nvidia["selected"] as? [String: Any])?["accelerator"] as? String, "cuda")

        let amd = try runPlanner(
            hardware: #"{"platform":"linux","architecture":"x86_64","gpu_vendor":"amd","gpu_memory_bytes":25769803776,"memory_bytes":68719476736,"available_disk_bytes":214748364800,"rocm_available":true,"vulkan_available":true,"runtimes":["llama.cpp"]}"#,
            candidateAccelerators: ["rocm", "vulkan", "cpu"]
        )
        XCTAssertEqual((amd["selected"] as? [String: Any])?["accelerator"] as? String, "rocm")
    }

    func testInstallerPlacesArtifactAndRegistrationInNolaModelsRoot() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let scripts = descriptor.instructionURL.deletingLastPathComponent().appendingPathComponent("scripts")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let artifact = root.appendingPathComponent("fixture.gguf")
        try Data("verified model fixture".utf8).write(to: artifact)
        let plan = root.appendingPathComponent("plan.json")
        try Data(#"{"status":"recommended","model":"publisher/model","selected":{"repo_id":"publisher/model-gguf","revision":"immutable","runtime":"llama.cpp","accelerator":"cuda","format":"gguf","quantization":"q4_k_m","context_length":131072,"size_bytes":22}}"#.utf8).write(to: plan)
        let models = root.appendingPathComponent("NolaModels")
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [scripts.appendingPathComponent("install_model_variant.py").path, "--plan", plan.path, "--models-root", models.path, "--artifact-url", artifact.absoluteURL.absoluteString, "--filename", "model.gguf", "--task-id", "task-qwen-test"]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        let catalog = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: models.appendingPathComponent("catalog.json"))) as? [String: Any])
        let installed = try XCTUnwrap((catalog["models"] as? [[String: Any]])?.first)
        XCTAssertEqual(installed["accelerator"] as? String, "cuda")
        XCTAssertTrue(FileManager.default.fileExists(atPath: try XCTUnwrap(installed["artifactPath"] as? String)))
        XCTAssertTrue(FileManager.default.fileExists(atPath: models.appendingPathComponent("publisher-model-gguf/model.json").path))
        let task = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: models.appendingPathComponent("publisher-model-gguf/task.json"))) as? [String: Any])
        XCTAssertEqual(task["status"] as? String, "configured")
        XCTAssertEqual(task["taskID"] as? String, "task-qwen-test")
        XCTAssertEqual(task["progress"] as? Double, 1.0)
    }

    func testInstallerResumesExistingPartialArtifact() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let installer = descriptor.instructionURL.deletingLastPathComponent().appendingPathComponent("scripts/install_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let bytes = Data((0..<4096).map { UInt8($0 % 251) })
        let source = root.appendingPathComponent("source.gguf")
        try bytes.write(to: source)
        let plan = root.appendingPathComponent("plan.json")
        try Data(#"{"status":"recommended","model":"publisher/resume","selected":{"repo_id":"publisher/resume","revision":"fixed","runtime":"llama.cpp","accelerator":"vulkan","format":"gguf","quantization":"q4_k_m","context_length":65536,"size_bytes":4096}}"#.utf8).write(to: plan)
        let models = root.appendingPathComponent("models")
        let directory = models.appendingPathComponent("publisher-resume")
        let staging = directory.appendingPathComponent(".staging/fixed-model-install-publisher-resume")
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        let partial = staging.appendingPathComponent("model.gguf.part")
        try bytes.prefix(1536).write(to: partial)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [installer.path, "--plan", plan.path, "--models-root", models.path, "--artifact-url", source.absoluteURL.absoluteString, "--filename", "model.gguf"]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        let recordData = try Data(contentsOf: directory.appendingPathComponent("model.json"))
        let record = try XCTUnwrap(JSONSerialization.jsonObject(with: recordData) as? [String: Any])
        let artifactPath = try XCTUnwrap(record["artifactPath"] as? String)
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: artifactPath)), bytes)
        XCTAssertFalse(FileManager.default.fileExists(atPath: partial.path))
    }

    private func runPlanner(hardware: String, candidateAccelerators: [String]) throws -> [String: Any] {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let script = descriptor.instructionURL.deletingLastPathComponent().appendingPathComponent("scripts/plan_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let hardwareURL = root.appendingPathComponent("hardware.json")
        let candidatesURL = root.appendingPathComponent("candidates.json")
        let output = root.appendingPathComponent("plan.json")
        try Data(hardware.utf8).write(to: hardwareURL)
        let candidate: [String: Any] = ["repo_id": "publisher/model-gguf", "revision": "abc", "format": "gguf", "runtime": "llama.cpp", "quantization": "q4_k_m", "parameter_billions": 4, "size_bytes": 3_000_000_000, "context_length": 131_072, "architectures": ["x86_64"], "accelerators": candidateAccelerators]
        try JSONSerialization.data(withJSONObject: ["candidates": [candidate]]).write(to: candidatesURL)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, "--model", "publisher/model", "--hardware-json", hardwareURL.path, "--candidates-json", candidatesURL.path, "--minimum-context", "65536", "--output", output.path]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any])
    }
}
