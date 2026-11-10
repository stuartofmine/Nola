// SPDX-License-Identifier: Apache-2.0
//! Native launch argv is a transport envelope, not a CLI command configuration.
use std::path::PathBuf;
use crate::host_invocation_request::*;

pub(crate) fn configure_surface() {
    let surface = std::env::var("NOLA_CLIENT_SURFACE").ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "mac" | "macos" | "mac_server" | "server" | "web" | "ios" | "android"))
        .unwrap_or_else(|| "agent_host".into());
    std::env::set_var("NOLA_DESKTOP_HOST", "1");
    std::env::set_var("NOLA_CLIENT_SURFACE", surface);
}

pub(crate) fn is_native_run(arguments: &[String]) -> bool {
    arguments.first().is_some_and(|arg| arg == "run")
        && arguments.iter().any(|arg| arg == "--host-request-stdin")
}

#[derive(Default)]
struct Launch {
    session: Option<String>, project: Option<PathBuf>, endpoint: Option<String>,
    model: Option<String>, allowed_dirs: Vec<PathBuf>,
    skill_dirs: Option<Vec<PathBuf>>,
}

impl Launch {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut launch = Self::default();
        let mut args = arguments.iter().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--json" | "--host-request-stdin" => {},
                "--no-skills" => launch.skill_dirs = Some(Vec::new()),
                "--session" | "--project" | "--endpoint" | "--model"
                | "--allow-dir" | "--skill-dir" => {
                    let value = args.next().ok_or_else(|| format!("missing native launch value for {arg}"))?;
                    match arg.as_str() {
                        "--session" => launch.session = Some(value.clone()),
                        "--project" => launch.project = Some(value.into()),
                        "--endpoint" => launch.endpoint = Some(value.clone()),
                        "--model" => launch.model = Some(value.clone()),
                        "--allow-dir" => launch.allowed_dirs.push(value.into()),
                        "--skill-dir" => launch.skill_dirs.get_or_insert_with(Vec::new).push(value.into()),
                        _ => unreachable!(),
                    }
                },
                _ => return Err(format!("unsupported native launch argument: {arg}")),
            }
        }
        if launch.session.as_deref().is_none_or(|id| id.trim().is_empty()) {
            return Err("native launch requires an explicit session identity".into());
        }
        Ok(launch)
    }

    fn compose(self, cancellation_only: bool) -> Result<HostInvocationRequest, String> {
        self.compose_with_current_dir(cancellation_only, || {
            std::env::current_dir().map_err(|error| error.to_string())
        })
    }

    fn compose_with_current_dir(
        self,
        cancellation_only: bool,
        current_dir: impl FnOnce() -> Result<PathBuf, String>,
    ) -> Result<HostInvocationRequest, String> {
        let raw_root = match self.project {
            Some(root) => root,
            None => current_dir()?,
        };
        let project_root = if cancellation_only { raw_root } else { normalize(raw_root)? };
        // Cold cancellation intentionally does not reload project configuration,
        // credentials, model defaults or directories from the abandoned Run.
        let runtime = if cancellation_only { crate::runtime_config::RuntimeConfig::default() }
            else { crate::runtime_config::RuntimeConfig::load(&project_root)? };
        let mut allowed_dirs = if self.allowed_dirs.is_empty() { runtime.allowed_dirs.clone() } else { self.allowed_dirs };
        allowed_dirs.insert(0, project_root.clone());
        if !cancellation_only {
            allowed_dirs = allowed_dirs.into_iter().map(normalize).collect::<Result<_, _>>()?;
        }
        allowed_dirs.dedup();
        let skill_dirs = if cancellation_only { Some(Vec::new()) } else {
            self.skill_dirs.map(|dirs| dirs.into_iter().map(normalize).collect::<Result<Vec<_>, _>>()).transpose()?
        };
        let force_tool_set = runtime.force_tool_set.as_deref().unwrap_or("auto").trim().to_ascii_lowercase();
        if !matches!(force_tool_set.as_str(), "auto" | "all" | "write") {
            return Err("invalid native Host force_tool_set".into());
        }
        if let Some(language) = runtime.language.as_deref() {
            crate::i18n::set_language(crate::i18n::parse_language_tag(language)
                .ok_or("invalid native Host language")?);
        }
        Ok(HostInvocationRequest {
            model_settings: HostModelRequest {
                endpoint: self.endpoint.or(runtime.endpoint.clone()).unwrap_or_default(),
                api_key: runtime.api_key.clone().unwrap_or_default(),
                model: self.model.or(runtime.model.clone()).unwrap_or_default(), swe_mode: false,
            },
            tool_settings: HostToolRequest {
                process_sandbox: runtime.process_sandbox.clone(),
                bounded_file_writes: false, task_worktree: None, task_sandbox_requirement: None,
                project_root, allowed_dirs, skill_dirs, bash_prefixes: runtime.bash_prefixes.clone(),
                auto_approve: false, permission_profile: Some(crate::permissions::PermissionProfile::Cautious),
                tools_enabled: true, force_tool_set,
                tool_allowlist: None, tool_allowlist_is_authorization_only: false,
                remote_tool_plan: None,
                network_policy: "ask".into(), mcp_capability_catalog: None, mcp_server_registry: Vec::new(),
            },
            session: HostSessionRequest {
                edited_resend_operation_id: None,
                user_delegation: None,
                supports_reviewed_text_parts: false, session_id: self.session, permission_scope_id: None,
                save_session: true, host_submission_id: None, host_user_message_id: None, host_assistant_message_id: None,
                expected_configuration_revision: None, expected_parent_message_id: None,
                resume_snapshot: None, desktop_resume: None, desktop_cancel: None, desktop_goal_control: None, desktop_goal_wake: None, goal_execution_id: None, queued_user_command_id: None, host_task_execution: None,
                automation_execution_lease: None, automation_execution_spec: None, automation_artifacts: Vec::new(),
            },
            input: HostInputRequest {
                message: String::new(), host_input_origin: nola_core::agent::AgentInputOrigin::InteractiveUser,
                host_turn_context: None, host_context_components: Vec::new(), host_attachments: Vec::new(),
                context_files: Vec::new(), expected_artifact_refs: Vec::new(),
                timezone: std::env::var("TZ").unwrap_or_else(|_| "local".into()),
            },
            execution: HostExecutionRequest {
                execution_backend: "nola".into(), external_execution_session_id: None,
                external_execution_model: None, external_execution_model_provider: None,
                max_iterations: runtime.max_iterations.unwrap_or(0),
            },
            runtime: runtime.host_runtime_options(),
        })
    }
}

fn normalize(path: PathBuf) -> Result<PathBuf, String> {
    let path = crate::config_paths::expand_home(&path);
    if path.exists() { path.canonicalize().map_err(|error| error.to_string()) }
    else if path.is_absolute() { Ok(path) }
    else { Ok(std::env::current_dir().map_err(|error| error.to_string())?.join(path)) }
}

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    let launch = Launch::parse(arguments)?;
    let payload = crate::native_host_entry::read_payload()?;
    let envelope: serde_json::Value = serde_json::from_str(&payload).map_err(|error| format!("invalid desktop Agent Host request: {error}"))?;
    let cancellation = crate::native_host_request::decode_desktop_cancel_operation(&envelope, launch.session.as_deref())?.is_some()
        || envelope.get("goal_wake_id").is_some()
        || envelope.get("goal_control").and_then(|v| v.get("action")).and_then(serde_json::Value::as_str)
            .is_some_and(|action| action != "start");
    let mut request = launch.compose(cancellation)?;
    crate::native_host_request::apply_desktop_host_request(&mut request, &payload)?;
    crate::native_host_entry::execute(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_parser_rejects_product_commands_and_requires_explicit_owner() {
        for args in [vec!["run", "--host-request-stdin"], vec!["run", "--session", "session", "--web"]] {
            assert!(Launch::parse(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
        let args = ["run", "--json", "--host-request-stdin", "--session", "session", "--project", "/workspace"];
        assert_eq!(Launch::parse(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).unwrap().session.as_deref(), Some("session"));
        assert!(!is_native_run(&["server".into()]));
    }

    #[test]
    fn permission_intent_is_accepted_only_from_the_canonical_stdin_profile() {
        for extra in [vec!["--yes"], vec!["--network-policy", "allow"], vec!["--network-policy", "deny"]] {
            let mut arguments = vec!["run", "--json", "--host-request-stdin", "--session", "session"];
            arguments.extend(extra);
            let error = Launch::parse(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>())
                .err().expect("duplicate argv permission policy must be rejected");
            assert!(error.contains("unsupported native launch argument"));
        }
    }

    #[test]
    fn credential_launch_arguments_are_rejected_without_echoing_the_secret() {
        let args = ["run", "--host-request-stdin", "--session", "session", "--api-key", "private-fixture-token"];
        let error = Launch::parse(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
            .err().expect("credentials belong to stdin, not argv");
        assert!(error.contains("--api-key"));
        assert!(!error.contains("private-fixture-token"));
    }

    #[test]
    fn cold_cancel_composition_does_not_touch_an_unavailable_workspace_or_skill_root() {
        let launch = Launch {
            session: Some("session".into()), project: Some("/unavailable-native-workspace".into()),
            skill_dirs: Some(vec!["/unavailable-native-skills".into()]), ..Launch::default()
        };
        let request = launch.compose_with_current_dir(true, || {
            panic!("explicit cancellation workspace must not resolve process cwd")
        }).unwrap();
        assert_eq!(request.tool_settings.project_root, PathBuf::from("/unavailable-native-workspace"));
        assert_eq!(request.tool_settings.skill_dirs, Some(Vec::new()));
        assert!(request.model_settings.api_key.is_empty());
        assert!(request.runtime.instructions.is_empty());
        assert_eq!(request.session.session_id.as_deref(), Some("session"));
    }
}
