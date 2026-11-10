import Foundation
import Testing
@testable import Nola

struct IOSCanonicalDataSourceReadStateTests {
    @Test(arguments: ["never_read", "pending", "ready", "empty", "denied", "unavailable", "failed"])
    func canonicalReadStatusIsNotCollapsedIntoValidation(status: String) throws {
        let data = Data("{\"status\":\"\(status)\",\"read_token\":\"read-1\",\"diagnostic\":\"detail\"}".utf8)
        let state = try JSONDecoder().decode(IOSCanonicalDataSourceReadState.self, from: data)
        #expect(state.status.rawValue == status)
        #expect(state.readToken == "read-1")
        #expect(state.diagnostic == "detail")
        #expect(try JSONDecoder().decode(
            IOSCanonicalDataSourceReadState.self,
            from: JSONEncoder().encode(state)
        ) == state)
    }

    @Test
    func committedContentReferenceAndReadTimesSurviveDecoding() throws {
        let data = Data(#"{"status":"ready","content_reference":{"uri":"file:///cache/read.txt","mime_type":"text/plain","byte_count":9000000000,"content_version":"v2"},"observed_at_ms":2000,"schema_version":1,"record_count":3,"requested_at_ms":1000,"completed_at_ms":3000}"#.utf8)
        let state = try JSONDecoder().decode(IOSCanonicalDataSourceReadState.self, from: data)
        #expect(state.contentReference?.byteCount == 9_000_000_000)
        #expect(state.contentReference?.uri == "file:///cache/read.txt")
        #expect(state.contentReference?.contentVersion == "v2")
        #expect(state.contentReference?.mimeType == "text/plain")
        #expect(state.observedAtMs == 2000)
        #expect(state.requestedAtMs == 1000)
        #expect(state.completedAtMs == 3000)
        #expect(state.schemaVersion == 1)
        #expect(state.recordCount == 3)
    }

    @Test
    func unsupportedReadStatusAndRemovedLifecycleAreRejected() {
        #expect(throws: DecodingError.self) {
            try JSONDecoder().decode(IOSCanonicalDataSourceReadState.self, from: Data(#"{"status":"succeeded"}"#.utf8))
        }
        #expect(throws: DecodingError.self) {
            try JSONDecoder().decode(IOSCanonicalDataSourceLifecycle.self, from: Data(#""disabled""#.utf8))
        }
    }
}

struct IOSApplicationRuntimeTests {
    @Test
    func memoryEditorPreservesUnprojectedNamespacesAndReturnsCommittedSnapshot() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-memory-namespace-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let initial = try store.saveMemoryText(longTermMemory: "Keep outside this editor", userPreferences: "", expected: store.loadProfile())
        let seed = try #require(initial.canonicalMemorySnapshot?.first)
        var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(seed.record)) as? [String: Any])
        object["id"] = UUID().uuidString.lowercased()
        object["namespace"] = ["kind": "custom", "name": "external_memory_namespace"]
        let record = try JSONDecoder().decode(IOSCanonicalSemanticMemoryRecord.self,
            from: JSONSerialization.data(withJSONObject: object))
        let external = try runtime.upsertMemory(IOSCanonicalMemoryCandidate(record: record, quality: nil, proposedAtMs: 1))
        let baseline = store.loadProfile()
        #expect(baseline.memoryRecords.count == 1)
        #expect(baseline.canonicalMemorySnapshot?.count == 2)
        let saved = try store.saveMemoryText(longTermMemory: "", userPreferences: "", expected: baseline)
        #expect(saved.memoryRecords.isEmpty)
        #expect(saved.canonicalMemorySnapshot == (try runtime.listMemories()))
        #expect(try runtime.listMemories().map(\.record) == [external.record])
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try reopened.listMemories().map(\.record) == [external.record])
    }

    @Test
    func mcpProjectionUsesCanonicalIdentityInsteadOfEmbeddedConfigurationIdentity() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-mcp-identity-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let canonicalID = UUID()
        let otherItem = UserMCPServer(name: "Other server", endpoint: "https://example.test/other")
        let other = try store.saveMCP(otherItem, expected: nil)
        let embedded = UserMCPServer(id: otherItem.id, name: "Stale name", endpoint: "https://example.test/first", enabled: true)
        let configuration = String(decoding: try JSONEncoder().encode(embedded), as: UTF8.self)
        let first = try runtime.createMCPServer(IOSCanonicalMCPServer(
            id: canonicalID.uuidString.lowercased(), displayName: "Canonical name", enabled: false,
            configurationJSON: configuration, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        let projection = try store.loadCapabilityLibrary()
        #expect(Set(projection.mcpServers.map(\.id)) == Set([canonicalID, otherItem.id]))
        var draft = try #require(projection.mcpServers.first { $0.id == canonicalID })
        #expect(draft.name == "Canonical name")
        #expect(!draft.enabled)
        #expect(draft.endpoint == "https://example.test/first")
        draft.name = "Edited first server"
        let edited = try store.saveMCP(draft, expected: first)
        #expect(edited.id == first.id)
        #expect(try store.loadMCPEditor(id: otherItem.id) == other)
        #expect(try store.deleteMCP(expected: edited))
        #expect(try runtime.listMCPServers() == [other])
    }

    @Test
    func capabilityLibraryRejectsPartialProjectionAndRecoversWithoutDeletingRecords() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-capability-read-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let skill = UserUploadedContextItem(name: "Keep", content: "Read safely")
        _ = try store.saveSkill(skill, expected: nil)
        #expect(try store.loadCapabilityLibrary().uploadedSkills.map(\.id) == [skill.id])
        let malformed = try runtime.createMCPServer(IOSCanonicalMCPServer(
            id: UUID().uuidString.lowercased(), displayName: "Malformed", enabled: false,
            configurationJSON: "{}", revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        #expect(throws: (any Error).self) { try store.loadCapabilityLibrary() }
        #expect(try runtime.listMCPServers() == [malformed])
        #expect(try runtime.listSkills().count == 1)
        #expect(try store.deleteMCP(expected: malformed))
        // Unrelated corrupt document settings must not block the capability query.
        _ = try runtime.putProductSetting(key: "assistant.uploaded_documents", valueJSON: "{}", updatedAtMs: 1)
        let recovered = try store.loadCapabilityLibrary()
        #expect(recovered.uploadedSkills.map(\.id) == [skill.id])
        #expect(recovered.mcpServers.isEmpty)
    }

    @Test
    func memoryOnlySaveDoesNotRewriteCapabilitiesAddedAfterEditorOpened() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-memory-isolation-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let baseline = store.loadProfile()
        let skill = try store.saveSkill(UserUploadedContextItem(name: "Keep", content: "Keep this skill"), expected: nil)
        let server = try runtime.createMCPServer(IOSCanonicalMCPServer(
            id: UUID().uuidString.lowercased(), displayName: "Keep", enabled: true,
            configurationJSON: "{}", revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        _ = try runtime.putProductSetting(key: "assistant.uploaded_documents", valueJSON: "[]", updatedAtMs: 1)
        let documents = try runtime.productSetting(key: "assistant.uploaded_documents")
        let saved = try store.saveMemoryText(longTermMemory: "Durable memory", userPreferences: "Concise", expected: baseline)
        #expect(saved.longTermMemory == "Durable memory")
        #expect(saved.userPreferences == "Concise")
        #expect(try runtime.skillEditor(id: skill.record.id) == skill)
        #expect(try runtime.listMCPServers() == [server])
        #expect(try runtime.productSetting(key: "assistant.uploaded_documents") == documents)
        #expect(throws: (any Error).self) {
            try store.saveMemoryText(longTermMemory: "Stale replacement", userPreferences: "", expected: baseline)
        }
        let reopened = CanonicalChatMemoryProfileStore(applicationRuntime: try IOSApplicationRuntime(databaseURL: database))
        #expect(reopened.loadProfile().longTermMemory == "Durable memory")
        let cleared = try reopened.saveMemoryText(longTermMemory: "", userPreferences: "", expected: reopened.loadProfile())
        #expect(cleared.memoryRecords.isEmpty)
        #expect(try runtime.skillEditor(id: skill.record.id) == skill)
        #expect(try runtime.listMCPServers() == [server])
    }

    @Test
    func individualMCPEditorPreservesConfigurationAndRejectsStaleChanges() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-mcp-profile-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        let id = UUID()
        var deleted: IOSCanonicalMCPServer?
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
            let configuration: [String: Any] = [
                "id": id.uuidString, "name": "Files", "endpoint": "https://example.test/mcp",
                "description": "Original", "enabled": true,
                "transport": "streamable-http", "headers": ["X-Fixture": "preserve"],
                "extension": ["timeout": 42, "capabilities": ["read", "write"]]
            ]
            let json = String(decoding: try JSONSerialization.data(withJSONObject: configuration), as: UTF8.self)
            let original = try runtime.createMCPServer(IOSCanonicalMCPServer(
                id: id.uuidString.lowercased(), displayName: "Files", enabled: true,
                configurationJSON: json, revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
            ))
            let skill = try store.saveSkill(UserUploadedContextItem(name: "Untouched", content: "Keep instructions", enabled: true), expected: nil)
            let other = try runtime.createMCPServer(IOSCanonicalMCPServer(
                id: UUID().uuidString.lowercased(), displayName: "Other", enabled: false,
                configurationJSON: "{}", revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
            ))
            var proposed = configuration
            proposed["name"] = "Renamed"
            proposed["endpoint"] = "https://example.test/new-mcp"
            proposed["description"] = "Edited"
            proposed["enabled"] = false
            let draft = try JSONDecoder().decode(UserMCPServer.self, from: JSONSerialization.data(withJSONObject: proposed))
            let edited = try store.saveMCP(draft, expected: original)
            #expect(edited.enabled)
            #expect(edited.createdAtMs == original.createdAtMs)
            let saved = try #require(try JSONSerialization.jsonObject(with: Data(edited.configurationJSON.utf8)) as? [String: Any])
            #expect(saved["name"] as? String == "Renamed")
            #expect(saved["endpoint"] as? String == "https://example.test/new-mcp")
            #expect(saved["description"] as? String == "Edited")
            #expect(saved["transport"] as? String == "streamable-http")
            #expect(saved["headers"] as? [String: String] == ["X-Fixture": "preserve"])
            #expect(NSDictionary(dictionary: try #require(saved["extension"] as? [String: Any])) == NSDictionary(dictionary: try #require(configuration["extension"] as? [String: Any])))
            #expect(throws: (any Error).self) { try store.saveMCP(draft, expected: original) }
            #expect(throws: (any Error).self) { try store.setMCPEnabled(false, expected: original) }
            #expect(throws: (any Error).self) { try store.deleteMCP(expected: original) }
            #expect(try store.loadMCPEditor(id: id) == edited)
            let disabled = try store.setMCPEnabled(false, expected: edited)
            #expect(!disabled.enabled)
            #expect(disabled.configurationJSON == edited.configurationJSON)
            #expect(disabled.userMCPServer?.enabled == false)
            #expect(try store.deleteMCP(expected: disabled))
            deleted = disabled
            #expect(try runtime.listMCPServers() == [other])
            #expect(try runtime.skillEditor(id: skill.record.id) == skill)
            #expect(try runtime.listMemories().isEmpty)
            #expect(throws: (any Error).self) { try store.saveMCP(draft, expected: nil) }
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: reopened)
        #expect(throws: (any Error).self) { try store.loadMCPEditor(id: id) }
        let stale = try #require(deleted)
        #expect(throws: (any Error).self) { try store.setMCPEnabled(true, expected: stale) }
        #expect(try reopened.listMCPServers().count == 1)
    }

    @Test
    func mcpEditorDoesNotOverwriteMalformedConfiguration() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-mcp-invalid-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let id = UUID()
        let original = try runtime.createMCPServer(IOSCanonicalMCPServer(
            id: id.uuidString.lowercased(), displayName: "Invalid", enabled: false,
            configurationJSON: "[]", revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        let draft = try JSONDecoder().decode(UserMCPServer.self, from: JSONSerialization.data(withJSONObject: [
            "id": id.uuidString, "name": "Replacement", "endpoint": "https://example.test/mcp",
            "description": "Draft", "enabled": false
        ]))
        #expect(throws: (any Error).self) { try store.saveMCP(draft, expected: original) }
        #expect(try store.loadMCPEditor(id: id) == original)
    }

    @Test
    func mcpEditorUsesCanonicalRevisionAndPreventsIdentityResurrection() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-mcp-editor-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        func input(_ name: String) -> IOSCanonicalMCPServer {
            IOSCanonicalMCPServer(id: "files", displayName: name, enabled: false,
                configurationJSON: #"{"endpoint":"http://localhost:9876/mcp"}"#,
                revision: 0, createdAtMs: 999, updatedAtMs: 1000, deletedAtMs: nil)
        }
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let created = try runtime.createMCPServer(input("Files"))
            #expect(created.revision > 0)
            #expect(throws: (any Error).self) { try runtime.createMCPServer(input("Duplicate")) }
            let edited = try runtime.configureMCPServer(input("Renamed"), expected: created)
            #expect(edited.revision > created.revision)
            #expect(edited.createdAtMs == created.createdAtMs)
            #expect(throws: (any Error).self) { try runtime.configureMCPServer(input("Stale"), expected: created) }
            #expect(throws: (any Error).self) { try runtime.deleteMCPServer(expected: created, deletedAtMs: 1001) }
            #expect(try runtime.listMCPServers() == [edited])
        }
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let current = try #require(try runtime.listMCPServers().first)
            #expect(current.displayName == "Renamed")
            #expect(try runtime.deleteMCPServer(expected: current, deletedAtMs: 1002))
        }
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try reopened.listMCPServers().isEmpty)
        #expect(throws: (any Error).self) { try reopened.createMCPServer(input("Resurrect")) }
    }

    @Test
    func individualSkillEditorDoesNotRewriteOtherCapabilities() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-skill-profile-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let store = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let server = try runtime.upsertMCPServer(IOSCanonicalMCPServer(
            id: "preserve", displayName: "Preserve MCP", enabled: false, configurationJSON: "{}",
            revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        let first = UserUploadedContextItem(name: "Same name", content: "Review text", enabled: true)
        let second = UserUploadedContextItem(name: "Same name", content: "Other text", enabled: true)
        let opened = try store.saveSkill(first, expected: nil)
        let other = try store.saveSkill(second, expected: nil)
        #expect(opened.record.id != other.record.id)
        let edited = try store.saveSkill(UserUploadedContextItem(
            id: first.id, name: "Renamed", content: "New instructions", description: "New summary", enabled: false
        ), expected: opened)
        #expect(edited.record.enabled)
        #expect(edited.record.createdAtMs == opened.record.createdAtMs)
        #expect(throws: (any Error).self) { try store.saveSkill(first, expected: opened) }
        #expect(throws: (any Error).self) { try store.deleteSkill(expected: opened) }
        #expect(try store.loadSkillEditor(id: first.id) == edited)
        #expect(try store.deleteSkill(expected: edited))
        #expect(try store.loadSkillEditor(id: second.id) == other)
        #expect(try runtime.listMCPServers() == [server])
        #expect(try runtime.listMemories().isEmpty)
        #expect(throws: (any Error).self) { try store.saveSkill(first, expected: nil) }
    }

    @Test
    func skillEditorRejectsStaleMutationsAndResurrectionAcrossFFI() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-skill-editor-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("product.db")
        func input(_ name: String, version: String = "v1") -> IOSCanonicalSkill {
            IOSCanonicalSkill(id: "review", source: "user", version: version, displayName: name,
                summary: "Preserve summary", category: "user", instructionsRef: "skill://review/SKILL.md",
                instructionsDigest: "3b7d3ff3c0cb68998d557a16e00de20ea6c57b4072cc9e1d6005fc0cf37e1d9f",
                instructionsText: "Review the supplied text.", requiredTools: [], defaultMode: nil,
                enabled: true, createdAtMs: 1, updatedAtMs: 2, deletedAtMs: nil)
        }
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let opened = try runtime.createUserSkill(input("Original"))
            #expect(throws: (any Error).self) { try runtime.createUserSkill(input("Duplicate")) }
            let edited = try runtime.configureUserSkill(input("Renamed", version: "v2"), expected: opened)
            #expect(edited.revision > opened.revision)
            #expect(edited.record.summary == opened.record.summary)
            #expect(throws: (any Error).self) { try runtime.configureUserSkill(input("Stale"), expected: opened) }
            #expect(throws: (any Error).self) { try runtime.setSkillEnabled(false, expected: opened, updatedAtMs: 3) }
            #expect(throws: (any Error).self) { try runtime.deleteUserSkill(expected: opened, deletedAtMs: 3) }
            #expect(try runtime.skillEditor(id: "review") == edited)
        }
        do {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            let reopened = try #require(try runtime.skillEditor(id: "review"))
            #expect(reopened.record.displayName == "Renamed")
            let disabled = try runtime.setSkillEnabled(false, expected: reopened, updatedAtMs: 4)
            #expect(!disabled.record.enabled)
            #expect(try runtime.deleteUserSkill(expected: disabled, deletedAtMs: 5))
        }
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        #expect(try runtime.listSkills().isEmpty)
        #expect(throws: (any Error).self) { try runtime.createUserSkill(input("Resurrect")) }
    }

    @Test
    func modelParametersUseCanonicalModelProfileAcrossColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-model-parameters-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let firstStore = try IOSApplicationRuntime(databaseURL: database)
        try firstStore.upsertModelProfile(IOSCanonicalModelProfile(
            id: "qwen3.5-4b", displayName: "Qwen", provider: "local",
            modelName: "qwen3.5-4b", runtime: "mlx", sourceJSON: #"{"format":"mlx"}"#,
            parametersJSON: "{}", selected: true, enabled: true, revision: 0,
            createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil
        ))
        let first = ModelParametersStore(applicationPort: firstStore)
        var parameters = ModelParameters.localDefaults
        parameters.temperature = 0.23
        parameters.maxContextLength = 131_072
        parameters.responseStyle = .professional
        try first.saveParameters(parameters, for: "qwen3.5-4b", isLocal: true)

        let reopenedStore = try IOSApplicationRuntime(databaseURL: database)
        let reopened = ModelParametersStore(applicationPort: reopenedStore)
            .loadParameters(for: "qwen3.5-4b", isLocal: true)
        #expect(reopened.temperature == 0.23)
        #expect(reopened.maxContextLength == 131_072)
        #expect(reopened.responseStyle == PromptResponseStyle.professional)
        #expect(try reopenedStore.modelProfile(id: "qwen3.5-4b")?.parametersJSON.isEmpty == false)
    }

    @Test
    func missingModelParameterSaveDoesNotCreateProfileOrPublishSuccess() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-missing-model-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let parameters = ModelParametersStore(applicationPort: runtime)
        #expect(throws: (any Error).self) {
            try parameters.saveParameters(.localDefaults, for: "missing", isLocal: true)
        }
        #expect(parameters.lastUpdated == nil)
        #expect(try runtime.listModelProfiles().isEmpty)
    }

    @Test
    func dataSourceSurvivesHandleAndSQLiteColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-store-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        do {
            let store = try IOSApplicationRuntime(databaseURL: database)
            let unsigned: [String: AgentCoreDataSourceJSON] = [
                "sourceId": .string("ios-real-source"),
                "connectorId": .string("ios.file"),
                "connectorVersion": .number(1),
                "readerDescription": .string("Bounded file read"),
                "readRecipe": .object([
                    "method": .string("localFileRead"),
                    "operationId": .string("ios.file.read"),
                    "configurationKeys": .array([.string("bookmarkReference")]),
                    "outputSchema": .object([
                        "type": .string("object"),
                        "properties": .object(["title": .object(["type": .string("string")])])
                    ])
                ]),
                "schemaFields": .array([.string("title")]),
                "redactedSample": .string("fixture"),
                "recordCount": .number(1),
                "maximumAgeMinutes": .number(60),
                "validationState": .string("verified"),
                "validatedAt": .string("2026-09-04T00:00:00Z"),
                "observedAt": .string("2026-09-04T00:00:00Z"),
                "validationDigest": .string("")
            ]
            let contract = try #require(AgentCoreDataSourceResolver.sealContract(unsigned))
            let saved = try store.saveDataSource(IOSCanonicalDataSourceSubmission(
                draft: IOSCanonicalDataSourceDraft(
                    id: "ios-real-source",
                    name: "iOS real source",
                    kind: .localFile,
                    configuration: IOSCanonicalDataSourceConfiguration(
                        connectorID: "ios.file",
                        locationKind: .localPath,
                        location: "file:///Documents/source.json",
                        parameters: [
                            "metadataJSON": .string(
                                #"{"bookmarkReference":"ios-real-source"}"#
                            )
                        ],
                        secretReferences: []
                    ),
                    accessPolicy: "conversationOptIn",
                    authorization: IOSCanonicalDataSourceAuthorization(
                        state: .authorized,
                        capability: "filesystem.read",
                        observedAtMs: 100
                    ),
                    freshnessPolicy: IOSCanonicalDataSourceFreshnessPolicy(
                        maximumAgeMinutes: 60,
                        allowStaleFallback: false
                    ),
                    capabilities: ["read", "context"],
                    privacy: "local-first",
                    createdAtMs: 100,
                    updatedAtMs: 100
                ),
                validationContract: contract,
                enabled: true
            ))
            #expect(saved.defaultEnabled)
            #expect(saved.readState.status == .neverRead)
            #expect(saved.readState.contentReference == nil)
        }

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let sources = try reopened.listDataSources()
        #expect(sources.count == 1)
        #expect(sources[0].defaultEnabled)
        #expect(sources[0].readState.status == .neverRead)
        #expect(sources[0].readState.readToken == nil)
        #expect(
            sources[0].configuration.parameters["metadataJSON"]
                == .string(#"{"bookmarkReference":"ios-real-source"}"#)
        )
    }

    @Test
    func memoryProfileSurvivesCanonicalStoreColdReopenAndDeletion() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-memory-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer {
            try? FileManager.default.removeItem(at: root)
        }
        let firstStore = try IOSApplicationRuntime(databaseURL: database)
        let profileStore = CanonicalChatMemoryProfileStore(applicationRuntime: firstStore)
        let documentData = try JSONEncoder().encode([
            UserUploadedContextItem(name: "Architecture", content: "Store is authoritative")
        ])
        _ = try firstStore.putProductSetting(key: "assistant.uploaded_documents",
            valueJSON: String(decoding: documentData, as: UTF8.self), updatedAtMs: 1)
        let saved = try profileStore.saveMemoryText(
            longTermMemory: "Nola keeps complete messages",
            userPreferences: "Reply concisely",
            expected: profileStore.loadProfile()
        )
        #expect(saved.memoryRecords.count == 2)

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        let reopenedProfile = CanonicalChatMemoryProfileStore(applicationRuntime: reopened).loadProfile()
        #expect(reopenedProfile.longTermMemory == "Nola keeps complete messages")
        #expect(reopenedProfile.userPreferences == "Reply concisely")
        #expect(reopenedProfile.uploadedDocuments.single?.content == "Store is authoritative")

        _ = try CanonicalChatMemoryProfileStore(applicationRuntime: reopened).saveMemoryText(
            longTermMemory: "", userPreferences: "", expected: reopenedProfile
        )
        let finalStore = try IOSApplicationRuntime(databaseURL: database)
        #expect(try finalStore.listMemories().isEmpty)
        #expect(CanonicalChatMemoryProfileStore(applicationRuntime: finalStore).loadProfile()
            .uploadedDocuments.single?.content == "Store is authoritative")
    }

    @Test
    func staleProfileSaveMustNotDeleteMemoryCommittedByAnotherEditor() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-stale-profile-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let firstEditor = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let secondEditor = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let baseline = try firstEditor.saveMemoryText(longTermMemory: "Original durable memory",
            userPreferences: "", expected: firstEditor.loadProfile())
        let secondDraft = secondEditor.loadProfile()
        _ = try secondEditor.saveMemoryText(longTermMemory: "Original durable memory\nAdded in another editor",
            userPreferences: "", expected: secondDraft)
        #expect(secondEditor.loadProfile().longTermMemory.contains("Added in another editor"))
        // An unchanged stale editor is not an instruction to delete new data.
        // A conflict is acceptable; silent successful removal is not.
        #expect(throws: (any Error).self) {
            _ = try firstEditor.saveMemoryText(longTermMemory: baseline.longTermMemory,
                userPreferences: baseline.userPreferences, expected: baseline)
        }
        #expect(firstEditor.loadProfile().longTermMemory.contains("Added in another editor"))
    }

    @Test
    func skillAndMcpSurviveCanonicalColdReopenWithoutUserDefaultsAuthority() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-extension-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer {
            try? FileManager.default.removeItem(at: root)
        }
        let skillID = UUID()
        let serverID = UUID()
        let first = CanonicalChatMemoryProfileStore(applicationRuntime: try IOSApplicationRuntime(databaseURL: database))
        _ = try first.saveMCP(UserMCPServer(id: serverID, name: "Files", endpoint: "https://mcp.example.com/files"), expected: nil)
        _ = try first.saveSkill(UserUploadedContextItem(id: skillID, name: "Review", content: "Review carefully"), expected: nil)
        let reopened = CanonicalChatMemoryProfileStore(applicationRuntime: try IOSApplicationRuntime(databaseURL: database)).loadProfile()
        #expect(reopened.uploadedSkills.map(\.id) == [skillID])
        #expect(reopened.uploadedSkills.single?.content == "Review carefully")
        #expect(reopened.mcpServers.map(\.id) == [serverID])
    }

    @Test
    func productSettingRevisionAndDeletionSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-setting-\(UUID().uuidString)", isDirectory: true)
        let database = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let first = try IOSApplicationRuntime(databaseURL: database)
        let saved = try first.putProductSetting(
            key: "assistant.memory.enabled",
            valueJSON: "{\"schemaVersion\":1,\"enabled\":false}",
            updatedAtMs: 100
        )
        #expect(saved.revision == 1)

        let reopened = try IOSApplicationRuntime(databaseURL: database)
        #expect(try reopened.productSetting(key: "assistant.memory.enabled")?.valueJSON.contains("false") == true)
        let updated = try reopened.putProductSetting(
            key: "assistant.memory.enabled",
            valueJSON: "{\"schemaVersion\":1,\"enabled\":true}",
            updatedAtMs: 200
        )
        #expect(updated.revision == 2)
        #expect(try reopened.deleteProductSetting(key: "assistant.memory.enabled"))

        let final = try IOSApplicationRuntime(databaseURL: database)
        #expect(try final.productSetting(key: "assistant.memory.enabled") == nil)
    }

    @Test
    func remoteReplicaPageAppliesThroughRealCoreABIAndSurvivesColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-ios-remote-sync-\(UUID().uuidString)", isDirectory: true)
        let sourceURL = root.appendingPathComponent("source.db")
        let targetURL = root.appendingPathComponent("target.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let source = try IOSApplicationRuntime(databaseURL: sourceURL)
        _ = try source.putProductSetting(
            key: "assistant.memory.enabled",
            valueJSON: "true",
            updatedAtMs: 100
        )
        let pending = try #require(source.pendingSyncChanges().first)

        let target = try IOSApplicationRuntime(databaseURL: targetURL)
        let result = try target.applyRemoteSyncPage(
            replicaID: "account-replica",
            changes: [.init(
                cursor: 1,
                operationID: pending.operationID,
                entityType: pending.entityType,
                entityID: pending.entityID,
                revision: pending.revision,
                deleted: pending.deleted,
                payloadJSON: pending.payloadJSON,
                createdAtMs: pending.createdAtMs
            )],
            nextCursor: 1,
            appliedAtMs: 200
        )
        #expect(result.applied == 1)
        #expect(result.skipped == 0)

        let reopened = try IOSApplicationRuntime(databaseURL: targetURL)
        #expect(try reopened.productSetting(key: "assistant.memory.enabled")?.valueJSON == "true")
        #expect(try reopened.syncCursor(replicaID: "account-replica")?.cursor == 1)
        #expect(try reopened.pendingSyncChanges().isEmpty)
    }

}

private extension Collection {
    var single: Element? { count == 1 ? first : nil }
}
