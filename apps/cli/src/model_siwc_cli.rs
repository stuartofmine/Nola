// SPDX-License-Identifier: Apache-2.0
//! Explicit local lifecycle; Core owns the account directory, SIWC owns secrets.
use crate::{model_siwc::{SiwcHost, Snapshot}, store::NolaStore};
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::model_connection::{ModelAccount, ModelAccountSnapshot};
use serde_json::{json, Value};
use std::io::{self, Write};

pub(crate) fn load(store: &NolaStore, id: &str) -> Result<Option<ModelAccountSnapshot>, String> {
    store.application()?.execute(ApplicationCommand::Model { command:ModelCommand::GetAccount {id:id.into()} })
}
pub(crate) fn owned(value: &ModelAccountSnapshot, revision: Option<i64>) -> Result<(),String> {
    if revision.is_some_and(|expected| expected!=value.revision) { return Err("siwc_core_revision_conflict".into()); }
    if serde_json::to_value(value.account.source).map_err(|_| "siwc_owner_invalid")? != "nola_chatgpt"
        || value.account.credential_ref.as_deref()!=Some(format!("siwc:{}",value.account.id).as_str()) {
        return Err("siwc_core_owner_mismatch".into());
    }
    Ok(())
}
fn confirm(interactive: bool, message: &str) -> Result<bool,String> {
    if !interactive { return Err("ChatGPT sign-in and sign-out require an interactive terminal for explicit confirmation".into()); }
    print!("{message} [y/N] ");io::stdout().flush().map_err(|_| "siwc_terminal_unavailable")?;
    let mut answer=String::new();io::stdin().read_line(&mut answer).map_err(|_| "siwc_terminal_unavailable")?;
    Ok(matches!(answer.trim().to_ascii_lowercase().as_str(),"y"|"yes"))
}
pub(crate) fn save_account(store:&NolaStore, id:&str, label:&str, expected:Option<i64>, session:&Snapshot) -> Result<ModelAccountSnapshot,String> {
    let state=if !session.signed_in {"disconnected"} else if session.reauthorization_required {"expired"} else if session.plan_usage_authorized {"ready"} else {"denied"};
    let input:ModelAccount=serde_json::from_value(json!({"id":id,"display_name":label,"source":"nola_chatgpt","credential_ref":format!("siwc:{id}"),"state":state})).map_err(|_| "siwc_core_source_unavailable")?;
    store.application()?.execute(ApplicationCommand::Model { command:ModelCommand::SaveAccount {input,expected_revision:expected,updated_at_ms:chrono::Utc::now().timestamp_millis()} })
}
fn print(value: &impl serde::Serialize) -> Result<(),String> {
    println!("{}",serde_json::to_string_pretty(value).map_err(|_| "siwc_snapshot_invalid")?);Ok(())
}
pub(crate) fn sync_catalog(store:&NolaStore, owner:&ModelAccountSnapshot, models:Vec<crate::model_siwc::ModelChoice>) -> Result<nola_domain::model_connection::ModelConnectionSnapshot,String> {
    use nola_domain::model_connection::{CatalogModel,ModelConnectionSnapshot};
    let id=format!("siwc-{}",owner.account.id);
    let mut app=store.application()?;
    let prior:Option<ModelConnectionSnapshot>=app.execute(ApplicationCommand::Model {command:ModelCommand::GetConnection {id:id.clone()}})?;
    let snapshot=if let Some(prior)=prior {
        let connection=&prior.document.connection;
        if connection.account_id.as_deref()!=Some(owner.account.id.as_str()) || serde_json::to_value(connection.source).map_err(|_|"siwc_connection_invalid")?!="nola_chatgpt" {return Err("siwc_connection_owner_changed".into());}
        prior
    } else {
        let input=serde_json::from_value(json!({"id":id,"display_name":format!("ChatGPT {}",owner.account.display_name),"base_url":"https://api.openai.com/v1","protocols":["siwc_responses"],"account_id":owner.account.id,"source":"nola_chatgpt","source_id":owner.account.id,"enabled":true})).map_err(|_|"siwc_connection_invalid")?;
        app.execute(ApplicationCommand::Model {command:ModelCommand::SaveConnection {input,expected_revision:None,updated_at_ms:chrono::Utc::now().timestamp_millis()}})?
    };
    let models=models.into_iter().map(|model|CatalogModel {
        model_id:model.slug,display_name:model.display_name,
        protocol:nola_domain::generation::ModelProtocol::SiwcResponses,
        context_window:None,output_limit:None,modalities:vec!["text".into()],
        reasoning_levels:model.supported_reasoning_levels.into_iter().map(|level|level.effort).collect(),
        summary_supported:model.supports_reasoning_summary_parameter,
        execution_capabilities:None,
    }).collect();
    app.execute(ApplicationCommand::Model {command:ModelCommand::RefreshConnectionCatalog {connection_id:id,expected_revision:snapshot.revision,models,observed_at_ms:chrono::Utc::now().timestamp_millis()}})
}
pub(crate) fn execute(operation:&str, account:Option<&str>, revision:Option<i64>, label:Option<&str>, interactive:bool) -> Result<(),String> {
    let store=NolaStore::open_default()?;
    // Initialize only this canonical owner before opening its private sibling vault.
    let mut application=store.application()?;
    let _:Value=application.execute(ApplicationCommand::Model {command:ModelCommand::ListAccounts})?;
    drop(application);
    let host=SiwcHost::open(store.path())?;
    let id=account.map(str::to_owned).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
    let current=load(&store,&id)?;
    if operation!="sign-in" { owned(current.as_ref().ok_or("siwc_account_not_found")?,revision)?; }
    else if current.is_some() { return Err("siwc_account_already_exists".into()); }
    let baseline=host.snapshot(&id)?;
    if operation=="status" { return print(&json!({"account":current,"session":baseline})); }
    let session_revision=baseline.as_ref().map(|s|s.session_revision).unwrap_or(0);
    if operation=="models" {
        let models=host.models(&id,session_revision)?;
        // Recheck Core after network I/O before publishing this owner's observation.
        let owner=load(&store,&id)?.ok_or("siwc_account_not_found")?;owned(&owner,revision)?;
        let connection=sync_catalog(&store,&owner,models)?;
        return print(&json!({"account_id":id,"account_revision":revision,"connection":connection,"inference_performed":false,"user_action":"Use model configure with the returned connection revision and exact siwc_responses:model key to enable/select models. No model was automatically enabled."}));
    }
    let display_name=label.or_else(||current.as_ref().map(|a|a.account.display_name.as_str())).ok_or("siwc_account_label_missing")?;
    let session=match operation {
        "sign-in"|"reauthorize"=>{
            if !confirm(interactive,"Authorize Nola as an independent local open-source app with your ChatGPT account? This does not reuse Codex credentials or start inference.")? {return Ok(());}
            let attempt=host.begin_sign_in(&id,session_revision)?;
            let cancellation=attempt.cancellation();
            let signals = crate::execution_signals::ExecutionSignals::install_callback(move || cancellation.cancel())?;
            println!("Open this one-time authorization URL in a browser on this computer. Ctrl-C cancels local waiting; it does not revoke already granted access.\n{}",attempt.authorization_url());
            let result=host.finish_sign_in(attempt);drop(signals);result?
        },
        "refresh"=>host.refresh(&id,session_revision)?,
        "sign-out"=>{
            if !confirm(interactive,"Stop using this Nola account and revoke its renewable session? Other accounts and source applications are unchanged.")? {return Ok(());}
            host.sign_out(&id,session_revision)?
        },
        _=>return Err("unsupported model chatgpt operation".into()),
    };
    let account=save_account(&store,&id,display_name,current.as_ref().map(|a|a.revision),&session)
        .map_err(|_|format!("siwc_core_commit_failed: private session for {id} changed, but Core account was not updated; reload status before continuing"))?;
    let user_action=if session.revocation_unconfirmed {
        Some("Nola cleared this local session, but remote revocation is unconfirmed. Revoke Nola access in ChatGPT settings; repeating local sign-out cannot revoke a token that has already been cleared.")
    } else {None};
    print(&json!({"account":account,"session":session,"user_action":user_action,"inference_performed":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_siwc_cli_rejects_source_login_and_stale_core_owner() {
        let id=uuid::Uuid::new_v4().to_string();
        let mut value:ModelAccountSnapshot=serde_json::from_value(json!({"revision":2,"account":{"id":id,"display_name":"fixture","source":"manual","credential_ref":format!("siwc:{id}"),"state":"ready"}})).unwrap();
        assert!(owned(&value,Some(2)).is_err());
        value.account.source=serde_json::from_value(json!("nola_chatgpt")).unwrap();
        assert!(owned(&value,Some(1)).is_err());assert!(owned(&value,Some(2)).is_ok());
        value.account.credential_ref=Some("codex-owner".into());assert!(owned(&value,None).is_err());
        assert!(confirm(false,"must not prompt").is_err());
    }
    #[test]
    fn model_siwc_cli_catalog_preserves_unknown_and_explicit_override_reaches_runtime() {
        let root=tempfile::tempdir().unwrap();let store=NolaStore::open(&root.path().join("fixture.sqlite")).unwrap();
        let id=uuid::Uuid::new_v4().to_string();
        let session=Snapshot {account_id:id.clone(),session_revision:1,client_id:"fixture".into(),subject:"subject".into(),email:None,signed_in:true,plan_usage_authorized:true,reauthorization_required:false,revocation_unconfirmed:false};
        let owner=save_account(&store,&id,"fixture",None,&session).unwrap();
        let saved=sync_catalog(&store,&owner,vec![crate::model_siwc::ModelChoice {slug:"exact:slug".into(),display_name:"Fixture".into(),..Default::default()}]).unwrap();
        assert_eq!(saved.document.connection.account_id.as_deref(),Some(id.as_str()));
        assert!(store.model_profiles().unwrap().iter().all(|profile|!profile.enabled));
        let model=&saved.document.catalog.as_ref().unwrap().models[0];
        assert_eq!(model.catalog_key(),"siwc_responses:exact:slug");
        assert_eq!(model.execution_capabilities,None);
        let mut app=store.application().unwrap();
        let connection_id=saved.document.connection.id.clone();
        let configured:nola_domain::model_connection::ModelConnectionSnapshot=app.execute(ApplicationCommand::Model {command:ModelCommand::ConfigureConnectionModels {
            connection_id:connection_id.clone(),expected_revision:saved.revision,enabled_model_ids:vec![model.catalog_key()],selected_model_id:Some(model.catalog_key()),updated_at_ms:chrono::Utc::now().timestamp_millis(),
        }}).unwrap();
        let baseline=store.model_profiles().unwrap().into_iter().find(|p|p.enabled).unwrap();
        let baseline_source:Value=serde_json::from_str(&baseline.source_json).unwrap();
        assert_eq!(baseline_source["executionCapabilityBasis"],"protocol_agent_baseline");
        assert_eq!(baseline_source["executionCapabilities"]["toolCalls"],true);
        let declared=nola_domain::model_capabilities::ModelExecutionCapabilities {tool_calls:true,..Default::default()};
        let updated:nola_domain::model_connection::ModelConnectionSnapshot=app.execute(ApplicationCommand::Model {command:ModelCommand::SetConnectionCapabilityOverride {
            connection_id,expected_revision:configured.revision,model_id:"exact:slug".into(),protocol:nola_domain::generation::ModelProtocol::SiwcResponses,capabilities:Some(declared),updated_at_ms:chrono::Utc::now().timestamp_millis(),
        }}).unwrap();
        assert_eq!(updated.document.catalog.unwrap().models[0].execution_capabilities,None);
        let profile=store.model_profile(&baseline.id).unwrap().unwrap();
        let source:Value=serde_json::from_str(&profile.source_json).unwrap();
        assert_eq!(source["executionCapabilityBasis"],"user_override");
        let resolved=crate::model_profile_resolution::resolve_remote_profile_settings(&profile,&json!({}),&Default::default(),&[]).unwrap();
        assert_eq!(resolved.runtime.model_capabilities.native_tool_calls,Some(true));
    }
}
