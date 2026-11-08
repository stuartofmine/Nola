import XCTest
@testable import Nola

@MainActor
final class MacConversationReasoningAdmissionTests: XCTestCase {
    func testSelectedLowBudgetDoesNotRequireUnselectedBudgetsToFit() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("admission.db"))
        let declaration = MacReasoningConfiguration(control: .budget,
            budgets: [1024, 4096, 8192], outputLimit: 4096)
        XCTAssertEqual(try MacConversationReasoningAdmission.validate(
            declaration: declaration, protocol: .anthropic, outputLimit: 4096,
            supportsReasoning: true, preference: "1024", port: port),
            .object(["kind": .string("budget"), "tokens": .number(1024)]))
        for preference in ["4096", "8192"] {
            XCTAssertThrowsError(try MacConversationReasoningAdmission.validate(
                declaration: declaration, protocol: .anthropic, outputLimit: 4096,
                supportsReasoning: true, preference: preference, port: port))
        }
    }

    func testSelectedPreferenceStillRequiresValidDeclarationAndCapability() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("admission.db"))
        for declaration in [
            MacReasoningConfiguration(control: .budget, budgets: [1024, 8192, 4096]),
            MacReasoningConfiguration(control: .budget, budgets: [1024, 4096]),
        ] {
            XCTAssertThrowsError(try MacConversationReasoningAdmission.validate(
                declaration: declaration, protocol: .anthropic, outputLimit: 4096,
                supportsReasoning: true, preference: "1024", port: port))
        }
        XCTAssertThrowsError(try MacConversationReasoningAdmission.validate(
            declaration: .init(control: .budget), protocol: .anthropic, outputLimit: 4096,
            supportsReasoning: false, preference: "1024", port: port))
    }
}
