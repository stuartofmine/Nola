// SPDX-License-Identifier: Apache-2.0
//! CLI mention completion. The draft carries the selected expert's exact identity
//! and revision so a restored draft cannot silently pick a replacement expert.
use crate::application_port::CliApplicationPort;
use nola_application::{
    agent_room::AgentRoomCommand, persistent_agent::AgentDirectory, ApplicationCommand,
};
use nola_domain::{
    expert_task_target::{MainExpertSelectionContextV1, MainExpertSelectionV1},
    persistent_agent::AgentAvailability,
};
use std::cell::RefCell;

const PREFIX: &str = "@expert[";
type BoundPrompt<'a> = (
    std::borrow::Cow<'a, str>,
    Option<MainExpertSelectionContextV1>,
);

#[derive(Clone)]
pub(crate) struct Candidate {
    pub token: String,
    pub label: String,
    name: String,
    offered: bool,
    selection: MainExpertSelectionV1,
}

impl Candidate {
    fn new(id: &str, revision: i64, name: &str) -> Self {
        let encoded_id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
        Self {
            token: format!("{PREFIX}{encoded_id}#{revision}]"),
            label: format!("@{name} · {}", expert_label()),
            name: name.to_owned(),
            offered: true,
            selection: MainExpertSelectionV1 {
                agent_id: id.into(),
                agent_revision: revision,
                participant_id: None,
                context_conversation_id: None,
            },
        }
    }
}

fn expert_label() -> &'static str {
    use crate::i18n::Language::*;
    match crate::i18n::current_language() {
        ZhHans => "专家",
        ZhHant => "專家",
        En => "expert",
        De => "Experte",
        Es => "experto",
        Fr => "expert",
        Ja => "専門家",
        Ko => "전문가",
        Ru => "эксперт",
    }
}

fn directory(session: Option<&str>) -> Result<Vec<Candidate>, String> {
    use nola_application::agent_room::ConversationRoomAssociation;
    let mut app = CliApplicationPort::persistent_default()?;
    let directory: AgentDirectory = app.execute(ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::Directory,
    })?;
    let mut candidates: Vec<Candidate> = directory
        .agents
        .iter()
        .filter(|agent| agent.availability() == &AgentAvailability::Active)
        .map(|agent| Candidate::new(agent.id().as_str(), agent.revision(), agent.name()))
        .collect();
    if let Some(id) = session {
        let association: Option<ConversationRoomAssociation> =
            app.execute(ApplicationCommand::AgentRoom {
                command: AgentRoomCommand::ForConversation {
                    conversation_id: id.to_owned(),
                },
            })?;
        if let Some(association) = association {
            for member in association
                .room
                .room
                .participants()
                .iter()
                .filter(|p| p.active)
            {
                // A draft can still hold the original directory reference after
                // this expert joins. Only offer the pinned member in the picker.
                for item in candidates
                    .iter_mut()
                    .filter(|item| item.selection.agent_id == member.agent_id.as_str())
                {
                    item.offered = false;
                }
                let mut item = Candidate::new(
                    member.agent_id.as_str(),
                    member.agent_revision,
                    &member.display_name,
                );
                let participant =
                    url::form_urlencoded::byte_serialize(member.id.as_bytes()).collect::<String>();
                item.token.pop();
                item.token.push_str(&format!("/{participant}]"));
                item.selection.participant_id = Some(member.id.clone());
                item.selection.context_conversation_id =
                    Some(member.context_conversation_id.clone());
                candidates.push(item);
            }
        }
    }
    let mut names = std::collections::HashMap::new();
    for item in candidates.iter().filter(|item| item.offered) {
        *names.entry(item.name.clone()).or_insert(0) += 1;
    }
    for item in candidates.iter_mut().filter(|item| item.offered) {
        if names[&item.name] > 1 {
            item.label
                .push_str(&format!(" · {}", item.selection.agent_id));
        }
    }
    Ok(candidates)
}

thread_local! {
    static COMPLETIONS: RefCell<Option<Vec<Candidate>>> = const { RefCell::new(None) };
}

pub(crate) fn begin_composer() {
    COMPLETIONS.with(|cache| cache.replace(None));
}

pub(crate) fn completions(prefix: &str, session: Option<&str>) -> Vec<Candidate> {
    COMPLETIONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let candidates = cache.get_or_insert_with(|| directory(session).unwrap_or_default());
        let query = prefix.to_lowercase();
        candidates
            .iter()
            .filter(|item| {
                item.offered
                    && (item.name.to_lowercase().contains(&query)
                        || item.selection.agent_id.to_lowercase().contains(&query))
            })
            .take(12)
            .cloned()
            .collect()
    })
}

pub(crate) fn submit<'a>(
    prompt: &'a str,
    session: Option<&str>,
) -> Result<BoundPrompt<'a>, String> {
    if !prompt
        .split_whitespace()
        .any(|word| word.starts_with(PREFIX))
    {
        return Ok((prompt.into(), None));
    }
    bind(prompt, &directory(session)?)
}

fn bind<'a>(prompt: &'a str, candidates: &[Candidate]) -> Result<BoundPrompt<'a>, String> {
    let mut text = String::with_capacity(prompt.len());
    let mut selections = Vec::new();
    for word in prompt.split_inclusive(char::is_whitespace) {
        if word.starts_with(PREFIX) {
            let (token, tail) = word
                .split_once(']')
                .ok_or("Incomplete expert mention. Use @ and Tab to choose an expert.")?;
            let entry = candidates.iter().find(|e| e.token.strip_suffix(']') == Some(token))
                .ok_or("The selected expert changed or is unavailable. Use @ and Tab to select the current expert.")?;
            if !selections.contains(&entry.selection) {
                selections.push(entry.selection.clone());
            }
            text.push('@');
            text.push_str(&entry.name);
            text.push_str(tail);
        } else {
            text.push_str(word);
        }
    }
    Ok((
        text.into(),
        Some(MainExpertSelectionContextV1 {
            version: 1,
            selected_experts: selections,
            expected_source_store_epoch: None,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_flow_typed_reference_keeps_revision_and_same_named_file_separate() {
        let expert = Candidate::new("with space]#", 4, "审查");
        let text = format!("{}，请看 @审查。\n再次选择 {}", expert.token, expert.token);
        let (prompt, selection) = bind(&text, std::slice::from_ref(&expert)).unwrap();
        assert_eq!(prompt, "@审查，请看 @审查。\n再次选择 @审查");
        assert_eq!(selection.unwrap().selected_experts, vec![expert.selection]);
        assert!(bind(&text, &[Candidate::new("with space]#", 5, "审查")]).is_err());
        assert!(submit("read @审查 and @./expert[file#2]", None)
            .unwrap()
            .1
            .is_none());
    }
}
