# MOD-02 · Core and the agent state machine

[简体中文](README.md) · English · [Architecture](../../README.en.md)

The agent engine owns the reducer, checkpoint and trace cursor. Kernel supplies deterministic decision logic. The Core facade reuses these types. The reducer consumes explicit time, frozen configuration and typed input, then returns state, domain events and proposed effects.

## Runtime behavior

A submission produces a candidate transition from committed state. Application adopts it after the transition and its next effects commit. Recovery uses the checkpoint, execution generation and committed events. A busy indicator has no authority to replay work.

## Failure and verification

Test an interrupted commit and a late observation against the same checkpoint. A rejected transition must leave the previous state usable. A terminal tool result alone cannot decide a Goal outcome.

## Implementation

- [nola-agent-engine/src/reducer_port.rs](../../../../crates/nola-engine/src/lib.rs)
- [agent/runtime/submission.rs](../../../../crates/nola-engine/src/loop_runner.rs)
- [core/src/lib.rs](../../../../crates/nola-app/src/lib.rs)

## Detailed references

[Completion](COMPLETION.md) · [Lifecycle](LIFECYCLE.md) · [Requirement Admission](REQUIREMENT_ADMISSION.md)
