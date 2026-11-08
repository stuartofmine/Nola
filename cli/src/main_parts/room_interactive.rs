// SPDX-License-Identifier: Apache-2.0
// Explicit room syntax only; natural-language content never selects recipients.
fn interactive_room_snapshot(id: &str) -> Result<nola_application::agent_room::RoomSnapshot,String> {
    let value: Option<nola_application::agent_room::RoomSnapshot> = application_port::CliApplicationPort::persistent_default()?
        .execute(nola_application::ApplicationCommand::AgentRoom { command:nola_application::agent_room::AgentRoomCommand::Read {room_id:id.into()} })?;
    value.ok_or("Room not found".into())
}

fn room_mention_token(member: &nola_domain::agent_room::RoomParticipant, members: &[nola_domain::agent_room::RoomParticipant]) -> String {
    // Duplicate or whitespace-containing names use the stable candidate ID.
    // Tab still presents the public name, and submit resolves only exact tokens.
    if !member.display_name.chars().any(char::is_whitespace) && !member.display_name.starts_with('@')
        && members.iter().filter(|other|other.active && other.display_name == member.display_name).count() == 1 {
        member.display_name.clone()
    } else { member.id.clone() }
}

fn room_explicit_recipient<'a>(text: &'a str, members: &[nola_domain::agent_room::RoomParticipant]) -> Result<(Option<String>, &'a str),String> {
    let Some(mention) = text.strip_prefix('@') else { return Ok((None,text)); };
    let (token,body) = mention.split_once(char::is_whitespace).ok_or("Choose an @recipient with Tab, then enter a message")?;
    let mut matching = members.iter().filter(|member|member.active && room_mention_token(member,members) == token);
    let member = matching.next()
        .ok_or("Unknown or ambiguous @recipient; use the room candidates")?;
    if matching.next().is_some() { return Err("Ambiguous @recipient; use a unique room candidate".into()); }
    let body=body.trim_start();
    if body.is_empty() || body.starts_with('@') { return Err("Choose one recipient and enter a message".into()); }
    Ok((Some(member.id.clone()),body))
}

fn interactive_room_completions(rest: &str) -> Vec<SlashCompletion> {
    let Some((id,tail)) = rest.split_once(char::is_whitespace) else { return vec![]; };
    let Some(query)=tail.trim_start().strip_prefix('@') else { return vec![]; };
    if query.chars().any(char::is_whitespace) { return vec![]; }
    let Ok(room)=interactive_room_snapshot(id) else { return vec![]; };
    room.room.participants().iter().filter(|member|member.active)
        .filter(|member|member.display_name.to_lowercase().starts_with(&query.to_lowercase()) || member.id.starts_with(query))
        .map(|member|SlashCompletion {name:format!("/room {id} @{} ",room_mention_token(member,room.room.participants()))})
        .take(4).collect()
}

fn interactive_room_completion_label(completion: &str) -> Option<String> {
    let rest=completion.strip_prefix("/room ")?;
    let (id,tail)=rest.split_once(' ')?;
    let token=tail.trim().strip_prefix('@')?;
    let room=interactive_room_snapshot(id).ok()?;
    let member=room.room.participants().iter().find(|member|member.active && room_mention_token(member,room.room.participants())==token)?;
    Some(if token == member.display_name {
        format!("@{}", member.display_name)
    } else {
        // Equal public names must remain distinguishable in the candidate row.
        format!("@{} · {}", member.display_name, token)
    })
}

fn interactive_room_command(rest: &str, config: &Config) -> Result<(),String> {
    use nola_application::{ApplicationCommand,agent_room::{AgentRoomCommand,RoomSnapshot,RoomSubmission,SubmitRoomMessage}};
    let mut app=application_port::CliApplicationPort::persistent_default()?;
    if rest.is_empty() {
        let rooms:Vec<RoomSnapshot>=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::List {limit:100,offset:0}})?;
        for room in rooms { terminal_notice!("  {}  {}",room.room.id(),room.room.title()); }
        terminal_notice!("  /room <room-id> [@recipient] <message> · Tab chooses a candidate; no @ uses the coordinator");
        return Ok(());
    }
    let (id,text)=rest.split_once(char::is_whitespace).unwrap_or((rest,""));
    let room=interactive_room_snapshot(id)?;
    if text.trim().is_empty() {
        terminal_notice!("  {}",room.room.title());
        for member in room.room.participants().iter().filter(|member|member.active) {
            let token = room_mention_token(member,room.room.participants());
            if token == member.display_name { terminal_notice!("  @{}", member.display_name); }
            else { terminal_notice!("  @{} — {}", token, member.display_name); }
        }
        terminal_notice!("  Type /room {id} @ then a name prefix and Tab; omit @ for automatic routing");
        return Ok(());
    }
    let (participant_id,text)=room_explicit_recipient(text.trim_start(),room.room.participants())?;
    let submission_id=uuid::Uuid::new_v4().to_string();
    terminal_notice!("  Sending to room {}…",room.room.title());
    let accepted:RoomSubmission=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::Submit {input:SubmitRoomMessage {
        room_id:id.into(),submission_id:submission_id.clone(),expected_room_revision:room.revision,text:text.into(),participant_id,at_ms:chrono::Utc::now().timestamp_millis(),parent_message_id:None,conversation_id:None,
    }}})?;
    terminal_notice!("  Accepted · processing…");
    let cancellation=interaction::ExecutionCancellation::interactive();
    let _scope=cancellation.enter();
    let _signals=crate::execution_signals::ExecutionSignals::install(cancellation.runtime_control(),None)?;
    let listener=interaction::start_execution_cancel_listener(&cancellation);
    let execution=crate::web::run_room_submission_with_permission(config,accepted,Some(interactive_room_permission));
    drop(listener);
    cancellation.wait_for_terminal_listener_release();
    execution?;
    let intent:Option<nola_application::agent_room::RoomExecutionIntent>=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::ExecutionIntent {room_id:id.into(),submission_id:submission_id.clone()}})?;
    let value:RoomSubmission=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::ReconcileExecution {room_id:id.into(),submission_id,at_ms:chrono::Utc::now().timestamp_millis()}})?;
    let recipient=room.room.participants().iter().find(|member|Some(&member.id)==value.selected_participant_id.as_ref());
    terminal_notice!("  @{} · {:?}",recipient.map(|member|member.display_name.as_str()).unwrap_or("—"),value.state);
    if let Some(context)=value.context_conversation_id {
        if let Some(intent)=intent {
            let assistant=format!("{}:assistant",intent.execution_id);
            for message in session::load_session_messages_with_ids(&context)? {
                if message["id"].as_str()==Some(assistant.as_str()) {
                    if let Some(text)=message["content"].as_str() { terminal_notice!("{}",text); }
                }
            }
        }
        terminal_notice!("  /resume {context}  — full context and pending permissions");
    }
    Ok(())
}

fn interactive_room_permission(event: &serde_json::Value) -> Result<serde_json::Value,String> {
    use crate::runtime_interaction::{PermissionPrompt,PermissionPromptKind};
    let challenge: nola_core::host_auth::HostPermissionChallenge = serde_json::from_value(event.clone())
        .map_err(|_| "Invalid room permission challenge")?;
    if challenge.protocol_version != nola_core::host_auth::HOST_PERMISSION_PROTOCOL_VERSION
        || challenge.expires_at <= u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0) {
        return Err("Room permission challenge is expired or unsupported".into());
    }
    let review = event["safe_review_text"].as_str().filter(|text|!text.trim().is_empty())
        .ok_or("Room operation has no safe review details; open its context to inspect")?;
    let prompt = PermissionPrompt {
        allow_conversation:challenge.allow_conversation, kind:PermissionPromptKind::Tool,
        title:format!("Room · {} · awaiting permission",challenge.tool_name),
        rows:vec![("Context".into(),challenge.session_id.clone()),("Operation".into(),review.into())],
        session_label:"Allow for this context".into(),session_value:challenge.session_id.clone(),
        custom_label:"Provide instruction".into(),supports_instruction:true,
    };
    let resolution = room_permission_resolution(interaction::prompt_permission(&prompt))?;
    Ok(serde_json::json!({"protocol_version":challenge.protocol_version,"nonce":challenge.nonce,
        "request_id":challenge.request_id,"host_id":challenge.host_id,"client_id":challenge.client_id,
        "session_id":challenge.session_id,"tool_name":challenge.tool_name,"resolution":resolution}))
}

fn room_permission_resolution(choice: crate::runtime_interaction::PermissionChoice) -> Result<serde_json::Value,String> {
    use crate::runtime_interaction::PermissionChoice;
    Ok(match choice {
        PermissionChoice::Once => serde_json::json!({"type":"authorization","decision":"allow_once"}),
        PermissionChoice::Session(_) => serde_json::json!({"type":"authorization","decision":"allow_conversation"}),
        PermissionChoice::Deny => serde_json::json!({"type":"authorization","decision":"deny"}),
        PermissionChoice::ReturnToComposer => serde_json::json!({"type":"intervention","intervention":{"type":"return_to_composer"}}),
        PermissionChoice::Custom(instruction) => serde_json::json!({"type":"intervention","intervention":{"type":"provide_instruction","instruction":instruction}}),
        PermissionChoice::TransportInterrupted(_) => return Err("Room permission input interrupted; inspect the context before continuing".into()),
    })
}

#[cfg(test)]
mod interactive_room_mentions_tests {
    use super::*;
    #[test]
    fn room_permission_choices_use_canonical_resolution_without_implicit_approval() {
        use crate::runtime_interaction::PermissionChoice;
        for choice in [PermissionChoice::Once,PermissionChoice::Session("context".into()),PermissionChoice::Deny,
            PermissionChoice::ReturnToComposer,PermissionChoice::Custom("do not write".into())] {
            let resolution=room_permission_resolution(choice).unwrap();
            assert!(serde_json::from_value::<nola_host_protocol::HostPermissionResolution>(resolution).is_ok());
        }
        assert_eq!(room_permission_resolution(PermissionChoice::ReturnToComposer).unwrap()["type"],"intervention");
        assert_eq!(room_permission_resolution(PermissionChoice::Deny).unwrap()["decision"],"deny");
        assert!(room_permission_resolution(PermissionChoice::TransportInterrupted(
            nola_host_protocol::PermissionTransportInterruptionReason::Disconnected)).is_err());
    }
    fn member(id:&str,name:&str)->nola_domain::agent_room::RoomParticipant {
        nola_domain::agent_room::RoomParticipant {id:id.into(),agent_id:nola_domain::persistent_agent::AgentId::try_from(id.to_owned()).unwrap(),agent_revision:1,display_name:name.into(),description:String::new(),context_conversation_id:format!("context-{id}"),active:true}
    }
    #[test]
    fn only_explicit_leading_candidate_changes_recipient() {
        let members=vec![member("one","Research"),member("two","写作")];
        for text in ["search 当前 follow-up","email a@b.com","translate @Research literally"] {
            assert_eq!(room_explicit_recipient(text,&members).unwrap(),(None,text));
        }
        assert_eq!(room_explicit_recipient("@写作 你好",&members).unwrap(),(Some("two".into()),"你好"));
        assert!(room_explicit_recipient("@unknown hello",&members).is_err());
        assert!(room_explicit_recipient("@Research @写作 hello",&members).is_err());
    }
    #[test]
    fn duplicate_names_require_exact_candidate_identity() {
        let members=vec![member("one","Same"),member("two","Same")];
        assert!(room_explicit_recipient("@Same hello",&members).is_err());
        assert_eq!(room_explicit_recipient("@two hello",&members).unwrap().0,Some("two".into()));
    }
}
