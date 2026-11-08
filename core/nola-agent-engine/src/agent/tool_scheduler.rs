// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Plans deterministic tool execution batches from Host-declared semantics.
// - Boundary: This module never executes tools or evaluates platform-specific policy.

use super::host::{
    HostCancellationMode, HostCapabilityCatalog, HostCapabilityParallelism, HostCapabilityRoute,
};
use super::protocol::CallId;
use super::tool_lifecycle::{ToolCallState, ToolCallStatus};

/// Prevents a single model step from exhausting Host threads, descriptors, or
/// network sockets. Remaining safe calls are admitted in subsequent batches.
pub use nola_kernel::MAX_CONCURRENT_TOOL_BATCH_SIZE;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolBatchPlan {
    pub call_ids: Vec<CallId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolInterruptBehavior {
    /// The Host contract says the effect can receive cancellation.
    Cancel,
    /// The operation cannot safely be detached or cancelled mid-flight.
    Block,
}

/// Turn-scoped owner of tool admission semantics. Core creates this runtime
/// from the frozen Host catalog; callers no longer combine scheduler and
/// cancellation rules independently.
pub struct ToolExecutionRuntime<'a> {
    tools: &'a [ToolCallState],
    catalog: &'a HostCapabilityCatalog,
}

impl<'a> ToolExecutionRuntime<'a> {
    pub fn new(tools: &'a [ToolCallState], catalog: &'a HostCapabilityCatalog) -> Self {
        Self { tools, catalog }
    }

    pub fn next_batch(&self) -> ToolBatchPlan {
        plan_next_tool_batch(self.tools, self.catalog)
    }

    pub fn interrupt_behavior(&self, call_id: &CallId) -> ToolInterruptBehavior {
        let cancellation = self
            .tools
            .iter()
            .find(|tool| &tool.call_id == call_id)
            .and_then(|tool| self.catalog.capabilities.get(&tool.name))
            .map(|capability| capability.cancellation)
            .unwrap_or(HostCancellationMode::Unsupported);
        match cancellation {
            HostCancellationMode::BestEffort | HostCancellationMode::Cooperative | HostCancellationMode::ProcessBound => {
                ToolInterruptBehavior::Cancel
            }
            HostCancellationMode::Unsupported => ToolInterruptBehavior::Block,
        }
    }

    pub fn blocking_interrupts(&self) -> Vec<CallId> {
        self.tools
            .iter()
            .filter(|tool| tool.status == ToolCallStatus::Running)
            .filter(|tool| self.interrupt_behavior(&tool.call_id) == ToolInterruptBehavior::Block)
            .map(|tool| tool.call_id.clone())
            .collect()
    }
}

impl ToolBatchPlan {
    pub fn is_empty(&self) -> bool {
        self.call_ids.is_empty()
    }
}

/// Returns the next executable batch while preserving model-emitted order.
///
/// A batch is either one serial/unknown capability or a consecutive run of
/// tools that the active Host explicitly declares concurrency-safe. A running
/// tool or unresolved permission is a barrier.
pub fn plan_next_tool_batch(
    tools: &[ToolCallState],
    catalog: &HostCapabilityCatalog,
) -> ToolBatchPlan {
    use nola_domain::{ToolBatchCandidate, ToolBatchCandidateState, ToolBatchCapabilityFacts};
    let candidates = tools
        .iter()
        .map(|tool| ToolBatchCandidate {
            call_id: tool.call_id.0.clone(),
            state: match tool.status {
                ToolCallStatus::Pending => ToolBatchCandidateState::Pending,
                ToolCallStatus::Running => ToolBatchCandidateState::Running,
                ToolCallStatus::AwaitingPermission => ToolBatchCandidateState::AwaitingPermission,
                ToolCallStatus::Ready
                | ToolCallStatus::AwaitingUserInput
                | ToolCallStatus::Completed
                | ToolCallStatus::Failed
                | ToolCallStatus::Denied
                | ToolCallStatus::Cancelled => ToolBatchCandidateState::Inactive,
            },
            capability: catalog
                .capabilities
                .get(&tool.name)
                .map(|capability| ToolBatchCapabilityFacts {
                    available: capability.availability
                        == super::host::CapabilityAvailability::Available,
                    is_tool: capability.route == HostCapabilityRoute::Tool,
                    concurrent_safe: capability.parallelism
                        == HostCapabilityParallelism::ConcurrentSafe,
                })
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    ToolBatchPlan {
        call_ids: nola_kernel::plan_tool_batch(&candidates)
            .call_ids
            .into_iter()
            .map(CallId::from_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::host::*;
    use crate::agent::protocol::ModelToolCall;
    use serde_json::json;

    fn descriptor() -> HostCapabilityDescriptor {
        HostCapabilityDescriptor {
            id: "tool".into(),
            title: "Tool".into(),
            route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Visible,
            permission_domain: None,
            input_schema: json!({"type":"object"}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 100,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: false,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::ConcurrentSafe,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        }
    }
    fn calls() -> Vec<ToolCallState> {
        (0..5)
            .map(|id| {
                ToolCallState::from_model(ModelToolCall {
                    call_id: CallId::from_string(id.to_string()),
                    name: "tool".into(),
                    arguments: json!({}),
                    permission_scope: None,
                    permission_rationale: None,
                })
            })
            .collect()
    }
    #[test]
    fn adapter_maps_host_facts_and_all_lifecycle_states_without_planning_locally() {
        let mut catalog = HostCapabilityCatalog::empty("host");
        catalog.capabilities.insert("tool".into(), descriptor());
        let mut tools = calls();
        assert_eq!(
            plan_next_tool_batch(&tools, &catalog).call_ids,
            tools[..4]
                .iter()
                .map(|t| t.call_id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(MAX_CONCURRENT_TOOL_BATCH_SIZE, 4);
        for status in [ToolCallStatus::Running, ToolCallStatus::AwaitingPermission] {
            tools[4].status = status;
            assert!(
                plan_next_tool_batch(&tools, &catalog).is_empty(),
                "barrier beyond batch must block"
            );
        }
        for status in [
            ToolCallStatus::Ready,
            ToolCallStatus::Completed,
            ToolCallStatus::Failed,
            ToolCallStatus::Denied,
            ToolCallStatus::Cancelled,
        ] {
            tools = calls();
            for tool in &mut tools[..4] {
                tool.status = status;
            }
            assert_eq!(
                plan_next_tool_batch(&tools, &catalog).call_ids,
                vec![tools[4].call_id.clone()]
            );
        }
    }
    #[test]
    fn unknown_unavailable_non_tool_and_serial_are_singletons() {
        let tools = calls();
        let mut catalog = HostCapabilityCatalog::empty("host");
        assert_eq!(
            plan_next_tool_batch(&tools, &catalog).call_ids,
            vec![tools[0].call_id.clone()]
        );
        for availability in [
            CapabilityAvailability::Unavailable,
            CapabilityAvailability::RequiresSetup,
            CapabilityAvailability::RequiresConnection,
            CapabilityAvailability::Disabled,
        ] {
            let mut capability = descriptor();
            capability.availability = availability;
            catalog.capabilities.insert("tool".into(), capability);
            assert_eq!(
                plan_next_tool_batch(&tools, &catalog).call_ids,
                vec![tools[0].call_id.clone()]
            );
        }
        let mut capability = descriptor();
        capability.route = HostCapabilityRoute::Model;
        catalog.capabilities.insert("tool".into(), capability);
        assert_eq!(
            plan_next_tool_batch(&tools, &catalog).call_ids,
            vec![tools[0].call_id.clone()]
        );
        let mut capability = descriptor();
        capability.parallelism = HostCapabilityParallelism::Serial;
        catalog.capabilities.insert("tool".into(), capability);
        assert_eq!(
            plan_next_tool_batch(&tools, &catalog).call_ids,
            vec![tools[0].call_id.clone()]
        );
    }

    #[test]
    fn runtime_derives_interrupt_behavior_from_the_frozen_descriptor() {
        let tools = calls();
        let mut catalog = HostCapabilityCatalog::empty("host");
        let mut capability = descriptor();
        capability.cancellation = HostCancellationMode::Unsupported;
        catalog.capabilities.insert("tool".into(), capability);
        let runtime = ToolExecutionRuntime::new(&tools, &catalog);
        assert_eq!(
            runtime.interrupt_behavior(&tools[0].call_id),
            ToolInterruptBehavior::Block
        );

        catalog.capabilities.get_mut("tool").unwrap().cancellation =
            HostCancellationMode::Cooperative;
        let runtime = ToolExecutionRuntime::new(&tools, &catalog);
        assert_eq!(
            runtime.interrupt_behavior(&tools[0].call_id),
            ToolInterruptBehavior::Cancel
        );
    }

    #[test]
    fn unsupported_running_tool_blocks_turn_interruption() {
        let mut tools = calls();
        tools[0].status = ToolCallStatus::Running;
        let mut catalog = HostCapabilityCatalog::empty("host");
        let mut capability = descriptor();
        capability.cancellation = HostCancellationMode::Unsupported;
        catalog.capabilities.insert("tool".into(), capability);
        assert_eq!(
            ToolExecutionRuntime::new(&tools, &catalog).blocking_interrupts(),
            vec![tools[0].call_id.clone()]
        );
    }
}
