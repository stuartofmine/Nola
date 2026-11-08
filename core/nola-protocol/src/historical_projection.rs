// SPDX-License-Identifier: Apache-2.0
//! Revision-pinned, on-demand interpretation of one canonical historical Part.
use nola_domain::CanonicalPartOwner;
pub use nola_domain::{HistoricalNotLoadedReason, HistoricalPartProjection};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoricalProjectionCommand {
    ReadHistoricalPart {
        owner: CanonicalPartOwner,
        expected_conversation_revision: i64,
        maximum_output_bytes: usize,
    },
}
impl crate::CommandMetadata for HistoricalProjectionCommand {
    fn wire_command_name(&self) -> &'static str {
        "read_historical_part"
    }
}
