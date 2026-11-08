// SPDX-License-Identifier: Apache-2.0

import Foundation
import Testing
@testable import Nola

struct MacMCPActivationTests {
    @Test func mcpEditorRejectsStaleWritesAndTombstoneCreation() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("mcp.db"))
        let input = MacCanonicalMCPServerInput(
            id: "test-server", displayName: "Original", enabled: true,
            configurationJSON: "{\"transport\":\"stdio\"}", revision: 0,
            createdAtMs: 10, updatedAtMs: 10, deletedAtMs: nil
        )
        let opened = try store.createMCPServer(input)
        #expect(throws: (any Error).self) { try store.createMCPServer(input) }
        let committed = try store.configureMCPServer(
            opened, displayName: "Edited", enabled: false,
            configurationJSON: opened.configurationJSON, updatedAtMs: 20
        )
        #expect(committed.revision > opened.revision)
        #expect(committed.createdAtMs == opened.createdAtMs)
        #expect(throws: (any Error).self) {
            try store.configureMCPServer(opened, displayName: "Stale", enabled: true,
                configurationJSON: opened.configurationJSON, updatedAtMs: 30)
        }
        #expect(throws: (any Error).self) { try store.deleteMCPServer(opened, deletedAtMs: 30) }
        #expect(try store.listMCPServers() == [committed])
        #expect(try store.deleteMCPServer(committed, deletedAtMs: 40))
        #expect(throws: (any Error).self) { try store.createMCPServer(input) }
        #expect(try store.listMCPServers().isEmpty)
    }

    @Test func discoveredMCPRequiresExplicitActivationAndFreezesExactServer() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let configURL = root.appendingPathComponent(".mcp.json")
        try """
        {
          // JSONC comments and trailing commas are accepted.
          "mcpServers": {
            "docs": {"type":"http","url":"https://example.test/mcp",},
            "disabled": {"type":"http","url":"https://disabled.test/mcp"}
          },
        }
        """.write(to: configURL, atomically: true, encoding: .utf8)
        let databaseURL = root.appendingPathComponent("nola-product.db")
        let store = try MacApplicationPort(databaseURL: databaseURL)

        #expect(MacLocalMCPActivationStore.serverNames(in: configURL) == ["disabled", "docs"])
        #expect(MacLocalMCPActivationStore.activeServerConfigurationData(
            workspaceURL: root,
            store: store
        ) == nil)

        let itemID = MacLocalMCPActivationStore.itemID(
            source: .workspace,
            configurationURL: configURL,
            serverName: "docs"
        )
        try MacLocalMCPActivationStore.setEnabled(
            true,
            itemID: itemID,
            store: store
        )
        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let data = try #require(MacLocalMCPActivationStore.activeServerConfigurationData(
            workspaceURL: root,
            store: reopened
        ))
        let servers = try #require(JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        #expect(servers.count == 1)
        #expect(servers[0]["server_id"] as? String == MacLocalMCPActivationStore.serverID(for: itemID))
        #expect(MacLocalMCPActivationStore.serverID(for: itemID) != MacLocalMCPActivationStore.serverID(for: "another-source:docs"))
        #expect(servers[0]["endpoint"] as? String == "https://example.test/mcp")
        #expect(servers[0]["enabled"] as? Bool == true)
        let persisted = try #require(reopened.listMCPServers().first(where: { $0.id == itemID }))
        #expect(persisted.enabled)
        #expect(persisted.displayName == "docs")
    }
}
