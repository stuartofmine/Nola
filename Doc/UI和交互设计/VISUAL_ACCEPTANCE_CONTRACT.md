> **发布兼容约束：重新发布的 Nola 1.0.0 是首个受支持基线，Nola 自有协议、ABI 与数据库格式/epoch 从 1 开始；此前开发格式不保留兼容或转换代码。从本次发布起，后续版本须维护已发布接口与数据兼容，破坏性变更必须有明确版本协商、升级说明及回归测试；数据库迁移必须可验证、可回滚并保护原始数据。未知或不支持的格式明确拒绝，不得自动删除、重建或静默改写用户数据。外部服务标准协议沿用其自身版本。**

# Visual Acceptance Contract

Visual acceptance is a product contract, not a screenshot-generation side
effect. A build, event replay, or generated PNG does not prove that an
interface is visually correct.

The [current Nola prototype](../../docs/design/nola-ui-prototype/README.md) is
the accepted visual direction and has its own [prototype validation record](../../docs/design/nola-ui-prototype/docs/VALIDATION.md).
Those local-demo checks do not satisfy this production application-window
contract. For implementation status, compare the prototype's feature matrix
with the actual source and retain the lifecycle evidence below.

## Required evidence

Every interactive conversation acceptance run must preserve four synchronized
artifacts:

1. The ordered Host event log, retained for transport diagnostics only.
2. The versioned canonical conversation projection used by the UI.
3. Screenshots or recordings of the real application window at defined
   lifecycle checkpoints.
4. A visual observation receipt produced after a human or image-capable
   reviewer actually inspects the artifacts.

The event log must never be replayed into a client-owned product state machine.
The screenshot must render the same canonical projection revision recorded by
the acceptance result.

## Lifecycle checkpoints

Conversation UI coverage requires observation at these checkpoints:

- The user message has committed and the empty assistant shell is visible.
- Reasoning or the first long-running activity is visible.
- A tool call is active with its meaningful operation and parameters visible.
- Markdown is partially streamed, including an open fenced code block.
- Output approaches the composer and smooth follow-scroll is active.
- The run reaches its terminal projection.
- Reasoning is expanded and collapsed.
- The conversation is switched away from and restored.
- The application is relaunched and the same Store projection is restored.

A component-only snapshot may test typography and layout, but it cannot satisfy
the window-level gate because it does not expose scrolling, overlays, sidebar
state, z-order, or transition flicker.

## Observation receipt

The reviewer writes a JSON receipt beside the captured artifact:

```json
{
  "schema_version": 1,
  "artifact_path": "/absolute/path/to/window-sequence",
  "artifact_kind": "application_window_sequence",
  "observer": "reviewer-or-visual-agent-id",
  "observation_method": "image_capable_reviewer",
  "verdict": "pass",
  "observations": [
    {"checkpoint": "assistant_shell", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/01-shell.png", "evidence_kind": "screenshot", "notes": "Opened frame: shell appeared immediately."},
    {"checkpoint": "active_reasoning", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/02-reasoning.png", "evidence_kind": "screenshot", "notes": "Opened frame: stable title and readable detail."},
    {"checkpoint": "active_tool", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/03-tool.png", "evidence_kind": "screenshot", "notes": "Opened frame: operation and parameters were visible."},
    {"checkpoint": "partial_markdown", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/04-stream.mov", "evidence_kind": "recording", "notes": "Watched recording: open fence rendered without replacing prior nodes."},
    {"checkpoint": "composer_follow", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/04-stream.mov", "evidence_kind": "recording", "notes": "Watched recording: smooth follow with correct z-order."},
    {"checkpoint": "terminal", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/05-terminal.png", "evidence_kind": "screenshot", "notes": "Opened frame: no duplicate, omission, or layout jump."},
    {"checkpoint": "reasoning_toggle", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/06-toggle.mov", "evidence_kind": "recording", "notes": "Watched recording: no flicker or delayed body movement."},
    {"checkpoint": "conversation_restore", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/07-restore.png", "evidence_kind": "screenshot", "notes": "Opened frame: projection and context usage restored."},
    {"checkpoint": "application_relaunch", "verdict": "pass", "evidence_path": "/absolute/path/to/window-sequence/08-relaunch.png", "evidence_kind": "screenshot", "notes": "Opened frame: same Store revision restored."}
  ],
  "motion_reviewed": true,
  "notes": "No duplicate text, layout jump, flicker, clipping, or stale state."
}
```

The artifact path must match exactly after path normalization. A missing,
malformed, mismatched, or failed receipt means the visual gate is not passed.
Capture metadata must state whether an artifact is a component snapshot, a
window screenshot, or a recording.

Every lifecycle checkpoint above must have an explicit passing observation.
`motion_reviewed` must be true; otherwise flicker, delayed reflow, unstable
follow-scroll, and z-order transitions remain unobserved. A single overall
verdict without per-checkpoint observations is invalid.

Every observation must reference an existing screenshot, recording frame, or
recording inside the application-window artifact directory. The reviewer must
open each screenshot and watch each referenced recording before writing the
notes. File generation, pixel dimensions, snapshot hashes, and process exit
status are capture evidence only; none proves that a person or image-capable
reviewer observed the UI.

Capture and observation are separate roles even when one agent performs both.
The capture process may record paths and lifecycle metadata, but it cannot
derive a passing verdict from exit status, image existence, OCR, pixel hashes,
or snapshot-diff thresholds. The observation step must load the rendered image
content (and play the complete motion interval), then record a concrete visual
finding. A receipt produced without that content-observation step is invalid.

Each checkpoint also records the correlated `submission_id`, `turn_id`,
`message_id`, `part_id` set, and projection revision. This correlation is not a
substitute for looking at the pixels; it proves whether a visible omission,
duplicate, or replacement originated before or after presentation.

The review is semantic as well as visual. At each screenshot checkpoint the
reviewer compares the visible user/assistant ordering, reasoning rows, tool
parameters, Markdown structure, attachment identities, and context usage with
the correlated canonical projection revision. A screenshot that looks
plausible but omits or duplicates projection content fails. A correct static
frame does not excuse flicker, replacement, delayed reflow, or scroll jumps in
the recording.

When a product-level visual gate is requested, its required scope is
`application_window_sequence`. A receipt for an `assistant_component_snapshot`
cannot satisfy that gate even if the component itself looks correct.

## Failure classification

Visual failures must be correlated with the same run's projection and event
artifacts:

- Correct Store projection plus wrong pixels is a client projection or render
  defect.
- Incorrect Store projection is an Application/Core materialization defect.
- Correct terminal projection plus missing or duplicated streamed content is a
  Host transport or incremental-render contract defect.
- Correct static frames plus visible motion defects require recording-based
  diagnosis rather than additional snapshots.

Acceptance reports must name the failing boundary. They must not repair the
symptom by merging text, replaying traces, collapsing messages, or inferring
missing state in the client.
