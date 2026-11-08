// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Runtime task facade separating execution leases from supervision and recovery.

/// Presentation is separate from durable delivery acknowledgement. Deferred
/// viewports never consume a notification or relax its formal owner proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationPresentation { Presented, Deferred }

#[path = "task_runtime_parts/process_control.rs"]
mod process_control;
#[cfg(unix)]
pub(crate) use process_control::bounded_process_output;
pub(crate) use process_control::{ShellChild as OwnedShellChild, stop_borrowed_process_group};

include!("task_runtime_parts/execution.rs");
include!("task_runtime_parts/supervision.rs");
include!("task_runtime_parts/remote_permission.rs");

#[cfg(test)]
#[path = "../../core/nola-application/tests/support/mod.rs"]
mod source_admission_support;

#[cfg(test)]
#[path = "task_permission_wait_tests.rs"]
mod permission_wait_tests;

#[cfg(test)]
#[path = "task_permission_prompt_tests.rs"]
mod permission_prompt_tests;

#[cfg(test)]
#[path = "task_terminal_spawn_replay_tests.rs"]
mod terminal_spawn_replay_tests;
