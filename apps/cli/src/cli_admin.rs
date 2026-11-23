// SPDX-License-Identifier: Apache-2.0
//! Pre-Config CLI administration commands (`skills`, `mcp`, `memory`, `update`, `version`)
//! supporting leading `--lang` / `--json` options without requiring an LLM endpoint.

use crate::application_port::CliApplicationPort;
use crate::i18n::{current_language, parse_language_tag, pick_language, set_language, Language};
use chrono::Utc;
use nola_application::{ApplicationCommand, McpCommand, McpServer, Skill, SkillCommand};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdminError {
    Usage(String),
    Runtime(String),
}

impl AdminError {
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::Runtime(_) => 1,
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Usage(msg) | Self::Runtime(msg) => msg,
        }
    }

    fn from_subcommand_error(error: String) -> Self {
        let trimmed = error.trim_start();
        if trimmed.starts_with("Usage:")
            || trimmed.starts_with("nola ")
            || trimmed.starts_with("--")
        {
            Self::Usage(error)
        } else {
            Self::Runtime(error)
        }
    }
}

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

pub(crate) fn run_if_requested(args: &[String]) -> Option<Result<(), AdminError>> {
    let context = match parse_leading_context(args) {
        Ok(context) => context,
        Err(error) => return Some(Err(AdminError::Usage(error))),
    };
    let remaining = &context.remaining;
    let desktop_usage = match remaining.as_slice() {
        [plugin, action, ..] if plugin == "plugin" => match action.as_str() {
            "desktop-library" => Some("nola plugin desktop-library [--json]"),
            "desktop-pages" => Some("nola plugin desktop-pages [id] [--json]"),
            _ => None,
        },
        _ => None,
    };
    if !is_admin_dispatch_target(remaining) && desktop_usage.is_none() {
        return None;
    }
    if let Some(language) = context.language {
        set_language(language);
    }
    if let Some(usage) = desktop_usage {
        return Some(
            strip_admin_flags(&remaining[2..], context.json_output)
                .map_err(AdminError::Usage)
                .and_then(|(args, _, help)| {
                    if help {
                        println!("{usage}");
                        return Ok(());
                    }
                    let mut command = remaining[..2].to_vec();
                    command.extend(args);
                    crate::harness_plugin::desktop_command(&command)
                        .map(|res| res.map_err(AdminError::from_subcommand_error))
                        .unwrap_or_else(|| Err(AdminError::Usage(usage.to_owned())))
                }),
        );
    }
    match remaining.first().map(String::as_str) {
        Some("version" | "--version" | "-V") => Some(
            strip_admin_flags(&remaining[1..], context.json_output)
                .map_err(AdminError::Usage)
                .and_then(|(args, json, help)| {
                    const USAGE: &str = "nola version | --version | -V [--json]";
                    if help {
                        println!("{USAGE}");
                        Ok(())
                    } else if args.is_empty() {
                        run_version(json).map_err(AdminError::Runtime)
                    } else {
                        Err(AdminError::Usage(USAGE.into()))
                    }
                }),
        ),
        Some("update") => crate::software_update::run_if_requested_with_json(
            remaining,
            context.json_output,
        )
        .map(|res| res.map_err(AdminError::from_subcommand_error)),
        Some("memory") => Some(
            crate::memory_admin::run_cli_args(&context.remaining[1..], context.json_output)
                .map_err(AdminError::from_subcommand_error),
        ),
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

fn admin_text(values: [&'static str; 9]) -> &'static str {
    pick_language(current_language(), values)
}

fn enabled_state_label(enabled: bool) -> &'static str {
    if enabled {
        admin_text([
            "已启用",
            "已啟用",
            "enabled",
            "aktiviert",
            "habilitado",
            "activé",
            "有効",
            "활성화됨",
            "включено",
        ])
    } else {
        admin_text([
            "已停用",
            "已停用",
            "disabled",
            "deaktiviert",
            "deshabilitado",
            "désactivé",
            "無効",
            "비활성화됨",
            "отключено",
        ])
    }
}

fn format_skill_enabled_notice(id: &str, enabled: bool, revision: i64) -> String {
    let state = enabled_state_label(enabled);
    match current_language() {
        Language::ZhHans => format!("技能 '{id}' {state}（版本 {revision}）。"),
        Language::ZhHant => format!("技能 '{id}' {state}（版本 {revision}）。"),
        Language::De => format!("Skill '{id}' {state} (Revision {revision})."),
        Language::Es => format!("Skill '{id}' {state} (revisión {revision})."),
        Language::Fr => format!("Skill « {id} » {state} (révision {revision})."),
        Language::Ja => format!("スキル '{id}' を{state}にしました（リビジョン {revision}）。"),
        Language::Ko => format!("스킬 '{id}' {state} (리비전 {revision})."),
        Language::Ru => format!("Навык «{id}»: {state} (ревизия {revision})."),
        Language::En => format!("Skill '{id}' {state} (revision {revision})."),
    }
}

fn format_skill_installed_notice(id: &str, source_url: &str) -> String {
    match current_language() {
        Language::ZhHans => format!("已启用 {id}（来源：{source_url}）"),
        Language::ZhHant => format!("已啟用 {id}（來源：{source_url}）"),
        Language::De => format!("{id} aus {source_url} aktiviert"),
        Language::Es => format!("{id} habilitado desde {source_url}"),
        Language::Fr => format!("{id} activé depuis {source_url}"),
        Language::Ja => format!("{source_url} から {id} を有効にしました"),
        Language::Ko => format!("{source_url}에서 {id} 활성화됨"),
        Language::Ru => format!("Включён {id} из {source_url}"),
        Language::En => format!("Enabled {id} from {source_url}"),
    }
}

fn format_mcp_added_notice(id: &str, revision: i64) -> String {
    match current_language() {
        Language::ZhHans => format!("已添加 MCP 服务器 '{id}'（版本 {revision}）。"),
        Language::ZhHant => format!("已新增 MCP 伺服器 '{id}'（版本 {revision}）。"),
        Language::De => format!("MCP-Server '{id}' hinzugefügt (Revision {revision})."),
        Language::Es => format!("Servidor MCP '{id}' añadido (revisión {revision})."),
        Language::Fr => format!("Serveur MCP « {id} » ajouté (révision {revision})."),
        Language::Ja => format!("MCP サーバー '{id}' を追加しました（リビジョン {revision}）。"),
        Language::Ko => format!("MCP 서버 '{id}' 추가됨 (리비전 {revision})."),
        Language::Ru => format!("MCP-сервер «{id}» добавлен (ревизия {revision})."),
        Language::En => format!("Added MCP server '{id}' (revision {revision})."),
    }
}

fn format_mcp_removed_notice(id: &str) -> String {
    match current_language() {
        Language::ZhHans => format!("已移除 MCP 服务器 '{id}'。"),
        Language::ZhHant => format!("已移除 MCP 伺服器 '{id}'。"),
        Language::De => format!("MCP-Server '{id}' entfernt."),
        Language::Es => format!("Servidor MCP '{id}' eliminado."),
        Language::Fr => format!("Serveur MCP « {id} » supprimé."),
        Language::Ja => format!("MCP サーバー '{id}' を削除しました。"),
        Language::Ko => format!("MCP 서버 '{id}' 제거됨."),
        Language::Ru => format!("MCP-сервер «{id}» удалён."),
        Language::En => format!("Removed MCP server '{id}'."),
    }
}

fn format_mcp_enabled_notice(id: &str, enabled: bool, revision: i64) -> String {
    let state = enabled_state_label(enabled);
    match current_language() {
        Language::ZhHans => format!("MCP 服务器 '{id}' {state}（版本 {revision}）。"),
        Language::ZhHant => format!("MCP 伺服器 '{id}' {state}（版本 {revision}）。"),
        Language::De => format!("MCP-Server '{id}' {state} (Revision {revision})."),
        Language::Es => format!("Servidor MCP '{id}' {state} (revisión {revision})."),
        Language::Fr => format!("Serveur MCP « {id} » {state} (révision {revision})."),
        Language::Ja => format!("MCP サーバー '{id}' を{state}にしました（リビジョン {revision}）。"),
        Language::Ko => format!("MCP 서버 '{id}' {state} (리비전 {revision})."),
        Language::Ru => format!("MCP-сервер «{id}»: {state} (ревизия {revision})."),
        Language::En => format!("MCP server '{id}' {state} (revision {revision})."),
    }
}

fn skills_usage() -> &'static str {
    "Usage:\n  nola skills [list] [--json]\n  nola skills find <name-or-github-url> [--json]\n  nola skills install <github-url> [--json]\n  nola skills enable <id> [--json]\n  nola skills disable <id> [--json]"
}

fn run_skills_command(args: &[String], leading_json: bool) -> Result<(), AdminError> {
    let (positional, json_output, show_help) =
        strip_admin_flags(args, leading_json).map_err(AdminError::Usage)?;
    if show_help {
        println!("{}", skills_usage());
        return Ok(());
    }
    if let [action, value] = positional.as_slice() {
        let result = match action.as_str() {
            "find" => Some(crate::github_skills::find(value).map_err(AdminError::Runtime)?),
            "install" => Some(crate::github_skills::install(value).map_err(AdminError::Runtime)?),
            _ => None,
        };
        if let Some(value) = result {
            if json_output {
                println!("{value}");
            } else if let Some(items) = value["repositories"].as_array().or(value["skills"].as_array()) {
                for item in items {
                    println!("{}  {}", item["repository"].as_str().or(item["path"].as_str()).unwrap_or_default(),
                             item["url"].as_str().unwrap_or_default());
                }
                if value["status"] == "selection_required" {
                    println!(
                        "{}",
                        admin_text([
                            "请选择一个 Skill URL 并运行 nola skills install <url>。",
                            "請選擇一個 Skill URL 並執行 nola skills install <url>。",
                            "Select a Skill URL and run nola skills install <url>.",
                            "Wählen Sie eine Skill-URL aus und führen Sie nola skills install <url> aus.",
                            "Selecciona una URL de Skill y ejecuta nola skills install <url>.",
                            "Sélectionnez une URL de Skill et exécutez nola skills install <url>.",
                            "Skill の URL を選び、nola skills install <url> を実行してください。",
                            "Skill URL을 선택한 뒤 nola skills install <url>을 실행하세요.",
                            "Выберите URL навыка и выполните nola skills install <url>.",
                        ])
                    );
                } else if items.is_empty() {
                    println!(
                        "{}",
                        admin_text([
                            "未找到匹配的 Skill 或仓库。",
                            "找不到相符的 Skill 或儲存庫。",
                            "No matching Skills or repositories found.",
                            "Keine passenden Skills oder Repositories gefunden.",
                            "No se encontraron Skills ni repositorios coincidentes.",
                            "Aucun Skill ou dépôt correspondant trouvé.",
                            "一致する Skill やリポジトリが見つかりませんでした。",
                            "일치하는 Skill 또는 저장소를 찾지 못했습니다.",
                            "Подходящие навыки или репозитории не найдены.",
                        ])
                    );
                }
            } else {
                println!(
                    "{}",
                    format_skill_installed_notice(
                        value["id"].as_str().unwrap_or_default(),
                        value["source_url"].as_str().unwrap_or_default(),
                    )
                );
            }
            return Ok(());
        }
    }
    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    if !matches!(refs.as_slice(), [] | ["list"] | ["enable" | "disable", _]) {
        return Err(AdminError::Usage(skills_usage().to_string()));
    }
    let mut application = CliApplicationPort::persistent_default().map_err(AdminError::Runtime)?;
    execute_skills_command(&mut application, &positional, json_output)
}

fn execute_skills_command(
    application: &mut CliApplicationPort,
    positional: &[String],
    json_output: bool,
) -> Result<(), AdminError> {
    let builtins = crate::skill_catalog_port::load_web_builtin_skills_with(application)
        .map_err(AdminError::Runtime)?;
    let uploaded = crate::skill_catalog_port::load_web_uploaded_skills_with(application)
        .map_err(AdminError::Runtime)?;
    let mut combined = Vec::with_capacity(builtins.len() + uploaded.len());
    combined.extend(builtins);
    combined.extend(uploaded);

    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    match refs.as_slice() {
        [] | ["list"] => {
            if json_output {
                println!("{}", json!({ "type": "skills", "skills": combined }));
            } else if combined.is_empty() {
                println!(
                    "{}",
                    admin_text([
                        "未找到技能。",
                        "找不到技能。",
                        "No skills found.",
                        "Keine Skills gefunden.",
                        "No se encontraron skills.",
                        "Aucun skill trouvé.",
                        "スキルが見つかりません。",
                        "스킬을 찾을 수 없습니다.",
                        "Навыки не найдены.",
                    ])
                );
            } else {
                for item in &combined {
                    let id = item["id"].as_str().unwrap_or("");
                    let name = item["name"].as_str().unwrap_or(id);
                    let enabled = item["enabled"].as_bool().unwrap_or(true);
                    let state = enabled_state_label(enabled);
                    println!("[{state}] {id} ({name})");
                }
            }
            Ok(())
        }
        [action @ ("enable" | "disable"), target_id] => {
            let enabled = *action == "enable";
            set_skill_enabled(application, target_id, enabled, json_output)
                .map_err(AdminError::Runtime)
        }
        _ => Err(AdminError::Usage(skills_usage().to_string())),
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
    if updated["record"]["source"] == "user" {
        crate::builtin_skills::sync_registered_custom(application, Some(&matched_id))?;
    }
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
        println!("{}", format_skill_enabled_notice(&matched_id, enabled, new_revision));
    }
    Ok(())
}

fn mcp_usage() -> &'static str {
    "Usage:\n  nola mcp [list] [--json]\n  nola mcp add <name> <command-or-url> [args...] [--json]\n  nola mcp remove <id-or-name> [--json]\n  nola mcp enable <id-or-name> [--json]\n  nola mcp disable <id-or-name> [--json]"
}

fn strip_mcp_admin_flags(
    args: &[String],
    mut json_output: bool,
) -> Result<(Vec<String>, bool, bool), String> {
    let mut positional = Vec::new();
    let mut show_help = false;
    let mut index = 0;
    while index < args.len() {
        let in_stdio_server_args = positional.first().map(String::as_str) == Some("add")
            && positional.len() >= 3
            && positional.get(2).is_some_and(|target| {
                let lower = target.trim().to_ascii_lowercase();
                !lower.starts_with("http://") && !lower.starts_with("https://")
            });
        if in_stdio_server_args {
            if args[index] == "--" {
                positional.extend(args[index + 1..].iter().cloned());
            } else {
                positional.extend(args[index..].iter().cloned());
            }
            break;
        }
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

fn run_mcp_command(args: &[String], leading_json: bool) -> Result<(), AdminError> {
    let (positional, json_output, show_help) =
        strip_mcp_admin_flags(args, leading_json).map_err(AdminError::Usage)?;
    if show_help {
        println!("{}", mcp_usage());
        return Ok(());
    }
    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    if !matches!(
        refs.as_slice(),
        [] | ["list"] | ["add", _, _, ..] | ["remove", _] | ["enable" | "disable", _]
    ) {
        return Err(AdminError::Usage(mcp_usage().to_string()));
    }
    let mut application = CliApplicationPort::persistent_default().map_err(AdminError::Runtime)?;
    execute_mcp_command(&mut application, &positional, json_output)
}

fn execute_mcp_command(
    application: &mut CliApplicationPort,
    positional: &[String],
    json_output: bool,
) -> Result<(), AdminError> {
    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    match refs.as_slice() {
        [] | ["list"] => list_mcp_servers(application, json_output).map_err(AdminError::Runtime),
        ["add", name, target, extra @ ..] => {
            add_mcp_server(application, name, target, extra, json_output)
                .map_err(AdminError::Runtime)
        }
        ["remove", id_or_name] => {
            remove_mcp_server(application, id_or_name, json_output).map_err(AdminError::Runtime)
        }
        [action @ ("enable" | "disable"), id_or_name] => {
            let enabled = *action == "enable";
            set_mcp_enabled(application, id_or_name, enabled, json_output)
                .map_err(AdminError::Runtime)
        }
        _ => Err(AdminError::Usage(mcp_usage().to_string())),
    }
}

fn fetch_mcp_servers(application: &mut CliApplicationPort) -> Result<Vec<McpServer>, String> {
    let servers: Vec<McpServer> = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::List,
    })?;
    Ok(servers
        .into_iter()
        .filter(|server| server.deleted_at_ms.is_none() && server.is_managed_definition())
        .collect())
}

fn find_mcp_server<'a>(servers: &'a [McpServer], id_or_name: &str) -> Result<&'a McpServer, String> {
    if let Some(server) = servers.iter().find(|server| server.id == id_or_name) {
        return Ok(server);
    }
    let mut matches = servers.iter().filter(|server| server.display_name == id_or_name);
    let server = matches.next().ok_or_else(|| format!("MCP server '{id_or_name}' not found"))?;
    if matches.next().is_some() {
        return Err(format!("Multiple MCP servers are named '{id_or_name}'. Use an ID from nola mcp list."));
    }
    Ok(server)
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
        println!(
            "{}",
            admin_text([
                "尚未配置 MCP 服务器。",
                "尚未設定 MCP 伺服器。",
                "No MCP servers configured.",
                "Keine MCP-Server konfiguriert.",
                "No hay servidores MCP configurados.",
                "Aucun serveur MCP configuré.",
                "MCP サーバーは設定されていません。",
                "구성된 MCP 서버가 없습니다.",
                "MCP-серверы не настроены.",
            ])
        );
    } else {
        for item in &items {
            let state = enabled_state_label(item["enabled"].as_bool().unwrap_or(true));
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
            "type": "streamable_http",
            "endpoint": trimmed_target,
            "url": trimmed_target,
        })
    } else {
        json!({
            "type": "stdio",
            "command": trimmed_target,
            "args": extra,
        })
    };

    let created: McpServer = application.execute(ApplicationCommand::Mcp {
        command: McpCommand::CreateNamed {
            display_name: trimmed_name.to_string(),
            configuration: config_value,
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
        println!("{}", format_mcp_added_notice(&created.id, created.revision));
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
        println!("{}", format_mcp_removed_notice(&id));
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
        println!("{}", format_mcp_enabled_notice(&id, enabled, updated.revision));
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
    fn cli_flow_mcp_recreation_and_reserved_name_keep_distinct_addressable_ids() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CliApplicationPort::persistent(&dir.path().join("mcp.sqlite")).unwrap();
        for name in ["server", "source:mcp:name"] {
            let mut ids = Vec::new();
            for _ in 0..2 {
                add_mcp_server(&mut app, name, "echo", &[], true).unwrap();
                assert!(add_mcp_server(&mut app, name, "echo", &[], true).is_err());
                let active = fetch_mcp_servers(&mut app).unwrap();
                let item = find_mcp_server(&active, name).unwrap();
                assert!(!item.id.contains(":mcp:"));
                assert_eq!(serde_json::from_str::<Value>(&item.configuration_json).unwrap()["server_id"], item.id);
                ids.push(item.id.clone());
                remove_mcp_server(&mut app, name, true).unwrap();
            }
            assert_ne!(ids[0], ids[1]);
        }
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

    #[test]
    fn strip_mcp_admin_flags_preserves_stdio_server_flags() {
        let args: Vec<String> = [
            "add", "my-server", "npx", "-y", "some-mcp", "--json", "--lang", "fr", "--help", "-h",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (positional, json_output, show_help) = strip_mcp_admin_flags(&args, false).unwrap();
        assert!(!json_output);
        assert!(!show_help);
        assert_eq!(
            positional,
            vec!["add", "my-server", "npx", "-y", "some-mcp", "--json", "--lang", "fr", "--help", "-h"]
        );

        let args_with_cli_json: Vec<String> = [
            "add", "--json", "my-server", "npx", "--json",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (positional, json_output, show_help) =
            strip_mcp_admin_flags(&args_with_cli_json, false).unwrap();
        assert!(json_output);
        assert!(!show_help);
        assert_eq!(positional, vec!["add", "my-server", "npx", "--json"]);

        let http_args: Vec<String> = [
            "add", "remote", "https://example.com/mcp", "--json",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (positional, json_output, show_help) = strip_mcp_admin_flags(&http_args, false).unwrap();
        assert!(json_output);
        assert!(!show_help);
        assert_eq!(positional, vec!["add", "remote", "https://example.com/mcp"]);
    }

    #[test]
    fn admin_usage_errors_report_exit_code_2() {
        for args in [
            vec!["version".into(), "unexpected".into()],
            vec!["skills".into(), "bogus".into()],
            vec!["mcp".into(), "add".into()],
            vec!["--lang".into(), "not-a-lang".into(), "version".into()],
        ] {
            let err = run_if_requested(&args).unwrap().unwrap_err();
            assert_eq!(err.exit_code(), 2, "expected exit code 2 for {args:?}");
        }
    }
}
