// SPDX-License-Identifier: MIT

import XCTest
@testable import Nola

@MainActor
final class MacLocalModelCatalogTests: XCTestCase {
    func testRuntimeStatusRetainsStartupTimingAcrossPhases() {
        let startedAt = Date(timeIntervalSince1970: 1_000)
        let status = MacLocalModelRuntimeStatus(
            phase: .loadingWeights,
            modelID: "model-id",
            displayName: "Model",
            errorDescription: nil,
            startedAt: startedAt,
            phaseStartedAt: Date(timeIntervalSince1970: 1_005)
        )

        XCTAssertTrue(status.isStarting)
        XCTAssertEqual(status.startedAt, startedAt)
        XCTAssertEqual(status.phaseStartedAt, Date(timeIntervalSince1970: 1_005))
        XCTAssertEqual(status.elapsedSeconds(at: Date(timeIntervalSince1970: 1_125)), 125)
    }

    func testRuntimeStatusStreamImmediatelyPublishesCurrentSnapshot() async throws {
        MacMLXOpenAIService.shared.stop()
        var iterator = MacMLXOpenAIService.shared.statusUpdates().makeAsyncIterator()
        let initial = await iterator.next()
        XCTAssertEqual(initial, .idle)
    }

    func testRuntimeStatusStreamDoesNotPollWhenNothingChanges() async {
        MacMLXOpenAIService.shared.stop()
        let duplicate = expectation(description: "unchanged runtime status must not republish")
        duplicate.isInverted = true
        let observer = Task { @MainActor in
            var iterator = MacMLXOpenAIService.shared.statusUpdates().makeAsyncIterator()
            _ = await iterator.next()
            if await iterator.next() != nil {
                duplicate.fulfill()
            }
        }
        await fulfillment(of: [duplicate], timeout: 1.2)
        observer.cancel()
    }

    func testFastEvaluationModelPickerHotPath() throws {
        let store = MacLocalModelConfigurationStore.shared
        store.invalidateReadinessCache()
        _ = store.isReady(.qwen3_5_0_8b)

        let iterations = 20_000
        let clock = ContinuousClock()
        let elapsed = clock.measure {
            for _ in 0..<iterations {
                _ = store.isReady(.qwen3_5_0_8b)
                _ = store.isReady(.qwen3_5_4b)
            }
        }
        let seconds = Double(elapsed.components.seconds)
            + Double(elapsed.components.attoseconds) / 1_000_000_000_000_000_000
        let secondsPerLookup = seconds / Double(iterations * 2)
        XCTAssertLessThan(
            secondsPerLookup,
            0.000_025,
            "Opening or switching the model picker must not repeat model-directory scans"
        )

        print("NOLA_PERF_MODEL_PICKER_SECONDS_PER_LOOKUP=\(secondsPerLookup)")
        let metrics: [String: Any] = [
            "model_picker_seconds_per_lookup": secondsPerLookup,
        ]
        let metricsURL = URL(fileURLWithPath: "/tmp/nola-fast-evaluation-mac-metrics.json")
        try JSONSerialization.data(withJSONObject: metrics, options: [.sortedKeys])
            .write(to: metricsURL, options: .atomic)
    }

    func testEveryBuiltInModelMeetsMinimumContextContract() {
        XCTAssertEqual(MacLocalModelProfile.all.count, 2)
        for model in MacLocalModelProfile.all {
            XCTAssertGreaterThanOrEqual(model.contextTokens, 65_536)
            XCTAssertGreaterThanOrEqual(model.nativeContextTokens, model.contextTokens)
            XCTAssertEqual(model.contextTokens, model.nativeContextTokens)
            XCTAssertGreaterThan(model.downloadBytes, 0)
            XCTAssertGreaterThan(model.estimatedRuntimeBytesAt64K, model.downloadBytes)
            XCTAssertGreaterThan(model.recommendedUnifiedMemoryBytes, 0)
        }
    }

    func testQwen35DownloadContractContainsWeightIndex() {
        for model in ModelDownloader.ModelType.allCases {
            let listedNames = model.requiredFiles.map(\.name)
            let names = Set(listedNames)
            XCTAssertEqual(names.count, listedNames.count, "A model file must be downloaded exactly once")
            XCTAssertTrue(names.contains("model.safetensors"))
            XCTAssertTrue(
                names.contains("model.safetensors.index.json"),
                "Qwen3.5 MLX packages publish an explicit weight index required by local loading"
            )
        }
    }

    func testEveryDownloadSourceUsesItsOwnRepository() throws {
        for model in ModelDownloader.ModelType.allCases {
            let huggingFace = try XCTUnwrap(model.fileURL(fileName: "config.json", source: .huggingface))
            let modelScope = try XCTUnwrap(model.fileURL(fileName: "config.json", source: .modelscope))
            XCTAssertEqual(huggingFace.host, "huggingface.co")
            XCTAssertEqual(modelScope.host, "modelscope.cn")
            XCTAssertNotEqual(huggingFace, modelScope, "The source picker must change the actual download route")
            XCTAssertTrue(model.pageURL(from: .modelscope).contains("modelscope.cn"))
        }
    }

    func testDownloadedModelValidationRejectsTruncatedOrMalformedCache() throws {
        let model = ModelDownloader.ModelType.qwen3_5_0_8b
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-validation-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        for file in model.requiredFiles {
            let data: Data
            if file.name.hasSuffix(".json") {
                data = Data("{}".utf8)
            } else {
                data = Data("fixture".utf8)
            }
            try data.write(to: root.appendingPathComponent(file.name))
        }
        XCTAssertFalse(
            model.validatesDownloadedFiles(at: root),
            "A truncated weight file must never be presented as a ready local model"
        )
    }

    func testCatalogResolvesOnlyCanonicalModelIdentity() {
        XCTAssertEqual(
            MacLocalModelProfile.resolve("mlx-community/Qwen3.5-4B-MLX-4bit"),
            .qwen35_4B
        )
        XCTAssertNil(MacLocalModelProfile.resolve("Qwen3.5 0.8B"))
        XCTAssertNil(MacLocalModelProfile.resolve("Qwen3.5-4B-MLX-4bit"))
        XCTAssertNil(MacLocalModelProfile.resolve("local:mlx-community/Qwen3.5-4B-MLX-4bit"))
        XCTAssertNil(MacLocalModelProfile.resolve(" mlx-community/Qwen3.5-4B-MLX-4bit "))
    }

    func testLoopbackProviderRequiresExplicitStart() {
        MacMLXOpenAIService.shared.stop()
        XCTAssertThrowsError(try MacMLXOpenAIService.shared.endpoint(for: .qwen35_08B))
        XCTAssertNil(MacMLXOpenAIService.shared.runningModelID)
        XCTAssertEqual(MacMLXOpenAIService.shared.status, .idle)
    }

    func testLocalModelReadinessRequiresTheSelectedModelToBeRunning() throws {
        let store = MacLocalModelConfigurationStore.shared
        let previous = store.selectedModel
        defer { try? store.select(previous) }
        try store.select(.qwen3_5_0_8b)
        MacMLXOpenAIService.shared.stop()

        let issue = MacReadinessService.blockingIssue(for: .localModel)
        XCTAssertNotNil(issue, "A downloaded selection must not be treated as a running endpoint")
    }

    func testSelectingModelDoesNotStartLoopbackProvider() throws {
        let store = MacLocalModelConfigurationStore.shared
        let previous = store.selectedModel
        defer { try? store.select(previous) }

        MacMLXOpenAIService.shared.stop()
        try store.select(.qwen3_5_0_8b)

        XCTAssertEqual(store.selectedModel, .qwen3_5_0_8b)
        XCTAssertFalse(store.isSelectedModelRunning)
        XCTAssertNil(MacMLXOpenAIService.shared.runningModelID)
    }

}
