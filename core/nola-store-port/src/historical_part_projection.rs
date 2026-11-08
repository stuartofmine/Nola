// SPDX-License-Identifier: Apache-2.0
//! Shared canonical JSON interpretation for presentation and context.
pub use nola_domain::{CanonicalPartOwner, HistoricalPartProjection, HistoricalPartRead};

pub trait HistoricalPartProjectionRepository: crate::tool_source::ToolSourceRepository {
    type Error: std::fmt::Display;
    fn read_historical_part_with_budget(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> Result<HistoricalPartRead, Self::Error>;
    /// None means canonical JSON is absent. A present unrecognized document
    /// returns explicit Unsupported. One snapshot pins all evidence and bytes.
    fn read_historical_part(
        &self,
        owner: &CanonicalPartOwner,
        expected_revision: i64,
        maximum_output_bytes: usize,
    ) -> Result<Option<HistoricalPartProjection>, Self::Error>;
}
