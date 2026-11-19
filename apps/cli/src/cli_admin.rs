// SPDX-License-Identifier: Apache-2.0
//! Pre-Config CLI administration commands (`skills`, `mcp`, `memory`, `update`, `version`)
//! supporting leading `--lang` / `--json` options without requiring an LLM endpoint.

use crate::application_port::CliApplicationPort;
use crate::i18n::{parse_language_tag, set_language};
use chrono::Utc;
use nola_application::{ApplicationCommand, McpCommand, McpServer, Skill, SkillCommand};
use serde_json::{json, Value};

#[derive(Debug, PartialEq, Eq)]
struct LeadingContext {
    json_output: bool,
    language: Option<crate::i18n::Language>,
    remaining: Vec<String>,
}

fn parse_leading_context(args: &[String]) -> Result<LeadingContext, String> {
    let mut json_output = false;
    let mut language = None;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            break;
        }
        if arg == "--json" {
            json_output = true;
            index += 1;
            continue;
        }
        if arg == "--lang" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--lang requires a language tag".to_string())?;
            language = Some(
                parse_language_tag(value)
                    .ok_or_else(|| format!("--lang unsupported language: {value}"))?,
            );
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--lang=") {
            language = Some(
                parse_language_tag(value)
                    .ok_or_else(|| format!("--lang unsupported language: {value}"))?,
            );
            index += 1;
            continue;
        }
        break;
    }

    Ok(LeadingContext {
        json_output,
        language,
        remaining: args[index..].to_vec(),
    })
}

fn is_admin_dispatch_target(remaining: &[String]) -> bool {
    matches!(
        remaining.first().map(String::as_str),
        Some("version" | "--version" | "-V" | "update" | "memory" | "skills" | "mcp")
    )
}

pub(crate) fn run_if_requested(args: &[String]) -> Option<Result<(), String>> {
    let context = match parse_leading_context(args) {
        Ok(context) => context,
        Err(error) => return Some(Err(error)),
    };
    if !is_admin_dispatch_target(&context.remaining) {
        return None;
    }
    if let Some(language) = context.language {
        set_language(language);
    }

    match context.remaining.first().map(String::as_str) {
        Some("version" | "--version" | "-V") => {
            if context.remaining.len() == 1
                || (context.remaining.len() == 2
                    && matches!(context.remaining[1].as_str(), "--help" | "-h"))
            {
                if context.remaining.get(1).is_some() {
                    println!("Usage:\n  nola version\n  nola --version | -V");
                    return Some(Ok(()));
                }
                return Some(run_version(context.json_output));
            }
            if context.remaining.first().map(String::as_str) == Some("version") {
                return Some(Err("Usage: nola version [--json]".to_string()));
            }
            None
        }
        Some("update") => crate::software_update::run_if_requested(&context.remaining),
        Some("memory") => Some(crate::memory_admin::run_cli_args(
            &context.remaining[1..],
            context.json_output,
        )),
        Some("skills") => Some(run_skills_command(
            &context.remaining[1..],
            context.json_output,
        )),
        Some("mcp") => Some(run_mcp_command(
            &context.remaining[1..],
            context.json_output,
        )),
        _ => None,
    }
}

fn run_version(json_output: bool) -> Result<(), String> {
    let source = nola_update_host::UpdateSource::bundled().map_err(|err| err.to_string())?;
    if json_output {
        println!(
            "{}",
            json!({
                "name": "Nola",
                "version": source.version,
                "build": source.build,
            })
        );
    } else {
        println!("Nola {} ({})", source.version, source.build);
    }
    Ok(())
}

fn strip_admin_flags(
    args: &[String],
    mut json_output: bool,
) -> Result<(Vec<String>, bool, bool), String> {
    let mut positional = Vec::new();
    let mut show_help = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            positional.extend(args[index + 1..].iter().cloned());
            break;
        }
        if arg == "-h" || arg == "--help" {
            show_help = true;
            index += 1;
        } else if arg == "--json" {
            json_output = true;
            index += 1;
        } else if arg == "--lang" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--lang requires a language tag".to_string())?;
            let language = parse_language_tag(value)
                .ok_or_else(|| format!("--lang unsupported language: {value}"))?;
            set_language(language);
            index += 2;
        } else if let Some(value) = arg.strip_prefix("--lang=") {
            let language = parse_language_tag(value)
                .ok_or_else(|| format!("--lang unsupported language: {value}"))?;
            set_language(language);
            index += 1;
        } else {
            positional.push(arg.clone());
            index += 1;
        }
    }
    Ok((positional, json_output, show_help))
}

fn skills_usage() -> &'static str {
    "Usage:\n  nola skills [list] [--json]\n  nola skills enable <id> [--json]\n  nola skills disable <id> [--json]"
}

fn run_skills_command(args: &[String], leading_json: bool) -> Result<(), String> {
    let (positional, json_output, show_help) = strip_admin_flags(args, leading_json)?;
    if show_help {
        println!("{}", skills_usage());
        return Ok(());
    }
    let mut application = CliApplicationPort::persistent_default()?;
    execute_skills_command(&mut application, &positional, json_output)
}

fn execute_skills_command(
    application: &mut CliApplicationPort,
    positional: &[String],
    json_output: bool,
) -> Result<(), String> {
    let builtins = crate::skill_catalog_port::load_web_builtin_skills_with(application)?;
    let uploaded = crate::skill_catalog_port::load_web_uploaded_skills_with(application)?;
    let mut combined = Vec::with_capacity(builtins.len() + uploaded.len());
    combined.extend(builtins);
    combined.extend(uploaded);

    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    match refs.as_slice() {
        [] | ["list"] => {
            if json_output {
                println!("{}", json!({ "type": "skills", "skills": combined }));
            } else if combined.is_empty() {
                println!("No skills found.");
            } else {
                for item in &combined {
                    let id = item["id"].as_str().unwrap_or("");
                    let name = item["name"].as_str().unwrap_or(id);
                    let enabled = item["enabled"].as_bool().unwrap_or(true);
                    let state = if enabled { "enabled" } else { "disabled" };
                    println!("[{state}] {id} ({name})");
                }
            }
            Ok(())
        }
        [action @ ("enable" | "disable"), target_id] => {
            let enabled = *action == "enable";
            set_skill_enabled(application, target_id, enabled, json_output)
        }
        _ => Err(skills_usage().to_string()),
    }
}

fn set_skill_enabled(
    application: &mut CliApplicationPort,
    target_id: &str,
    enabled: bool,
    json_output: bool,
) -> Result<(), String> {
    let editors: Vec<Value> = application.execute(ApplicationCommand::Skill {
        command: SkillCommand::ListEditors,
    })?;
    let mut parsed = Vec::new();
    for editor in editors {
        let revision = editor["revision"]
            .as_i64()
            .ok_or_else(|| "skill editor revision missing".to_string())?;
        let skill: Skill = serde_json::from_value(editor["record"].clone())
            .map_err(|error| error.to_string())?;
        parsed.push((skill, revision));
    }
    let (matched_id, revision) = parsed
        .iter()
        .find(|(skill, _)| skill.id == target_id)
        .or_else(|| {
            parsed
                .iter()
                .find(|(skill, _)| skill.display_name.eq_ignore_ascii_case(target_id))
        })
        .map(|(skill, revision)| (skill.id.clone(), *revision))
        .ok_or_else(|| format!("skill '{target_id}' not found"))?;

    let updated: Value = application.execute(ApplicationCommand::Skill {
        command: SkillCommand::SetEnabledChecked {
            id: matched_id.clone(),
            enabled,
            expected_revision: revision,
            updated_at_ms: Utc::now().timestamp_millis(),
        },
    })?;
    let new_revision = updated["revision"].as_i64().unwrap_or(revision + 1);
    if json_output {
        println!(
            "{}",
            json!({
                "type": "skill_updated",
                "id": matched_id,
                "enabled": enabled,
                "revision": new_revision,
            })
        );
    } else {
        let state = if enabled { "enabled" } else { "disabled" };
        println!("Skill '{matched_id}' {state} (revision {new_revision}).");
    }
    Ok(())
}

fn mcp_usage() -> &'static str {
    "Usage:\n  nola mcp [list] [--json]\n  nola mcp add <name> <command-or-url> [args...] [--json]\n  nola mcp remove <id-or-name> [--json]\n  nola mcp enable <id-or-name> [--json]\n  nola mcp disable <id-or-name> [--json]"
}

fn run_mcp_command(args: &[String], leading_json: bool) -> Result<(), String> {
    let (positional, json_output, show_help) = strip_admin_flags(args, leading_json)?;
    if show_help {
        println!("{}", mcp_usage());
        return Ok(());
    }
    let mut application = CliApplicationPort::persistent_default()?;
    execute_mcp_command(&mut application, &positional, json_output)
}

fn execute_mcp_command(
    application: &mut CliApplicationPort,
    positional: &[String],
    json_output: bool,
) -> Result<(), String> {
    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    match refs.as_slice() {
        [] | ["list"] => list_mcp_servers(application, json_output),
        ["add", name, target, extra @ ..] => {
            add_mcp_server(application, name, target, extra, json_output)
        }
        ["remove", id_or_name] => remove_mcp_server(application, id_or_name, json_output),
        [action @ ("enable" | "disable"), id_or_name] => {
            let enabled = *action == "enable";
            set_mcp_enabled(application, id_or_name, enabled, json_output)
        }
        _ => Err(mcp_usage().to_string()),
    }
}

fn fetch_mcp_servers(application: &mut CliApplicationPort) -> Result<Vec<McpServer>, String> {
    let servers: Vec<McpServer> = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::List,
    })?;
    Ok(servers
        .into_iter()
        .filter(|server| server.deleted_at_ms.is_none() && !server.id.contains(":mcp:"))
        .collect())
}

fn find_mcp_server<'a>(servers: &'a [McpServer], id_or_name: &str) -> Result<&'a McpServer, String> {
    servers
        .iter()
        .find(|server| server.id == id_or_name)
        .or_else(|| {
            servers
                .iter()
                .find(|server| server.display_name == id_or_name)
        })
        .ok_or_else(|| format!("MCP server '{id_or_name}' not found"))
}

fn mcp_server_summary(server: &McpServer) -> Value {
    let config = serde_json::from_str::<Value>(&server.configuration_json).unwrap_or(Value::Null);
    json!({
        "id": server.id,
        "name": server.display_name,
        "enabled": server.enabled,
        "revision": server.revision,
        "transport": config.get("type").and_then(Value::as_str).unwrap_or("unknown"),
        "target": config
            .get("command")
            .or_else(|| config.get("endpoint"))
            .or_else(|| config.get("url"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        "args": config.get("args").cloned().unwrap_or_else(|| json!([])),
    })
}

fn list_mcp_servers(
    application: &mut CliApplicationPort,
    json_output: bool,
) -> Result<(), String> {
    let servers = fetch_mcp_servers(application)?;
    let items: Vec<Value> = servers.iter().map(mcp_server_summary).collect();
    if json_output {
        println!("{}", json!({ "type": "mcp_servers", "servers": items }));
    } else if items.is_empty() {
        println!("No MCP servers configured.");
    } else {
        for item in &items {
            let state = if item["enabled"].as_bool().unwrap_or(true) {
                "enabled"
            } else {
                "disabled"
            };
            let id = item["id"].as_str().unwrap_or("");
            let name = item["name"].as_str().unwrap_or(id);
            let target = item["target"].as_str().unwrap_or("");
            println!("[{state}] {id} ({name}) -> {target}");
        }
    }
    Ok(())
}

fn add_mcp_server(
    application: &mut CliApplicationPort,
    name: &str,
    target: &str,
    extra: &[&str],
    json_output: bool,
) -> Result<(), String> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() || trimmed_name.chars().any(char::is_control) {
        return Err("MCP server name must be non-empty".to_string());
    }
    let trimmed_target = target.trim();
    if trimmed_target.is_empty() {
        return Err("MCP server command or URL must be non-empty".to_string());
    }
    let existing = fetch_mcp_servers(application)?;
    if existing
        .iter()
        .any(|server| server.id == trimmed_name || server.display_name == trimmed_name)
    {
        return Err(format!("MCP server '{trimmed_name}' already exists"));
    }

    let lower = trimmed_target.to_ascii_lowercase();
    let config_value = if lower.starts_with("http://") || lower.starts_with("https://") {
        if !extra.is_empty() {
            return Err(
                "HTTP MCP servers accept a URL without command arguments".to_string(),
            );
        }
        let parsed = reqwest::Url::parse(trimmed_target)
            .map_err(|err| format!("invalid MCP URL '{trimmed_target}': {err}"))?;
        if parsed.host_str().is_none() {
            return Err("MCP URL requires a valid host".to_string());
        }
        json!({
            "server_id": trimmed_name,
            "name": trimmed_name,
            "type": "streamable_http",
            "endpoint": trimmed_target,
            "url": trimmed_target,
            "enabled": true,
        })
    } else {
        json!({
            "server_id": trimmed_name,
            "name": trimmed_name,
            "type": "stdio",
            "command": trimmed_target,
            "args": extra,
            "enabled": true,
        })
    };

    let configuration_json =
        serde_json::to_string(&config_value).map_err(|error| error.to_string())?;
    let created: McpServer = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::Create {
            id: trimmed_name.to_string(),
            display_name: trimmed_name.to_string(),
            enabled: true,
            configuration_json,
            created_at_ms: Utc::now().timestamp_millis(),
        },
    })?;
    if json_output {
        println!(
            "{}",
            json!({
                "type": "mcp_server_added",
                "server": mcp_server_summary(&created),
            })
        );
    } else {
        println!(
            "Added MCP server '{}' (revision {}).",
            created.id, created.revision
        );
    }
    Ok(())
}

fn remove_mcp_server(
    application: &mut CliApplicationPort,
    id_or_name: &str,
    json_output: bool,
) -> Result<(), String> {
    let servers = fetch_mcp_servers(application)?;
    let server = find_mcp_server(&servers, id_or_name)?;
    let id = server.id.clone();
    let expected_revision = server.revision;
    let _: Value = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::DeleteChecked {
            id: id.clone(),
            expected_revision,
            deleted_at_ms: Utc::now().timestamp_millis(),
        },
    })?;
    if json_output {
        println!("{}", json!({ "type": "mcp_server_removed", "id": id }));
    } else {
        println!("Removed MCP server '{id}'.");
    }
    Ok(())
}

fn set_mcp_enabled(
    application: &mut CliApplicationPort,
    id_or_name: &str,
    enabled: bool,
    json_output: bool,
) -> Result<(), String> {
    let servers = fetch_mcp_servers(application)?;
    let server = find_mcp_server(&servers, id_or_name)?;
    let id = server.id.clone();
    let display_name = server.display_name.clone();
    let expected_revision = server.revision;
    let mut config_value =
        serde_json::from_str::<Value>(&server.configuration_json).unwrap_or_else(|_| json!({}));
    if let Some(obj) = config_value.as_object_mut() {
        obj.insert("enabled".to_string(), Value::Bool(enabled));
    }
    let configuration_json =
        serde_json::to_string(&config_value).map_err(|error| error.to_string())?;
    let updated: McpServer = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::Configure {
            id: id.clone(),
            expected_revision,
            display_name,
            enabled,
            configuration_json,
            updated_at_ms: Utc::now().timestamp_millis(),
        },
    })?;
    if json_output {
        println!(
            "{}",
            json!({
                "type": "mcp_server_updated",
                "server": mcp_server_summary(&updated),
            })
        );
    } else {
        let state = if enabled { "enabled" } else { "disabled" };
        println!(
            "MCP server '{id}' {state} (revision {}).",
            updated.revision
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_flags_route_admin_commands_and_preserve_regular_commands() {
        let parsed = parse_leading_context(&[
            "--lang".into(),
            "zh".into(),
            "--json".into(),
            "skills".into(),
            "list".into(),
        ])
        .unwrap();
        assert!(parsed.json_output);
        assert_eq!(parsed.language, Some(crate::i18n::Language::ZhHans));
        assert_eq!(parsed.remaining, vec!["skills", "list"]);
        assert!(is_admin_dispatch_target(&parsed.remaining));

        // Double dash stops leading parsing so `nola -- version` runs LLM chat.
        let dash = parse_leading_context(&["--".into(), "version".into()]).unwrap();
        assert!(!is_admin_dispatch_target(&dash.remaining));

        // Unknown flags stay untouched for Config::from_args.
        let model_flag =
            parse_leading_context(&["--model".into(), "gpt-4o".into(), "hello".into()]).unwrap();
        assert!(!is_admin_dispatch_target(&model_flag.remaining));

        // Trailing version with leading lang works cleanly.
        let ver = parse_leading_context(&["--lang=ko".into(), "-V".into()]).unwrap();
        assert!(is_admin_dispatch_target(&ver.remaining));
        assert_eq!(ver.language, Some(crate::i18n::Language::Ko));
    }

    #[test]
    fn skills_enable_disable_persists_in_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("skills-admin.sqlite");
        let mut application = CliApplicationPort::persistent(&db).unwrap();

        execute_skills_command(&mut application, &["list".into()], true).unwrap();
        let builtins = crate::skill_catalog_port::load_web_builtin_skills_with(&mut application).unwrap();
        let first_id = builtins[0]["id"].as_str().unwrap().to_string();

        execute_skills_command(
            &mut application,
            &["disable".into(), first_id.clone()],
            false,
        )
        .unwrap();
        let after_disable =
            crate::skill_catalog_port::load_web_builtin_skills_with(&mut application).unwrap();
        let entry = after_disable
            .iter()
            .find(|item| item["id"] == first_id)
            .unwrap();
        assert_eq!(entry["enabled"], false);

        execute_skills_command(
            &mut application,
            &["enable".into(), first_id.clone()],
            false,
        )
        .unwrap();
        let after_enable =
            crate::skill_catalog_port::load_web_builtin_skills_with(&mut application).unwrap();
        let entry = after_enable
            .iter()
            .find(|item| item["id"] == first_id)
            .unwrap();
        assert_eq!(entry["enabled"], true);
    }

    #[test]
    fn mcp_add_toggle_and_remove_persists_in_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("mcp-admin.sqlite");
        let mut application = CliApplicationPort::persistent(&db).unwrap();

        execute_mcp_command(
            &mut application,
            &[
                "add".into(),
                "fs".into(),
                "npx".into(),
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
                "/tmp".into(),
            ],
            false,
        )
        .unwrap();
        let listed = fetch_mcp_servers(&mut application).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "fs");
        assert!(listed[0].enabled);

        execute_mcp_command(&mut application, &["disable".into(), "fs".into()], false).unwrap();
        let listed = fetch_mcp_servers(&mut application).unwrap();
        assert!(!listed[0].enabled);

        execute_mcp_command(&mut application, &["enable".into(), "fs".into()], false).unwrap();
        let listed = fetch_mcp_servers(&mut application).unwrap();
        assert!(listed[0].enabled);

        execute_mcp_command(&mut application, &["remove".into(), "fs".into()], false).unwrap();
        let listed = fetch_mcp_servers(&mut application).unwrap();
        assert!(listed.is_empty());

        // HTTP URL validation
        execute_mcp_command(
            &mut application,
            &[
                "add".into(),
                "remote".into(),
                "https://mcp.example.invalid/mcp".into(),
            ],
            true,
        )
        .unwrap();
        let listed = fetch_mcp_servers(&mut application).unwrap();
        assert_eq!(listed.len(), 1);
        let summary = mcp_server_summary(&listed[0]);
        assert_eq!(summary["transport"], "streamable_http");
    }
}
