// SPDX-License-Identifier: Apache-2.0
//! Native product control-plane commands. No terminal renderer or CLI dispatcher.
//! Server still consumes shared Web composition settings; invocation runtime does not.
use crate::config::{CommandMode, Config};
use serde_json::{json, Value};

pub(crate) fn run() -> Result<(), String> {
    if let Some(result) = crate::model_import_native_bridge::run_if_requested() { return result; }
    if let Some(result) = crate::model_siwc_native_bridge::run_if_requested() { return result; }
    if let Some(code) = crate::application_service_entry::run_if_requested() { std::process::exit(code); }
    if let Some(code) = crate::task_runtime::run_agent_worker_if_requested() { std::process::exit(code); }
    if let Some(code) = crate::task_runtime::run_shell_worker_if_requested() { std::process::exit(code); }
    crate::native_host_launch::configure_surface();
    if let Some(code) = crate::external_executor::run_catalog_command_if_requested() { std::process::exit(code); }
    if let Some(code) = crate::memory_worker::run_if_requested() { std::process::exit(code); }
    #[cfg(target_os = "macos")]
    if let Some(code) = crate::background_scheduler::run_if_requested() { std::process::exit(code); }
    if let Some(code) = crate::memory_admin::run_if_requested() { std::process::exit(code); }
    let mut config = Config::from_args_with_setup(
        std::env::args().skip(1), &crate::configuration_setup::NoConfigurationSetup,
    )?;
    if config.show_help {
        println!("nola-agent-host: native stdin run, server, sessions, session show/part/sources/delete");
        return Ok(());
    }
    validate_command(&config.command)?;
    config.permission_profile = Some(config.permission_profile.unwrap_or(crate::permissions::PermissionProfile::Cautious));
    if matches!(config.command, CommandMode::Server | CommandMode::SessionDelete(_)) {
        crate::session_process_observation::cleanup_stale_session_run_locks();
    }
    match &config.command {
        CommandMode::Server => {
            if std::env::var_os("NOLA_MODELS_ROOT").is_none() {
                if let Ok(root) = crate::local_models::models_root() { std::env::set_var("NOLA_MODELS_ROOT", root); }
            }
            let result = crate::web::run_web_server(&config);
            crate::local_models::shutdown();
            result
        },
        CommandMode::Sessions => {
            let sessions = crate::session::list_sessions()?;
            for saved in &sessions { let _ = crate::session_process_observation::session_running_process(&saved.id); }
            write_json(json!({"type":"sessions", "sessions":sessions}))
        },
        CommandMode::SessionShow(id) => write_json(session_projection(id)?),
        CommandMode::SessionPart {conversation_id, message_id, part_id, revision} => {
            let owner = nola_protocol::CanonicalPartOwner::new(conversation_id.clone(), message_id.clone(), part_id.clone())
                .map_err(|error| error.to_string())?;
            let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
            write_json(serde_json::to_value(crate::historical_part::read(&mut application, owner, *revision)?)
                .map_err(|error| error.to_string())?)
        },
        CommandMode::SessionSources {conversation_id, compaction_part_id} => {
            let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
            write_json(application.load_compaction_lineage(conversation_id, compaction_part_id.as_deref())?)
        },
        CommandMode::SessionDelete(id) => {
            crate::core_agent_runtime::end_agent_core_session(id)?;
            crate::session::delete_session(id)?;
            write_json(json!({"type":"session_deleted", "id":id}))
        },
        _ => Err("nola-agent-host requires native stdin for runs; unsupported product command".into()),
    }
}

fn validate_command(command: &CommandMode) -> Result<(), String> {
    match command {
        CommandMode::Server | CommandMode::Sessions | CommandMode::SessionShow(_)
        | CommandMode::SessionPart { .. } | CommandMode::SessionSources { .. }
        | CommandMode::SessionDelete(_) => Ok(()),
        _ => Err("nola-agent-host requires native stdin for runs; unsupported product command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_plane_accepts_existing_native_consumers_but_not_terminal_commands() {
        for command in [CommandMode::Server, CommandMode::Sessions, CommandMode::SessionShow("session".into()), CommandMode::SessionDelete("session".into())] {
            assert!(validate_command(&command).is_ok());
        }
        for command in [CommandMode::Run, CommandMode::Init, CommandMode::Wiki, CommandMode::Web] {
            assert!(validate_command(&command).is_err());
        }
    }
}

fn write_json(value: Value) -> Result<(), String> {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &value).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())
}

fn session_projection(id: &str) -> Result<Value, String> {
    let _ = crate::session_process_observation::session_running_process(id);
    let saved = crate::session::load_session(id)?;
    let traces = crate::session::load_session_trace_runs(id)?;
    let messages = crate::session::load_session_messages_with_ids(id)?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let conversation = application.load_conversation_projection_v1(id)?;
    let projection = application.load_session_projection(id)?;
    let skills = projection.as_ref().map(|projection| {
        let runtime = &projection.skill_runtime_projection;
        crate::session::SessionSkillProjection {
            session_id: runtime.session_id.clone(), cursor: runtime.cursor, revision: runtime.revision,
            catalog_revision: runtime.catalog_revision, order: runtime.order.clone(), runs: runtime.runs.clone(),
        }
    });
    Ok(crate::session_projection_output::session_show_json_value(
        &saved, &messages, &traces, &conversation,
        projection.as_ref().map(|value| &value.task_projection),
        projection.as_ref().map(|value| &value.memory_runtime_projection),
        projection.as_ref().map(|value| &value.goal_projection),
        projection.as_ref().map(|value| &value.permission_projection), skills.as_ref(),
    ))
}
