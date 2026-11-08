# 跨端 UI 对齐历史证据

2026-11-08 从现行对齐清单中分离。以下日期化观察、失败、通过与当时下一步均为历史记录，不作为现行待办或当前实现断言；不修改原测试结果。现行门槛见[跨端对齐](../../UI和交互设计/CROSS_PLATFORM_UI_PARITY.md)。

## Observed starting gaps (2026-09-22)

- Mac API model rows mix always-editable identity, routing, allowed uses, thinking,
  and a separately saved parameter window; hierarchy and save scope are unclear.
- Mac thinking editor offers protocol-incompatible controls; changing the endpoint
  protocol can leave an incompatible persisted declaration.
- Web model center uses a different route-card flow and lacks the same typed
  conversation override contract. A color-only restyle would not close this gap.
- iOS/Android have separate model screens and parameter pipelines; the new Mac
  reasoning and override fields are not yet demonstrated end-to-end there.
- Connected-host execution does not yet carry the new Mac parameter snapshot.

## Progress and remaining evidence

2026-09-22, first alignment pass:

- Mac model rows group secondary configuration; route guidance is subordinate;
  endpoint protocol is visible beside connection identity. Thinking control
  choices are protocol-specific and persistence rejects mismatched declarations.
- Mac targeted suite: 14 tests passed, including two rendered model-page themes.
  Both `/tmp/nola-model-center-light.png` and `-dark.png` were visually inspected:
  no clipping at 900×760; endpoint fields and model actions remain distinguishable.
  The observed missing Chinese model-configuration label was subsequently added.
  This is component evidence, not window-lifecycle acceptance. Narrow widths,
  expanded controls and save/cancel flows still need observation.
- Web now uses Local / API tabs, roving keyboard focus and retained form drafts.
  Node navigation behavior test passed; six existing navigation tests passed.
  No Web rendered parity claim yet. Endpoint CRUD and typed settings still differ.
- Next backend boundary: `web/app/templates/models.html` currently reads/writes
  singleton `/api/agent/settings`; implementation is in
  `cli/src/web_parts/streaming_state.rs` and settings type in `routes.rs`.
  Replace that model-center dependence with canonical endpoint/model profiles,
  rather than inventing another browser-only schema.

Svelte migration foundation:

- `web/frontend` now contains Svelte 5/TypeScript/Vite sources. Rust `/models`
  serves its build directly; `/ui/` serves generated assets through the existing
  path validation and authentication layer. `dev.sh` builds the UI first.
- Local model list, download/start/stop/select/remove, revision-checked parameter
  editing, progress polling, API parameter editing, save/reset/error and tab
  behavior are component-owned; no iframe or legacy model scripts are loaded.
- The API client retains same-origin credentials and the existing CSRF contract.
  API model settings still use the legacy singleton backend at this stage;
  canonical multi-endpoint functionality remains REQUIRED, not deferred scope.
- Type checking and production build passed. Browser skill bootstrap succeeded
  but browser selection returned `Browser is not available: iab`, and discovery
  returned an empty list. Rendered browser acceptance is therefore still missing.
- Public request configuration now shares `RuntimeConfig::with_model_request`
  with the local stdin host: protocol, thinking, generation options and explicit
  zero values reach the runtime. CLI full suite passed 1215 tests, 2 ignored.
  Mac/iOS/Android payload producers still need full typed snapshot integration.

Canonical Web model-parameter pass:

- `/api/model-profiles` projects native canonical remote profiles, grouped in
  Svelte by stored endpoint and model order. Credentials/source metadata are not
  blindly exposed. Existing profiles take precedence over the singleton editor.
- `/api/model-profiles/:id/parameters` uses Application's revision-checked
  parameter command. Patches preserve unrelated fields, selection and identity;
  stale writes fail. Zero values and nullable seed reset are explicit.
- Three Rust tests passed, including isolated SQLite persistence and stale-write
  rejection; six frontend tests passed; Svelte checking had zero errors/warnings
  and the production build succeeded. These are not visual-parity evidence.
- Endpoint CRUD, discovery, global selection and profile-driven Web inference
  are still required. Do not describe the new parameter editor as full model
  center parity or full execution integration.
- Fixed `web/.gitignore` unintentionally excluding `frontend/src/lib`; API and
  model-contract sources/tests are now part of the deliverable worktree.

Workspace navigation alignment pass:

- Svelte model page now uses a Mac-shaped workspace/sidebar structure: workspace
  destinations, searchable actual conversation list, explicit new-conversation
  action, bottom settings entry, narrow-screen hide/show control. It does not
  fabricate pinned or tool-capability state that the current API lacks.
- Conversation links encode opaque IDs. The existing chat initializer now honors
  the explicit ID and reports missing/deleted conversations rather than silently
  opening another conversation. This bridge is transitional; chat still needs
  full Svelte migration.
- Eight frontend tests, zero-warning Svelte check and production build passed.
  Browser bootstrap was retried; selection still reported no `iab` browser and
  discovery returned no browsers. Visual acceptance remains unverified, not pass.
- Mac's separate Capabilities destination, pinned/rename/delete conversation
  controls and native/mobile counterparts still require parity work. The sidebar
  structure change alone is not a claim of full navigation parity.

Mac conversation configuration and preview review:

- Keep model defaults separate from effective conversation values. A sparse
  conversation/model override wins; unset fields inherit. Resetting an override
  must display the underlying defaults, not the previously resolved override.
  Local profiles must not consult the default remote endpoint.
- Numeric editors preserve intermediate input; validate on Save rather than
  clamping every keystroke. Validate inherited reasoning contracts against the
  effective protocol and output limit, including protocol-only overrides.
- Overview sections remain expanded. Session sandboxes show their display name;
  full paths remain available through Copy Path and Reveal in Finder actions.
- Resolve relative Markdown document links against the conversation workspace,
  never the process working directory. Decode paths once and keep web/network
  links out of the local preview route. Resolution is not an access grant.
- PDF uses PDFKit; other common document formats use Quick Look. This does not
  imply that every file format, encrypted file, or inaccessible file can preview.
- Responses HTTP fixture tests pass (4 tests). These verify payload/stream
  contracts, not acceptance by a user's live provider or complete UI parity.
- Targeted Mac tests pass (26): configuration/reset/protocol validation, common
  document routing, real PDFKit loading, relative links, percent filenames and
  existing code-reference behavior. Mac build succeeds; live provider and manual
  visual acceptance remain outstanding.

Mac model-catalog boundary correction (2026-09-23):

- API model discovery is an optional, explicit network action. Saving a manually
  declared model no longer requires a provider `/models` catalog, which many
  compatible and Anthropic Messages endpoints do not expose. The canonical
  configuration store still validates endpoint identity, model identity,
  protocol-compatible reasoning and output limits before persistence.
- An isolated Mac test persists and reloads manually declared Responses and
  Anthropic models at an unreachable endpoint; this is configuration-contract
  evidence, not evidence of live provider inference or visual acceptance.
- API-model edits now have an explicit Discard Changes action with confirmation;
  Save and Discard operate on the same draft rather than leaving edits implicit.
  This still needs window-level focus, narrow-width and unsaved-navigation QA.

Web thinking-editor parity pass:

- Svelte canonical model profiles now edit supported efforts or three Anthropic
  budgets alongside numeric parameters. Endpoint protocol remains authoritative;
  configuring capabilities does not choose a conversation's effort.
- Parameters and reasoning metadata save in one Application transaction, with
  profile revision checking. Invalid reasoning cannot partially save parameters.
  Output-limit edits validate inherited budgets too. Existing selection, enabled
  state, model identity and unrelated parameters/source fields are preserved.
- `UpsertProfile` now supports Application expected-revision checks; the Web
  adapter never implements a separate read-check-write concurrency protocol.
- Five isolated Rust profile tests and ten frontend tests pass. Svelte checking
  has no warnings/errors and production assets build. Browser discovery returned
  no available browsers; this remains unverified visually.
- Application command-receipt regression run also completed successfully: four
  matching tests passed; filtered-out tests are not counted as coverage.
- At this stage Web chat still used the singleton route; explicit canonical
  profile selection is connected in the later pass below. Mac's Keychain-backed
  endpoint credentials must not be confused with Web's singleton credentials.
  Endpoint CRUD, cross-host credential binding and full Svelte chat migration
  remain required; this editor alone is not full Mac/Web functional parity.

Web model-editor interaction pass:

- Local-model deletion, API reset and editor discard use the same Svelte
  confirmation dialog instead of browser `confirm`. Each request has its own
  decision; overlapping prompts cannot reuse approval. Default focus is Cancel.
- Editors prevent native Escape dismissal while saving, lock fields during the
  request, reject duplicate submissions and expose accessible dialog names and
  busy state. Local numeric editors allow valid decimal values off preset steps.
- Twelve frontend unit tests and warning-free Svelte checking pass. Confirmation
  tests cover decision-state logic, not real browser focus, keyboard or layout;
  those still require visual/interaction acceptance.

Web canonical-profile inference pass:

- The transitional chat model chooser lists enabled chat-capable canonical
  profiles and persists their opaque profile IDs with the conversation. Chat and
  manual compaction both send that ID to the same server resolver. Unknown or
  disabled selections are explicit failures, not implicit default-provider use.
- The server loads endpoint identity/protocol and generation defaults from the
  profile, then overlays request parameters without mutating defaults. Sparse
  generation overrides preserve other fields; explicit zero and seed null work.
  Inherited fallback routes and model capability guesses are not reused.
- Credentials are reused only for an exactly matching normalized endpoint, or
  supplied explicitly for this request. An unrelated singleton/config key never
  follows a selected profile. Mac Keychain retrieval and general per-endpoint
  credential management are still missing; unmatched profiles currently send no
  key and authentication-requiring providers will reject them.
- Fixed the legacy send guard treating a running local model as a missing API
  configuration. Send freezes model selection before asynchronous attachment
  preparation; the generation path shares the same availability validation.
- Node tests execute production selection/validation/request-builder functions
  and conversation navigation. These are transition coverage, not evidence that
  chat has migrated to Svelte or that UI parity has been visually accepted.
- Ten Rust profile tests pass, including a local HTTP Responses stream that
  asserts endpoint path, endpoint-bound authorization, selected model, effort,
  session output limit and omitted default sampling. Both Node regression
  scripts pass. No user's live provider or Keychain credentials were accessed.

Svelte login migration pass:

- Rust `/login` GET/HEAD now serves the compiled Svelte app, with a standalone
  login component rather than the legacy template or the workspace sidebar.
  JSON login still uses the existing rate-limited session endpoint and HttpOnly
  cookie flow. Tokens stay in component memory and are cleared after success;
  no browser-storage credential copy was introduced.
- Anonymous GET/HEAD access is limited to generated `/ui/assets/` resources.
  Model pages, profile APIs and asset mutations remain authenticated/blocked.
- Client and server redirect validation normalize paths and reject external
  origins, backslashes, control characters, API/login loops and paths that turn
  into network-path references after normalization.
- Login includes token reveal/validation, sanitized errors, logout/handoff
  notices, command copying with manual-copy fallback, and Chinese/English text.
  Other locale translations and real keyboard/layout acceptance remain pending.
- Fifteen frontend tests and three Rust auth-boundary/session tests pass; Svelte
  checking and production build succeed. Browser discovery was retried and
  returned no browsers, so visual acceptance is not claimed. Legacy login
  template remains for the separate compatibility renderer, not Rust `/login`.

Android generation-parameter and editor parity pass:

- One-shot, text-stream and event-stream provider requests share one validated
  generation-parameter encoder. Explicit output limits are no longer silently
  truncated to 32K; invalid values fail before transport. Memory-worker context
  reservation uses the same validated output limit.
- Model parameter editing preserves the existing Float precision, keeps raw
  invalid input rather than converting pasted negative values into positives,
  and marks invalid fields. Inputs and switches lock during save. Bottom-sheet
  hide transitions refuse dismissal while saving and confirm dirty drafts.
- Chat settings output/context editors use the shared token bounds and keep
  intermediate text. Invalid limits disable Save instead of clamping keystrokes.
- Full Android JVM run: 270 tests, zero failures/errors, two explicitly gated
  real-provider tests skipped (268 passed). New mocked-transport tests inspect
  all three request modes for 64K output, zero temperature and fractional Top P.
  This is not device/Compose gesture acceptance or proof that Android Responses
  reasoning and conversation override integration is complete.

Svelte remote-model metadata editing pass:

- The canonical remote model editor now includes display name, enabled state,
  priority, and the same four purpose capabilities as Mac (chat, retrieval,
  issue repair, sensitive data). These are model routing declarations, not tool
  permission grants. Endpoint identity, credentials and default selection are
  not editable through this operation.
- Metadata, generation parameters and thinking declarations commit through one
  revision-checked Application command. Invalid metadata rejects the entire
  mutation; stale revisions do not overwrite newer edits. Sparse patches retain
  false/zero values and preserve all unowned fields.
- Chat Completions effort choices now match Mac's low/medium/high controls;
  Responses retains the explicit effort choices. Capability drafts copy arrays
  and compare them as sets, so checkbox order is not an edit.
- Sixteen frontend tests, Svelte checking and production build pass. Ten Rust
  profile tests pass, including combined metadata/parameter writes, rejected
  invalid metadata, stale revisions and preservation of endpoint/reasoning data.
  Browser visual/keyboard acceptance is still unverified. Endpoint CRUD,
  discovery, credential management and remaining Svelte route migration are
  not completed by this change.

Mac endpoint responsive-header pass:

- Endpoint identity and actions use a horizontal layout when they fit and a
  stacked layout at compact widths. Names wrap to two lines; names and URLs
  retain full-value help. Delete has an explicit accessibility label.
- The production Mac model view was rendered at 520 and 900 points in light
  and dark appearances with a long mixed-language endpoint name and long URL.
  All four PNGs were inspected: compact headers stack, wide headers remain
  inline, and controls remain visible. This verifies these fixtures, not every
  locale, interaction, or complete model-page design.
- Svelte model identity blocks now shrink/wrap long unbroken text while the
  configure button retains its width. Frontend checking, 16 tests and build pass;
  browser visual verification remains pending.

Svelte model save/read lifecycle pass:

- Model saves apply the canonical response (including its revision) immediately
  instead of depending on a second GET. Older in-flight reads are cancelled so
  they cannot overwrite a newer save receipt.
- Catalog loading publishes only complete snapshots. Empty-profile fallback
  settings must also load successfully before replacing the current view;
  switching to canonical profiles clears the obsolete singleton draft.
- Singleton mutations distinguish write errors from committed writes whose
  refresh failed. Successful clear removes the old draft immediately; a failed
  follow-up read cannot leave a resubmittable old credential/configuration form.
  Reload locks editing and component teardown cancels pending reads.
- Nineteen frontend tests pass, including snapshot failure propagation and
  save-receipt revision replacement; Svelte check and build pass. These tests
  cover the extracted data transitions, not browser interaction acceptance.

iOS numeric editor validation pass:

- Save availability and submission now share the same full validation predicate,
  including ranges, integral Top K, optional seed and raw-text parse failures.
  Invalid dirty drafts show an explanation rather than only a disabled button.
- Raw numeric input retains decimal precision and localized decimal separators;
  the dirty signature separates raw drafts from the last typed parameter field.
- Targeted iOS simulator testing passes for canonical model selection/parameter
  persistence and OpenAI request encoding, including explicit 64K output with
  zero temperature. Parser coverage includes invalid/incomplete text, precision,
  French decimal commas and integer overflow. No live provider was contacted.
  This is simulator unit/integration coverage, not touch/keyboard visual QA.

Svelte capability-page migration pass:

- Rust GET/HEAD `/capabilities` serves the built Svelte application; the legacy
  template is no longer used by this Rust route. The workspace shell selects a
  dedicated skills/MCP page with query-preserving tab selection, search, source
  filter, responsive cards and a native-dialog editor.
- Create/edit/delete and enabled-state changes use existing host APIs. MCP
  submissions carry the original complete server revision baseline and never
  mix unrelated settings into the mutation. Discovery/enable validation remains
  on the host; editing capability availability does not grant tool permission.
- Failed saves preserve the draft. Busy mutations lock controls and dismissal;
  dirty dismissal confirms, before-unload protects unsaved content, and built-in
  skills have no deletion action. MCP tools/resources/templates/prompts render
  as escaped text with descriptor help. New MCP entries default to disabled.
- Twenty-three frontend tests pass, Svelte check reports no errors/warnings,
  and production build succeeds. Two Rust MCP catalog tests pass (baseline
  validation and preserving another editor's addition).
- Browser setup/discovery was retried using the browser skill; no browser was
  available. Visual/keyboard acceptance remains outstanding. Mac category
  grouping, built-in restore, full localization, broader skills concurrency,
  and remaining routes are not claimed complete by this migration.

Capability category and built-in restore parity pass:

- Corrected the Web built-in predicate to consume the actual Rust `built_in`
  boolean and `source: built_in` contract. Earlier UI-only fixtures used an alias
  and therefore failed to catch this real projection mismatch.
- Skills group in Mac's category order, with custom skills in the custom section;
  sorting retains original mutation indices and does not reorder stored data.
  MCP remains a separate group. Unknown categories remain visible as custom.
- Built-in names/descriptions are read-only because the host owns that metadata.
  A confirmed restore uses the server's `default_content`, retains identity and
  enabled state, and leaves failures editable. It does not invent defaults or
  grant permission. The backend test proves content overrides clear without
  re-enabling a disabled skill.
- Twenty-five frontend tests and the targeted Rust restore/split test pass;
  Svelte check and production build pass. Visual acceptance, localization and
  broader cross-editor skill concurrency remain outstanding.

Mac creation/editor parameter consistency pass:

- New remote-model validation and displayed limits now use the shared context
  and output bounds, removing the stale 2M/32K limits. Regression checks accept
  4M context and 64K output through the creation validator.
- Both the creation wizard integer fields and standalone parameter editor keep
  raw numeric text. Invalid/incomplete/out-of-range drafts prevent submission;
  standalone edits count as dirty even if no valid numeric value was parsed.
  Reset and successful saves clear raw drafts. Decimal input no longer formats
  stored precision down to two places.
- Choosing a preset explicitly applies its values and clears only its sampling
  drafts. Manual edits update the preset label without reapplying the matched
  preset and overwriting input.
- Mac workspace behavior and conversation configuration suites complete with
  187 tests, zero failures and one skipped test. These checks do not constitute
  keyboard-level acceptance of every new raw-text editor path.

Mac remote parameter revision parity pass:

- The standalone remote parameter editor now captures its canonical profile and
  revision when opened, like the local and iOS editors. Save uses the Application
  parameter-only command with that expected revision, rather than the reset API.
- Save receipts advance the editor's own revision without re-reading a newer
  concurrent draft as its baseline. Conflicts retain entered values. Missing,
  deleted, malformed, or ambiguous name-only profiles do not become editable
  fallback defaults; Save remains disabled if loading failed.
- Parameter command envelopes carry no model-selection intent. The first test
  run caught this invariant and the envelope was corrected before rerunning.
  Eleven Mac configuration tests then passed, including stale-window rejection,
  sequential saves via commit receipts and source/selection preservation.
- Error recovery currently requires reopening the editor to load a fresh
  snapshot; an in-place reload/merge workflow is not implemented by this pass.

Native parameter reload recovery pass (supersedes the reopening limitation):

- Mac and iOS parameter errors offer an in-place Reload action. Dirty drafts
  require confirmation before replacement; cancelling keeps the draft. Reads
  replace values, version snapshot and clean-state baseline only after success;
  failed reads leave the existing draft and version intact.
- Editors and reset controls stay disabled when no initial snapshot could be
  loaded. Mac uses the shared inline-notice action styling. Reload does not
  silently merge conflicts or grant permission to overwrite a newer version.
- Eleven Mac configuration tests pass after final changes; iOS arm64 simulator
  build succeeds. A generic multi-architecture simulator build failed at x86_64
  link time with unresolved Rust Core symbols. That build gap is outstanding,
  not a successful validation result. Confirmation/keyboard UI acceptance is
  also not established by these compile and persistence checks.

Android parameter reload recovery pass:

- Model parameter errors now offer a reload action. Dirty drafts use the shared
  discard confirmation; busy reads prevent editing, dismissal and resubmission.
  Failed reads retain the snapshot and draft instead of replacing them with
  defaults. Successful reads advance an editor version that explicitly resets
  raw fields, including when persisted numeric values have not changed.
- Snapshot identity gates acceptance of results so an obsolete read cannot
  overwrite a different editor or reopen a closed sheet. Pure state regressions
  cover matching reloads, equal-value snapshots, changed editors and closure.
- Full Android JVM test task passes after the change. Compose touch/keyboard
  confirmation and full mobile protocol parity are still unverified.

Svelte execution-results migration pass:

- Rust GET/HEAD `/results` serves the compiled Svelte page. It uses existing
  read-only result list/detail APIs; no invented delete or conversation actions.
  Active runs and history are separated, with search, status filtering, counts,
  empty/error states and the automation-management link.
- Detail presents summary, provenance/times/ack count, failure, snapshot metadata,
  script stdout/stderr and extracted events. Provider HTML/logs stay escaped text;
  external links accept only HTTP(S) without embedded credentials. List errors
  preserve previous data. Detail replacement/close aborts obsolete reads.
- Active-run refresh is visibility-aware and bounded to one pending read; timers
  and requests are released on teardown. Existing 100-result backend retention
  projection remains; pagination and richer result navigation are not added.
- Twenty-seven frontend tests pass; Svelte checking and build pass. The Rust
  scheduled/result route parser test passes after recompilation. This is not
  a browser-level route/interaction acceptance or complete Mac visual parity;
  full localization and visual/keyboard verification remain outstanding.

### Model thinking defaults across Mac and Svelte Web

- Mac stores thinking preference in the per-conversation/per-model sparse override,
  separately from the model's `defaultPreference`. Switching models retains each
  route's selection; explicit Automatic does not inherit a configured default.
- Web accepts the same canonical reasoning field, preserves it across unrelated
  edits, and exposes a protocol-specific default selector in the Svelte model
  editor. Removing support for the selected default requires an explicit new
  choice before saving; it cannot silently appear as Automatic.
- Web request resolution treats missing `reasoning_selection` as inheritance and
  explicit null as Automatic. Responses maps exact declared efforts; compatible
  Chat and Anthropic map concise/balanced/detailed to effort or budget.
- Regression coverage includes default inheritance, explicit null, explicit
  session effort, invalid defaults, protocol-specific mappings, canonical CAS
  persistence, and a local HTTP Responses request using the inherited default.
  Real provider and browser interaction acceptance remain outstanding. Mobile
  thinking-default parity and remaining Svelte page migrations are not complete.
- Validation: 28 frontend tests, Svelte check (zero errors/warnings), production
  build, and 12 Rust model-profile tests pass for this change.

### Svelte dashboard migration

- `/dashboard` GET/HEAD now serves the compiled Svelte shell. The page uses
  `/api/home` for stored result counts, task enablement, next schedule and latest
  outcomes. It retains automation/history/chat/settings links, with shared page,
  card, button, spacing and dark-mode tokens rather than template-specific styles.
- Failed/partial outcomes take focus priority, followed by active work and the
  next valid enabled schedule. Invalid/past schedules are not promoted. A host
  response is labeled only as connectivity, not proof of healthy model/permission
  configuration. Statistics describe stored records, not an invented time window.
- Refresh retains the last successful snapshot on failure and explicitly marks
  it stale. Active tasks poll every five seconds while visible; in-flight reads,
  teardown cancellation and focus/visibility refresh are bounded. No mutation or
  new permission is introduced by opening the dashboard.
- Validation: 32 frontend tests pass; Svelte check has no errors/warnings and the
  build passes. A real isolated Web Host test verifies all five migrated pages
  return the Svelte fixture on GET and an empty body on HEAD, plus an empty `/api/home`
  snapshot. This proves routing, not the rendered compiled page or visual parity.
- The test launcher now isolates `NOLA_HOME`, waits up to 30 seconds for startup,
  detects early exit, and kills/waits on timeout instead of leaking a child after
  the previous three-second timeout. The final route test leaves no test host.
- Browser discovery returned no available browser. Screenshot/keyboard/visual
  acceptance remains outstanding; dashboard localization beyond Chinese also
  remains work toward full cross-platform parity.

### Automation migration prerequisite: shared-store safety

- Before migrating `/scheduled-actions`, inspection found incompatible native
  agent and Web source/script metadata stored in the same canonical definitions
  table. Both clients used a whole-table reconcile; native decoding also assumed
  every stored record was a native task. This prevents honest functional parity.
- Reconcile now requires explicit format scope at the Application wire boundary,
  and Store enforces scope, atomic collision rejection, scoped tombstones and
  preservation of foreign records/revisions. Both client adapters select their
  decoder by format; Web no longer feeds native metadata to its writer.
- `/scheduled-actions` remains on its existing template until the complete
  semantic execution/editing contract is migrated. Merely porting its old UI
  would preserve a different feature model from Mac; full parity is not claimed.
- Validation: 11 Store/sync tests, 4 CLI schedule tests, 3 Application automation
  port tests and the Mac canonical automation FFI/reopen test pass. Coverage
  includes foreign-record preservation in both directions, missing-scope
  rejection, identity collision rollback and scoped sync tombstones. Concurrent
  edits within the same format still need revision-aware per-record mutations.

### Mobile conversation/model settings scope

- iOS conversation settings previously loaded model defaults into the session's
  response-style binding on appearance, then saved that session style into the
  model parameter record. Removed both directions of this accidental coupling.
- Replaced the duplicate inline default editor with navigation to the existing
  `ModelParametersView`. Both model-center and chat entry points now share raw
  numeric validation, original-revision save, conflict reload, unowned-field
  preservation and discard confirmation. The default navigation back button is
  hidden so its unguarded pop cannot bypass the existing Done/discard action.
- Android's chat settings no longer copy the session style into the model-default
  payload. This is not full Android session persistence: its response-style
  repository still stores an app preference, and the duplicate default editor
  remains. Full per-conversation/per-model overrides and reasoning controls on
  mobile are still incomplete.
- Validation: seven iOS model-selection/parameter tests pass on the available
  arm64 iPhone 18 Pro simulator, including a hosted conversation-settings view
  asserting appearance does not write the response-style binding. Android unit
  tests pass (272 total, two skipped). Full navigation/discard interaction and
  cross-platform screenshot acceptance are not established by these tests.

### Android shared parameter editor

- Moved the model-center parameter sheet and discard UI to `ui/components`, so
  chat settings and Model Center use one editor instead of importing a screen's
  private form or keeping duplicate sliders and numeric validation.
- Chat settings saves only its own settings/memory operation. Model-default edits
  are a separate explicit entry with scope text, fresh revision baseline on open,
  awaited save, retained draft/error on rejection and confirmed reload. Completion
  from an old selected model cannot close or reset the new model's editor.
- Added stream-output control to the shared editor so replacing the chat form
  does not remove that option. Web-host targets retain their existing restriction.
  Unedited fields, including response style and retrieval, remain untouched.
- Added device interaction coverage for the shared form and the chat entry path,
  including disabled Save for an invalid draft and reload/discard behavior.
  This change does not resolve Android's global response-style repository or
  implement all per-conversation generation/reasoning overrides.

### Thinking validation parity follow-up

- Mac preserves a stale conversation thinking selection instead of silently
  replacing it with Automatic. The conversation editor exposes inheritance and
  explicit provider-default choices separately, and invalid effective choices
  block Send. Its draft identity follows the selected model route.
- Web request resolution now validates the effective selection: a valid explicit
  session override (including null/provider default) is not rejected because a
  shadowed model default became invalid. Unsupported explicit efforts still fail.
- The Svelte model editor validates protocol compatibility, declared effort
  choices, increasing integer budgets and the current output limit before Save.
  Unsupported stored effort entries remain visible and removable. Invalid
  defaults remain visible until corrected rather than being silently replaced.
- Validation: Mac's 26 configuration tests, 35 frontend tests and 12 Web model
  profile tests pass. Svelte check reports zero errors/warnings; production build
  succeeds. These checks do not establish browser visual acceptance, complete
  conversation-page migration or full iOS/Android thinking configuration parity.

### Web canonical default-model selection

- Svelte API model cards now expose Set as Default for enabled chat-capable
  models. The operation sends the displayed profile revision to Application's
  existing selection transaction, rather than writing singleton host settings.
- The response refreshes the complete remote profile list because selecting one
  model also deselects and advances the revision of the previous default. Old
  in-flight catalog reads are aborted before applying the returned snapshot.
- The current chat page's Auto option now resolves the canonical selected remote
  profile and freezes its ID into the request. Explicit conversation selections
  win; unavailable defaults fail instead of silently selecting a different route.
- Validation: 36 frontend tests, Svelte check/build, 13 model-profile Rust tests,
  and chat routing/navigation JavaScript tests pass. Selection coverage includes
  persistence, both profiles' revisions, unchanged parameters, stale revision
  rejection, disabled endpoints/models, and non-chat capabilities.
- Remaining: endpoint CRUD/discovery/credentials, chat Svelte migration, automatic
  cross-tab catalog invalidation and rendered browser acceptance are not proven
  by this change. Existing chat caches receive changed defaults on catalog reload.

### Svelte local model editor recovery

- Local parameter failures now stay in the editor rather than sharing the page's
  catalog error state. Reload requires discard confirmation and only replaces
  the draft/revision after a successful read of the same installed model.
  Missing models, invalid revisions and failed reads retain the existing draft.
- Catalog reads have an abortable latest-request owner. Superseded responses or
  errors cannot replace a newer read, including transports that ignore abort.
  Mutations and teardown cancel older reads; successful refresh clears stale
  read errors. Failed reads label retained catalog data rather than presenting
  an indefinite loading state.
- Download polling pauses on hidden documents and the API tab, then refreshes
  when the local tab/window becomes active. Switching tabs preserves mounted
  model editors and API drafts.
- Validation: 41 frontend tests pass, including ordering/cancellation and editor
  snapshot tests; Svelte check and production build pass. Browser skill bootstrap
  and discovery returned no browser, so actual modal interaction and narrow/
  dark visual acceptance remain unverified. No substitute visual claim is made.

### Parallel model-editor and Markdown pass

- Mac thinking inputs retain raw numeric drafts; invalid or overflowing budgets
  cannot silently reuse an older value. Invalid default efforts and incompatible
  controls stay visible so users can correct them. Schema checks reject negative
  inactive budgets/unknown efforts too, matching Web's configuration boundary.
  Eighteen reasoning/configuration tests pass after the schema follow-up; the
  380pt light/dark editor snapshot test also passed in the initial UI validation.
- Android's shared editor fixes its comparison and submission baseline to the
  editing version, preventing an external refresh from silently rebasing an open
  draft. Failed saves reveal the error and announce it to accessibility services.
  Five device interactions passed; light/dark screenshots were inspected at the
  tested device size. The emulator was not restarted or cleared.
- iOS draft comparison is structural rather than delimiter-joined text. Re-entering
  an unchanged displayed number is not dirty; incomplete input remains dirty and
  free-text fields cannot collide. Invalid loaded parameters show errors, large
  accessibility text stacks numeric rows, and the keyboard has a Done action.
  Conversation settings disable edits/navigation/dismissal during awaited saves.
  Seven translations now describe the actual model-default scope. All nine tests
  in ModelSelectionCanonicalEndToEndTests passed on the available iOS 27 iPhone
  18 Pro simulator; the run guard left no booted test devices. Dynamic Type and
  save-lock interaction screenshots still need full device visual acceptance.
- Results now uses a reusable Svelte Markdown component for conclusions and AI
  summaries. It renders headings, emphasis, nested/task lists, quotes, tables and
  code blocks with copy actions. Tables/code have keyboard-focusable overflow;
  paragraph/list spacing and typography use the shared theme in light/dark mode.
- The renderer follows the token/component approach observed in the reference
  Open WebUI implementation. Marked produces tokens; Svelte escapes text and
  creates semantic nodes. Raw model HTML is never assigned to an HTML sink.
  Link schemes/credentials are checked, and images are explicit links rather
  than automatic remote loads. Logs and error diagnostics remain literal text.
  API references: https://marked.js.org/using_pro and
  https://github.com/fb55/entities (entity decoding is only used for text/URLs,
  never to produce executable markup).
- Web validation: 46 tests, including rendering the actual Svelte component on
  the server and checking semantic markup, code literals, dangerous URLs/HTML,
  tables, task lists and incomplete fences. Check/build pass. No browser is
  available for visual interaction acceptance. Math, host-backed file previews,
  image previews and full chat migration remain incomplete.

### Endpoint configuration and conversation tone isolation

- Svelte now edits existing endpoints (name, base URL, protocol and enabled state).
  The draft freezes every member model's revision and reasoning contract. Invalid
  protocol changes remain visible instead of clearing model settings implicitly.
  Saving uses the Application endpoint command; membership/revision checks, model
  updates and sync outbox writes commit in one Store savepoint. Success returns
  the full canonical remote-model projection. Model parameters, selection,
  unrelated endpoints and credentials are not overwritten.
- Validation: 50 frontend tests, Svelte check and production build pass. Backend
  validation passes 16 model-profile tests, two Application port tests and one
  real Host HTTP test (encoded endpoint ID, protocol change, complete projection,
  stale-revision rejection). Injected second-model write failure rolls back both
  the first model and its outbox changes. Browser visual acceptance is still
  unavailable. Endpoint creation/deletion and credential management are separate
  outstanding work.
- Mobile conversation tone uses optional shared `promptResponseStyle`, distinct
  from Mac's concise/balanced/detailed response style and reasoning effort. An
  absent/null value inherits the selected model's default; explicit values stay
  scoped to the conversation. Saving awaits persistence and retains foreign JSON
  fields. Android validates the three wire values at its write boundaries.
- Android validation: 276 JVM tests (274 passed, two skipped), eight device tests
  passed, plus six targeted JVM tests after the write-boundary validation follow-up.
  Device coverage includes cold reopening, conversation isolation, inheritance
  restoration and unchanged model defaults. No emulator reset was needed.
- iOS conversation-configuration validation passes all 12 canonical model
  selection tests, including rejecting non-object settings JSON. Mac metadata
  writes now preserve mobile tone/unknown fields while still removing cleared
  Mac-owned optional fields; all 13 conversation-configuration tests pass.
- Expanded iOS ChatManager acceptance initially reported eight failures across
  seven cases (25 tests executed), exposing an incremental-host-response contract
  mismatch and a message-source decoding error. Accepted Host deltas now preserve
  an absent snapshot instead of rejecting them or fabricating a full state;
  dispatch/start transitions still require a full snapshot. Progress/accepted
  responses also retain their delivery lease until a terminal response, matching
  Core's response contract. All 25 ChatManager tests, seven application-contract
  tests and 13 projection-contract tests pass after this fix. The message-source
  error did not reproduce; no ID decoder/type change was made.
- Final iOS protocol acceptance passes all 46 tests, including a real-Core
  duplicate-response/cold-reopen case. Its fixture now uses the production Host
  response envelope and execution/model-start acknowledgements rather than
  omitting the delivery lease or jumping directly to completion. Assertions still
  verify deduplication and reopening, and additionally verify that an actual
  execution-start delta omits the snapshot. Product permission checks remain
  strict. Log: `/tmp/nola-ios-delta-protocol-verified.log`.
- Settings writes still perform read/merge/whole-object replacement, not CAS.
  Unknown-field preservation does not solve concurrent cross-client lost updates;
  a revisioned field-patch contract remains required.

### Web localization migration

- 2026-09-23 follow-up: the still-active legacy `/chat` now exposes the same
  seven-language preference as Svelte, persisted through `nola.web.locale` and
  the strict `nola_ui_locale` cookie. Switching with an unsent draft asks first.
  Its German, Spanish, French, Japanese and Russian catalogs now cover all 622
  literal translation keys (108 missing entries per language were added), with
  placeholder and duplicate-key checks in the normal frontend test command.
  The live Rust Host serves the locale module and honors all seven explicit
  preferences independently of authentication. This does not mean the legacy
  chat has been migrated to Svelte or that browser visual/IME acceptance passed.
- A seven-language model-display catalog now covers previously literal Top P,
  Top K, Min P, Flash Attention and effort labels. The wire effort values stay
  unchanged. Conversation timestamps use the chosen locale, invalid times show
  an explicit unavailable mark, and the automation `local` time-zone label is
  translated. Frontend validation is 173 passing tests, zero Svelte diagnostics
  and a successful production build.

- The original Svelte migration omitted localization: many components contained
  Chinese literals and the document declared Simplified Chinese unconditionally.
  A shared locale store now resolves a saved preference before supported browser
  languages, with English fallback. Preferences support system/default plus
  Simplified Chinese, English, German, Spanish, French, Japanese and Russian;
  the document language and component labels update without discarding drafts.
- App navigation, login, confirmation defaults and Markdown controls use complete
  seven-language catalogs. Local model management has its own complete catalog,
  including parameter labels, destructive confirmations, status and aria labels.
  The endpoint credential editor also uses a complete seven-language catalog;
  its errors/status store message keys so changing language updates them.
  API model settings, remote model configuration and endpoint editing now use
  complete seven-language catalogs too, including thinking-tier display labels.
  API calls now carry the selected locale in Accept-Language. User model names,
  conversation titles, Markdown content/code and provider diagnostics are not
  translated or interpreted as HTML.
- Tests enforce locale coverage and interpolation-placeholder parity, language
  preference persistence/restricted-storage fallback, request language and actual
  Svelte Markdown rendering in every supported language. This coverage applies
  to migrated catalogs, not the entire Web product. Remaining feature pages,
  helper validation messages, backend error codes and legacy HTML preference
  integration still need localization migration. Browser discovery returned no
  available browser; rendered visual/interactive acceptance remains unverified.
  Final integration validation for this pass: 61 frontend tests pass, Svelte
  check reports zero errors/warnings, and the production build succeeds. The
  overview, execution-results and capabilities pages still need text extraction;
  model helper/backend error messages also remain untranslated in some paths.

### Remaining Svelte localization pass

- Overview, execution results and capabilities now use complete seven-language
  catalogs (101 activity keys and 70 capability keys). Status/format/frequency
  labels, date/time formatting, search against displayed formats, confirmations
  and accessibility labels follow the selected locale. Unknown protocol values,
  user content and MCP descriptors remain unchanged.
- Model validation now returns structured code/parameters/nested cause data;
  throwing paths retain typed issues too. Seventeen validation messages have
  complete translations and re-render with the selected locale. No language-text
  matching is used to determine behavior or validation outcomes.
- Catalogs share one registry. Tests check all registered languages, duplicate
  keys, placeholder parity and literal Svelte translation-key references. System
  locale changes affect system-mode users only; explicit choices synchronize
  across tabs, with listeners removed on teardown. Missing-locale helper defaults
  use English, matching the app fallback; UI calls pass their selected locale.
- Final validation: 71 frontend tests pass, Svelte check has zero errors/warnings,
  production build succeeds, and source inspection finds no Chinese literals in
  Svelte page/component files. This is source/contract evidence, not visual
  acceptance or proof that all backend diagnostics are localized. Language
  preference integration with still-legacy HTML pages, backend error contracts,
  full chat/data/settings/automation migration and browser visual acceptance
  remain incomplete.

### Data-source page migration and locale bridge

- `/data-sources` GET/HEAD now serves Svelte. The page supports the existing
  Host-local file/HTTP source API: creation, editing, deletion, explicit testing,
  access policies, saved validation contracts and provenance/usage records. It
  uses the shared shell, confirmations, theme and complete seven-language catalog.
  Operation status reflects actual pending requests, not timer-generated phases.
- Sources and usage have independent cancellable reads and error states. Writes
  invalidate older reads; failed validation preserves the editor and refreshes
  server state because this API can persist a failed draft before returning 400.
  A new editor keeps one client-generated source ID across retries. Unknown cloud
  disclosure remains unknown, and usage history is taken from the server rather
  than optimistically removed with a source.
- CLI local ingestion now rejects private vault paths, case variants and symlink
  targets before reading/sampling. Eight source unit tests and a real isolated
  Host HTTP test pass. The HTTP test covers Svelte route GET/HEAD, file and mock
  HTTP sources, CRUD/testing, policies, rejected inputs, failed records, private
  vault isolation and same-ID failed-create retry without duplicates.
- Frontend acceptance: 78 tests pass, Svelte check has zero errors/warnings and
  production build succeeds. Browser interaction/visual acceptance remains
  unverified. Mac-specific platform/health sources and their authorization APIs
  are not exposed by Web; this page does not pretend those capabilities exist.
  Client revision CAS, broader local/network access policy and filesystem race
  protection are not completed by this migration.
- Svelte writes a strict seven-value, non-sensitive `nola_ui_locale` cookie so
  legacy HTML navigation can retain the chosen language. System mode clears it;
  HTTPS adds Secure, and all writes use Path=/ and SameSite=Lax. Rust prefers a
  valid unique cookie over Accept-Language, rejecting aliases, encoded/HTML values
  and duplicates. Three Rust unit tests and frontend cookie/preference tests pass.
  A separate real-Host HTTP test verifies both legacy pages across seven languages,
  invalid/system preferences, browser/host fallbacks and unchanged session/Bearer
  authentication and logout. New source IDs use getRandomValues rather than
  requiring randomUUID, including a test with no randomUUID capability; actual
  LAN browser interaction remains unverified.

### Turn identity and large-number presentation follow-up

- Android interactive turns now capture the conversation's chosen model,
  parameters and a dedicated cloud client. Context preparation and subsequent
  tool/model requests use this same snapshot, rather than mutable global model
  state. Local engine replacement fails explicitly. New conversations persist
  their model and prompt tone in the initial settings object. Full JVM acceptance:
  281 tests, 279 passed and two skipped; A/B/A mock HTTP requests verify model,
  sampling/output parameters, endpoint and authentication isolation. No device or
  real-provider acceptance was performed in this follow-up.
- iOS structured projection number display uses checked Int64 conversion before
  falling back to Double text, preventing traps at the rounded Int64 upper bound
  or large valid JSON numbers. All 16 projection tests pass, including three new
  boundary/normal-formatting cases. This does not change JSON or identifier types.

### Web settings localization and migration (2026-09-22)

- `/settings` now uses the Svelte shell, including browser-session status,
  assistant permissions and messaging-worker/channel configuration. These
  controls share the seven-language catalog, explicit language selection and
  browser-language fallback used by the other migrated pages. The existing
  locale cookie keeps the preference when navigating to remaining legacy pages.
- Permissions use canonical `bash_prefixes` and sparse owned-field writes;
  unknown profiles cannot silently become broader permissions. Drafts survive
  failed requests and allowing all operations requires explicit confirmation.
- Browser logout is distinguished from trusted loopback access. The old password
  form only redirected without changing a password; the replacement accurately
  describes Host-token authentication instead of presenting a nonfunctional form.
- Messaging configuration does not display saved tokens. Changing service identity
  requires an explicit replacement or clear action; enabled connection saves
  explain that they can register a webhook. Development checks use mocks only.
- Frontend validation: 93 tests passed, Svelte check reported zero errors and
  warnings, and the production build passed. Nine backend unit tests and two
  isolated real-Host HTTP tests passed, including locale/auth separation, token
  isolation and preservation of model parameters and nonempty MCP configuration.
  Sidebar errors retranslate immediately without displaying remote error bodies.
  Browser visual acceptance is not
  completed. Chat still uses a legacy page; this is not full Mac
  parity, and settings writes still lack cross-client revision CAS.

### Web automation migration and shared revision checks (2026-09-22)

- `/scheduled-actions` uses Svelte, shared page/card/theme styles and seven-language
  labels. It preserves source-task creation, enable/disable, explicit manual run,
  full script review, deletion and task-scoped result links. Weekdays follow the
  backend's Sunday=1 convention; invalid times/days and unsupported timezones
  fail validation rather than silently becoming another schedule.
- The API advertises actual execution capabilities. This Host currently supports
  manual source-task execution, not Mac's prompt/skill/context automation or
  scheduled Web source dispatch. The page states this limitation instead of
  presenting an estimated date as a scheduled job.
- Script approval and enable/disable use a shared Application conditional-write
  command. Store revision validation, row update and sync event are transactional.
  The UI reviews the complete script and sends the reviewed revision; stale
  requests conflict instead of approving new content.
- Canonical identity/name/format/enabled/revision override untrusted metadata
  projections. Completing a run no longer writes its old definition snapshot:
  latest-result display is projected from run records, preserving edits/deletion.
- Results can be scoped by exact opaque source ID, including active rows and
  summary counts. Activity transport errors are stable localized keys rather
  than raw backend messages; stored task output remains original content.
- Migrated routes load their page modules on demand. The initial JavaScript
  entry decreased from about 502 kB to 289 kB (minified, before compression);
  this is a build-size measurement, not a measured browser-latency claim.
  Page-load failures offer a full reload so stale deployment chunk URLs can recover.
- Validation: 108 frontend tests, including actual Svelte server rendering in
  seven languages, passed; type checking reported no errors/warnings and the
  production build passed. Five Rust Store/Application/projection tests and one
  isolated real-Host HTTP test passed. The latter executes only a temporary local
  file task, verifies result linkage and unchanged definition revision, and never
  executes a script or external network task. These checks do not replace browser
  interaction tests.
- The larger architecture mismatch and remaining requirements are recorded in
  [the automation contract](../../软件架构设计/模块/12-记忆数据源与自动化/AUTOMATION_CROSS_PLATFORM_CONTRACT.md). In particular,
  this migration does not complete shared scheduling, Mac task-editor parity,
  revision-bound deletion or idempotent manual runs. Browser visual acceptance
  remains unavailable because the in-app browser has no connected backend.

### Chat migration foundation (2026-09-23)

- Chat is still served by the legacy `/chat` route. The complete replacement
  gates are recorded in [Web chat acceptance](WEB_CHAT_SVELTE_ACCEPTANCE.md),
  with the current wire contract in [transport contract](../../软件架构设计/模块/14-远程Host与同步/WEB_CHAT_TRANSPORT_CONTRACT.md).
  No partial Svelte chat screen has replaced existing functionality.
- `sse-json-decoder.mjs` is now the shared framing implementation used by the
  legacy sender and the new typed Svelte transport. The duplicate parser was
  removed from the trace runtime. CRLF split across chunks no longer invents
  blank lines; malformed/truncated events fail instead of silently disappearing
  or accepting a partial durable commit. UTF-8 boundaries, cancellation and
  same-origin CSRF/language handling have focused coverage.
- New `chatTimeline`, `chatTurn` and `chatScroll` modules separate transcript
  identity, submission durability and viewport intent. Canonical Part snapshots
  replace by identity rather than concatenate; pagination watermarks and Agent
  cursors are not compared across unrelated journals. Explicit submission binding
  rejects stale callbacks. Only a host/session/submission/message-matched durable
  commit releases a turn; permission acknowledgements, Trace terminal events,
  EOF, cancellation requests and `[DONE]` do not.
- Scroll intent has generation-bound layout tokens, explicit user ownership and
  stable history anchors. These modules are tested foundations, not yet a claim
  that the legacy page's composer, scrolling or session configuration has been
  replaced. Missing history stream-owner metadata remains a recovery constraint.
- Validation: 144 frontend tests pass, including byte transport → timeline → turn
  integration using the actual Core durable-commit fixture. Type checking and
  production build pass. Two isolated Host HTTP tests verify the shared module's
  rendered URL, JavaScript MIME type, GET/HEAD serving and the existing locale/auth
  contract. The migrated standalone decoder test also passes. No real model or
  browser/IME/scroll visual acceptance was performed for this foundation.

### Endpoint credential verification evidence

- The Web credential dialog is backed by an endpoint/URL/protocol-bound private
  host vault and revision-checked replace/clear operations. Inference consumes
  the bound credential rather than sharing a different endpoint's token. A clear
  tombstone prevents legacy fallback from silently restoring it. Static serving
  resolves symlinks and rejects the vault directory/out-of-root files.
- Four vault tests, two static-boundary tests and one dual-mock real HTTP/inference
  acceptance test pass. Coverage includes replacement, clear, URL changes,
  revision/identity conflicts, malformed-request redaction and absence of new
  secrets from HTTP output, captured logs and canonical database/WAL. These are
  isolated mock-provider tests, not live paid-provider acceptance.
- The vault is Unix permission-protected plaintext, not encryption; Windows vault
  management fails closed. Legacy singleton credentials remain in their former
  storage and were not migrated by this change. Canonical/vault checks do not
  constitute one cross-database transaction. See the canonical-transactions
  document for this boundary and the remaining migration requirements.
