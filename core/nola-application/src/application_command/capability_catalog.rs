// SPDX-License-Identifier: Apache-2.0
//! Capability catalog command dispatch.

use serde_json::{json, Value};

use super::{encode_application_result, ApplicationCommand};
use crate::{repositories::CapabilityCatalogRepositories, ProductSetting};

pub(super) fn execute<R: CapabilityCatalogRepositories + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider>(
    store: &mut R,
    command: ApplicationCommand,
) -> Result<Value, String> {
    match command {
        ApplicationCommand::SubmitUiInteraction { input } => encode_application_result(crate::generative_ui::submit_ui_interaction(store, input)?),
        ApplicationCommand::ExecuteUiTool { input } => encode_application_result(crate::generative_ui::execute_ui_tool(store, input)?),
        ApplicationCommand::ReadUiAuthority => encode_application_result(store.ui_authority_id().map_err(|error| error.to_string())?),
        ApplicationCommand::ReadUiCapability => encode_application_result(nola_agent_engine::agent::generative_ui::capability_contract()),
        ApplicationCommand::PreviewUiSurface { input } => encode_application_result(crate::generative_ui::preview_ui_surface(store, input)?),
        ApplicationCommand::ReadUiSurface { identity } => encode_application_result(crate::generative_ui::read_ui_snapshot(store, &identity)?),
        ApplicationCommand::ReadUiSurfaceChanges { identity, base_revision } => encode_application_result(crate::generative_ui::read_ui_surface_changes(store, &identity, base_revision)?),
        ApplicationCommand::ReadUiSurfaceRevision { identity, revision } => encode_application_result(crate::generative_ui::read_ui_revision_snapshot(store, &identity, revision)?),
        ApplicationCommand::ListUiSurfaces { conversation_id } => encode_application_result(crate::generative_ui::list_ui_snapshots(store, &conversation_id)?),
        ApplicationCommand::StepCompactionSummary { step } => encode_application_result(nola_kernel::compaction_summary::step_compaction_summary(step)?),
        ApplicationCommand::EvaluateCompactionMaintenance { facts } => encode_application_result(nola_kernel::compaction_maintenance::evaluate_compaction_maintenance(&facts)),

        ApplicationCommand::PersistPreparedHostToolPlan { snapshot, now_ms } => {
            store.persist_prepared_host_tool_plan(&snapshot, now_ms).map_err(|error| error.to_string())?;
            encode_application_result(())
        }
        ApplicationCommand::ReadPreparedHostToolPlan { reference, now_ms } => {
            encode_application_result(store.prepared_host_tool_plan(&reference, now_ms).map_err(|error| error.to_string())?)
        }
        ApplicationCommand::LoadAuxiliaryOperation { operation_id } => {
            encode_application_result(store.auxiliary_operation(&operation_id).map_err(|error| error.to_string())?)
        }
        ApplicationCommand::ListConversationAuxiliarySources { conversation_id, after, limit } => {
            encode_application_result(store.conversation_auxiliary_sources(&conversation_id, after.as_ref(), limit).map_err(|error| error.to_string())?)
        }
        ApplicationCommand::LoadAuxiliarySourceSlot { local_owner, scenario } => {
            encode_application_result(store.auxiliary_source_slot(&local_owner, &scenario).map_err(|error| error.to_string())?)
        }
        ApplicationCommand::AuxiliaryInvocation { command } => {
            encode_application_result(crate::execute_auxiliary_invocation(store, command)?)
        }
        ApplicationCommand::QueryAutomationOccurrences {input} => encode_application_result(crate::automation_occurrence::query(&input)?),
        ApplicationCommand::ResolveRemoteModelProfile { profile_id, expected_revision, request } => {
            use nola_store_port::model::ModelRepository;
            let profile:crate::ModelProfile=store.model_repository().model_profile(&profile_id).map_err(|e|e.to_string())?.ok_or("Model profile unavailable")?.into();
            if profile.revision!=expected_revision {return Err("Model profile revision changed".into());}
            let resolved=crate::model_profile_resolution::resolve_remote_profile(&profile,&request,Default::default())?;
            let source:Value=serde_json::from_str(&profile.source_json).map_err(|_|"Invalid model source")?;
            let (credential_ref,required)=if let Some(id)=source["connectionID"].as_str() {
                let connection=store.model_repository().model_connection(id).map_err(|e|e.to_string())?.ok_or("Model connection unavailable")?.document.connection;
                if !connection.enabled || connection.base_url!=resolved.endpoint {return Err("Model connection changed".into());}
                match connection.account_id {
                    Some(id)=>{let account=store.model_repository().model_account(&id).map_err(|e|e.to_string())?.ok_or("Model account unavailable")?.account;
                        if account.state!=nola_domain::model_connection::ModelAccountState::Ready {return Err("Model account needs reconnection".into());}
                        (account.credential_ref,true)},
                    None=>(None,false),
                }
            } else {(source["credentialRef"].as_str().map(str::to_string),source["credentialRequired"].as_bool().unwrap_or(true))};
            encode_application_result(json!({"resolved":resolved,"credential_ref":credential_ref,"credential_required":required}))
        }
        ApplicationCommand::ResolveChatModel { intent, default_value_json, candidates } =>
            encode_application_result(crate::model_selection::resolve_chat_model(&intent, default_value_json.as_deref(), &candidates)),
        ApplicationCommand::AdmitHostOutput { routes, preference } => {
            let admission = nola_host_protocol::HostOutputAdmission::admit(routes, preference)
                .map_err(|error| error.to_string())?;
            encode_application_result(admission)
        }
        ApplicationCommand::ResolveModelReasoning {
            declaration,
            protocol,
            output_limit,
            supports_reasoning,
            requested,
            tier,
        } => crate::resolve_model_reasoning_request(
            &declaration,
            protocol,
            output_limit,
            supports_reasoning,
            &requested,
            tier,
        ),
        ApplicationCommand::Context { command } => {
            let mut context = store.context_repository();
            crate::context::execute_context_command(&mut context, command)
        }
        ApplicationCommand::Skill { command } => {
            let mut skills = store.skill_repository();
            crate::skill::execute_skill_command(&mut skills, command)
        }
        ApplicationCommand::Mcp { command } => {
            let mut servers = store.mcp_repository();
            crate::mcp::execute_mcp_command(&mut servers, command)
        }
        ApplicationCommand::Plugin { command } => {
            let mut plugins = store.plugin_repository();
            crate::plugin::execute(&mut plugins, command)
        }
        ApplicationCommand::DataSource { command } => {
            let mut data_sources = store.data_source_repository();
            crate::data_source::execute_data_source_command(&mut data_sources, command)
        }
        ApplicationCommand::DocumentSource { command } => {
            let mut documents = store.document_source_repository();
            crate::document_source::execute_document_source_command(&mut documents, command)
        }
        ApplicationCommand::Model { command } => {
            if matches!(&command, crate::ModelCommand::ImportConnection { .. }) {
                crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
                    let mut models = store.model_repository();
                    crate::model::execute_model_command(&mut models, command)
                })
            } else {
                let mut models = store.model_repository();
                crate::model::execute_model_command(&mut models, command)
            }
        }
        ApplicationCommand::PutProductSetting {
            key,
            value_json,
            updated_at_ms,
        } => {
            if key.starts_with(crate::expert_peer::PRIVATE_PREFIX) { return Err("expert_peer_private_setting_requires_owning_operation".into()); }
            if nola_domain::model_connection::owns_setting(&key) {
                return Err("model connections and accounts require their owning model command".into());
            }
            if key == crate::model_selection::DEFAULT_CHAT_MODEL_KEY {
                crate::model_selection::parse_default_chat_configuration(Some(&value_json))?;
            }
            let mut settings = store.product_setting_repository();
            let setting =
                nola_store_port::product_setting::ProductSettingRepository::put_product_setting(
                    &mut settings,
                    &key,
                    &value_json,
                    updated_at_ms,
                )
                .map_err(|error| error.to_string())?;
            encode_application_result(ProductSetting::from(setting))
        }
        ApplicationCommand::PutProductSettingChecked {
            key,
            value_json,
            expected_revision,
            updated_at_ms,
        } => {
            let mut settings = store.product_setting_repository();
            encode_application_result(crate::product_setting::put_checked(
                &mut settings,
                &key,
                &value_json,
                expected_revision,
                updated_at_ms,
            )?)
        }
        ApplicationCommand::GetProductSetting { key } => {
            if key.starts_with(crate::expert_peer::PRIVATE_PREFIX) { return Err("expert_peer_private_setting_requires_owning_operation".into()); }
            let settings = store.product_setting_repository();
            let setting =
                nola_store_port::product_setting::ProductSettingRepository::product_setting(
                    &settings, &key,
                )
                .map_err(|error| error.to_string())?
                .map(ProductSetting::from);
            encode_application_result(setting)
        }
        ApplicationCommand::ListReadableExpertTemplates { actor, now_epoch_ms, request_host_id } => {
            crate::expert_prompt_catalog::read(&store.product_setting_repository(), &actor, now_epoch_ms, request_host_id)
        }
        ApplicationCommand::DeleteProductSetting { key } => {
            if nola_domain::remote_expert_parent::owns_setting(&key) { return Err("remote_expert_parent_source_requires_owning_receiver".into()); }
            if key.starts_with(crate::expert_peer::PRIVATE_PREFIX) { return Err("expert_peer_private_setting_requires_owning_operation".into()); }
            if nola_domain::model_connection::owns_setting(&key) {
                return Err("model connections and accounts require their owning model command".into());
            }
            if key == crate::persistent_agent::DIRECTORY_KEY || key.starts_with(crate::agent_room::ROOM_KEY_PREFIX) {
                return Err("Agent and room records must use their owning command; generic deletion is forbidden".into());
            }
            let mut settings = store.product_setting_repository();
            Ok(json!(
                nola_store_port::product_setting::ProductSettingRepository::delete_product_setting(
                    &mut settings,
                    &key,
                )
                .map_err(|error| error.to_string())?
            ))
        }
        _ => Err("command is outside the capability catalog family".into()),
    }
}
