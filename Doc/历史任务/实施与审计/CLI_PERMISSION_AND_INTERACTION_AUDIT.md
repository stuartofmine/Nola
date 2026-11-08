> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# CLI Permission and Interaction Audit

## Scope and evidence

This audit covers the Rust CLI permission protocol, interactive cancellation,
durable Goal continuation, tool transcript presentation, and their acceptance
tests. Evidence comes from canonical events in `~/.nola/nola.db`, not only from
terminal output or mock-provider tests.

Conversation `20260901015905807155000-94627-0` recorded a Turn cancelled with
`turn_cancelled.reason = returned_to_composer`. The Goal nevertheless created
successor runs, retried Bash, then completed. Returning to the composer therefore
cancelled one Turn without reliably pausing the durable Goal. That trace is the
regression fixture; the current implementation and PTY test now prove that the
same choice pauses the Goal and creates no successor request.

## Findings

### P0 resolved locally: return-to-composer is an atomic user-control boundary

Required invariant:

> After Return to composer or Escape, no model request, tool invocation, queued
> follow-up, retry, or Goal continuation may start until explicit user input.

The concrete defect was in the CLI Goal loop: it handled terminal reason
`cancelled`, while permission cancellation emits `returned_to_composer`. That
reason fell through to the automatic “missing Goal signal” continuation path.

The interaction now claims the response once, cancels the Turn and effects, pauses
the owning Goal and revoke its lease, clear continuation work, persist that state,
and only then restore the composer.

### P0 test gap resolved locally, pending real-provider validation

The rewritten matrix records run and provider-request counts before returning to
the composer, waits through the former continuation window, then asserts that no
successor run or request exists. Deterministic PTY coverage proves the state
transition; natural replanning and substitution still require the configured
real provider to answer.

### P1 resolved in Core: rejection cannot substitute another tool

Current Core marks `user_denied_permission`, removes tools from the subsequent
model request, injects an explicit no-retry/no-substitution instruction, and
fails the Turn if the model still attempts a tool after one bounded synthesis
retry. The real-provider matrix now verifies that a denied `write_file` does not
fall back to Bash and creates no side effect.

### P1 resolved locally, pending real-provider validation: permission protocols
are isomorphic

Agent Core and `nola-application` now both represent `ReturnToComposer`.
Application persistence, background task resolution, CLI Host mapping, and Core
cancellation use the same typed decision and survive a cold database reopen.

### P1 resolved locally: the UI shows reviewed intent, not executor identity

`bash` is a transport, not the action being reviewed. The UI should show
`git status --short`, `Check shell syntax · build.sh`, or `Read files · 18` and
retain exact command, cwd, arguments, exit status, and output in details.
Presentation parsing must never authorize a call or rewrite model routing.

### P1 resolved locally: read-operation folding uses a real lifecycle boundary

Read results now remain in one live group until a non-read operation, assistant
text, or terminal event closes it. The expanded budget adapts to terminal height
from four to twelve rows; larger groups collapse into one bounded receipt while
canonical events retain every call.

### P1 resolved locally: canonical retention and terminal density are separate

Core storage remains lossless. The terminal uses one stable live row,
completed history cells, semantic grouping of read/list/search calls, and a
height-aware compact history budget. Older operations collapse into summaries;
expanded details expose bounded results and full-result references.

Cancelled shell calls now persist their concrete command as searchable fallback
text rather than the executor name. A 60x24 PTY run with fifty completed reads
leaves one grouped receipt while all fifty ToolParts remain queryable.

### P1 resolved: fixed investigation-call limits contradicted large-workspace use

Core previously rejected the 25th distinct inspection in a mutation epoch. That
was a hidden Turn limit and made the required fifty-read acceptance case
impossible. Distinct investigations are now unbounded; duplicate-call detection,
consecutive-failure protection, cancellation, context limits, and user permission
remain the safety controls.

### P1 resolved: real-E2E transport hid the authoritative provider failure

The real-provider wrapper previously rebuilt an optimized binary on every run,
used a 240-second probe timeout behind a 60-second scenario timeout, and deleted
failure evidence. It now defaults to the debug artifact, reports build/scenario
durations, bounds the probe below the scenario deadline, converts timeout into a
diagnostic assertion, and automatically preserves failed runtime evidence.

## Layer ownership

| Layer | Responsibility | Current weakness |
|---|---|---|
| Agent Core | Evaluate concrete capability/resource and checkpoint decisions | Local contracts pass; real-model denial/replanning remains gated by provider availability |
| Application store | Persist pending requests and one typed resolution | Cold-reopen tests pass for custom and return-to-composer |
| CLI host | Render and collect one response | Concrete shell intent and scoped session choice are rendered |
| Goal coordinator | Lease, pause, resume, and schedule slices | `returned_to_composer` pauses without successor work |
| Transcript | Project canonical events into semantic cells | Height-aware exploration folding passes fifty-read PTY |
| Tests | Prove the entire state machine | Deterministic matrix passes; real-provider matrix remains red externally |

## Module-group audit

| Module group | Contract reviewed | Evidence/status |
|---|---|---|
| Configuration/provider | Tests receive endpoint, model and key only from explicit environment; secrets are redacted | Isolation and syntax pass; upstream currently disconnects |
| Model protocol/streaming | Failed and incomplete streams produce one terminal state and remain recoverable | Deterministic tests exist; real stream blocked upstream |
| Core runtime/catalog | Model selects schema tools; unknown tools refresh catalog once and resume from checkpoint | Both unknown-tool contract tests pass |
| Tool policy/permissions | Core evaluates concrete capability, resource and call; display text never authorizes | Deny and allow-conversation Core contracts pass |
| Foreground interaction | One permission response; Return/Escape stop work before restoring composer | PTY invariant passes |
| Background supervision | Typed permission decisions map identically across foreground/background paths | Cold-reopen and mapping tests pass |
| Goal coordinator | User cancellation suppresses retries, queued work and autonomous continuation | PTY successor-run assertion passes |
| Persistence/projection | Complete ordered Parts retain input, result, state and references independently of compact UI | Application and fifty-read DB assertions pass |
| Transcript/emitter | Current activity stays live; completed exploration is height-aware and bounded | 60x24 fifty-read PTY passes |
| Skills/MCP/data tools | Public capability calls remain subject to the same Core permission protocol | Deterministic contracts present; live Skill E2E not reached while provider is down |
| Context/compaction | Tool history is canonical; model context receives bounded results rather than terminal presentation | Existing Core tests remain required in full regression |
| Localization | Tool display names and permission choices are localized; commands/resources remain literal | Targeted permission UI passes; broad locale matrix remains part of full regression |

## Reference comparison

- **Codex CLI** pauses active Goal work at interruption, finalizes live execution
  into persistent history, parses commands into semantic operations, coalesces
  sequential reads, and bounds compact output while retaining transcript details.
- **Claude Code** prioritizes cancellation over queued work, treats the active
  abort signal as authoritative, claims permission resolution once, and does not
  convert abort into a recoverable tool error.
- **OpenCode** uses structured session permission events and propagates session
  abort through active work; the UI is a projection, not permission truth.

## Real-model test redesign

Tests must read `MODEL_API_URL`, `MODEL_NAME`, and `MODEL_API_KEY` from their
environment. They use a temporary HOME, database, workspace, and log; never the
developer's `~/.nola/config.json`, never the repository as a mutation fixture,
and never print or persist the API key.

Mandatory scenarios:

1. Read-only inspection: structured tools may run; no permission or mutation.
2. Return during a Goal: destructive request, choose return, assert unchanged
   fixture, cancelled Turn, paused Goal, no successor run/provider request.
3. Escape during permission: identical durable guarantees.
4. Allow once: next matching invocation asks again.
5. Allow conversation: survives CLI restart only in the same conversation and
   only for Core's concrete resource scope.
6. Deny: no side effect, Bash substitution, retry loop, or false success.
7. Custom instruction: original call ends and next composer input is a new Turn.
8. Unknown-tool recovery: one bounded recovery, no unavailable-tool loop.
9. Chinese/English natural follow-ups and ambiguous “search/current”: no keyword
   routing or authorization.
10. Projection integrity: compact semantic terminal history while canonical DB
    retains ordered complete calls, arguments, results, and terminal state.

Cancellation cases record run/provider counts before the choice, wait through
the former continuation window, and assert the counts do not increase. Seeing
the composer is not sufficient.

## Remediation order

1. Treat `returned_to_composer` as a user pause in the Goal coordinator and add
   an invariant test that forbids successor runs.
2. Unify foreground, resumed, and background permission decisions.
3. Validate foreground denial with the real provider and canonical trace.
4. Continue replacing executor labels with semantic operation summaries and details.
5. Keep consecutive read results in one live group and close it only at a real
   presentation boundary.
6. Replace permission acceptance tests with the real-model matrix, retaining
   deterministic parsing and transition unit tests.
7. Replay canonical database events from every real-model case as release evidence.

## Acceptance gates

- Return creates exactly one cancelled Turn and zero automatic successor runs.
- Escape has the same guarantee.
- Grants use Core capability/resource scope, never words or display labels.
- No permission screen uses bare `bash` as its primary label.
- Fifty sequential reads render as bounded grouped history while all calls remain
  queryable in canonical storage.
- The environment-configured real-model suite emits redacted evidence and passes.

## Validation status

Local typed-protocol, cold-reopen, unit, PTY, and CLI build checks pass for the
changes above. The environment-configured real provider is currently not a valid
completion signal: a direct request closes after about 5.4 seconds, and the
bounded probe records HTTP 502 with `RemoteDisconnected`. The failed run is
preserved at `/tmp/nola-real-cli-skill-permission.mctcfV`; its provider-ready
scenario terminated in 31.6 seconds instead of hanging behind the old 240-second
probe deadline. The real-model gate intentionally remains red until that external
endpoint returns a model response; no mock result is substituted for it.

## 2026-09-01 validation addendum

This audit is backed by process-level evidence rather than source-shape assertions alone.

- Core workspace: all tests passed.
- CLI binary unit suite: `869 passed; 0 failed`. This includes two 10,000,000-character conversations, bounded terminal projection, Office Skill artifact generation/validation/replay/cold restore, permission persistence, and background-task state.
- CLI PTY suite: `69 passed; 0 failed` in 208.33 seconds. It covers real pseudo-terminal rendering and input, permission choices, Return to composer interruption, Goal/Skill/background restart, repeated compaction, exactly-once side effects, sub-agent cancellation, provider fallback, and dynamic folding of 50 read tools while preserving all canonical ToolParts.
- Canonical process tests passed: session persistence, turn recovery, Web host identity, desktop host identity, pending-effect recovery, and source-layout boundaries.
- Provider-probe tests: `4 passed; 0 failed`.
- The environment-driven real-provider suite now uses `MODEL_API_URL`, `MODEL_API_KEY`, and `MODEL_NAME`, records stage timings, bounds readiness probes, and preserves failure evidence. The configured provider did not complete a request during this run: the upstream endpoint closed the connection without a response, so real-model behavioral scenarios remain externally pending rather than being reported as mock-backed success.

Test-contract corrections made during this audit:

- Permission PTY assertions now validate user-visible localized actions, concrete Bash commands, resource names, permission choices, and durable outcomes. They no longer require internal executor identifiers such as `chrome_native_type_text`, `append_file`, or `stop_task` to leak into the UI.
- Office Skill E2E calls now include the canonical `skill_update.run_id` required by the current protocol.
- The 10M-character cold-restore test keeps an 8-second contention-tolerant default budget for parallel test runs. Set `NOLA_CLI_10M_COLD_RESTORE_BUDGET_MS=3000` to run the strict performance gate.
- Real-provider failures report the failing stage, elapsed time, HTTP/provider error, and bounded output tails instead of timing out with an opaque traceback.

### Real-provider gate hardening after the validation addendum

A subsequent goal continuation found and fixed an actual entrypoint defect that the earlier tests missed: macOS Bash 3.2 with `set -u` rejected expansion of an empty debug-profile array before the real suite started. The wrapper now uses explicit debug/release build branches and exposes a no-network preflight mode. `scripts/e2e/test_real_cli_wrapper.py` executes both branches with `/bin/bash`, rejects unsupported profiles before build/provider work, and verifies that scenario failures are emitted as structured JSON without a Python traceback.

The real gate was then executed again with the repository debug binary. It reached an actual OpenAI-compatible readiness request and failed cleanly with `status=502`, `RemoteDisconnected: Remote end closed connection without response`. An independent valid `curl` Chat Completions request using the same environment produced exit 52 and an empty HTTP response, confirming that the failure is upstream rather than specific to the Python probe. The latest preserved runtime is `/tmp/nola-real-cli-skill-permission.7sCPzH`; direct curl evidence is `/tmp/nola-real-provider-body.ZotbkB`. No API key is written to this audit.
