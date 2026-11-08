// SPDX-License-Identifier: Apache-2.0
//! Source journal persistence, not a remote input or presentation authority.
use nola_domain::assistant_journal::*;
use nola_domain::assistant_journal_operations::*;

#[derive(Clone, Debug)]
pub enum AssistantSourceCommitStamp {
    /// A committed Surface revision, with no Agent checkpoint mutation.
    UiSurface { surface: nola_domain::generative_ui::UiSurfaceRevisionRef },
    Admission { run_id: String },
    /// Exact child admission may update only its parent Task guidance metadata.
    TaskRunBinding { task_id: String, child_run_id: String },
    Transition { transition_id: String },
    /// A real idle checkpoint write, not a Run admission or execution grant.
    IdleCheckpoint { conversation_id: String, checkpoint_revision: i64, event_cursor: i64 },
}
#[derive(Clone, Debug)]
pub struct CommitAssistantJournal {
    pub stamp: AssistantSourceCommitStamp,
    pub expected: AssistantJournalCursor,
    pub batch: AssistantOperationBatchV1,
    /// Verified supplied bytes authorize their introduction; digest existence does not.
    pub supplied_chunks: Vec<AssistantTextChunkV1>,
}
#[derive(Clone, Debug)]
pub struct AssistantJournalFrame {
    pub batch: AssistantOperationBatchV1,
    pub supplied_chunks: Vec<AssistantTextChunkV1>,
}
#[derive(Clone, Debug)]
pub struct AssistantPublicationGroup {
    /// Initial cursor captured while planning. Later frames extend this cursor.
    pub expected: AssistantJournalCursor,
    pub frames: Vec<AssistantJournalFrame>,
}
#[derive(Clone, Debug)]
pub struct CommitAssistantPublicationSet {
    pub stamp: AssistantSourceCommitStamp,
    /// Immutable ordered complete set, not independently publishable prefixes.
    /// Every group has a distinct, independently proven owner in the source conversation.
    pub groups: Vec<AssistantPublicationGroup>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantPublicationCommit {
    Applied(Vec<AssistantJournalCursor>),
    Replay(Vec<AssistantJournalCursor>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantJournalCommit {
    Applied(AssistantJournalCursor),
    Replay(AssistantJournalCursor),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredAssistantJournalRecord {
    pub header: AssistantJournalRecordHeaderV1,
    pub batch: AssistantOperationBatchV1,
}
#[derive(Clone, Debug)]
pub struct StageAssistantJournalContent {
    pub source_host_id: String,
    pub record: StoredAssistantJournalRecord,
    pub part_id: String,
    pub content: JournalContentRef,
    pub expected_prefix: JournalContentRef,
    pub chunks: Vec<AssistantTextChunkV1>,
    /// Discard only this exact record/Part/content preparation and begin again.
    pub restart: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalContentNeed {
    pub part_id: String,
    pub content: JournalContentRef,
    pub expected_prefix: JournalContentRef,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalReceptionState {
    pub after: AssistantJournalCursor,
    pub complete: bool,
}
/// Keyed committed state. Retirement is not absence or permission to reuse an
/// ID. An explicit repeated removal may no-op, while creation must reject it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantJournalPartState {
    NeverDeclared,
    Live(JournalPartHead),
    Retired,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantJournalReadWindow {
    pub after: AssistantJournalCursor,
    pub through: AssistantJournalCursor,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantJournalPage {
    Records {
        records: Vec<StoredAssistantJournalRecord>,
        next: AssistantJournalCursor,
        complete: bool,
    },
    SnapshotRequired,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantJournalSnapshot {
    pub snapshot_id: String,
    pub at: AssistantJournalCursor,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantSnapshotRelease {
    Released,
    AlreadyReleased,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantPartPage {
    pub parts: Vec<JournalPartHead>,
    /// Continue whenever present, even if `parts` is empty: inspected directory
    /// entries may be tombstones or may have been created after this snapshot.
    pub next_part_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantPartChange {
    pub part_id: String,
    /// None is an explicit retirement, never an omitted update.
    pub head: Option<JournalPartHead>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantPartChangePage {
    pub changes: Vec<AssistantPartChange>,
    pub next_part_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantManifestPage {
    pub chunks: Vec<JournalChunkRef>,
    pub next_chunk: u64,
    pub complete: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantContentRange {
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub total_utf8_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

pub trait AssistantJournalRepository {
    type Error: std::fmt::Display;
    /// Called after journal publication in the canonical terminal UoW. The
    /// transition stamp selects persisted status and ownership, never caller data.
    fn seal_assistant_run_terminal(&mut self, transition_id: &str)
        -> Result<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>, Self::Error>;
    fn begin_receive_assistant_run_terminal(&mut self, source_host_id: &str, seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal) -> Result<nola_domain::assistant_run_terminal::AssistantRunTerminalReception,Self::Error>;
    fn stage_assistant_run_terminal_manifest(&mut self, source_host_id: &str, page: &nola_domain::assistant_run_terminal::AssistantRunTerminalManifestPage) -> Result<nola_domain::assistant_run_terminal::AssistantRunTerminalReception,Self::Error>;
    fn commit_received_assistant_run_terminal(&mut self, source_host_id: &str, seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal) -> Result<nola_domain::assistant_run_terminal::AssistantRunTerminalReception,Self::Error>;
    fn local_assistant_run_terminal_seal(&self, conversation_id: &str, run_id: &str)
        -> Result<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>, Self::Error>;
    fn read_local_assistant_run_terminal_manifest(&self,
        seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
        after_entry: JournalNumber, max_records: usize, max_encoded_bytes: usize,
    ) -> Result<nola_domain::assistant_run_terminal::AssistantRunTerminalManifestPage, Self::Error>;
    /// Exact current committed head for an authenticated source announcement.
    /// An already-covered watermark still requires historical digest proof.
    fn assistant_journal_reception_state(
        &self, source_host_id: &str, through: &AssistantJournalCursor,
    ) -> Result<AssistantJournalReceptionState, Self::Error>;
    /// Read only; return the next incomplete content in operation order. None
    /// means receive may be attempted, NOT that the record has been committed.
    fn next_assistant_journal_content(
        &self, source_host_id: &str, record: &StoredAssistantJournalRecord,
    ) -> Result<Option<AssistantJournalContentNeed>, Self::Error>;
    fn stage_assistant_journal_content(
        &mut self, input: &StageAssistantJournalContent,
    ) -> Result<JournalContentRef, Self::Error>;
    /// Host identity comes from the authenticated adapter, not the record body.
    /// Requires the exact existing remote Run binding and an outer write UoW.
    fn receive_assistant_journal_record(
        &mut self,
        source_host_id: &str,
        record: &StoredAssistantJournalRecord,
        chunks: &[AssistantTextChunkV1],
    ) -> Result<AssistantJournalCommit, Self::Error>;
    fn assistant_source_epoch(&self) -> Result<String, Self::Error>;
    /// Discover committed local owners after reconnect without loading a
    /// checkpoint or transcript. This is an observation, not a terminal seal.
    fn local_run_assistant_journal_heads(
        &self, conversation_id: &str, run_id: &str, after_message_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AssistantJournalCursor>, Self::Error>;
    /// A source transport must not relabel an imported remote journal as local.
    fn local_assistant_journal_head(
        &self, identity: &AssistantJournalIdentity,
    ) -> Result<AssistantJournalCursor, Self::Error>;
    fn assistant_journal_head(
        &self,
        identity: &AssistantJournalIdentity,
    ) -> Result<AssistantJournalCursor, Self::Error>;
    /// None means never declared. A retired ID is an error, not an empty slot.
    fn assistant_journal_part(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
    ) -> Result<Option<JournalPartHead>, Self::Error>;
    fn assistant_journal_part_state(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
    ) -> Result<AssistantJournalPartState, Self::Error>;
    /// Bounded immutable manifest facts for the exact currently published head
    /// reference. This is a read, not a snapshot handle or permission to publish.
    /// The caller pins its planning UoW; an obsolete/prepared/unreachable ref is
    /// rejected. No chunk body is read or returned.
    fn assistant_journal_content_manifest(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
        content: &JournalContentRef,
        after_chunk: u64,
        max_records: usize,
    ) -> Result<AssistantManifestPage, Self::Error>;
    /// Complete singleton-set adapter, not an incremental multi-frame writer.
    /// Requires outer write UoW. Never accepts a caller-provided source revision.
    fn commit_assistant_journal(
        &mut self,
        input: &CommitAssistantJournal,
    ) -> Result<AssistantJournalCommit, Self::Error>;
    /// Atomically seals all ordered groups/frames for one actual source commit.
    /// Exact whole-set replay is allowed; changed, missing, or extra frames and
    /// owners conflict even if an individual record would have matched.
    /// Content prepared before this call remains owned by the outer command:
    /// callers must roll that command back after any publication error.
    fn commit_assistant_publication_set(
        &mut self,
        input: &CommitAssistantPublicationSet,
    ) -> Result<AssistantPublicationCommit, Self::Error>;
    /// Bounded preparation for Declare/Replace. No visible head or journal is published.
    fn prepare_assistant_content(
        &mut self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
        previous: &JournalContentRef,
        supplied: &[AssistantTextChunkV1],
    ) -> Result<JournalContentRef, Self::Error>;
    fn open_assistant_journal_read(
        &self,
        after: &AssistantJournalCursor,
    ) -> Result<AssistantJournalReadWindow, Self::Error>;
    /// max_bytes bounds stored header JSON + canonical binary batch bytes, NOT
    /// transport JSON. Each transport must also enforce its actual encoded budget.
    fn read_assistant_journal_page(
        &self,
        window: &AssistantJournalReadWindow,
        max_records: usize,
        max_bytes: usize,
    ) -> Result<AssistantJournalPage, Self::Error>;
    /// Persisted immutable handle; caller owns write UoW, no transaction spans pages.
    fn open_assistant_journal_snapshot(
        &mut self,
        identity: &AssistantJournalIdentity,
    ) -> Result<AssistantJournalSnapshot, Self::Error>;
    /// Pin an exact committed watermark, never substitute the latest head.
    fn open_assistant_journal_snapshot_at(
        &mut self,
        at: &AssistantJournalCursor,
    ) -> Result<AssistantJournalSnapshot, Self::Error>;
    /// Requires outer write UoW and the exact original handle. Logical release
    /// rejects further reads; it neither deletes bodies nor implements global GC.
    fn release_assistant_journal_snapshot(
        &mut self,
        snapshot: &AssistantJournalSnapshot,
    ) -> Result<AssistantSnapshotRelease, Self::Error>;
    /// max_bytes bounds encoded head payloads, excluding transport envelope overhead.
    /// max_records bounds inspected directory entries, not only visible results.
    /// Follow next_part_id until None; an empty result alone is not end-of-snapshot.
    fn read_assistant_snapshot_parts(
        &self,
        snapshot: &AssistantJournalSnapshot,
        after_part_id: Option<&str>,
        max_records: usize,
        max_bytes: usize,
    ) -> Result<AssistantPartPage, Self::Error>;
    fn read_assistant_snapshot_changes(
        &self,
        snapshot: &AssistantJournalSnapshot,
        base: &AssistantJournalCursor,
        after_part_id: Option<&str>,
        max_records: usize,
        max_bytes: usize,
    ) -> Result<AssistantPartChangePage, Self::Error>;
    fn read_assistant_snapshot_manifest(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part_id: &str,
        content: &JournalContentRef,
        after_chunk: u64,
        max_records: usize,
    ) -> Result<AssistantManifestPage, Self::Error>;
    fn read_assistant_snapshot_chunk(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part_id: &str,
        content: &JournalContentRef,
        chunk_index: u64,
    ) -> Result<AssistantTextChunkV1, Self::Error>;
    /// Read a reference introduced by the exact record at snapshot.at, including
    /// intermediate roots not present in the record's final visible Part head.
    /// The active handle, owner, record digest and reference membership are required.
    fn read_assistant_record_chunk(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part_id: &str,
        content: &JournalContentRef,
        chunk_index: u64,
    ) -> Result<AssistantTextChunkV1, Self::Error>;
    /// Original public journal content, not a canonical private field cast.
    /// limit_bytes is raw UTF-8 output bytes (4..=32768), not JSON wire bytes.
    fn read_assistant_snapshot_range(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part_id: &str,
        content: &JournalContentRef,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> Result<AssistantContentRange, Self::Error>;
}
