// SPDX-License-Identifier: Apache-2.0
// Suggested prompts are drafts that the user can edit before sending.
enum ComposerSuggestion {
    static func inserting(_ suggestion: String, into draft: String) -> String {
        guard !draft.isEmpty else { return suggestion }
        if draft == suggestion || draft.hasSuffix("\n" + suggestion) { return draft }
        return draft + (draft.hasSuffix("\n") ? "" : "\n") + suggestion
    }
}
