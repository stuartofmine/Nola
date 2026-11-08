// SPDX-License-Identifier: Apache-2.0
//! Typed product command boundary used by native bindings.

use nola_agent_engine::agent_trace::AgentTraceEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentCommand, ResolveUnknownEffectRecovery, AutomationCommand, BeginAssistantTurn,
    BeginMaintenanceTurn, BuildMemoryWorkerPlan, ClaimCommittedHostRequests, ContextCommand,
    ConversationPlanCommand, CreateConversation, CreateConversationBranch, DataSourceCommand,
    DeleteConversationMessage, DocumentSourceCommand, EditConversationMessage,
    HeartbeatEffectDelivery, ImportConversationTranscript, LoadRuntimeEffectWorkSchedule,
    McpCommand, MemoryCommand, ModelCommand, PrepareRemoteSubmissionInput, RemoteSubmissionIdentity, SourceSubmissionReceipt, PermissionGrantInput,
    PermissionRequestCommand, QueryClaimableMemoryJobs, ReduceAndAcceptHostResponse,
    ReduceAndCommitActiveSubmission, ReduceAndCommitIdleSubmission, RemoteSyncPage,
    RevertConversation, RuntimeTaskCommand, SessionRunLease, SkillCommand, SopRunCommand,
    WorkStepTransactionCommand,
    WorkspaceCheckpoint,
};

/// Current Host-selected model facts; comparison with the frozen template belongs to Core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalSelectedModel {
    pub profile: Option<nola_host_protocol::HostModelProfileReference>,
    pub primary: nola_host_protocol::HostModelRouteBinding,
    pub generation: nola_host_protocol::HostGenerationBinding,
    pub output_admission: nola_host_protocol::HostOutputAdmission,
}

mod agent_runtime;
mod goal_step;
mod capability_catalog;
mod conversation;
pub(crate) use conversation::{read_permission_projection, read_permission_configuration, read_current_permission_projection};
mod operations;

/// Cold discovery and control-bound refresh both require an exact canonical
/// Run. Neither mode may silently substitute the latest Run for its caller.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionProjectionReadScope {
    CurrentRun,
    Control {
        session_id: String,
        #[serde(deserialize_with = "required_permission_turn")]
        active_turn_id: Option<String>,
        event_cursor: u64,
        permission_projection_revision: u64,
    },
}

fn required_permission_turn<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApplicationCommand {
    SubmitUiInteraction { input: crate::generative_ui::SubmitUiInteraction },
    ExecuteUiTool { input: crate::generative_ui::ExecuteUiTool },
    ReadUiSurface { identity: nola_domain::generative_ui::UiSurfaceIdentity },
    PreviewUiSurface { input: nola_domain::generative_ui::UiDraftPreviewInput },
    ReadUiSurfaceChanges { identity: nola_domain::generative_ui::UiSurfaceIdentity, base_revision: Option<u64> },
    ReadUiSurfaceRevision { identity: nola_domain::generative_ui::UiSurfaceIdentity, revision: u64 },
    ListUiSurfaces { conversation_id: String },
    ReadUiAuthority,
    ReadUiCapability,
    StepCompactionSummary { step: nola_kernel::compaction_summary::CompactionSummaryStep },
    EvaluateCompactionMaintenance { facts: nola_domain::compaction_maintenance::CompactionMaintenanceFacts },
    /// Trusted local Host issuance only; never expose a raw snapshot HTTP write.
    PersistPreparedHostToolPlan {
        snapshot: nola_domain::host_tool_plan::HostToolPlanSnapshot,
        now_ms: u64,
    },
    ReadPreparedHostToolPlan {
        reference: nola_domain::host_tool_plan::HostToolPlanReference,
        now_ms: u64,
    },
    ArchiveDirectoryRevision {},
    BeginArchiveRestore {
        input: nola_protocol::BeginArchiveRestore,
    },
    ArchiveRestoreProgress {
        restore_id: String,
    },
    StageArchiveManifest {
        input: nola_protocol::StageArchiveManifest,
    },
    CommitArchiveRestore {
        input: nola_protocol::CommitArchiveRestore,
    },
    PublishUploadedTranscript {
        input: nola_protocol::uploaded_transcript::UploadedTranscriptManifest,
    },
    BeginContentUpload {
        input: nola_protocol::content_upload::BeginContentUpload,
    },
    AppendContentChunk {
        input: nola_protocol::content_upload::AppendContentChunk,
    },
    ContentUploadProgress {
        input: nola_protocol::content_upload::ContentUploadRequest,
    },
    SealContentUpload {
        input: nola_protocol::content_upload::ContentUploadRequest,
    },
    CancelContentUpload {
        input: nola_protocol::content_upload::ContentUploadRequest,
    },
    ReadContentUploadChunk {
        input: nola_protocol::content_upload::ReadContentUploadChunk,
    },
    PutConversationPdfResource {
        input: nola_store_port::conversation_resource::PutConversationPdfResource,
    },
    ReadConversationResource {
        conversation_id: String,
        resource_id: String,
    },
    ListConversationResources {
        conversation_id: String,
    },
    DeleteConversationResource {
        conversation_id: String,
        resource_id: String,
    },
    BeginAssistantTurn {
        input: BeginAssistantTurn,
    },
    PromoteQueuedAssistantTurn {
        input: crate::PromoteQueuedAssistantTurn,
    },
    BeginQueuedUserTurn {
        input: crate::BeginQueuedUserTurn,
    },
    ListOpenUserInputWaits {
        conversation_id: String,
        run_id: Option<String>,
    },
    ListOwnedTaskUserInputWaits {
        owner_conversation_id: String,
        task_id: Option<String>,
    },
    LoadUserInputWait {
        conversation_id: String,
        wait_id: String,
    },
    RespondToConversationWait {
        input: crate::RespondToConversationWait,
    },
    BeginTaskAssistantTurn {
        lease: crate::RuntimeTaskLeaseWitness,
        input: BeginAssistantTurn,
    },
    BeginAutomationAssistantTurn {
        lease: nola_domain::automation_execution::AutomationExecutionLease,
        input: BeginAssistantTurn,
    },
    ObserveAutomationExecution { occurrence_id: String },
    ListPendingAutomationExecutions { after_occurrence_id: Option<String>, limit: usize },
    ReconcileAutomationExecution { occurrence_id: String, reconciled_at_ms: i64 },
    ObserveAutomationWorker {
        lease: nola_domain::automation_execution::AutomationExecutionLease,
        observation: nola_domain::automation_execution::AutomationWorkerObservation,
        observed_at_ms: i64,
    },
    AckAutomationNotification { occurrence_id: String, notification_id: String, delivered_at_ms: i64 },
    BeginMaintenanceTurn {
        input: BeginMaintenanceTurn,
    },
    QueryClaimableMemoryJobs {
        input: QueryClaimableMemoryJobs,
    },
    BuildMemoryWorkerPlan {
        input: BuildMemoryWorkerPlan,
    },
    LoadCommittedHostRequests {
        run_id: String,
    },
    InspectRunProcessOwners { conversation_id: String, expected_run_id: String },
    ValidateRunProcessObservations { plan: crate::run_process_observation::RunProcessObservationPlan, observations: Vec<crate::run_process_observation::RunProcessObservation> },
    LoadRunDeliveryOwnership {
        run_id: String,
    },
    StepGoal { conversation_id: String, source_host_id: String, control: Option<nola_agent_engine::agent::GoalRuntimeCommandEnvelope>,
        start_configuration: Option<Box<nola_agent_engine::agent::protocol::AgentCoreConfig>>, selected_model: Option<GoalSelectedModel>,
        #[serde(default)] user_prompt: String, host_available: Option<bool> },
    StartGoalExecution { input: crate::goal_execution::StartGoalExecution },
    PrepareGoalControlIntent { input: crate::goal_control_intent::PrepareGoalControlIntent },
    LoadGoalExecutionBinding { conversation_id: String, goal_id: String },
    PrepareGoalExecution { conversation_id: String, goal_id: String },
    LoadHostExecutionState {
        conversation_id: String,
    },
    LoadConversationTaskProjectionV1 {
        conversation_id: String,
    },
    ClaimCommittedHostRequests {
        input: ClaimCommittedHostRequests,
    },
    ClaimApplicationControlRequests {
        input: ClaimCommittedHostRequests,
    },
    PrepareDelegatedTask { input: crate::delegation_preparation::PrepareDelegatedTask },
    CaptureMobileDelegationConfiguration { config: nola_agent_engine::agent::AgentCoreConfig, settings: crate::mobile_delegation_configuration::MobileDelegationSettings },
    LoadMobileTaskChildConfiguration { task_id: String },
    LoadMobileTaskWakePlan { now_ms: i64 },
    InspectMobilePermissionRequest { request: nola_agent_engine::agent::host::HostRequestEnvelope, now_ms: i64 },
    InspectMobileMcpRequest { request: nola_agent_engine::agent::host::HostRequestEnvelope, now_ms: i64 },
    PrepareMobileTaskGuidance { input: crate::mobile_delegation::PrepareMobileDelegatedTask },
    ObserveMobileTask { input: crate::mobile_delegation::PrepareMobileDelegatedTask },
    ApplyPendingTaskNotice { conversation_id: String, consumer_id: String, now_ms: i64 },
    PrepareMobileDelegatedTask { input: crate::mobile_delegation::PrepareMobileDelegatedTask },
    AdmitMobileTaskChild { input: crate::mobile_delegation::AdmitMobileTaskChild },
    ExecuteTaskCancellationForEffect {
        request: nola_agent_engine::agent::host::HostRequestEnvelope,
        now_ms: i64,
    },
    #[serde(skip_deserializing)]
    ImportHostTaskCancellation {
        authorization: nola_domain::agent_task_admission::AgentTaskCancellationAuthorization,
        now_ms: i64,
    },
    PrepareRunEffectRecovery {
        input: crate::effect_delivery::PrepareRunEffectRecovery,
    },
    HeartbeatEffectDelivery {
        input: HeartbeatEffectDelivery,
    },
    ResolveUnknownEffectRecovery {
        input: ResolveUnknownEffectRecovery,
    },
    InspectOwnedTaskRecovery { owner: crate::owned_task_cancellation::OwnedTaskCancellation },
    ResolveOwnedTaskRecovery { owner: crate::owned_task_cancellation::OwnedTaskCancellation, input: ResolveUnknownEffectRecovery },
    LoadRuntimeEffectWorkSchedule {
        input: LoadRuntimeEffectWorkSchedule,
    },
    ActiveRun {
        conversation_id: String,
    },
    CanonicalRunTerminal {
        conversation_id: String,
        run_id: String,
    },
    ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse,
    },
    ReduceAndCommitActiveSubmission {
        input: ReduceAndCommitActiveSubmission,
    },
    RequestRunCancellation {
        input: crate::RequestRunCancellation,
    },
    LoadRunCancellationDeliveryFacts {
        input: crate::cancellation_delivery::CancellationRunIdentity,
    },
    ClaimCancellationHostRequests {
        input: crate::cancellation_delivery::ClaimCancellationHostRequests,
    },
    ReduceAndCommitIdleSubmission {
        input: ReduceAndCommitIdleSubmission,
    },
    RevertConversation {
        input: RevertConversation,
    },
    CreateConversationBranch {
        input: CreateConversationBranch,
    },
    PrepareRemoteSubmission { input: PrepareRemoteSubmissionInput },
    AbandonPreparedRemoteSubmission { identity: RemoteSubmissionIdentity },
    AdoptObservedRemoteRun { receipt: SourceSubmissionReceipt },
    ObservedSourceSubmissionReceipt { source_host_id: String, client_id: String, conversation_id: String, run_id: String, assistant_message_id: String },
    ObserveSubmissionFrontier { source_host_id: String, request: nola_domain::remote_submission::SubmissionFrontierRequest },
    BindRemoteSubmissionRequest { identity: RemoteSubmissionIdentity, request: Value },
    ReadRemoteSubmissionIntent { conversation_id: String },
    ReadAdmittedRemoteRunIdentity { source_host_id: String, client_id: String, conversation_id: String, run_id: String, assistant_message_id: String },
    ReadRemoteSubmissionRequest { identity: RemoteSubmissionIdentity, request_digest: String },
    RegisterSourceSubmission { identity: RemoteSubmissionIdentity, request: Value },
    ReadSourceSubmissionReceipt { identity: RemoteSubmissionIdentity, request_digest: String },
    CancelSourceSubmission { identity: RemoteSubmissionIdentity, request_digest: String },
    CancelRejectedQueuedUiAction { identity: RemoteSubmissionIdentity, request_digest: String, now_ms: i64 },
    RejectSourceSubmission { identity: RemoteSubmissionIdentity, request_digest: String, rejection_code: String },
    AcceptRemoteSubmissionReceipt { receipt: SourceSubmissionReceipt },
    CreateConversation {
        input: CreateConversation,
    },
    AdmitConversationUserMessage {
        input: nola_protocol::AdmitConversationUserMessage,
    },
    AgentRoom { command: crate::agent_room::AgentRoomCommand },
    ImportConversationTranscript {
        input: ImportConversationTranscript,
    },
    EditConversationMessage {
        input: EditConversationMessage,
    },
    DeleteConversationMessage {
        input: DeleteConversationMessage,
    },
    ReadAssistantJournal {
        after: nola_domain::assistant_journal::AssistantJournalCursor,
        through: nola_domain::assistant_journal::AssistantJournalCursor,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    LocalAssistantJournalHead {
        identity: nola_domain::assistant_journal::AssistantJournalIdentity,
    },
    LocalRunAssistantJournalHeads {
        conversation_id: String,
        run_id: String,
        after_message_id: Option<String>,
        limit: usize,
    },
    LocalAssistantRunTerminalSeal {
        conversation_id: String,
        run_id: String,
    },
    LocalRunMessageAdmissionHead {
        conversation_id: String,
        run_id: String,
    },
    LocalRunContextUsage {
        identity: nola_domain::assistant_journal::AssistantJournalIdentity,
    },
    AcceptRemoteRunContextUsage {
        source_host_id: String,
        record: nola_domain::committed_run_context_usage::CommittedRunContextUsage,
    },
    ReadRunMessageAdmissions {
        after: nola_domain::run_message_admission::RunMessageAdmissionCursor,
        through: nola_domain::run_message_admission::RunMessageAdmissionCursor,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    RunMessageAdmissionReceptionState {
        source_host_id: String,
        through: nola_domain::run_message_admission::RunMessageAdmissionCursor,
    },
    ReceiveRunMessageAdmissions {
        source_host_id: String,
        page: nola_domain::run_message_admission::RunMessageAdmissionPage,
    },
    BeginReceiveAssistantRunTerminal {
        source_host_id: String,
        seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    },
    StageAssistantRunTerminalManifest {
        source_host_id: String,
        page: nola_domain::assistant_run_terminal::AssistantRunTerminalManifestPage,
    },
    CommitReceivedAssistantRunTerminal {
        source_host_id: String,
        seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    },
    ReadLocalAssistantRunTerminalManifest {
        seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
        after_entry: nola_domain::assistant_journal_operations::JournalNumber,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    OpenAssistantJournalSnapshot {
        at: nola_domain::assistant_journal::AssistantJournalCursor,
    },
    ReceiveAssistantJournalRecord {
        source_host_id: String,
        record: crate::AssistantJournalRecord,
        chunks: Vec<crate::AssistantJournalChunk>,
    },
    StageAssistantJournalContent {
        input: crate::StageAssistantJournalContent,
    },
    NextAssistantJournalContent {
        source_host_id: String,
        record: crate::AssistantJournalRecord,
    },
    AssistantJournalReceptionState {
        source_host_id: String,
        through: nola_domain::assistant_journal::AssistantJournalCursor,
    },
    ReadAssistantJournalManifest {
        snapshot: crate::AssistantJournalSnapshotHandle,
        part_id: String,
        content: nola_domain::assistant_journal_operations::JournalContentRef,
        after_chunk: nola_domain::assistant_journal_operations::JournalNumber,
        max_records: usize,
    },
    ReadAssistantJournalChunk {
        snapshot: crate::AssistantJournalSnapshotHandle,
        part_id: String,
        content: nola_domain::assistant_journal_operations::JournalContentRef,
        chunk_index: nola_domain::assistant_journal_operations::JournalNumber,
    },
    ReadAssistantJournalRecordChunk {
        snapshot: crate::AssistantJournalSnapshotHandle,
        part_id: String,
        content: nola_domain::assistant_journal_operations::JournalContentRef,
        chunk_index: nola_domain::assistant_journal_operations::JournalNumber,
    },
    ReadAssistantJournalParts {
        snapshot: crate::AssistantJournalSnapshotHandle,
        after_part_id: Option<String>,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    ReadAssistantJournalPartChanges {
        snapshot: crate::AssistantJournalSnapshotHandle,
        base: nola_domain::assistant_journal::AssistantJournalCursor,
        after_part_id: Option<String>,
        max_records: usize,
        max_encoded_bytes: usize,
    },
    ReleaseAssistantJournalSnapshot {
        snapshot: crate::AssistantJournalSnapshotHandle,
    },
    ReadAssistantJournalRange {
        snapshot: crate::AssistantJournalSnapshotHandle,
        part_id: String,
        content: nola_domain::assistant_journal_operations::JournalContentRef,
        offset_bytes: nola_domain::assistant_journal_operations::JournalNumber,
        limit_bytes: u32,
    },
    InspectRunRecovery {
        conversation_id: String,
        expected_run_id: String,
        assistant_message_id: String,
    },
    InspectUnknownEffectRecovery {
        conversation_id: String,
        expected_run_id: String,
        assistant_message_id: String,
    },
    ListRecoverableRuns {
        after_run_id: Option<String>,
        limit: usize,
    },
    AuxiliaryInvocation {
        command: nola_domain::AuxiliaryInvocationCommand,
    },
    LoadAuxiliaryOperation {
        operation_id: String,
    },
    LoadAuxiliarySourceSlot {
        local_owner: nola_domain::AuxiliaryInvocationOwner,
        scenario: nola_domain::AuxiliaryInvocationScenario,
    },
    ListConversationAuxiliarySources {
        conversation_id: String,
        after: Option<nola_domain::AuxiliarySourceCursor>,
        limit: usize,
    },
    EncodeHostWorkerIdentity {
        identity: nola_host_protocol::HostWorkerIdentity,
    },
    InspectCancellationProcessOwners {
        identity: crate::cancellation_delivery::CancellationRunIdentity,
        control: nola_agent_engine::agent::host::HostRequestEnvelope,
    },
    ValidateCancellationProcessObservations {
        plan: crate::cancellation_process_observation::CancellationProcessObservationPlan,
        current_process: nola_host_protocol::HostProcessObservation,
        observations: Vec<crate::cancellation_process_observation::CancellationProcessObservation>,
    },
    ValidateRunRecoveryAuthority {
        conversation_id: String,
        expected_run_id: String,
        assistant_message_id: String,
        expected_invocation_config_digest: String,
        current: crate::RunRecoveryAuthority,
    },
    ValidateMobileRunRecoveryAuthority {
        conversation_id: String,
        expected_run_id: String,
        assistant_message_id: String,
        expected_invocation_config_digest: String,
        current: crate::RunRecoveryAuthority,
    },
    LoadExactRunInvocation {
        conversation_id: String,
        expected_run_id: String,
        assistant_message_id: String,
    },
    Context {
        command: ContextCommand,
    },
    PatchConversationMetadata {
        input: nola_store_port::conversation_profile::PatchConversationMetadataInput,
    },
    PatchConversationConfiguration {
        input: nola_store_port::conversation_profile::PatchConversationConfigurationInput,
    },
    SelectConversationExecution {
        input: crate::conversation_configuration::SelectConversationExecutionInput,
    },
    ReadPermissionConfiguration {
        conversation_id: String,
        source_host_id: Option<String>,
        client_id: Option<String>,
        #[serde(default)]
        unresolved_run_ids: Vec<String>,
        #[serde(default)]
        source_unconfirmed: bool,
        minimum_policy_revision: Option<i64>,
    },
    SetConversationPermission {
        conversation_id: String,
        expected_revision: i64,
        expected_run_id: Option<String>,
        expected_source_host_id: Option<String>,
        profile: nola_domain::PermissionProfile,
        additional_configuration: Option<nola_domain::configuration_patch::ConversationConfigurationPatch>,
        updated_at_ms: i64,
    },
    LoadConversation {
        conversation_id: String,
    },
    LoadRunIdentity {
        run_id: String,
    },
    LoadConversationMetadata {
        conversation_id: String,
    },
    ReadHistoricalPart {
        owner: nola_domain::CanonicalPartOwner,
        expected_conversation_revision: i64,
        maximum_output_bytes: usize,
    },
    ReadConversationPartDerivation {
        owner: nola_domain::CanonicalPartOwner,
        expected_conversation_revision: i64,
    },
    ReadConversationToolSources {
        owner: nola_domain::CanonicalPartOwner,
        expected_conversation_revision: i64,
    },
    ReadCodeCellObservation {
        query: nola_store_port::code_cell::CodeCellQuery,
    },
    ReadCodeCellHistoryForEffect {
        request: nola_agent_engine::agent::host::HostRequestEnvelope,
    },
    ReadConversationToolSourceRange {
        owner: nola_domain::CanonicalPartOwner,
        source_item_id: String,
        expected_conversation_revision: i64,
        offset_bytes: u64,
        limit_bytes: usize,
    },
    LoadConversationExportPage {
        conversation_id: String,
        after_ordinal: Option<i64>,
        expected_revision: Option<i64>,
    },
    LoadCompactionLineage {
        conversation_id: String,
        compaction_part_id: Option<String>,
    },
    ReadConversationExportRange {
        conversation_id: String,
        message_id: String,
        part_id: String,
        field: nola_protocol::ExportBodyField,
        expected_revision: i64,
        offset_bytes: u64,
        limit_bytes: usize,
    },
    ReadConversationTextRange {
        conversation_id: String,
        message_id: String,
        part_id: String,
        offset_bytes: u64,
        limit_bytes: usize,
    },
    SearchConversationText {
        conversation_id: String,
        query: String,
        limit: usize,
    },
    QueryCanonicalHistory {
        input: nola_domain::canonical_history::CanonicalHistoryQuery,
    },
    LoadConversationProjectionV1 {
        conversation_id: String,
    },
    #[serde(skip_deserializing)]
    PreviewExpertTemplateCapture { input: nola_domain::expert_message::AdmitExpertConversationMessage, authority: crate::expert_template_capture::HostExpertTemplateAuthority },
    /// Trusted native/server Host factory only; never renderer Application JSON.
    #[serde(skip_deserializing)]
    AdmitExpertConversationMessage {
        input: nola_domain::expert_message::AdmitExpertConversationMessage,
        context: crate::expert_template_capture::HostExpertTemplateCaptureContext,
        captured_at_ms: i64,
    },
    #[serde(skip_deserializing)]
    PrepareEditedExpertResend {
        input: nola_domain::edited_resend::PrepareEditedResend,
        authority: crate::expert_template_capture::HostExpertTemplateAuthority,
        captured_at_ms: i64,
    },
    #[serde(skip_deserializing)]
    LoadPreparedExpertResend { conversation_id: String, authority: crate::expert_template_capture::HostExpertTemplateAuthority },
    #[serde(skip_deserializing)]
    LoadExpertMessageManifest { conversation_id: String, message_id: String, authority: crate::expert_template_capture::HostExpertTemplateAuthority },
    #[serde(skip_deserializing)]
    LoadExpertChildInstructions { conversation_id: String, expected_run_id: String, authority: crate::expert_template_capture::HostExpertTemplateAuthority },
    PrepareEditedResend { input: nola_domain::edited_resend::PrepareEditedResend },
    LoadPreparedEditedResend { conversation_id: String },
    /// Trusted local Host transport only; never renderer Application JSON.
    #[serde(skip_deserializing)]
    LoadEditedResendHostMaterial { conversation_id: String },
    #[serde(skip_deserializing)]
    AttestEditedResendHostPolicy { conversation_id: String, core_config: nola_agent_engine::agent::AgentCoreConfig, submission: nola_agent_engine::agent::AgentSubmission },
    ListActivity { input: crate::ActivityQuery },
    LoadActivityDetail { conversation_id:String, run_id:String, assistant_message_id:String },
    LoadTodayFacts { input: crate::TodayFactsQuery },
    LoadConversationContextUsageV1 {
        conversation_id: String,
    },
    LoadConversationPermissionProjectionV1 {
        conversation_id: String,
        run_id: String,
        scope: PermissionProjectionReadScope,
    },
    LoadConversationProjectionPageV1 {
        conversation_id: String,
        offset: usize,
        limit: usize,
    },
    LoadConversationSourceOwner { conversation_id: String },
    LoadTaskChildConversationWindow {
        parent_conversation_id: String, parent_run_id: String, task_id: String,
        child_conversation_id: String, before_ordinal: Option<i64>,
        limit: usize, content_byte_limit: usize,
    },
    LoadConversationProjectionWindowV1 {
        conversation_id: String,
        before_ordinal: Option<i64>,
        limit: usize,
        content_byte_limit: usize,
    },
    LoadConversationProjectionWindowUpdateV1 {
        conversation_id: String,
        before_ordinal: Option<i64>,
        limit: usize,
        content_byte_limit: usize,
        after_sequence: u64,
    },
    LoadConversationTailSnapshotV1 {
        conversation_id: String,
        message_limit: usize,
        /// Maximum Unicode scalars in each text tail (excluding its marker).
        part_text_limit: usize,
        /// Maximum UTF-8 bytes in each original or projected JSON payload.
        part_json_limit: usize,
    },
    LoadConversationSettings {
        conversation_id: String,
    },
    ListConversations {
        query: nola_store_port::conversation_query::ConversationListQuery,
    },
    RenameConversation {
        conversation_id: String,
        title: String,
        title_source: Option<nola_domain::conversation_configuration::ConversationTitleSource>,
        updated_at_ms: i64,
    },
    DeleteConversation {
        conversation_id: String,
    },
    UpsertPermissionGrant {
        input: PermissionGrantInput,
    },
    EffectivePermissionGrants {
        conversation_id: Option<String>,
        host_id: Option<String>,
    },
    ListPermissionGrants {
        conversation_id: Option<String>,
        host_id: Option<String>,
    },
    RevokePermissionGrant {
        id: String,
        revoked_at_ms: i64,
    },
    Skill {
        command: SkillCommand,
    },
    Mcp {
        command: McpCommand,
    },
    Plugin {
        command: crate::PluginCommand,
    },
    DataSource {
        command: DataSourceCommand,
    },
    DocumentSource {
        command: DocumentSourceCommand,
    },
    Memory {
        command: MemoryCommand,
    },
    Model {
        command: ModelCommand,
    },
    AdmitHostOutput {
        routes: Vec<nola_host_protocol::HostRouteTokenLimits>,
        preference: nola_domain::generation::OutputTokenPreference,
    },
    ResolveEffectiveModelParameters { input: crate::effective_model_parameters::ResolveEffectiveModelParameters },
    QueryAutomationOccurrences { input:crate::automation_occurrence::OccurrenceQuery },
    ResolveRemoteModelProfile { profile_id:String, expected_revision:i64, request:Value },
    ReconcileRoomRouting { room_id:String, submission_id:String, source_host_id:Option<String>, at_ms:i64 },
    ResolveChatModel {
        intent: crate::model_selection::ChatModelIntent,
        default_value_json: Option<String>,
        candidates: Vec<crate::model_selection::ChatModelCandidate>,
    },
    ResolveModelReasoning {
        declaration: Value,
        protocol: nola_domain::generation::ModelProtocol,
        output_limit: u64,
        supports_reasoning: bool,
        requested: Value,
        #[serde(default)]
        tier: Option<nola_domain::reasoning_tiers::ReasoningTier>,
    },
    ConversationPlan {
        command: ConversationPlanCommand,
    },
    RuntimeTask {
        command: RuntimeTaskCommand,
    },
    RecoverExpiredAgentTask { input: crate::RecoverExpiredAgentTask },
    ClaimAgentTaskFair { input: crate::ClaimAgentTaskFair },
    ExportAgentTaskAdmission {
        conversation_id: String,
        run_id: String,
        task_id: String,
        event_cursor: u64,
    },
    /// Constructed only by the trusted local Host transport, never ordinary
    /// Application JSON or model/tool arguments.
    #[serde(skip_deserializing)]
    ImportHostAgentTaskAdmission { admission: crate::AgentTaskAdmission },
    ExportAgentTaskTerminalProjection {
        task_id: String,
        expected_receipt_id: String,
        guidance_message_ids: Vec<String>,
    },
    #[serde(skip_deserializing)]
    ImportHostAgentTaskTerminalProjection {
        projection: nola_domain::agent_task_admission::AgentTaskTerminalProjection,
    },
    ExportAgentTaskGuidanceProjection {
        task_id: String,
        message_id: String,
        settlement: bool,
    },
    #[serde(skip_deserializing)]
    ImportHostAgentTaskGuidanceProjection {
        projection: nola_domain::agent_task_admission::AgentTaskGuidanceProjection,
    },
    SopRun {
        command: SopRunCommand,
    },
    WorkStepTransaction {
        command: WorkStepTransactionCommand,
    },
    PermissionRequest {
        command: PermissionRequestCommand,
    },
    Automation {
        command: AutomationCommand,
    },
    EnqueueAgentCommand {
        input: AgentCommand,
    },
    EnqueueTaskTerminalReconciliation {
        task_id: String,
        expected_receipt_id: String,
    },
    LoadTaskGuidanceReceipt {
        task_id: String,
        message_id: String,
        parent_conversation_id: String,
        parent_run_id: String,
    },
    SubmitTaskGuidance {
        task_id: String,
        message_id: String,
        parent_turn_id: String,
        content: String,
        submitted_at_ms: i64,
    },
    ApplyTaskGuidance {
        task_id: String,
        message_id: String,
        lease: Option<crate::RuntimeTaskLeaseWitness>,
        expected_run_id: String,
        now_ms: i64,
    },
    ApplyActivePermissionPolicy {
        conversation_id: String,
        policy: nola_domain::ActivePermissionPolicy,
        now_ms: i64,
    },
    PeekAgentCommand {
        session_id: String,
    },
    ClaimNextAgentCommand {
        session_id: String,
        lease_owner: String,
        now_ms: i64,
        lease_until_ms: i64,
    },
    LeasedAgentCommands {
        session_id: String,
    },
    ReleaseAgentCommandLease {
        session_id: String,
        command_id: String,
        lease_owner: String,
    },
    AcknowledgeAgentCommand {
        session_id: String,
        command_id: String,
        lease_owner: String,
    },
    RecordAgentTrace {
        event: AgentTraceEvent,
        envelope: Option<Value>,
    },
    LoadExactRunTraceReplay {
        conversation_id: String,
        run_id: String,
        assistant_message_id: String,
        after_sequence: i64,
        through_sequence: Option<i64>,
        limit: usize,
    },
    TraceEventsAfter {
        trace_id: String,
        sequence: i64,
    },
    TraceRunsForConversation {
        conversation_id: String,
    },
    LatestTraceId {
        conversation_id: String,
    },
    RecoverOrphanedRuns {
        conversation_id: Option<String>,
        reason: String,
        interrupted_at: String,
        interrupted_at_ms: i64,
    },
    RecoverObservedOrphanedRun {
        conversation_id: String,
        expected_run_id: String,
        host_invocation: nola_host_protocol::HostInvocationBinding,
        observed_session_lease: Option<nola_store_port::workspace_ownership::StoredSessionRunLease>,
        observed_deliveries: Vec<nola_agent_engine::agent::host::EffectDeliveryLease>,
        interrupted_at: String,
        interrupted_at_ms: i64,
    },
    RecoverExpiredRunDeliveries {
        conversation_id: String,
        expected_run_id: String,
        host_invocation: nola_host_protocol::HostInvocationBinding,
        interrupted_at: String,
        interrupted_at_ms: i64,
    },
    SaveWorkspaceCheckpoint {
        input: WorkspaceCheckpoint,
    },
    ListWorkspaceCheckpoints {
        root: String,
    },
    DeleteWorkspaceCheckpoint {
        id: String,
    },
    AcquireSessionRunLease {
        input: SessionRunLease,
    },
    GetSessionRunLease {
        session_id: String,
    },
    ListSessionRunLeases,
    ReleaseSessionRunLease {
        session_id: String,
        owner: String,
    },
    PutProductSetting {
        key: String,
        value_json: String,
        updated_at_ms: i64,
    },
    PutProductSettingChecked {
        key: String,
        value_json: String,
        expected_revision: Option<i64>,
        updated_at_ms: i64,
    },
    GetProductSetting {
        key: String,
    },
    #[serde(skip_deserializing)]
    ListReadableExpertTemplates {
        actor: nola_agent_engine::agent::resource_access::AgentResourcePrincipal,
        now_epoch_ms: i64,
        request_host_id: Option<String>,
    },
    DeleteProductSetting {
        key: String,
    },
    PendingSyncChanges {
        limit: usize,
    },
    ResolveSyncChange {
        operation_id: String,
        status: String,
        remote_cursor: Option<i64>,
        resolved_at_ms: i64,
    },
    GetSyncCursor {
        replica_id: String,
    },
    SaveSyncCursor {
        replica_id: String,
        cursor: i64,
        updated_at_ms: i64,
    },
    ApplyRemoteSyncPage {
        input: RemoteSyncPage,
    },
    LoadCheckpoint {
        conversation_id: String,
    },
    LoadSessionProjection {
        conversation_id: String,
    },
    LoadGoalControlObservation { conversation_id: String },
    LoadGoalExecutionOwner { conversation_id: String, goal_id: String, run_id: String },
    LoadSessionGoalProjection {
        conversation_id: String,
    },
    FindResumableGoalSession {
        project_root: String,
    },
    LoadResumeSnapshot {
        project_root: String,
        host_id: Option<String>,
        requested_at_ms: i64,
    },
    AcknowledgeResumeSnapshot {
        input: crate::AcknowledgeResumeSnapshot,
    },
}

#[derive(Clone, Copy)]
enum ApplicationCommandFamily {
    Conversation,
    AgentRuntime,
    CapabilityCatalog,
    Operations,
}

impl ApplicationCommand {
    fn family(&self) -> ApplicationCommandFamily {
        match self {
            Self::ArchiveDirectoryRevision { .. }
            | Self::BeginArchiveRestore { .. }
            | Self::ArchiveRestoreProgress { .. }
            | Self::StageArchiveManifest { .. }
            | Self::CommitArchiveRestore { .. }
            | Self::PublishUploadedTranscript { .. }
            | Self::BeginContentUpload { .. }
            | Self::AppendContentChunk { .. }
            | Self::ContentUploadProgress { .. }
            | Self::SealContentUpload { .. }
            | Self::CancelContentUpload { .. }
            | Self::ReadContentUploadChunk { .. }
            | Self::PutConversationPdfResource { .. }
            | Self::ReadConversationResource { .. }
            | Self::ListConversationResources { .. }
            | Self::DeleteConversationResource { .. }
            | Self::RevertConversation { .. }
            | Self::CreateConversationBranch { .. }
            | Self::PrepareRemoteSubmission { .. }
            | Self::AbandonPreparedRemoteSubmission { .. }
            | Self::AdoptObservedRemoteRun { .. }
            | Self::ObserveSubmissionFrontier { .. }
            | Self::ObservedSourceSubmissionReceipt { .. }
            | Self::BindRemoteSubmissionRequest { .. }
            | Self::ReadRemoteSubmissionIntent { .. }
            | Self::ReadAdmittedRemoteRunIdentity { .. }
            | Self::ReadRemoteSubmissionRequest { .. }
            | Self::RegisterSourceSubmission { .. }
            | Self::ReadSourceSubmissionReceipt { .. }
            | Self::CancelRejectedQueuedUiAction { .. }
            | Self::CancelSourceSubmission { .. }
            | Self::RejectSourceSubmission { .. }
            | Self::AcceptRemoteSubmissionReceipt { .. }
            | Self::CanonicalRunTerminal { .. }
            | Self::CreateConversation { .. }
            | Self::AdmitConversationUserMessage { .. }
            | Self::AgentRoom { .. }
            | Self::ImportConversationTranscript { .. }
            | Self::EditConversationMessage { .. }
            | Self::DeleteConversationMessage { .. }
            | Self::ReadAssistantJournal { .. }
            | Self::LocalAssistantJournalHead { .. }
            | Self::LocalRunAssistantJournalHeads { .. }
            | Self::LocalAssistantRunTerminalSeal { .. }
            | Self::LocalRunMessageAdmissionHead { .. }
            | Self::LocalRunContextUsage { .. }
            | Self::AcceptRemoteRunContextUsage { .. }
            | Self::ReadRunMessageAdmissions { .. }
            | Self::RunMessageAdmissionReceptionState { .. }
            | Self::ReceiveRunMessageAdmissions { .. }
            | Self::BeginReceiveAssistantRunTerminal { .. }
            | Self::StageAssistantRunTerminalManifest { .. }
            | Self::CommitReceivedAssistantRunTerminal { .. }
            | Self::ReadLocalAssistantRunTerminalManifest { .. }
            | Self::OpenAssistantJournalSnapshot { .. }
            | Self::ReleaseAssistantJournalSnapshot { .. }
            | Self::ReadAssistantJournalRange { .. }
            | Self::ReadAssistantJournalParts { .. }
            | Self::ReadAssistantJournalPartChanges { .. }
            | Self::ReadAssistantJournalManifest { .. }
            | Self::ReadAssistantJournalChunk { .. }
            | Self::ReadAssistantJournalRecordChunk { .. }
            | Self::ReceiveAssistantJournalRecord { .. }
            | Self::StageAssistantJournalContent { .. }
            | Self::NextAssistantJournalContent { .. }
            | Self::AssistantJournalReceptionState { .. }
            | Self::InspectRunRecovery { .. }
            | Self::InspectUnknownEffectRecovery { .. }
            | Self::LoadExactRunInvocation { .. }
            | Self::ValidateRunRecoveryAuthority { .. }
            | Self::ValidateMobileRunRecoveryAuthority { .. }
            | Self::EncodeHostWorkerIdentity { .. }
            | Self::ListRecoverableRuns { .. }
            | Self::PatchConversationConfiguration { .. }
            | Self::SelectConversationExecution { .. }
            | Self::ReadPermissionConfiguration { .. }
            | Self::SetConversationPermission { .. }
            | Self::PatchConversationMetadata { .. }
            | Self::LoadConversation { .. }
            | Self::LoadConversationMetadata { .. }
            | Self::ReadHistoricalPart { .. }
            | Self::ReadConversationPartDerivation { .. }
            | Self::ReadConversationToolSources { .. }
            | Self::ReadCodeCellObservation { .. }
            | Self::ReadCodeCellHistoryForEffect { .. }
            | Self::ReadConversationToolSourceRange { .. }
            | Self::LoadConversationExportPage { .. }
            | Self::LoadCompactionLineage { .. }
            | Self::ReadConversationExportRange { .. }
            | Self::ReadConversationTextRange { .. }
            | Self::SearchConversationText { .. }
            | Self::QueryCanonicalHistory { .. }
            | Self::LoadRunIdentity { .. }
            | Self::LoadConversationProjectionV1 { .. }
            | Self::AdmitExpertConversationMessage { .. }
            | Self::PrepareEditedExpertResend { .. }
            | Self::ResolveEffectiveModelParameters { .. }
            | Self::PreviewExpertTemplateCapture { .. }
            | Self::LoadPreparedExpertResend { .. }
            | Self::LoadExpertMessageManifest { .. }
            | Self::LoadExpertChildInstructions { .. }
            | Self::PrepareEditedResend { .. }
            | Self::LoadPreparedEditedResend { .. }
            | Self::LoadEditedResendHostMaterial { .. }
            | Self::AttestEditedResendHostPolicy { .. }
            | Self::ListActivity { .. }
            | Self::LoadActivityDetail { .. }
            | Self::LoadTodayFacts { .. }
            | Self::LoadConversationContextUsageV1 { .. }
            | Self::LoadConversationPermissionProjectionV1 { .. }
            | Self::LoadConversationProjectionPageV1 { .. }
            | Self::LoadConversationSourceOwner { .. }
            | Self::LoadTaskChildConversationWindow { .. }
            | Self::LoadConversationProjectionWindowV1 { .. }
            | Self::LoadConversationProjectionWindowUpdateV1 { .. }
            | Self::LoadConversationTailSnapshotV1 { .. }
            | Self::LoadConversationSettings { .. }
            | Self::ListConversations { .. }
            | Self::RenameConversation { .. }
            | Self::DeleteConversation { .. } => ApplicationCommandFamily::Conversation,

            Self::BeginAssistantTurn { .. }
            | Self::PromoteQueuedAssistantTurn { .. }
            | Self::BeginQueuedUserTurn { .. }
            | Self::ListOpenUserInputWaits { .. }
            | Self::ListOwnedTaskUserInputWaits { .. }
            | Self::LoadUserInputWait { .. }
            | Self::RespondToConversationWait { .. }
            | Self::BeginTaskAssistantTurn { .. }
            | Self::BeginAutomationAssistantTurn { .. }
            | Self::ObserveAutomationExecution { .. }
            | Self::ListPendingAutomationExecutions { .. }
            | Self::ReconcileAutomationExecution { .. }
            | Self::ObserveAutomationWorker { .. }
            | Self::AckAutomationNotification { .. }
            | Self::BeginMaintenanceTurn { .. }
            | Self::QueryClaimableMemoryJobs { .. }
            | Self::BuildMemoryWorkerPlan { .. }
            | Self::LoadCommittedHostRequests { .. }
            | Self::InspectRunProcessOwners { .. }
            | Self::ValidateRunProcessObservations { .. }
            | Self::LoadRunDeliveryOwnership { .. }
            | Self::ReconcileRoomRouting { .. }
            | Self::StepGoal { .. }
            | Self::StartGoalExecution { .. }
            | Self::PrepareGoalControlIntent { .. }
            | Self::LoadGoalExecutionBinding { .. }
            | Self::PrepareGoalExecution { .. }
            | Self::LoadHostExecutionState { .. }
            | Self::LoadConversationTaskProjectionV1 { .. }
            | Self::ClaimApplicationControlRequests { .. }
            | Self::PrepareDelegatedTask { .. }
            | Self::CaptureMobileDelegationConfiguration { .. }
            | Self::LoadMobileTaskChildConfiguration { .. }
            | Self::LoadMobileTaskWakePlan { .. }
            | Self::InspectMobilePermissionRequest { .. }
            | Self::InspectMobileMcpRequest { .. }
            | Self::PrepareMobileTaskGuidance { .. }
            | Self::ObserveMobileTask { .. }
            | Self::ApplyPendingTaskNotice { .. }
            | Self::PrepareMobileDelegatedTask { .. }
            | Self::AdmitMobileTaskChild { .. }
            | Self::ExecuteTaskCancellationForEffect { .. }
            | Self::ImportHostTaskCancellation { .. }
            | Self::ClaimCommittedHostRequests { .. }
            | Self::PrepareRunEffectRecovery { .. }
            | Self::HeartbeatEffectDelivery { .. }
            | Self::ResolveUnknownEffectRecovery { .. }
            | Self::InspectOwnedTaskRecovery { .. }
            | Self::ResolveOwnedTaskRecovery { .. }
            | Self::LoadRuntimeEffectWorkSchedule { .. }
            | Self::ActiveRun { .. }
            | Self::ReduceAndAcceptHostResponse { .. }
            | Self::ReduceAndCommitActiveSubmission { .. }
            | Self::RequestRunCancellation { .. }
            | Self::LoadRunCancellationDeliveryFacts { .. }
            | Self::InspectCancellationProcessOwners { .. }
            | Self::ValidateCancellationProcessObservations { .. }
            | Self::ClaimCancellationHostRequests { .. }
            | Self::ReduceAndCommitIdleSubmission { .. }
            | Self::UpsertPermissionGrant { .. }
            | Self::EffectivePermissionGrants { .. }
            | Self::ListPermissionGrants { .. }
            | Self::RevokePermissionGrant { .. }
            | Self::Memory { .. }
            | Self::ConversationPlan { .. }
            | Self::RuntimeTask { .. }
            | Self::RecoverExpiredAgentTask { .. }
            | Self::ClaimAgentTaskFair { .. }
            | Self::ExportAgentTaskAdmission { .. }
            | Self::ImportHostAgentTaskAdmission { .. }
            | Self::ExportAgentTaskTerminalProjection { .. }
            | Self::ImportHostAgentTaskTerminalProjection { .. }
            | Self::ExportAgentTaskGuidanceProjection { .. }
            | Self::ImportHostAgentTaskGuidanceProjection { .. }
            | Self::SopRun { .. }
            | Self::WorkStepTransaction { .. }
            | Self::PermissionRequest { .. }
            | Self::EnqueueAgentCommand { .. }
            | Self::EnqueueTaskTerminalReconciliation { .. }
            | Self::LoadTaskGuidanceReceipt { .. }
            | Self::SubmitTaskGuidance { .. }
            | Self::ApplyTaskGuidance { .. }
            | Self::ApplyActivePermissionPolicy { .. }
            | Self::PeekAgentCommand { .. }
            | Self::ClaimNextAgentCommand { .. }
            | Self::LeasedAgentCommands { .. }
            | Self::ReleaseAgentCommandLease { .. }
            | Self::AcknowledgeAgentCommand { .. }
            | Self::RecordAgentTrace { .. }
            | Self::LoadExactRunTraceReplay { .. }
            | Self::TraceEventsAfter { .. }
            | Self::TraceRunsForConversation { .. }
            | Self::LatestTraceId { .. }
            | Self::RecoverOrphanedRuns { .. }
            | Self::RecoverObservedOrphanedRun { .. }
            | Self::RecoverExpiredRunDeliveries { .. }
            | Self::LoadCheckpoint { .. }
            | Self::LoadSessionProjection { .. }
            | Self::LoadGoalControlObservation { .. }
            | Self::LoadGoalExecutionOwner { .. }
            | Self::LoadSessionGoalProjection { .. }
            | Self::FindResumableGoalSession { .. }
            | Self::LoadResumeSnapshot { .. }
            | Self::AcknowledgeResumeSnapshot { .. } => ApplicationCommandFamily::AgentRuntime,

            Self::SubmitUiInteraction { .. }
            | Self::ExecuteUiTool { .. }
            | Self::ReadUiSurface { .. }
            | Self::ReadUiSurfaceChanges { .. }
            | Self::PreviewUiSurface { .. }
            | Self::ReadUiSurfaceRevision { .. }
            | Self::ReadUiAuthority
            | Self::ReadUiCapability
            | Self::ListUiSurfaces { .. }
            | Self::StepCompactionSummary { .. }
            | Self::EvaluateCompactionMaintenance { .. }
            | Self::Context { .. }
            | Self::PersistPreparedHostToolPlan { .. }
            | Self::ReadPreparedHostToolPlan { .. }
            | Self::AuxiliaryInvocation { .. }
            | Self::LoadAuxiliaryOperation { .. }
            | Self::LoadAuxiliarySourceSlot { .. }
            | Self::ListConversationAuxiliarySources { .. }
            | Self::Skill { .. }
            | Self::Mcp { .. }
            | Self::Plugin { .. }
            | Self::DataSource { .. }
            | Self::DocumentSource { .. }
            | Self::Model { .. }
            | Self::ResolveModelReasoning { .. }
            | Self::QueryAutomationOccurrences { .. }
            | Self::ResolveRemoteModelProfile { .. }
            | Self::ResolveChatModel { .. }
            | Self::AdmitHostOutput { .. }
            | Self::PutProductSetting { .. }
            | Self::PutProductSettingChecked { .. }
            | Self::GetProductSetting { .. }
            | Self::ListReadableExpertTemplates { .. }
            | Self::DeleteProductSetting { .. } => ApplicationCommandFamily::CapabilityCatalog,

            Self::Automation { .. }
            | Self::SaveWorkspaceCheckpoint { .. }
            | Self::ListWorkspaceCheckpoints { .. }
            | Self::DeleteWorkspaceCheckpoint { .. }
            | Self::AcquireSessionRunLease { .. }
            | Self::GetSessionRunLease { .. }
            | Self::ListSessionRunLeases
            | Self::ReleaseSessionRunLease { .. }
            | Self::PendingSyncChanges { .. }
            | Self::ResolveSyncChange { .. }
            | Self::GetSyncCursor { .. }
            | Self::SaveSyncCursor { .. }
            | Self::ApplyRemoteSyncPage { .. } => ApplicationCommandFamily::Operations,
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::ApplicationCommand;

    #[test]
    fn rejects_removed_conversation_import_command() {
        let removed_shape = serde_json::json!({
            "type": "import_conversation",
            "input": {
                "id": "conversation-1",
                "title": "Legacy",
                "revision": 1,
                "settings_json": null,
                "messages": [],
                "created_at_ms": 1,
                "updated_at_ms": 1
            }
        });
        assert!(serde_json::from_value::<ApplicationCommand>(removed_shape).is_err());
    }

    #[test]
    fn rejects_messages_inside_create_conversation() {
        let invalid = serde_json::json!({
            "type": "create_conversation",
            "input": {
                "id": "conversation-1",
                "title": "Current",
                "settings_json": null,
                "messages": [],
                "created_at_ms": 1,
                "updated_at_ms": 1
            }
        });
        assert!(serde_json::from_value::<ApplicationCommand>(invalid).is_err());
    }

    #[test]
    fn rejects_full_config_on_an_active_run_submission() {
        let invalid = serde_json::json!({
            "type": "reduce_and_commit_active_submission",
            "input": {
                "conversation_id": "conversation-1",
                "run_id": "run-1",
                "transition_id": "transition-1",
                "invocation_config_digest": "digest-1",
                "core_config": {},
                "submission": {
                    "type": "interrupt",
                    "reason": "user_cancelled"
                },
                "committed_at_ms": 1
            }
        });
        assert!(serde_json::from_value::<ApplicationCommand>(invalid).is_err());
    }

    #[test]
    fn rejects_client_owned_conversation_revision() {
        let invalid = serde_json::json!({
            "type": "create_conversation",
            "input": {
                "id": "conversation-1",
                "title": "Current",
                "revision": 42,
                "settings_json": null,
                "created_at_ms": 1,
                "updated_at_ms": 1
            }
        });
        assert!(serde_json::from_value::<ApplicationCommand>(invalid).is_err());
    }

    #[test]
    fn rejects_removed_store_primitive_commands() {
        for command_type in [
            "begin_turn",
            "commit_transition",
            "accept_host_event",
            "save_checkpoint",
            "delete_checkpoint",
            "pending_effects",
            "acknowledge_effect",
            "apply_conversation_projection",
            "append_messages",
            "reduce_and_commit_idle_projection",
            "begin_remote_turn",
            "open_remote_conversation_turn",
            "commit_remote_transition",
            "commit_remote_conversation_projection",
            "commit_remote_assistant_projection",
            "remote_assistant_projection",
            "save_remote_session_mirror",
            "upsert_skill",
            "list_skills",
            "load_skill_catalog",
            "set_skill_enabled",
            "delete_user_skill",
            "upsert_mcp_server",
            "list_mcp_servers",
            "delete_mcp_server",
            "upsert_data_source",
            "list_data_sources",
            "delete_data_source",
            "set_data_source_override",
            "get_data_source_override",
            "append_data_source_usage",
            "list_data_source_usage",
            "replace_document_sources",
            "list_document_sources",
            "search_document_sources",
            "upsert_memory",
            "save_memory_deduplicated",
            "list_memories",
            "get_memory",
            "memories_by_ids",
            "search_memories",
            "delete_memory",
            "upsert_model_profile",
            "get_model_profile",
            "list_model_profiles",
            "delete_model_profile",
            "report_model_runtime",
            "get_model_runtime",
            "save_conversation_plan",
            "get_conversation_plan",
            "create_runtime_task",
            "load_runtime_task",
            "list_runtime_tasks",
            "list_all_runtime_tasks",
            "claim_runtime_task",
            "heartbeat_runtime_task",
            "retry_runtime_task",
            "finish_runtime_task",
            "fail_expired_runtime_task",
            "recover_expired_runtime_task",
            "append_runtime_task_event",
            "list_runtime_task_events",
            "list_runtime_task_events_after_session_cursor",
            "upsert_automation",
            "reconcile_automations",
            "list_automations",
            "enqueue_automation_run",
            "claim_automation_run",
            "heartbeat_automation_run",
            "retry_automation_run",
            "finish_automation_run",
            "load_automation_run",
            "list_automation_runs",
        ] {
            let removed = serde_json::json!({"type": command_type});
            assert!(
                serde_json::from_value::<ApplicationCommand>(removed).is_err(),
                "removed Store primitive {command_type} must not remain decodable"
            );
        }
    }
}

pub(super) fn encode_application_result<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("encode Application command result failed: {error}"))
}

mod application_command_executor_seal {
    pub trait Sealed {}

    impl<R: crate::repositories::ApplicationCommandRepositories> Sealed for R {}
}

/// Sealed port used by Runtime to dispatch the broad wire command enum without
/// exposing every concrete repository capability as its generic bound.
///
/// Implementations are restricted to the repository value passed as `self`, so
/// command admission and mutation cannot silently use different transactions.
pub trait ApplicationCommandExecutor: application_command_executor_seal::Sealed {
    fn execute_application_command(&mut self, command: ApplicationCommand)
        -> Result<Value, String>;
}

impl<R: crate::repositories::ApplicationCommandRepositories> ApplicationCommandExecutor for R {
    fn execute_application_command(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<Value, String> {
        execute_application_command_from_repositories(self, command)
    }
}

pub fn execute_application_command<R: ApplicationCommandExecutor + ?Sized>(
    store: &mut R,
    command: ApplicationCommand,
) -> Result<Value, String> {
    store.execute_application_command(command)
}

fn execute_application_command_from_repositories<
    R: crate::repositories::ApplicationCommandRepositories,
>(
    store: &mut R,
    command: ApplicationCommand,
) -> Result<Value, String> {
    let command = match command {
        ApplicationCommand::ReconcileRoomRouting {room_id,submission_id,source_host_id,at_ms} => return encode_application_result(crate::agent_room::reconcile_routing(store,&room_id,&submission_id,source_host_id.as_deref(),at_ms)?),
        ApplicationCommand::ResolveEffectiveModelParameters { input } => return encode_application_result(crate::effective_model_parameters::resolve(store, &input)?),
        ApplicationCommand::ListActivity { input } => return encode_application_result(crate::activity::list(store,input)?),
        ApplicationCommand::LoadActivityDetail { conversation_id,run_id,assistant_message_id } => return encode_application_result(crate::activity::detail(store,conversation_id,run_id,assistant_message_id)?),
        ApplicationCommand::LoadTodayFacts { input } => return encode_application_result(crate::load_today_facts(store, input)?),
        ApplicationCommand::LoadEditedResendHostMaterial { conversation_id } => {
            uuid::Uuid::parse_str(&conversation_id).map_err(|_|"edited_resend_invalid_target_identity")?;
            return encode_application_result(nola_store_port::edited_resend::EditedResendRepository::edited_resend_host_material(store,&conversation_id).map_err(|e|e.to_string())?);
        },
        ApplicationCommand::AttestEditedResendHostPolicy { conversation_id, core_config, submission } => {
            return encode_application_result(crate::edited_resend::attest_fresh_policy(store,&conversation_id,&core_config,&submission)?);
        },
        ApplicationCommand::PreviewExpertTemplateCapture { input, authority } => return encode_application_result(crate::expert_message::preview_capture(store, &input, &authority)?),
        ApplicationCommand::AdmitExpertConversationMessage { input, context, captured_at_ms } => return encode_application_result(crate::expert_message::admit(store, input, &context, captured_at_ms)?),
        ApplicationCommand::PrepareEditedExpertResend { input, authority, captured_at_ms } => return encode_application_result(crate::expert_message::prepare_resend(store, input, &authority, captured_at_ms)?),
        ApplicationCommand::LoadPreparedExpertResend { conversation_id, authority } => return encode_application_result(crate::expert_message::load_prepared_authorized(store, &conversation_id, &authority)?),
        ApplicationCommand::LoadExpertMessageManifest { conversation_id, message_id, authority } => return encode_application_result(crate::expert_message::load_manifest_authorized(store, &conversation_id, &message_id, &authority)?),
        ApplicationCommand::LoadExpertChildInstructions { conversation_id, expected_run_id, authority } => return encode_application_result(crate::expert_message::load_child_instructions_authorized(store, &conversation_id, &expected_run_id, &authority)?),
        ApplicationCommand::PrepareEditedResend { input } => return encode_application_result(crate::edited_resend::prepare(store, input)?),
        ApplicationCommand::LoadPreparedEditedResend { conversation_id } => {
            uuid::Uuid::parse_str(&conversation_id).map_err(|_| "edited_resend_invalid_target_identity")?;
            return encode_application_result(nola_store_port::edited_resend::EditedResendRepository::prepared_edited_resend(store, &conversation_id).map_err(|e| e.to_string())?);
        },
        command => command,
    };
    match command.family() {
        ApplicationCommandFamily::Conversation => conversation::execute(store, command),
        ApplicationCommandFamily::AgentRuntime => agent_runtime::execute(store, command),
        ApplicationCommandFamily::CapabilityCatalog => capability_catalog::execute(store, command),
        ApplicationCommandFamily::Operations => operations::execute(store, command),
    }
}

// Consumes the wire command only for embedded/service dispatch. No JSON round
// trip or body clone is needed to cross into the Application implementation.
impl From<nola_protocol::TranscriptCommand> for ApplicationCommand {
    fn from(command: nola_protocol::TranscriptCommand) -> Self {
        match command {
            nola_protocol::TranscriptCommand::AdmitConversationUserMessage { input } => {
                Self::AdmitConversationUserMessage { input }
            }
            nola_protocol::TranscriptCommand::ImportConversationTranscript { input } => {
                Self::ImportConversationTranscript { input }
            }
            nola_protocol::TranscriptCommand::PublishUploadedTranscript { input } => {
                Self::PublishUploadedTranscript { input }
            }
        }
    }
}

impl From<nola_protocol::ExportCommand> for ApplicationCommand {
    fn from(command: nola_protocol::ExportCommand) -> Self {
        match command {
            nola_protocol::ExportCommand::ReadConversationToolSources { owner, expected_conversation_revision } =>
                Self::ReadConversationToolSources { owner, expected_conversation_revision },
            nola_protocol::ExportCommand::ReadConversationToolSourceRange { owner, source_item_id, expected_conversation_revision, offset_bytes, limit_bytes } =>
                Self::ReadConversationToolSourceRange { owner, source_item_id, expected_conversation_revision, offset_bytes, limit_bytes },
            nola_protocol::ExportCommand::ReadConversationPartDerivation {
                owner,
                expected_conversation_revision,
            } => Self::ReadConversationPartDerivation {
                owner,
                expected_conversation_revision,
            },
            nola_protocol::ExportCommand::LoadConversationExportPage {
                conversation_id,
                after_ordinal,
                expected_revision,
            } => Self::LoadConversationExportPage {
                conversation_id,
                after_ordinal,
                expected_revision,
            },
            nola_protocol::ExportCommand::ReadConversationExportRange {
                conversation_id,
                message_id,
                part_id,
                field,
                expected_revision,
                offset_bytes,
                limit_bytes,
            } => Self::ReadConversationExportRange {
                conversation_id,
                message_id,
                part_id,
                field,
                expected_revision,
                offset_bytes,
                limit_bytes,
            },
        }
    }
}

impl From<nola_protocol::ArchiveRestoreCommand> for ApplicationCommand {
    fn from(command: nola_protocol::ArchiveRestoreCommand) -> Self {
        match command {
            nola_protocol::ArchiveRestoreCommand::ArchiveDirectoryRevision { .. } => {
                Self::ArchiveDirectoryRevision {}
            }
            nola_protocol::ArchiveRestoreCommand::BeginArchiveRestore { input } => {
                Self::BeginArchiveRestore { input }
            }
            nola_protocol::ArchiveRestoreCommand::ArchiveRestoreProgress { restore_id } => {
                Self::ArchiveRestoreProgress { restore_id }
            }
            nola_protocol::ArchiveRestoreCommand::StageArchiveManifest { input } => {
                Self::StageArchiveManifest { input }
            }
            nola_protocol::ArchiveRestoreCommand::CommitArchiveRestore { input } => {
                Self::CommitArchiveRestore { input }
            }
        }
    }
}

impl nola_protocol::CommandMetadata for ApplicationCommand {
    fn wire_command_name(&self) -> &'static str {
        match self {
            Self::ArchiveDirectoryRevision { .. } => "archive_directory_revision",
            Self::BeginArchiveRestore { .. } => "begin_archive_restore",
            Self::ArchiveRestoreProgress { .. } => "archive_restore_progress",
            Self::StageArchiveManifest { .. } => "stage_archive_manifest",
            Self::CommitArchiveRestore { .. } => "commit_archive_restore",
            Self::PublishUploadedTranscript { .. } => "publish_uploaded_transcript",
            Self::BeginContentUpload { .. } => "begin_content_upload",
            Self::AppendContentChunk { .. } => "append_content_chunk",
            Self::ContentUploadProgress { .. } => "content_upload_progress",
            Self::SealContentUpload { .. } => "seal_content_upload",
            Self::CancelContentUpload { .. } => "cancel_content_upload",
            Self::ReadContentUploadChunk { .. } => "read_content_upload_chunk",
            Self::PutConversationPdfResource { .. } => "put_conversation_pdf_resource",
            Self::ReadConversationResource { .. } => "read_conversation_resource",
            Self::ListConversationResources { .. } => "list_conversation_resources",
            Self::DeleteConversationResource { .. } => "delete_conversation_resource",
            Self::BeginAssistantTurn { .. } => "begin_assistant_turn",
            Self::PromoteQueuedAssistantTurn { .. } => "promote_queued_assistant_turn",
            Self::BeginQueuedUserTurn { .. } => "begin_queued_user_turn",
            Self::ListOpenUserInputWaits { .. } => "list_open_user_input_waits",
            Self::ListOwnedTaskUserInputWaits { .. } => "list_owned_task_user_input_waits",
            Self::LoadUserInputWait { .. } => "load_user_input_wait",
            Self::RespondToConversationWait { .. } => "respond_to_conversation_wait",
            Self::BeginTaskAssistantTurn { .. } => "begin_task_assistant_turn",
            Self::BeginAutomationAssistantTurn { .. } => "begin_automation_assistant_turn",
            Self::ObserveAutomationExecution { .. } => "observe_automation_execution",
            Self::ListPendingAutomationExecutions { .. } => "list_pending_automation_executions",
            Self::ReconcileAutomationExecution { .. } => "reconcile_automation_execution",
            Self::ObserveAutomationWorker { .. } => "observe_automation_worker",
            Self::AckAutomationNotification { .. } => "ack_automation_notification",
            Self::BeginMaintenanceTurn { .. } => "begin_maintenance_turn",
            Self::QueryClaimableMemoryJobs { .. } => "query_claimable_memory_jobs",
            Self::BuildMemoryWorkerPlan { .. } => "build_memory_worker_plan",
            Self::LoadCommittedHostRequests { .. } => "load_committed_host_requests",
            Self::InspectRunProcessOwners { .. } => "inspect_run_process_owners",
            Self::ValidateRunProcessObservations { .. } => "validate_run_process_observations",
            Self::LoadRunDeliveryOwnership { .. } => "load_run_delivery_ownership",
            Self::LoadRunIdentity { .. } => "load_run_identity",
            Self::StepGoal { .. } => "step_goal",
            Self::StartGoalExecution { .. } => "start_goal_execution",
            Self::PrepareGoalControlIntent { .. } => "prepare_goal_control_intent",
            Self::PrepareGoalExecution { .. } => "prepare_goal_execution",
            Self::LoadGoalExecutionBinding { .. } => "load_goal_execution_binding",
            Self::LoadHostExecutionState { .. } => "load_host_execution_state",
            Self::LoadConversationTaskProjectionV1 { .. } => "load_conversation_task_projection_v1",
            Self::ClaimApplicationControlRequests { .. } => "claim_application_control_requests",
            Self::ImportHostTaskCancellation { .. } => "import_host_task_cancellation",
            Self::PrepareDelegatedTask { .. } => "prepare_delegated_task",
            Self::CaptureMobileDelegationConfiguration { .. } => "capture_mobile_delegation_configuration",
            Self::LoadMobileTaskChildConfiguration { .. } => "load_mobile_task_child_configuration",
            Self::LoadMobileTaskWakePlan { .. } => "load_mobile_task_wake_plan",
            Self::InspectMobilePermissionRequest { .. } => "inspect_mobile_permission_request",
            Self::InspectMobileMcpRequest { .. } => "inspect_mobile_mcp_request",
            Self::PrepareMobileTaskGuidance { .. } => "prepare_mobile_task_guidance",
            Self::ObserveMobileTask { .. } => "observe_mobile_task",
            Self::ApplyPendingTaskNotice { .. } => "apply_pending_task_notice",
            Self::PrepareMobileDelegatedTask { .. } => "prepare_mobile_delegated_task",
            Self::AdmitMobileTaskChild { .. } => "admit_mobile_task_child",
            Self::ExecuteTaskCancellationForEffect { .. } => "execute_task_cancellation_for_effect",
            Self::ClaimCommittedHostRequests { .. } => "claim_committed_host_requests",
            Self::HeartbeatEffectDelivery { .. } => "heartbeat_effect_delivery",
            Self::ResolveUnknownEffectRecovery { .. } => "resolve_unknown_effect_recovery",
            Self::InspectOwnedTaskRecovery { .. } => "inspect_owned_task_recovery",
            Self::ResolveOwnedTaskRecovery { .. } => "resolve_owned_task_recovery",
            Self::LoadRuntimeEffectWorkSchedule { .. } => "load_runtime_effect_work_schedule",
            Self::PrepareRunEffectRecovery { .. } => "prepare_run_effect_recovery",
            Self::ActiveRun { .. } => "active_run",
            Self::ReduceAndAcceptHostResponse { .. } => "reduce_and_accept_host_response",
            Self::ReduceAndCommitActiveSubmission { .. } => "reduce_and_commit_active_submission",
            Self::RequestRunCancellation { .. } => "request_run_cancellation",
            Self::LoadRunCancellationDeliveryFacts { .. } => "load_run_cancellation_delivery_facts",
            Self::ClaimCancellationHostRequests { .. } => "claim_cancellation_host_requests",
            Self::ReduceAndCommitIdleSubmission { .. } => "reduce_and_commit_idle_submission",
            Self::RevertConversation { .. } => "revert_conversation",
            Self::CreateConversationBranch { .. } => "create_conversation_branch",
            Self::PrepareRemoteSubmission { .. } => "prepare_remote_submission",
            Self::AbandonPreparedRemoteSubmission { .. } => "abandon_prepared_remote_submission",
            Self::AdoptObservedRemoteRun { .. } => "adopt_observed_remote_run",
            Self::ObserveSubmissionFrontier { .. } => "observe_submission_frontier",
            Self::ObservedSourceSubmissionReceipt { .. } => "observed_source_submission_receipt",
            Self::BindRemoteSubmissionRequest { .. } => "bind_remote_submission_request",
            Self::ReadRemoteSubmissionIntent { .. } => "read_remote_submission_intent",
            Self::ReadAdmittedRemoteRunIdentity { .. } => "read_admitted_remote_run_identity",
            Self::ReadRemoteSubmissionRequest { .. } => "read_remote_submission_request",
            Self::RegisterSourceSubmission { .. } => "register_source_submission",
            Self::ReadSourceSubmissionReceipt { .. } => "read_source_submission_receipt",
            Self::CancelSourceSubmission { .. } => "cancel_source_submission",
            Self::CancelRejectedQueuedUiAction { .. } => "cancel_rejected_queued_ui_action",
            Self::RejectSourceSubmission { .. } => "reject_source_submission",
            Self::AcceptRemoteSubmissionReceipt { .. } => "accept_remote_submission_receipt",
            Self::CanonicalRunTerminal { .. } => "canonical_run_terminal",
            Self::ReadAssistantJournal { .. } => "read_assistant_journal",
            Self::LocalAssistantJournalHead { .. } => "local_assistant_journal_head",
            Self::LocalRunAssistantJournalHeads { .. } => "local_run_assistant_journal_heads",
            Self::LocalAssistantRunTerminalSeal { .. } => "local_assistant_run_terminal_seal",
            Self::LocalRunMessageAdmissionHead { .. } => "local_run_message_admission_head",
            Self::LocalRunContextUsage { .. } => "local_run_context_usage",
            Self::AcceptRemoteRunContextUsage { .. } => "accept_remote_run_context_usage",
            Self::ReadRunMessageAdmissions { .. } => "read_run_message_admissions",
            Self::RunMessageAdmissionReceptionState { .. } => {
                "run_message_admission_reception_state"
            }
            Self::ReceiveRunMessageAdmissions { .. } => "receive_run_message_admissions",
            Self::BeginReceiveAssistantRunTerminal { .. } => "begin_receive_assistant_run_terminal",
            Self::StageAssistantRunTerminalManifest { .. } => {
                "stage_assistant_run_terminal_manifest"
            }
            Self::CommitReceivedAssistantRunTerminal { .. } => {
                "commit_received_assistant_run_terminal"
            }
            Self::ReadLocalAssistantRunTerminalManifest { .. } => {
                "read_local_assistant_run_terminal_manifest"
            }
            Self::OpenAssistantJournalSnapshot { .. } => "open_assistant_journal_snapshot",
            Self::ReleaseAssistantJournalSnapshot { .. } => "release_assistant_journal_snapshot",
            Self::ReadAssistantJournalRange { .. } => "read_assistant_journal_range",
            Self::ReadAssistantJournalParts { .. } => "read_assistant_journal_parts",
            Self::ReadAssistantJournalPartChanges { .. } => "read_assistant_journal_part_changes",
            Self::ReadAssistantJournalManifest { .. } => "read_assistant_journal_manifest",
            Self::ReadAssistantJournalChunk { .. } => "read_assistant_journal_chunk",
            Self::ReadAssistantJournalRecordChunk { .. } => "read_assistant_journal_record_chunk",
            Self::ReceiveAssistantJournalRecord { .. } => "receive_assistant_journal_record",
            Self::StageAssistantJournalContent { .. } => "stage_assistant_journal_content",
            Self::NextAssistantJournalContent { .. } => "next_assistant_journal_content",
            Self::AssistantJournalReceptionState { .. } => "assistant_journal_reception_state",
            Self::InspectRunRecovery { .. } => "inspect_run_recovery",
            Self::InspectUnknownEffectRecovery { .. } => "inspect_unknown_effect_recovery",
            Self::LoadExactRunInvocation { .. } => "load_exact_run_invocation",
            Self::ValidateRunRecoveryAuthority { .. } => "validate_run_recovery_authority",
            Self::ValidateMobileRunRecoveryAuthority { .. } => "validate_mobile_run_recovery_authority",
            Self::EncodeHostWorkerIdentity { .. } => "encode_host_worker_identity",
            Self::InspectCancellationProcessOwners { .. } => "inspect_cancellation_process_owners",
            Self::ValidateCancellationProcessObservations { .. } => "validate_cancellation_process_observations",
            Self::AuxiliaryInvocation { .. } => "auxiliary_invocation",
            Self::LoadAuxiliaryOperation { .. } => "load_auxiliary_operation",
            Self::LoadAuxiliarySourceSlot { .. } => "load_auxiliary_source_slot",
            Self::ListConversationAuxiliarySources { .. } => "list_conversation_auxiliary_sources",
            Self::ListRecoverableRuns { .. } => "list_recoverable_runs",
            Self::CreateConversation { .. } => "create_conversation",
            Self::AdmitConversationUserMessage { .. } => "admit_conversation_user_message",
            Self::AgentRoom { .. } => "agent_room",
            Self::ImportConversationTranscript { .. } => "import_conversation_transcript",
            Self::EditConversationMessage { .. } => "edit_conversation_message",
            Self::DeleteConversationMessage { .. } => "delete_conversation_message",
            Self::SubmitUiInteraction { .. } => "submit_ui_interaction",
            Self::ExecuteUiTool { .. } => "execute_ui_tool",
            Self::ReadUiSurface { .. } => "read_ui_surface",
            Self::PreviewUiSurface { .. } => "preview_ui_surface",
            Self::ReadUiSurfaceChanges { .. } => "read_ui_surface_changes",
            Self::ReadUiSurfaceRevision { .. } => "read_ui_surface_revision",
            Self::ListUiSurfaces { .. } => "list_ui_surfaces",
            Self::ReadUiAuthority => "read_ui_authority",
            Self::ReadUiCapability => "read_ui_capability",
            Self::StepCompactionSummary { .. } => "step_compaction_summary",
            Self::EvaluateCompactionMaintenance { .. } => "evaluate_compaction_maintenance",
            Self::Context { .. } => "context",
            Self::PatchConversationConfiguration { .. } => "patch_conversation_configuration",
            Self::SelectConversationExecution { .. } => "select_conversation_execution",
            Self::ReadPermissionConfiguration { .. } => "read_permission_configuration",
            Self::SetConversationPermission { .. } => "set_conversation_permission",
            Self::PatchConversationMetadata { .. } => "patch_conversation_metadata",
            Self::LoadConversation { .. } => "load_conversation",
            Self::LoadConversationMetadata { .. } => "load_conversation_metadata",
            Self::LoadConversationExportPage { .. } => "load_conversation_export_page",
            Self::LoadCompactionLineage { .. } => "load_compaction_lineage",
            Self::ReadHistoricalPart { .. } => "read_historical_part",
            Self::ReadConversationPartDerivation { .. } => "read_conversation_part_derivation",
            Self::ReadConversationToolSources { .. } => "read_conversation_tool_sources",
            Self::ReadConversationToolSourceRange { .. } => "read_conversation_tool_source_range",
            Self::ReadCodeCellObservation { .. } => "read_code_cell_observation",
            Self::ReadCodeCellHistoryForEffect { .. } => "read_code_cell_history_for_effect",
            Self::ReadConversationExportRange { .. } => "read_conversation_export_range",
            Self::ReadConversationTextRange { .. } => "read_conversation_text_range",
            Self::SearchConversationText { .. } => "search_conversation_text",
            Self::QueryCanonicalHistory { .. } => "query_canonical_history",
            Self::LoadConversationProjectionV1 { .. } => "load_conversation_projection_v1",
            Self::PreviewExpertTemplateCapture { .. } => "preview_expert_template_capture",
            Self::AdmitExpertConversationMessage { .. } => "admit_expert_conversation_message",
            Self::PrepareEditedExpertResend { .. } => "prepare_edited_expert_resend",
            Self::LoadPreparedExpertResend { .. } => "load_prepared_expert_resend",
            Self::LoadExpertMessageManifest { .. } => "load_expert_message_manifest",
            Self::LoadExpertChildInstructions { .. } => "load_expert_child_instructions",
            Self::PrepareEditedResend { .. } => "prepare_edited_resend",
            Self::LoadPreparedEditedResend { .. } => "load_prepared_edited_resend",
            Self::LoadEditedResendHostMaterial { .. } => "load_edited_resend_host_material",
            Self::AttestEditedResendHostPolicy { .. } => "attest_edited_resend_host_policy",
            Self::ListActivity { .. } => "list_activity",
            Self::LoadActivityDetail { .. } => "load_activity_detail",
            Self::LoadTodayFacts { .. } => "load_today_facts",
            Self::LoadConversationContextUsageV1 { .. } => "load_conversation_context_usage_v1",
            Self::LoadConversationPermissionProjectionV1 { .. } => {
                "load_conversation_permission_projection_v1"
            }
            Self::LoadConversationProjectionPageV1 { .. } => "load_conversation_projection_page_v1",
            Self::LoadConversationSourceOwner { .. } => "load_conversation_source_owner",
            Self::LoadTaskChildConversationWindow { .. } => "load_task_child_conversation_window",
            Self::LoadConversationProjectionWindowV1 { .. } => {
                "load_conversation_projection_window_v1"
            }
            Self::LoadConversationProjectionWindowUpdateV1 { .. } => {
                "load_conversation_projection_window_update_v1"
            }
            Self::LoadConversationTailSnapshotV1 { .. } => "load_conversation_tail_snapshot_v1",
            Self::LoadConversationSettings { .. } => "load_conversation_settings",
            Self::ListConversations { .. } => "list_conversations",
            Self::RenameConversation { .. } => "rename_conversation",
            Self::DeleteConversation { .. } => "delete_conversation",
            Self::UpsertPermissionGrant { .. } => "upsert_permission_grant",
            Self::EffectivePermissionGrants { .. } => "effective_permission_grants",
            Self::ListPermissionGrants { .. } => "list_permission_grants",
            Self::RevokePermissionGrant { .. } => "revoke_permission_grant",
            Self::Skill { .. } => "skill",
            Self::Mcp { .. } => "mcp",
            Self::Plugin { .. } => "plugin",
            Self::DataSource { .. } => "data_source",
            Self::DocumentSource { .. } => "document_source",
            Self::Memory { .. } => "memory",
            Self::Model { .. } => "model",
            Self::ResolveModelReasoning { .. } => "resolve_model_reasoning",
            Self::QueryAutomationOccurrences { .. } => "query_automation_occurrences",
            Self::ResolveRemoteModelProfile { .. } => "resolve_remote_model_profile",
            Self::ReconcileRoomRouting { .. } => "reconcile_room_routing",
            Self::ResolveChatModel { .. } => "resolve_chat_model",
            Self::ResolveEffectiveModelParameters { .. } => "resolve_effective_model_parameters",
            Self::AdmitHostOutput { .. } => "admit_host_output",
            Self::ConversationPlan { .. } => "conversation_plan",
            Self::RuntimeTask { .. } => "runtime_task",
            Self::RecoverExpiredAgentTask { .. } => "recover_expired_agent_task",
            Self::ClaimAgentTaskFair { .. } => "claim_agent_task_fair",
            Self::ExportAgentTaskAdmission { .. } => "export_agent_task_admission",
            Self::ImportHostAgentTaskAdmission { .. } => "import_host_agent_task_admission",
            Self::ExportAgentTaskTerminalProjection { .. } => "export_agent_task_terminal_projection",
            Self::ImportHostAgentTaskTerminalProjection { .. } => "import_host_agent_task_terminal_projection",
            Self::ExportAgentTaskGuidanceProjection { .. } => "export_agent_task_guidance_projection",
            Self::ImportHostAgentTaskGuidanceProjection { .. } => "import_host_agent_task_guidance_projection",
            Self::SopRun { .. } => "sop_run",
            Self::WorkStepTransaction { .. } => "work_step_transaction",
            Self::PermissionRequest { .. } => "permission_request",
            Self::Automation { .. } => "automation",
            Self::EnqueueAgentCommand { .. } => "enqueue_agent_command",
            Self::EnqueueTaskTerminalReconciliation { .. } => {
                "enqueue_task_terminal_reconciliation"
            }
            Self::LoadTaskGuidanceReceipt { .. } => "load_task_guidance_receipt",
            Self::SubmitTaskGuidance { .. } => "submit_task_guidance",
            Self::ApplyTaskGuidance { .. } => "apply_task_guidance",
            Self::ApplyActivePermissionPolicy { .. } => "apply_active_permission_policy",
            Self::PeekAgentCommand { .. } => "peek_agent_command",
            Self::ClaimNextAgentCommand { .. } => "claim_next_agent_command",
            Self::LeasedAgentCommands { .. } => "leased_agent_commands",
            Self::ReleaseAgentCommandLease { .. } => "release_agent_command_lease",
            Self::AcknowledgeAgentCommand { .. } => "acknowledge_agent_command",
            Self::RecordAgentTrace { .. } => "record_agent_trace",
            Self::LoadExactRunTraceReplay { .. } => "load_exact_run_trace_replay",
            Self::TraceEventsAfter { .. } => "trace_events_after",
            Self::TraceRunsForConversation { .. } => "trace_runs_for_conversation",
            Self::LatestTraceId { .. } => "latest_trace_id",
            Self::RecoverOrphanedRuns { .. } => "recover_orphaned_runs",
            Self::RecoverObservedOrphanedRun { .. } => "recover_observed_orphaned_run",
            Self::RecoverExpiredRunDeliveries { .. } => "recover_expired_run_deliveries",
            Self::SaveWorkspaceCheckpoint { .. } => "save_workspace_checkpoint",
            Self::ListWorkspaceCheckpoints { .. } => "list_workspace_checkpoints",
            Self::DeleteWorkspaceCheckpoint { .. } => "delete_workspace_checkpoint",
            Self::AcquireSessionRunLease { .. } => "acquire_session_run_lease",
            Self::GetSessionRunLease { .. } => "get_session_run_lease",
            Self::ListSessionRunLeases => "list_session_run_leases",
            Self::ReleaseSessionRunLease { .. } => "release_session_run_lease",
            Self::PutProductSetting { .. } => "put_product_setting",
            Self::PutProductSettingChecked { .. } => "put_product_setting_checked",
            Self::PersistPreparedHostToolPlan { .. } => "persist_prepared_host_tool_plan",
            Self::ReadPreparedHostToolPlan { .. } => "read_prepared_host_tool_plan",
            Self::GetProductSetting { .. } => "get_product_setting",
            Self::ListReadableExpertTemplates { .. } => "list_readable_expert_templates",
            Self::DeleteProductSetting { .. } => "delete_product_setting",
            Self::PendingSyncChanges { .. } => "pending_sync_changes",
            Self::ResolveSyncChange { .. } => "resolve_sync_change",
            Self::GetSyncCursor { .. } => "get_sync_cursor",
            Self::SaveSyncCursor { .. } => "save_sync_cursor",
            Self::ApplyRemoteSyncPage { .. } => "apply_remote_sync_page",
            Self::LoadCheckpoint { .. } => "load_checkpoint",
            Self::LoadSessionProjection { .. } => "load_session_projection",
            Self::LoadGoalControlObservation { .. } => "load_goal_control_observation",
            Self::LoadGoalExecutionOwner { .. } => "load_goal_execution_owner",
            Self::LoadSessionGoalProjection { .. } => "load_session_goal_projection",
            Self::FindResumableGoalSession { .. } => "find_resumable_goal_session",
            Self::LoadResumeSnapshot { .. } => "load_resume_snapshot",
            Self::AcknowledgeResumeSnapshot { .. } => "acknowledge_resume_snapshot",
        }
    }
}

#[cfg(test)]
mod command_metadata_tests {
    use super::ApplicationCommand;
    use nola_protocol::CommandMetadata;
    #[test]
    fn retired_parent_claim_confirmation_and_boolean_promotion_are_not_commands() {
        for command in [
            serde_json::json!({"type":"read_task_claim_confirmation","conversation_id":"parent",
                "operation_id":"claim","task_id":"task","after_revision":null}),
            serde_json::json!({"type":"runtime_task","command":{"type":"mark_core_promoted",
                "task_id":"task","now_ms":1}}),
        ] {
            assert!(serde_json::from_value::<ApplicationCommand>(command).is_err());
        }
    }
    #[test]
    fn conversation_list_requires_explicit_bounded_selection() {
        assert!(serde_json::from_value::<ApplicationCommand>(serde_json::json!({"type":"list_conversations"})).is_err());
        assert!(serde_json::from_value::<ApplicationCommand>(serde_json::json!({"type":"list_conversations","query":{"limit":128,"offset":0}})).is_err());
        assert!(serde_json::from_value::<ApplicationCommand>(serde_json::json!({"type":"list_conversations","query":{"project_root":null,"non_empty":true,"limit":128,"offset":0}})).is_ok());
    }
    #[test]
    fn diagnostic_names_match_wire_tags() {
        let commands = [
            ApplicationCommand::ExportAgentTaskAdmission {
                conversation_id: "parent".into(), run_id: "run".into(),
                task_id: "task".into(), event_cursor: 1,
            },
            ApplicationCommand::ListConversations { query: nola_store_port::conversation_query::ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 } },
            ApplicationCommand::LoadConversationProjectionV1 {
                conversation_id: "c".into(),
            },
            ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "c".into(),
                title: "Title".into(),
                updated_at_ms: 1,
            },
        ];
        for command in commands {
            assert_eq!(
                serde_json::to_value(&command).unwrap()["type"],
                command.wire_command_name()
            );
        }
    }

    #[test]
    fn source_task_admission_export_is_a_read_only_identity_scoped_command() {
        let command = ApplicationCommand::ExportAgentTaskAdmission {
            conversation_id: "parent".into(), run_id: "exact-run".into(),
            task_id: "exact-task".into(), event_cursor: 7,
        };
        assert!(command.is_read_only_query());
        let encoded = serde_json::to_value(&command).unwrap();
        assert_eq!(encoded["run_id"], "exact-run");
        assert_eq!(encoded["task_id"], "exact-task");
        assert_eq!(encoded["event_cursor"], 7);
        let decoded: ApplicationCommand = serde_json::from_value(encoded).unwrap();
        assert!(decoded.is_read_only_query());
    }
}

impl From<nola_protocol::HistoricalProjectionCommand> for ApplicationCommand {
    fn from(command: nola_protocol::HistoricalProjectionCommand) -> Self {
        match command {
            nola_protocol::HistoricalProjectionCommand::ReadHistoricalPart {
                owner,
                expected_conversation_revision,
                maximum_output_bytes,
            } => Self::ReadHistoricalPart {
                owner,
                expected_conversation_revision,
                maximum_output_bytes,
            },
        }
    }
}
