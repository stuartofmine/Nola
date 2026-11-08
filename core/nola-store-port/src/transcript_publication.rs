// SPDX-License-Identifier: Apache-2.0
//! Finalized transcript publication. Preparation never makes a Part visible.
use crate::transcript_write::{AppendMessagesInput, AppendMessagesResult};
use nola_domain::CanonicalBodyReference;

#[derive(Debug, Clone)]
pub struct PreparedTranscriptPublication {
    pub import_id: String,
    /// Exact bounded manifest encoding binds replay, including source uploads.
    pub manifest_json: String,
    pub expected_conversation_revision: i64,
    pub append: AppendMessagesInput,
    pub references: Vec<CanonicalBodyReference>,
}

pub trait TranscriptPublicationRepository {
    type Error: std::fmt::Display;
    fn transcript_publication_replay(
        &self,
        import_id: &str,
        manifest_json: &str,
    ) -> Result<Option<AppendMessagesResult>, Self::Error>;
    fn preflight_transcript_publication(
        &self,
        conversation_id: &str,
        expected_revision: i64,
        expected_count: usize,
    ) -> Result<(), Self::Error>;
    /// Streams an immutable object; failure leaves no partial object. This may
    /// commit before publication, but may not create a visible transcript Part.
    fn prepare_transcript_content(
        &mut self,
        reference: &CanonicalBodyReference,
        chunks: &mut dyn Iterator<Item = Result<(u64, String), String>>,
        maximum_bytes: u64,
    ) -> Result<(), Self::Error>;
    /// Requires the command's outer write unit, so entity/reference/revision,
    /// sync facts, import identity and command receipt share its final commit.
    fn publish_transcript(
        &mut self,
        input: &PreparedTranscriptPublication,
    ) -> Result<AppendMessagesResult, Self::Error>;
}
