// SPDX-License-Identifier: Apache-2.0
//! CLI configuration inspection and startup catalog publication.
use crate::config::{CommandMode, Config, ConfigFileAction};
use crate::i18n::{ui_text, UiText};
use crate::model_connection_host::file_config::{self, FileModels};
use crate::runtime_config::RuntimeConfig;
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn usage() -> String {
    [
        ("nola config", ui_text(UiText::FileHelpShow)),
        ("nola config edit [path]", ui_text(UiText::FileHelpEdit)),
        ("nola config check [path]", ui_text(UiText::FileHelpCheck)),
        (
            "nola config apply [path] --dry-run",
            ui_text(UiText::FileHelpPreview),
        ),
        ("nola config apply [path]", ui_text(UiText::FileHelpApply)),
        ("nola config example", ui_text(UiText::FileHelpExample)),
    ]
    .iter()
    .map(|(command, detail)| format!("{command}\n  {detail}"))
    .collect::<Vec<_>>()
    .join("\n")
}

pub(crate) fn overview_label() -> &'static str {
    ui_text(UiText::FileOverview)
}

fn example() -> Value {
    json!({"llm": {
        "connections": [{"id":"local", "name":"Local", "protocol":"chat_completions",
            "endpoint":"http://127.0.0.1:8080/v1", "models":[{"id":"my-model", "name":"My model"}]}],
        "default_model":{"connection":"local","model":"my-model"}
    }})
}

struct Inspection {
    runtime: RuntimeConfig,
    catalog: Option<FileModels>,
}

impl Inspection {
    fn load(project: &Path, explicit: Option<&str>) -> Result<Self, String> {
        let (runtime, catalog) = if let Some(path) = explicit {
            let source = Path::new(path);
            let value = crate::runtime_config::read_jsonc_file(source)?
                .ok_or_else(|| format!("{}: file not found", source.display()))?;
            RuntimeConfig::from_file(value, source)?
        } else {
            RuntimeConfig::inspect_files(project)?
        };
        Self::validated(runtime, catalog)
    }

    fn validated(runtime: RuntimeConfig, catalog: Option<FileModels>) -> Result<Self, String> {
        if let Some(policy) = &runtime.network_policy {
            crate::config::normalize_network_policy(policy)?;
        }
        if let Some(tools) = &runtime.force_tool_set {
            crate::config::normalize_force_tool_set(tools)?;
        }
        if runtime
            .language
            .as_deref()
            .is_some_and(|tag| crate::i18n::parse_language_tag(tag).is_none())
        {
            return Err("invalid language in configuration; see nola --help".into());
        }
        Ok(Self { runtime, catalog })
    }

    fn output(
        &self,
        status: &str,
        json_output: bool,
        receipt: Option<&Value>,
        saved: Option<&SavedDefault>,
    ) -> String {
        let catalog = self.catalog.as_ref();
        let runtime = &self.runtime;
        let connections = catalog
            .map(FileModels::declared_connections)
            .unwrap_or_default();
        let mut settings = json!({
            "language":runtime.language, "network_policy":runtime.network_policy,
            "force_tool_set":runtime.force_tool_set, "max_iterations":runtime.max_iterations,
            "debug":runtime.debug_mode
        });
        if let Some(fields) = settings.as_object_mut() {
            fields.retain(|_, value| !value.is_null());
        }
        if json_output {
            return json!({
                "status":status, "files":runtime.sources, "catalog_source":runtime.model_binding_source,
                "default_model":runtime.model_binding, "route_model":runtime.model, "runtime_settings":settings,
                "connections":connections,
                "saved_default":saved, "receipt":receipt
            }).to_string();
        }
        let mut lines = Vec::new();
        let saved_line =
            saved.map(|saved| format!("{}  {}", ui_text(UiText::FileHostDefault), saved.label()));
        if runtime.sources.is_empty() {
            lines.extend(saved_line);
            lines.push(ui_text(UiText::FileNone).into());
            if let Some(home) = crate::platform_paths::home_dir() {
                lines.push(format!(
                    "  {}",
                    display_path(&home.join(".nola/config.jsonc"))
                ));
            }
            lines.push("  nola config example".into());
            return lines.join("\n");
        }
        lines.push(ui_text(UiText::FileLoadOrder).into());
        for path in &runtime.sources {
            let mut line = format!("  {}", display_path(path));
            if runtime.model_binding_source.as_ref() == Some(path) {
                line.push_str("  ·  ");
                line.push_str(ui_text(UiText::FileModelSource));
            }
            lines.push(line);
        }
        if let Some(fields) = settings.as_object().filter(|fields| !fields.is_empty()) {
            let values = fields
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>();
            lines.push(format!(
                "{}  {}",
                ui_text(UiText::FileSettings),
                values.join("  ")
            ));
        }
        if let Some(default) = &runtime.model_binding {
            lines.push(String::new());
            for connection in connections {
                lines.push(format!(
                    "  {}  ·  {}  ·  {}",
                    connection.id, connection.origin, connection.protocol
                ));
                lines.push(format!("    {}", connection.models.join(", ")));
                if let Some(name) = connection.api_key_env {
                    lines.push(format!("    api_key_env = {name}"));
                }
            }
            lines.push(String::new());
            lines.push(format!(
                "{}  {} / {}",
                ui_text(UiText::FileDeclaredDefault),
                default.connection,
                default.model
            ));
            lines.extend(saved_line);
            lines.push(
                match status {
                    "applied" => ui_text(UiText::FileApplied),
                    "preview" => ui_text(UiText::FilePreview),
                    "checked" => ui_text(UiText::FileChecked),
                    _ => ui_text(UiText::FileStartup),
                }
                .into(),
            );
        } else {
            lines.extend(saved_line);
            if let Some(model) = &runtime.model {
                lines.push(format!("{}  {model}", ui_text(UiText::FileRouteModel)));
            }
            lines.push(ui_text(UiText::FileRuntimeOnly).into());
        }
        lines.join("\n")
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedDefault {
    Automatic,
    Profile {
        model: String,
        connection: Option<String>,
        enabled: bool,
    },
    Unavailable,
}

impl SavedDefault {
    fn label(&self) -> String {
        match self {
            Self::Automatic => ui_text(UiText::FileAutomatic).to_owned(),
            Self::Unavailable => ui_text(UiText::FileHostUnavailable).to_owned(),
            Self::Profile {
                model,
                connection,
                enabled,
            } => {
                let mut label = match connection {
                    Some(id) => format!("{} / {model}", id.strip_prefix("config-").unwrap_or(id)),
                    None => model.clone(),
                };
                if !enabled {
                    label.push_str(ui_text(UiText::FileDisabled));
                }
                label
            }
        }
    }
}

fn display_path(path: &Path) -> String {
    if let Ok(directory) = std::env::current_dir() {
        if let Ok(relative) = path.strip_prefix(directory) {
            return format!("./{}", relative.display());
        }
    }
    if let Some(home) = crate::platform_paths::user_home_dir() {
        if let Ok(relative) = path.strip_prefix(home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
}

fn saved_default() -> Result<SavedDefault, String> {
    use nola_application::{model_selection, ApplicationCommand, ModelCommand};
    let path = crate::store::default_db_path()?;
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(SavedDefault::Automatic);
    }
    let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
    let setting: Option<nola_application::ProductSetting> =
        app.execute(ApplicationCommand::GetProductSetting {
            key: "chat.default_model".into(),
        })?;
    let default = model_selection::parse_default_chat_configuration(
        setting.as_ref().map(|s| s.value_json.as_str()),
    )?;
    let model_selection::DefaultChatSelection::Profile { profile_id } = default.selection else {
        return Ok(SavedDefault::Automatic);
    };
    let profile: Option<nola_application::ModelProfile> =
        app.execute(ApplicationCommand::Model {
            command: ModelCommand::GetProfile { id: profile_id },
        })?;
    let Some(profile) = profile.filter(|p| p.deleted_at_ms.is_none()) else {
        return Ok(SavedDefault::Unavailable);
    };
    let source: Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "invalid stored model source")?;
    Ok(SavedDefault::Profile {
        model: profile.model_name,
        connection: source["connectionID"].as_str().map(str::to_owned),
        enabled: profile.enabled && source["endpointEnabled"] != false,
    })
}

pub(crate) fn overview(project: &Path, json_output: bool) -> Result<String, String> {
    let inspection = Inspection::load(project, None)?;
    // The file overview remains available when the local Host cannot be read.
    let saved = saved_default().unwrap_or(SavedDefault::Unavailable);
    Ok(inspection.output("show", json_output, None, Some(&saved)))
}

fn edit(project: &Path, explicit: Option<&str>, json_output: bool) -> Result<String, String> {
    let path = if let Some(path) = explicit {
        path.into()
    } else {
        // Discovery must also work when the file the user wants to repair is invalid.
        let home = crate::platform_paths::home_dir().ok_or("cannot locate user configuration")?;
        let mut target = home.join(".nola/config.jsonc");
        for candidate in crate::runtime_config::config_candidates(project, Some(&home))
            .into_iter()
            .rev()
        {
            if candidate
                .try_exists()
                .map_err(|error| format!("{}: {error}", candidate.display()))?
            {
                target = candidate;
                break;
            }
        }
        target
    };
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.into());
    let words = if Path::new(&editor).is_file() {
        vec![editor]
    } else {
        shlex::split(&editor).ok_or("invalid quoting in VISUAL or EDITOR")?
    };
    let (program, args) = words.split_first().ok_or("empty editor command")?;
    let directory = path
        .parent()
        .ok_or("configuration needs a parent directory")?;
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let mut command = std::process::Command::new(program);
    command.args(args).arg(&path);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A newly created configuration may contain a directly configured API key.
        unsafe {
            command.pre_exec(|| {
                libc::umask(0o077);
                Ok(())
            });
        }
    }
    let status = command
        .status()
        .map_err(|error| format!("{program}: {error}"))?;
    if !status.success() {
        return Err(format!("{program}: {status}"));
    }
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(if json_output {
            json!({"status":"not_saved","path":path}).to_string()
        } else {
            ui_text(UiText::FileNotSaved).to_string()
        });
    }
    let file = path.to_str().ok_or("config path is not UTF-8")?;
    Ok(Inspection::load(project, Some(file))?.output("checked", json_output, None, None))
}

/// Called once after CLI argument/authority validation and before restoring a conversation.
pub(crate) fn apply_at_startup(config: &Config) -> Result<(), String> {
    if !matches!(config.command, CommandMode::Run | CommandMode::Init | CommandMode::Wiki | CommandMode::Web | CommandMode::Server) {
        return Ok(());
    }
    let source = config.runtime.startup_catalog_error.as_ref().map(|(path, _)| path)
        .or(config.runtime.model_binding_source.as_ref());
    let Some(source) = source else { return Ok(()); };
    let apply = || -> Result<(), String> {
        if let Some((_, error)) = &config.runtime.startup_catalog_error { return Err(error.clone()); }
        let path = source.to_str().ok_or("config path is not UTF-8")?;
        let models = file_config::declarations(&file_config::read_document(path)?, source)?;
        if config.runtime.model_binding.as_ref() != Some(&models.default_model) {
            return Err(format!("{path} changed while reading; retry"));
        }
        let origins = ["localhost", "127.0.0.1", "[::1]"].map(|host| format!("http://{host}:{}", config.web_port));
        file_config::apply_on_startup(&crate::store::NolaStore::open_default()?, &models, &origins, source)?;
        Ok(())
    };
    if let Err(error) = apply() {
        let edit = file_config::file_command("edit", source);
        let diagnostic = format!("{}  {}\n{error}\n  {edit}", ui_text(UiText::FileSkipped), display_path(source));
        // Never invent a route. Existing conversations restore their own binding;
        // fresh conversations may use only a previously committed Host default.
        if matches!(config.command, CommandMode::Run | CommandMode::Init | CommandMode::Wiki)
            && config.cli_profile_selection.is_none() && config.session_id.is_none()
            && !config.continue_last && !config.resume_session
            && !matches!(saved_default(), Ok(SavedDefault::Profile { enabled:true, .. })) {
            return Err(diagnostic);
        }
        eprintln!("{diagnostic}");
    }
    Ok(())
}

pub(crate) fn execute(config: &Config) -> Result<String, String> {
    let (action, path, legacy) = match &config.command {
        CommandMode::ConfigFile { action, path } => (*action, path.clone(), false),
        CommandMode::ModelApplyConfig { path } => (
            ConfigFileAction::Apply,
            Some(file_config::selected_path(path.as_deref())?),
            true,
        ),
        _ => return Err("expected config command".into()),
    };
    if action == ConfigFileAction::Show {
        return overview(&config.project_root, config.json_output);
    }
    if action == ConfigFileAction::Edit {
        return edit(&config.project_root, path.as_deref(), config.json_output);
    }
    if action == ConfigFileAction::Example {
        return serde_json::to_string_pretty(&example()).map_err(|error| error.to_string());
    }
    let mut inspection = if legacy {
        let path = Path::new(path.as_deref().ok_or("missing config path")?);
        let catalog = file_config::declarations(
            &file_config::read_document(path.to_str().ok_or("config path is not UTF-8")?)?,
            path,
        )?;
        Inspection {
            runtime: RuntimeConfig {
                sources: vec![path.to_owned()],
                model_binding: Some(catalog.default_model.clone()),
                model_binding_source: Some(path.to_owned()),
                ..Default::default()
            },
            catalog: Some(catalog),
        }
    } else if let (ConfigFileAction::Apply, Some(source)) = (action, path.as_deref()) {
        let (runtime, catalog) =
            RuntimeConfig::from_file(file_config::read_document(source)?, Path::new(source))?;
        Inspection::validated(runtime, catalog)?
    } else {
        Inspection::load(&config.project_root, path.as_deref())?
    };

    // Startup may read linked or large runtime files. Publishing a catalog must
    // use the same bounded, non-symlink reader as an explicitly selected file.
    if action == ConfigFileAction::Apply && path.is_none() {
        if let Some(source) = &inspection.runtime.model_binding_source {
            let source = source.to_str().ok_or("config path is not UTF-8")?;
            let checked =
                file_config::declarations(&file_config::read_document(source)?, Path::new(source))?;
            if inspection.runtime.model_binding.as_ref() != Some(&checked.default_model) {
                return Err(format!("{source} changed while reading; retry"));
            }
            inspection.catalog = Some(checked);
        }
    }
    let origins = ["localhost", "127.0.0.1", "[::1]"]
        .map(|host| format!("http://{host}:{}", config.web_port));
    let (status, receipt) = match (action, inspection.catalog.as_ref()) {
        (ConfigFileAction::Check, _) => ("checked", None),
        (ConfigFileAction::Apply, None) => ("nothing_to_apply", None),
        (ConfigFileAction::Apply, Some(catalog)) if config.dry_run => {
            catalog.check_gateway_routes(&origins)?;
            ("preview", None)
        }
        (ConfigFileAction::Apply, Some(catalog)) => (
            "applied",
            Some(file_config::apply(
                &crate::store::NolaStore::open_default()?,
                catalog,
                &origins,
            )?),
        ),
        _ => ("show", None),
    };
    if let (true, Some(receipt)) = (legacy, &receipt) {
        // Existing scripts consume this receipt even without --json.
        return serde_json::to_string_pretty(receipt).map_err(|error| error.to_string());
    }
    Ok(inspection.output(status, config.json_output, receipt.as_ref(), None))
}
