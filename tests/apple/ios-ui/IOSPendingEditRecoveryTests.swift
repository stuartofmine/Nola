// SPDX-License-Identifier: Apache-2.0
import SQLite3
import XCTest

final class IOSPendingEditRecoveryTests: XCTestCase {
    func testColdOpenAfterRewindChecksOriginalEditWithoutSending() throws {
        #if targetEnvironment(simulator)
        continueAfterFailure = false
        let owner = UUID(), run = UUID()
        let dir = URL(fileURLWithPath: "/tmp/nola-ios-edit-test-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let app = XCUIApplication()
        defer { app.terminate(); try? FileManager.default.removeItem(at: dir) }
        let database = dir.appendingPathComponent("application.db")
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = dir.lastPathComponent
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database.path
        app.launch()
        app.waitForRootTab("chat")
        app.terminate()

        // Represent the moment after rewind: the previous user bubble no longer
        // exists. Seed only the isolated owner in the v2 schema; no model, run or credentials.
        var db: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(database.path, &db, SQLITE_OPEN_READWRITE, nil), SQLITE_OK)
        defer { sqlite3_close(db) }
        let sql = """
        INSERT INTO sessions(id,title,workspace_root,created_at_ms,updated_at_ms) VALUES ('\(owner.uuidString)','Interrupted edit','',1,1);
        INSERT INTO kv(key,value_json,updated_at_ms) VALUES ('conv_meta:\(owner.uuidString)','{"revision":2}',1);
        """
        XCTAssertEqual(sqlite3_exec(db, sql, nil, nil, nil), SQLITE_OK)
        let folder = database.appendingPathExtension("drafts").appendingPathComponent("conversation-" + owner.uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let intent: [String: Any] = ["operationID":run.uuidString, "targetConversationID":UUID().uuidString, "createdAtMs":2,
            "sourceConversationID":owner.uuidString, "sourceMessageID":"original", "expectedSourceRevision":1,
            "expectedMessageRevision":1, "text":"Keep this exact edit"]
        let file = folder.appendingPathComponent("pending-edit.json")
        let original = try JSONSerialization.data(withJSONObject: intent)
        try original.write(to: file)

        app.launch()
        app.waitForRootTab("chat").tap()
        let row = app.buttons["chat.conversation." + owner.uuidString.lowercased()]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        let recover = app.buttons["chat.message.pendingEdit"]
        XCTAssertTrue(recover.waitForExistence(timeout: 8))
        recover.tap()
        let check = app.buttons["chat.message.editor.save"]
        XCTAssertEqual(check.label, "Check original request")
        XCTAssertEqual(app.textViews["chat.message.editor"].value as? String, "Keep this exact edit")
        check.tap()
        XCTAssertNotEqual(check.label, "Check original request")
        app.buttons["Close"].tap()
        recover.tap()
        XCTAssertEqual(check.label, "Check original request")
        XCTAssertEqual(try Data(contentsOf: file), original)
        var statement: OpaquePointer?
        XCTAssertEqual(sqlite3_prepare_v2(db, "SELECT COUNT(*) FROM events", -1, &statement, nil), SQLITE_OK)
        defer { sqlite3_finalize(statement) }
        XCTAssertEqual(sqlite3_step(statement), SQLITE_ROW)
        XCTAssertEqual(sqlite3_column_int(statement, 0), 0)
        #else
        throw XCTSkip("Requires an isolated simulator database")
        #endif
    }
}
