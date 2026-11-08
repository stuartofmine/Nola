> **单版本开发约束：Nola 仅有一个未发布的内部开发版本。只保留当前架构、接口、协议和数据格式；删除旧版本兼容层、旧字段别名、双读/双写、兼容回退及历史数据迁移/转换代码，不为旧接口保留实现。文中历史兼容或数据转换计划不再执行，历史记录不构成保留旧实现的要求。不匹配的协议或存储格式明确拒绝；不得自动删除、重建或改写用户数据。当前版本的会话恢复、事务回滚和外部服务协议适配不属于旧版本兼容。**

# CLI / Web / macOS Alignment Audit

Date: 2026-09-02

## Executive verdict

CLI, Web, and macOS already share the Rust Agent Core contract, canonical Part
lifecycle fixture, Application Store semantics, capability catalog, and Agent
Host protocol. They are not three independent agents. The remaining gap is at
the product boundary: each surface currently proves a different subset of the
same lifecycle with a different Harness, so a green test on one surface does
not prove that submission, permission suspension, streaming, terminal
reconciliation, and the next user input work together on the other two.

The target is semantic alignment, not identical UI:

```text
original user input
  -> Application command
  -> Agent Core transition
  -> committed Host effect
  -> correlated Host response
  -> canonical Core Part events
  -> committed ConversationProjection
  -> platform-native presentation
```

CLI remains a terminal product, Web remains the browser product served by the
CLI Rust host, and macOS remains a native SwiftUI/AppKit product connected to
`nola-agent-host`. macOS must not launch or parse the CLI UI.

## Normative ownership

| Concern | Canonical owner | CLI | Web | macOS |
| --- | --- | --- | --- | --- |
| Transcript and context usage | Application `ConversationProjection` | Render projection | Replace canonical cache by revision | Replace canonical cache by revision |
| Text, reasoning, tool, source and artifact lifecycle | Core Parts | Terminal projection | JS production reducer | Swift production reducer |
| Turn completion, failure and cancellation | Core terminal event | Return composer | Stop stream and release composer | Stop visible run before process EOF |
| Pending user and assistant shell | Client overlay with submission-owned IDs | Active terminal turn | Immediate pending bubble | Immediate native bubble |
| Permission suspension | Core pending effect and permission projection | Modal terminal state | Browser permission surface | Native permission surface |
| Durable permission scope | Core/Application rules | Profile plus scoped grant | Same rule semantics | Same rule semantics |
| Host I/O and retries | Agent Host | In-process Host adapter | Same Rust Web Host | `nola-agent-host` local/remote adapter |
| Diagnostics | Trace journal | Debug output | Inspector/debug UI | Inspector/debug UI |

Trace, transport `done` frames, process EOF, and animation completion are not
product-content authorities. None may append or replace assistant text.

## Lifecycle contract

Every product surface must implement these states with the same meaning:

1. `idle`: no active Turn; input accepts focus.
2. `submitted`: the user message and one empty assistant shell are visible
   before the first provider byte.
3. `reasoning`: stable reasoning Part identity; detail changes do not recreate
   the whole bubble.
4. `tool`: the current canonical tool Part is visible; completed historical
   calls remain available in disclosure without flooding the primary status.
5. `waiting_for_permission`: the same Turn and Host effect are suspended. A
   scoped grant resumes that effect; returning to input cancels the Turn.
6. `streaming`: one Text Part grows monotonically. Markdown and reveal pacing
   are presentation overrides, not alternate text buffers.
7. `terminal`: exactly one Core terminal transition is observed and the
   committed projection is reconciled without a second final-body insertion.
8. `idle_after_terminal`: the composer is usable immediately and a second
   input creates a normal successor Turn without a hidden model request.

## Current evidence

| Boundary | Existing evidence | Assessment |
| --- | --- | --- |
| Core Part lifecycle | Core-generated `canonical_part_lifecycle_v1.json` covers stable identity, reasoning, permission wait, tool progress/result, sources, citations, replacement, completion, cancellation, and late response rejection. | Strong and authoritative. |
| CLI projection | Rust tests consume the shared fixture for human and JSON projection. Real PTY tests cover cursor/input, permission return, cancellation, context footer, Goal completion, and a second post-Goal input. | Strongest full interaction Harness. |
| Web projection | Playwright executes the production `agent-trace-runtime.js` reducer against the shared fixture. Rust tests inspect terminal projection handling and composer release in `chat.html`. | Part reducer is strong; composer assertion is still partly structural rather than a browser interaction. |
| macOS projection | A standalone Swift Harness compiles the production decoder/reducer and consumes the shared fixture. XCTest covers immediate terminal visibility, Goal completion, cancellation, tools, reasoning, Markdown, and native surfaces. | Strong reducer evidence; product lifecycle remains split across several XCTest cases. |
| Cross-surface release gate | Tests were previously launched independently and could be omitted after a change in another surface. | Missing before this audit; added by `scripts/test_cli_web_mac_alignment.sh`. |

## Findings

### P0: independent green suites did not prove one shared lifecycle

The three surfaces used the same Core but did not have one required command
that exercised the shared fixture and terminal-to-next-input behavior across
all three. This is why a CLI PTY regression could remain invisible to Web or
macOS tests, and why template assertions could pass while browser interaction
was wrong.

Resolution: `scripts/test_cli_web_mac_alignment.sh` is the alignment gate. It
runs the Core fixture drift check, CLI production projections, CLI real PTY
successor input, Web production reducer, Web terminal/composer contract, the
macOS production reducer process Harness, and macOS terminal/visible-run tests.

### P0: terminal state previously had multiple tempting authorities

Provider stop, stream EOF, Trace completion, Core terminal, and committed
projection can arrive close together. Reconstructing final text or running
state from more than one of them causes duplicate answers, overwritten old
messages, delayed composer restoration, or final-layout jumps.

Required rule: Core terminal owns execution state; the committed
`ConversationProjection` owns durable content. EOF is cleanup only. Trace is
diagnostic only. A terminal event never carries a second assistant body.

### P1: Web interaction coverage is weaker than its reducer coverage

The production JS reducer is exercised in a real browser, but some composer
release checks still assert source structure in Rust. This catches accidental
deletion but not focus loss, disabled controls, duplicated pending bubbles, or
visual reflow.

Required follow-up: keep the fast structural test, and make the credential-free
Web product Harness drive submit, permission wait, incremental Markdown,
terminal projection, second input, and cancellation through the rendered page.

### P1: macOS interaction evidence is fragmented

The production Swift reducer consumes the shared fixture, while pending bubble,
visible-run, disclosure, reveal, scroll anchoring, and next-input behavior are
covered by separate XCTest cases. Fragmentation can miss an ordering failure at
the handoff between those components.

Required follow-up: one native product-path scenario must capture submitted,
reasoning, permission/tool, streaming, terminal, and post-terminal input with
stable message IDs and screenshots from the same conversation.

### P1: Harness output did not identify the same correlations

CLI PTY output, Web browser state, and macOS XCTest failures do not consistently
report conversation, Turn, message, Part cursor, projection revision, effect,
and request identity together. A failure can therefore look like a UI issue
when it is an outbox or recovery issue.

Required rule: failure artifacts report these identifiers together without
logging private prompt or full tool-result content.

### P2: native presentation is allowed to differ, semantics are not

CLI should retain compact scrollback and explicit keyboard behavior. Web may
use DOM disclosures and responsive panels. macOS may use native animation,
sidebar inspection, drag/drop, and accessibility. These differences must not
create separate tool histories, permission decisions, context usage, or final
message assembly.

## Harness matrix

| Scenario | Core | CLI PTY | Web browser | macOS native |
| --- | ---: | ---: | ---: | ---: |
| Shared Part fixture | Required | Required | Required | Required |
| Immediate pending shell | N/A | Required | Required | Required |
| Reasoning identity | Required | Required | Required | Required |
| Permission wait/resume | Required | Required | Required | Required |
| Return-to-input cancellation | Required | Required | Required | Required |
| Incremental Markdown | Required | Required | Required | Required |
| Terminal projection handoff | Required | Required | Required | Required |
| Second input after terminal | Required | Required | Required | Required |
| Reopen canonical transcript | Required | Required | Required | Required |
| Visual flicker/reflow observation | N/A | Terminal recording | Screenshot/recording | Screenshot/recording |

Mock-model tests prove protocol determinism, not semantic routing quality. Real
model E2E remains mandatory for tool selection, multilingual natural
follow-ups, unknown-tool recovery, and permission policy. Clients must submit
the original query and may not repair routing with keyword lists.

## Release gate

A change to Core events, Host envelopes, projection, permissions, streaming,
goals, or composer state is incomplete until:

1. `scripts/test_cli_web_mac_alignment.sh` passes.
2. The affected platform's broader contract suite passes.
3. A real multi-turn product session proves original-input routing and a second
   input after terminal.
4. Web/macOS screenshots and recordings are actually inspected for pending
   shell timing, flicker, reflow, scroll anchoring, and terminal handoff.
5. Reopening the conversation returns the same ordered canonical transcript
   and context usage.

The unified gate is intentionally narrow enough for routine development. Full
real-provider and visual suites remain release/nightly gates rather than being
silently replaced by unit tests.

## Applied fixes and observed evidence

This audit changed the Harness and product path rather than only documenting
the gap:

1. Added `scripts/test_cli_web_mac_alignment.sh` as the shared fast gate.
2. Restored the macOS process Harness to the production source graph by
   compiling `MacCanonicalMessageIdentity` and translating the Core fixture
   through the canonical Host wire envelope before production decoding.
3. Replaced duplicated Web Playwright launch assumptions with
   `web/tests/playwright_browser.py`. It accepts an explicit browser, falls
   back to installed Chrome/Chromium, and uses deterministic headless GPU
   settings. Global pytest plugin autoload is disabled by the alignment gate;
   the reducer test fell from roughly 153 seconds to about two seconds.
4. Added Web browser assertions for pending-to-Markdown handoff, terminal
   geometry, and open fenced-code streaming.
5. Fixed Web cold restore anchoring. Canonical Parts and
   `content-visibility:auto` can expand an offscreen message after the first
   `scrollTop` assignment, so restore now re-anchors frame by frame with
   immediate scrolling until both height and bottom distance stabilize.
6. Added a real Web Host E2E assertion that the final restored assistant is
   actually inside the message viewport. The test covers two Turns, destructive
   permission, tool execution, incremental Markdown, process-backed storage,
   Trace journal restore, and cold reload.
7. Made the macOS production workspace visual test retain its PNG in xcresult,
   so human visual review is possible instead of relying only on pixel ratios.

Observed evidence on 2026-09-02:

- The alignment gate passed end to end.
- The real Web Host E2E passed in 14.62 seconds after the restore fix. Its
  screenshot showed the second terminal assistant, tool activity, and final
  text fully above the composer.
- The macOS visual test passed, exported one retained production-workspace PNG,
  and the opened image showed one ordered response, stable header hierarchy,
  canonical context usage, and an opaque composer without transcript overlap.
# Deep ownership corrections

- Product identity and execution policy are composed once by Core. Web no longer injects a second `system` context component.
- Tool availability is independent from authorization. Web keeps the Host catalog stable and submits normalized permission rules to Core for concrete-call evaluation.
- macOS local and remote execution now preserve the same typed context kinds (`memory`, `prompt_template`, `attachment`, `platform_data`, and others) instead of flattening remote context into `system` text.
- The completed-Goal PTY acceptance path must persist both the next user message and its assistant response; merely repainting a composer placeholder is not sufficient evidence.
