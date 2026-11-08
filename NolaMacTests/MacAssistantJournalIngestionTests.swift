// SPDX-License-Identifier: Apache-2.0
import CryptoKit
import Foundation
import Network
import XCTest
@testable import Nola

final class MacAssistantJournalIngestionTests: XCTestCase {
    func testEncryptedRecordReceptionResumesPersistedPreparationAfterFailedPage() async throws {
        guard MacProcessRuntimePolicy.usesEphemeralCredentials else {
            throw XCTSkip("Requires isolated test credentials")
        }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("journal-ingestion-\(UUID())")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let sourceFixture = try MacAssistantJournalTestFixture(conversationID: "conversation", runID: "run",
            assistantMessageID: "assistant", sourceHostID: "journal-host", userText: "Work", userMessageID: "user")
        let source = sourceFixture.source
        let answer = (0..<6_000).map { "第\($0)行：跨端正文🙂\n" }.joined()
        let initialCursor = try sourceFixture.admittedHead()
        XCTAssertEqual(initialCursor.sequence, "0", "Admission identifies the owner without publishing empty text")
        // Commit an actual first publication before the multi-chunk delta.
        // Admission alone deliberately publishes no empty journal record.
        let split = answer.index(answer.startIndex, offsetBy: 10)
        _ = try sourceFixture.append(String(answer[..<split]))
        let watermark = try sourceFixture.append(String(answer[split...]))
        let server = try JournalLoopbackServer(source: source)
        let baseURL = try await server.start()
        defer { server.stop() }
        let database = root.appendingPathComponent("receiver.db")
        var port = try MacApplicationPort(databaseURL: database)
        _ = try port.prepareFixtureRemoteSubmission(.init(conversationID: "conversation", conversationTitle: "Journal",
            submissionID: "run", sourceHostID: "journal-host", clientID: "fixture-client", userMessageID: "user", userPartID: "user:text",
            userText: "Work", userContentJSON: nil, assistantMessageID: "assistant", assistantPartID: "assistant:text",
            contextCheckpointID: nil, createdAtMs: 1))
        _ = try port.acceptRemoteSubmissionReceipt(sourceFixture.submissionReceipt())
        let configuration = MacRemoteHostConfigurationStore(applicationPort: port)
        try configuration.save(url: baseURL.absoluteString, token: JournalLoopbackServer.token, hostID: "journal-host")
        var remote = await MacRemoteHostService(configurationStore: configuration)
        let target = MacRemoteHostConfiguration(baseURL: baseURL, token: JournalLoopbackServer.token,
                                                 expectedHostID: "journal-host")
        let notification = String(decoding: try JSONSerialization.data(withJSONObject: [
            "type": "assistant_journal_committed_v1", "session_id": "conversation",
            "source_host_id": "journal-host",
            "through": try JSONSerialization.jsonObject(with: JSONEncoder().encode(watermark))
        ]), as: UTF8.self)
        do {
            _ = try await remote.parseFrame(notification,
                configuration: .init(baseURL: baseURL, token: JournalLoopbackServer.token, expectedHostID: "foreign-host"),
                expectedConversationID: "conversation", expectedRunID: "run")
            XCTFail("The transport Host must match the notification before any content is fetched")
        } catch MacRemoteHostError.invalidResponse { }
        do {
            _ = try await remote.parseFrame(notification, configuration: target, expectedConversationID: "another-conversation", expectedRunID: "run")
            XCTFail("An admitted message in another conversation must not be imported through this stream")
        } catch MacRemoteHostError.invalidResponse { }
        XCTAssertTrue(server.readAfterSequences.isEmpty)
        // A settings edit must not redirect a stream already connected to its
        // captured target, even when the advertised Host name stays the same.
        try configuration.save(url: "http://127.0.0.1:1", token: JournalLoopbackServer.token, hostID: "journal-host")
        var recovered = false
        do {
            _ = try await remote.parseFrame(notification, configuration: target, expectedConversationID: "conversation", expectedRunID: "run")
        } catch {
            guard case MacRemoteHostError.http(503, _) = error else { throw error }
            recovered = true
            let pending = try port.assistantJournalReceptionState(sourceHostID: "journal-host", through: watermark)
            XCTAssertFalse(pending.complete)
            XCTAssertNotEqual(pending.after.sequence, "0", "Earlier record pages must be committed before retry")
            port = try MacApplicationPort(databaseURL: database)
            let restored = MacRemoteHostConfigurationStore(applicationPort: port)
            try restored.save(url: baseURL.absoluteString, token: JournalLoopbackServer.token, hostID: "journal-host")
            remote = await MacRemoteHostService(configurationStore: restored)
            let event = try await remote.parseFrame(notification, configuration: target, expectedConversationID: "conversation", expectedRunID: "run")
            XCTAssertEqual(event?.assistantJournalCursor, watermark)
            XCTAssertEqual(event?.text, "", "Content markers must not become visible status text")
            XCTAssertTrue(try port.assistantJournalReceptionState(sourceHostID: "journal-host", through: watermark).complete)
        }
        XCTAssertTrue(recovered, "The fixture must exercise a failed page, not just a successful transfer")
        let beforeReplay = server.chunkIndices
        let pagesBeforeReplay = server.readAfterSequences
        let replay = try await remote.parseFrame(notification, configuration: target, expectedConversationID: "conversation", expectedRunID: "run")
        XCTAssertEqual(replay?.assistantJournalCursor, watermark)
        let recoveredHeads = try await remote.synchronizeRunAssistantJournals(conversationID: "conversation", runID: "run", configuration: target)
        XCTAssertTrue(recoveredHeads.contains(watermark))
        let terminalSeal = try sourceFixture.finish()
        let sealedWatermark = try XCTUnwrap(sourceFixture.heads().first { $0.identity == watermark.identity })
        let terminalObject: [String: Any] = [
            "type": "assistant_run_terminal_committed_v1", "session_id": "conversation", "run_id": "run",
            "source_host_id": "journal-host", "seal_digest": terminalSeal.seal_digest,
        ]
        let terminalFrame = String(decoding: try JSONSerialization.data(withJSONObject: terminalObject), as: UTF8.self)
        let discoveryBeforeTerminal = server.headRequests
        for expectedRun in [nil, "other-run"] as [String?] {
            do {
                _ = try await remote.parseFrame(terminalFrame, configuration: target,
                    expectedConversationID: "conversation", expectedRunID: expectedRun)
                XCTFail("Terminal catch-up requires the captured stream Run, not just the conversation")
            } catch MacRemoteHostError.invalidResponse { }
        }
        for declaredHost in [nil, "foreign-host"] as [String?] {
            var invalid = terminalObject
            if let declaredHost { invalid["source_host_id"] = declaredHost }
            else { invalid.removeValue(forKey: "source_host_id") }
            let frame = String(decoding: try JSONSerialization.data(withJSONObject: invalid), as: UTF8.self)
            do {
                _ = try await remote.parseFrame(frame, configuration: target,
                    expectedConversationID: "conversation", expectedRunID: "run")
                XCTFail("The parser must not overwrite a missing or foreign terminal Host")
            } catch MacRemoteHostError.invalidResponse { }
        }
        XCTAssertEqual(server.headRequests, discoveryBeforeTerminal, "Foreign terminal identities must fail before discovery")
        let deliveredTerminal = try await remote.parseFrame(terminalFrame, configuration: target,
            expectedConversationID: "conversation", expectedRunID: "run")
        XCTAssertEqual(deliveredTerminal?.assistantRunTerminalSealDigest, terminalSeal.seal_digest)
        XCTAssertEqual(deliveredTerminal?.canonicalRunID, "run")
        XCTAssertTrue(try port.beginReceiveAssistantRunTerminal(sourceHostID: "journal-host", seal: terminalSeal).committed,
            "Terminal completion must follow actual manifest admission, not the hint alone")
        XCTAssertEqual(server.chunkIndices, beforeReplay, "An exact committed replay must not download chunks")
        XCTAssertEqual(Array(server.readAfterSequences.prefix(pagesBeforeReplay.count)), pagesBeforeReplay)
        XCTAssertTrue(server.readAfterSequences.dropFirst(pagesBeforeReplay.count).allSatisfy { $0 != "0" },
            "Terminal catch-up may read its new completion record, but must not replay committed history")
        XCTAssertEqual(pagesBeforeReplay.filter { $0 == "0" }.count, 1, "Resume must not replay the entire journal")
        XCTAssertGreaterThan(pagesBeforeReplay.count, 2)
        XCTAssertEqual(beforeReplay.filter { $0 == "0" }.count, 1,
            "Chunk indices extend the committed body prefix across records; recovery must not redownload its first chunk")
        let snapshot = try port.openAssistantJournalSnapshot(at: watermark)
        defer { try? port.releaseAssistantJournalSnapshot(snapshot) }
        let parts = try port.assistantJournalParts(snapshot: snapshot)
        XCTAssertEqual(parts.at, watermark)
        let text = try XCTUnwrap(parts.parts.first { head in
            head.bodies.contains { $0.utf8_bytes == String(answer.utf8.count) }
        })
        let body = try XCTUnwrap(text.bodies.first)
        let viewReference = MacJournalMessageReference(databaseURL: database, cursor: watermark)
        let sourceReference = MacJournalMessageReference(databaseURL: sourceFixture.databaseURL, cursor: watermark)
        let sourceIsLocal = try await MacJournalTextRead.isLocal(sourceReference)
        XCTAssertTrue(sourceIsLocal)
        do {
            _ = try await MacJournalTextRead.isLocal(viewReference)
            XCTFail("An imported journal cannot authorize interpreting a source Host path as a local file")
        } catch { }
        let coldPort = try MacApplicationPort(databaseURL: database)
        let coldWindow = try coldPort.loadConversationWindow("conversation")
        XCTAssertTrue(coldWindow.assistantJournalHeads.contains(sealedWatermark))
        let coldMessages = try MacCanonicalConversationRepository.projectMessages(coldWindow, databaseURL: database)
        XCTAssertEqual(coldMessages.first(where: { $0.id == MacCanonicalMessageIdentity.stableID(watermark.identity.assistant_message_id) })?.journalReference,
            MacJournalMessageReference(databaseURL: database, cursor: sealedWatermark))
        let viewParts = try await MacJournalTextRead.parts(viewReference)
        XCTAssertEqual(viewParts.map(\.part_id), parts.parts.map(\.part_id))
        let exportedMarkdown = try await MacJournalTextRead.markdown(viewReference,
            historicalFallback: "stale cumulative projection must not be copied")
        XCTAssertEqual(exportedMarkdown, answer,
            "explicit copy must read all Unicode ranges from the pinned journal, not the old message text")
        let initialReference = MacJournalMessageReference(databaseURL: database, cursor: initialCursor)
        let initialParts = try await MacJournalTextRead.parts(initialReference)
        let advancedParts = try await MacJournalTextRead.updatedParts(viewReference,
            from: initialReference, existing: initialParts)
        XCTAssertEqual(advancedParts.map(\.part_id), viewParts.map(\.part_id), "delta order must match full canonical order")
        for (advanced, complete) in zip(advancedParts, viewParts) {
            XCTAssertEqual(advanced.identity, complete.identity)
            XCTAssertEqual(advanced.generation, complete.generation)
            XCTAssertEqual(advanced.revision, complete.revision)
            XCTAssertEqual(advanced.metadata_revision, complete.metadata_revision)
            XCTAssertEqual(advanced.bodies, complete.bodies)
            XCTAssertEqual(advanced.order_key.section, complete.order_key.section)
            XCTAssertEqual(advanced.order_key.position, complete.order_key.position)
        }
        let preview = try await MacJournalTextRead.range(viewReference, partID: text.part_id, content: body, offset: "0")
        XCTAssertTrue(answer.hasPrefix(preview.text))
        XCTAssertLessThanOrEqual(preview.text.utf8.count, 32768)
        XCTAssertFalse(preview.end_of_content, "The view must not silently hydrate the whole long answer")
        let cancelledReads = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            do {
                _ = try await MacJournalTextRead.updatedParts(viewReference,
                    from: viewReference, existing: viewParts)
                XCTFail("Cancelled updates must not adopt even the unchanged-cursor fast path")
            } catch is CancellationError { }
            do {
                _ = try await MacJournalTextRead.parts(viewReference)
                XCTFail("A cancelled view must not start another head read")
            } catch is CancellationError { }
            do {
                _ = try await MacJournalTextRead.range(viewReference, partID: text.part_id, content: body, offset: "0")
                XCTFail("A dismissed reader must not start another body read")
            } catch is CancellationError { }
        }
        try await cancelledReads.value
        var restored = "", offset = "0"
        while true {
            let range = try port.assistantJournalRange(snapshot: snapshot, partID: text.part_id,
                                                       content: body, offsetBytes: offset)
            XCTAssertEqual(range.offset_bytes, offset)
            restored += range.text
            if range.end_of_content { break }
            XCTAssertNotEqual(range.next_offset_bytes, offset)
            offset = range.next_offset_bytes
        }
        XCTAssertEqual(restored, answer)
    }

    private func command(_ client: MacApplicationServiceClient, _ input: [String:Any]) throws -> [String:Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: client.execute(commandData: JSONSerialization.data(withJSONObject: input))) as? [String:Any])
    }

    private func seedSource(_ client: MacApplicationServiceClient, answer: String,
                            onBegin: (([String: Any]) -> Void)? = nil) throws -> ([String:Any], [String:Any]) {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let capability: [String:Any] = ["id":"model.stream", "title":"Fixture", "route":"model", "availability":"available",
            "exposure":"internal", "input_schema":["type":"object"],
            "output_policy":["preview_char_limit":4000,"persist_full_output":true,"allow_artifacts":true],
            "supports_streaming":true,"parallelism":"serial","cancellation":"cooperative","replay_safety":"recoverable"]
        let begun = try command(client, ["type":"begin_assistant_turn", "input":[
            "conversation_id":"conversation","conversation_title":"Source","user_message_id":"user","user_part_id":"user:text",
            "user_text":"Work","assistant_message_id":"assistant","created_at_ms":now,
            "core_config":["session_id":"conversation","model":"fixture-model",
                "turn_identity": ["model_capability_id": "mac-journal-fixture", "model_capability_revision": 1,
                    "model_capability": MacAssistantJournalTestFixture.modelCapabilityProfile(
                        toolCapabilityID: nil, contextWindow: 128_000, maxOutput: 4096)],
                "host_catalog":["host_id":"journal-host",
                "revision":1,"availability_epoch":1,"capabilities":["model.stream":capability]]],
            "submission":["type":"user_input","items":[["type":"message","id":"user","role":"user","content":"Work"]],
                "submission_id":"run","submitted_at_epoch_ms":now,"delivery":"current_turn"]]])
        let digest = try XCTUnwrap(begun["invocation_config_digest"] as? String)
        onBegin?(try XCTUnwrap((begun["assistant_publications"] as? [[String: Any]])?.first))
        let identity = try XCTUnwrap(((begun["assistant_publications"] as? [[String:Any]])?.first)?["identity"] as? [String:Any])
        let claimed = try command(client, ["type":"claim_committed_host_requests","input":["run_id":"run",
            "worker_generation":"fixture-worker","lease_owner":"fixture-owner","claimed_at_ms":now,
            "lease_expires_at_ms":now+120_000,"limit":32]])
        var model = try XCTUnwrap((claimed["requests"] as? [[String:Any]])?.first {
            ($0["payload"] as? [String:Any])?["type"] as? String == "model_stream"
        })
        func accept(_ sequence: Int, _ payload: [String:Any]) throws {
            let keys = ["request_id","attempt_id","delegated_invocation_id","effect_id","host_id","lease_id",
                        "session_id","turn_id","catalog_revision","availability_epoch","delivery"]
            var response = model.filter { keys.contains($0.key) }
            response["sequence"] = sequence; response["payload"] = payload
            _ = try command(client, ["type":"reduce_and_accept_host_response","input":["conversation_id":"conversation",
                "run_id":"run","invocation_config_digest":digest,"response":response,"received_at_ms":now+Int64(sequence)]])
        }
        try accept(1, ["type":"execution_started"])
        try accept(2, ["type":"model_attempt_started","provider":"fixture","model":"fixture-model","reason":"initial"])
        let loaded = try command(client, ["type":"load_committed_host_requests","run_id":"run"])
        var refreshed = try XCTUnwrap((loaded["requests"] as? [[String:Any]])?.first { $0["effect_id"] as? String == model["effect_id"] as? String })
        refreshed["delivery"] = model["delivery"]; model = refreshed
        let part = try XCTUnwrap((model["payload"] as? [String:Any])?["text_part_id"] as? String)
        try accept(3, ["type":"model_delta","channel":"text","part_id":part,"delta":answer])
        return (identity, try command(client, ["type":"local_assistant_journal_head","identity":identity]))
    }

}

/// Real loopback HTTP + AES-GCM, backed by the source's real Application Service.
/// Only the HTTP adapter is a fixture; no journal records or content roots are mocked.
private nonisolated final class JournalLoopbackServer: @unchecked Sendable {
    static let token = "isolated-journal-test-token"
    private let listener: NWListener
    private let queue = DispatchQueue(label: "journal-loopback")
    private let source: MacApplicationServiceClient
    private var ready: CheckedContinuation<URL, Error>?
    private var indices: [String] = []
    private var afterSequences: [String] = []
    private var failed = false
    private var discoveredHeads = 0
    var headRequests: Int { queue.sync { discoveredHeads } }
    var chunkIndices: [String] { queue.sync { indices } }
    var readAfterSequences: [String] { queue.sync { afterSequences } }
    init(source: MacApplicationServiceClient) throws {
        self.source = source
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host:"127.0.0.1",port:.any)
        listener = try NWListener(using: parameters)
    }
    func start() async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            ready = continuation
            listener.stateUpdateHandler = { [self] state in
                if case .ready = state, let port = listener.port {
                    ready?.resume(returning: URL(string:"http://127.0.0.1:\(port.rawValue)")!); ready = nil
                } else if case .failed(let error) = state { ready?.resume(throwing:error); ready = nil }
            }
            listener.newConnectionHandler = { [self] connection in
                connection.start(queue:queue); read(connection, buffer:Data())
            }
            listener.start(queue:queue)
        }
    }
    func stop() {
        queue.async { [self] in
            listener.newConnectionHandler = nil
            listener.stateUpdateHandler = nil
            listener.cancel()
        }
    }
    private func read(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength:1,maximumLength:65536) { [self] data,_,done,error in
            let collected = buffer + (data ?? Data())
            guard error == nil, collected.count <= 1024*1024 else { connection.cancel(); return }
            if let split = collected.range(of:Data("\r\n\r\n".utf8)) {
                let lines = String(decoding:collected[..<split.lowerBound],as:UTF8.self).components(separatedBy:"\r\n")
                let headers = Dictionary(lines.dropFirst().compactMap { line -> (String,String)? in
                    guard let separator = line.firstIndex(of:":") else { return nil }
                    return (String(line[..<separator]).lowercased(),String(line[line.index(after:separator)...]).trimmingCharacters(in:.whitespaces))
                },uniquingKeysWith: { first,_ in first })
                let length = Int(headers["content-length"] ?? "") ?? -1
                if length >= 0, collected.count-split.upperBound >= length {
                    reply(connection,headers:headers,body:collected.subdata(in:split.upperBound..<(split.upperBound+length))); return
                }
            }
            if done { connection.cancel() } else { read(connection,buffer:collected) }
        }
    }
    private func reply(_ connection: NWConnection, headers: [String:String], body: Data) {
        do {
            let digest = SHA256.hash(data:body).map { String(format:"%02x",$0) }.joined()
            guard digest == headers["x-nola-mobile-body-sha256"],
                  let timestamp = headers["x-nola-mobile-timestamp"], let nonce = headers["x-nola-mobile-nonce"],
                  let signature = headers["x-nola-mobile-auth"] else { throw MacRemoteHostError.invalidResponse }
            let message = "v1\nPOST\n/api/mobile/chat/agent/v2/journal\n\(timestamp)\n\(nonce)\n\(digest)"
            guard HMAC<SHA256>.isValidAuthenticationCode(try decode(signature),authenticating:Data(message.utf8),
                using:SymmetricKey(data:Data(Self.token.utf8))) else { throw MacRemoteHostError.invalidResponse }
            let envelope = try JSONSerialization.jsonObject(with:body) as! [String:Any]
            let key = SymmetricKey(data:Data(SHA256.hash(data:Data(Self.token.utf8))))
            let box = try AES.GCM.SealedBox(nonce:AES.GCM.Nonce(data:decode(envelope["nonce"] as! String)),
                ciphertext:decode(envelope["ciphertext"] as! String),tag:decode(envelope["tag"] as! String))
            var input = try JSONSerialization.jsonObject(with:AES.GCM.open(box,using:key,
                authenticating:Data("nola-mobile-bridge-v1".utf8))) as! [String:Any]
            let operation = input.removeValue(forKey:"operation") as! String
            if operation == "heads" { discoveredHeads += 1 }
            let types = ["read":"read_assistant_journal","open_snapshot":"open_assistant_journal_snapshot","record_chunk":"read_assistant_journal_record_chunk",
                         "context_usage":"local_run_context_usage",
                         "heads":"local_run_assistant_journal_heads",
                         "admission_head":"local_run_message_admission_head", "admissions":"read_run_message_admissions",
                         "terminal_seal":"local_assistant_run_terminal_seal", "terminal_manifest":"read_local_assistant_run_terminal_manifest",
                         "release_snapshot":"release_assistant_journal_snapshot"]
            guard let type = types[operation] else { throw MacRemoteHostError.invalidResponse }
            if operation == "read" {
                afterSequences.append((input["after"] as! [String:Any])["sequence"] as! String)
                input["max_records"] = 1 // Exercise short pages, not a single all-history response.
            }
            if operation == "record_chunk" {
                indices.append(input["chunk_index"] as! String)
                if input["chunk_index"] as? String == "1" && !failed {
                    failed = true; try send(connection,status:503,value:["error":"injected page failure"],key:key); return
                }
            }
            input["type"] = type
            let result = try JSONSerialization.jsonObject(with:source.execute(commandData:JSONSerialization.data(withJSONObject:input)),options:[.fragmentsAllowed])
            try send(connection,status:200,value:["source_host_id":"journal-host","result":result],key:key)
        } catch { connection.cancel() }
    }
    private func send(_ connection: NWConnection, status: Int, value: [String:Any], key: SymmetricKey) throws {
        let box = try AES.GCM.seal(JSONSerialization.data(withJSONObject:value),using:key,authenticating:Data("nola-mobile-bridge-v1".utf8))
        let body = try JSONSerialization.data(withJSONObject:["type":"encrypted","v":1,"alg":"AES-256-GCM",
            "nonce":encode(Data(box.nonce)),"ciphertext":encode(box.ciphertext),"tag":encode(box.tag)])
        var response = Data("HTTP/1.1 \(status) Test\r\nContent-Type: application/json\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n".utf8)
        response.append(body)
        connection.send(content:response,completion:.contentProcessed { _ in connection.cancel() })
    }
    private func encode(_ data: Data) -> String { data.base64EncodedString().replacingOccurrences(of:"+",with:"-").replacingOccurrences(of:"/",with:"_").replacingOccurrences(of:"=",with:"") }
    private func decode(_ text: String) throws -> Data {
        let normalized = text.replacingOccurrences(of:"-",with:"+").replacingOccurrences(of:"_",with:"/")
        return try XCTUnwrap(Data(base64Encoded:normalized+String(repeating:"=",count:(4-normalized.count%4)%4)))
    }
}
