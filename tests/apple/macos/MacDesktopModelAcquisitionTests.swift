// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacDesktopModelAcquisitionTests: XCTestCase {
    private let fm = FileManager.default

    private func temporaryRoot() throws -> URL {
        let root = fm.temporaryDirectory.appendingPathComponent("NolaHubTests-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return root
    }

    private func wait(for phase: MacLocalModelAcquisition.Phase, in model: MacLocalModelAcquisition) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while model.phase != phase && ContinuousClock.now < deadline {
            if case let .failed(message) = model.phase { XCTFail(message); return }
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTAssertEqual(model.phase, phase)
    }

    func testRecommendationsRequireConfirmationForEmptyAndExactNamesAndKeepThePinnedPlan() async throws {
        let root = try temporaryRoot()
        let executable = root.appendingPathComponent("owned-test-host")
        let commands = root.appendingPathComponent("commands")
        let candidate = #"{"repository":"two/model","variant":"Q4","revision":"r1","downloadBytes":64,"estimatedMemoryBytes":128,"contextTokens":1024,"plan":{"expectedRevision":"r1","files":["w.gguf"]}}"#
        let hardware = #"{"cpu":"Test CPU","cores":8,"memoryBytes":4000000000,"gpu":"Test GPU","unifiedMemory":true,"freeDiskBytes":8000000000}"#
        try """
        #!/bin/sh
        [ "$1 $2" = "model acquire" ] && [ "$NOLA_MODELS_ROOT" = "$TEST_ROOT" ] || exit 3
        printf '%s|%s|%s|%s\\n' "$3" "$4" "$5" "$6" >> "$TEST_ROOT/commands"
        case "$3" in
          hardware) echo '{"hardware":\(hardware)}';;
          recommend) echo '{"hardware":\(hardware),"recommendationSource":"hub","candidates":[\(candidate)]}';;
          confirm) echo '{"downloadedBytes":32,"totalBytes":64}'; echo '{"model":{"id":"owned"}}';;
          *) exit 4;;
        esac
        """.write(to: executable, atomically: true, encoding: .utf8)
        try fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let model = MacLocalModelAcquisition(client: .init(executable: executable, root: root, environment: ["TEST_ROOT": root.path]))
        await model.loadHardware()
        XCTAssertEqual(model.hardware?.cpu, "Test CPU")
        model.recommendOnOpen(engine: .llamaCpp, source: "modelscope")
        model.recommendOnOpen(engine: .llamaCpp, source: "modelscope")
        try await wait(for: .choosing, in: model)
        model.updateInputs(engine: .llamaCpp, source: "modelscope")
        model.recommendOnOpen(engine: .llamaCpp, source: "modelscope")
        XCTAssertEqual(model.candidates.first?.estimatedMemoryBytes, 128)
        model.query = "two/model"
        model.updateInputs(engine: .vllm, source: "huggingface")
        XCTAssertEqual(model.phase, .idle)
        model.recommend(engine: .vllm, source: "huggingface", name: model.query)
        try await wait(for: .choosing, in: model)
        XCTAssertEqual(try String(contentsOf: commands, encoding: .utf8).split(separator: "\n"), [
            "hardware|||", "recommend|llama.cpp|modelscope|", "recommend|vllm|huggingface|two/model"
        ])
        let offered = try XCTUnwrap(model.candidates.first)
        model.confirm(offered)
        model.confirm(offered)
        try await wait(for: .installed("owned"), in: model)
        XCTAssertEqual(model.received, 32)
        XCTAssertEqual(model.total, 64)
        let lines = try String(contentsOf: commands, encoding: .utf8).split(separator: "\n")
        XCTAssertEqual(lines.count, 4)
        let arguments = try XCTUnwrap(lines.last).split(separator: "|")
        XCTAssertEqual(Array(arguments.prefix(3)), ["confirm", "vllm", "huggingface"])
        let sent = try JSONDecoder().decode([String: MacCanonicalJSONValue].self, from: Data(try XCTUnwrap(arguments.last).utf8))
        XCTAssertEqual(sent, offered.plan)
    }

    func test128KRecommendationsRequireAKnownNativeWindowAndDisplayShorterEstimates() throws {
        func decode(_ context: Int, known: Bool?, nativeLimit: Int?, reason: String) throws -> MacModelAcquisitionEvent.Candidate {
            var values: [String: Any] = ["repository": "unsloth/test", "revision": "rev", "variant": "Q4",
                "downloadBytes": 512, "estimatedMemoryBytes": 1024, "contextTokens": context, "contextLimitReason": reason,
                "plan": ["contextTokens": context]]
            values["nativeContextKnown"] = known
            values["nativeContextTokens"] = nativeLimit
            return try JSONDecoder().decode(MacModelAcquisitionEvent.Candidate.self,
                from: JSONSerialization.data(withJSONObject: values))
        }
        let complete = try decode(131_072, known: true, nativeLimit: 131_072, reason: "target_met")
        XCTAssertTrue(complete.estimates128K)
        for value in [
            try decode(65_536, known: true, nativeLimit: 131_072, reason: "memory_budget"),
            try decode(32_768, known: true, nativeLimit: 32_768, reason: "native_limit"),
            try decode(131_072, known: false, nativeLimit: nil, reason: "metadata_unknown"),
            try decode(131_072, known: nil, nativeLimit: nil, reason: "metadata_unknown")
        ] {
            XCTAssertFalse(value.estimates128K)
            XCTAssertNotEqual(value.contextDescription, complete.contextDescription)
            XCTAssertTrue(value.contextDescription.contains(value.contextTokens.formatted()))
        }
    }

    func testCompatibleSnapshotsAreListedWithTheirOwnNativeContextAndEntryArtifact() throws {
        let root = try temporaryRoot()
        var records: [[String: Any]] = []
        for engine in MacLocalInferenceEngine.allCases {
            let directory = root.appendingPathComponent("catalog-\(engine.rawValue)/snapshots/fixture", isDirectory: true)
            try fm.createDirectory(at: directory, withIntermediateDirectories: true)
            let files = engine == .llamaCpp ? ["weights.gguf"] : ["config.json", "model.safetensors"]
            for file in files { try Data([1]).write(to: directory.appendingPathComponent(file)) }
            try JSONSerialization.data(withJSONObject: ["totalBytes":files.count, "artifacts":files.map { ["path":$0,"bytes":1] as [String:Any] }])
                .write(to: directory.appendingPathComponent("snapshot.json"))
            records.append(["id":engine.rawValue,"name":engine.title,"state":"configured","runtime":engine.rawValue,
                            "format":engine.format,"artifactBytes":files.count,"contextWindowTokens":2048,
                            "snapshotPath":directory.path,"artifactPath":directory.appendingPathComponent(files[0]).path])
        }
        var escaped = records[0]; escaped["id"] = "escape"; escaped["snapshotPath"] = root.deletingLastPathComponent().path
        var mismatched = records[1]; mismatched["id"] = "wrong-format"; mismatched["format"] = "mlx"
        var missing = records[1]; missing["id"] = "missing-entry"; missing["artifactPath"] = root.appendingPathComponent("other.gguf").path
        try JSONSerialization.data(withJSONObject: ["models":records + [escaped, mismatched, missing]])
            .write(to: root.appendingPathComponent("catalog.json"))
        let profiles = MacLocalModelProfile.installed(in: root)
        XCTAssertEqual(Set(profiles.map(\.id)), Set(MacLocalInferenceEngine.allCases.map(\.rawValue)))
        XCTAssertTrue(profiles.allSatisfy { $0.contextTokens == 2048 })
        let llama = try XCTUnwrap(profiles.first { $0.engine == .llamaCpp })
        let vllm = try XCTUnwrap(profiles.first { $0.engine == .vllm })
        for profile in [llama, vllm] {
            let launch = try MacExternalLocalModelLaunch(profile: profile, token: "fixture", context: 2048,
                environment: ["NOLA_LLAMA_SERVER":"/usr/bin/true","NOLA_VLLM_BIN":"/usr/bin/true"])
            XCTAssertTrue(launch.arguments.contains("127.0.0.1"))
            XCTAssertTrue(launch.arguments.contains("2048"))
            XCTAssertTrue(launch.arguments.contains(profile.id))
            XCTAssertTrue(launch.arguments.contains(try XCTUnwrap(profile.engine == .llamaCpp ? profile.artifactURL?.path : profile.installedDirectory?.path)))
            let endpoint = MacMLXOpenAIService.Endpoint(baseURL: "http://127.0.0.1/v1", apiKey: "fixture",
                modelID: profile.id, contextWindowTokens: 2048, profileRevision: 1, engine: try XCTUnwrap(profile.engine))
            XCTAssertEqual(try endpoint.nativeRoute()["adapter"] as? String, profile.engine?.adapter)
        }
        let catalog = root.appendingPathComponent("catalog.json")
        var configured = records[1]
        configured["recommendedContextTokens"] = 1024
        configured["recommendedDevice"] = "cpu"
        try JSONSerialization.data(withJSONObject: ["schemaVersion": 1, "models": [configured]]).write(to: catalog)
        let recommended = try XCTUnwrap(MacLocalModelProfile.installed(in: root).first)
        XCTAssertEqual(recommended.contextTokens, 2048)
        XCTAssertEqual(recommended.defaultParameters.maxContextLength, 1024)
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("manual.sqlite"))
        let settings = MacLocalModelConfigurationStore(applicationPort: application)
        var manual = settings.parameters(for: recommended)
        manual.maxContextLength = 1536
        try settings.persist(profile: recommended, selected: true, parameters: manual)
        try settings.persist(profile: recommended, selected: true)
        XCTAssertEqual(settings.parameters(for: recommended).maxContextLength, 1536)
        let launch = try MacExternalLocalModelLaunch(profile: recommended, token: "fixture", context: 1024,
            environment: ["NOLA_LLAMA_SERVER": "/usr/bin/true"])
        XCTAssertTrue(launch.arguments.contains("--n-gpu-layers"))
        let slots = try XCTUnwrap(launch.arguments.firstIndex(of: "--parallel"))
        XCTAssertEqual(launch.arguments[slots + 1], "1")
        let tools = root.appendingPathComponent("bin")
        try fm.createDirectory(at: tools, withIntermediateDirectories: true)
        let unified = tools.appendingPathComponent("llama")
        try fm.createSymbolicLink(at: unified, withDestinationURL: URL(fileURLWithPath: "/usr/bin/true"))
        let fallback = try MacExternalLocalModelLaunch(profile: recommended, token: "fixture", context: 1024,
            environment: ["NOLA_LLAMA_SERVER": root.appendingPathComponent("missing").path, "PATH": tools.path])
        XCTAssertEqual(fallback.executable, unified)
        XCTAssertEqual(fallback.arguments.first, "serve")
        configured["contextWindowTokens"] = 262_144
        configured["recommendedContextTokens"] = 131_072
        try JSONSerialization.data(withJSONObject: ["schemaVersion": 1, "models": [configured]]).write(to: catalog)
        let longContext = try XCTUnwrap(MacLocalModelProfile.installed(in: root).first)
        XCTAssertEqual(longContext.defaultParameters.maxContextLength, 131_072)
        XCTAssertEqual(settings.parameters(for: longContext).maxContextLength, 1536)
        var unknownContext = records[0]
        unknownContext["id"] = "old-skill-without-context"
        unknownContext["contextWindowTokens"] = NSNull()
        try JSONSerialization.data(withJSONObject: ["schemaVersion": 2, "models": records + [unknownContext]])
            .write(to: catalog)
        let mixed = MacLocalModelProfile.installed(in: root)
        XCTAssertEqual(mixed.count, 4)
        XCTAssertEqual(mixed.first { $0.id == "old-skill-without-context" }?.contextTokens, 4096)
        for version in [1, 2] {
            try JSONSerialization.data(withJSONObject: ["schemaVersion": version, "models": records])
                .write(to: catalog)
            XCTAssertEqual(MacLocalModelProfile.installed(in: root).count, 3)
        }
        let manifest = try XCTUnwrap(llama.installedDirectory).appendingPathComponent("snapshot.json")
        let original = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: manifest)) as? [String: Any])
        for version: Any in [12, NSNull(), "1"] as [Any] {
            var modified = original
            modified["schemaVersion"] = version
            try JSONSerialization.data(withJSONObject: modified).write(to: manifest)
            XCTAssertEqual(MacLocalModelProfile.installed(in: root).count, 2)
        }
        for version: Any in [3, NSNull(), "1"] as [Any] {
            let bytes = try JSONSerialization.data(withJSONObject: ["schemaVersion": version, "models": records])
            try bytes.write(to: catalog)
            XCTAssertTrue(MacLocalModelProfile.installed(in: root).isEmpty)
            XCTAssertEqual(try Data(contentsOf: catalog), bytes)
        }
    }

    func testChinaUTC8DefaultsToModelScopeAndDirectURLParsingDetectsSource() {
        let shanghai = TimeZone(identifier: "Asia/Shanghai")!
        let newYork = TimeZone(identifier: "America/New_York")!
        XCTAssertEqual(
            MacLocalModelAcquisition.defaultDownloadSource(timeZone: shanghai, locale: Locale(identifier: "en_US")),
            "modelscope"
        )
        XCTAssertEqual(
            MacLocalModelAcquisition.defaultDownloadSource(timeZone: newYork, locale: Locale(identifier: "zh_Hans_CN")),
            "modelscope"
        )
        XCTAssertEqual(
            MacLocalModelAcquisition.defaultDownloadSource(timeZone: newYork, locale: Locale(identifier: "en_US")),
            "huggingface"
        )

        let ms = MacLocalModelAcquisition.parseDirectInput("https://modelscope.cn/models/Qwen/Qwen2.5-7B-Instruct/files")
        XCTAssertEqual(ms.source, "modelscope")
        XCTAssertEqual(ms.query, "Qwen/Qwen2.5-7B-Instruct")

        let hf = MacLocalModelAcquisition.parseDirectInput("https://huggingface.co/mlx-community/Qwen3-4B-4bit/tree/main")
        XCTAssertEqual(hf.source, "huggingface")
        XCTAssertEqual(hf.query, "mlx-community/Qwen3-4B-4bit")

        let plain = MacLocalModelAcquisition.parseDirectInput("mlx-community/Qwen3-4B-4bit")
        XCTAssertNil(plain.source)
        XCTAssertEqual(plain.query, "mlx-community/Qwen3-4B-4bit")
    }

    func testSessionCachesHardwareAndRecommendationsWithoutRespawningSubprocess() async throws {
        let root = try temporaryRoot()
        let executable = root.appendingPathComponent("cache-test-host")
        let commands = root.appendingPathComponent("commands")
        let candidate = #"{"repository":"mlx-community/Qwen3-4B-MLX-4bit","variant":"4bit","revision":"r1","downloadBytes":2500000000,"estimatedMemoryBytes":4000000000,"contextTokens":32768,"plan":{"expectedRevision":"r1","files":["model.safetensors"]}}"#
        let hardware = #"{"cpu":"Apple M2","cores":8,"memoryBytes":16000000000,"gpu":"Apple M2","unifiedMemory":true,"freeDiskBytes":80000000000}"#
        try """
        #!/bin/sh
        printf '%s|%s|%s|%s\\n' "$3" "$4" "$5" "$6" >> "$TEST_ROOT/commands"
        case "$3" in
          hardware) echo '{"hardware":\(hardware)}';;
          recommend) echo '{"hardware":\(hardware),"recommendationSource":"hub","candidates":[\(candidate)]}';;
          *) exit 4;;
        esac
        """.write(to: executable, atomically: true, encoding: .utf8)
        try fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)

        let model = MacLocalModelAcquisition(client: .init(executable: executable, root: root, environment: ["TEST_ROOT": root.path]))
        await model.loadHardware()
        await model.loadHardware()
        model.recommend(engine: .mlx, source: "modelscope", name: "")
        try await wait(for: .choosing, in: model)
        model.reset()
        model.recommend(engine: .mlx, source: "modelscope", name: "")
        XCTAssertEqual(model.phase, .choosing)
        XCTAssertEqual(model.candidates.first?.displayTitle, "Qwen3 4B")
        XCTAssertEqual(model.candidates.first?.fitBadge(deviceMemoryBytes: 16_000_000_000).isComfortable, true)
        let lines = try String(contentsOf: commands, encoding: .utf8).split(separator: "\n")
        XCTAssertEqual(lines, ["hardware|||", "recommend|mlx|modelscope|"])
    }
}
