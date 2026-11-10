// SPDX-License-Identifier: Apache-2.0
//! Cold maintenance may use cheaper reasoning only with concrete adapter evidence.
//! Never apply this to a physical cache fork or infer support from a model name.
use super::contracts::{resolved_model_protocol, LlmConfig};
use nola_domain::generation::{ModelProtocol, ReasoningSelection};
use serde_json::Value;
use std::{collections::HashMap, io::Read, sync::{Mutex, OnceLock}, time::{Duration, Instant}};

type ProbeCache = Mutex<HashMap<String, (Instant, bool)>>;
static RESPONSES_MAINTENANCE_CACHE: OnceLock<ProbeCache> = OnceLock::new();

fn declares_strata_responses(value: &Value, model: &str) -> bool {
    value["service"] == "strata" && value["model"] == model
        && value["dialects"].as_array().is_some_and(|paths|
            paths.iter().any(|path| path == "/v1/responses"))
}

fn probe_strata_responses(config: &LlmConfig) -> Option<bool> {
    let mut url = reqwest::Url::parse(&config.endpoint).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        return None;
    }
    let base = url.path().trim_end_matches('/').trim_end_matches("/responses").to_owned();
    // Probe only inside the configured API namespace, never guess another host.
    url.set_path(&format!("{base}/status"));
    url.set_fragment(None);
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(800)).build().ok()?;
    let mut request = client.get(url);
    if !config.api_key.is_empty() { request = request.bearer_auth(&config.api_key); }
    let response = request.send().ok()?;
    if !response.status().is_success() { return None; }
    let mut bytes = Vec::new();
    response.take(262_145).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 262_144 { return None; }
    let value = serde_json::from_slice::<Value>(&bytes).ok()?;
    Some(declares_strata_responses(&value, &config.model))
}

pub(crate) fn configure_cold_compaction_reasoning(config: &mut LlmConfig) {
    if config.compaction_output_budget.is_none() || config.native_model_route.is_some()
        || config.cancellation.is_requested()
        || resolved_model_protocol(config.provider.as_deref(), &config.endpoint) != ModelProtocol::Responses {
        return;
    }
    let key = super::request_snapshot::request_configuration_identity(config);
    let cache = RESPONSES_MAINTENANCE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cached = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key)
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(300)).map(|(_, supported)| *supported);
    let supported = cached.unwrap_or_else(|| {
        let supported = probe_strata_responses(config).unwrap_or(false);
        let mut entries = cache.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(300));
        if entries.len() >= 128 { entries.clear(); }
        entries.insert(key, (Instant::now(), supported));
        supported
    });
    if supported && !config.cancellation.is_requested() {
        config.reasoning_selection = Some(ReasoningSelection::Effort { value: "none".into() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn compaction_reasoning_requires_exact_adapter_model_and_protocol_evidence() {
        let status = json!({"service":"strata","model":"model","dialects":["/v1/responses"]});
        assert!(declares_strata_responses(&status,"model"));
        assert!(!declares_strata_responses(&status,"other-model"));
        assert!(!declares_strata_responses(&json!({"service":"openai","model":"model","dialects":["/v1/responses"]}),"model"));
        assert!(!declares_strata_responses(&json!({"service":"strata","model":"model","dialects":[]}),"model"));
        assert!(!declares_strata_responses(&json!({}),"qwen3.8-flash-next-iq3_s"));
    }
}
