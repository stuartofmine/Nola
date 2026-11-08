// SPDX-License-Identifier: Apache-2.0
//! Source-issued plans, not client-writable product settings.
use nola_domain::host_tool_plan::{HostToolPlanReference, HostToolPlanSnapshot};

pub trait PreparedHostToolPlanRepository {
    type Error: std::fmt::Display;
    fn persist_prepared_host_tool_plan(&mut self, snapshot: &HostToolPlanSnapshot, now_ms: u64)
        -> Result<(), Self::Error>;
    fn prepared_host_tool_plan(&self, reference: &HostToolPlanReference, now_ms: u64)
        -> Result<Option<HostToolPlanSnapshot>, Self::Error>;
}
