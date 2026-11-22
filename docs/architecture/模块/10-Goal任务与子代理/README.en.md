# MOD-10 · Goals, tasks and experts

[简体中文](README.md) · English · [Architecture](../../README.en.md)

An expert template supplies a role prompt, useful tools, Skills and model settings. Dispatch binds the member revision and parent context. The worker starts after durable Task admission. The parent consumes the terminal receipt. Room and participant records remain part of the implementation.

## Runtime behavior

The user addresses the main agent. An inline @ mention identifies an expert and the main agent can submit a Task when needed. Experts and temporary subagents share execution, cancellation, recovery and result delivery. Each expert keeps its child conversation under the same main conversation. The main agent handles clarification.

## Failure and verification

Test two turns with the same expert, edit and resend across an expert turn, cancellation and reconnect. Core rejects recursive dispatch from an expert. A timed-out wait leaves the Task available for later inspection.

## Implementation

- [nola-application/src/goal_execution.rs](../../../../core/nola-application/src/goal_execution.rs)
- [src/agent_room/submission.rs](../../../../core/nola-application/src/agent_room/submission.rs)
- [nola-mobile-bindings/src/child_pump.rs](../../../../core/nola-mobile-bindings/src/child_pump.rs)
- [nola-application/src/main_expert_selection.rs](../../../../core/nola-application/src/main_expert_selection.rs)
- [nola-application/src/expert_peer_task.rs](../../../../core/nola-application/src/expert_peer_task.rs)

## Detailed references

[Agent Thread Architecture](AGENT_THREAD_ARCHITECTURE.md) · [Delegated Agent Host Contract](DELEGATED_AGENT_HOST_CONTRACT.md) · [Expert Subagent Execution](EXPERT_SUBAGENT_EXECUTION.en.md) · [Goals And Tasks](GOALS_AND_TASKS.md) · [Goal Long Horizon Architecture](GOAL_LONG_HORIZON_ARCHITECTURE.md) · [Room Dispatch](ROOM_DISPATCH.md)
