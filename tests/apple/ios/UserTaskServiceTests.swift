// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements UserTaskServiceTests logic in the project module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
import XCTest
@testable import Nola

final class UserTaskServiceTests: XCTestCase {
    func testUserTaskDefinitionSurvivesCanonicalColdReopenAndDeletesAsTombstone() throws {
        let databaseURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-user-task-\(UUID().uuidString).db")
        defer { try? FileManager.default.removeItem(at: databaseURL) }
        let createdAt = Date(timeIntervalSince1970: 1_778_000_000)
        let task = UserTask(
            id: UUID(),
            type: .reminder,
            title: "Review architecture",
            description: "Confirm the Application ownership boundary",
            targetDate: createdAt.addingTimeInterval(3_600),
            createdAt: createdAt,
            updatedAt: createdAt
        )

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
            let storage = try LocalJSONStorage(applicationRuntime: runtime)
            try storage.saveUserTask(task)
            XCTAssertEqual(try storage.getUserTask(id: task.id), task)
        }

        do {
            let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
            let storage = try LocalJSONStorage(applicationRuntime: runtime)
            XCTAssertEqual(try storage.getAllUserTasks(), [task])
            try storage.deleteUserTask(id: task.id)
            XCTAssertNil(try storage.getUserTask(id: task.id))
            XCTAssertTrue(try storage.getAllUserTasks().isEmpty)
        }
    }
}
