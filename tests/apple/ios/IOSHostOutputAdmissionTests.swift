import Foundation
import SQLite3
import XCTest
@testable import Nola

@MainActor
final class IOSHostOutputAdmissionTests: XCTestCase {
    func testHTTPExplicitAdmissionPreservesValidatedRoute() throws {
        try withRuntime { runtime, _ in
            let route = httpRoute()
            let result = try runtime.admitHostOutput(routes: [limits(route)],
                preference: .object(["explicit": .unsignedInteger(2_048)]))
            let object = try object(result)
            XCTAssertEqual(object["routes"], .array([limits(route)]))
            let resolved = try self.object(try XCTUnwrap(object["resolved"]))
            XCTAssertEqual(resolved["source"], .string("explicit_preference"))
            XCTAssertEqual(try number(resolved["tokens"]), 2_048)
            XCTAssertGreaterThan(try number(object["version"]), 0)
        }
    }

    func testNativeDefaultAndKnownCapAreResolvedBySharedDomain() throws {
        try withRuntime { runtime, _ in
            let route: AgentCoreJSONValue = .object([
                "kind": .string("native"), "adapter": .string("mlx"),
                "adapter_contract_revision": .unsignedInteger(1), "model_id": .string("fixture-native"),
                "profile_ref": .null,
            ])
            let defaultResult = try object(runtime.admitHostOutput(routes: [limits(route)], preference: .string("default")))
            let product = try object(try XCTUnwrap(defaultResult["resolved"]))
            XCTAssertEqual(product["source"], .string("product_default"))
            XCTAssertEqual(try number(product["tokens"]), 16_384)
            let cappedResult = try object(runtime.admitHostOutput(routes: [limits(route, output: 4_096)],
                preference: .string("default")))
            let capped = try object(try XCTUnwrap(cappedResult["resolved"]))
            XCTAssertEqual(capped["source"], .string("known_capability_default"))
            XCTAssertEqual(try number(capped["tokens"]), 4_096)
            XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(route, output: 0)],
                preference: .string("default")))
        }
    }

    func testMalformedSecretSourceAndProtocolAreRejectedWithoutStorageWrites() throws {
        try withRuntime { runtime, database in
            try withWriterFence(database) {
                let paths = [database, database.appendingPathExtension("pending-commands.json"),
                    URL(fileURLWithPath: database.path + "-wal")]
                let before = paths.map { try? Data(contentsOf: $0) }
                _ = try runtime.admitHostOutput(routes: [limits(httpRoute())], preference: .string("default"))
                var fields = try object(httpRoute())
                fields["credential_source"] = .object(["kind": .string("none"), "account": .string("not-allowed")])
                XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(.object(fields))], preference: .string("default")))
                fields = try object(httpRoute())
                fields["credential_source"] = .object(["kind": .string("ephemeral")])
                XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(.object(fields))], preference: .string("default")),
                    "Empty credential digest cannot claim a nonempty ephemeral secret")
                fields = try object(httpRoute())
                fields["protocol"] = .string("unsupported")
                XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(.object(fields))], preference: .string("default")))
                XCTAssertEqual(paths.map { try? Data(contentsOf: $0) }, before,
                    "Pure admission must not write the database, WAL or pending mutation file")
            }
        }
    }

    private func httpRoute() -> AgentCoreJSONValue {
        .object([
            "kind": .string("http"), "protocol": .string("responses"), "siwc_account": .null,
            "output_token_field": .string("max_output_tokens"), "provider": .string("openai"),
            "endpoint": .string("https://fixture.invalid/v1"), "endpoint_query_sha256": .null,
            "endpoint_query_source": .object(["kind": .string("none")]), "model": .string("fixture-model"),
            "credential_sha256": .string("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            "credential_source": .object(["kind": .string("none")]),
        ])
    }

    private func limits(_ route: AgentCoreJSONValue, output: UInt64? = nil) -> AgentCoreJSONValue {
        .object(["route": route, "max_input_tokens": .null, "max_total_tokens": .null,
            "max_output_tokens": output.map(AgentCoreJSONValue.unsignedInteger) ?? .null])
    }

    private func object(_ value: AgentCoreJSONValue) throws -> [String: AgentCoreJSONValue] {
        guard case .object(let fields) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        return fields
    }

    private func number(_ value: AgentCoreJSONValue?) throws -> UInt64 {
        switch value {
        case .unsignedInteger(let value): return value
        case .signedInteger(let value): return try XCTUnwrap(UInt64(exactly: value))
        case .number(let value): return try XCTUnwrap(UInt64(exactly: value))
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
    }

    private func withWriterFence(_ database: URL, _ body: () throws -> Void) throws {
        var connection: OpaquePointer?
        let opened = sqlite3_open_v2(database.path, &connection, SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX, nil)
        defer { sqlite3_close(connection) }
        func requireSuccess(_ code: Int32) throws {
            guard code == SQLITE_OK else {
                throw NSError(domain: "HostOutputAdmissionFixture.SQLite", code: Int(code),
                    userInfo: [NSLocalizedDescriptionKey: String(cString: sqlite3_errmsg(connection))])
            }
        }
        try requireSuccess(opened)
        try requireSuccess(sqlite3_busy_timeout(connection, 5_000))
        // Fence unrelated archive-publication writers before taking the byte
        // oracle. A query that actually needs a write cannot succeed behind
        // this independent connection's writer lock; readers remain available.
        try requireSuccess(sqlite3_exec(connection, "BEGIN IMMEDIATE", nil, nil, nil))
        defer { XCTAssertEqual(sqlite3_exec(connection, "ROLLBACK", nil, nil, nil), SQLITE_OK) }
        try body()
    }

    private func withRuntime(_ body: (IOSApplicationRuntime, URL) throws -> Void) throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("application.db")
        try body(IOSApplicationRuntime(databaseURL: database), database)
    }
}
