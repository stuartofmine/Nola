// SPDX-License-Identifier: Apache-2.0
//! Archive staging is durable, while replacement has one canonical write unit.
pub use nola_domain::archive_restore::*;
use nola_domain::CanonicalBodyReference;
pub trait ArchiveRestoreRepository {
    type Error: std::fmt::Display;
    fn archive_directory_revision(&self) -> Result<u64, Self::Error>;
    fn begin_archive_restore(
        &mut self,
        input: &BeginArchiveRestore,
    ) -> Result<ArchiveRestoreProgress, Self::Error>;
    fn archive_restore_progress(
        &self,
        restore_id: &str,
    ) -> Result<ArchiveRestoreProgress, Self::Error>;
    fn archive_restore_session_exists(&self, restore_id: &str) -> Result<bool, Self::Error>;
    fn archive_restore_accepts_upload(
        &self,
        restore_id: &str,
        conversation_id: &str,
    ) -> Result<bool, Self::Error>;
    fn prepared_archive_reference(
        &self,
        restore_id: &str,
        content_id: &str,
    ) -> Result<Option<CanonicalBodyReference>, Self::Error>;
    fn prepare_archive_content(
        &mut self,
        restore_id: &str,
        reference: &CanonicalBodyReference,
        chunks: &mut dyn Iterator<Item = Result<(u64, String), String>>,
        maximum_bytes: u64,
    ) -> Result<(), Self::Error>;
    fn stage_archive_entry(
        &mut self,
        restore_id: &str,
        sequence: u64,
        entry: &ArchiveRestoreEntry,
    ) -> Result<ArchiveRestoreProgress, Self::Error>;
    fn commit_archive_restore(
        &mut self,
        input: &CommitArchiveRestore,
    ) -> Result<ArchiveRestoreResult, Self::Error>;
}

/// Metadata needed to publish receipt-owned projection changes for archive
/// commits. Reads use the caller's existing transaction and never hydrate bodies.
pub trait ArchiveProjectionRepository {
    type Error: std::fmt::Display;
    fn archive_restore_projection_ids<'a>(
        &self,
        changes: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Vec<String>, Self::Error>;
    fn archive_restore_conversation_ids(
        &self,
        restore_id: &str,
    ) -> Result<Vec<String>, Self::Error>;
}
