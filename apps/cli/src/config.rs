use std::env;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::native_tool_policy::decode_desktop_host_tool_allowlist;

use crate::i18n::{help_text, parse_language_tag, set_language, tr, Message};
use crate::runtime_config::RuntimeConfig;
use crate::config_paths::expand_home;
use crate::configuration_setup::{ConfigurationSetup, NoConfigurationSetup};
use nola_core::agent::capability::McpCapabilityCatalog;
use nola_core::agent::AgentInputOrigin;
use nola_core::context_engine::ContextComponent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::external_execution_settings::HostAttachment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigFileAction { Show, Edit, Check, Apply, Example }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandMode {
    Run,
    Init,
    Wiki,
    Web,
    Server,
    Sessions,
    Room { action: String, value: Option<String>, agent_id: Option<String>, text: Option<String> },
    ModelConnections,
    ModelManage,
    ConfigFile { action: ConfigFileAction, path: Option<String> },
    ModelApplyConfig { path: Option<String> },
    ModelChatgpt { operation: String, account_id: Option<String>, revision: Option<i64>, label: Option<String> },
    ModelDiscover { connection_id: String, revision: i64 },
    ModelConfigure { connection_id: String, revision: i64, enabled_model_ids: Vec<String>, selected_model_id: Option<String> },
    ModelProbe { connection_id: String, revision: i64, model_id: String, protocol: String },
    ModelOverride { connection_id: String, revision: i64, model_id: String, protocol: String,
        capabilities: Option<nola_application::ModelExecutionCapabilities> },
    ModelImport { source: String, path: String, credential_path: Option<String>, include_credentials: bool, allow_environment: bool },
    Plugin(PluginCommandMode),
    HarnessPlugin(crate::harness_plugin::HarnessPluginCommand),
    SessionShow(String),
    SessionPart {
        conversation_id: String,
        message_id: String,
        part_id: String,
        revision: i64,
    },
    SessionSources {
        conversation_id: String,
        compaction_part_id: Option<String>,
    },
    SessionDelete(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginCommandMode {
    Inspect(String), Install(String), List { after: Option<String> }, Enable(String), Disable(String), Remove(String),
    CredentialSet { id: String, reference: String },
    CredentialClear { id: String, reference: String },
}

pub use crate::host_invocation_request::{DesktopResumeIdentity, DesktopCancelIdentity,
    AutomationArtifact};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Explicit new-conversation Core selection, never loaded from a config file.
    #[serde(skip)]
    pub cli_profile_selection: Option<(String, i64)>,
    pub command: CommandMode,
    pub resume_session: bool,
    pub show_help: bool,
    pub message: String,
    pub host_request_stdin: bool,
    /// Set only by the authenticated desktop stdin operation parser, never by
    /// config-file deserialization or the interactive --resume flag.
    #[serde(skip)]
    pub desktop_resume: Option<DesktopResumeIdentity>,
    #[serde(skip)]
    pub desktop_cancel: Option<DesktopCancelIdentity>,
    /// In-process worker authority, never accepted from serialized configuration.
    #[serde(skip)]
    pub(crate) host_task_execution: Option<crate::runtime_task_port::RuntimeTaskExecutionAuthority>,
    /// Explicit occurrence claim, accepted only through the native launch
    /// payload and revalidated by Application in the Run admission transaction.
    #[serde(skip)]
    pub(crate) automation_execution_lease: Option<nola_domain::automation_execution::AutomationExecutionLease>,
    #[serde(skip)]
    pub(crate) automation_execution_spec: Option<nola_domain::automation_execution::AutomationExecutionSpec>,
    #[serde(skip)]
    pub(crate) automation_artifacts: Vec<AutomationArtifact>,
    /// Provenance supplied by a native Host request. Ordinary CLI and native
    /// chat input remain interactive; background automation must opt into a
    /// non-interactive origin explicitly.
    pub host_input_origin: AgentInputOrigin,
    /// Model-visible control/context for an internal execution Turn. This is
    /// submitted through Core's turn-context channel and never persisted as a
    /// visible conversation message.
    pub host_turn_context: Option<String>,
    pub host_context_components: Vec<ContextComponent>,
    #[serde(skip)]
    pub main_expert_selection: Option<nola_domain::expert_task_target::MainExpertSelectionContextV1>,
    /// Files explicitly attached by the product Host for this user Turn.
    /// This is transport metadata, not a routing or permission signal.
    pub host_attachments: Vec<HostAttachment>,
    /// One revision-consistent Application recovery view retained for the
    /// complete Goal execution slice. Executor and verifier turns must not
    /// rebuild this state from independent Store queries.
    pub resume_snapshot: Option<nola_application::ResumeSnapshot>,
    /// Stable identities allocated by the product Application before the
    /// desktop Host starts. They prevent the UI and Agent runtime from
    /// inventing different records for the same Turn.
    pub host_submission_id: Option<String>,
    pub host_user_message_id: Option<String>,
    pub host_assistant_message_id: Option<String>,
    pub expected_configuration_revision: Option<i64>,
    pub expected_parent_message_id: Option<String>,
    /// Conversation-scoped execution unit selected by a product Host.
    pub execution_backend: String,
    /// Native session/thread identifier owned by the selected external executor.
    pub external_execution_session_id: Option<String>,
    pub external_execution_model: Option<String>,
    pub external_execution_model_provider: Option<String>,
    /// Typed MCP descriptors supplied by the active persisted Host registry.
    /// CLI argument parsing never invents entries for this catalog.
    pub mcp_capability_catalog: Option<McpCapabilityCatalog>,
    /// Discovered MCP registry retained by the Host so protocol
    /// `list_changed` notifications can refresh the next Turn's frozen
    /// capability catalog without client-side routing or schema synthesis.
    pub mcp_server_registry: Vec<Value>,
    pub swe_mode: bool,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub project_root: PathBuf,
    pub allowed_dirs: Vec<PathBuf>,
    /// Explicit directories that may be loaded through the Skill capability.
    /// `None` selects the managed standalone catalog, independently of workspace
    /// filesystem grants; `Some([])` explicitly disables Skill loading.
    pub skill_dirs: Option<Vec<PathBuf>>,
    pub bash_prefixes: Vec<String>,
    pub max_iterations: usize,
    pub json_output: bool,
    pub debug_mode: bool,
    pub auto_approve: bool,
    /// Explicit product permission profile for this request/Turn. `None`
    /// preserves the surface default and is resolved to Recommended by every
    /// product entrypoint. Keeping this on Config avoids process-global
    /// environment state leaking between concurrent Web/native requests.
    pub permission_profile: Option<crate::permissions::PermissionProfile>,
    pub tools_enabled: bool,
    pub force_tool_set: String,
    pub tool_allowlist: Option<Vec<String>>,
    /// The desktop policy receipt is an execution authorization boundary, not
    /// a request to expose every authorized schema directly to the model.
    #[serde(default)]
    pub tool_allowlist_is_authorization_only: bool,
    pub remote_tool_plan: Option<nola_domain::host_tool_plan::HostToolPlanSnapshot>,
    pub timezone: String,
    pub network_policy: String,
    pub session_id: Option<String>,
    /// Conversation identity that owns permission grants. Child Agent runs
    /// keep their own `session_id` for transcript persistence while inheriting
    /// the parent conversation's permission scope explicitly.
    pub permission_scope_id: Option<String>,
    pub continue_last: bool,
    pub save_session: bool,
    pub output_path: Option<PathBuf>,
    pub context_files: Vec<PathBuf>,
    /// Durable artifact resource identities declared by an external caller.
    /// These are transported as a typed Core Turn contract and never inferred
    /// from the user's message.
    #[serde(default)]
    pub expected_artifact_refs: Vec<String>,
    pub force: bool,
    pub dry_run: bool,
    pub runtime: RuntimeConfig,
    pub web_host: String,
    pub web_port: u16,
    /// An explicit encrypted remote transport terminates on loopback; it is not local trust.
    #[serde(default)]
    pub tailcat_ingress: bool,
    /// Trusted Host-only optional transport executable and its pinned digest.
    #[serde(default)]
    pub tailcat_helper: Option<(PathBuf, String)>,
    pub web_root: PathBuf,
}

#[derive(Debug)]
pub(crate) enum ConfigStartError {
    Usage(String),
    Runtime(String),
}

impl From<String> for ConfigStartError {
    fn from(message: String) -> Self { Self::Usage(message) }
}
impl From<&str> for ConfigStartError {
    fn from(message: &str) -> Self { Self::Usage(message.to_owned()) }
}
impl ConfigStartError {
    fn into_message(self) -> String {
        match self { Self::Usage(message) | Self::Runtime(message) => message }
    }
}

impl Config {
    pub(crate) fn host_invocation_request(&self) -> Result<crate::host_invocation_request::HostInvocationRequest, String> {
        use crate::host_invocation_request::*;
        let selected_session = if self.save_session && self.continue_last {
            crate::session::latest_session_id_for_project(&self.project_root.to_string_lossy())?
        } else { self.session_id.clone() };
        let profile = self.permission_profile.unwrap_or_default();
        Ok(HostInvocationRequest {
            model_settings: HostModelRequest {
                endpoint: self.endpoint.clone(),
                api_key: self.api_key.clone(),
                model: self.model.clone(),
                swe_mode: self.swe_mode.clone(),
            },
            tool_settings: HostToolRequest {
                process_sandbox: self.runtime.process_sandbox.clone(),
                bounded_file_writes: false, task_worktree: None, task_sandbox_requirement: None,
                project_root: self.project_root.clone(),
                allowed_dirs: self.allowed_dirs.clone(),
                skill_dirs: self.skill_dirs.clone(),
                bash_prefixes: self.bash_prefixes.clone(),
                auto_approve: self.auto_approve.clone(),
                permission_profile: Some(profile),
                tools_enabled: self.tools_enabled.clone(),
                force_tool_set: self.force_tool_set.clone(),
                tool_allowlist: self.tool_allowlist.clone(),
                tool_allowlist_is_authorization_only: self.tool_allowlist_is_authorization_only.clone(),
                remote_tool_plan: self.remote_tool_plan.clone(),
                network_policy: self.network_policy.clone(),
                mcp_capability_catalog: self.mcp_capability_catalog.clone(),
                mcp_server_registry: self.mcp_server_registry.clone(),
            },
            session: HostSessionRequest {
                edited_resend_operation_id: None,
                user_delegation: None,
                main_expert_selection: self.main_expert_selection.clone(),
                supports_reviewed_text_parts: !self.json_output,
                session_id: selected_session,
                permission_scope_id: self.permission_scope_id.clone(),
                save_session: self.save_session.clone(),
                host_submission_id: self.host_submission_id.clone(),
                host_user_message_id: self.host_user_message_id.clone(),
                host_assistant_message_id: self.host_assistant_message_id.clone(),
                expected_configuration_revision: self.expected_configuration_revision,
                expected_parent_message_id: self.expected_parent_message_id.clone(),
                resume_snapshot: self.resume_snapshot.clone(),
                desktop_resume: self.desktop_resume.clone(),
                desktop_cancel: self.desktop_cancel.clone(),
                desktop_goal_control: None, desktop_goal_wake: None, goal_execution_id: None,
                queued_user_command_id: None,
                host_task_execution: self.host_task_execution.clone(),
                automation_execution_lease: self.automation_execution_lease.clone(),
                automation_execution_spec: self.automation_execution_spec.clone(),
                automation_artifacts: self.automation_artifacts.clone(),
            },
            input: HostInputRequest {
                message: self.message.clone(),
                host_input_origin: self.host_input_origin.clone(),
                host_turn_context: self.host_turn_context.clone(),
                host_context_components: self.host_context_components.clone(),
                host_attachments: self.host_attachments.clone(),
                context_files: self.context_files.clone(),
                expected_artifact_refs: self.expected_artifact_refs.clone(),
                timezone: self.timezone.clone(),
            },
            execution: HostExecutionRequest {
                execution_backend: self.execution_backend.clone(),
                external_execution_session_id: self.external_execution_session_id.clone(),
                external_execution_model: self.external_execution_model.clone(),
                external_execution_model_provider: self.external_execution_model_provider.clone(),
                max_iterations: self.max_iterations.clone(),
            },
            runtime: self.runtime.host_runtime_options(),
        })
    }
    pub(crate) fn host_execution_settings(&self) -> crate::host_execution_settings::HostExecutionSettings {
        crate::host_execution_settings::HostExecutionSettings {
            memory_workspace_id: crate::host_execution_settings::execution_workspace_identity(&self.project_root, None, None, self.runtime.process_sandbox.as_ref()).ok().map(|identity| identity.workspace_id),
            bounded_file_writes: false, task_worktree: None, task_sandbox_requirement: None,
            external: self.external_execution_settings(),
            reasoning_tier: self.runtime.reasoning_tier,
            native_model_route: self.runtime.native_model_route.is_some(),
            admitted_memory_policy: self.runtime.admitted_memory_policy.clone(),
        }
    }

    pub(crate) fn external_execution_settings(&self) -> crate::external_execution_settings::ExternalExecutionSettings {
        crate::external_execution_settings::ExternalExecutionSettings {
            project_root: self.project_root.clone(),
            execution_backend: self.execution_backend.clone(),
            external_execution_session_id: self.external_execution_session_id.clone(),
            external_execution_model: self.external_execution_model.clone(),
            external_execution_model_provider: self.external_execution_model_provider.clone(),
            host_context_components: self.host_context_components.clone(),
            host_attachments: self.host_attachments.clone(),
            budget: self.runtime.external_execution.clone(),
            permission_profile: self.permission_profile.unwrap_or_default(),
            host_input_origin: self.host_input_origin,
        }
    }

    /// Called after product/session/request overrides have been composed.
    /// The immutable value cannot reload files, read stdin, or mutate Config.
    pub(crate) fn model_invocation_settings(&self) -> Result<crate::model_invocation_settings::ModelInvocationSettings, String> {
        use crate::model_invocation_settings::{ModelInvocationSettings, ModelInvocationOptions};
        let options = ModelInvocationOptions {
                provider: self.runtime.provider.clone(), siwc: self.runtime.siwc.clone(), native_model_route: self.runtime.native_model_route.clone(),
                llm_fallbacks: self.runtime.llm_fallbacks.clone(), max_output_tokens: self.runtime.max_output_tokens,
                chat_output_token_field: self.runtime.chat_output_token_field,
                reasoning_selection: self.runtime.reasoning_selection.clone(), output_admission: self.runtime.output_admission.clone(),
                compaction: self.runtime.compaction.clone(),
            };
        if self.execution_backend != "nola" {
            return ModelInvocationSettings::external(self.execution_backend.clone(),
                self.external_execution_model.clone(), self.external_execution_model_provider.clone(),
                options, self.swe_mode).map_err(|error| error.to_string());
        }
        ModelInvocationSettings::new(self.endpoint.clone(), self.api_key.clone(), self.model.clone(),
            options, self.swe_mode).map_err(|error| error.to_string())
    }

    pub fn from_args<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Self::from_args_with_setup(args, &NoConfigurationSetup)
    }

    pub(crate) fn from_args_with_setup<I>(args: I, setup: &dyn ConfigurationSetup) -> Result<Self, String>
    where I: IntoIterator<Item = String>,
    {
        Self::from_entry_args(args, setup).map_err(ConfigStartError::into_message)
    }

    pub(crate) fn from_entry_args<I>(args: I, setup: &dyn ConfigurationSetup) -> Result<Self, ConfigStartError>
    where I: IntoIterator<Item = String>,
    {
        Self::from_args_with_runtime(args, setup, None)
    }

    fn from_args_with_runtime<I>(args: I, setup: &dyn ConfigurationSetup, runtime: Option<RuntimeConfig>) -> Result<Self, ConfigStartError>
    where I: IntoIterator<Item = String>,
    {
        let raw_args: Vec<String> = args.into_iter().collect();
        let pre_double_dash_end = raw_args
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(raw_args.len());
        if raw_args[..pre_double_dash_end]
            .iter()
            .any(|arg| arg == "-h" || arg == "--help")
        {
            return Self::for_help_args(&raw_args[..pre_double_dash_end], runtime).map_err(Into::into);
        }

        let mut permission_profile = permission_profile_from_environment(env::var("NOLA_PERMISSION_PROFILE")).map_err(ConfigStartError::Runtime)?;
        let mut permission_profile_explicit = false;
        let mut args = raw_args.into_iter().peekable();
        let mut command = None;
        let mut message_parts = Vec::new();
        let mut swe_mode = env::var("NOLA_CLI_AGENT_MODE")
            .ok()
            .map(|value| value.eq_ignore_ascii_case("swe"))
            .unwrap_or(false);
        let mut endpoint = String::new();
        let mut api_key = String::new();
        let mut model = String::new();
        let mut allowed_dirs = env_allowed_dirs();
        let mut skill_dirs: Option<Vec<PathBuf>> = None;
        let allowed_dirs_from_env = !allowed_dirs.is_empty();
        let project_root_env = first_env(&["NOLA_CLI_PROJECT_ROOT"]);
        let mut project_root_arg = if project_root_env.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(project_root_env.trim()))
        };
        let mut bash_prefixes = env::var("NOLA_CLI_BASH_PREFIXES")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let bash_prefixes_from_env = !bash_prefixes.is_empty();
        let mut openai_prompt_cache = env::var("NOLA_OPENAI_PROMPT_CACHE")
            .ok()
            .and_then(|raw| parse_bool_flag(&raw));
        let mut openai_prompt_cache_key = env::var("NOLA_OPENAI_PROMPT_CACHE_KEY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let mut openai_prompt_cache_key_prefix = env::var("NOLA_OPENAI_PROMPT_CACHE_KEY_PREFIX")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let mut openai_prompt_cache_retention = env::var("NOLA_OPENAI_PROMPT_CACHE_RETENTION")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        // Zero preserves unbounded execution unless a config file or explicit
        // CLI option selects a bounded execution slice.
        let mut max_iterations = 0;
        let mut max_iterations_explicit = false;
        let (mut debug_mode, mut debug_mode_explicit) = env::var("NOLA_CLI_DEBUG")
            .ok()
            .and_then(|raw| parse_bool_flag(&raw).map(|value| (value, true)))
            .unwrap_or((false, false));
        let mut json_output = false;
        let mut auto_approve = false;
        let mut tools_enabled = true;
        let mut force_tool_set = env::var("NOLA_CLI_FORCE_TOOL_SET")
            .ok()
            .and_then(|value| normalize_force_tool_set(&value).ok())
            .unwrap_or_else(|| "auto".to_string());
        let force_tool_set_from_env = env::var_os("NOLA_CLI_FORCE_TOOL_SET").is_some();
        let mut show_help = false;
        let network_policy_env = first_env(&["NOLA_CLI_NETWORK_POLICY"]);
        let mut network_policy =
            normalize_network_policy(network_policy_env.trim()).unwrap_or("allow".to_string());
        let network_policy_from_env = !network_policy_env.trim().is_empty();
        let mut session_id = None;
        let mut continue_last = false;
        let mut resume_session = false;
        let mut save_session = true;
        let mut output_path = None;
        let mut context_files = Vec::new();
        let mut expected_artifact_refs = Vec::new();
        let mut force = false;
        let mut dry_run = false;
        let mut host_request_stdin = false;
        let mut web_host = first_env(&["NOLA_WEB_HOST", "WEB_HOST"]);
        if web_host.trim().is_empty() {
            web_host = "127.0.0.1".to_string();
        }
        let mut web_port = first_env(&["NOLA_WEB_PORT", "PORT"])
            .parse::<u16>()
            .unwrap_or(5001);
        let mut tailcat_ingress = false;
        let mut tailcat_program = None;
        let mut tailcat_digest = None;
        let mut web_root_arg = env::var_os("NOLA_WEB_ROOT").map(PathBuf::from);
        let mut endpoint_explicit = false;
        let mut api_key_explicit = false;
        let mut model_explicit = false;
        let mut profile_id:Option<String> = None;
        let mut profile_revision:Option<i64> = None;
        let mut allowed_dirs_explicit = false;
        let mut bash_prefixes_explicit = false;
        let mut force_tool_set_explicit = false;
        let mut network_policy_explicit = false;
        let mut language_explicit = false;

        while let Some(arg) = args.next() {
            if arg == "--" {
                if command.is_none() {
                    command = Some(CommandMode::Run);
                }
                message_parts.extend(args);
                break;
            }
            // Global options may precede an explicit subcommand. Do not commit
            // to Run merely because the first token is an option; otherwise
            // `nola --model ... session show <id>` is interpreted as a chat
            // prompt and can even create a spurious conversation.
            if command.is_none() && !is_global_option_token(&arg) {
                match arg.as_str() {
                    "run" => command = Some(CommandMode::Run),
                    "--resume" | "resume" => {
                        command = Some(CommandMode::Run);
                        resume_session = true;
                    }
                    "--continue" | "continue" => {
                        command = Some(CommandMode::Run);
                        continue_last = true;
                    }
                    "init" | "/init" => command = Some(CommandMode::Init),
                    "wiki" => command = Some(CommandMode::Wiki),
                    "web" => command = Some(CommandMode::Web),
                    "server" | "serve" => command = Some(CommandMode::Server),
                    "sessions" => command = Some(CommandMode::Sessions),
                    "config" => command = Some(CommandMode::ConfigFile { action: ConfigFileAction::Show, path: None }),
                    "model" => {
                        command = Some(match args.next().as_deref() {
                            Some("connections") => CommandMode::ModelConnections,
                            Some("manage") => CommandMode::ModelManage,
                            Some("apply-config") => CommandMode::ModelApplyConfig {
                                path: args
                                    .next_if(|arg| !arg.starts_with('-'))
                                    .map(normalize_cli_file_path)
                                    .transpose()?,
                            },
                            Some("chatgpt") => {
                                let operation = next_command_value(&mut args).ok_or("model chatgpt requires sign-in, reauthorize, status, models, refresh or sign-out")?;
                                if operation == "sign-in" {
                                    let label = next_command_value(&mut args).ok_or("model chatgpt sign-in requires an account label")?;
                                    if label.trim().is_empty() || label.len() > 256 || label.chars().any(char::is_control) { return Err("invalid account label".into()); }
                                    CommandMode::ModelChatgpt { operation, account_id:None, revision:None, label:Some(label) }
                                } else {
                                    if !matches!(operation.as_str(), "reauthorize" | "status" | "models" | "refresh" | "sign-out") { return Err("unsupported model chatgpt operation".into()); }
                                    let account_id = next_command_value(&mut args).ok_or("model chatgpt operation requires account ID")?;
                                    let parsed = uuid::Uuid::parse_str(&account_id).map_err(|_| "account ID must be a UUID")?;
                                    if parsed.to_string() != account_id { return Err("account ID must use canonical UUID form".into()); }
                                    let revision = if operation == "status" {
                                        None
                                    } else if operation == "sign-out" {
                                        match args.next_if(|arg| !arg.starts_with('-')) {
                                            Some(value) => Some(parse_positive_revision(&value, "account revision")?),
                                            None => resolve_latest_chatgpt_account_revision(&account_id).map_err(ConfigStartError::Runtime)?,
                                        }
                                    } else {
                                        let raw = next_command_value(&mut args).ok_or("model chatgpt operation requires Core account revision")?;
                                        Some(parse_positive_revision(&raw, "account revision")?)
                                    };
                                    CommandMode::ModelChatgpt { operation, account_id:Some(account_id), revision, label:None }
                                }
                            },
                            Some(operation @ ("discover" | "configure")) => {
                                let connection_id = next_command_value(&mut args).ok_or("model operation requires connection ID")?;
                                if operation == "discover" {
                                    let revision = match args.next_if(|arg| !arg.starts_with('-')) {
                                        Some(raw) => parse_positive_revision(&raw, "connection revision")?,
                                        None => resolve_latest_connection_revision(&connection_id).map_err(ConfigStartError::Runtime)?,
                                    };
                                    CommandMode::ModelDiscover { connection_id, revision }
                                } else {
                                    let raw_rev = next_command_value(&mut args).ok_or("model operation requires connection revision")?;
                                    let revision = parse_positive_revision(&raw_rev, "connection revision")?;
                                    let selected = next_command_value(&mut args).ok_or("model configure requires selected catalog key or none")?;
                                    let selected_model_id = if selected == "none" { None } else { Some(selected) };
                                    let raw_keys = next_command_value(&mut args).ok_or("model configure requires enabled catalog keys as a JSON array or comma-separated list")?;
                                    let enabled_model_ids = parse_configure_enabled_keys(&raw_keys)?;
                                    if enabled_model_ids.iter().any(|key| key.trim().is_empty()) || enabled_model_ids.iter().collect::<std::collections::HashSet<_>>().len() != enabled_model_ids.len() {
                                        return Err("enabled catalog keys must be nonempty and unique".into());
                                    }
                                    if selected_model_id.as_ref().is_some_and(|key| !enabled_model_ids.contains(key)) {
                                        return Err("selected catalog key must belong to the enabled set".into());
                                    }
                                    CommandMode::ModelConfigure { connection_id, revision, enabled_model_ids, selected_model_id }
                                }
                            },
                            Some("probe") => {
                                let mut positional = Vec::new();
                                while positional.len() < 4 {
                                    let Some(token) = args.next_if(|arg| !arg.starts_with('-')) else {
                                        break;
                                    };
                                    positional.push(token);
                                }
                                let (connection_id, revision, model_id, protocol) = match positional.as_slice() {
                                    [conn, rev_raw, model_id, protocol] => (
                                        conn.clone(),
                                        parse_positive_revision(rev_raw, "connection revision")?,
                                        model_id.clone(),
                                        protocol.clone(),
                                    ),
                                    [conn, model_id, protocol] => (
                                        conn.clone(),
                                        resolve_latest_connection_revision(conn).map_err(ConfigStartError::Runtime)?,
                                        model_id.clone(),
                                        protocol.clone(),
                                    ),
                                    _ => {
                                        return Err("Usage: nola model probe <connection-id> [<revision>] <model-id> <responses|chat_completions|anthropic>".into());
                                    }
                                };
                                if !matches!(protocol.as_str(), "responses" | "chat_completions" | "anthropic") { return Err("unsupported probe protocol".into()); }
                                CommandMode::ModelProbe { connection_id, revision, model_id, protocol }
                            },
                            Some("override") => {
                                let connection_id = next_command_value(&mut args).ok_or("model override requires connection ID")?;
                                let revision = parse_positive_revision(&next_command_value(&mut args).ok_or("model override requires connection revision")?, "connection revision")?;
                                let model_id = next_command_value(&mut args).ok_or("model override requires model ID")?;
                                let protocol = next_command_value(&mut args).ok_or("model override requires protocol")?;
                                if !matches!(protocol.as_str(), "responses" | "chat_completions" | "anthropic" | "siwc_responses") { return Err("unsupported override protocol".into()); }
                                let value = next_command_value(&mut args).ok_or("model override requires clear or an explicit capabilities JSON object")?;
                                let capabilities = if value == "clear" { None } else {
                                    let capabilities: nola_application::ModelExecutionCapabilities = serde_json::from_str(&value)
                                        .map_err(|_| "invalid capabilities object; explicitly declare toolCalls/imageInput/reasoning/samplingParameters")?;
                                    nola_application::ModelExecutionCapabilities::from_source(&serde_json::json!({"executionCapabilities":capabilities}))?;
                                    Some(capabilities)
                                };
                                CommandMode::ModelOverride { connection_id, revision, model_id, protocol, capabilities }
                            },
                            Some("import") => {
                                let source = next_command_value(&mut args).ok_or("model import requires open_code/codex/kimi_code and a configuration path")?;
                                if !matches!(source.as_str(), "open_code" | "codex" | "kimi_code") { return Err("unsupported import source".into()); }
                                let raw_path = next_command_value(&mut args).ok_or("model import requires a configuration path")?;
                                let path = normalize_cli_file_path(raw_path)?;
                                let mut credential_path = None;
                                let mut include_credentials = false;
                                let mut allow_environment = false;
                                while args.peek().is_some() {
                                    if let Some(raw_cred) = peek_option_value(&mut args, "--credential-path", "credential path required")? {
                                        credential_path = Some(normalize_cli_file_path(raw_cred)?);
                                        continue;
                                    }
                                    match args.peek().map(String::as_str) {
                                        Some("--include-credentials") => { args.next(); include_credentials = true; },
                                        Some("--allow-environment") => { args.next(); allow_environment = true; },
                                        _ => break,
                                    }
                                }
                                if credential_path.is_some() && !include_credentials { return Err("--credential-path requires --include-credentials".into()); }
                                CommandMode::ModelImport { source, path, credential_path, include_credentials, allow_environment }
                            },
                            _ => return Err("model requires connections, apply-config, manage, chatgpt, discover, configure, probe, override or import".into()),
                        });
                    }
                    "room" => {
                        let action = args.next().ok_or("room requires list/directory/create/show/join/send/recover")?;
                        if !matches!(action.as_str(), "list" | "directory" | "create" | "show" | "join" | "send" | "recover") {
                            return Err(format!("unsupported room action: {action}").into());
                        }
                        let value = if matches!(action.as_str(), "create" | "show" | "join" | "send" | "recover") {
                            Some(next_command_value(&mut args).ok_or("room action requires a title or room ID")?)
                        } else { None };
                        let mut agent_id = if action == "join" {
                            Some(next_command_value(&mut args).ok_or("room join requires an agent ID")?)
                        } else if action == "send" {
                            peek_option_value(&mut args, "--to", "--to requires a participant ID")?
                        } else {
                            None
                        };
                        let text = if action == "send" { Some(next_command_value(&mut args).ok_or("room send requires message text")?) } else { None };
                        if action == "send" && agent_id.is_none() {
                            agent_id = peek_option_value(&mut args, "--to", "--to requires a participant ID")?;
                        }
                        command = Some(CommandMode::Room { action, value, agent_id, text });
                    }
                    "plugin" => {
                        let action = args.next().ok_or("plugin requires inspect/install/list/enable/disable/remove/credential-set/credential-clear")?;
                        if let Some(runtime) = crate::harness_plugin::Harness::named(&action) {
                            let operation = crate::harness_plugin::HarnessPluginCommand::parse(runtime, args)?;
                            command = Some(CommandMode::HarnessPlugin(operation));
                            break;
                        }
                        if !matches!(action.as_str(), "inspect" | "install" | "list" | "enable" | "disable" | "remove" | "credential-set" | "credential-clear") {
                            return Err(format!("unsupported plugin action: {action}").into());
                        }
                        let mode = if action == "list" {
                            let after = peek_option_value(&mut args, "--after", "plugin list --after requires a plugin id")?;
                            PluginCommandMode::List { after }
                        } else {
                            let value = next_command_value(&mut args).ok_or("plugin action requires a directory or plugin id")?;
                            match action.as_str() {
                                "inspect" => PluginCommandMode::Inspect(value),
                                "install" => PluginCommandMode::Install(value),
                                "enable" => PluginCommandMode::Enable(value),
                                "disable" => PluginCommandMode::Disable(value),
                                "remove" => PluginCommandMode::Remove(value),
                                "credential-set" | "credential-clear" => {
                                    let reference = next_command_value(&mut args).ok_or("plugin credential command requires a reference")?;
                                    if action == "credential-set" {
                                        PluginCommandMode::CredentialSet { id: value, reference }
                                    } else { PluginCommandMode::CredentialClear { id: value, reference } }
                                }
                                _ => return Err(format!("unsupported plugin action: {action}").into()),
                            }
                        };
                        command = Some(CommandMode::Plugin(mode));
                    }
                    "session" => {
                        let action = args
                            .next()
                            .ok_or_else(|| tr(Message::SessionRequiresAction).to_string())?;
                        if !matches!(action.as_str(), "show" | "part" | "sources" | "delete" | "rm") {
                            return Err(format!("{} {action}", tr(Message::UnknownSessionActionPrefix)).into());
                        }
                        let id = next_command_value(&mut args).ok_or_else(|| {
                            format!("session {action} {}", tr(Message::RequiresSessionIdSuffix))
                        })?;
                        command = Some(match action.as_str() {
                            "show" => CommandMode::SessionShow(id),
                            "part" => {
                                let message_id = next_command_value(&mut args).ok_or("session part requires <conversation> <message> <part> <conversation-revision>")?;
                                let part_id = next_command_value(&mut args).ok_or("session part requires <conversation> <message> <part> <conversation-revision>")?;
                                let revision = args.next().ok_or("session part requires <conversation> <message> <part> <conversation-revision>")?
                                    .parse::<i64>().map_err(|_| "session part requires a positive conversation revision from conversation_projection.revision")?;
                                if revision < 1 {
                                    return Err(
                                        "session part requires a positive conversation revision from conversation_projection.revision".into()
                                    );
                                }
                                CommandMode::SessionPart {
                                    conversation_id: id,
                                    message_id,
                                    part_id,
                                    revision,
                                }
                            }
                            "sources" => {
                                let compaction_part_id = match args.peek() {
                                    Some(value) if !value.starts_with('-') => args.next(),
                                    _ => None,
                                };
                                CommandMode::SessionSources {
                                    conversation_id: id,
                                    compaction_part_id,
                                }
                            }
                            "delete" | "rm" => CommandMode::SessionDelete(id),
                            _ => {
                                return Err(format!(
                                    "{} {action}",
                                    tr(Message::UnknownSessionActionPrefix)
                                ).into())
                            }
                        });
                    }
                    _ if arg.starts_with('-') => {
                        return Err(format!("{} {arg}", tr(Message::UnknownCommandPrefix)).into())
                    }
                    _ => {
                        command = Some(CommandMode::Run);
                        message_parts.push(arg);
                    }
                }
                continue;
            }

            if arg == "-h" || arg == "--help" {
                show_help = true;
            } else if arg == "--json" {
                json_output = true;
            } else if arg == "--host-request-stdin" {
                host_request_stdin = true;
            } else if arg == "--debug" {
                debug_mode = true;
                debug_mode_explicit = true;
            } else if let Some(value) = option_value(&arg, "--permission-profile", &mut args)? {
                let profile = crate::permissions::PermissionProfile::from_canonical_name(&value)
                    .ok_or_else(|| "--permission-profile must be cautious or allow_all".to_string())?;
                permission_profile = Some(profile);
                permission_profile_explicit = true;
            } else if arg == "--yes" || arg == "-y" {
                auto_approve = true;
                if !permission_profile_explicit {
                    permission_profile = Some(crate::permissions::PermissionProfile::AllowAll);
                }
            } else if arg == "--no-tools" {
                tools_enabled = false;
            } else if let Some(value) = option_value(&arg, "--max-iterations", &mut args)? {
                max_iterations = value
                    .parse::<usize>()
                    .map_err(|_| tr(Message::MaxIterationsPositive).to_string())?;
                max_iterations_explicit = true;
            } else if let Some(value) = option_value(&arg, "--lang", &mut args)? {
                let language = parse_language_tag(&value).ok_or_else(|| {
                    format!("--lang {}", tr(Message::InteractiveInvalidLanguageValue))
                })?;
                set_language(language);
                language_explicit = true;
            } else if let Some(value) = option_value(&arg, "--force-tool-set", &mut args)? {
                force_tool_set_explicit = true;
                force_tool_set = normalize_force_tool_set(&value)?;
            } else if arg == "--continue" || arg == "-c" {
                continue_last = true;
            } else if arg == "--resume" {
                resume_session = true;
            } else if arg == "--swe-mode" {
                swe_mode = true;
            } else if arg == "--no-save" {
                save_session = false;
            } else if arg == "--force" {
                force = true;
            } else if arg == "--dry-run" {
                dry_run = true;
            } else if let Some(value) = option_value(&arg, "--profile", &mut args)? {
                if profile_id.is_some() || value.is_empty() || value.len()>512 || value.chars().any(char::is_control) {return Err("invalid or duplicate --profile".into());}
                profile_id=Some(value);
            } else if let Some(value) = option_value(&arg, "--profile-revision", &mut args)? {
                let revision=value.parse::<i64>().map_err(|_|"invalid --profile-revision")?;
                if revision<1 || profile_revision.is_some() {return Err("invalid or duplicate --profile-revision".into());}
                profile_revision=Some(revision);
            } else if let Some(value) = option_value(&arg, "--endpoint", &mut args)? {
                endpoint_explicit = true;
                endpoint = value;
            } else if let Some(value) = option_value(&arg, "--api-key", &mut args)? {
                api_key_explicit = true;
                api_key = value;
            } else if let Some(value) = short_or_long_option_value(&arg, "--model", "-m", &mut args)? {
                model_explicit = true;
                model = value;
            } else if let Some(value) = option_value(&arg, "--allow-dir", &mut args)? {
                if !allowed_dirs_explicit {
                    allowed_dirs.clear();
                }
                allowed_dirs_explicit = true;
                allowed_dirs.push(PathBuf::from(value));
            } else if let Some(value) = option_value(&arg, "--skill-dir", &mut args)? {
                skill_dirs
                    .get_or_insert_with(Vec::new)
                    .push(PathBuf::from(value));
            } else if arg == "--no-skills" {
                skill_dirs = Some(Vec::new());
            } else if let Some(value) = option_value(&arg, "--project", &mut args)? {
                project_root_arg = Some(PathBuf::from(value));
            } else if let Some(value) = option_value(&arg, "--output", &mut args)? {
                output_path = Some(PathBuf::from(value));
            } else if let Some(value) = option_value(&arg, "--context-file", &mut args)? {
                context_files.push(PathBuf::from(value));
            } else if let Some(value) = option_value(&arg, "--expected-artifact", &mut args)? {
                let value = value.trim();
                if value.is_empty() {
                    return Err("--expected-artifact requires a non-empty resource".into());
                }
                if !expected_artifact_refs
                    .iter()
                    .any(|current| current == value)
                {
                    expected_artifact_refs.push(value.to_string());
                }
            } else if let Some(value) = option_value(&arg, "--tailcat-helper", &mut args)? {
                tailcat_program = Some(PathBuf::from(value));
            } else if let Some(value) = option_value(&arg, "--tailcat-helper-sha256", &mut args)? {
                tailcat_digest = Some(value);
            } else if arg == "--tailcat-ingress" {
                tailcat_ingress = true;
            } else if let Some(value) = option_value(&arg, "--host", &mut args)? {
                web_host = value;
            } else if let Some(value) = option_value(&arg, "--port", &mut args)? {
                web_port = value
                    .parse::<u16>()
                    .map_err(|_| tr(Message::InvalidPortValue).to_string())?;
            } else if let Some(value) = option_value(&arg, "--web-root", &mut args)? {
                web_root_arg = Some(PathBuf::from(value));
            } else if let Some(value) = option_value(&arg, "--bash-prefix", &mut args)? {
                if !bash_prefixes_explicit {
                    bash_prefixes.clear();
                }
                bash_prefixes_explicit = true;
                bash_prefixes.push(value);
            } else if let Some(value) = option_value(&arg, "--session", &mut args)? {
                session_id = Some(value);
            } else if let Some(value) = option_value(&arg, "--agent-mode", &mut args)? {
                if value.eq_ignore_ascii_case("swe") {
                    swe_mode = true;
                } else if value.eq_ignore_ascii_case("standard") {
                    swe_mode = false;
                } else {
                    return Err(format!("{} {value}", tr(Message::UnknownAgentModePrefix)).into());
                }
            } else if let Some(value) = option_value(&arg, "--network-policy", &mut args)? {
                network_policy_explicit = true;
                network_policy = normalize_network_policy(&value)?;
            } else if arg.starts_with('-') {
                return Err(format!("{} {arg}", tr(Message::UnknownOptionPrefix)).into());
            } else if let Some(CommandMode::ModelApplyConfig { path: slot @ None }) = command.as_mut() {
                *slot = Some(normalize_cli_file_path(arg)?);
            } else {
                message_parts.push(arg);
            }
        }

        let mut command = command.unwrap_or(CommandMode::Run);
        if let CommandMode::ConfigFile { action, path } = &mut command {
            let usage = "nola config [show | edit [path] | check [path] | apply [path] [--dry-run] | example]";
            match message_parts.as_slice() {
                [] => {}
                [verb] if verb == "show" => {}
                [verb] if verb == "example" => *action = ConfigFileAction::Example,
                [verb, rest @ ..] if rest.len() <= 1 && matches!(verb.as_str(), "edit" | "apply" | "check") => {
                    *action = match verb.as_str() {
                        "apply" => ConfigFileAction::Apply,
                        "edit" => ConfigFileAction::Edit,
                        _ => ConfigFileAction::Check,
                    };
                    *path = rest.first().cloned().map(normalize_cli_file_path).transpose()?;
                }
                _ => return Err(usage.into()),
            }
            if dry_run && *action != ConfigFileAction::Apply {
                return Err("--dry-run requires nola config apply".into());
            }
            message_parts.clear();
        }
        // Reject selection ambiguity before loading defaults or invoking setup,
        // which may prompt and persist an unrelated API configuration.
        let cli_profile_selection = match (profile_id, profile_revision) {
            (None, None) => None,
            (Some(id), Some(revision)) => {
                if !matches!(command, CommandMode::Run) || endpoint_explicit || api_key_explicit || model_explicit
                    || continue_last || resume_session || session_id.is_some() || !save_session || host_request_stdin {
                    return Err("--profile requires a new saved conversation and cannot combine with endpoint/model/API key, session/resume/continue, no-save, or Host stdin".into());
                }
                Some((id, revision))
            }
            (Some(id), None) => {
                if !matches!(command, CommandMode::Run) || endpoint_explicit || api_key_explicit || model_explicit
                    || continue_last || resume_session || session_id.is_some() || !save_session || host_request_stdin {
                    return Err("--profile requires a new saved conversation and cannot combine with endpoint/model/API key, session/resume/continue, no-save, or Host stdin".into());
                }
                let revision = resolve_latest_profile_revision(&id).map_err(ConfigStartError::Runtime)?;
                Some((id, revision))
            }
            (None, Some(_)) => {
                return Err("--profile and --profile-revision must be specified together".into());
            }
        };
        if !matches!(command, CommandMode::Run) && !message_parts.is_empty() {
            return Err(tr(Message::UnexpectedCommandArgs).into());
        }

        let raw_project_root = project_root_arg.unwrap_or(
            env::current_dir()
                .map_err(|err| ConfigStartError::Runtime(format!("{} {err}", tr(Message::CurrentDirFailedPrefix))))?,
        );
        let project_root = normalize_path(raw_project_root)?;
        let web_root = normalize_web_root(web_root_arg, &project_root)?;
        let mut runtime = match runtime {
            Some(runtime) => runtime,
            // The command validates its own source. A broken ambient model catalog
            // must not prevent the user from applying a corrected explicit file.
            None if matches!(command, CommandMode::ConfigFile { .. } | CommandMode::ModelApplyConfig { .. }) => RuntimeConfig::default(),
            None if !host_request_stdin && matches!(command, CommandMode::Run | CommandMode::Init | CommandMode::Wiki | CommandMode::Web | CommandMode::Server) =>
                RuntimeConfig::load_for_startup(&project_root).map_err(ConfigStartError::Runtime)?,
            None => RuntimeConfig::load(&project_root).map_err(ConfigStartError::Runtime)?,
        };
        if runtime.uses_file_catalog() && matches!(command, CommandMode::Run | CommandMode::Init | CommandMode::Wiki) {
            if !host_request_stdin && (endpoint_explicit || api_key_explicit || model_explicit) {
                return Err("connection catalog selection cannot combine with endpoint/model/API key flags; use --profile and --profile-revision".into());
            }
        }
        if !max_iterations_explicit {
            max_iterations = runtime.max_iterations.unwrap_or(max_iterations);
        }
        if !endpoint_explicit {
            if let Some(value) = &runtime.endpoint {
                endpoint = value.clone();
            }
        }
        if !api_key_explicit {
            if let Some(value) = &runtime.api_key {
                api_key = value.clone();
            }
        }
        if !model_explicit {
            if let Some(value) = &runtime.model {
                model = value.clone();
            }
        }
        if !allowed_dirs_explicit && !allowed_dirs_from_env {
            allowed_dirs = vec![project_root.clone()];
            for dir in &runtime.allowed_dirs {
                if !allowed_dirs.iter().any(|existing| existing == dir) {
                    allowed_dirs.push(dir.clone());
                }
            }
        }
        if let Some(index) = allowed_dirs
            .iter()
            .position(|existing| existing == &project_root)
        {
            if index > 0 {
                allowed_dirs.remove(index);
                allowed_dirs.insert(0, project_root.clone());
            }
        } else {
            allowed_dirs.insert(0, project_root.clone());
        }
        if !debug_mode_explicit {
            if let Some(value) = runtime.debug_mode {
                debug_mode = value;
            }
        }
        if !bash_prefixes_explicit && !bash_prefixes_from_env && !runtime.bash_prefixes.is_empty() {
            bash_prefixes = runtime.bash_prefixes.clone();
        }
        if !force_tool_set_explicit && !force_tool_set_from_env {
            if let Some(value) = &runtime.force_tool_set {
                force_tool_set = normalize_force_tool_set(value)?;
            }
        }
        if !network_policy_explicit && !network_policy_from_env {
            if let Some(value) = &runtime.network_policy {
                network_policy = normalize_network_policy(value)?;
            }
        }
        if let Some(value) = openai_prompt_cache.take() {
            runtime.openai_prompt_cache = Some(value);
        }
        if let Some(value) = openai_prompt_cache_key.take() {
            runtime.openai_prompt_cache_key = Some(value);
        }
        if let Some(value) = openai_prompt_cache_key_prefix.take() {
            runtime.openai_prompt_cache_key_prefix = Some(value);
        }
        if let Some(value) = openai_prompt_cache_retention.take() {
            runtime.openai_prompt_cache_retention = Some(value);
        }

        // Saved conversations own their model configuration. Argument parsing
        // must not require a global route before the selected conversation (or
        // picker choice) can be hydrated by initialize_or_restore.
        let restore_conversation_model = matches!(command, CommandMode::Run)
            && save_session && (session_id.is_some() || continue_last || resume_session);
        if matches!(command, CommandMode::Run | CommandMode::Init | CommandMode::Wiki)
            && !restore_conversation_model
            && !show_help && !host_request_stdin && endpoint.trim().is_empty()
            && cli_profile_selection.is_none()
            && !runtime.uses_file_catalog()
        {
            if let Some(generated) = setup.configure_missing_model().map_err(ConfigStartError::Runtime)? {
                endpoint = generated.endpoint.clone();
                if !api_key_explicit { api_key = generated.api_key.clone(); }
                if !model_explicit { model = generated.model.clone(); }
                runtime.endpoint = Some(generated.endpoint);
                if !api_key_explicit && !generated.api_key.is_empty() {
                    runtime.api_key = Some(generated.api_key);
                }
                if !model_explicit && !generated.model.is_empty() {
                    runtime.model = Some(generated.model);
                }
            }
        }

        if matches!(
            command,
            CommandMode::Run | CommandMode::Init | CommandMode::Wiki
        ) && endpoint.trim().is_empty()
            && !restore_conversation_model
            && !host_request_stdin
            && !show_help
            && cli_profile_selection.is_none()
            && !runtime.uses_file_catalog()
        {
            return Err(ConfigStartError::Runtime(format!("{}\n  nola config\n  nola config edit", tr(Message::MissingEndpoint))));
        }
        if tailcat_ingress
            && (!matches!(command, CommandMode::Web | CommandMode::Server)
                || web_host.trim().is_empty()
                || web_port == 0)
        {
            return Err(
                "--tailcat-ingress requires a Web/Server listener with a nonzero port".into(),
            );
        }
        let tailcat_helper = match (tailcat_program, tailcat_digest) {
            (None, None) => None,
            (Some(program), Some(digest)) if tailcat_ingress => Some((program, digest)),
            _ => return Err("Tailcat helper requires --tailcat-ingress, an exact executable and SHA256".into()),
        };
        if let Some((program,digest)) = &tailcat_helper {
            crate::tailcat_transport::verified_command(program, digest)?;
        }
        if continue_last && session_id.is_some() {
            return Err(tr(Message::ContinueOrSession).into());
        }
        if resume_session && session_id.is_some() {
            return Err("cannot use --resume with --session".into());
        }
        if resume_session && continue_last {
            return Err("cannot use --resume with --continue".into());
        }

        if !language_explicit {
            if let Some(value) = runtime.language.as_deref() {
                let language = parse_language_tag(value).ok_or_else(|| {
                    ConfigStartError::Runtime(format!("{} {value}", tr(Message::InteractiveInvalidLanguageValue)))
                })?;
                set_language(language);
            }
        }

        if model.trim().is_empty() && endpoint.to_ascii_lowercase().contains("deepseek") {
            model = "deepseek-chat".to_string();
        }

        let mut normalized_allowed_dirs = Vec::new();
        for directory in allowed_dirs {
            let directory = normalize_path(directory)?;
            if !normalized_allowed_dirs.contains(&directory) {
                normalized_allowed_dirs.push(directory);
            }
        }
        let allowed_dirs = normalized_allowed_dirs;
        let skill_dirs = match skill_dirs {
            Some(directories) => {
                let mut normalized = Vec::new();
                for directory in directories {
                    let directory = normalize_path(directory)?;
                    if !normalized.contains(&directory) {
                        normalized.push(directory);
                    }
                }
                Some(normalized)
            }
            None => None,
        };

        Ok(Self {
            cli_profile_selection,
            command,
            show_help,
            resume_session,
            message: message_parts.join(" "),
            host_request_stdin,
            desktop_resume: None,
            desktop_cancel: None,
            host_task_execution: None,
            automation_execution_lease: None,
            automation_execution_spec: None,
            automation_artifacts: Vec::new(),
            host_input_origin: AgentInputOrigin::InteractiveUser,
            host_turn_context: None,
            host_context_components: Vec::new(),
            host_attachments: Vec::new(),
            resume_snapshot: None,
            host_submission_id: None,
            host_user_message_id: None,
            host_assistant_message_id: None,
            main_expert_selection: None,
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            execution_backend: "nola".to_string(),
            external_execution_session_id: None,
            external_execution_model: None,
            external_execution_model_provider: None,
            mcp_capability_catalog: None,
            mcp_server_registry: Vec::new(),
            endpoint,
            api_key,
            swe_mode,
            model,
            project_root,
            allowed_dirs,
            skill_dirs,
            bash_prefixes,
            max_iterations,
            json_output,
            debug_mode,
            auto_approve,
            permission_profile,
            tools_enabled,
            force_tool_set,
            tool_allowlist: None,
            tool_allowlist_is_authorization_only: false,
            remote_tool_plan: None,
            timezone: env::var("TZ").unwrap_or_else(|_| "local".to_string()),
            network_policy,
            session_id,
            permission_scope_id: None,
            continue_last,
            save_session,
            output_path,
            context_files,
            expected_artifact_refs,
            force,
            dry_run,
            runtime,
            web_host,
            web_port,
            tailcat_ingress,
            tailcat_helper,
            web_root,
        })
    }

    /// Builds a deterministic configuration for unit tests that exercise
    /// runtime composition without contacting a provider. Production callers
    /// must continue to supply an endpoint explicitly or through configuration;
    /// keeping this fixture here prevents tests from depending on a developer's
    /// personal `~/.nola` file.
    #[cfg(test)]
    pub(crate) fn for_test() -> Result<Self, String> {
        Self::from_args_with_runtime([
            "nola".to_string(),
            "--endpoint".to_string(), "http://127.0.0.1:11434/v1".to_string(),
            "--api-key".to_string(), String::new(),
            "--model".to_string(), "fixture".to_string(),
        ], &NoConfigurationSetup, Some(RuntimeConfig::default())).map_err(ConfigStartError::into_message)
    }
}

/// Applies the identity-bound Core tool-policy receipt supplied by a desktop
/// Host. The receipt authorizes the capabilities this process may execute;
/// each capability's typed exposure metadata independently controls whether
/// its schema is direct or deferred in a model request.
pub fn apply_desktop_host_tool_policy(config: &mut Config) -> Result<(), String> {
    let encoded = match env::var("NOLA_CORE_TOOL_POLICY_RECEIPT") {
        Ok(value) => value,
        Err(_) if !config.host_request_stdin => return Ok(()),
        Err(_) => return Err("Agent Host tool policy receipt is missing.".to_string()),
    };
    let policy_digest = env::var("NOLA_CORE_TOOL_POLICY_DIGEST")
        .map_err(|_| "Agent Host tool policy digest is missing.".to_string())?;
    let manifest_digest = env::var("NOLA_CORE_TOOL_MANIFEST_DIGEST")
        .map_err(|_| "Agent Host tool manifest digest is missing.".to_string())?;
    let session_id = config
        .session_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Agent Host tool policy requires a session identity.".to_string())?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    config.tool_allowlist = Some(decode_desktop_host_tool_allowlist(
        &encoded,
        &policy_digest,
        &manifest_digest,
        session_id,
        now_ms,
    )?);
    config.tool_allowlist_is_authorization_only = true;
    Ok(())
}


fn normalize_web_root(
    web_root_arg: Option<PathBuf>,
    project_root: &Path,
) -> Result<PathBuf, String> {
    let candidates = if let Some(path) = web_root_arg {
        vec![path]
    } else {
        vec![
            project_root.join("apps/web"),
            project_root.join("web"),
            project_root.join("../web"),
        ]
    };
    for candidate in candidates {
        let expanded = expand_home(&candidate);
        if expanded.exists() {
            return expanded.canonicalize().map_err(|err| {
                format!(
                    "{} {}: {err}",
                    tr(Message::FailedResolvePathPrefix),
                    expanded.display()
                )
            });
        }
    }
    Ok(project_root.join("web"))
}

fn permission_profile_from_environment(value: Result<String, env::VarError>) -> Result<Option<crate::permissions::PermissionProfile>, String> {
    match value {
        Ok(value) => crate::permissions::PermissionProfile::from_canonical_name(&value)
            .map(Some)
            .ok_or_else(|| "NOLA_PERMISSION_PROFILE must be cautious or allow_all".to_string()),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err("NOLA_PERMISSION_PROFILE must be valid UTF-8".to_string()),
    }
}

#[cfg(test)]
#[test]
fn permission_environment_distinguishes_absent_from_invalid_explicit_configuration() {
    assert_eq!(permission_profile_from_environment(Err(env::VarError::NotPresent)).unwrap(), None);
    for name in ["cautious", "allow_all"] {
        assert_eq!(permission_profile_from_environment(Ok(name.into())).unwrap().unwrap().canonical_name(), name);
    }
    for name in ["moderate", "allow-all", "allowall", "", " cautious ", "unknown"] {
        assert!(permission_profile_from_environment(Ok(name.into())).is_err());
    }
    assert!(permission_profile_from_environment(Err(env::VarError::NotUnicode(std::ffi::OsString::from("invalid")))).is_err());
}

pub(crate) fn normalize_network_policy(raw: &str) -> Result<String, String> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "allow" | "ask" | "deny" => Ok(value),
        _ => Err(tr(Message::NetworkPolicyMustBeOneOf).to_string()),
    }
}

pub(crate) fn normalize_force_tool_set(raw: &str) -> Result<String, String> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "auto" | "all" | "write" => Ok(value),
        _ => Err(tr(Message::ForceToolSetMustBeOneOf).to_string()),
    }
}

fn is_global_switch(arg: &str) -> bool {
    matches!(
        arg,
        "--json"
            | "--tailcat-ingress"
            | "--host-request-stdin"
            | "--debug"
            | "--yes"
            | "-y"
            | "--no-tools"
            | "--no-skills"
            | "--continue"
            | "-c"
            | "--swe-mode"
            | "--no-save"
            | "--force"
            | "--dry-run"
    )
}

fn is_global_option_token(arg: &str) -> bool {
    if is_global_switch(arg) || arg == "-m" || arg.starts_with("-m=") {
        return true;
    }
    [
        "--force-tool-set",
        "--max-iterations",
        "--permission-profile",
        "--endpoint",
        "--api-key",
        "--model",
        "--profile",
        "--profile-revision",
        "--allow-dir",
        "--skill-dir",
        "--project",
        "--output",
        "--context-file",
        "--expected-artifact",
        "--host",
        "--port",
        "--web-root",
        "--bash-prefix",
        "--session",
        "--agent-mode",
        "--lang",
        "--network-policy",
    ]
    .iter()
    .any(|name| arg == *name || arg.starts_with(&format!("{name}=")))
}

fn parse_bool_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enable" => Some(true),
        "0" | "false" | "no" | "off" | "disable" => Some(false),
        _ => None,
    }
}

fn first_env(names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_default()
}

fn short_or_long_option_value<I>(
    arg: &str,
    long_name: &str,
    short_name: &str,
    args: &mut std::iter::Peekable<I>,
) -> Result<Option<String>, String>
where
    I: Iterator<Item = String>,
{
    if let Some(value) = option_value(arg, long_name, args)? {
        return Ok(Some(value));
    }
    option_value(arg, short_name, args)
}

// A trailing option cannot supply a missing ID, title or message. An explicit
// separator lets the caller provide option-shaped text such as a room title.
fn next_command_value<I: Iterator<Item = String>>(args: &mut std::iter::Peekable<I>) -> Option<String> {
    if args.peek().is_some_and(|arg| arg == "--") {
        args.next();
        return args.next();
    }
    args.next_if(|arg| !arg.starts_with("--") && !is_global_option_token(arg))
}

fn peek_option_value<I>(
    args: &mut std::iter::Peekable<I>,
    name: &str,
    missing_error: &str,
) -> Result<Option<String>, String>
where
    I: Iterator<Item = String>,
{
    let Some(token) = args.peek() else {
        return Ok(None);
    };
    if token == name {
        args.next();
        return next_command_value(args)
            .map(Some)
            .ok_or_else(|| missing_error.to_string());
    }
    let prefix = format!("{name}=");
    if token.starts_with(&prefix) {
        let token = args.next().unwrap();
        let value = token.strip_prefix(&prefix).unwrap_or_default().to_string();
        if value.is_empty() {
            return Err(missing_error.to_string());
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn parse_positive_revision(raw: &str, label: &str) -> Result<i64, String> {
    let revision = raw
        .parse::<i64>()
        .map_err(|_| format!("invalid {label}"))?;
    if revision < 1 {
        return Err(format!("{label} must be positive"));
    }
    Ok(revision)
}

fn parse_configure_enabled_keys(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return serde_json::from_str::<Vec<String>>(trimmed)
            .map_err(|_| "enabled catalog keys must be a JSON string array".to_string());
    }
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    Ok(trimmed
        .split(',')
        .map(|item| item.trim().to_string())
        .collect())
}

fn normalize_cli_file_path(raw: String) -> Result<String, String> {
    let expanded = expand_home(&PathBuf::from(raw));
    let resolved = if expanded.is_absolute() {
        expanded
    } else {
        env::current_dir()
            .map_err(|err| format!("{} {err}", tr(Message::CurrentDirFailedPrefix)))?
            .join(expanded)
    };
    Ok(resolved.to_string_lossy().into_owned())
}

fn resolve_latest_profile_revision(profile_id: &str) -> Result<i64, String> {
    let store = crate::store::NolaStore::open_default()?;
    store
        .model_profiles()?
        .into_iter()
        .find(|profile| profile.id == profile_id && profile.deleted_at_ms.is_none())
        .map(|profile| profile.revision)
        .ok_or_else(|| {
            format!(
                "Selected Core profile '{profile_id}' is unavailable; pass --profile-revision explicitly"
            )
        })
}

fn resolve_latest_connection_revision(connection_id: &str) -> Result<i64, String> {
    let store = crate::store::NolaStore::open_default()?;
    let mut app = store.application()?;
    let snapshot: Option<nola_domain::model_connection::ModelConnectionSnapshot> =
        app.execute(nola_application::ApplicationCommand::Model {
            command: nola_application::ModelCommand::GetConnection {
                id: connection_id.to_string(),
            },
        })?;
    snapshot
        .map(|item| item.revision)
        .ok_or_else(|| format!("model connection '{connection_id}' was not found; pass an explicit revision"))
}

fn resolve_latest_chatgpt_account_revision(account_id: &str) -> Result<Option<i64>, String> {
    let store = crate::store::NolaStore::open_default()?;
    let mut app = store.application()?;
    let snapshot: Option<nola_domain::model_connection::ModelAccountSnapshot> =
        app.execute(nola_application::ApplicationCommand::Model {
            command: nola_application::ModelCommand::GetAccount {
                id: account_id.to_string(),
            },
        })?;
    Ok(snapshot.map(|item| item.revision))
}

impl Config {
    fn for_help_args(pre_double_dash: &[String], runtime: Option<RuntimeConfig>) -> Result<Self, String> {
        let mut json_output = false;
        let mut iter = pre_double_dash.iter().peekable();
        while let Some(arg) = iter.next() {
            if arg == "--json" {
                json_output = true;
            } else if arg == "--lang" {
                let value = iter
                    .next()
                    .ok_or_else(|| format!("--lang {}", tr(Message::RequiresValueSuffix)))?;
                let language = parse_language_tag(value).ok_or_else(|| {
                    format!("--lang {}", tr(Message::InteractiveInvalidLanguageValue))
                })?;
                set_language(language);
            } else if let Some(value) = arg.strip_prefix("--lang=") {
                let language = parse_language_tag(value).ok_or_else(|| {
                    format!("--lang {}", tr(Message::InteractiveInvalidLanguageValue))
                })?;
                set_language(language);
            }
        }
        let project_root = env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from("."));
        let web_root = normalize_web_root(None, &project_root)?;
        Ok(Self {
            cli_profile_selection: None,
            command: CommandMode::Run,
            show_help: true,
            resume_session: false,
            message: String::new(),
            host_request_stdin: false,
            desktop_resume: None,
            desktop_cancel: None,
            host_task_execution: None,
            automation_execution_lease: None,
            automation_execution_spec: None,
            automation_artifacts: Vec::new(),
            host_input_origin: AgentInputOrigin::InteractiveUser,
            host_turn_context: None,
            host_context_components: Vec::new(),
            main_expert_selection: None,
            host_attachments: Vec::new(),
            resume_snapshot: None,
            host_submission_id: None,
            host_user_message_id: None,
            host_assistant_message_id: None,
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            execution_backend: "nola".to_string(),
            external_execution_session_id: None,
            external_execution_model: None,
            external_execution_model_provider: None,
            mcp_capability_catalog: None,
            mcp_server_registry: Vec::new(),
            endpoint: String::new(),
            api_key: String::new(),
            swe_mode: false,
            model: String::new(),
            project_root: project_root.clone(),
            allowed_dirs: vec![project_root],
            skill_dirs: None,
            bash_prefixes: Vec::new(),
            max_iterations: 0,
            json_output,
            debug_mode: false,
            auto_approve: false,
            permission_profile: None,
            tools_enabled: true,
            force_tool_set: "auto".to_string(),
            tool_allowlist: None,
            tool_allowlist_is_authorization_only: false,
            remote_tool_plan: None,
            timezone: env::var("TZ").unwrap_or_else(|_| "local".to_string()),
            network_policy: "allow".to_string(),
            session_id: None,
            permission_scope_id: None,
            continue_last: false,
            save_session: true,
            output_path: None,
            context_files: Vec::new(),
            expected_artifact_refs: Vec::new(),
            force: false,
            dry_run: false,
            runtime: runtime.unwrap_or_default(),
            web_host: "127.0.0.1".to_string(),
            web_port: 5001,
            tailcat_ingress: false,
            tailcat_helper: None,
            web_root,
        })
    }
}

fn option_value<I>(
    arg: &str,
    name: &str,
    args: &mut std::iter::Peekable<I>,
) -> Result<Option<String>, String>
where
    I: Iterator<Item = String>,
{
    if arg == name {
        return next_command_value(args)
            .map(Some)
            .ok_or_else(|| format!("{name} {}", tr(Message::RequiresValueSuffix)));
    }
    let prefix = format!("{name}=");
    if let Some(value) = arg.strip_prefix(&prefix) {
        return Ok(Some(value.to_string()));
    }
    Ok(None)
}

fn env_allowed_dirs() -> Vec<PathBuf> {
    env::var_os("NOLA_CLI_ALLOWED_DIRS")
        .map(|raw| env::split_paths(&raw).collect())
        .unwrap_or_default()
}

fn normalize_path(path: PathBuf) -> Result<PathBuf, String> {
    let expanded = expand_home(&path);
    if expanded.exists() {
        return expanded.canonicalize().map_err(|err| {
            format!(
                "{} {}: {err}",
                tr(Message::FailedResolvePathPrefix),
                expanded.display()
            )
        });
    }
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        env::current_dir()
            .map_err(|err| format!("{} {err}", tr(Message::CurrentDirFailedPrefix)))?
            .join(expanded)
    })
}

pub(crate) fn help_topic(args: &[String]) -> Option<&str> {
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let arg = word.as_str();
        if is_global_switch(arg) || matches!(arg, "--help" | "-h") {
            continue;
        }
        if is_global_option_token(arg) {
            if !arg.contains('=') {
                words.next();
            }
            continue;
        }
        return match arg {
            "model" if words.next().is_some_and(|subcommand| subcommand == "apply-config") => Some("model apply-config"),
            "config" | "room" | "model" | "session" | "sessions" | "plugin" | "web" | "server"
                | "init" | "wiki" => Some(arg),
            "serve" => Some("server"),
            "/init" => Some("init"),
            _ => None,
        };
    }
    None
}

const MODEL_USAGE: &str = "nola model connections
nola model apply-config [config-path] [--dry-run]
nola model acquire hardware
nola model acquire engine-status|setup-engine <llama.cpp|vllm|mlx>
nola model acquire recommend <llama.cpp|vllm|mlx> <huggingface|modelscope> [model-name-or-repository]
nola model acquire confirm <engine> <source> <plan-json>
nola model acquire search|install <engine> <source> <model-name-or-repository>
nola model manage
nola model chatgpt sign-in <label> | status <account-id> | reauthorize|models|refresh <account-id> <revision> | sign-out <account-id> [revision]
nola model discover <connection-id> [revision]
nola model configure <connection-id> <revision> <selected-key|none> <enabled-keys-json-or-csv>
  Build each catalog key from the discovered protocol and model_id, for example responses:my-model.
nola model probe <connection-id> [revision] <model-id> <responses|chat_completions|anthropic>
nola model override <connection-id> <revision> <model-id> <protocol> <clear|capabilities-json>
nola model import <open_code|codex|kimi_code> <path> [--include-credentials] [--credential-path <path>] [--allow-environment]";

pub fn print_help(topic: Option<&str>) {
    use std::io::Write;
    let mut output = std::io::stdout().lock();
    macro_rules! println {
        ($($args:tt)*) => {
            if let Err(error) = writeln!(output, $($args)*) {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    eprintln!("cannot write help: {error}");
                }
                return;
            }
        };
    }

    let text = help_text();
    if let Some(topic) = topic {
        if topic == "config" {
            println!("{}", crate::config_files::usage());
            return;
        }
        if topic == "model apply-config" {
            println!("nola model apply-config [config-path] [--dry-run]");
            println!("{}", crate::i18n::ui_text(crate::i18n::UiText::ApplyConfigHelp));
            return;
        }
        if topic == "model" {
            println!("{MODEL_USAGE}");
            return;
        }
        let options: &[&str] = match topic {
            "web" => &["--project", "--host", "--port", "--web-root"],
            "server" => &["--project", "--host", "--port"],
            "init" | "wiki" => &["--project", "--output", "--force", "--dry-run", "--yes"],
            "sessions" | "session" | "plugin" | "room" => &["--json"],
            _ => &[],
        };
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or("").trim_end_matches(',');
            if (first == "nola" && words.next() == Some(topic)) || options.contains(&first) {
                println!("{}", line.trim_start());
            }
        }
        return;
    }
    let mut lines = text.lines();
    if let Some(title) = lines.next() {
        println!("{title}");
        println!("{}", "-".repeat(title.chars().count().max(8)));
        for line in lines {
            println!("{line}");
        }
    }

}

#[cfg(test)]
mod tests {
    #[test]
    fn web_root_defaults_remain_user_project_paths_after_source_reorganization() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        assert_eq!(
            super::normalize_web_root(None, &project).unwrap(),
            project.join("web")
        );

        let sibling_web = directory.path().join("web");
        std::fs::create_dir_all(&sibling_web).unwrap();
        assert_eq!(
            super::normalize_web_root(None, &project).unwrap(),
            sibling_web.canonicalize().unwrap()
        );

        std::fs::create_dir_all(project.join("web")).unwrap();
        assert_eq!(
            super::normalize_web_root(None, &project).unwrap(),
            project.join("web").canonicalize().unwrap()
        );

        std::fs::create_dir_all(project.join("apps/web")).unwrap();
        assert_eq!(
            super::normalize_web_root(None, &project).unwrap(),
            project.join("apps/web").canonicalize().unwrap()
        );

        let explicit = project.join("web");
        assert_eq!(
            super::normalize_web_root(Some(explicit.clone()), &project).unwrap(),
            explicit.canonicalize().unwrap()
        );
    }

    #[test]
    fn tailcat_ingress_requires_an_explicit_private_host_listener() {
        let parse = |args: &[&str]| Config::from_args(args.iter().map(|arg| arg.to_string()));
        for args in [
            vec!["server", "--tailcat-ingress", "--host", "127.0.0.1", "--port", "5001"],
            vec!["server", "--tailcat-ingress", "--host", "0.0.0.0", "--port", "5001"],
            vec!["web", "--tailcat-ingress", "--host", "192.168.1.10", "--port", "5001"],
        ] {
            let configured = parse(&args).unwrap();
            assert!(configured.tailcat_ingress);
        }
        assert!(!parse(&["server"]).unwrap().tailcat_ingress);
        for args in [
            vec!["server", "--tailcat-ingress", "--port", "0"],
            vec!["web", "--tailcat-ingress", "--host", "0.0.0.0", "--port", "0"],
            vec!["sessions", "--tailcat-ingress"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn model_profile_selection_requires_exact_revision_and_new_conversation() {
        let parse=|args:&[&str]|super::Config::from_args(args.iter().map(|s|s.to_string()));
        let selected=parse(&["run","--profile","owned-profile","--profile-revision","2","hello"]).unwrap();
        assert_eq!(selected.cli_profile_selection,Some(("owned-profile".into(),2)));
        assert_eq!(selected.message,"hello");
        for args in [vec!["run","--profile","x"],vec!["run","--profile-revision","1"],vec!["run","--profile","x","--profile-revision","0"]] {assert!(parse(&args).is_err());}
        for extra in [vec!["--model","x"],vec!["--endpoint","http://fixture"],vec!["--api-key","fixture"],vec!["--session","existing"],vec!["--continue"],vec!["--resume"],vec!["--no-save"],vec!["--host-request-stdin"]] {
            let mut args=vec!["run","--profile","x","--profile-revision","1"];args.extend(extra);assert!(parse(&args).is_err());
        }
        let value=serde_json::to_value(&selected).unwrap();assert!(value.get("cli_profile_selection").is_none());
    }
    #[test]
    fn model_chatgpt_lifecycle_has_no_inference_or_source_token_command() {
        let parse=|args:&[&str]| super::Config::from_args(args.iter().map(|value|value.to_string()));
        assert!(matches!(parse(&["model","chatgpt","sign-in","Personal"]).unwrap().command,super::CommandMode::ModelChatgpt {ref operation,account_id:None,..} if operation=="sign-in"));
        let id="11111111-1111-4111-8111-111111111111";
        assert!(matches!(parse(&["model","chatgpt","status",id]).unwrap().command,super::CommandMode::ModelChatgpt {revision:None,..}));
        for operation in ["reauthorize","models","refresh","sign-out"] {
            assert!(matches!(parse(&["model","chatgpt",operation,id,"2"]).unwrap().command,super::CommandMode::ModelChatgpt {revision:Some(2),..}));
            assert!(parse(&["model","chatgpt",operation,id,"0"]).is_err());
        }
        for operation in ["chat","run","import-token"] {assert!(parse(&["model","chatgpt",operation,id]).is_err());}
        assert!(parse(&["model","chatgpt","status","codex-owner"]).is_err());
    }

    #[test]
    fn model_management_commands_keep_explicit_revision_and_enabled_set() {
        let parse = |args: &[&str]| super::Config::from_args(args.iter().map(|value| value.to_string()));
        assert!(matches!(parse(&["model", "manage"]).unwrap().command, super::CommandMode::ModelManage));
        assert!(matches!(parse(&["model", "apply-config", "/tmp/config.json"]).unwrap().command,
            super::CommandMode::ModelApplyConfig { path } if path.as_deref() == Some("/tmp/config.json")));
        assert!(matches!(parse(&["model", "apply-config"]).unwrap().command,
            super::CommandMode::ModelApplyConfig { path: None }));
        assert!(matches!(parse(&["model", "discover", "connection", "3"]).unwrap().command,
            super::CommandMode::ModelDiscover { revision:3, .. }));
        let configured = parse(&["model", "configure", "connection", "3", "responses:exact:model", r#"["responses:exact:model"]"#]).unwrap();
        assert!(matches!(configured.command, super::CommandMode::ModelConfigure { revision:3, selected_model_id:Some(ref selected), ref enabled_model_ids, .. }
            if selected == "responses:exact:model" && enabled_model_ids == &["responses:exact:model"]));
        assert!(matches!(parse(&["model", "configure", "connection", "3", "none", "[]"]).unwrap().command,
            super::CommandMode::ModelConfigure { selected_model_id:None, ref enabled_model_ids, .. } if enabled_model_ids.is_empty()));
        for args in [
            vec!["model", "discover", "connection", "0"],
            vec!["model", "configure", "connection", "3", "responses:a", "[]"],
            vec!["model", "configure", "connection", "3", "none", r#"["responses:a","responses:a"]"#],
            vec!["model", "configure", "connection", "3", "none", "{}"],
            vec!["model", "configure", "connection", "3", "none", r#"[""]"#],
        ] { assert!(parse(&args).is_err(), "must reject {args:?}"); }
    }

    #[test]
    fn model_override_requires_explicit_valid_capabilities_or_clear() {
        let parse = |value: &str| super::Config::from_args(
            ["model", "override", "connection", "2", "model", "responses", value].into_iter().map(str::to_string));
        let cleared = parse("clear").unwrap();
        assert!(matches!(cleared.command, super::CommandMode::ModelOverride { capabilities:None, revision:2, .. }));
        let declared = parse(r#"{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":false}"#).unwrap();
        assert!(matches!(declared.command, super::CommandMode::ModelOverride { capabilities:Some(caps), .. } if caps.tool_calls));
        let siwc=super::Config::from_args(["model","override","connection","2","model","siwc_responses",r#"{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":false}"#].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(siwc.command,super::CommandMode::ModelOverride {protocol,capabilities:Some(caps),..} if protocol=="siwc_responses" && caps.tool_calls));
        for invalid in ["{}", "null", "true", r#"{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":false,"unknown":true}"#,
            r#"{"toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":false,"samplingRequiresNoReasoning":true}"#] {
            assert!(parse(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn model_probe_requires_an_exact_route_before_confirmation() {
        let valid = super::Config::from_args(["model", "probe", "connection", "2", "model", "responses"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(valid.command, super::CommandMode::ModelProbe {revision: 2, protocol, ..} if protocol == "responses"));
        for args in [vec!["model", "probe"], vec!["model", "probe", "c", "0", "m", "responses"], vec!["model", "probe", "c", "1", "m", "auto"]] {
            assert!(super::Config::from_args(args.into_iter().map(str::to_string)).is_err());
        }
    }
    #[test]
    fn room_commands_require_explicit_action_and_identity() {
        let list = super::Config::from_args(["room", "list"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(list.command, super::CommandMode::Room { action, .. } if action == "list"));
        let join = super::Config::from_args(["room", "join", "room-id", "agent-id"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(join.command, super::CommandMode::Room { value: Some(room), agent_id: Some(agent), .. } if room == "room-id" && agent == "agent-id"));
        let send = super::Config::from_args(["room", "send", "room-id", "search 当前 follow-up"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(send.command, super::CommandMode::Room { agent_id: None, text: Some(text), .. } if text == "search 当前 follow-up"));
        let explicit = super::Config::from_args(["room", "send", "room-id", "hello", "--to", "participant"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(explicit.command, super::CommandMode::Room { agent_id: Some(id), .. } if id == "participant"));
        for args in [vec!["room"], vec!["room", "join", "room-id"], vec!["room", "search"]] {
            assert!(super::Config::from_args(args.into_iter().map(str::to_string)).is_err());
        }
    }
    #[test]
    fn effect_settings_snapshot_excludes_cli_state_and_resolves_permission_precedence() {
        let mut config = super::Config::for_test().unwrap();
        config.project_root = std::path::PathBuf::from("/frozen-workspace");
        config.permission_profile = Some(crate::permissions::PermissionProfile::Cautious);
        config.auto_approve = true;
        config.network_policy = "allow".into();
        config.runtime.external_execution.max_processes = Some(3);
        let admitted = config.host_execution_settings();
        config.project_root = std::path::PathBuf::from("/later-workspace");
        config.runtime.external_execution.max_processes = Some(100);
        config.permission_profile = Some(crate::permissions::PermissionProfile::AllowAll);
        assert_eq!(admitted.external.project_root, std::path::PathBuf::from("/frozen-workspace"));
        assert_eq!(admitted.external.budget.max_processes, Some(3));
        assert_eq!(admitted.external.permission_profile, crate::permissions::PermissionProfile::Cautious);
    }

    use std::path::{Path, PathBuf};

    use base64::Engine as _;

    use super::{CommandMode, Config, PluginCommandMode};
    use crate::config_paths::expand_home_from;
    use crate::{tr, Message};

    #[test]
    fn model_profile_selection_never_invokes_api_setup_even_when_invalid() {
        use crate::configuration_setup::{ConfigurationSetup,GeneratedConfiguration};
        struct Setup(std::cell::Cell<usize>);
        impl ConfigurationSetup for Setup {
            fn configure_missing_model(&self)->Result<Option<GeneratedConfiguration>,String> {
                self.0.set(self.0.get()+1);Err("profile selection must not invoke API setup".into())
            }
        }
        struct RestoreHome(Option<std::ffi::OsString>);
        impl Drop for RestoreHome {fn drop(&mut self) {match &self.0 {Some(value)=>std::env::set_var("NOLA_HOME",value),None=>std::env::remove_var("NOLA_HOME")}}}
        let _guard=crate::store::test_env_lock();let root=tempfile::tempdir().unwrap();
        let _restore=RestoreHome(std::env::var_os("NOLA_HOME"));std::env::set_var("NOLA_HOME",root.path());
        let setup=Setup(std::cell::Cell::new(0));
        let project=root.path().to_str().unwrap();
        let parse=|args:Vec<&str>|Config::from_args_with_setup(args.into_iter().map(str::to_owned),&setup);
        let valid=parse(vec!["run","--project",project,"--profile","owned","--profile-revision","1"]).unwrap();
        assert!(valid.endpoint.is_empty());assert!(valid.api_key.is_empty());
        for args in [
            vec!["run","--profile","owned"],
            vec!["run","--profile-revision","1"],
            vec!["run","--profile","owned","--profile-revision","1","--endpoint",""],
            vec!["run","--profile","owned","--profile-revision","1","--continue"],
        ] {assert!(parse(args).is_err());}
        assert_eq!(setup.0.get(),0,"neither accepted nor rejected profile input may invoke setup");
    }

    #[test]
    fn saved_conversation_selection_defers_model_setup_until_hydration() {
        use crate::configuration_setup::{ConfigurationSetup, GeneratedConfiguration};
        struct Setup(std::cell::Cell<usize>);
        impl ConfigurationSetup for Setup {
            fn configure_missing_model(&self) -> Result<Option<GeneratedConfiguration>, String> {
                self.0.set(self.0.get() + 1);
                Err("fresh model setup required".into())
            }
        }
        struct RestoreHome(Option<std::ffi::OsString>);
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                match &self.0 {
                    Some(value) => std::env::set_var("NOLA_HOME", value),
                    None => std::env::remove_var("NOLA_HOME"),
                }
            }
        }
        let _guard = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let _restore = RestoreHome(std::env::var_os("NOLA_HOME"));
        std::env::set_var("NOLA_HOME", root.path());
        let setup = Setup(std::cell::Cell::new(0));
        let parse = |arguments: &[&str]| Config::from_args_with_setup(
            arguments.iter().copied().chain(["--endpoint", "", "--project", root.path().to_str().unwrap()])
                .map(str::to_owned), &setup);
        for args in [&["--session", "saved"][..], &["--continue"][..], &["--resume"][..]] {
            assert!(parse(args).is_ok());
        }
        assert!(parse(&["--resume", "--session", "saved"]).is_err());
        assert_eq!(setup.0.get(), 0, "navigation must not configure a replacement model");
        for args in [&[][..], &["--session", "saved", "--no-save"][..], &["init"][..], &["wiki"][..]] {
            assert!(parse(args).is_err());
        }
        assert_eq!(setup.0.get(), 4, "fresh/ephemeral invocations still require a model");
    }

    #[test]
    fn configuration_setup_is_explicit_and_cannot_override_explicit_model_credentials() {
        use crate::configuration_setup::{ConfigurationSetup, GeneratedConfiguration};
        struct Setup(std::cell::Cell<usize>);
        impl ConfigurationSetup for Setup {
            fn configure_missing_model(&self) -> Result<Option<GeneratedConfiguration>, String> {
                self.0.set(self.0.get() + 1);
                Ok(Some(GeneratedConfiguration { endpoint: "https://setup.invalid/v1".into(),
                    api_key: "setup-key".into(), model: "setup-model".into() }))
            }
        }
        let _guard = crate::store::test_env_lock();
        let setup = Setup(std::cell::Cell::new(0));
        let args = ["run", "--endpoint", "", "--api-key", "explicit-key", "--model", "explicit-model"];
        assert!(Config::from_args(args.map(str::to_string)).is_err(), "default parsing cannot prompt");
        let configured = Config::from_args_with_setup(args.map(str::to_string), &setup).unwrap();
        assert_eq!(setup.0.get(), 1);
        assert_eq!(configured.endpoint, "https://setup.invalid/v1");
        assert_eq!(configured.api_key, "explicit-key");
        assert_eq!(configured.model, "explicit-model");
        let host = Config::from_args_with_setup(["run", "--host-request-stdin", "--endpoint", ""].map(str::to_string), &setup).unwrap();
        assert_eq!(setup.0.get(), 1, "Host hydration must never invoke terminal setup");
        assert!(host.model_invocation_settings().is_err(), "unhydrated Host settings are not executable");
    }

    #[test]
    fn model_invocation_settings_freeze_values_without_product_entry_or_secret_debug() {
        let _guard = crate::store::test_env_lock();
        let mut config = Config::for_test().unwrap();
        config.api_key = "private-key-for-frozen-settings".into();
        config.endpoint = "https://fixture.invalid/v1?token=private-query".into();
        config.model = "original-model".into();
        let settings = config.model_invocation_settings().unwrap();
        let admission = crate::host_output_admission::admit(&settings, false).unwrap();
        config.model = "changed-model".into();
        config.api_key = "changed-key".into();
        config.command = CommandMode::Web;
        config.message = "different user prompt".into();
        assert_eq!(settings.model(), "original-model");
        assert_eq!(crate::host_output_admission::admit(&settings, false).unwrap(), admission);
        assert!(crate::host_output_admission::validate_snapshot(&config.model_invocation_settings().unwrap(), &admission).is_err());
        let debug = format!("{settings:?}");
        assert!(!debug.contains("private-key"));
        assert!(!debug.contains("private-query"));
    }

    #[test]
    fn tilde_expansion_uses_user_home_not_application_data_root() {
        assert_eq!(
            expand_home_from(Path::new("~/Desktop"), Some(Path::new("/Users/example"))),
            PathBuf::from("/Users/example/Desktop")
        );
    }

    #[test]
    fn parses_run_command_with_explicit_options() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--model=qwen-coder",
                "--allow-dir",
                ".",
                "--bash-prefix",
                "git",
                "hello",
                "repo",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.endpoint, "http://127.0.0.1:11434/v1");
        assert!(matches!(config.command, CommandMode::Run));
        assert_eq!(config.model, "qwen-coder");
        assert_eq!(config.message, "hello repo");
        assert_eq!(config.bash_prefixes, vec!["git"]);
        assert_eq!(config.allowed_dirs.len(), 1);
        assert!(config.save_session);
    }

    #[test]
    fn parses_desktop_host_request_stdin_flag_without_inline_message() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--host-request-stdin",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(config.host_request_stdin);
        assert!(config.message.is_empty());
    }

    #[test]
    fn desktop_host_request_defers_endpoint_validation_until_backend_is_known() {
        let config = Config::from_args(
            ["run", "--host-request-stdin"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert!(config.host_request_stdin);
        assert_eq!(config.execution_backend, "nola");
    }

    #[test]
    fn parses_help_without_endpoint() {
        let config = Config::from_args(["--help"].into_iter().map(str::to_string)).unwrap();
        assert!(config.show_help);
    }

    #[test]
    fn parses_revision_pinned_session_part_without_a_model() {
        let config = Config::from_args(
            ["session", "part", "c", "m", "p", "7"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        assert!(
            matches!(config.command, CommandMode::SessionPart { conversation_id, message_id, part_id, revision }
            if conversation_id=="c" && message_id=="m" && part_id=="p" && revision==7)
        );
        for revision in ["0", "-1", "unknown"] {
            assert!(Config::from_args(
                ["session", "part", "c", "m", "p", revision]
                    .into_iter()
                    .map(str::to_owned)
            )
            .is_err());
        }
    }

    #[test]
    fn parses_session_sources_with_latest_or_explicit_compaction() {
        let latest = Config::from_args(
            ["session", "sources", "conversation", "--json"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        assert!(latest.json_output);
        assert!(matches!(
            latest.command,
            CommandMode::SessionSources {
                conversation_id,
                compaction_part_id: None,
            } if conversation_id == "conversation"
        ));

        let explicit = Config::from_args(
            ["session", "sources", "conversation", "compaction-part"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        assert!(matches!(
            explicit.command,
            CommandMode::SessionSources {
                conversation_id,
                compaction_part_id: Some(part_id),
            } if conversation_id == "conversation" && part_id == "compaction-part"
        ));
    }

    #[test]
    fn parses_session_flags() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--session",
                "work",
                "--no-save",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.session_id.as_deref(), Some("work"));
        assert!(!config.save_session);
    }

    #[test]
    fn plugin_inspection_is_an_explicit_command_not_a_chat_prompt() {
        let config = Config::from_args(["plugin", "inspect", "./fixture"]
            .into_iter().map(str::to_string)).unwrap();
        assert!(matches!(config.command, CommandMode::Plugin(PluginCommandMode::Inspect(path)) if path == "./fixture"));
        for args in [vec!["plugin"], vec!["plugin", "inspect"], vec!["plugin", "unknown", "./fixture"]] {
            assert!(Config::from_args(args.into_iter().map(str::to_string)).is_err());
        }
    }

    #[test]
    fn plugin_management_and_inventory_cursor_are_explicit_commands() {
        for action in ["install", "enable", "disable", "remove"] {
            let config = Config::from_args(["plugin", action, "fixture"]
                .into_iter().map(str::to_string)).unwrap();
            assert!(matches!(config.command, CommandMode::Plugin(_)));
        }
        let config = Config::from_args(["plugin", "list", "--after", "example"]
            .into_iter().map(str::to_string)).unwrap();
        assert!(matches!(config.command,
            CommandMode::Plugin(PluginCommandMode::List { after: Some(id) }) if id == "example"));
        assert!(Config::from_args(["plugin", "list", "--after"]
            .into_iter().map(str::to_string)).is_err());
    }

    #[test]
    fn parses_session_management_commands_without_endpoint() {
        let list = Config::from_args(["sessions"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(list.command, CommandMode::Sessions));

        let show =
            Config::from_args(["session", "show", "abc"].into_iter().map(str::to_string)).unwrap();
        assert!(matches!(show.command, CommandMode::SessionShow(id) if id == "abc"));

        let delete =
            Config::from_args(["session", "delete", "abc"].into_iter().map(str::to_string))
                .unwrap();
        assert!(matches!(delete.command, CommandMode::SessionDelete(id) if id == "abc"));
    }

    #[test]
    fn parses_session_management_commands_after_global_options() {
        let show = Config::from_args(
            [
                "--endpoint",
                "https://api.deepseek.com",
                "--model",
                "deepseek-v4-flash-vision-exp",
                "--json",
                "session",
                "show",
                "abc",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(show.json_output);
        assert!(matches!(show.command, CommandMode::SessionShow(id) if id == "abc"));
    }

    #[test]
    fn parses_web_command_without_endpoint() {
        let config = Config::from_args(
            [
                "web",
                "--host",
                "0.0.0.0",
                "--port",
                "5055",
                "--web-root",
                "../web",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(matches!(config.command, CommandMode::Web));
        assert_eq!(config.web_host, "0.0.0.0");
        assert_eq!(config.web_port, 5055);
        assert!(config.web_root.ends_with("web"));
    }

    #[test]
    fn parses_init_and_wiki_commands() {
        let init = Config::from_args(
            [
                "init",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--project",
                ".",
                "--output",
                "NOLA.md",
                "--dry-run",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert!(matches!(init.command, CommandMode::Init));
        assert!(init.dry_run);
        assert_eq!(
            init.output_path.as_deref(),
            Some(std::path::Path::new("NOLA.md"))
        );

        let wiki = Config::from_args(
            [
                "wiki",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--output",
                "wiki.md",
                "--force",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert!(matches!(wiki.command, CommandMode::Wiki));
        assert!(wiki.force);
        assert_eq!(
            wiki.output_path.as_deref(),
            Some(std::path::Path::new("wiki.md"))
        );
    }

    #[test]
    fn parses_project_root_independently_from_allowed_dirs() {
        let root = std::env::current_dir().unwrap().canonicalize().unwrap();
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--project",
                ".",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.project_root, root);
        assert_eq!(config.allowed_dirs, vec![root]);
    }

    #[test]
    fn explicit_allowed_dir_does_not_change_project_root() {
        let root = std::env::current_dir().unwrap().canonicalize().unwrap();
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--project",
                ".",
                "--allow-dir",
                "..",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.project_root, root);
        assert_eq!(config.allowed_dirs.len(), 2);
        assert_eq!(config.allowed_dirs[0], config.project_root);
        assert_ne!(config.allowed_dirs[1], config.project_root);
    }

    #[test]
    fn loads_project_current_config_for_selected_project() {
        let root =
            std::env::temp_dir().join(format!("nola-cli-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".nola")).unwrap();
        let root = root.canonicalize().unwrap();
        std::fs::write(
            root.join(".nola/current_config.json"),
            r#"{
                "llm": {
                    "endpoint": "http://127.0.0.1:8080/v1",
                    "api_key": "project-secret",
                    "model": "project-model"
                },
                "debug": true,
                "allowed_dirs": ["src"],
                "max_iterations": 7
            }"#,
        )
        .unwrap();

        let config = Config::from_args(
            [
                "run",
                "--project",
                root.to_str().unwrap(),
                "hello from project config",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.project_root, root);
        assert_eq!(
            config.runtime.endpoint.as_deref(),
            Some("http://127.0.0.1:8080/v1")
        );
        assert_eq!(config.runtime.model.as_deref(), Some("project-model"));
        assert_eq!(config.endpoint, "http://127.0.0.1:8080/v1");
        assert_eq!(config.api_key, "project-secret");
        assert_eq!(config.model, "project-model");
        assert!(config.debug_mode);
        assert_eq!(config.runtime.max_iterations, Some(7));
        assert_eq!(config.max_iterations, 7);
        assert_eq!(config.runtime.allowed_dirs, vec![root.join("src")]);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn apply_config_and_help_work_when_an_ambient_catalog_is_broken() {
        let project = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("nola.json"), r#"{"llm":{"connections":[],"default_model":{}}}"#).unwrap();
        let parse = |words: &[&str]| {
            let mut args = vec!["--project".to_owned(), project.path().to_string_lossy().into_owned()];
            args.extend(words.iter().map(|word| word.to_string()));
            super::Config::from_entry_args(args, &crate::configuration_setup::NoConfigurationSetup)
        };
        assert!(matches!(parse(&["model","apply-config"]).unwrap().command, super::CommandMode::ModelApplyConfig { .. }));
        let startup = parse(&["hello"]).unwrap();
        assert!(startup.runtime.startup_catalog_error.is_some());
        assert!(startup.runtime.model_binding.is_none());
        assert!(matches!(parse(&["model","apply-config","one","extra"]), Err(super::ConfigStartError::Usage(_))));
        let args = ["--lang","zh","model","apply-config","--help"].map(str::to_owned);
        assert_eq!(super::help_topic(&args), Some("model apply-config"));
    }

    #[test]
    fn explicit_iteration_budget_overrides_runtime_config_and_accepts_unbounded_zero() {
        let bounded = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--max-iterations",
                "9",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(bounded.max_iterations, 9);

        let unbounded = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--max-iterations=0",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert_eq!(unbounded.max_iterations, 0);
    }

    #[test]
    fn iteration_budget_is_a_global_option_before_the_implicit_run_command() {
        let config = Config::from_args(
            [
                "--project",
                ".",
                "--json",
                "--max-iterations",
                "9",
                "--session",
                "bounded-run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(matches!(config.command, CommandMode::Run));
        assert_eq!(config.max_iterations, 9);
        assert_eq!(config.session_id.as_deref(), Some("bounded-run"));
    }

    #[test]
    fn parses_agent_mode_flags() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--swe-mode",
                "fix issue",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(config.swe_mode);

        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--agent-mode=swe",
                "fix issue",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(config.swe_mode);

        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--agent-mode",
                "standard",
                "fix issue",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(!config.swe_mode);

        let invalid = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--agent-mode",
                "experimental",
                "fix issue",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap_err();

        assert!(invalid.contains(tr(Message::UnknownAgentModePrefix)));
    }

    #[test]
    fn parses_network_policy() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--network-policy",
                "ask",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.network_policy, "ask");

        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--network-policy",
                "deny",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.network_policy, "deny");

        let invalid = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--network-policy",
                "open-all",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap_err();

        assert!(invalid.contains(tr(Message::NetworkPolicyMustBeOneOf)));
    }

    #[test]
    fn parses_force_tool_set() {
        let default_config = Config::from_args(
            ["run", "--endpoint", "http://127.0.0.1:11434/v1", "hello"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(default_config.force_tool_set, "auto");

        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--force-tool-set",
                "write",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.force_tool_set, "write");

        let invalid = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--force-tool-set",
                "read",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap_err();

        assert!(invalid.contains(tr(Message::ForceToolSetMustBeOneOf)));
    }

    #[test]
    fn parses_structured_expected_artifacts_without_inspecting_message_text() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--expected-artifact",
                "/workspace/alpha.out",
                "--expected-artifact=/workspace/beta.out",
                "produce the outputs",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(
            config.expected_artifact_refs,
            ["/workspace/alpha.out", "/workspace/beta.out"]
        );

        let ordinary_chat = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--",
                "please discuss --expected-artifact /workspace/alpha.out",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();
        assert!(ordinary_chat.expected_artifact_refs.is_empty());
        assert!(ordinary_chat.message.contains("--expected-artifact"));
    }

    #[test]
    fn rejects_conflicting_session_flags() {
        let err = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--session",
                "work",
                "--continue",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap_err();

        assert!(err.contains("--continue"));
        assert!(err.contains("--session"));
    }

    #[test]
    fn rejects_conflicting_resume_and_session_flags() {
        let err = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--resume",
                "--session",
                "work",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap_err();

        assert!(err.contains("--resume"));
        assert!(err.contains("--session"));
    }

    #[test]
    fn rejects_conflicting_resume_and_continue_flags() {
        let err = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--resume",
                "--continue",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap_err();

        assert!(err.contains("--resume"));
        assert!(err.contains("--continue"));
    }

    #[test]
    fn parses_continue_flag() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "--continue",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(config.continue_last);
        assert!(config.session_id.is_none());
        assert_eq!(config.message, "hello");
    }

    #[test]
    fn parses_resume_flag() {
        let config = Config::from_args(
            ["run", "--endpoint", "http://127.0.0.1:11434/v1", "--resume"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();

        assert!(config.resume_session);
        assert!(config.session_id.is_none());
        assert!(!config.continue_last);
    }

    #[test]
    fn parses_continue_short_flag() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "http://127.0.0.1:11434/v1",
                "-c",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert!(config.continue_last);
        assert!(config.session_id.is_none());
    }

    #[test]
    fn preserves_explicit_deepseek_model_name() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "https://api.deepseek.com/chat/completions",
                "--model",
                "deepseek-v4-pro",
                "--api-key",
                "test",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.model, "deepseek-v4-pro");
    }

    #[test]
    fn keeps_deepseek_chat_model() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "https://api.deepseek.com/chat/completions",
                "--model",
                "deepseek-chat",
                "--api-key",
                "test",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.model, "deepseek-chat");
    }

    #[test]
    fn keeps_non_deepseek_model_on_non_deepseek_endpoint() {
        let config = Config::from_args(
            [
                "run",
                "--endpoint",
                "https://api.openrouter.ai/v1",
                "--model",
                "deepseek-v4-flash",
                "--api-key",
                "test",
                "hello",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(config.model, "deepseek-v4-flash");
    }

    #[test]
    fn desktop_host_receipt_limits_execution_authorization() {
        let input: nola_core::tool_policy::ToolPolicyInput = serde_json::from_value(serde_json::json!({
            "schema_version":1,"platform":"macos","now_ms":1000,
            "expected_identity":{"host_id":"local","client_id":"mac","session_id":"conversation-1"},
            "runtime":{"manifest_id":"test","manifest_version":1,"issued_at_ms":1,"expires_at_ms":60000,
                "host_id":"local","client_id":"mac","session_id":"conversation-1","routes":["agent_host"],
                "capabilities":["files_read"],"authorizations":{"files_read":"granted"},
                "tools":[{"tool_id":"read_file","handler_id":"read_file","route":"agent_host","available":true,
                    "schema":{"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}},
                    {"tool_id":"write_file","handler_id":"write_file","route":"agent_host","available":false,
                    "schema":{"type":"function","function":{"name":"write_file","parameters":{"type":"object"}}}}]},
            "user_policy":{"max_risk":"high"}
        })).unwrap();
        let resolved = nola_core::tool_policy::resolve_tool_policy(&input);
        let mut receipt = serde_json::to_value(&resolved).unwrap();
        receipt["expected_identity"] = serde_json::to_value(&input.expected_identity).unwrap();
        receipt["policy_input"] = serde_json::to_value(input).unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&receipt).unwrap());
        let tools = super::decode_desktop_host_tool_allowlist(&encoded, &resolved.policy_digest,
            &resolved.manifest_digest, "conversation-1", 10_000).unwrap();

        assert_eq!(tools, vec!["read_file"]);
    }

    #[test]
    fn desktop_host_receipt_rejects_wrong_identity_and_expiry() {
        let receipt = serde_json::json!({
            "policy_digest": "policy-1",
            "manifest_digest": "manifest-1",
            "manifest_valid_until_ms": 20_000,
            "expected_identity": {"client_id": "mac", "session_id": "conversation-1"},
            "enabled_tools": [
                {"tool_id": "read_file", "enabled": true, "route": "agent_host"}
            ]
        });
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&receipt).unwrap());

        let wrong_identity = super::decode_desktop_host_tool_allowlist(
            &encoded,
            "policy-1",
            "manifest-1",
            "conversation-2",
            10_000,
        )
        .unwrap_err();
        assert!(wrong_identity.contains("identity"));

        let expired = super::decode_desktop_host_tool_allowlist(
            &encoded,
            "policy-1",
            "manifest-1",
            "conversation-1",
            20_000,
        )
        .unwrap_err();
        assert!(expired.contains("expired"));
    }

    #[test]
    fn yes_and_permission_profile_precedence_and_short_model_alias() {
        let parse = |args: &[&str]| Config::from_args(args.iter().map(|s| s.to_string()));

        // -y sets auto_approve and AllowAll by default
        let y_only = parse(&["--endpoint", "http://127.0.0.1:11434/v1", "-y", "hi"]).unwrap();
        assert!(y_only.auto_approve);
        assert_eq!(
            y_only.permission_profile,
            Some(crate::permissions::PermissionProfile::AllowAll)
        );

        // Explicit --permission-profile wins regardless of order relative to -y/--yes
        let explicit_before = parse(&[
            "--endpoint",
            "http://127.0.0.1:11434/v1",
            "--permission-profile",
            "cautious",
            "-y",
            "hi",
        ])
        .unwrap();
        assert!(explicit_before.auto_approve);
        assert_eq!(
            explicit_before.permission_profile,
            Some(crate::permissions::PermissionProfile::Cautious)
        );

        let explicit_after = parse(&[
            "--endpoint",
            "http://127.0.0.1:11434/v1",
            "--yes",
            "--permission-profile=cautious",
            "hi",
        ])
        .unwrap();
        assert!(explicit_after.auto_approve);
        assert_eq!(
            explicit_after.permission_profile,
            Some(crate::permissions::PermissionProfile::Cautious)
        );

        // -m works before and after message
        let m_before =
            parse(&["--endpoint", "http://127.0.0.1:11434/v1", "-m", "gpt-4o", "hi"]).unwrap();
        assert_eq!(m_before.model, "gpt-4o");
        assert_eq!(m_before.message, "hi");

        let m_after =
            parse(&["--endpoint", "http://127.0.0.1:11434/v1", "hi", "-m", "gpt-4o"]).unwrap();
        assert_eq!(m_after.model, "gpt-4o");
        assert_eq!(m_after.message, "hi");
    }

    #[test]
    fn subcommand_help_short_circuits_before_subcommand_dispatch_and_respects_double_dash() {
        let parse = |args: &[&str]| Config::from_args(args.iter().map(|s| s.to_string()));
        for args in [
            vec!["room", "create", "--help"],
            vec!["session", "delete", "--help"],
            vec!["model", "--help"],
            vec!["model", "discover", "-h"],
            vec!["plugin", "--help"],
            vec!["web", "--help"],
            vec!["--lang", "zh", "room", "create", "--help"],
        ] {
            let parsed = parse(&args).unwrap();
            assert!(parsed.show_help, "{args:?} must trigger show_help");
            assert!(matches!(parsed.command, CommandMode::Run));
        }

        // After --, "--help" is message text, not help flag!
        let after_sep =
            parse(&["--endpoint", "http://127.0.0.1:11434/v1", "--", "--help"]).unwrap();
        assert!(!after_sep.show_help);
        assert!(matches!(after_sep.command, CommandMode::Run));
        assert_eq!(after_sep.message, "--help");

        // Double dash before flag-like message does not error with unknown command "--"
        let run_sep = parse(&[
            "--endpoint",
            "http://127.0.0.1:11434/v1",
            "--",
            "--list-execution-backends",
            "init",
        ])
        .unwrap();
        assert!(!run_sep.show_help);
        assert!(matches!(run_sep.command, CommandMode::Run));
        assert_eq!(run_sep.message, "--list-execution-backends init");
    }

    #[test]
    fn subcommand_equals_flags_relative_paths_and_optional_revisions_work() {
        let _guard = crate::store::test_env_lock();
        struct RestoreDb(Option<std::ffi::OsString>);
        impl Drop for RestoreDb {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => std::env::set_var("NOLA_DB_PATH", v),
                    None => std::env::remove_var("NOLA_DB_PATH"),
                }
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("config-test.sqlite");
        let _restore = RestoreDb(std::env::var_os("NOLA_DB_PATH"));
        std::env::set_var("NOLA_DB_PATH", &db_path);

        let parse = |args: &[&str]| Config::from_args(args.iter().map(|s| s.to_string()));

        // --flag=val on room, plugin, and model import
        let room_send = parse(&["room", "send", "room-1", "hi", "--to=agent-1"]).unwrap();
        assert!(matches!(
            room_send.command,
            CommandMode::Room {
                agent_id: Some(ref id),
                text: Some(ref text),
                ..
            } if id == "agent-1" && text == "hi"
        ));

        let plugin_list = parse(&["plugin", "list", "--after=plug-1"]).unwrap();
        assert!(matches!(
            plugin_list.command,
            CommandMode::Plugin(PluginCommandMode::List { after: Some(ref a) }) if a == "plug-1"
        ));

        let imported = parse(&[
            "model",
            "import",
            "codex",
            "relative/config.toml",
            "--include-credentials",
            "--credential-path=relative/auth.json",
        ])
        .unwrap();
        match imported.command {
            CommandMode::ModelImport {
                path,
                credential_path,
                ..
            } => {
                assert!(std::path::Path::new(&path).is_absolute());
                assert!(path.ends_with("relative/config.toml"));
                let cred = credential_path.unwrap();
                assert!(std::path::Path::new(&cred).is_absolute());
                assert!(cred.ends_with("relative/auth.json"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let applied = parse(&["model", "apply-config", "relative/models.json"]).unwrap();
        match applied.command {
            CommandMode::ModelApplyConfig { path } => {
                let path = path.unwrap();
                assert!(std::path::Path::new(&path).is_absolute());
                assert!(path.ends_with("relative/models.json"));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        // Comma-separated keys in model configure
        let csv_cfg = parse(&[
            "model",
            "configure",
            "conn-1",
            "2",
            "chat_completions:gpt-4o",
            "chat_completions:gpt-4o,responses:gpt-4.1",
        ])
        .unwrap();
        match csv_cfg.command {
            CommandMode::ModelConfigure {
                enabled_model_ids,
                selected_model_id,
                ..
            } => {
                assert_eq!(selected_model_id.as_deref(), Some("chat_completions:gpt-4o"));
                assert_eq!(
                    enabled_model_ids,
                    vec!["chat_completions:gpt-4o", "responses:gpt-4.1"]
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        // Seed store with profile, connection, and chatgpt account to verify optional <revision>
        let store = crate::store::NolaStore::open(&db_path).unwrap();
        store
            .upsert_model_profile(nola_application::ModelProfile {
                id: "seeded-profile".into(),
                display_name: "Seeded".into(),
                model_name: "gpt-4o".into(),
                provider: "openai-compatible".into(),
                runtime: "remote".into(),
                source_json: serde_json::json!({
                    "baseURL": "https://fixture.invalid/v1",
                    "endpointID": "ep-seeded",
                    "wireProtocol": "chat_completions",
                    "capabilities": ["chat"],
                    "executionCapabilities": {
                        "toolCalls": true,
                        "imageInput": false,
                        "reasoning": false,
                        "samplingParameters": true
                    }
                })
                .to_string(),
                parameters_json: serde_json::json!({
                    "temperature": 0.5,
                    "topP": 1.0,
                    "maxOutputTokens": 4096,
                    "maxContextLength": 32000
                })
                .to_string(),
                selected: true,
                enabled: true,
                revision: 4,
                created_at_ms: 1,
                updated_at_ms: 1,
                deleted_at_ms: None,
            })
            .unwrap();

        let expected_profile_rev = store
            .model_profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == "seeded-profile")
            .unwrap()
            .revision;
        let profile_no_rev = parse(&["run", "--profile", "seeded-profile", "hello"]).unwrap();
        assert_eq!(
            profile_no_rev.cli_profile_selection,
            Some(("seeded-profile".into(), expected_profile_rev))
        );

        let mut app = store.application().unwrap();
        let conn_input: nola_domain::model_connection::ModelConnection =
            serde_json::from_value(serde_json::json!({
                "id": "conn-seeded",
                "display_name": "Seeded Conn",
                "base_url": "https://fixture.invalid/v1",
                "protocols": ["chat_completions"],
                "source": "manual",
                "enabled": true
            }))
            .unwrap();
        let conn_snap: nola_domain::model_connection::ModelConnectionSnapshot = app
            .execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::SaveConnection {
                    input: conn_input,
                    expected_revision: None,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
        let discover_no_rev = parse(&["model", "discover", "conn-seeded"]).unwrap();
        assert!(matches!(
            discover_no_rev.command,
            CommandMode::ModelDiscover { revision, .. } if revision == conn_snap.revision
        ));
        let probe_no_rev = parse(&[
            "model",
            "probe",
            "conn-seeded",
            "gpt-4o",
            "chat_completions",
        ])
        .unwrap();
        assert!(matches!(
            probe_no_rev.command,
            CommandMode::ModelProbe { revision, ref protocol, .. }
                if revision == conn_snap.revision && protocol == "chat_completions"
        ));

        let acct_id = "22222222-2222-4222-8222-222222222222";
        let acct_input: nola_domain::model_connection::ModelAccount =
            serde_json::from_value(serde_json::json!({
                "id": acct_id,
                "display_name": "Seeded Account",
                "source": "nola_chatgpt",
                "credential_ref": format!("siwc:{acct_id}"),
                "state": "ready"
            }))
            .unwrap();
        let acct_snap: nola_domain::model_connection::ModelAccountSnapshot = app
            .execute(nola_application::ApplicationCommand::Model {
                command: nola_application::ModelCommand::SaveAccount {
                    input: acct_input,
                    expected_revision: None,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
        let sign_out_no_rev = parse(&["model", "chatgpt", "sign-out", acct_id]).unwrap();
        assert!(matches!(
            sign_out_no_rev.command,
            CommandMode::ModelChatgpt { revision: Some(r), .. } if r == acct_snap.revision
        ));
    }
}
