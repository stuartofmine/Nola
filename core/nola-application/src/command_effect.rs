// SPDX-License-Identifier: Apache-2.0
//! Typed transport admission semantics. New/unclassified commands require a
//! durable receipt; neither command names nor client flags authorize writes.
use crate::*;

impl ApplicationCommand {
    pub fn is_read_only_query(&self) -> bool {
        if let Self::AgentRoom {command}=self {return command.is_read_only_query();}
        if let Self::AgentRoomAtSource {command,..}=self {return command.is_read_only_query();}
        if let Self::ExpertPeerTaskAction { input, .. } = self {
            return matches!(&input.request.payload, nola_agent_engine::agent::host::HostRequest::ExecuteTool { input, .. } if matches!(input.get("action").and_then(serde_json::Value::as_str), Some("directory"|"read"|"wait")));
        }
        matches!(
            self,
            Self::ExportRemoteExpertParentAdmission { .. }
                | Self::ReadUiSurface { .. }
                | Self::ReadUiSurfaceChanges { .. }
                | Self::PreviewUiSurface { .. }
                | Self::ReadUiSurfaceRevision { .. }
                | Self::ListUiSurfaces { .. }
                | Self::ReadUiAuthority
                | Self::ReadUiCapability
                | Self::ArchiveDirectoryRevision { .. }
                | Self::StepCompactionSummary { .. }
                | Self::EvaluateCompactionMaintenance { .. }
                | Self::AgentRoom { command: crate::agent_room::AgentRoomCommand::Directory | crate::agent_room::AgentRoomCommand::ChildConversations { .. } | crate::agent_room::AgentRoomCommand::PeerExchanges { .. } | crate::agent_room::AgentRoomCommand::ChildHistory { .. } | crate::agent_room::AgentRoomCommand::RoleTemplates | crate::agent_room::AgentRoomCommand::Read { .. } | crate::agent_room::AgentRoomCommand::List { .. } | crate::agent_room::AgentRoomCommand::Submission { .. } | crate::agent_room::AgentRoomCommand::Submissions { .. } | crate::agent_room::AgentRoomCommand::ExecutionIntent { .. } | crate::agent_room::AgentRoomCommand::ForConversation { .. } }
                | Self::AuxiliaryInvocation { command: nola_domain::AuxiliaryInvocationCommand::Inspect { .. } }
                | Self::AuxiliaryInvocation { command: nola_domain::AuxiliaryInvocationCommand::InspectSource { .. } }
                | Self::LoadAuxiliarySourceSlot { .. }
                | Self::LoadAuxiliaryOperation { .. }
                | Self::ListConversationAuxiliarySources { .. }
                | Self::ObserveAutomationExecution { .. }
                | Self::ListPendingAutomationExecutions { .. }
                | Self::ListOpenUserInputWaits { .. }
                | Self::LoadUserInputWait { .. }
                | Self::ListOwnedTaskUserInputWaits { .. }
                | Self::ResolveModelReasoning { .. }
                | Self::ResolveRemoteModelProfile { .. }
                | Self::QueryAutomationOccurrences { .. }
                | Self::ResolveChatModel { .. }
                | Self::ResolveEffectiveModelParameters { .. }
                | Self::AdmitHostOutput { .. }
                | Self::ArchiveRestoreProgress { .. }
                | Self::ContentUploadProgress { .. }
                | Self::ReadContentUploadChunk { .. }
                | Self::ReadConversationResource { .. }
                | Self::ListConversationResources { .. }
                | Self::QueryClaimableMemoryJobs { .. }
                | Self::BuildMemoryWorkerPlan { .. }
                | Self::LoadCommittedHostRequests { .. }
                | Self::InspectRunProcessOwners { .. }
                | Self::ValidateRunProcessObservations { .. }
                | Self::LoadRunDeliveryOwnership { .. }
                | Self::LoadRunIdentity { .. }
                | Self::LoadGoalExecutionBinding { .. }
                | Self::PrepareGoalExecution { .. }
                | Self::LoadHostExecutionState { .. }
                | Self::LoadConversationTaskProjectionV1 { .. }
                | Self::LoadRunCancellationDeliveryFacts { .. }
                | Self::LoadRuntimeEffectWorkSchedule { .. }
                | Self::ActiveRun { .. }
                | Self::CanonicalRunTerminal { .. }
                | Self::ReadAssistantJournal { .. }
                | Self::LocalAssistantJournalHead { .. }
                | Self::LocalRunAssistantJournalHeads { .. }
                | Self::LocalAssistantRunTerminalSeal { .. }
                | Self::LocalRunMessageAdmissionHead { .. }
                | Self::ReadRunMessageAdmissions { .. }
                | Self::RunMessageAdmissionReceptionState { .. }
                | Self::ReadLocalAssistantRunTerminalManifest { .. }
                | Self::ReadAssistantJournalRange { .. }
                | Self::ReadAssistantJournalParts { .. }
                | Self::ReadAssistantJournalPartChanges { .. }
                | Self::ReadAssistantJournalManifest { .. }
                | Self::ReadAssistantJournalChunk { .. }
                | Self::ReadAssistantJournalRecordChunk { .. }
                | Self::NextAssistantJournalContent { .. }
                | Self::AssistantJournalReceptionState { .. }
                | Self::InspectRunRecovery { .. }
                | Self::ReadPermissionConfiguration { .. }
                | Self::InspectUnknownEffectRecovery { .. }
                | Self::InspectOwnedTaskRecovery { .. }
                | Self::LoadExactRunInvocation { .. }
                | Self::ValidateRunRecoveryAuthority { .. }
                | Self::ValidateMobileRunRecoveryAuthority { .. }
                | Self::EncodeHostWorkerIdentity { .. }
                | Self::InspectCancellationProcessOwners { .. }
                | Self::ValidateCancellationProcessObservations { .. }
                | Self::ListRecoverableRuns { .. }
                | Self::LoadConversation { .. }
                | Self::LoadConversationMetadata { .. }
                | Self::LoadConversationExportPage { .. }
                | Self::LoadCompactionLineage { .. }
                | Self::ReadHistoricalPart { .. }
                | Self::ReadConversationPartDerivation { .. }
                | Self::ReadConversationToolSources { .. }
                | Self::ReadConversationToolSourceRange { .. }
                | Self::ReadCodeCellObservation { .. }
                | Self::ReadCodeCellHistoryForEffect { .. }
                | Self::ReadConversationExportRange { .. }
                | Self::ReadConversationTextRange { .. }
                | Self::SearchConversationText { .. }
                | Self::QueryCanonicalHistory { .. }
                | Self::LoadConversationProjectionV1 { .. }
                | Self::LoadExpertPeerForEffect{..} | Self::ReadExpertExchangeCapability | Self::DeriveExpertPeerAuthority{..} | Self::ExpertPeerDirectory { .. } | Self::PreviewExpertPeer { .. } | Self::LoadExpertPeer { .. } | Self::LoadExpertPeerForSubmission { .. }
                | Self::PreviewExpertTemplateCapture { .. }
                | Self::LoadPreparedExpertResend { .. }
                | Self::LoadExpertMessageManifest { .. }
                | Self::LoadExpertChildInstructions { .. }
                | Self::LoadPreparedEditedResend { .. }
                | Self::LoadEditedResendHostMaterial { .. }
                | Self::AttestEditedResendHostPolicy { .. }
                | Self::LoadTodayFacts { .. }
                | Self::LoadConversationContextUsageV1 { .. }
                | Self::LocalRunContextUsage { .. }
                | Self::LoadConversationPermissionProjectionV1 { .. }
                | Self::LoadConversationProjectionPageV1 { .. }
                | Self::LoadConversationProjectionWindowV1 { .. }
                | Self::LoadConversationProjectionWindowUpdateV1 { .. }
                | Self::LoadConversationTailSnapshotV1 { .. }
                | Self::LoadConversationSettings { .. }
                | Self::ListRootConversations { .. }
                | Self::ListConversations { .. }
                | Self::EffectivePermissionGrants { .. }
                | Self::ListPermissionGrants { .. }
                | Self::PeekAgentCommand { .. }
                | Self::LeasedAgentCommands { .. }
                | Self::TraceEventsAfter { .. }
                | Self::TraceRunsForConversation { .. }
                | Self::LatestTraceId { .. }
                | Self::ListWorkspaceCheckpoints { .. }
                | Self::GetSessionRunLease { .. }
                | Self::ListSessionRunLeases
                | Self::GetProductSetting { .. }
                | Self::ListReadableExpertTemplates { .. }
                | Self::ReadPreparedHostToolPlan { .. }
                | Self::PendingSyncChanges { .. }
                | Self::GetSyncCursor { .. }
                | Self::LoadCheckpoint { .. }
                | Self::LoadConversationSourceOwner { .. }
                | Self::LoadSessionProjection { .. }
                | Self::LoadGoalControlObservation { .. }
                | Self::LoadGoalExecutionOwner { .. }
                | Self::LoadSessionGoalProjection { .. }
                | Self::ReadRemoteSubmissionIntent { .. }
                | Self::ReadRemoteSubmissionRequest { .. }
                | Self::ReadSourceSubmissionReceipt { .. }
                | Self::ObserveSubmissionFrontier { .. }
                | Self::ObservedSourceSubmissionReceipt { .. }
                | Self::CaptureMobileDelegationConfiguration { .. }
                | Self::LoadMobileTaskChildConfiguration { .. }
                | Self::LoadMobileTaskWakePlan { .. }
                | Self::InspectMobilePermissionRequest { .. }
                | Self::InspectMobileMcpRequest { .. }
                | Self::ExportAgentTaskAdmission { .. }
                | Self::ExportAgentTaskTerminalProjection { .. }
                | Self::ExportAgentTaskGuidanceProjection { .. }
                | Self::LoadTaskGuidanceReceipt { .. }
                | Self::FindResumableGoalSession { .. }
                | Self::LoadResumeSnapshot { .. }
                | Self::Context {
                    command: ContextCommand::CountCheckpoints { .. }
                        | ContextCommand::PrepareCompaction { .. }
                        | ContextCommand::PrepareMaintenance { .. }
                        | ContextCommand::LoadCheckpoint { .. }
                }
                | Self::Skill {
                    command: SkillCommand::List
                        | SkillCommand::ListEditors
                        | SkillCommand::GetEditor { .. }
                        | SkillCommand::LoadCatalog
                }
                | Self::Mcp {
                    command: McpCommand::List
                }
                | Self::Plugin {
                    command: PluginCommand::Get { .. } | PluginCommand::List { .. }
                }
                | Self::DataSource {
                    command: DataSourceCommand::Get { .. }
                        | DataSourceCommand::List
                        | DataSourceCommand::ResolveForConversation { .. }
                        | DataSourceCommand::GetConversationOverride { .. }
                        | DataSourceCommand::ListUsage { .. }
                }
                | Self::DocumentSource {
                    command: DocumentSourceCommand::List { .. }
                        | DocumentSourceCommand::Search { .. }
                }
                | Self::Memory {
                    command: MemoryCommand::List { .. }
                        | MemoryCommand::Get { .. }
                        | MemoryCommand::GetMany { .. }
                        | MemoryCommand::UsageStats { .. }
                        | MemoryCommand::Recall { .. }
                }
                | Self::Model {
                    command: ModelCommand::GetProfile { .. }
                        | ModelCommand::ListProfiles
                        | ModelCommand::GetRuntime { .. }
                        | ModelCommand::GetConnection { .. }
                        | ModelCommand::ListConnections
                        | ModelCommand::GetModelProbeAttempt { .. }
                        | ModelCommand::GetAccount { .. }
                        | ModelCommand::ListAccounts
                        | ModelCommand::ListGatewayAttempts { .. }
                }
                | Self::ConversationPlan {
                    command: ConversationPlanCommand::Get { .. }
                        | ConversationPlanCommand::History { .. }
                        | ConversationPlanCommand::Diff { .. }
                }
                | Self::RuntimeTask {
                    command: RuntimeTaskCommand::ReadOwnedCancellation { .. }
                        | RuntimeTaskCommand::Get { .. }
                        | RuntimeTaskCommand::ListProject { .. }
                        | RuntimeTaskCommand::ListAll
                        | RuntimeTaskCommand::LatestEventCursor
                        | RuntimeTaskCommand::AttemptRetiredForUserInput { .. }
                        | RuntimeTaskCommand::ListEvents { .. }
                        | RuntimeTaskCommand::EventsAfterSessionCursor { .. }
                        | RuntimeTaskCommand::PendingPermissionSuspension { .. }
                        | RuntimeTaskCommand::LatestPermissionSuspension { .. }
                        | RuntimeTaskCommand::Attempt { .. }
                        | RuntimeTaskCommand::CurrentAttempt { .. }
                        | RuntimeTaskCommand::AgentAuthority { .. }
                        | RuntimeTaskCommand::PreparedAgentTaskIds { .. }
                        | RuntimeTaskCommand::SourceAgentTaskAdmissionLocation { .. }
                        | RuntimeTaskCommand::PendingCommandReconciliation { .. }
                        | RuntimeTaskCommand::PermissionResume { .. }
                        | RuntimeTaskCommand::PermissionContinuationGrant { .. }
                        | RuntimeTaskCommand::RunBinding { .. }
                        | RuntimeTaskCommand::IsChildSession { .. }
                        | RuntimeTaskCommand::TerminalReceipt { .. }
                        | RuntimeTaskCommand::NotificationDelivered { .. }
                        | RuntimeTaskCommand::PendingTaskGuidance { .. }
                }
                | Self::SopRun {
                    command: SopRunCommand::Load { .. }
                        | SopRunCommand::ValidateInvocation { .. }
                        | SopRunCommand::ValidatePermissionInvocation { .. }
                        | SopRunCommand::List { .. }
                        | SopRunCommand::VisibleActive { .. }
                }
                | Self::WorkStepTransaction {
                    command: WorkStepTransactionCommand::Get { .. }
                        | WorkStepTransactionCommand::ListIncomplete { .. }
                }
                | Self::PermissionRequest {
                    command: PermissionRequestCommand::Get { .. }
                        | PermissionRequestCommand::PendingForTask { .. }
                        | PermissionRequestCommand::PendingForOwner { .. }
                }
                | Self::Automation {
                    command: AutomationCommand::Definition {
                        command: AutomationDefinitionCommand::List | AutomationDefinitionCommand::Get { .. }
                    } | AutomationCommand::Run {
                        command: AutomationRunCommand::Load { .. }
                            | AutomationRunCommand::List { .. }
                    }
                }
        )
    }

    /// Returns `true` only for Tier 1 read-only queries whose execution in
    /// `ApplicationRuntime::execute` is strictly supervisor-independent and
    /// upload-staging-independent, reading solely from the committed `NolaStore`.
    ///
    /// Commands that consult `RuntimeSupervisor` (hot active runs, deferred
    /// streaming deltas, permission projections, cancellation delivery facts,
    /// goal/host execution state) or `content_upload_store` return `false` and
    /// always serialize through the writer runtime.
    pub fn can_execute_on_read_replica(&self) -> bool {
        if !self.is_read_only_query() {
            return false;
        }
        matches!(
            self,
            Self::ListConversations { .. }
                | Self::ListRootConversations { .. }
                | Self::LoadConversation { .. }
                | Self::LoadConversationMetadata { .. }
                | Self::LoadConversationSettings { .. }
                | Self::LoadConversationProjectionV1 { .. }
                | Self::LoadConversationProjectionPageV1 { .. }
                | Self::LoadConversationProjectionWindowV1 { .. }
                | Self::LoadConversationProjectionWindowUpdateV1 { .. }
                | Self::LoadConversationTailSnapshotV1 { .. }
                | Self::LoadConversationTaskProjectionV1 { .. }
                | Self::LoadConversationContextUsageV1 { .. }
                | Self::LocalRunContextUsage { .. }
                | Self::LoadConversationExportPage { .. }
                | Self::LoadCompactionLineage { .. }
                | Self::ReadHistoricalPart { .. }
                | Self::ReadConversationPartDerivation { .. }
                | Self::ReadConversationToolSources { .. }
                | Self::ReadConversationToolSourceRange { .. }
                | Self::ReadCodeCellObservation { .. }
                | Self::ReadCodeCellHistoryForEffect { .. }
                | Self::ReadConversationExportRange { .. }
                | Self::ReadConversationTextRange { .. }
                | Self::SearchConversationText { .. }
                | Self::QueryCanonicalHistory { .. }
                | Self::ReadConversationResource { .. }
                | Self::ListConversationResources { .. }
                | Self::ReadAssistantJournal { .. }
                | Self::LocalAssistantJournalHead { .. }
                | Self::LocalRunAssistantJournalHeads { .. }
                | Self::LocalAssistantRunTerminalSeal { .. }
                | Self::LocalRunMessageAdmissionHead { .. }
                | Self::ReadRunMessageAdmissions { .. }
                | Self::RunMessageAdmissionReceptionState { .. }
                | Self::ReadLocalAssistantRunTerminalManifest { .. }
                | Self::ReadAssistantJournalRange { .. }
                | Self::ReadAssistantJournalParts { .. }
                | Self::ReadAssistantJournalPartChanges { .. }
                | Self::ReadAssistantJournalManifest { .. }
                | Self::ReadAssistantJournalChunk { .. }
                | Self::ReadAssistantJournalRecordChunk { .. }
                | Self::NextAssistantJournalContent { .. }
                | Self::AssistantJournalReceptionState { .. }
                | Self::ReadUiSurface { .. }
                | Self::ReadUiSurfaceChanges { .. }
                | Self::PreviewUiSurface { .. }
                | Self::ReadUiSurfaceRevision { .. }
                | Self::ListUiSurfaces { .. }
                | Self::ReadUiAuthority
                | Self::ReadUiCapability
                | Self::GetProductSetting { .. }
                | Self::LoadTodayFacts { .. }
                | Self::ActiveRun { .. }
                | Self::CanonicalRunTerminal { .. }
                | Self::ListOpenUserInputWaits { .. }
                | Self::LoadUserInputWait { .. }
                | Self::ListOwnedTaskUserInputWaits { .. }
                | Self::EffectivePermissionGrants { .. }
                | Self::ListPermissionGrants { .. }
                | Self::ListReadableExpertTemplates { .. }
                | Self::ResolveModelReasoning { .. }
                | Self::ResolveRemoteModelProfile { .. }
                | Self::QueryAutomationOccurrences { .. }
                | Self::ResolveChatModel { .. }
                | Self::ResolveEffectiveModelParameters { .. }
                | Self::LoadAuxiliarySourceSlot { .. }
                | Self::LoadAuxiliaryOperation { .. }
                | Self::ListConversationAuxiliarySources { .. }
                | Self::AuxiliaryInvocation { .. }
                | Self::AgentRoom { .. }
                | Self::Skill { .. }
                | Self::Mcp { .. }
                | Self::Plugin { .. }
                | Self::DataSource { .. }
                | Self::DocumentSource { .. }
                | Self::Memory { .. }
                | Self::Model { .. }
                | Self::ConversationPlan { .. }
                | Self::RuntimeTask { .. }
                | Self::SopRun { .. }
                | Self::WorkStepTransaction { .. }
                | Self::PermissionRequest { .. }
                | Self::Automation { .. }
                | Self::ListWorkspaceCheckpoints { .. }
                | Self::PendingSyncChanges { .. }
                | Self::GetSyncCursor { .. }
                | Self::ArchiveDirectoryRevision { .. }
                | Self::ArchiveRestoreProgress { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_automation_readback_is_read_only() {
        let command = ApplicationCommand::Automation {
            command: AutomationCommand::Definition {
                command: AutomationDefinitionCommand::Get { id: "automation".into() },
            },
        };
        assert!(command.is_read_only_query());
        let roundtrip: ApplicationCommand = serde_json::from_value(serde_json::to_value(command).unwrap()).unwrap();
        assert!(roundtrip.is_read_only_query());
    }

    #[test]
    fn permission_recovery_queries_do_not_admit_write_receipts() {
        for command in [
            RuntimeTaskCommand::PendingPermissionSuspension { task_id: "task".into() },
            RuntimeTaskCommand::PendingCommandReconciliation { task_id: "task".into() },
            RuntimeTaskCommand::PermissionResume {
                task_id: "task".into(), suspension_id: "suspension".into(),
                permission_request_id: "permission".into(), continuation_id: "continuation".into(),
            },
        ] {
            let encoded = serde_json::to_value(ApplicationCommand::RuntimeTask { command }).unwrap();
            let decoded: ApplicationCommand = serde_json::from_value(encoded).unwrap();
            assert!(decoded.is_read_only_query());
        }
        assert!(!ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RequestCancel { task_id: "task".into(), now_ms: 1 },
        }.is_read_only_query());
    }
    use serde_json::json;

    #[test]
    fn host_output_admission_query_uses_shared_constructor_without_store_changes() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("admission.sqlite");
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        let observer = rusqlite::Connection::open(&database).unwrap();
        observer.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        // Exclude the independent archive publisher from the measured interval.
        // WAL readers still run, while an attempted query-side write must fail.
        observer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let data_version = || {
            observer
                .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
                .unwrap()
        };
        let original_version = data_version();
        let route = json!({
            "route":{"kind":"native","adapter":"mlx","adapter_contract_revision":1,
                     "model_id":"local-model","profile_ref":null},
            "max_input_tokens":2048,"max_output_tokens":8192,"max_total_tokens":4096
        });
        // An explicit output limit must leave room for input in the total
        // context window; 4096 would consume this fixture's entire window.
        for preference in [json!("default"), json!({"explicit":2048})] {
            let wire = json!({"type":"admit_host_output","routes":[route.clone()],"preference":preference});
            let command: ApplicationCommand = serde_json::from_value(wire.clone()).unwrap();
            assert!(command.is_read_only_query());
            assert_eq!(
                nola_protocol::CommandMetadata::wire_command_name(&command),
                "admit_host_output"
            );
            assert_eq!(serde_json::to_value(&command).unwrap(), wire);
            let expected = nola_host_protocol::HostOutputAdmission::admit(
                serde_json::from_value(wire["routes"].clone()).unwrap(),
                serde_json::from_value(wire["preference"].clone()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                execute_application_command(&mut store, command).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
        for routes in [json!([]), json!([route.clone(), route.clone()])] {
            let command = serde_json::from_value(json!({
                "type":"admit_host_output","routes":routes,"preference":"default"
            }))
            .unwrap();
            assert!(execute_application_command(&mut store, command).is_err());
        }
        for tokens in [4096, 16384] {
            let command = serde_json::from_value(json!({
                "type":"admit_host_output","routes":[route.clone()],"preference":{"explicit":tokens}
            })).unwrap();
            assert!(execute_application_command(&mut store, command).is_err());
        }
        assert_eq!(
            data_version(),
            original_version,
            "pure admission committed database changes"
        );
        observer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn reasoning_resolution_wire_contract_is_a_read_only_query() {
        let wire = json!({
            "type":"resolve_model_reasoning",
            "declaration":{"control":"effort","efforts":["low","medium","high"]},
            "protocol":"responses",
            "output_limit":16384,
            "supports_reasoning":true,
            "requested":null
        });
        let decoded: ApplicationCommand = serde_json::from_value(wire.clone()).unwrap();
        assert!(decoded.is_read_only_query());
        assert_eq!(
            nola_protocol::CommandMetadata::wire_command_name(&decoded),
            "resolve_model_reasoning"
        );
        let mut canonical = wire.clone();
        canonical["tier"] = serde_json::Value::Null;
        assert_eq!(serde_json::to_value(decoded).unwrap(), canonical);
        for (field, invalid) in [
            ("protocol", json!("unknown")),
            ("output_limit", json!(-1)),
            ("output_limit", json!(1024.5)),
            ("supports_reasoning", json!("true")),
            ("tier", json!("unknown")),
        ] {
            let mut invalid_wire = wire.clone();
            invalid_wire[field] = invalid;
            assert!(serde_json::from_value::<ApplicationCommand>(invalid_wire).is_err());
        }
    }

    #[test]
    fn reasoning_query_dispatch_uses_shared_resolution_without_store_changes() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("reasoning.sqlite");
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        let observer = rusqlite::Connection::open(&database).unwrap();
        observer.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        // Hold only the writer reservation: queries must remain read-only.
        observer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let data_version = || {
            observer
                .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
                .unwrap()
        };
        let original_version = data_version();
        let pending = store.pending_sync_changes(100).unwrap().len();
        for (requested, tier, expected) in [
            (
                json!({"kind":"effort","value":"low"}),
                None,
                json!({"selection":{"kind":"effort","value":"low"}}),
            ),
            (serde_json::Value::Null, None, json!({"selection":null})),
            (
                serde_json::Value::Null,
                Some(nola_domain::reasoning_tiers::ReasoningTier::Balanced),
                json!({"selection":{"kind":"effort","value":"medium"}}),
            ),
        ] {
            let command = ApplicationCommand::ResolveModelReasoning {
                declaration: json!({"control":"effort","efforts":["low","medium","high"]}),
                protocol: nola_domain::generation::ModelProtocol::Responses,
                output_limit: 16384,
                supports_reasoning: true,
                requested,
                tier,
            };
            assert!(command.is_read_only_query());
            assert_eq!(
                execute_application_command(&mut store, command).unwrap(),
                expected
            );
        }
        assert_eq!(store.pending_sync_changes(100).unwrap().len(), pending);
        assert!(store.list_model_profiles().unwrap().is_empty());
        assert_eq!(
            data_version(),
            original_version,
            "pure resolution committed database changes"
        );
        observer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn query_semantics_come_from_typed_variants_not_ambiguous_payload_words() {
        for (command, query) in [
            (
                json!({"type":"prepare_goal_execution","conversation_id":"conversation","goal_id":"goal"}),
                true,
            ),
            (
                json!({"type":"prepare_goal_control_intent","input":{"conversation_id":"conversation",
                    "operation_id":"operation","action":"pause","expected_goal_id":"goal",
                    "expected_control_token":"opaque","objective":"","constraints":[],"wake_at_epoch_ms":null}}),
                false,
            ),
            (
                json!({"type":"get_product_setting","key":"delete everything 删除"}),
                true,
            ),
            (
                json!({"type":"put_product_setting","key":"search current 查询","value_json":"true","updated_at_ms":1}),
                false,
            ),
            (
                json!({"type":"data_source","command":{"type":"list"}}),
                true,
            ),
            (
                json!({"type":"model","command":{"type":"reset_parameters","profile_ids":[],"parameters_json":"{}","updated_at_ms":1}}),
                false,
            ),
            (
                json!({"type":"runtime_task","command":{"type":"latest_event_cursor"}}),
                true,
            ),
            (
                json!({"type":"load_runtime_effect_work_schedule","input":{"conversation_ids":["conversation"],"now_ms":42,"limit":8}}),
                true,
            ),
            (
                json!({"type":"runtime_task","command":{"type":"acknowledge_delivery","delivery":{"owner_conversation_id":"owner","task_id":"current search","occurrence_id":"terminal:receipt","consumer_id":"cli","command_id":"display"}}}),
                false,
            ),
            (
                json!({"type":"skill","command":{"type":"apply_changes","commands":[{"type":"list"}]}}),
                false,
            ),
        ] {
            let decoded: ApplicationCommand = serde_json::from_value(command).unwrap();
            assert_eq!(decoded.is_read_only_query(), query);
        }

        let lineage = ApplicationCommand::LoadCompactionLineage {
            conversation_id: "current search 删除".into(),
            compaction_part_id: Some("compaction".into()),
        };
        assert!(lineage.is_read_only_query());
        assert_eq!(
            nola_protocol::CommandMetadata::wire_command_name(&lineage),
            "load_compaction_lineage"
        );
    }

    #[test]
    fn read_replica_classification_isolates_supervisor_independent_queries() {
        // Tier 1 supervisor-independent committed Store queries return true.
        for command in [
            json!({"type":"list_conversations","query":{"project_root":null,"non_empty":true,"limit":32,"offset":0}}),
            json!({"type":"get_product_setting","key":"theme"}),
            json!({"type":"load_conversation_projection_window_v1","conversation_id":"conv-1","before_ordinal":null,"limit":20,"content_byte_limit":65536}),
            json!({"type":"skill","command":{"type":"list"}}),
            json!({"type":"mcp","command":{"type":"list"}}),
            json!({"type":"model","command":{"type":"list_profiles"}}),
            json!({"type":"runtime_task","command":{"type":"latest_event_cursor"}}),
        ] {
            let decoded: ApplicationCommand = serde_json::from_value(command).unwrap();
            assert!(decoded.is_read_only_query());
            assert!(
                decoded.can_execute_on_read_replica(),
                "{} must be eligible for read-only replica execution",
                nola_protocol::CommandMetadata::wire_command_name(&decoded)
            );
        }

        // Supervisor-consulting, upload-consulting, and mutating commands return false.
        for decoded in [
            ApplicationCommand::LoadHostExecutionState {
                conversation_id: "conv-1".into(),
            },
            ApplicationCommand::LoadCommittedHostRequests {
                run_id: "run-1".into(),
            },
            ApplicationCommand::LoadSessionGoalProjection {
                conversation_id: "conv-1".into(),
            },
            ApplicationCommand::ReadPermissionConfiguration {
                conversation_id: "conv-1".into(),
                source_host_id: None,
                client_id: None,
                unresolved_run_ids: Vec::new(),
                source_unconfirmed: false,
                minimum_policy_revision: None,
            },
            ApplicationCommand::LoadConversationPermissionProjectionV1 {
                conversation_id: "conv-1".into(),
                run_id: "run-1".into(),
                scope: PermissionProjectionReadScope::CurrentRun,
            },
            ApplicationCommand::PutProductSetting {
                key: "theme".into(),
                value_json: "\"dark\"".into(),
                updated_at_ms: 1,
            },
        ] {
            assert!(
                !decoded.can_execute_on_read_replica(),
                "{} must NOT execute on a read-only replica",
                nola_protocol::CommandMetadata::wire_command_name(&decoded)
            );
        }
    }
}
