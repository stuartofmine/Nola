// SPDX-License-Identifier: Apache-2.0
//! Repository contracts grouped by Application command family.
//!
//! A family trait describes one coherent use-case surface. The wire-level
//! dispatcher is the only adapter that needs their union; individual services
//! depend on the smallest family or repository port they actually use.

use nola_store_port::{
    agent_queue::AgentQueueRepository,
    archive_restore::{ArchiveProjectionRepository, ArchiveRestoreRepository},
    automation::AutomationRepositoryProvider,
    consistent_read::ConsistentReadRepository,
    context::ContextRepositoryProvider,
    conversation_history::ConversationProjectionRepository,
    conversation_profile::ConversationWriteRepositoryProvider,
    conversation_query::ConversationQueryRepository,
    conversation_resource::ConversationResourceRepository,
    conversation_revert::ConversationRevertRepository,
    data_source::DataSourceRepositoryProvider,
    document_source::DocumentSourceRepositoryProvider,
    effect_delivery::EffectDeliveryRepository,
    mcp::McpRepositoryProvider,
    plugin::PluginRepositoryProvider,
    memory::{IdleCheckpointRepository, MemoryRepository},
    model::ModelRepositoryProvider,
    permission::PermissionRepository,
    product_setting::ProductSettingRepositoryProvider,
    protocol_journal::CommandProtocolRepositoryProvider,
    protocol_journal::ProjectionJournalRepository,
    remote_session::RemoteSessionRepository,
    replica_sync::ReplicaSyncRepositoryProvider,
    resume_snapshot::ResumeSnapshotRepository,
    run_creation::RunCreationRepository,
    runtime_task::RuntimeTaskRepository,
    skill::SkillRepositoryProvider,
    sop_run::SopRunRepository,
    store_maintenance::StoreMaintenanceRepository,
    trace_journal::TraceJournalRepository,
    transcript_export::TranscriptExportRepository,
    transcript_publication::TranscriptPublicationRepository,
    work_step_transaction::WorkStepTransactionRepository,
    workspace_ownership::WorkspaceOwnershipRepositoryProvider,
    TranscriptReadRepository,
};

/// Canonical execution and its public journal share one transaction owner.
pub trait AssistantPublicationRepositories:
    nola_store_port::runtime_task::TaskAuthorityReadRepository
    + nola_store_port::remote_session::CanonicalRunTerminalRepository
    + nola_store_port::assistant_journal::AssistantJournalRepository
    + nola_store_port::code_cell::CodeCellObservationRepository
    + nola_store_port::trace_journal::TraceJournalRepository
    + nola_store_port::assistant_ownership::AssistantJournalPlanningRepository
    + nola_store_port::run_effect::RunEffectRepository
    + CommandProtocolRepositoryProvider
{}
impl<T: ?Sized> AssistantPublicationRepositories for T where
    T: nola_store_port::runtime_task::TaskAuthorityReadRepository
        + nola_store_port::remote_session::CanonicalRunTerminalRepository
        + nola_store_port::assistant_journal::AssistantJournalRepository
        + nola_store_port::code_cell::CodeCellObservationRepository
        + nola_store_port::trace_journal::TraceJournalRepository
        + nola_store_port::assistant_ownership::AssistantJournalPlanningRepository
        + nola_store_port::run_effect::RunEffectRepository
        + CommandProtocolRepositoryProvider
{}

/// New turn admission validates frozen Goal execution and selected model authority.
pub trait AssistantAdmissionRepositories: AssistantPublicationRepositories
    + nola_store_port::goal_execution::GoalExecutionRepository + ModelRepositoryProvider
    + nola_store_port::product_setting::ProductSettingRepositoryProvider
    + nola_store_port::conversation_profile::ConversationWriteRepositoryProvider
    + nola_store_port::conversation_query::ConversationQueryRepository
    + nola_store_port::conversation_history::ConversationProjectionRepository
    + nola_store_port::room_presentation::RoomPresentationRepository {}
impl<T: ?Sized> AssistantAdmissionRepositories for T where T: AssistantPublicationRepositories
    + nola_store_port::goal_execution::GoalExecutionRepository + ModelRepositoryProvider
    + nola_store_port::product_setting::ProductSettingRepositoryProvider
    + nola_store_port::conversation_profile::ConversationWriteRepositoryProvider
    + nola_store_port::conversation_query::ConversationQueryRepository
    + nola_store_port::conversation_history::ConversationProjectionRepository
    + nola_store_port::room_presentation::RoomPresentationRepository {}

/// Canonical conversation, archive, transcript, and projection commands.
pub trait ConversationCommandRepositories:
    nola_store_port::room_presentation::RoomPresentationRepository
    + ConversationWriteRepositoryProvider
    + nola_store_port::runtime_task::RuntimeTaskRepository
    + ModelRepositoryProvider
    + nola_store_port::expert_message::ExpertMessageRepository
    + nola_store_port::prepared_host_tool_plan::PreparedHostToolPlanRepository
    + nola_store_port::product_setting::ProductSettingRepositoryProvider
    + nola_store_port::code_cell::CodeCellObservationRepository
    + AgentQueueRepository
    + nola_store_port::run_effect::RunEffectRepository
    + nola_store_port::run_message_admission::RunMessageAdmissionRepository
    + CommandProtocolRepositoryProvider
    + nola_store_port::assistant_journal::AssistantJournalRepository
    + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
    + ConversationResourceRepository
    + ConversationRevertRepository
    + nola_store_port::trace_journal::TraceJournalRepository
    + RemoteSessionRepository
    + ArchiveRestoreRepository
    + ArchiveProjectionRepository
    + TranscriptPublicationRepository
    + ConversationQueryRepository
    + TranscriptReadRepository
    + TranscriptExportRepository
    + ConversationProjectionRepository
    + ConsistentReadRepository
    + PermissionRepository
{
}

impl<T: ?Sized> ConversationCommandRepositories for T where
    T: nola_store_port::room_presentation::RoomPresentationRepository
        + ConversationWriteRepositoryProvider
        + nola_store_port::runtime_task::RuntimeTaskRepository
        + ModelRepositoryProvider
        + nola_store_port::expert_message::ExpertMessageRepository
        + nola_store_port::prepared_host_tool_plan::PreparedHostToolPlanRepository
        + nola_store_port::product_setting::ProductSettingRepositoryProvider
        + nola_store_port::code_cell::CodeCellObservationRepository
        + AgentQueueRepository
        + nola_store_port::run_effect::RunEffectRepository
        + nola_store_port::run_message_admission::RunMessageAdmissionRepository
        + CommandProtocolRepositoryProvider
        + nola_store_port::assistant_journal::AssistantJournalRepository
        + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
        + ConversationResourceRepository
        + ConversationRevertRepository
        + nola_store_port::trace_journal::TraceJournalRepository
        + RemoteSessionRepository
        + ArchiveRestoreRepository
        + ArchiveProjectionRepository
        + TranscriptPublicationRepository
        + ConversationQueryRepository
        + TranscriptReadRepository
        + TranscriptExportRepository
        + ConversationProjectionRepository
        + ConsistentReadRepository
        + PermissionRepository
{
}

/// Agent execution, recovery, permission, memory, and durable work commands.
pub trait AgentRuntimeCommandRepositories:
    SopRunRepository
    + AutomationRepositoryProvider
    + nola_store_port::user_input_wait::UserInputWaitRepository
    + AssistantAdmissionRepositories
    + nola_store_port::cancellation_delivery::CancellationDeliveryRepository
    + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
    + ResumeSnapshotRepository
    + WorkStepTransactionRepository
    + PermissionRepository
    + MemoryRepository
    + IdleCheckpointRepository
    + RunCreationRepository
    + AgentQueueRepository
    + TraceJournalRepository
    + RuntimeTaskRepository
    + nola_store_port::goal_execution::GoalExecutionRepository
    + nola_store_port::run_message_admission::RunMessageAdmissionRepository
    + EffectDeliveryRepository
    + StoreMaintenanceRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + ConsistentReadRepository
    + RemoteSessionRepository
{
}

impl<T: ?Sized> AgentRuntimeCommandRepositories for T where
    T: SopRunRepository
        + AutomationRepositoryProvider
        + nola_store_port::user_input_wait::UserInputWaitRepository
        + AssistantAdmissionRepositories
        + nola_store_port::cancellation_delivery::CancellationDeliveryRepository
        + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
        + ResumeSnapshotRepository
        + WorkStepTransactionRepository
        + PermissionRepository
        + MemoryRepository
        + IdleCheckpointRepository
        + RunCreationRepository
        + AgentQueueRepository
        + TraceJournalRepository
        + RuntimeTaskRepository
        + nola_store_port::goal_execution::GoalExecutionRepository
    + nola_store_port::run_message_admission::RunMessageAdmissionRepository
        + EffectDeliveryRepository
        + StoreMaintenanceRepository
        + nola_store_port::conversation_history::CanonicalHistoryRepository
        + ConsistentReadRepository
        + RemoteSessionRepository
{
}

/// User-configurable capability catalogs and provider metadata.
pub trait CapabilityCatalogRepositories:
    SkillRepositoryProvider
    + nola_store_port::run_effect::RunEffectRepository
    + nola_store_port::effect_delivery::EffectDeliveryRepository
    + AgentQueueRepository
    + nola_store_port::generative_ui::GenerativeUiRepository
    + ProjectionJournalRepository
    + nola_store_port::prepared_host_tool_plan::PreparedHostToolPlanRepository
    + nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository
    + DataSourceRepositoryProvider
    + ProductSettingRepositoryProvider
    + ModelRepositoryProvider
    + McpRepositoryProvider
    + PluginRepositoryProvider
    + ContextRepositoryProvider
    + DocumentSourceRepositoryProvider
{
}

impl<T: ?Sized> CapabilityCatalogRepositories for T where
    T: SkillRepositoryProvider
        + nola_store_port::run_effect::RunEffectRepository
    + nola_store_port::effect_delivery::EffectDeliveryRepository
    + AgentQueueRepository
    + nola_store_port::generative_ui::GenerativeUiRepository
    + ProjectionJournalRepository
    + nola_store_port::prepared_host_tool_plan::PreparedHostToolPlanRepository
        + nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository
        + DataSourceRepositoryProvider
        + ProductSettingRepositoryProvider
        + ModelRepositoryProvider
        + McpRepositoryProvider
        + PluginRepositoryProvider
        + ContextRepositoryProvider
        + DocumentSourceRepositoryProvider
{
}

/// Background automation, workspace ownership, and replica synchronization.
pub trait OperationsCommandRepositories:
    AutomationRepositoryProvider + WorkspaceOwnershipRepositoryProvider + ReplicaSyncRepositoryProvider
{
}

impl<T: ?Sized> OperationsCommandRepositories for T where
    T: AutomationRepositoryProvider
        + WorkspaceOwnershipRepositoryProvider
        + ReplicaSyncRepositoryProvider
{
}

/// Durable command admission and transaction ownership.
pub trait CommandProtocolRepositories: CommandProtocolRepositoryProvider {}

impl<T: ?Sized> CommandProtocolRepositories for T where T: CommandProtocolRepositoryProvider {}

/// Full repository union required only by the wire-level command dispatcher.
pub trait ApplicationCommandRepositories:
    ConversationCommandRepositories
    + AgentRuntimeCommandRepositories
    + CapabilityCatalogRepositories
    + OperationsCommandRepositories
    + nola_store_port::expert_message::ExpertMessageRepository
    + nola_store_port::edited_resend::EditedResendRepository
    + nola_store_port::activity::ActivityRepository
    + nola_store_port::automation_facts::AutomationFactsRepository
    + nola_store_port::room_presentation::RoomPresentationRepository
    + nola_store_port::user_input_wait::UserInputWaitRepository
{
}

impl<T: ?Sized> ApplicationCommandRepositories for T where
    T: ConversationCommandRepositories
        + AgentRuntimeCommandRepositories
        + CapabilityCatalogRepositories
        + OperationsCommandRepositories
        + nola_store_port::expert_message::ExpertMessageRepository
        + nola_store_port::edited_resend::EditedResendRepository
        + nola_store_port::activity::ActivityRepository
    + nola_store_port::automation_facts::AutomationFactsRepository
        + nola_store_port::room_presentation::RoomPresentationRepository
        + nola_store_port::user_input_wait::UserInputWaitRepository
{
}

/// Runtime-owned command execution, including hot Run state and upload staging.
pub trait RuntimeExecutionRepositories:
    crate::application_command::ApplicationCommandExecutor
    + ConversationProjectionRepository
    + ConsistentReadRepository
    + nola_store_port::cancellation_delivery::CancellationDeliveryRepository
    + AutomationRepositoryProvider
    + nola_store_port::user_input_wait::UserInputWaitRepository
    + RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + AssistantAdmissionRepositories
    + IdleCheckpointRepository
    + RuntimeTaskRepository
    + RemoteSessionRepository
    + AgentQueueRepository
    + CommandProtocolRepositoryProvider
    + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
    + nola_store_port::run_effect::RunEffectRepository
    + EffectDeliveryRepository
    + StoreMaintenanceRepository
    + ArchiveRestoreRepository
    + TranscriptPublicationRepository
    + ConversationQueryRepository
{
}

impl<T: ?Sized> RuntimeExecutionRepositories for T where
    T: crate::application_command::ApplicationCommandExecutor
        + ConversationProjectionRepository
        + ConsistentReadRepository
        + nola_store_port::cancellation_delivery::CancellationDeliveryRepository
        + AutomationRepositoryProvider
        + nola_store_port::user_input_wait::UserInputWaitRepository
        + RunCreationRepository
        + nola_store_port::conversation_history::CanonicalHistoryRepository
        + AssistantAdmissionRepositories
        + IdleCheckpointRepository
        + RuntimeTaskRepository
        + RemoteSessionRepository
        + AgentQueueRepository
        + CommandProtocolRepositoryProvider
        + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
        + nola_store_port::run_effect::RunEffectRepository
        + EffectDeliveryRepository
        + StoreMaintenanceRepository
        + ArchiveRestoreRepository
        + TranscriptPublicationRepository
        + ConversationQueryRepository
{
}

/// Command-envelope admission, mutation, revision, and projection journaling.
pub trait ReceivedCommandRepositories:
    RuntimeExecutionRepositories
    + nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository
    + CommandProtocolRepositories
    + ArchiveProjectionRepository
    + ConversationProjectionRepository
    + ProjectionJournalRepository
{
}

impl<T: ?Sized> ReceivedCommandRepositories for T where
    T: RuntimeExecutionRepositories
        + nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository
        + CommandProtocolRepositories
        + ArchiveProjectionRepository
        + ConversationProjectionRepository
        + ProjectionJournalRepository
{
}
