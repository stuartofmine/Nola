# A2UI runtime and client rendering

[简体中文](A2UI_RUNTIME_CONTRACT.md) · English

A2UI v0.9.1 identifies the external UI wire format. Nola binds a surface to the source Host and Store, conversation, participant context, run, message part, surface ID and generation. A model-supplied surface ID has meaning only inside that binding. The kernel evaluates the component graph and data bindings. The client paints the resulting projection and owns focus, scrolling, animations and unsubmitted edits.

| Operation | Authority and visible result |
| --- | --- |
| Publish | The admitted built-in render_ui operation validates the closed schema. Application commits the surface and the corresponding conversation journal entry. Publishing a form is content rendering. Explicit permission rules still apply to the declared operation. Non-graphical CLI/Host surfaces defer `render_ui` from Turn-1 active schemas while keeping it discoverable on demand via `tool_search` under the same authorization contract. |
| Edit or preview | Pending field edits belong to a specific surface and draft generation. Validate the revision and data bindings. Preview material stays outside the model context and history recall. |
| Submit an action | Validate the actual surface owner, revision and action identity. Application commits the accepted input. A queued submission ID establishes queue admission only. Any subsequent external operation goes through its own tool admission and authorization. |
| Refresh the renderer | A current base can yield unchanged. A valid older base can yield a delta or a full snapshot. An unknown base or changed catalog requires a snapshot. A base ahead of the authority is an error. A missing surface stays missing. |

Historical revisions can be displayed read-only. Opening one cannot revive a previous action handle. A stale action or an unsupported component should produce an explicit recoverable presentation where applicable. Avoid inferring success from button text. The internal surface revision remains available for consistency checks while the chat shows the form and its outcome.

Verify deterministic kernel transitions, atomic publication and replay, then exercise actual form entry, submission, redraw and stale actions on each target client. Markdown diagrams use their existing renderer. [Kernel](../../../../crates/nola-modules/src/a2ui.rs) · [Application journal publication](../../../../crates/nola-store/src/db.rs) · [Client projections](README.en.md)
