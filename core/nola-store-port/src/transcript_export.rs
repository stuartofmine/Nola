// SPDX-License-Identifier: Apache-2.0
//! Export reads never hydrate a whole Part or substitute a display preview.
pub use nola_domain::transcript_export::*;

pub trait TranscriptExportRepository: crate::tool_source::ToolSourceRepository {
    type Error: std::fmt::Display;
    fn read_part_derivation(
        &self,
        owner: &CanonicalPartOwner,
        expected_conversation_revision: i64,
    ) -> Result<Option<StoredPartDerivation>, Self::Error>;
    fn load_export_page(
        &self,
        conversation_id: &str,
        after_ordinal: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<Option<ExportPage>, Self::Error>;
    fn read_export_range(
        &self,
        conversation_id: &str,
        message_id: &str,
        part_id: &str,
        field: ExportBodyField,
        expected_revision: i64,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> Result<ExportRange, Self::Error>;
}
