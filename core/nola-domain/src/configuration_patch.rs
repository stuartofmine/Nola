// SPDX-License-Identifier: Apache-2.0
//! Domain-owned configuration patch validation.
use crate::generation::GenerationOptions;
use serde_json::{Value, json};
use crate::generation::{validate_thinking_budget, REASONING_EFFORT_VALUES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationConfigurationPatch {
    raw: String,
    fields: serde_json::Map<String, Value>,
}
impl ConversationConfigurationPatch {
    pub fn fields(&self) -> &serde_json::Map<String, Value> { &self.fields }
}
impl std::str::FromStr for ConversationConfigurationPatch {
    type Err = String;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        validate_patch(raw)?;
        let value: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
        if value.as_object().is_some_and(|fields| fields.is_empty()) { return Err("configuration patch must not be empty".into()); }
        Ok(Self { raw: raw.into(), fields: value.as_object().expect("validated object").clone() })
    }
}
impl std::ops::Deref for ConversationConfigurationPatch {
    type Target = str;
    fn deref(&self) -> &str { &self.raw }
}
impl serde::Serialize for ConversationConfigurationPatch {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> { serializer.serialize_str(&self.raw) }
}
impl<'de> serde::Deserialize<'de> for ConversationConfigurationPatch {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

pub fn knowledge_patch_fields(value: &Value) -> Result<&serde_json::Map<String, Value>, &'static str> {
    let fields = value.as_object().ok_or("knowledge must be an object")?;
    if fields.is_empty() || fields.iter().any(|(key, value)| !matches!(key.as_str(), "keyPointsMarkdown" | "wikiMarkdown") || !value.is_string()) {
        return Err("knowledge patch must contain only Markdown string fields");
    }
    Ok(fields)
}

pub fn valid_model_identity(key: &str) -> bool {
    if let Some(encoded) = key.strip_prefix("connectedHost:") {
        let Some((host, profile)) = encoded.split_once(':') else {
            return false;
        };
        return key.len() <= 2048
            && [host, profile].iter().all(|part| {
                decode_identity_component(part)
                    .filter(|bytes| bytes.len() <= 512)
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .is_some_and(|value| !value.trim().is_empty())
            });
    }
    key.len() <= 512
        && key.split_once(':').is_some_and(|(kind, id)| {
            ["configured", "local"].contains(&kind) && !id.trim().is_empty()
        })
}

/// Canonical URL-safe, unpadded base64 for bounded identity components only.
fn decode_identity_component(value: &str) -> Option<Vec<u8>> {
    if value.len() > 684 || value.len() % 4 == 1 { return None; }
    let mut bytes=Vec::with_capacity(value.len()*3/4);
    let mut accumulator=0u32;
    let mut bits=0;
    for byte in value.bytes() {
        let digit=match byte { b'A'..=b'Z'=>byte-b'A', b'a'..=b'z'=>byte-b'a'+26,
            b'0'..=b'9'=>byte-b'0'+52, b'-'=>62, b'_'=>63, _=>return None };
        accumulator=(accumulator<<6)|u32::from(digit); bits+=6;
        if bits>=8 { bits-=8; bytes.push((accumulator>>bits) as u8); }
    }
    if bits>0 && accumulator & ((1<<bits)-1) != 0 { return None; }
    Some(bytes)
}

pub fn validate_patch(raw: &str) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|_| "invalid conversation configuration JSON")?;
    let object = value.as_object().ok_or("configuration must be an object")?;
    if let Some(policy) = object.get("executionPolicy").filter(|value| !value.is_null()) {
        let policy: crate::execution_policy::ConversationExecutionPolicy =
            serde_json::from_value(policy.clone()).map_err(|error| error.to_string())?;
        policy.validate()?;
    }
    for (key, value) in object {
        let valid = match key.as_str() {
            "language" | "selectedPromptTemplateID" | "promptResponseStyle" => value.is_null() || value.as_str().is_some_and(|value| value.len() <= 512),
            "executionBackend" => value.as_str().is_some_and(|value| crate::execution_backend::ExecutionBackendKind::ALL.iter().any(|kind| kind.id() == value)),
            "persistenceEnabled" | "promptTemplateSelectionIsExplicit" => value.is_boolean(),
            "contextLimitOverride" => value.is_null() || value.as_u64().is_some_and(|value| value >= 512 && value <= 4_000_000),
            "dataSourceOverrides" => value.as_object().is_some_and(|fields| fields.len() <= 4096 && fields.iter().all(|(key, value)| !key.trim().is_empty() && (value.is_boolean() || value.is_null()))),
            "externalExecutionSessions" | "externalExecutionModelIDs" | "externalExecutionModelProviderIDs" => value.as_object().is_some_and(|fields| fields.len() <= 128 && fields.iter().all(|(key, value)| !key.trim().is_empty() && (value.is_string() || value.is_null()))),
            "dataSourceIds" => value.is_null() || value.as_array().is_some_and(|values| values.len() <= 4096 && values.iter().all(|value| value.as_str().is_some_and(|value| !value.trim().is_empty()))),
            _ => true,
        };
        if !valid { return Err(format!("invalid configuration field: {key}")); }
    }
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "modelSelectionID" | "executionHostRawValue" | "executionHostBinding" | "clientHostParameters" | "modelOverrides" | "clientModelPresentation" | "permissionProfile" | "isPinned" | "memoryEnabledOverride" | "retrievalEnabledOverride" | "knowledge" | "language" | "selectedPromptTemplateID" | "promptResponseStyle" | "executionBackend" | "externalExecutionSessions" | "persistenceEnabled" | "dataSourceOverrides" | "dataSourceIds" | "executionPolicy" | "promptTemplateSelectionIsExplicit" | "contextLimitOverride" | "externalExecutionModelIDs" | "externalExecutionModelProviderIDs"))
    {
        return Err("unsupported conversation configuration field".into());
    }
    if object.get("modelSelectionID").is_some_and(|value| !value.is_null() && !value.as_str().is_some_and(valid_model_identity)) {
        return Err("model selection requires an explicit model identity".into());
    }
    if object.get("executionHostRawValue").is_some_and(|value| !value.is_null() && !value.as_str().is_some_and(|value| ["local", "remote"].contains(&value))) {
        return Err("invalid conversation execution host".into());
    }
    if let Some(binding) = object.get("executionHostBinding").filter(|value| !value.is_null()) {
        let fields = binding.as_object().ok_or("executionHostBinding must be an object")?;
        if fields.keys().any(|key| !["baseURL", "tlsCertificateSHA256"].contains(&key.as_str())) {
            return Err("unsupported host binding field".into());
        }
        let raw = fields.get("baseURL").and_then(Value::as_str).filter(|value| value.len() <= 4096).ok_or("host baseURL required")?;
        let url = url::Url::parse(raw).map_err(|_| "invalid host URL")?;
        if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err("host URL must be http(s) without credentials, query or fragment".into());
        }
        if fields.get("tlsCertificateSHA256").is_some_and(|pin| !pin.is_null() && !pin.as_str().is_some_and(|pin| pin.len() == 64 && pin.bytes().all(|byte| byte.is_ascii_hexdigit()))) {
            return Err("invalid host TLS certificate pin".into());
        }
        if object.get("modelSelectionID").is_some_and(|value| !value.is_null()) {
            return Err("model selection and execution host binding are mutually exclusive".into());
        }
    }
    if let Some(parameters) = object.get("clientHostParameters").filter(|value| !value.is_null()) {
        validate_patch(&serde_json::json!({"modelOverrides":{"local:host-parameter-validation":parameters}}).to_string())?;
    }
    if let Some(models) = object.get("clientModelPresentation") {
        let models = models.as_object().filter(|models| models.len() <= 128)
            .ok_or("clientModelPresentation must be a bounded model map")?;
        for (key, model) in models {
            if key != "execution-host" && !valid_model_identity(key) { return Err("invalid presentation model identity".into()); }
            if model.is_null() { continue; }
            let fields = model.as_object().ok_or("model presentation must be an object or null")?;
            for (field, value) in fields {
                let valid = match field.as_str() {
                    "showThinking" | "streamOutput" | "enableRetrieval" => value.is_null() || value.is_boolean(),
                    "responseStyle" => value.is_null() || value.as_str().is_some_and(|value| value.len() <= 128),
                    "customSystemPrompt" => value.is_null() || value.as_str().is_some_and(|value| value.len() <= 262_144),
                    _ => false,
                };
                if !valid { return Err("invalid model presentation field".into()); }
            }
        }
    }
    if let Some(knowledge) = object.get("knowledge") {
        knowledge_patch_fields(knowledge)?;
    }
    if object.get("isPinned").is_some_and(|value| !value.is_boolean()) {
        return Err("isPinned must be a boolean".into());
    }
    if object.get("memoryEnabledOverride").is_some_and(|value| !value.is_boolean() && !value.is_null()) {
        return Err("memoryEnabledOverride must be boolean or null".into());
    }
    if object.get("retrievalEnabledOverride").is_some_and(|value| !value.is_boolean() && !value.is_null()) {
        return Err("retrievalEnabledOverride must be boolean or null".into());
    }
    if let Some(permission) = object.get("permissionProfile") {
        if !permission.is_null()
            && !permission
                .as_str()
                .is_some_and(|value| crate::PermissionProfile::from_canonical_name(value).is_some())
        {
            return Err("invalid conversation permission profile".into());
        }
    }
    if let Some(models) = object.get("modelOverrides") {
        let models = models
            .as_object()
            .ok_or("modelOverrides must be an object")?;
        if models.len() > 128 {
            return Err("too many model overrides".into());
        }
        for (key, model) in models {
            if !valid_model_identity(key) {
                return Err("invalid conversation model identity".into());
            }
            if model.is_null() {
                continue;
            }
            let fields = model
                .as_object()
                .ok_or("model override must be an object or null")?;
            if key.starts_with("connectedHost:")
                && fields.keys().any(|field| {
                    ["wireProtocol", "reasoning", "reasoningPreference", "reasoningTier"].contains(&field.as_str())
                })
            {
                return Err(
                    "connected host overrides cannot redefine protocol or reasoning capabilities"
                        .into(),
                );
            }
            for (field, value) in fields {
                if ![
                    "temperature",
                    "topP",
                    "maxOutputTokens",
                    "maxContextLength",
                    "topK",
                    "presencePenalty",
                    "frequencyPenalty",
                    "repetitionPenalty",
                    "seed",
                    "stopSequences",
                    "wireProtocol",
                    "reasoning",
                    "reasoningPreference",
                    "reasoningTier",
                    "samplingOverride",
                ]
                .contains(&field.as_str())
                {
                    return Err("unsupported model override field".into());
                }
                if value.is_null() {
                    continue;
                }
                let valid = match field.as_str() {
                    "temperature" => number(value, 0.0, 2.0),
                    "topP" => number(value, 0.0, 1.0),
                    "maxOutputTokens" => integer(value, 64, 1_000_000),
                    "maxContextLength" => integer(value, 512, 4_000_000),
                    // The shared Domain contract validates these after wire decoding.
                    "presencePenalty" | "frequencyPenalty" | "repetitionPenalty" | "topK"
                    | "seed" => true,
                    "stopSequences" => value.as_str().is_some_and(|text| text.len() <= 65536),
                    "wireProtocol" => value.as_str().is_some_and(|text| {
                        ["responses", "chat_completions", "anthropic"].contains(&text)
                    }),
                    "reasoningTier" => serde_json::from_value::<crate::reasoning_tiers::ReasoningTier>(value.clone()).is_ok(),
                    "reasoningPreference" => value.as_str().is_some_and(|text| {
                        [
                            "automatic",
                            "none",
                            "minimal",
                            "low",
                            "medium",
                            "high",
                            "xhigh",
                            "max",
                            "concise",
                            "balanced",
                            "detailed",
                        ]
                        .contains(&text)
                    }),
                    "reasoning" => value.is_object(),
                    "samplingOverride" => value.is_boolean(),
                    _ => false,
                };
                if !valid {
                    return Err(format!("invalid model override: {field}"));
                }
            }
            if let Some(reasoning) = fields.get("reasoning").filter(|value| !value.is_null()) {
                let protocol = fields
                    .get("wireProtocol")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| {
                        if reasoning["control"] == "budget" {
                            "anthropic"
                        } else {
                            "responses"
                        }
                    });
                let output = fields
                    .get("maxOutputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(1_000_000);
                validate_model_reasoning(reasoning, protocol, output)?;
            }
            validate_generation_fields(fields)?;
        }
    }
    Ok(())
}

/// Only wire spelling lives here. The accepted values and limits are owned by
/// Domain and shared with execution, so persistence cannot admit a value that
/// the runtime rejects (for example topK=0).
fn validate_generation_fields(fields: &serde_json::Map<String, Value>) -> Result<(), String> {
    let mut options = serde_json::Map::new();
    for (source, target) in [
        ("topK", "top_k"),
        ("presencePenalty", "presence_penalty"),
        ("frequencyPenalty", "frequency_penalty"),
        ("repetitionPenalty", "repetition_penalty"),
    ] {
        if let Some(value) = fields.get(source).filter(|value| !value.is_null()) {
            options.insert(target.into(), value.clone());
        }
    }
    if let Some(seed) = fields.get("seed").filter(|value| !value.is_null()) {
        let seed: crate::generation_override::SeedOverride =
            serde_json::from_value(seed.clone())
                .map_err(|error| format!("invalid seed override: {error}"))?;
        options.insert(
            "seed".into(),
            serde_json::to_value(seed.resolved()).map_err(|error| error.to_string())?,
        );
    }
    if let Some(stop) = fields.get("stopSequences").and_then(Value::as_str) {
        options.insert(
            "stop".into(),
            serde_json::json!(stop
                .lines()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()),
        );
    }
    let options: GenerationOptions = serde_json::from_value(Value::Object(options))
        .map_err(|error| format!("invalid generation parameters: {error}"))?;
    options.validate()
}
fn number(value: &Value, min: f64, max: f64) -> bool {
    value
        .as_f64()
        .is_some_and(|value| value.is_finite() && value >= min && value <= max)
}
fn integer(value: &Value, min: u64, max: u64) -> bool {
    value
        .as_u64()
        .is_some_and(|value| value >= min && value <= max)
}


/// Shared endpoint/model editor contract. Inactive fields must remain schema
/// valid; changing protocol never erases a model's explicit thinking choice.
pub fn validate_model_reasoning(
    reasoning: &Value,
    protocol: &str,
    output_limit: u64,
) -> Result<(), String> {
    if reasoning.is_null() {
        return Ok(());
    }
    let object = reasoning
        .as_object()
        .ok_or("reasoning must be an object or null")?;
    if object.keys().any(|key| {
        ![
            "control",
            "responses",
            "efforts",
            "budgets",
            "outputLimit",
            "defaultPreference",
        ]
        .contains(&key.as_str())
    }) {
        return Err("unsupported reasoning configuration field".into());
    }
    if object
        .get("responses")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("responses must be a boolean".into());
    }
    if object.get("outputLimit").is_some_and(|value| {
        !value.as_u64().is_some_and(|limit| {
            crate::generation::resolve_output_token_limit(Some(limit)).is_ok()
        })
    }) {
        return Err("invalid reasoning outputLimit".into());
    }
    let known = REASONING_EFFORT_VALUES;
    if object.get("efforts").is_some_and(|value| {
        !value.as_array().is_some_and(|values| {
            values
                .iter()
                .all(|value| value.as_str().is_some_and(|value| known.contains(&value)))
        })
    }) {
        return Err("invalid supported reasoning efforts".into());
    }
    if object.get("budgets").is_some_and(|value| {
        !value
            .as_array()
            .is_some_and(|values| values.iter().all(|value| value.as_u64().is_some()))
    }) {
        return Err("invalid reasoning budgets".into());
    }
    let tiers = ["concise", "balanced", "detailed"];
    let options: Vec<&str> = match reasoning["control"].as_str() {
        Some("automatic") => Vec::new(),
        Some("effort") if protocol != "anthropic" => {
            let efforts = reasoning["efforts"]
                .as_array()
                .ok_or("efforts must be an array")?;
            if efforts.is_empty() {
                return Err("invalid supported reasoning efforts".into());
            }
            if matches!(protocol, "responses" | "siwc_responses") {
                efforts.iter().filter_map(Value::as_str).collect()
            } else {
                if !["low", "medium", "high"]
                    .iter()
                    .all(|value| efforts.contains(&json!(value)))
                {
                    return Err(
                        "Chat Completions three-tier control requires low, medium and high".into(),
                    );
                }
                tiers.to_vec()
            }
        }
        Some("budget") if protocol == "anthropic" => {
            let budgets = reasoning["budgets"]
                .as_array()
                .ok_or("budgets must be an array")?;
            let values = budgets
                .iter()
                .map(|value| value.as_u64().ok_or("budget must be a positive integer"))
                .collect::<Result<Vec<_>, _>>()?;
            if values.len() != 3
                || values
                    .iter()
                    .any(|value| validate_thinking_budget(*value, output_limit).is_err())
                || !values.windows(2).all(|pair| pair[0] < pair[1])
            {
                return Err("three increasing budgets below the output limit are required".into());
            }
            tiers.to_vec()
        }
        _ => return Err("thinking control is incompatible with the model protocol".into()),
    };
    if let Some(value) = object
        .get("defaultPreference")
        .filter(|value| !value.is_null())
    {
        let preference = value
            .as_str()
            .ok_or("defaultPreference must be a string or null")?;
        if preference != "automatic" && !options.contains(&preference) {
            return Err("default thinking selection is incompatible with the control".into());
        }
    }
    Ok(())
}
