// SPDX-License-Identifier: Apache-2.0
//! Pin an authenticated Host transport's Room command to its actual Store.
//! Epoch validation and the existing command share the same transaction.
use crate::{ApplicationCommand, repositories::ApplicationCommandRepositories};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRoomSourceReply {
    pub source_store_epoch: String,
    pub source_matches: bool,
    pub result: Value,
}

pub fn execute<S: ApplicationCommandRepositories>(
    store: &mut S,
    expected: Option<String>,
    command: crate::agent_room::AgentRoomCommand,
) -> Result<AgentRoomSourceReply, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let epoch=store.assistant_source_epoch().map_err(|e|e.to_string())?;
        if expected.as_ref().is_some_and(|value|value.is_empty() || value!=&epoch) {
            return Ok(AgentRoomSourceReply {source_store_epoch:epoch,source_matches:false,result:Value::Null});
        }
        let result=crate::execute_application_command(store, ApplicationCommand::AgentRoom {command})?;
        Ok(AgentRoomSourceReply {source_store_epoch:epoch,source_matches:true,result})
    })
}
