// SPDX-License-Identifier: Apache-2.0
// Presentation derives from a canonical result, never transport completion text.
import Foundation

nonisolated extension AppleRunResult.Value {
    var blockedFeedback: String? {
        guard kind == .blocked else { return nil }
        let fields = payload.journalObject
        if fields?["reason"]?.journalObject?["code"]?.journalString == "context.input_budget_exceeded" {
            return NSLocalizedString("run.blocked.inputBudget", value: "This request exceeds the model’s input budget. Shorten the conversation or attachments, reduce enabled tools, or choose a model with a larger context window.", comment: "Canonical blocked input budget")
        }
        if let condition = fields?["resolution_condition"]?.journalString,
           !condition.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return condition }
        return NSLocalizedString("Blocked", comment: "Canonical Run result")
    }
}
