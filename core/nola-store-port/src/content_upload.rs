// SPDX-License-Identifier: Apache-2.0
//! Owner-scoped upload staging; sealing is not canonical publication.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadOwner {
    pub conversation_id: String,
    pub import_id: String,
    pub part_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadProgress {
    pub expected_bytes: u64,
    pub committed_bytes: u64,
    pub sealed: bool,
    pub cancelled: bool,
}

/// Mutations commit atomically before returning. Implementations enforce owner
/// identity, quotas, contiguous offsets and identical chunk replay. Chunks are
/// at most 32768 UTF-8 bytes. Progress and reads never create uploads.
/// Cancellation retains an identity tombstone; sealing requires complete data.
pub trait ContentUploadRepository {
    type Error: std::fmt::Display;

    fn begin(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        expected: u64,
    ) -> Result<UploadProgress, Self::Error>;
    fn progress(&self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error>;
    fn append(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
        text: &str,
    ) -> Result<UploadProgress, Self::Error>;
    fn seal(&mut self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error>;
    fn cancel(&mut self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error>;
    fn read_chunk(
        &self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
    ) -> Result<Option<String>, Self::Error>;
}
