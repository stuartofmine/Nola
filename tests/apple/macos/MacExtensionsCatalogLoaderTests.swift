// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor
final class MacExtensionsCatalogLoaderTests: XCTestCase {
    private func item(_ id: String, source: MacExtensionItem.Source = .workspace) -> MacExtensionItem {
        .init(id: id, kind: .skill, source: source, title: id, detail: "Fixture", path: nil, isEnabled: true)
    }

    func testAllRefreshIncludesHostAndFailedRefreshPreservesLastSuccessfulCatalog() async {
        var hostCalls = 0
        var failHost = false
        let local = item("local")
        let host = item("remote", source: .connectedHost)
        let loader = MacExtensionsCatalogLoader(local: { _ in [local] }, host: {
            hostCalls += 1
            if failHost { throw NSError(domain: "fixture", code: 1) }
            return [host]
        }, removedSkills: { false })
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(loader.items, [local, host])
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(hostCalls, 2)
        XCTAssertEqual(loader.items, [local, host])
        failHost = true
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(loader.items, [local, host])
        XCTAssertNotNil(loader.errorMessage)
        XCTAssertFalse(loader.isLoading)
        failHost = false
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertNil(loader.errorMessage)
        await loader.load(includeHost: false, workspaceURL: nil)
        XCTAssertEqual(hostCalls, 4)
        XCTAssertEqual(loader.items, [local])
    }

    func testLatestSourceWinsWhenEarlierLocalDiscoveryFinishesLast() async {
        let entered = expectation(description: "Earlier local discovery suspended")
        var pending: CheckedContinuation<[MacExtensionItem], Never>?
        var calls = 0
        var hostCalls = 0
        let local = item("new-local")
        let remote = item("remote", source: .connectedHost)
        let loader = MacExtensionsCatalogLoader(local: { _ in
            calls += 1
            if calls == 1 {
                return await withCheckedContinuation { continuation in
                    pending = continuation
                    entered.fulfill()
                }
            }
            return [local]
        }, host: { hostCalls += 1; return [remote] }, removedSkills: { false })
        let stale = Task { await loader.load(includeHost: false, workspaceURL: nil) }
        await fulfillment(of: [entered], timeout: 2)
        XCTAssertTrue(loader.isLoading)
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(loader.items, [local, remote])
        pending?.resume(returning: [item("stale-local")])
        await stale.value
        XCTAssertEqual(loader.items, [local, remote])
        XCTAssertEqual(hostCalls, 1)
        XCTAssertFalse(loader.isLoading)
    }

    func testDisappearanceInvalidatesInFlightHostPublication() async {
        let entered = expectation(description: "Host query suspended")
        var pending: CheckedContinuation<[MacExtensionItem], Never>?
        let loader = MacExtensionsCatalogLoader(local: { _ in [] }, host: {
            await withCheckedContinuation { continuation in
                pending = continuation
                entered.fulfill()
            }
        }, removedSkills: { false })
        let request = Task { await loader.load(includeHost: true, workspaceURL: nil) }
        await fulfillment(of: [entered], timeout: 2)
        loader.invalidate()
        pending?.resume(returning: [item("stale-host", source: .connectedHost)])
        await request.value
        XCTAssertTrue(loader.items.isEmpty)
        XCTAssertFalse(loader.isLoading)
    }
}
