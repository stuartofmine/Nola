// SPDX-License-Identifier: Apache-2.0
//! Internal immutable Part-field preparation. References are not client authority.
use nola_domain::CanonicalBodyReference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedBodyRange {
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

pub trait CanonicalBodyRepository {
    type Error: std::fmt::Display;

    /// Caller authorizes the canonical owner. Atomically prepare immutable raw
    /// bytes; JSON fields additionally pass the original Store JSON grammar.
    /// This operation never publishes a Message/Part or emits replica facts.
    fn prepare_body(
        &mut self,
        reference: &CanonicalBodyReference,
        chunks: &mut dyn Iterator<Item = Result<(u64, String), String>>,
        maximum_bytes: u64,
    ) -> Result<(), Self::Error>;

    /// Internal prepared-object reader, distinct from visible transcript reads.
    /// The full field-aware descriptor must equal the persisted descriptor.
    fn read_prepared_body_range(
        &self,
        reference: &CanonicalBodyReference,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> Result<PreparedBodyRange, Self::Error>;
}
