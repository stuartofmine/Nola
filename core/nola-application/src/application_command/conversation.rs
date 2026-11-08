// SPDX-License-Identifier: Apache-2.0
//! Conversation, archive, transcript, and projection command dispatch.

use serde_json::{json, Value};

use super::{encode_application_result, ApplicationCommand};
use crate::{repositories::ConversationCommandRepositories, Conversation, ConversationSummary};

pub(crate) fn read_permission_projection<R: nola_store_port::conversation_query::ConversationQueryRepository>(
    store: &R,
    conversation_id: String,
    run_id: String,
    scope: super::PermissionProjectionReadScope,
    current: Option<&nola_store_port::conversation_query::CurrentPermissionState>,
) -> Result<Value, String> {
    if conversation_id.trim().is_empty() || run_id.trim().is_empty() {
        return Err("permission projection requires exact conversation and Run identities".into());
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    struct PendingPermissions {
        active_policy: Option<nola_domain::ActivePermissionPolicy>,
        revision: u64,
        pending: Vec<nola_agent_engine::agent::permissions::PendingPermissionRequest>,
    }
    #[derive(serde::Deserialize)]
    struct AdmissionEnvelope {
        session_id: String,
        turn_id: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Admission {
        session_id: String,
        turn_id: String,
        core_envelope: AdmissionEnvelope,
    }
    #[derive(serde::Deserialize)]
    struct Inputs {
        conversation_id: String,
        run_id: Option<String>,
        admission: Option<Admission>,
        session_id: String,
        state_session_id: String,
        active_turn_id: Option<String>,
        cursor: u64,
        cursor_type: String,
        stored_cursor: u64,
        permissions: PendingPermissions,
    }
    let permissions = store
        .load_session_permission_inputs(&conversation_id, current)
        .map_err(|error| error.to_string())?
        .map(|json| -> Result<_, String> {
            let inputs: Inputs =
                serde_json::from_str(&json).map_err(|error| error.to_string())?;
            if crate::assistant_owner::validate_checkpoint_identity(
                &conversation_id,
                &inputs.conversation_id,
                &inputs.session_id,
                &inputs.state_session_id,
            )
            .is_err()
                || inputs.cursor != inputs.stored_cursor
                || inputs.cursor_type != "integer"
            {
                return Err(
                    "permission projection identity, revision or cursor mismatch".into(),
                );
            }
            if inputs.run_id.as_deref() != Some(run_id.as_str()) {
                return Err("permission projection canonical Run changed".into());
            }
            let admission = inputs.admission.as_ref()
                .ok_or("permission projection has no canonical Turn admission evidence")?;
            if admission.session_id != inputs.session_id
                || admission.core_envelope.session_id != inputs.session_id
                || admission.turn_id != run_id
                || admission.core_envelope.turn_id.as_deref() != Some(run_id.as_str()) {
                return Err("permission projection differs from its admitted Core owner".into());
            }
            if inputs.permissions.pending.iter().any(|pending|
                inputs.active_turn_id.as_deref() != Some(pending.turn_id.0.as_str())) {
                return Err("pending permission belongs to another Turn".into());
            }
            if let Some(policy) = &inputs.permissions.active_policy {
                policy.validate()?;
                if policy.run_id != run_id || inputs.active_turn_id.as_deref() != Some(policy.run_id.as_str()) {
                    return Err("permission policy acknowledgement belongs to another Run".into());
                }
            }
            if let super::PermissionProjectionReadScope::Control {
                session_id, active_turn_id, event_cursor, permission_projection_revision,
            } = &scope {
                if inputs.session_id != *session_id
                    || inputs.active_turn_id != *active_turn_id
                    || inputs.cursor < *event_cursor
                    || inputs.permissions.revision != *permission_projection_revision {
                    return Err("permission control witness is stale or foreign".into());
                }
            }
            Ok(json!({
                "conversation_id": inputs.conversation_id,
                "run_id": run_id,
                "session_id": inputs.session_id,
                "active_turn_id": inputs.active_turn_id,
                "event_cursor": inputs.cursor,
                "permission_projection_revision": inputs.permissions.revision,
                "permissions": inputs.permissions,
            }))
        })
        .transpose()?;
    encode_application_result(permissions)
}

pub(crate) fn read_current_permission_projection<R: nola_store_port::conversation_query::ConversationQueryRepository
    + nola_store_port::run_effect::RunEffectRepository>(
    store: &R, conversation_id: String, run_id: String, scope: super::PermissionProjectionReadScope,
) -> Result<Value, String> {
    let recovered = crate::checkpoint_read_override(store, &conversation_id)?;
    let current = recovered.as_ref().map(|checkpoint| -> Result<_, String> {
        let value: Value = serde_json::from_str(&checkpoint.checkpoint_json).map_err(|e| e.to_string())?;
        serde_json::from_value::<nola_store_port::conversation_query::CurrentPermissionState>(json!({
            "conversation_id": checkpoint.conversation_id,
            "revision": checkpoint.revision,
            "event_cursor": value["last_event_cursor"]["sequence"],
            "session_id": value["session_id"],
            "state_session_id": value["state"]["session_id"],
            "active_turn_id": value["state"]["execution"]["active_turn"]["turn_id"],
            "permissions": value["state"]["execution"]["permissions"],
            "active_permission_policy": value["state"]["execution"]["active_permission_policy"],
        })).map_err(|e| e.to_string())
    }).transpose()?;
    read_permission_projection(store, conversation_id, run_id, scope, current.as_ref())
}

/// Called only inside a repository snapshot (or its enclosing write unit).
/// The clients supply transport identity and unresolved observations, never the
/// authoritative owner or policy acknowledgement.
pub(crate) fn read_permission_configuration<R: nola_store_port::conversation_query::ConversationQueryRepository
    + nola_store_port::run_effect::RunEffectRepository + nola_store_port::remote_session::RemoteSessionRepository
    + nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository
    + nola_store_port::conversation_history::ConversationProjectionRepository>(
    store: &R,
    conversation_id: &str,
    source_host_id: Option<&str>,
    client_id: Option<&str>,
    unresolved_run_ids: &[String],
    source_unconfirmed: bool,
    minimum_policy_revision: Option<i64>,
    current: Option<&nola_store_port::conversation_query::CurrentPermissionState>,
) -> Result<Value, String> {
    use nola_domain::{PermissionProfile, RunRecoveryObservation};
    use nola_domain::remote_submission::RemoteSubmissionPhase;
    if conversation_id.is_empty() || conversation_id.trim() != conversation_id {
        return Err("invalid permission conversation identity".into());
    }
    let Some(conversation) = store.load_conversation_page(conversation_id, 0, 0)
        .map_err(|e| e.to_string())? else { return Ok(Value::Null); };
    // An unscoped display read asks whether the desired profile is effective.
    // A save observer supplies its own commit revision; unrelated configuration
    // edits must not create a fictitious pending permission update.
    let minimum_policy_revision = minimum_policy_revision.unwrap_or(1);
    if minimum_policy_revision < 1 || minimum_policy_revision > conversation.configuration_revision {
        return Err("permission acknowledgement revision is outside the committed configuration".into());
    }
    let settings: Value = serde_json::from_str(conversation.settings_json.as_deref().unwrap_or("{}"))
        .map_err(|e| e.to_string())?;
    let profile: Option<PermissionProfile> = serde_json::from_value(settings["permissionModeRawValue"].clone())
        .map_err(|e| e.to_string())?;
    let active = store.active_run(conversation_id).map_err(|e| e.to_string())?;
    let mut owner = json!({"kind":"inactive"});
    let mut policy: Option<nola_domain::ActivePermissionPolicy> = None;
    let mut active_run_id = None;
    if let Some(active) = active {
        let assistant = active.assistant_message_id.as_deref()
            .filter(|id| !id.is_empty()).ok_or("permission Run has no assistant owner")?;
        let inspection = crate::inspect_run_recovery(store, conversation_id.into(),
            active.run_id.clone(), assistant.into())?;
        match inspection.observation {
            RunRecoveryObservation::Terminal { .. } => {},
            RunRecoveryObservation::Active { source_host_id: source, .. } => {
                owner = if let Some(source) = source {
                    if source_host_id.is_some_and(|host| host != source) {
                        return Err("reconnect the active source Host before changing permissions".into());
                    }
                    let identity = store.admitted_remote_run_identity(&active.run_id)
                        .map_err(|e| e.to_string())?.ok_or("source admission is not confirmed")?;
                    identity.validate()?;
                    if identity.source_host_id != source
                        || identity.conversation_id != conversation_id
                        || identity.submission_id != active.run_id
                        || identity.assistant_message_id != assistant
                        || client_id.is_some_and(|client| client != identity.client_id) {
                        return Err("permission source admission identity mismatch".into());
                    }
                    json!({"kind":"remote","identity":identity})
                } else {
                    json!({"kind":"local","run_id":active.run_id,"assistant_message_id":assistant})
                };
                // Remote acknowledgements use the source configuration clock,
                // not this mirror's independent CAS revision. The source Host
                // endpoint owns that observation; this read establishes owner.
                if owner["kind"] == "local" {
                    let projection = match current {
                        Some(current) => read_permission_projection(store, conversation_id.into(), active.run_id.clone(),
                            super::PermissionProjectionReadScope::CurrentRun, Some(current))?,
                        None => read_current_permission_projection(store, conversation_id.into(), active.run_id.clone(),
                            super::PermissionProjectionReadScope::CurrentRun)?,
                    };
                    policy = serde_json::from_value(projection["permissions"]["active_policy"].clone())
                        .map_err(|e| e.to_string())?;
                }
                if let Some(policy) = &policy {
                    policy.validate()?;
                    if policy.run_id != active.run_id || policy.configuration_revision > conversation.configuration_revision {
                        return Err("permission acknowledgement differs from configuration owner".into());
                    }
                }
                active_run_id = Some(active.run_id);
            }
            _ => return Err("permission Run owner is not confirmed".into()),
        }
    } else {
        let intent = store.remote_submission_intent(conversation_id).map_err(|e| e.to_string())?;
        let mut unresolved: Vec<&str> = unresolved_run_ids.iter().map(String::as_str).collect();
        if let Some(intent) = intent.as_ref().filter(|intent| intent.phase == RemoteSubmissionPhase::Admitted) {
            unresolved.push(&intent.identity.submission_id);
        }
        for run in unresolved {
            if store.canonical_run_terminal(conversation_id, run).map_err(|e| e.to_string())?.is_none() {
                return Err("existing permission Run is not confirmed terminal".into());
            }
        }
        if source_unconfirmed {
            return Err("source Host status must be confirmed before saving permissions".into());
        }
    }
    let state = if active_run_id.is_none() { "inactive" } else if policy.as_ref().is_some_and(|policy|
        policy.configuration_revision >= minimum_policy_revision
            && policy.profile == profile.unwrap_or(PermissionProfile::Cautious)) {
        "active"
    } else { "pending" };
    Ok(json!({
        "conversation_id":conversation_id,
        "configuration_revision":conversation.configuration_revision,
        "settings":settings,
        "permission_profile":profile,
        "owner":owner,
        "active_run_id":active_run_id,
        "active_policy":policy,
        "state":state,
    }))
}

pub(super) fn execute<R: ConversationCommandRepositories>(
    store: &mut R,
    command: ApplicationCommand,
) -> Result<Value, String> {
    match command {
        ApplicationCommand::LocalRunContextUsage { identity } => encode_application_result(
            store.local_run_context_usage(&identity).map_err(|e|e.to_string())?),
        ApplicationCommand::AcceptRemoteRunContextUsage { source_host_id, record } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(store, |store|
                store.accept_remote_run_context_usage(&source_host_id, &record).map_err(|e|e.to_string()))?),
        ApplicationCommand::LocalRunMessageAdmissionHead {
            conversation_id,
            run_id,
        } => encode_application_result(
            store
                .local_run_message_admission_head(&conversation_id, &run_id)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::ReadRunMessageAdmissions {
            after,
            through,
            max_records,
            max_encoded_bytes,
        } => encode_application_result(
            store
                .read_run_message_admissions(&after, &through, max_records, max_encoded_bytes)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::RunMessageAdmissionReceptionState {
            source_host_id,
            through,
        } => encode_application_result(
            store
                .run_message_admission_reception_state(&source_host_id, &through)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::ReceiveRunMessageAdmissions {
            source_host_id,
            page,
        } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(
                store,
                |store| {
                    store
                        .receive_run_message_admissions(&source_host_id, &page)
                        .map_err(|e| e.to_string())
                },
            )?,
        ),
        ApplicationCommand::BeginReceiveAssistantRunTerminal {
            source_host_id,
            seal,
        } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(
                store,
                |store| {
                    store
                        .begin_receive_assistant_run_terminal(&source_host_id, &seal)
                        .map_err(|e| e.to_string())
                },
            )?,
        ),
        ApplicationCommand::StageAssistantRunTerminalManifest {
            source_host_id,
            page,
        } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(
                store,
                |store| {
                    store
                        .stage_assistant_run_terminal_manifest(&source_host_id, &page)
                        .map_err(|e| e.to_string())
                },
            )?,
        ),
        ApplicationCommand::CommitReceivedAssistantRunTerminal {
            source_host_id,
            seal,
        } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(
                store,
                |store| {
                    store
                        .commit_received_assistant_run_terminal(&source_host_id, &seal)
                        .map_err(|e| e.to_string())
                },
            )?,
        ),
        ApplicationCommand::LocalAssistantRunTerminalSeal {
            conversation_id,
            run_id,
        } => encode_application_result(
            store
                .local_assistant_run_terminal_seal(&conversation_id, &run_id)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::ReadLocalAssistantRunTerminalManifest {
            seal,
            after_entry,
            max_records,
            max_encoded_bytes,
        } => encode_application_result(
            store
                .read_local_assistant_run_terminal_manifest(
                    &seal,
                    after_entry,
                    max_records,
                    max_encoded_bytes,
                )
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::AssistantJournalReceptionState {
            source_host_id,
            through,
        } => encode_application_result(
            store
                .assistant_journal_reception_state(&source_host_id, &through)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::LocalAssistantJournalHead { identity } => encode_application_result(
            store
                .local_assistant_journal_head(&identity)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::LocalRunAssistantJournalHeads {
            conversation_id,
            run_id,
            after_message_id,
            limit,
        } => encode_application_result(
            store
                .local_run_assistant_journal_heads(
                    &conversation_id,
                    &run_id,
                    after_message_id.as_deref(),
                    limit,
                )
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::StageAssistantJournalContent { input } => {
            encode_application_result(crate::assistant_journal_read::stage_content(store, input)?)
        }
        ApplicationCommand::NextAssistantJournalContent {
            source_host_id,
            record,
        } => encode_application_result(crate::assistant_journal_read::next_content(
            store,
            &source_host_id,
            record,
        )?),
        ApplicationCommand::ReceiveAssistantJournalRecord {
            source_host_id,
            record,
            chunks,
        } => encode_application_result(crate::assistant_journal_read::receive_record(
            store,
            &source_host_id,
            record,
            chunks,
        )?),
        ApplicationCommand::ReadAssistantJournalManifest {
            snapshot,
            part_id,
            content,
            after_chunk,
            max_records,
        } => encode_application_result(crate::assistant_journal_read::read_manifest(
            store,
            snapshot,
            &part_id,
            &content,
            after_chunk,
            max_records,
        )?),
        ApplicationCommand::ReadAssistantJournalChunk {
            snapshot,
            part_id,
            content,
            chunk_index,
        } => encode_application_result(crate::assistant_journal_read::read_chunk(
            store,
            snapshot,
            &part_id,
            &content,
            chunk_index,
        )?),
        ApplicationCommand::ReadAssistantJournalRecordChunk {
            snapshot,
            part_id,
            content,
            chunk_index,
        } => encode_application_result(crate::assistant_journal_read::read_record_chunk(
            store,
            snapshot,
            &part_id,
            &content,
            chunk_index,
        )?),
        ApplicationCommand::ReadAssistantJournalParts {
            snapshot,
            after_part_id,
            max_records,
            max_encoded_bytes,
        } => encode_application_result(crate::assistant_journal_read::read_parts(
            store,
            snapshot,
            after_part_id.as_deref(),
            max_records,
            max_encoded_bytes,
        )?),
        ApplicationCommand::ReadAssistantJournalPartChanges {
            snapshot,
            base,
            after_part_id,
            max_records,
            max_encoded_bytes,
        } => encode_application_result(crate::assistant_journal_read::read_part_changes(
            store,
            snapshot,
            base,
            after_part_id.as_deref(),
            max_records,
            max_encoded_bytes,
        )?),
        ApplicationCommand::OpenAssistantJournalSnapshot { at } => {
            encode_application_result(crate::assistant_journal_read::open_snapshot(store, at)?)
        }
        ApplicationCommand::ReleaseAssistantJournalSnapshot { snapshot } => {
            encode_application_result(crate::assistant_journal_read::release_snapshot(
                store, snapshot,
            )?)
        }
        ApplicationCommand::ReadAssistantJournalRange {
            snapshot,
            part_id,
            content,
            offset_bytes,
            limit_bytes,
        } => encode_application_result(crate::assistant_journal_read::read_range(
            store,
            snapshot,
            &part_id,
            &content,
            offset_bytes,
            limit_bytes,
        )?),
        ApplicationCommand::ReadAssistantJournal {
            after,
            through,
            max_records,
            max_encoded_bytes,
        } => encode_application_result(crate::assistant_journal_read::read(
            store,
            after,
            through,
            max_records,
            max_encoded_bytes,
        )?),
        ApplicationCommand::InspectRunRecovery {
            conversation_id,
            expected_run_id,
            assistant_message_id,
        } => encode_application_result(crate::inspect_run_recovery(
            store,
            conversation_id,
            expected_run_id,
            assistant_message_id,
        )?),
        ApplicationCommand::InspectUnknownEffectRecovery {
            conversation_id, expected_run_id, assistant_message_id,
        } => {
            if [&conversation_id, &expected_run_id, &assistant_message_id].iter()
                .any(|id| id.is_empty() || id.trim() != id.as_str() || id.len() > 512)
            {
                return Err("unknown-effect recovery inspection requires exact owner identities".into());
            }
            let facts = store.inspect_unknown_effect_recovery(
                &conversation_id, &expected_run_id, &assistant_message_id,
            ).map_err(|error| error.to_string())?;
            encode_application_result(facts.map(|facts| nola_protocol::UnknownEffectRecoveryInspection {
                conversation_id, run_id: expected_run_id, assistant_message_id,
                invocation_config_digest: facts.invocation_config_digest,
                targets: facts.targets,
                independent_pending_effects: facts.independent_pending_effects, retry_allowed: facts.retry_allowed,
            }))
        }
        ApplicationCommand::ListRecoverableRuns { after_run_id, limit } => {
            encode_application_result(crate::list_recoverable_runs(store, after_run_id, limit)?)
        }
        ApplicationCommand::EncodeHostWorkerIdentity { identity } =>
            encode_application_result(identity.lease_identity()?),
        ApplicationCommand::ValidateRunRecoveryAuthority { conversation_id, expected_run_id,
            assistant_message_id, expected_invocation_config_digest, current } =>
            encode_application_result(crate::validate_run_recovery_authority(store, conversation_id,
                expected_run_id, assistant_message_id, expected_invocation_config_digest, current)?),
        ApplicationCommand::ValidateMobileRunRecoveryAuthority { conversation_id, expected_run_id,
            assistant_message_id, expected_invocation_config_digest, current } =>
            encode_application_result(crate::run_recovery_authority::validate_mobile_run_recovery_authority(store, conversation_id,
                expected_run_id, assistant_message_id, expected_invocation_config_digest, current)?),
        ApplicationCommand::LoadExactRunInvocation {
            conversation_id,
            expected_run_id,
            assistant_message_id,
        } => encode_application_result(crate::load_exact_run_invocation(
            store,
            conversation_id,
            expected_run_id,
            assistant_message_id,
        )?),
        ApplicationCommand::CanonicalRunTerminal {
            conversation_id,
            run_id,
        } => encode_application_result(
            store
                .canonical_run_terminal(&conversation_id, &run_id)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::ArchiveDirectoryRevision { .. } => encode_application_result(
            store
                .archive_directory_revision()
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::PutConversationPdfResource { input } => encode_application_result(
            store
                .put_conversation_pdf_resource(&input)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::ReadConversationResource {
            conversation_id,
            resource_id,
        } => encode_application_result(
            store
                .read_conversation_resource(&conversation_id, &resource_id)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::ListConversationResources { conversation_id } => {
            encode_application_result(
                store
                    .list_conversation_resources(&conversation_id)
                    .map_err(|error| error.to_string())?,
            )
        }
        ApplicationCommand::DeleteConversationResource {
            conversation_id,
            resource_id,
        } => encode_application_result(
            store
                .delete_conversation_resource(&conversation_id, &resource_id)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::BeginArchiveRestore { input } => encode_application_result(
            store
                .begin_archive_restore(&input)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::ArchiveRestoreProgress { restore_id } => encode_application_result(
            store
                .archive_restore_progress(&restore_id)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::CommitArchiveRestore { input } => encode_application_result(
            store
                .commit_archive_restore(&input)
                .map_err(|error| error.to_string())?,
        ),
        ApplicationCommand::StageArchiveManifest { .. }
        | ApplicationCommand::PublishUploadedTranscript { .. }
        | ApplicationCommand::BeginContentUpload { .. }
        | ApplicationCommand::AppendContentChunk { .. }
        | ApplicationCommand::ContentUploadProgress { .. }
        | ApplicationCommand::ReadContentUploadChunk { .. }
        | ApplicationCommand::SealContentUpload { .. }
        | ApplicationCommand::CancelContentUpload { .. } => {
            Err("content upload requires the Application runtime".into())
        }
        ApplicationCommand::QueryCanonicalHistory { input } => {
            input.validate()?;
            let response = store.query_canonical_history(&input).map_err(|error| error.to_string())?;
            response.validate_for(&input)?;
            encode_application_result(response)
        }
        ApplicationCommand::SearchConversationText {
            conversation_id,
            query,
            limit,
        } => encode_application_result(crate::conversation_recall::search(
            store,
            &conversation_id,
            &query,
            limit,
        )?),
        ApplicationCommand::ReadConversationTextRange {
            conversation_id,
            message_id,
            part_id,
            offset_bytes,
            limit_bytes,
        } => encode_application_result(crate::conversation_recall::read_range(
            store,
            &conversation_id,
            &message_id,
            &part_id,
            offset_bytes,
            limit_bytes,
        )?),
        ApplicationCommand::RevertConversation { input } => {
            encode_application_result(crate::revert_conversation(store, input)?)
        }
        ApplicationCommand::CreateConversationBranch { input } => {
            let mut conversations = store.conversation_write_repository();
            encode_application_result(crate::create_conversation_branch(
                &mut conversations,
                input,
            )?)
        }
        ApplicationCommand::PrepareRemoteSubmission { input } => encode_application_result(
            store
                .prepare_remote_submission(&input)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::AbandonPreparedRemoteSubmission { identity } => {
            encode_application_result(
                store
                    .abandon_prepared_remote_submission(&identity)
                    .map_err(|e| e.to_string())?,
            )
        }
        ApplicationCommand::AdoptObservedRemoteRun { receipt } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(
                store,
                |store| {
                    store
                        .adopt_observed_remote_run(&receipt)
                        .map_err(|e| e.to_string())
                },
            )?,
        ),
        ApplicationCommand::ObserveSubmissionFrontier { source_host_id, request } => {
            use nola_domain::remote_submission::{RemoteSubmissionPhase, SubmissionFrontier};
            request.validate(&source_host_id)?;
            let has_owner = if let (Some(run_id), Some(assistant)) = (&request.expected_run_id, &request.assistant_message_id) {
                let receipt = store.observed_source_submission_receipt(&source_host_id,
                    &request.client_id, &request.conversation_id, run_id, assistant)
                    .map_err(|error| error.to_string())?
                    .ok_or("submission frontier has no admitted source owner")?;
                receipt.validate()?;
                if receipt.phase != RemoteSubmissionPhase::Admitted {
                    return Err("submission frontier source is not admitted".into());
                }
                true
            } else { false };
            // This existing repository operation pins one read snapshot for
            // both configuration revision and the visible canonical tail.
            // Do not combine a local mirror revision with a separate Host read.
            let snapshot = store.load_conversation_tail_snapshot(&request.conversation_id, 1, 1, 1)
                .map_err(|error| error.to_string())?;
            // Explicit model selection can create the source conversation
            // before its first submission. It has configuration but no Run
            // owner yet. Existing execution/history still requires its exact
            // admitted owner; an empty visible projection alone is not proof.
            let unstarted_source = if let Some(snapshot) = snapshot.as_ref().filter(|_| !has_owner) {
                snapshot.total_message_count == 0
                    && snapshot.conversation.messages.is_empty()
                    && !store.conversation_has_admitted_history(&request.conversation_id).map_err(|error| error.to_string())?
                    && matches!(store.conversation_source_owner(&request.conversation_id).map_err(|error| error.to_string())?, nola_domain::ConversationSourceOwner::Local)
                    && matches!(store.conversation_execution_ownership(&request.conversation_id).map_err(|error| error.to_string())?, nola_domain::ConversationExecutionOwnership::User)
            } else { false };
            if snapshot.is_some() != has_owner && !unstarted_source {
                return Err("submission frontier conversation requires its admitted source owner".into());
            }
            let (configuration_revision, parent_message_id) = match snapshot {
                Some(snapshot) => (Some(snapshot.conversation.configuration_revision),
                    snapshot.conversation.messages.last().map(|message| message.id.clone())),
                None => (None, None),
            };
            encode_application_result(SubmissionFrontier { protocol_version: 1, source_host_id,
                client_id: request.client_id, conversation_id: request.conversation_id,
                configuration_revision, parent_message_id })
        },
        ApplicationCommand::ObservedSourceSubmissionReceipt {
            source_host_id,
            client_id,
            conversation_id,
            run_id,
            assistant_message_id,
        } => encode_application_result(
            store
                .observed_source_submission_receipt(
                    &source_host_id,
                    &client_id,
                    &conversation_id,
                    &run_id,
                    &assistant_message_id,
                )
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::BindRemoteSubmissionRequest { identity, request } => {
            encode_application_result(
                store
                    .bind_remote_submission_request(&identity, &request)
                    .map_err(|e| e.to_string())?,
            )
        }
        ApplicationCommand::ReadAdmittedRemoteRunIdentity { source_host_id, client_id, conversation_id, run_id, assistant_message_id } => {
            let identity = store.admitted_remote_run_identity(&run_id).map_err(|e| e.to_string())?;
            if let Some(identity) = &identity {
                identity.validate()?;
                if identity.source_host_id != source_host_id || identity.client_id != client_id
                    || identity.conversation_id != conversation_id || identity.submission_id != run_id
                    || identity.assistant_message_id != assistant_message_id {
                    return Err("admitted remote Run owner mismatch".into());
                }
            }
            encode_application_result(identity)
        }
        ApplicationCommand::ReadRemoteSubmissionIntent { conversation_id } => {
            encode_application_result(
                store
                    .remote_submission_intent(&conversation_id)
                    .map_err(|e| e.to_string())?,
            )
        }
        ApplicationCommand::ReadRemoteSubmissionRequest {
            identity,
            request_digest,
        } => encode_application_result(
            store
                .remote_submission_request(&identity, &request_digest)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::RegisterSourceSubmission { identity, request } => {
            encode_application_result(
                store
                    .register_source_submission(&identity, &request)
                    .map_err(|e| e.to_string())?,
            )
        }
        ApplicationCommand::ReadSourceSubmissionReceipt {
            identity,
            request_digest,
        } => encode_application_result(
            store
                .source_submission_receipt(&identity, &request_digest)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::CancelRejectedQueuedUiAction { identity, request_digest, now_ms } => crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
            use nola_domain::remote_submission::RemoteSubmissionPhase;
            let receipt = store.source_submission_receipt(&identity, &request_digest)
                .map_err(|e| e.to_string())?.ok_or("queued action has no dispatch receipt")?;
            if !matches!(receipt.phase, RemoteSubmissionPhase::Rejected | RemoteSubmissionPhase::Cancelled)
                || receipt.admission.is_some() {
                return Err("only an explicitly unadmitted terminal action can be cancelled".into());
            }
            let head = store.peek_agent_command(&identity.conversation_id).map_err(|e| e.to_string())?;
            let Some(head) = head else { return Ok(json!({"cancelled":true,"command_id":identity.submission_id})); };
            if head.command_id != identity.submission_id || head.lease_until_ms.is_some_and(|until| until > now_ms) {
                return Err("queued action is not the unleased queue head".into());
            }
            let boundary: nola_agent_engine::agent::host::AgentBoundaryInput = serde_json::from_str(&head.payload_json).map_err(|e| e.to_string())?;
            let nola_agent_engine::agent::host::AgentBoundaryInput::Submission {
                submission: nola_agent_engine::agent::AgentSubmission::UserInput { items, submission_id, .. }
            } = boundary else { return Err("queue head is not a UI action".into()); };
            if submission_id.as_deref() != Some(identity.submission_id.as_str())
                || !matches!(items.as_slice(), [nola_agent_engine::agent::AgentItem::UiAction { id, .. }] if id.as_str() == identity.user_message_id) {
                return Err("queued action ownership mismatch".into());
            }
            // Join the existing command-receipt UoW, or open one for direct callers.
            // A cancellation lease arbitrates with a concurrent dispatch; the
            // canonical submission and UI interaction receipts remain intact.
            let owner = format!("cancel-ui:{}", identity.submission_id);
            let lease = store.claim_next_agent_command(&identity.conversation_id, &owner, now_ms, now_ms.saturating_add(1000))
                .map_err(|e| e.to_string())?.ok_or("queued action was concurrently claimed")?;
            if lease.command_id != identity.submission_id
                || !store.acknowledge_agent_command(&identity.conversation_id, &identity.submission_id, &owner).map_err(|e| e.to_string())? {
                return Err("queued action cancellation lost ownership".into());
            }
            Ok(json!({"cancelled":true,"command_id":identity.submission_id}))
        }),
        ApplicationCommand::CancelSourceSubmission {
            identity,
            request_digest,
        } => encode_application_result(
            store
                .settle_source_submission(&identity, &request_digest, None)
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::RejectSourceSubmission {
            identity,
            request_digest,
            rejection_code,
        } => encode_application_result(
            store
                .settle_source_submission(&identity, &request_digest, Some(&rejection_code))
                .map_err(|e| e.to_string())?,
        ),
        ApplicationCommand::AcceptRemoteSubmissionReceipt { receipt } => encode_application_result(
            crate::assistant_publication_transaction::with_publication_transaction(
                store,
                |store| {
                    store
                        .accept_remote_submission_receipt(&receipt)
                        .map_err(|e| e.to_string())
                },
            )?,
        ),
        ApplicationCommand::CreateConversation { input } => {
            let mut conversations = store.conversation_write_repository();
            encode_application_result(crate::conversation::create_conversation(
                &mut conversations,
                input,
            )?)
        }
        ApplicationCommand::AgentRoom { command } => {
            use crate::agent_room::{self, AgentRoomCommand};
            match command {
                AgentRoomCommand::EnsureForConversation { conversation_id, at_ms } => encode_application_result(
                    agent_room::ensure_for_conversation(store, conversation_id, at_ms)?),
                AgentRoomCommand::AttachConversation { conversation_id, room_id, at_ms } => encode_application_result(
                    agent_room::attach_conversation(store, conversation_id, room_id, at_ms)?),
                AgentRoomCommand::ForConversation { conversation_id } => encode_application_result(
                    agent_room::for_conversation(store, &conversation_id)?),
                AgentRoomCommand::PrepareExecution { room_id, submission_id, expected_revision, intent, at_ms } => encode_application_result(
                    agent_room::prepare_execution(store, &room_id, &submission_id, expected_revision, intent, at_ms)?),
                AgentRoomCommand::ExecutionIntent { room_id, submission_id } => encode_application_result(
                    agent_room::execution_intent(&store.product_setting_repository(), &room_id, &submission_id)?),
                AgentRoomCommand::ReconcileExecution { room_id, submission_id, at_ms } => encode_application_result(
                    agent_room::reconcile_execution(store, &room_id, &submission_id, at_ms)?),
                AgentRoomCommand::CancelPending { room_id, submission_id, expected_revision, at_ms } => encode_application_result(
                    agent_room::cancel_pending(&mut store.product_setting_repository(), &room_id, &submission_id, expected_revision, at_ms)?),
                AgentRoomCommand::ObserveExecution { room_id, submission_id, run_id, at_ms } => encode_application_result(
                    agent_room::observe_execution(store, &room_id, &submission_id, &run_id, at_ms)?),
                AgentRoomCommand::ClaimExecution { room_id, submission_id, expected_revision, at_ms } => encode_application_result(
                    agent_room::claim_execution(store, &room_id, &submission_id, expected_revision, at_ms)?),
                AgentRoomCommand::FailSubmission { room_id, submission_id, expected_revision, code, at_ms } => encode_application_result(
                    agent_room::fail_submission(&mut store.product_setting_repository(), &room_id, &submission_id, expected_revision, &code, at_ms)?),
                AgentRoomCommand::SelectRecipient { room_id, submission_id, expected_revision, participant_id, at_ms } => encode_application_result(
                    agent_room::select_recipient(&mut store.product_setting_repository(), &room_id, &submission_id, expected_revision, &participant_id, at_ms)?),
                AgentRoomCommand::Submit { input } => {
                    let anchored = input.parent_message_id.is_some();
                    let value = if anchored {
                        let conversation_id = input.conversation_id.clone().ok_or("room_conversation_owner_missing")?;
                        agent_room::submit_for_conversation(store, &conversation_id, input)?
                    } else {
                        agent_room::submit(&mut store.product_setting_repository(), input)?
                    };
                    encode_application_result(value)
                },
                AgentRoomCommand::Submission { room_id, submission_id } => encode_application_result(
                    agent_room::submission(&store.product_setting_repository(), &room_id, &submission_id)?),
                AgentRoomCommand::Submissions { room_id, limit, offset } => encode_application_result(
                    agent_room::submissions(&store.product_setting_repository(), &room_id, limit, offset)?),
                AgentRoomCommand::List { limit, offset } => encode_application_result(
                    agent_room::list(&store.product_setting_repository(), limit, offset)?),
                AgentRoomCommand::Directory => encode_application_result(
                    crate::persistent_agent::load(&store.product_setting_repository())?),
                AgentRoomCommand::ConfigureAgent { input } => encode_application_result(
                    crate::persistent_agent::configure(&mut store.product_setting_repository(), input)?),
                AgentRoomCommand::Create { title, at_ms } => encode_application_result(
                    agent_room::create(&mut store.product_setting_repository(), title, at_ms)?),
                AgentRoomCommand::Read { room_id } => encode_application_result(
                    agent_room::load(&store.product_setting_repository(), &room_id)?),
                AgentRoomCommand::Join { input } => encode_application_result(agent_room::join(store, input)?),
                AgentRoomCommand::Deactivate { room_id, participant_id, expected_revision, at_ms } => encode_application_result(
                    agent_room::deactivate(&mut store.product_setting_repository(), &room_id, &participant_id, expected_revision, at_ms)?),
            }
        }
        ApplicationCommand::ImportConversationTranscript { input } => {
            let mut conversations = store.conversation_write_repository();
            encode_application_result(crate::conversation::import_conversation_transcript(
                &mut conversations,
                input,
            )?)
        }
        ApplicationCommand::AdmitConversationUserMessage { input } => {
            let conversation = store.load_conversation(&input.conversation_id)
                .map_err(|error| error.to_string())?
                .ok_or("conversation_not_found")?;
            let result = crate::conversation::admit_user_message(
                &mut store.conversation_write_repository(),
                &conversation,
                input,
            )?;
            encode_application_result(result)
        }
        ApplicationCommand::EditConversationMessage { input } => {
            let mut conversations = store.conversation_write_repository();
            encode_application_result(crate::conversation::edit_conversation_message(
                &mut conversations,
                input,
            )?)
        }
        ApplicationCommand::DeleteConversationMessage { input } => {
            let mut conversations = store.conversation_write_repository();
            encode_application_result(crate::conversation::delete_conversation_message(
                &mut conversations,
                input,
            )?)
        }
        ApplicationCommand::PatchConversationMetadata { input } => {
            crate::conversation::validate_metadata_patch(&input.patch_json)?;
            let mut conversations = store.conversation_write_repository();
            use nola_store_port::conversation_profile::ConversationProfileRepository;
            Ok(json!(conversations.patch_conversation_metadata(&input)
                .map_err(|error| error.to_string())?))
        }
        ApplicationCommand::PatchConversationConfiguration { input } => {
            encode_application_result(crate::permission_policy::patch_configuration(store, &input)?)
        }
        ApplicationCommand::SelectConversationExecution { input } => {
            let patch = input.into_patch()?;
            encode_application_result(crate::permission_policy::patch_configuration(store, &patch)?)
        }
        ApplicationCommand::ReadPermissionConfiguration {
            conversation_id, source_host_id, client_id, unresolved_run_ids, source_unconfirmed, minimum_policy_revision,
        } => store.with_consistent_read(|store| {
            read_permission_configuration(store, &conversation_id, source_host_id.as_deref(),
                client_id.as_deref(), &unresolved_run_ids, source_unconfirmed, minimum_policy_revision, None)
        }),
        ApplicationCommand::SetConversationPermission {
            conversation_id, expected_revision, expected_run_id, expected_source_host_id, profile, additional_configuration, updated_at_ms,
        } => crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
            let snapshot = read_permission_configuration(store, &conversation_id,
                expected_source_host_id.as_deref(), None, &[], false, None, None)?;
            if snapshot.is_null() {
                return Ok(json!({"status":"not_found","conversation_id":conversation_id}));
            }
            if snapshot["active_run_id"].as_str() != expected_run_id.as_deref()
                || snapshot["owner"]["identity"]["source_host_id"].as_str() != expected_source_host_id.as_deref() {
                return Err("permission owner changed before configuration commit".into());
            }
            let mut patch = json!({"permissionProfile":profile});
            if let Some(additional) = additional_configuration {
                if additional.fields().contains_key("permissionProfile") {
                    return Err("permission profile must have one source".into());
                }
                for (key, value) in additional.fields() { patch[key] = value.clone(); }
            }
            let input = nola_store_port::conversation_profile::PatchConversationConfigurationInput {
                conversation_id, expected_revision, updated_at_ms,
                patch_json: patch.to_string().parse()?,
            };
            encode_application_result(crate::permission_policy::patch_configuration(store, &input)?)
        }),
        ApplicationCommand::LoadRunIdentity { run_id } => {
            use nola_store_port::transcript_write::TranscriptWriteRepository;
            if run_id.is_empty() || run_id.len() > 512 || run_id.trim() != run_id
                || run_id.chars().any(char::is_control) {
                return Err("invalid Run identity".into());
            }
            let conversation_id = store.conversation_write_repository().conversation_id_for_run(&run_id)
                .map_err(|error| error.to_string())?.ok_or("run_not_found")?;
            Ok(json!({"run_id": run_id, "conversation_id": conversation_id}))
        }
        ApplicationCommand::LoadConversation { conversation_id } => {
            let conversation = store
                .load_conversation(&conversation_id)
                .map_err(|error| error.to_string())?
                .map(Conversation::from);
            encode_application_result(conversation)
        }
        ApplicationCommand::LoadConversationMetadata { conversation_id } => {
            let metadata = store.load_conversation_page(&conversation_id, 0, 0)
                .map_err(|error| error.to_string())?
                .map(|conversation| json!({
                    "id": conversation.id,
                    "revision": conversation.revision,
                    "configuration_revision": conversation.configuration_revision,
                    "settings_json": conversation.settings_json,
                }));
            encode_application_result(metadata)
        }
        ApplicationCommand::LoadConversationProjectionV1 { conversation_id } => {
            encode_application_result(
                crate::load_conversation_projection_v1(store, &conversation_id)
                    .map_err(|error| error.to_string())?,
            )
        }
        ApplicationCommand::ReadHistoricalPart {
            owner,
            expected_conversation_revision,
            maximum_output_bytes,
        } => encode_application_result(crate::conversation_projection::read_historical_part(
            store,
            &owner,
            expected_conversation_revision,
            maximum_output_bytes,
        )?),
        ApplicationCommand::ReadConversationPartDerivation {
            owner,
            expected_conversation_revision,
        } => encode_application_result(crate::transcript_export::read_part_derivation(
            store,
            &owner,
            expected_conversation_revision,
        )?),
        ApplicationCommand::ReadConversationToolSources { owner, expected_conversation_revision } =>
            encode_application_result(crate::transcript_export::read_tool_sources(store, &owner, expected_conversation_revision)?),
        ApplicationCommand::ReadCodeCellObservation { query } => {
            if query.conversation_id.trim().is_empty() || query.cell_id.trim().is_empty() {
                return Err("code cell observation requires an exact conversation and cell".into());
            }
            encode_application_result(store.read_code_cell(&query).map_err(|error| error.to_string())?)
        },
        ApplicationCommand::ReadCodeCellHistoryForEffect { request } =>
            encode_application_result(crate::code_cell_publication::read_for_effect(store, &request)?),
        ApplicationCommand::ReadConversationToolSourceRange { owner, source_item_id, expected_conversation_revision, offset_bytes, limit_bytes } =>
            encode_application_result(crate::transcript_export::read_tool_source_range(store, &owner, &source_item_id, expected_conversation_revision, offset_bytes, limit_bytes)?),
        ApplicationCommand::LoadConversationExportPage {
            conversation_id,
            after_ordinal,
            expected_revision,
        } => encode_application_result(crate::transcript_export::load_page(
            store,
            &conversation_id,
            after_ordinal,
            expected_revision,
        )?),
        ApplicationCommand::LoadCompactionLineage {
            conversation_id,
            compaction_part_id,
        } => encode_application_result(crate::transcript_export::load_compaction_lineage(
            store,
            &conversation_id,
            compaction_part_id.as_deref(),
        )?),
        ApplicationCommand::ReadConversationExportRange {
            conversation_id,
            message_id,
            part_id,
            field,
            expected_revision,
            offset_bytes,
            limit_bytes,
        } => encode_application_result(crate::transcript_export::read_range(
            store,
            &conversation_id,
            &message_id,
            &part_id,
            field,
            expected_revision,
            offset_bytes,
            limit_bytes,
        )?),
        ApplicationCommand::LoadConversationContextUsageV1 { conversation_id } => {
            let usage = crate::conversation_projection::load_conversation_context_usage_v1(
                store,
                &conversation_id,
            )
            .map_err(|error| error.to_string())?;
            encode_application_result(usage)
        }
        ApplicationCommand::LoadConversationPermissionProjectionV1 { conversation_id, run_id, scope } => {
            store.with_consistent_read(|store| read_current_permission_projection(store, conversation_id, run_id, scope))
        }
        ApplicationCommand::LoadConversationProjectionPageV1 {
            conversation_id,
            offset,
            limit,
        } => {
            if limit == 0 || limit > 256 {
                return Err("conversation projection page limit must be between 1 and 256".into());
            }
            let projection =
                crate::load_conversation_projection_page_v1(store, &conversation_id, offset, limit)
                    .map_err(|error| error.to_string())?;
            encode_application_result(projection)
        }
        ApplicationCommand::LoadConversationSourceOwner { conversation_id } => {
            encode_application_result(store.conversation_source_owner(&conversation_id).map_err(|error|error.to_string())?)
        }
        ApplicationCommand::LoadTaskChildConversationWindow {
            parent_conversation_id, parent_run_id, task_id, child_conversation_id,
            before_ordinal, limit, content_byte_limit,
        } => {
            if limit == 0 || limit > 200 || content_byte_limit == 0 || content_byte_limit > 2_000_000
                || before_ordinal.is_some_and(|value| value <= 0) {
                return Err("invalid task child conversation window bounds".into());
            }
            let window: crate::conversation_projection::ConversationProjectionWindowV1 = store.with_consistent_read(|store| {
                let authority = store.runtime_agent_task_authority(&task_id).map_err(|error| error.to_string())?
                    .ok_or("Task child source authority unavailable")?;
                authority.validate()?;
                let admission = authority.admission.ok_or("Task child has not been admitted")?;
                if admission.task_id != task_id || admission.parent_conversation_id != parent_conversation_id
                    || admission.parent_run_id != parent_run_id || admission.child_conversation_id != child_conversation_id {
                    return Err("Task child source ownership mismatch".into());
                }
                let window = crate::load_conversation_projection_window_v1(store, &child_conversation_id,
                    before_ordinal, limit, content_byte_limit).map_err(|error| error.to_string())?;
                if window.projection.execution_ownership != (nola_domain::ConversationExecutionOwnership::TaskChild {
                    parent_conversation_id, parent_run_id, task_id, child_conversation_id,
                }) { return Err("Task child projection no longer belongs to requested owner".into()); }
                Ok(window)
            })?;
            encode_application_result(window)
        }
        ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id,
            before_ordinal,
            limit,
            content_byte_limit,
        } => {
            if limit == 0 || limit > 200 {
                return Err(
                    "conversation projection window limit must be between 1 and 200".into(),
                );
            }
            if content_byte_limit == 0 || content_byte_limit > 2_000_000 {
                return Err(
                    "conversation projection window byte limit must be between 1 and 2000000"
                        .into(),
                );
            }
            if before_ordinal.is_some_and(|ordinal| ordinal <= 0) {
                return Err("conversation projection window cursor must be positive".into());
            }
            let window: crate::conversation_projection::ConversationProjectionWindowV1 = store.with_consistent_read(|store| {
                crate::load_conversation_projection_window_v1(
                    store,
                    &conversation_id,
                    before_ordinal,
                    limit,
                    content_byte_limit,
                )
                .map_err(|error| error.to_string())
            })?;
            encode_application_result(window)
        }
        ApplicationCommand::LoadConversationProjectionWindowUpdateV1 {
            conversation_id,
            before_ordinal,
            limit,
            content_byte_limit,
            after_sequence,
        } => {
            if limit == 0
                || limit > 200
                || content_byte_limit == 0
                || content_byte_limit > 2_000_000
            {
                return Err("invalid conversation window update budget".into());
            }
            if before_ordinal.is_some_and(|ordinal| ordinal <= 0) {
                return Err("conversation projection window cursor must be positive".into());
            }
            let window: Option<crate::conversation_projection::ConversationProjectionWindowV1> = store.with_consistent_read(|store| {
                let scope = nola_store_port::protocol_journal::ProjectionScopeKey {
                    scope_type: "conversation".into(),
                    scope_id: conversation_id.clone(),
                };
                let current = store
                    .projection_cursor(&scope)
                    .map_err(|error| error.to_string())?
                    .map(|cursor| cursor.current_sequence)
                    .unwrap_or(0);
                if after_sequence > current {
                    return Err("conversation window event cursor is ahead of the journal".into());
                }
                if current == after_sequence {
                    if store
                        .load_conversation_page(&conversation_id, 0, 0)
                        .map_err(|error| error.to_string())?
                        .is_none()
                    {
                        return Err(format!("missing conversation {conversation_id}"));
                    }
                    return Ok(None);
                }
                crate::load_conversation_projection_window_v1(
                    store,
                    &conversation_id,
                    before_ordinal,
                    limit,
                    content_byte_limit,
                )
                .map(Some)
                .map_err(|error| error.to_string())
            })?;
            encode_application_result(window)
        }
        ApplicationCommand::LoadConversationTailSnapshotV1 {
            conversation_id,
            message_limit,
            part_text_limit,
            part_json_limit,
        } => {
            if message_limit == 0 || message_limit > 256 {
                return Err("conversation tail message limit must be between 1 and 256".into());
            }
            if part_text_limit == 0 || part_text_limit > 512_000 {
                return Err("conversation tail text limit must be between 1 and 512000".into());
            }
            if part_json_limit == 0 || part_json_limit > 256_000 {
                return Err("conversation tail JSON limit must be between 1 and 256000".into());
            }
            let snapshot = store
                .load_conversation_tail_snapshot(
                    &conversation_id,
                    message_limit,
                    part_text_limit,
                    part_json_limit,
                )
                .map_err(|error| error.to_string())?
                .map(crate::ConversationTailSnapshot::try_from)
                .transpose()?;
            encode_application_result(snapshot)
        }
        ApplicationCommand::LoadConversationSettings { conversation_id } => {
            encode_application_result(
                store
                    .load_conversation_settings(&conversation_id)
                    .map_err(|error| error.to_string())?,
            )
        }
        ApplicationCommand::ListConversations { query } => {
            let conversations: Vec<ConversationSummary> = store
                .list_conversation_page(&query)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(Into::into)
                .collect();
            encode_application_result(conversations)
        }
        ApplicationCommand::RenameConversation {
            conversation_id,
            title,
            title_source,
            updated_at_ms,
        } => {
            let mut conversations = store.conversation_write_repository();
            Ok(json!(crate::conversation::rename_conversation(
                &mut conversations,
                &conversation_id,
                &title,
                title_source,
                updated_at_ms,
            )?))
        }
        ApplicationCommand::DeleteConversation { conversation_id } => {
            let mut conversations = store.conversation_write_repository();
            Ok(json!(crate::conversation::delete_conversation(
                &mut conversations,
                &conversation_id,
            )?))
        }
        _ => Err("command is outside the conversation family".into()),
    }
}

#[cfg(test)]
mod submission_frontier_tests {
    use crate::*;
    use nola_agent_engine::agent::{AgentInputDelivery, AgentItem, ItemId};
    use serde_json::json;

    #[test]
    fn configured_empty_source_has_frontier_but_history_requires_admitted_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("frontier.db");
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        let query = || ApplicationCommand::ObserveSubmissionFrontier {
            source_host_id: "source".into(),
            request: nola_domain::remote_submission::SubmissionFrontierRequest {
                expected_host_id: "source".into(), client_id: "client".into(),
                conversation_id: "conversation".into(), expected_run_id: None, assistant_message_id: None,
            },
        };
        assert!(execute_application_command(&mut store, query()).unwrap()["configuration_revision"].is_null());
        execute_application_command(&mut store, ApplicationCommand::CreateConversation { input: CreateConversation {
            id: "conversation".into(), title: "Selected before first send".into(),
            settings_json: Some("{}".into()), created_at_ms: 1, updated_at_ms: 1,
        }}).unwrap();
        execute_application_command(&mut store, ApplicationCommand::SelectConversationExecution {
            input: crate::conversation_configuration::SelectConversationExecutionInput {
                conversation_id: "conversation".into(), expected_revision: 1,
                selection: crate::conversation_configuration::ConversationExecutionSelection::Inherit,
                execution_host: Some("local".into()), execution_backend: None,
                external_execution_sessions: None, additional_configuration: None, updated_at_ms: 2,
            },
        }).unwrap();
        // The Host may prepare its Core configuration before admission fails;
        // this checkpoint contains no admitted turn or message history.
        let idle = nola_agent_engine::agent::AgentCore::try_new(crate::user_input_wait_tests::config()).unwrap()
            .checkpoint(nola_agent_engine::agent::recovery::CheckpointReason::ConfigurationChanged);
        store.save_checkpoint(&nola_store_port::conversation_revert::SaveCheckpointInput {
            conversation_id: "conversation".into(), expected_revision: 0, revision: 1,
            event_cursor: idle.last_event_cursor.sequence as i64,
            checkpoint_json: serde_json::to_string(&idle).unwrap(), updated_at_ms: 3,
        }).unwrap();
        drop(store);
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        assert!(nola_store_port::run_effect::RunEffectRepository::checkpoint_revision(&store, "conversation").unwrap().is_some(),
            "selection must exercise a configuration-only checkpoint");
        let frontier = execute_application_command(&mut store, query()).unwrap();
        assert_eq!(frontier["configuration_revision"], json!(2));
        assert!(frontier["parent_message_id"].is_null());
        let input = BeginAssistantTurn {
            expected_configuration_revision: Some(2), expected_parent_message_id: None,
            conversation_id: "conversation".into(), conversation_title: "Selected before first send".into(),
            user_message_id: "origin".into(), user_part_id: "body".into(), user_text: "Hello".into(),
            user_content_json: None, assistant_message_id: "assistant".into(), context_checkpoint_id: None,
            core_config: crate::user_input_wait_tests::config(), created_at_ms: 1000,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("origin".into()), "Hello")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("origin".into()), submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
            },
        };
        crate::assistant_turn::admit_with_core(&mut store, input, None).unwrap();
        assert!(execute_application_command(&mut store, query()).unwrap_err().contains("admitted source owner"));
    }
}
