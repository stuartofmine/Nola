// SPDX-License-Identifier: Apache-2.0
//! Model route identity and native channel validation; no product configuration or terminal.
use nola_host_protocol::HostModelRouteBinding;
use sha2::{Digest, Sha256};

pub(crate) fn route_binding(
    provider: Option<&str>,
    endpoint: &str,
    model: &str,
    credential: &str,
    token_field: nola_application::ChatOutputTokenField,
) -> Result<HostModelRouteBinding, String> {
    if endpoint.len() > 4096
        || model.len() > 1024
        || provider.is_some_and(|value| value.len() > 128)
    {
        return Err("Host invocation route exceeds schema limits".into());
    }
    let sha = |value: &[u8]| format!("{:x}", Sha256::digest(value));
    let (endpoint, query) = if endpoint.trim().is_empty() {
        (String::new(), None)
    } else {
        let mut url = url::Url::parse(endpoint.trim())
            .map_err(|_| "Host invocation unavailable: invalid provider endpoint")?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("Host invocation unavailable: provider endpoint must be HTTP(S) without user-info or fragment".into());
        }
        let query = url.query().map(|value| sha(value.as_bytes()));
        url.set_query(None);
        (url.to_string(), query)
    };
    Ok(HostModelRouteBinding::Http(nola_host_protocol::HostHttpModelRouteBinding {
        siwc_account: None,
        protocol: crate::llm::resolved_model_protocol(provider, &endpoint),
        output_token_field: crate::llm::resolved_output_token_field(
            crate::llm::resolved_model_protocol(provider, &endpoint), &endpoint, token_field),
        provider: provider
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_ascii_lowercase),
        endpoint,
        endpoint_query_source: if query.is_some() {
            nola_host_protocol::HostSecretSource::Ephemeral
        } else {
            nola_host_protocol::HostSecretSource::None
        },
        endpoint_query_sha256: query,
        model: model.trim().to_string(),
        credential_sha256: sha(credential.as_bytes()),
        credential_source: if credential.is_empty() {
            nola_host_protocol::HostSecretSource::None
        } else {
            nola_host_protocol::HostSecretSource::Ephemeral
        },
    }))
}

pub(crate) fn validate_native_transport(
    route: &nola_host_protocol::HostNativeModelRouteBinding,
    endpoint: &str,
    model: &str,
    credential: &str,
) -> Result<(), String> {
    HostModelRouteBinding::Native(route.clone()).validate().map_err(str::to_string)?;
    let url = url::Url::parse(endpoint).map_err(|_| "invalid native model channel endpoint")?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if route.adapter != nola_host_protocol::HostNativeModelAdapter::Mlx
        || route.profile_ref.is_none() || route.model_id != model
        || credential.trim().is_empty() || !loopback
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        return Err("native model channel does not match its admitted local model identity".into());
    }
    Ok(())
}
