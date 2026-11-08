use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use nola_core::agent::permissions::PermissionRule as CorePermissionRule;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config_paths::{expand_home, project_ancestor_directories};

pub(crate) fn reasoning_enabled(
    selection: Option<&nola_domain::generation::ReasoningSelection>,
    declared: Option<bool>, provider: Option<&str>, model: &str,
) -> bool {
    selection.map(|selection| selection.enables_thinking()).unwrap_or_else(||
        declared.unwrap_or_else(|| nola_core::agent::resolve_model_capability_profile(provider, model).reasoning))
}

impl RuntimeConfig {
    pub(crate) fn host_runtime_options(&self) -> crate::host_invocation_request::HostRuntimeOptions {
        crate::host_invocation_request::HostRuntimeOptions {
                edited_resend_model_capability: None,
                edited_resend_prompt_definitions: Vec::new(),
                expert_template_authority: None,
                selected_profile: self.selected_profile.clone(),
                siwc: self.siwc.clone(),
                code_mode_limits: self.code_mode_limits.clone(),
                language_services: self.language_services.clone(),
                native_model_route: self.native_model_route.clone(),
                mac_launch: self.mac_launch.clone(),
                memory_enabled: self.memory_enabled.clone(),
                admitted_memory_policy: self.admitted_memory_policy.clone(),
                provider: self.provider.clone(),
                llm_fallbacks: self.llm_fallbacks.clone(),
                temperature: self.temperature.clone(),
                top_p: self.top_p.clone(),
                max_output_tokens: self.max_output_tokens.clone(),
                output_admission: self.output_admission.clone(),
                admitted_max_retries: self.admitted_max_retries.clone(),
                chat_output_token_field: self.chat_output_token_field.clone(),
                reasoning_selection: self.reasoning_selection.clone(),
                reasoning_tier: self.reasoning_tier.clone(),
                sampling_override: self.sampling_override.clone(),
                generation_options: self.generation_options.clone(),
                language: self.language.clone(),
                openai_prompt_cache: self.openai_prompt_cache.clone(),
                openai_prompt_cache_key: self.openai_prompt_cache_key.clone(),
                openai_prompt_cache_key_prefix: self.openai_prompt_cache_key_prefix.clone(),
                openai_prompt_cache_retention: self.openai_prompt_cache_retention.clone(),
                openai_prompt_cache_history_breakpoint: self.openai_prompt_cache_history_breakpoint,
                openai_prompt_cache_history_breakpoint_endpoints: self.openai_prompt_cache_history_breakpoint_endpoints.clone(),
                model_capabilities: self.model_capabilities.clone(),
                instructions: self.instructions.clone(),
                agents: self.agents.clone(),
                permissions: self.permissions.clone(),
                core_permission_rules: self.core_permission_rules.clone(),
                compaction: self.compaction.clone(),
                external_execution: self.external_execution.clone(),

        }
    }
}

impl RuntimeConfig {
    /// A per-request restriction, never a replacement for Host authority.
    pub fn requested_memory_enabled(request: &Value) -> Result<Option<bool>, String> {
        request.get("memory_enabled").map(|value| {
            value.as_bool().ok_or_else(|| "memory_enabled must be boolean when present".to_string())
        }).transpose()
    }
}

crate::model_request_overrides::impl_model_request_overrides!(RuntimeConfig);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeConfig {
    #[serde(rename="nativeShellSandbox",default, skip_serializing_if="Option::is_none")]
    pub process_sandbox: Option<crate::process_sandbox::FrozenProcessSandbox>,
    /// Trusted selection evidence produced by the profile resolver, never by
    /// configuration JSON or a model request.
    #[serde(skip)]
    pub selected_profile: Option<nola_host_protocol::HostModelProfileReference>,
    /// Trusted Core + private SIWC owner identity; never accepted from request JSON.
    #[serde(skip)]
    pub siwc: Option<crate::model_siwc::SiwcExecutionContext>,
    /// Host-owned ceiling. Model requests may only lower these limits.
    pub code_mode_limits: nola_domain::code_mode::CodeCellLimits,
    /// Explicit host configuration, never inferred from model tool arguments.
    pub language_services: Vec<nola_domain::language_service::LanguageServiceConfiguration>,
    /// Logical native identity, supplied only by the trusted desktop launch pipe.
    #[serde(skip)]
    pub native_model_route: Option<nola_host_protocol::HostNativeModelRouteBinding>,
    #[serde(skip)]
    pub mac_launch: Option<nola_host_protocol::HostMacLaunchDescriptor>,
    /// Client intent only. Host policy is independently resolved at admission.
    #[serde(default)]
    pub memory_enabled: Option<bool>,
    /// Exact Host-admitted policy. Never supplied by configuration JSON.
    #[serde(skip)]
    pub admitted_memory_policy: Option<nola_core::memory::MemoryPolicy>,
    pub sources: Vec<PathBuf>,
    pub endpoint: Option<String>,
    /// Startup selection references the Core catalog; never carries credentials.
    #[serde(skip)]
    pub model_binding: Option<crate::model_connection_host::file_config::ModelRef>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    /// Ordered, explicit provider alternatives used only when the active
    /// provider fails before publishing any model output. These are runtime
    /// configuration, never environment-derived routing hints.
    pub llm_fallbacks: Vec<RuntimeLlmProvider>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u64>,
    /// Immutable per-Run admission evidence. Never populated by configuration-file merging.
    #[serde(default, skip_deserializing)]
    pub output_admission: Option<nola_host_protocol::HostOutputAdmission>,
    /// Run admission owns this value; configuration-file merges cannot set it.
    #[serde(skip)]
    pub admitted_max_retries: Option<u32>,
    #[serde(default)]
    pub chat_output_token_field: nola_application::ChatOutputTokenField,
    #[serde(default)]
    pub reasoning_selection: Option<nola_core::agent::reasoning::ReasoningSelection>,
    /// User-selected complexity, independent of the model's wire reasoning control.
    #[serde(default)]
    pub reasoning_tier: nola_domain::reasoning_tiers::ReasoningTier,
    #[serde(default)]
    pub sampling_override: bool,
    #[serde(default)]
    pub generation_options: nola_core::agent::reasoning::GenerationOptions,
    pub language: Option<String>,
    pub openai_prompt_cache: Option<bool>,
    pub openai_prompt_cache_key: Option<String>,
    pub openai_prompt_cache_key_prefix: Option<String>,
    pub openai_prompt_cache_retention: Option<String>,
    /// Explicit endpoint support for Responses history cache breakpoints; off by default.
    pub openai_prompt_cache_history_breakpoint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_prompt_cache_history_breakpoint_endpoints: Option<Vec<String>>,
    pub model_capabilities: RuntimeModelCapabilities,
    pub allowed_dirs: Vec<PathBuf>,
    pub bash_prefixes: Vec<String>,
    pub max_iterations: Option<usize>,
    pub force_tool_set: Option<String>,
    pub network_policy: Option<String>,
    pub debug_mode: Option<bool>,
    pub instructions: Vec<String>,
    pub agents: BTreeMap<String, RuntimeAgent>,
    pub permissions: Vec<PermissionRule>,
    /// Product-Host rules are already normalized by Application/Core and must
    /// never pass through the CLI permission-pattern parser.
    pub core_permission_rules: Vec<CorePermissionRule>,
    pub compaction: RuntimeCompaction,
    #[serde(default)]
    pub external_execution: RuntimeExternalExecution,
}

#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuntimeLlmProvider {
    pub provider: Option<String>,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

impl std::fmt::Debug for RuntimeLlmProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Endpoints can embed credentials too. Debug must remain safe when
        // nested inside an invocation or fallback diagnostic structure.
        formatter.write_str("RuntimeLlmProvider { route_and_credentials: [redacted] }")
    }
}

pub use nola_domain::execution_policy::{
    ConversationAgentPolicy as RuntimeAgent,
    ConversationCompactionPolicy as RuntimeCompaction,
    ConversationExternalExecutionBudget as RuntimeExternalExecution,
    ConversationPermissionDeclaration as PermissionRule,
    ConversationPermissionAction as PermissionAction,
};

pub use nola_domain::model_runtime::RuntimeModelCapabilities;
pub(crate) trait RuntimeModelCapabilitiesConfig {
    fn merge(&mut self, obj: &Map<String, Value>) -> Result<(), String>;
}
impl RuntimeModelCapabilitiesConfig for RuntimeModelCapabilities {
    fn merge(&mut self, obj: &Map<String, Value>) -> Result<(), String> {
        if let Some(value) = obj.get("reasoning_declaration") {
            self.reasoning_declaration = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid reasoning_declaration: {error}"))?;
        }
        self.instruction_profile = string_from_map(obj, "instruction_profile")
            .or_else(|| string_from_map(obj, "instructionProfile"))
            .or(self.instruction_profile.take());
        self.native_tool_calls = bool_from_map(obj, "native_tool_calls")
            .or_else(|| bool_from_map(obj, "nativeToolCalls"))
            .or(self.native_tool_calls);
        self.parallel_tool_calls = bool_from_map(obj, "parallel_tool_calls")
            .or_else(|| bool_from_map(obj, "parallelToolCalls"))
            .or(self.parallel_tool_calls);
        self.reasoning = bool_from_map(obj, "reasoning").or(self.reasoning);
        if let Some(value) = obj.get("reasoning_summary_supported") {
            self.reasoning_summary_supported = serde_json::from_value(value.clone())
                .map_err(|_| "invalid reasoning summary capability")?;
        }
        self.reasoning_continuity = bool_from_map(obj, "reasoning_continuity")
            .or_else(|| bool_from_map(obj, "reasoningContinuity"))
            .or(self.reasoning_continuity);
        self.vision_input = bool_from_map(obj, "vision_input")
            .or_else(|| bool_from_map(obj, "visionInput"))
            .or(self.vision_input);
        self.native_pdf_input = bool_from_map(obj, "native_pdf_input")
            .or_else(|| bool_from_map(obj, "nativePdfInput"))
            .or(self.native_pdf_input);
        self.request_continuation = bool_from_map(obj, "request_continuation")
            .or_else(|| bool_from_map(obj, "requestContinuation"))
            .or(self.request_continuation);
        self.max_request_continuations = usize_from_map(obj, "max_request_continuations")
            .or_else(|| usize_from_map(obj, "maxRequestContinuations"))
            .and_then(|value| u8::try_from(value).ok())
            .or(self.max_request_continuations);
        self.stream_text = bool_from_map(obj, "stream_text")
            .or_else(|| bool_from_map(obj, "streamText"))
            .or(self.stream_text);
        self.stream_reasoning = bool_from_map(obj, "stream_reasoning")
            .or_else(|| bool_from_map(obj, "streamReasoning"))
            .or(self.stream_reasoning);
        self.stream_tool_calls = bool_from_map(obj, "stream_tool_calls")
            .or_else(|| bool_from_map(obj, "streamToolCalls"))
            .or(self.stream_tool_calls);
        Ok(())
    }
}

impl RuntimeConfig {
    pub fn load(project_root: &Path) -> Result<Self, String> {
        let home = crate::platform_paths::home_dir();
        Self::load_with_home(project_root, home.as_deref())
    }

    fn load_with_home(project_root: &Path, home: Option<&Path>) -> Result<Self, String> {
        let mut result = RuntimeConfig::default();
        // Only declarations read from user configuration belong in this list.
        // Product defaults are evaluated by the Kernel invocation profile and
        // trusted tool/resource contracts, never fabricated as user grants.
        for path in config_candidates(project_root, home) {
            let Some(value) = read_jsonc_file(&path)? else {
                continue;
            };
            result.sources.push(path.clone());
            result.merge_value(value, &path)?;
        }
        Ok(result)
    }

    fn merge_value(&mut self, value: Value, source: &Path) -> Result<(), String> {
        let Some(obj) = value.as_object() else {
            return Err(format!("{} must contain a JSON object", source.display()));
        };

        if let Some(value) = obj.get("code_mode_limits") {
            let limits: nola_domain::code_mode::CodeCellLimits = serde_json::from_value(value.clone())
                .map_err(|error| format!("{}: invalid code_mode_limits: {error}", source.display()))?;
            limits.validate().map_err(|error| format!("{}: {error}", source.display()))?;
            // Replace one validated configuration, never blend budgets from
            // different scopes or accept arbitrary model-provided ceilings.
            self.code_mode_limits = limits;
        }

        if let Some(value) = obj.get("language_services") {
            let services: Vec<nola_domain::language_service::LanguageServiceConfiguration> =
                serde_json::from_value(value.clone())
                    .map_err(|error| format!("{}: invalid language_services: {error}", source.display()))?;
            nola_domain::language_service::validate_language_services(&services)
                .map_err(|error| format!("{}: {error}", source.display()))?;
            // A more specific config replaces the entire service set. Do not
            // merge executable, argv and environment from different scopes.
            self.language_services = services;
        }

        let llm = obj.get("llm").and_then(Value::as_object);
        if let Some(value) = llm.filter(|value| value.contains_key("connections") || value.contains_key("default_model")) {
            let models = crate::model_connection_host::file_config::FileModels::parse(Value::Object(value.clone()))?;
            self.model_binding = Some(models.default_model);
            self.endpoint = None;
            self.api_key = None;
            self.model = None;
            self.provider = None;
            self.llm_fallbacks.clear();
        }
        if let Some(tier) = obj.get("reasoning_tier").or_else(|| llm.and_then(|value| value.get("reasoning_tier"))) {
            self.reasoning_tier = serde_json::from_value(tier.clone())
                .map_err(|error| format!("{}: invalid reasoning_tier: {error}", source.display()))?;
        }
        self.endpoint = string_from(obj, "endpoint")
            .or_else(|| llm.and_then(|m| string_from_map(m, "endpoint")))
            .or(self.endpoint.take());
        self.api_key = string_from(obj, "api_key")
            .or_else(|| string_from(obj, "apiKey"))
            .or_else(|| llm.and_then(|m| string_from_map(m, "api_key")))
            .or_else(|| llm.and_then(|m| string_from_map(m, "apiKey")))
            .or(self.api_key.take());
        self.model = string_from(obj, "model")
            .or_else(|| llm.and_then(|m| string_from_map(m, "model")))
            .or(self.model.take());
        self.provider = string_from(obj, "provider")
            .or_else(|| llm.and_then(|m| string_from_map(m, "provider")))
            .or(self.provider.take());
        if self.model_binding.is_some() && (self.endpoint.is_some() || self.api_key.is_some() || self.model.is_some() || self.provider.is_some()) {
            return Err("cannot mix connection catalog configuration with a single-route configuration layer".into());
        }
        if let Some(value) = llm
            .and_then(|m| m.get("fallbacks"))
            .or_else(|| obj.get("llm_fallbacks"))
            .or_else(|| obj.get("llmFallbacks"))
        {
            self.llm_fallbacks = parse_llm_fallbacks(value, source)?;
        }
        if let Some(capabilities) = llm
            .and_then(|m| m.get("capabilities"))
            .or_else(|| obj.get("model_capabilities"))
            .or_else(|| obj.get("modelCapabilities"))
            .and_then(Value::as_object)
        {
            self.model_capabilities.merge(capabilities)?;
        }
        self.language = string_from(obj, "language")
            .or_else(|| string_from(obj, "lang"))
            .or_else(|| llm.and_then(|m| string_from_map(m, "language")))
            .or_else(|| llm.and_then(|m| string_from_map(m, "lang")))
            .or(self.language.take());
        if let Some(openai_cache_value) = llm.and_then(|m| m.get("openai_cache")) {
            match openai_cache_value {
                Value::Bool(enabled) => {
                    self.openai_prompt_cache = Some(*enabled);
                }
                Value::String(raw) => {
                    self.openai_prompt_cache = parse_bool(raw).or(self.openai_prompt_cache.take());
                }
                Value::Object(openai_cache) => {
                    self.openai_prompt_cache =
                        bool_from_map(openai_cache, "enabled").or(self.openai_prompt_cache.take());
                    self.openai_prompt_cache_key = string_from(openai_cache, "key")
                        .or_else(|| string_from(openai_cache, "cache_key"))
                        .or(self.openai_prompt_cache_key.take());
                    self.openai_prompt_cache_key_prefix = string_from(openai_cache, "key_prefix")
                        .or_else(|| string_from(openai_cache, "keyPrefix"))
                        .or(self.openai_prompt_cache_key_prefix.take());
                    self.openai_prompt_cache_history_breakpoint = bool_from_map(openai_cache, "history_breakpoint").or(self.openai_prompt_cache_history_breakpoint);
                    if let Some(endpoints) = openai_cache.get("history_breakpoint_endpoints") {
                        if openai_cache.contains_key("history_breakpoint") {
                            return Err("choose history_breakpoint or history_breakpoint_endpoints, not both".into());
                        }
                        self.openai_prompt_cache_history_breakpoint_endpoints = Some(serde_json::from_value(endpoints.clone())
                            .map_err(|_| "history_breakpoint_endpoints must be a list of endpoint strings")?);
                        self.openai_prompt_cache_history_breakpoint = None;
                    } else if openai_cache.contains_key("history_breakpoint") {
                        self.openai_prompt_cache_history_breakpoint_endpoints = None;
                    }
                    self.openai_prompt_cache_retention = string_from(openai_cache, "retention")
                        .or_else(|| string_from(openai_cache, "prompt_cache_retention"))
                        .or(self.openai_prompt_cache_retention.take());
                }
                _ => {}
            }
        } else if let Some(openai_cache_value) = llm.and_then(|m| m.get("openaiCache")) {
            match openai_cache_value {
                Value::Bool(enabled) => {
                    self.openai_prompt_cache = Some(*enabled);
                }
                Value::String(raw) => {
                    self.openai_prompt_cache = parse_bool(raw).or(self.openai_prompt_cache.take());
                }
                Value::Object(openai_cache) => {
                    self.openai_prompt_cache =
                        bool_from_map(openai_cache, "enabled").or(self.openai_prompt_cache.take());
                    self.openai_prompt_cache_key = string_from(openai_cache, "key")
                        .or_else(|| string_from(openai_cache, "cache_key"))
                        .or(self.openai_prompt_cache_key.take());
                    self.openai_prompt_cache_key_prefix = string_from(openai_cache, "key_prefix")
                        .or_else(|| string_from(openai_cache, "keyPrefix"))
                        .or(self.openai_prompt_cache_key_prefix.take());
                    self.openai_prompt_cache_history_breakpoint = bool_from_map(openai_cache, "history_breakpoint").or(self.openai_prompt_cache_history_breakpoint);
                    if let Some(endpoints) = openai_cache.get("history_breakpoint_endpoints") {
                        if openai_cache.contains_key("history_breakpoint") {
                            return Err("choose history_breakpoint or history_breakpoint_endpoints, not both".into());
                        }
                        self.openai_prompt_cache_history_breakpoint_endpoints = Some(serde_json::from_value(endpoints.clone())
                            .map_err(|_| "history_breakpoint_endpoints must be a list of endpoint strings")?);
                        self.openai_prompt_cache_history_breakpoint = None;
                    } else if openai_cache.contains_key("history_breakpoint") {
                        self.openai_prompt_cache_history_breakpoint_endpoints = None;
                    }
                    self.openai_prompt_cache_retention = string_from(openai_cache, "retention")
                        .or_else(|| string_from(openai_cache, "prompt_cache_retention"))
                        .or(self.openai_prompt_cache_retention.take());
                }
                _ => {}
            }
        } else {
            self.openai_prompt_cache = llm
                .and_then(|m| bool_from_map(m, "openai_cache"))
                .or_else(|| llm.and_then(|m| bool_from_map(m, "openaiCache")))
                .or(self.openai_prompt_cache);
            self.openai_prompt_cache_key = llm
                .and_then(|m| string_from_map(m, "openai_prompt_cache_key"))
                .or_else(|| llm.and_then(|m| string_from_map(m, "openaiPromptCacheKey")))
                .or(self.openai_prompt_cache_key.take());
            self.openai_prompt_cache_key_prefix = llm
                .and_then(|m| string_from_map(m, "openai_prompt_cache_key_prefix"))
                .or_else(|| llm.and_then(|m| string_from_map(m, "openaiPromptCacheKeyPrefix")))
                .or(self.openai_prompt_cache_key_prefix.take());
            self.openai_prompt_cache_retention = llm
                .and_then(|m| string_from_map(m, "openai_prompt_cache_retention"))
                .or_else(|| llm.and_then(|m| string_from_map(m, "openaiPromptCacheRetention")))
                .or(self.openai_prompt_cache_retention.take());
        }
        if let Some(items) =
            string_array(obj.get("allowed_dirs").or_else(|| obj.get("allowedDirs")))
        {
            self.allowed_dirs = items
                .into_iter()
                .map(|item| expand_config_path(&item, source))
                .collect();
        }
        if let Some(items) =
            string_array(obj.get("bash_prefixes").or_else(|| obj.get("bashPrefixes")))
        {
            self.bash_prefixes = items;
        }
        self.max_iterations = usize_from(obj, "max_iterations")
            .or_else(|| usize_from(obj, "maxIterations"))
            .or(self.max_iterations);
        self.force_tool_set = string_from(obj, "force_tool_set")
            .or_else(|| string_from(obj, "forceToolSet"))
            .or(self.force_tool_set.take());
        self.network_policy = string_from(obj, "network_policy")
            .or_else(|| string_from(obj, "networkPolicy"))
            .or(self.network_policy.take());
        self.debug_mode = bool_from(obj, "debug")
            .or_else(|| bool_from(obj, "debug_mode"))
            .or(self.debug_mode.take());
        if let Some(items) = string_array(obj.get("instructions")) {
            self.instructions = items;
        }
        if let Some(compaction) = obj.get("compaction").and_then(Value::as_object) {
            merge_runtime_compaction(&mut self.compaction, compaction);
        }
        if let Some(value)=obj.get("nativeShellSandbox") {
            let trusted=crate::platform_paths::home_dir().ok_or("Cannot resolve trusted sandbox configuration")?.join(".nola/config.json");
            if source!=trusted {return Err("nativeShellSandbox is only accepted from trusted application configuration, not project files".into());}
            let spec:crate::process_sandbox::FrozenProcessSandbox=serde_json::from_value(value.clone()).map_err(|_|"Invalid nativeShellSandbox configuration")?;
            spec.validate_declaration()?;
            self.process_sandbox=Some(spec);
        }
        if let Some(external_execution) = obj
            .get("external_execution")
            .or_else(|| obj.get("externalExecution"))
            .and_then(Value::as_object)
        {
            merge_runtime_external_execution(&mut self.external_execution, external_execution);
        }
        // Layered user rules are appended in source order; the evaluator uses the
        // last matching rule, so project configuration overrides home configuration.
        self.permissions.extend(parse_permissions(
            obj.get("permissions").or_else(|| obj.get("permission")),
        )?);
        if let Some(agents) = obj
            .get("agents")
            .or_else(|| obj.get("agent"))
            .and_then(Value::as_object)
        {
            for (name, value) in agents {
                let agent = parse_agent(value)?;
                self.agents
                    .entry(name.to_string())
                    .and_modify(|existing| merge_runtime_agent(existing, agent.clone()))
                    .or_insert(agent);
            }
        }
        Ok(())
    }
}

fn parse_llm_fallbacks(value: &Value, source: &Path) -> Result<Vec<RuntimeLlmProvider>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| format!("{} llm.fallbacks must be an array", source.display()))?;
    items
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let object = value.as_object().ok_or_else(|| {
                format!(
                    "{} llm.fallbacks[{index}] must be an object",
                    source.display()
                )
            })?;
            let endpoint = string_from_map(object, "endpoint").ok_or_else(|| {
                format!(
                    "{} llm.fallbacks[{index}].endpoint is required",
                    source.display()
                )
            })?;
            if endpoint.trim().is_empty() {
                return Err(format!(
                    "{} llm.fallbacks[{index}].endpoint must not be empty",
                    source.display()
                ));
            }
            Ok(RuntimeLlmProvider {
                provider: string_from_map(object, "provider"),
                endpoint,
                api_key: string_from_map(object, "api_key")
                    .or_else(|| string_from_map(object, "apiKey")),
                model: string_from_map(object, "model"),
            })
        })
        .collect()
}

fn merge_runtime_agent(target: &mut RuntimeAgent, other: RuntimeAgent) {
    target.description = other.description.or(target.description.take());
    target.prompt = other.prompt.or(target.prompt.take());
    if other.model.is_some() { target.executor = None; }
    if other.executor.is_some() { target.model = None; }
    target.model = other.model.or(target.model.take());
    target.executor = other.executor.or(target.executor.take());
    if !other.tools.is_empty() {
        target.tools = other.tools;
    }
    target.permission.extend(other.permission);
    target.max_turns = other.max_turns.or(target.max_turns);
}

fn merge_runtime_compaction(target: &mut RuntimeCompaction, obj: &Map<String, Value>) {
    target.token_threshold = usize_from_map(obj, "token_threshold")
        .or(target.token_threshold);
    target.context_window_tokens = usize_from_map(obj, "context_window_tokens")
        .or(target.context_window_tokens);
    target.keep_recent_messages = usize_from_map(obj, "keep_recent_messages")
        .or(target.keep_recent_messages);
    target.compacted_tool_chars = usize_from_map(obj, "compacted_tool_chars")
        .or(target.compacted_tool_chars);
    target.compacted_assistant_chars = usize_from_map(obj, "compacted_assistant_chars")
        .or(target.compacted_assistant_chars);
}

fn merge_runtime_external_execution(target: &mut RuntimeExternalExecution, obj: &Map<String, Value>) {
    target.max_wall_time_seconds = positive_usize_from_map(obj, "max_wall_time_seconds")
        .or(target.max_wall_time_seconds);
    target.max_output_bytes = positive_usize_from_map(obj, "max_output_bytes")
        .or(target.max_output_bytes);
    target.max_cpu_time_seconds = positive_usize_from_map(obj, "max_cpu_time_seconds")
        .or(target.max_cpu_time_seconds);
    target.max_memory_bytes = positive_usize_from_map(obj, "max_memory_bytes")
        .or(target.max_memory_bytes);
    target.max_processes = positive_usize_from_map(obj, "max_processes")
        .or(target.max_processes);
    target.max_line_bytes = positive_usize_from_map(obj, "max_line_bytes")
        .or(target.max_line_bytes);
}

fn config_candidates(project_root: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home {
        out.push(home.join(".nola/config.jsonc"));
        out.push(home.join(".nola/config.json"));
    }
    for directory in project_ancestor_directories(project_root) {
        out.extend([
            directory.join(".nola/config.jsonc"),
            directory.join(".nola/config.json"),
            directory.join("nola.jsonc"),
            directory.join("nola.json"),
            directory.join(".nola/current_config.jsonc"),
            directory.join(".nola/current_config.json"),
        ]);
    }
    let mut seen = HashSet::new();
    out.retain(|path| seen.insert(path.clone()));
    out
}

fn read_jsonc_file(path: &Path) -> Result<Option<Value>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("read {} failed: {err}", path.display())),
    };
    let stripped = strip_json_comments(&text);
    serde_json::from_str(&stripped)
        .map(Some)
        .map_err(|err| format!("parse {} failed: {err}", path.display()))
}

fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = '\0';
            for next in chars.by_ref() {
                if prev == '*' && next == '/' {
                    break;
                }
                if next == '\n' {
                    out.push('\n');
                }
                prev = next;
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn parse_agent(value: &Value) -> Result<RuntimeAgent, String> {
    let Some(obj) = value.as_object() else {
        return Err("agent config must be an object".to_string());
    };
    Ok(RuntimeAgent {
        description: string_from_map(obj, "description"),
        prompt: string_from_map(obj, "prompt"),
        model: obj.get("model").map(|value| {
            let intent: nola_application::RunGenerationIntent = serde_json::from_value(value.clone())
                .map_err(|error| format!("agent model requires a bound profile intent: {error}"))?;
            intent.validate()?;
            Ok::<_, String>(intent)
        }).transpose()?,
        executor: obj.get("executor").map(|value| {
            let executor: nola_domain::delegation::ExternalAgentExecutor = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid agent executor: {error}"))?;
            executor.validate()?;
            if obj.get("model").is_some() { return Err("Agent model and executor are mutually exclusive".into()); }
            Ok::<_, String>(executor)
        }).transpose()?,
        tools: string_array(obj.get("tools")).unwrap_or_default(),
        permission: parse_permissions(obj.get("permission").or_else(|| obj.get("permissions")))?,
        max_turns: usize_from_map(obj, "max_turns").or_else(|| usize_from_map(obj, "maxTurns")),
    })
}

fn parse_permissions(value: Option<&Value>) -> Result<Vec<PermissionRule>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Some(items) = value.as_array() {
        return items.iter().map(parse_permission_rule).collect();
    }
    let Some(obj) = value.as_object() else {
        return Err("permissions must be an array or object".to_string());
    };
    let mut out = Vec::new();
    for (permission, value) in obj {
        if let Some(action) = value.as_str().and_then(parse_permission_action) {
            out.push(PermissionRule {
                permission: permission.to_string(),
                pattern: "*".to_string(),
                action,
            });
            continue;
        }
        let Some(patterns) = value.as_object() else {
            return Err(format!(
                "permission '{permission}' must be string or object"
            ));
        };
        for (pattern, raw_action) in patterns {
            let Some(action) = raw_action.as_str().and_then(parse_permission_action) else {
                return Err(format!(
                    "permission '{permission}' pattern '{pattern}' has invalid action"
                ));
            };
            out.push(PermissionRule {
                permission: permission.to_string(),
                pattern: pattern.to_string(),
                action,
            });
        }
    }
    Ok(out)
}

fn bool_from_map(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    obj.get(key).and_then(bool_from_value)
}

fn bool_from_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => parse_bool(value),
        _ => None,
    }
}

fn bool_from(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    bool_from_map(obj, key)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enable" => Some(true),
        "0" | "false" | "no" | "off" | "disable" => Some(false),
        _ => None,
    }
}

fn parse_permission_rule(value: &Value) -> Result<PermissionRule, String> {
    let Some(obj) = value.as_object() else {
        return Err("permission rule must be an object".to_string());
    };
    let permission = string_from_map(obj, "permission")
        .or_else(|| string_from_map(obj, "tool"))
        .ok_or_else(|| "permission rule missing permission".to_string())?;
    let pattern = string_from_map(obj, "pattern").unwrap_or_else(|| "*".to_string());
    let action = string_from_map(obj, "action")
        .and_then(|value| parse_permission_action(&value))
        .ok_or_else(|| "permission rule missing action".to_string())?;
    Ok(PermissionRule {
        permission,
        pattern,
        action,
    })
}

fn parse_permission_action(value: &str) -> Option<PermissionAction> {
    match value.trim().to_ascii_lowercase().as_str() {
        "allow" | "accept" | "always" => Some(PermissionAction::Allow),
        "ask" | "prompt" => Some(PermissionAction::Ask),
        "deny" | "reject" | "block" => Some(PermissionAction::Deny),
        _ => None,
    }
}

fn string_from(obj: &Map<String, Value>, key: &str) -> Option<String> {
    string_from_map(obj, key)
}

fn string_from_map(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn usize_from(obj: &Map<String, Value>, key: &str) -> Option<usize> {
    usize_from_map(obj, key)
}

fn usize_from_map(obj: &Map<String, Value>, key: &str) -> Option<usize> {
    obj.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn positive_usize_from_map(obj: &Map<String, Value>, key: &str) -> Option<usize> {
    usize_from_map(obj, key).filter(|value| *value > 0)
}

fn expand_config_path(raw: &str, source: &Path) -> PathBuf {
    let expanded = expand_home(Path::new(raw));
    if expanded.is_absolute() {
        expanded
    } else {
        config_relative_base(source).join(expanded)
    }
}

fn config_relative_base(source: &Path) -> PathBuf {
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    if parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".nola")
    {
        return parent.parent().unwrap_or(parent).to_path_buf();
    }
    parent.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::RuntimeModelCapabilitiesConfig;
    #[test]
    fn complete_reasoning_declaration_is_explicit_and_invalid_wire_is_rejected() {
        let mut capabilities = super::RuntimeModelCapabilities::default();
        capabilities.merge(serde_json::json!({"reasoning_declaration":{"control":"budget","tokens":[1024,4096,8192]}}).as_object().unwrap()).unwrap();
        assert!(matches!(capabilities.reasoning_declaration, Some(nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Budget { ref tokens }) if tokens == &vec![1024,4096,8192]));
        assert!(capabilities.merge(serde_json::json!({"reasoning_declaration":{"control":"budget","tokens":"invalid"}}).as_object().unwrap()).is_err());
    }
    #[test]
    fn native_pdf_capability_is_explicit_and_independent() {
        let mut capabilities = super::RuntimeModelCapabilities::default();
        assert!(capabilities.is_empty());
        capabilities.merge(serde_json::json!({"vision_input":true}).as_object().unwrap()).unwrap();
        assert_eq!(capabilities.native_pdf_input, None);
        capabilities.merge(serde_json::json!({"native_pdf_input":true}).as_object().unwrap()).unwrap();
        assert_eq!(capabilities.native_pdf_input, Some(true));
        capabilities.merge(serde_json::json!({"nativePdfInput":false}).as_object().unwrap()).unwrap();
        assert_eq!(capabilities.native_pdf_input, Some(false));
        let restored: super::RuntimeModelCapabilities = serde_json::from_str("{}").unwrap();
        assert_eq!(restored.native_pdf_input, None);
    }
    #[test]
    fn provider_debug_never_exposes_nested_fallback_credentials() {
        let provider = super::RuntimeLlmProvider {
            provider: Some("private-provider".into()),
            endpoint: "https://username:embedded-secret@example.invalid/v1".into(),
            api_key: Some("fallback-secret".into()),
            model: Some("private-model".into()),
        };
        let debug = format!("{provider:?}");
        assert!(debug.contains("[redacted]"));
        for private in ["embedded-secret", "fallback-secret", "private-provider", "private-model"] {
            assert!(!debug.contains(private));
        }
        let mut runtime = super::RuntimeConfig::default();
        runtime.llm_fallbacks.push(provider.clone());
        assert!(!format!("{runtime:?}").contains("fallback-secret"));
        // Private worker transport still needs credentials; redaction must not
        // silently break its existing explicit serialization contract.
        let roundtrip: super::RuntimeLlmProvider =
            serde_json::from_value(serde_json::to_value(&provider).unwrap()).unwrap();
        assert_eq!(roundtrip, provider);
    }

    #[test]
    fn memory_request_is_strict_and_cannot_supply_admitted_policy() {
        use serde_json::json;
        assert_eq!(super::RuntimeConfig::requested_memory_enabled(&json!({})).unwrap(), None);
        for enabled in [false, true] {
            assert_eq!(super::RuntimeConfig::requested_memory_enabled(&json!({"memory_enabled":enabled})).unwrap(), Some(enabled));
        }
        for invalid in [json!(null), json!(0), json!("false"), json!({"long_term_enabled":true})] {
            assert!(super::RuntimeConfig::requested_memory_enabled(&json!({"memory_enabled":invalid})).is_err());
        }
        let mut input = serde_json::to_value(super::RuntimeConfig::default()).unwrap();
        input["admitted_memory_policy"] = json!({"long_term_enabled":true});
        let runtime: super::RuntimeConfig = serde_json::from_value(input).unwrap();
        assert!(runtime.admitted_memory_policy.is_none());
    }
    #[test]
    fn model_request_reasoning_tier_is_explicit_strict_and_independent_of_effort() {
        use nola_domain::reasoning_tiers::ReasoningTier;
        let defaults = super::RuntimeConfig::default();
        assert_eq!(defaults.reasoning_tier, ReasoningTier::Balanced);
        let effort_only = defaults.with_model_request(&serde_json::json!({
            "reasoning_selection":{"kind":"effort","value":"high"}
        })).unwrap();
        assert_eq!(effort_only.reasoning_tier, ReasoningTier::Balanced);
        for (wire, expected) in [("concise", ReasoningTier::Concise), ("balanced", ReasoningTier::Balanced), ("detailed", ReasoningTier::Detailed)] {
            let selected = defaults.with_model_request(&serde_json::json!({"reasoning_tier":wire})).unwrap();
            assert_eq!(selected.reasoning_tier, expected);
            assert_eq!(selected.with_model_request(&serde_json::json!({})).unwrap().reasoning_tier, expected);
            assert!(selected.reasoning_selection.is_none());
        }
        for invalid in [serde_json::json!(null), serde_json::json!(1), serde_json::json!("high"), serde_json::json!("simple")] {
            assert!(defaults.with_model_request(&serde_json::json!({"reasoning_tier":invalid})).is_err());
        }
        let mut persisted = serde_json::to_value(&defaults).unwrap();
        persisted.as_object_mut().unwrap().remove("reasoning_tier");
        assert_eq!(serde_json::from_value::<super::RuntimeConfig>(persisted).unwrap().reasoning_tier, ReasoningTier::Balanced);
        let mut configured = super::RuntimeConfig::default();
        configured.merge_value(serde_json::json!({"reasoning_tier":"detailed"}), Path::new("fixture.json")).unwrap();
        assert_eq!(configured.reasoning_tier, ReasoningTier::Detailed);
        assert!(configured.merge_value(serde_json::json!({"reasoning_tier":"high"}), Path::new("fixture.json")).is_err());
    }

    #[test]
    fn model_request_snapshot_preserves_zero_and_inherits_missing_fields() {
        let mut defaults = super::RuntimeConfig::default();
        defaults.temperature = Some(1.2);
        defaults.top_p = Some(0.8);
        defaults.max_output_tokens = Some(4096);
        let snapshot = defaults.with_model_request(&serde_json::json!({
            "temperature": 0, "sampling_override": true,
            "model_protocol": "responses",
            "max_output_tokens": 64000,
            "reasoning_selection": {"kind": "effort", "value": "high"}
        })).unwrap();
        assert_eq!(snapshot.temperature, Some(0.0));
        assert_eq!(snapshot.top_p, Some(0.8));
        assert_eq!(snapshot.max_output_tokens, Some(64000));
        assert_eq!(snapshot.provider.as_deref(), Some("openai-responses"));
        assert!(snapshot.sampling_override);
        assert_eq!(defaults.temperature, Some(1.2));
        assert!(defaults.reasoning_selection.is_none());
    }

    #[test]
    fn model_request_snapshot_rejects_invalid_values_instead_of_clamping() {
        let defaults = super::RuntimeConfig::default();
        for request in [
            serde_json::json!({"temperature": -1}),
            serde_json::json!({"topP": 2}),
            serde_json::json!({"maxTokens": 0}),
            serde_json::json!({"contextWindowTokens": "128000"}),
            serde_json::json!({"sampling_override": "true"}),
            serde_json::json!({"model_protocol": "unknown"}),
            serde_json::json!({"generation_options": {"unknown": true}}),
            serde_json::json!({"generation_options": {"presence_penalty": 3}}),
            serde_json::json!({"max_output_tokens": 1024, "reasoning_selection": {"kind": "budget", "tokens": 2048}}),
        ] {
            assert!(defaults.with_model_request(&request).is_err(), "{request}");
        }
    }

    #[test]
    fn model_request_snapshot_can_clear_reasoning_and_switch_protocol() {
        let snapshot = super::RuntimeConfig::default().with_model_request(&serde_json::json!({
            "model_protocol": "responses",
            "reasoning_selection": {"kind": "effort", "value": "high"}
        })).unwrap();
        let cleared = snapshot.with_model_request(&serde_json::json!({
            "model_protocol": "chat_completions", "reasoning_selection": null,
            "generation_options": {"seed": 0, "stop": ["END"]}
        })).unwrap();
        assert!(cleared.reasoning_selection.is_none());
        assert_eq!(cleared.model_capabilities.reasoning_continuity, Some(false));
        assert_eq!(cleared.generation_options.seed, Some(0));
        assert_eq!(cleared.generation_options.stop, Some(vec!["END".into()]));
    }
    use super::*;

    #[test]
    fn history_cache_endpoint_scope_is_parsed_and_frozen_for_host() {
        let mut runtime = RuntimeConfig::default();
        runtime.merge_value(serde_json::json!({"llm":{"openai_cache":{
            "enabled":true,
            "history_breakpoint_endpoints":["https://api.openai.com/v1"]
        }}}), Path::new("/config.json")).unwrap();
        let frozen = runtime.host_runtime_options();
        let shared: nola_host_protocol::delegation::DelegatedRuntimeConfiguration =
            serde_json::from_value(serde_json::to_value(&frozen).unwrap()).unwrap();
        assert_eq!(shared.openai_prompt_cache_history_breakpoint_endpoints,
            Some(vec!["https://api.openai.com/v1".into()]));
        runtime.openai_prompt_cache_history_breakpoint_endpoints = Some(vec![]);
        assert_eq!(frozen.openai_prompt_cache_history_breakpoint_endpoints,
            Some(vec!["https://api.openai.com/v1".into()]));
    }

    #[test]
    fn code_mode_budget_configuration_is_validated_and_frozen_for_host() {
        let source = Path::new("/config.json");
        let mut runtime = RuntimeConfig::default();
        let mut limits = nola_domain::code_mode::CodeCellLimits::default();
        limits.max_calls = 12;
        limits.max_in_flight = 3;
        runtime.merge_value(serde_json::json!({"code_mode_limits":limits}), source).unwrap();
        let frozen = runtime.host_runtime_options();
        assert_eq!(frozen.code_mode_limits, limits);
        let mut invalid = limits.clone();
        invalid.max_in_flight = 13;
        assert!(runtime.merge_value(serde_json::json!({"code_mode_limits":invalid}), source).is_err());
        assert_eq!(runtime.code_mode_limits, limits);
        limits.max_calls = 20;
        runtime.merge_value(serde_json::json!({"code_mode_limits":limits}), source).unwrap();
        assert_eq!(frozen.code_mode_limits.max_calls, 12);
        assert_eq!(runtime.code_mode_limits.max_calls, 20);
    }

    #[test]
    fn language_service_configuration_replaces_atomically_and_freezes_for_host() {
        let source = Path::new("/config.json");
        let mut runtime = RuntimeConfig::default();
        let service = serde_json::json!({
            "id":"rust","executable":"/tools/rust-analyzer","arguments":[],
            "project_root":"/workspace","language_ids":["rust"],"environment":{},"timeout_ms":10000
        });
        runtime.merge_value(serde_json::json!({"language_services":[service]}), source).unwrap();
        let frozen = runtime.host_runtime_options();
        assert!(runtime.merge_value(serde_json::json!({"language_services":[{"id":"broken"}]}), source).is_err());
        assert_eq!(runtime.language_services, frozen.language_services);
        runtime.merge_value(serde_json::json!({"language_services":[]}), source).unwrap();
        assert!(runtime.language_services.is_empty());
        assert_eq!(frozen.language_services.len(), 1);
    }

    #[test]
    fn strips_jsonc_comments_without_touching_strings() {
        let input = r#"{"url":"http://x/y",/* hide */"n":1}// tail"#;
        let stripped = strip_json_comments(input);
        assert!(stripped.contains("http://x/y"));
        assert!(!stripped.contains("hide"));
    }

    #[test]
    fn parses_permission_object() {
        let value = serde_json::json!({
            "bash": {"cargo test*": "allow", "rm *": "deny"},
            "edit": "ask"
        });
        let rules = parse_permissions(Some(&value)).unwrap();
        assert!(rules.contains(&PermissionRule {
            permission: "bash".to_string(),
            pattern: "cargo test*".to_string(),
            action: PermissionAction::Allow,
        }));
        assert!(rules.contains(&PermissionRule {
            permission: "edit".to_string(),
            pattern: "*".to_string(),
            action: PermissionAction::Ask,
        }));
    }

    #[test]
    fn only_explicit_user_rules_survive_layered_configuration() {
        let mut config = RuntimeConfig::default();
        config
            .merge_value(
                serde_json::json!({"permissions": {"bash": "deny"}}),
                Path::new("/home/user/.nola/config.json"),
            )
            .unwrap();
        config
            .merge_value(
                serde_json::json!({"model": "project-model"}),
                Path::new("/workspace/.nola/config.json"),
            )
            .unwrap();

        let effective_bash_default = config
            .permissions
            .iter()
            .filter(|rule| rule.permission == "bash" && rule.pattern == "*")
            .last()
            .unwrap();
        assert_eq!(effective_bash_default.action, PermissionAction::Deny);
        assert_eq!(config.permissions.len(), 1);
        assert_eq!(config.permissions[0].permission, "bash");
    }

    #[test]
    fn empty_home_and_project_never_manufacture_user_permission_rules() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let home = root.path().join("isolated-home");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let config = RuntimeConfig::load_with_home(&project, Some(&home)).unwrap();
        assert!(config.permissions.is_empty(),
            "ordinary/sensitive/unknown classification belongs to trusted contracts, not fake user defaults");
        assert!(config.core_permission_rules.is_empty());
    }

    #[test]
    fn parses_explicit_ordered_llm_fallbacks_without_environment_routing() {
        let mut config = RuntimeConfig::default();
        config
            .merge_value(
                serde_json::json!({
                    "llm": {
                        "fallbacks": [
                            {
                                "provider": "anthropic",
                                "endpoint": "https://fallback-one.example/v1",
                                "apiKey": "fallback-one-key",
                                "model": "claude-fallback"
                            },
                            {
                                "endpoint": "http://127.0.0.1:11434/v1",
                                "model": "local-fallback"
                            }
                        ]
                    }
                }),
                Path::new("/workspace/.nola/config.json"),
            )
            .unwrap();

        assert_eq!(config.llm_fallbacks.len(), 2);
        assert_eq!(
            config.llm_fallbacks[0].provider.as_deref(),
            Some("anthropic")
        );
        assert_eq!(
            config.llm_fallbacks[0].api_key.as_deref(),
            Some("fallback-one-key")
        );
        assert_eq!(
            config.llm_fallbacks[1].endpoint,
            "http://127.0.0.1:11434/v1"
        );
    }

    #[test]
    fn native_shell_sandbox_is_host_only_and_serializes_its_canonical_key() {
        let _lock=crate::store::test_env_lock();let root=tempfile::tempdir().unwrap();
        struct RestoreHome(Option<std::ffi::OsString>);
        impl Drop for RestoreHome {fn drop(&mut self) {match &self.0 {Some(value)=>std::env::set_var("NOLA_HOME",value),None=>std::env::remove_var("NOLA_HOME")}}}
        let _restore=RestoreHome(std::env::var_os("NOLA_HOME"));std::env::set_var("NOLA_HOME",root.path());
        let spec=crate::process_sandbox::unavailable_process_sandbox_fixture(root.path());
        let value=serde_json::json!({"nativeShellSandbox":spec});
        let mut config=RuntimeConfig::default();
        config.merge_value(value.clone(),&root.path().join(".nola/config.json")).unwrap();
        assert!(serde_json::to_value(&config).unwrap()["nativeShellSandbox"].is_object());
        let frozen=config.process_sandbox.clone();
        assert!(config.merge_value(serde_json::json!({"nativeShellSandbox":null}),&root.path().join("project/.nola/config.json")).is_err());
        assert_eq!(config.process_sandbox,frozen);
        assert!(RuntimeConfig::default().merge_value(value,&root.path().join("project/.nola/config.json")).is_err());
    }

    #[test]
    fn parses_external_execution_resource_budgets_from_runtime_config() {
        let mut config = RuntimeConfig::default();
        config
            .merge_value(
                serde_json::json!({
                    "external_execution": {
                        "max_wall_time_seconds": 7200,
                        "max_output_bytes": 1048576,
                        "max_cpu_time_seconds": 3600,
                        "max_memory_bytes": 2147483648_u64,
                        "max_processes": 32,
                        "max_line_bytes": 262144
                    }
                }),
                Path::new("/workspace/.nola/config.json"),
            )
            .unwrap();

        assert_eq!(config.external_execution.max_wall_time_seconds, Some(7200));
        assert_eq!(config.external_execution.max_output_bytes, Some(1_048_576));
        assert_eq!(config.external_execution.max_cpu_time_seconds, Some(3600));
        assert_eq!(
            config.external_execution.max_memory_bytes,
            Some(2_147_483_648)
        );
        assert_eq!(config.external_execution.max_processes, Some(32));
        assert_eq!(config.external_execution.max_line_bytes, Some(262_144));
    }

    #[test]
    fn rejects_fallback_without_endpoint() {
        let mut config = RuntimeConfig::default();
        let error = config
            .merge_value(
                serde_json::json!({"llm": {"fallbacks": [{"model": "missing-endpoint"}]}}),
                Path::new("/workspace/.nola/config.json"),
            )
            .expect_err("fallback endpoint is required");
        assert!(
            error.contains("fallbacks[0].endpoint is required"),
            "{error}"
        );
    }

    #[test]
    fn loads_layered_runtime_config() {
        let root =
            std::env::temp_dir().join(format!("nola-runtime-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        let home_config_dir = home.join(".nola");
        let project_config_dir = project.join(".nola");
        std::fs::create_dir_all(&home_config_dir).unwrap();
        std::fs::create_dir_all(&project_config_dir).unwrap();
        std::fs::write(
            home_config_dir.join("config.json"),
            r#"{
                "llm": {"endpoint": "http://127.0.0.1:8080/v1", "model": "global-model"},
                "max_iterations": 12,
                "allowed_dirs": ["global-src"]
            }"#,
        )
        .unwrap();
        std::fs::write(
            project_config_dir.join("current_config.json"),
            r#"{
                // project-local defaults
                "llm": {
                    "model": "local-model",
                    "openai_cache": {
                        "enabled": true,
                        "key": "project-cache-key",
                        "key_prefix": "project",
                        "retention": "1h"
                    },
                    "capabilities": {
                        "instruction_profile": "qwen",
                        "native_tool_calls": true,
                        "parallel_tool_calls": false,
                        "reasoning": true,
                        "reasoning_continuity": true,
                        "request_continuation": false,
                        "max_request_continuations": 1,
                        "stream_text": true,
                        "stream_reasoning": true,
                        "stream_tool_calls": true
                    }
                },
                "debug": true,
                "max_iterations": 20,
                "allowed_dirs": ["src"],
                "language": "zh",
                "agents": {
                    "review": {
                        "prompt": "Review only.",
                        "tools": ["git_diff", "read_file"],
                        "max_turns": 4
                    }
                },
                "compaction": {"token_threshold": 12345, "context_window_tokens": 262144}
            }"#,
        )
        .unwrap();

        let config = RuntimeConfig::load_with_home(&project, Some(&home)).unwrap();

        assert_eq!(config.endpoint.as_deref(), Some("http://127.0.0.1:8080/v1"));
        assert_eq!(config.model.as_deref(), Some("local-model"));
        assert_eq!(config.openai_prompt_cache, Some(true));
        assert_eq!(
            config.openai_prompt_cache_key.as_deref(),
            Some("project-cache-key")
        );
        assert_eq!(
            config.openai_prompt_cache_key_prefix.as_deref(),
            Some("project")
        );
        assert_eq!(config.openai_prompt_cache_retention.as_deref(), Some("1h"));
        assert_eq!(
            config.model_capabilities.instruction_profile.as_deref(),
            Some("qwen")
        );
        assert_eq!(config.model_capabilities.parallel_tool_calls, Some(false));
        assert_eq!(config.model_capabilities.reasoning_continuity, Some(true));
        assert_eq!(config.model_capabilities.request_continuation, Some(false));
        assert_eq!(config.model_capabilities.max_request_continuations, Some(1));
        assert_eq!(config.max_iterations, Some(20));
        assert_eq!(
            config.allowed_dirs,
            vec![project.canonicalize().unwrap().join("src")]
        );
        assert_eq!(config.language.as_deref(), Some("zh"));
        assert_eq!(config.debug_mode, Some(true));
        assert_eq!(
            config
                .agents
                .get("review")
                .and_then(|agent| agent.max_turns),
            Some(4)
        );
        assert_eq!(config.compaction.token_threshold, Some(12345));
        assert_eq!(config.compaction.context_window_tokens, Some(262144));
        assert_eq!(
            config.sources,
            vec![
                home.join(".nola/config.json"),
                project
                    .canonicalize()
                    .unwrap()
                    .join(".nola/current_config.json")
            ]
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
