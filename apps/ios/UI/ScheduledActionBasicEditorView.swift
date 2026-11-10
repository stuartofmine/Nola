// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledActionBasicEditorView logic in the iOS UI layer.
// - Interfaces:
//   - ScheduledActionBasicEditorView
// - APIs:
//   - none
//
import SwiftUI

struct ScheduledActionBasicEditorView: View {
    let existingSource: URLDataSource?

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 16) {
                Text(NSLocalizedString(
                    "scheduled.actions.editor.removed.title",
                    value: "Manage automations in AI chat",
                    comment: "Scheduled action manual editor removed title"
                ))
                .font(.title2.weight(.bold))

                Text(NSLocalizedString(
                    "scheduled.actions.editor.removed.body",
                    value: "Nola no longer provides a manual form for scheduled tasks. Web fetch, search automation, pause, delete, and edit can all be done in chat with natural language. Local reminders go to the task and notification system instead of URL automations.",
                    comment: "Scheduled action manual editor removed body"
                ))
                .foregroundColor(.secondary)
                .fixedSize(horizontal: false, vertical: true)

                if let source = existingSource {
                    Text(source.name)
                        .font(.headline)
                    Text(source.url)
                        .font(.callout)
                        .foregroundColor(.secondary)
                        .textSelection(.enabled)
                }

                Spacer()
            }
            .padding()
            .navigationTitle(NSLocalizedString("scheduled.actions.redesigned.title", value: "Automation & Scheduling", comment: "Scheduled actions redesigned page title"))
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}
