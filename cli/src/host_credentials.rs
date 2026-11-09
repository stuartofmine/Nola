// SPDX-License-Identifier: Apache-2.0
//! Join canonical endpoint metadata with the Host-private credential adapter.
//! Secret storage is isolated from the CLI and from canonical Application state.

pub use nola_credential_host::{identity, HostCredentialVault};
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointCredentialSnapshot {
    #[serde(rename = "endpointID")]
    pub endpoint_id: String,
    #[serde(rename = "baseURL")]
    pub base_url: String,
    pub wire_protocol: String,
    pub expected_revisions: std::collections::BTreeMap<String, i64>,
    pub credential_revision: i64,
    pub has_credential: bool,
}

pub fn endpoint_binding(
    store: &crate::store::NolaStore,
    endpoint_id: &str,
) -> Result<EndpointCredentialSnapshot, String> {
    let mut revisions = std::collections::BTreeMap::new();
    let mut endpoint: Option<(String, String, String)> = None;
    for profile in store.model_profiles()? {
        if profile.runtime != "remote" || profile.deleted_at_ms.is_some() {
            continue;
        }
        let source: serde_json::Value = serde_json::from_str(&profile.source_json)
            .map_err(|_| "Invalid endpoint configuration")?;
        if source["endpointID"].as_str() != Some(endpoint_id) {
            continue;
        }
        let base_url = source["baseURL"]
            .as_str()
            .ok_or("Endpoint URL is missing")?;
        let protocol = source["wireProtocol"]
            .as_str()
            .ok_or("Endpoint wire protocol is missing")?;
        let bound = identity(base_url, protocol)?;
        if endpoint
            .as_ref()
            .is_some_and(|(_, _, previous)| *previous != bound)
        {
            return Err("Endpoint members have inconsistent identities".into());
        }
        endpoint = Some((base_url.into(), protocol.into(), bound));
        revisions.insert(profile.id, profile.revision);
    }
    let (base_url, wire_protocol, _) = endpoint.ok_or("Endpoint is unavailable")?;
    Ok(EndpointCredentialSnapshot {
        endpoint_id: endpoint_id.into(),
        base_url,
        wire_protocol,
        expected_revisions: revisions,
        credential_revision: 0,
        has_credential: false,
    })
}

pub fn snapshot(
    store: &crate::store::NolaStore,
    endpoint_id: &str,
) -> Result<EndpointCredentialSnapshot, String> {
    let mut snapshot = endpoint_binding(store, endpoint_id)?;
    let bound = identity(&snapshot.base_url, &snapshot.wire_protocol)?;
    if let Some(record) = HostCredentialVault::open(store.path())?.record(endpoint_id)? {
        snapshot.credential_revision = record.revision;
        snapshot.has_credential = record.identity == bound && record.secret.is_some();
    }
    Ok(snapshot)
}

/// Credentials come only from the exact endpoint's vault record. An absent,
/// cleared, or differently bound record never borrows a process-global key.
#[cfg(not(any(unix, windows)))]
pub fn for_profile(
    _: &crate::store::NolaStore,
    _: &nola_application::ModelProfile,
    _: Option<&str>,
) -> Result<Option<String>, String> {
    Err("Endpoint credential storage is unavailable on this Host".into())
}

#[cfg(any(unix, windows))]
pub fn for_profile(
    store: &crate::store::NolaStore,
    profile: &nola_application::ModelProfile,
    protocol_override: Option<&str>,
) -> Result<Option<String>, String> {
    let source: serde_json::Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "Invalid endpoint configuration")?;
    let endpoint_id = source["endpointID"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("Endpoint identity is missing")?;
    let protocol = source["wireProtocol"]
        .as_str()
        .ok_or("Endpoint wire protocol is missing")?;
    if protocol == "siwc_responses" {
        if protocol_override.is_some_and(|value| value != protocol) || source["sourceKind"] != "nola_chatgpt" {
            return Err("SIWC cannot enter the API credential route".into());
        }
        return Ok(None); // Dedicated execution adapter validates the Core owner.
    }
    if source["connectionID"].as_str().is_some() {
        return crate::model_connection_host::credential_for_profile(store, &source, protocol_override.unwrap_or(protocol));
    }
    let bound = identity(
        source["baseURL"]
            .as_str()
            .ok_or("Endpoint URL is missing")?,
        protocol_override.unwrap_or(protocol),
    )?;
    let vault = HostCredentialVault::open(store.path())?;
    let Some(record) = vault.record(endpoint_id)? else {
        return Ok(None);
    };
    Ok(if record.identity == bound {
        record.secret
    } else {
        None
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn runtime_lookup_is_scoped_to_endpoint_identity_and_explicit_clear() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let mut profile = nola_application::ModelProfile {
            id:"model".into(), display_name:"Fixture".into(), provider:"openai-compatible".into(), model_name:"fixture".into(), runtime:"remote".into(),
            source_json:serde_json::json!({"endpointID":"a","baseURL":"https://a.invalid/v1","wireProtocol":"responses"}).to_string(), parameters_json:"{}".into(),
            selected:false, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        };
        store.upsert_model_profile(profile.clone()).unwrap();
        assert_eq!(for_profile(&store, &profile, None).unwrap(), None);
        let mut vault = HostCredentialVault::open(store.path()).unwrap();
        vault
            .write(
                "a",
                &identity("https://a.invalid/v1", "responses").unwrap(),
                0,
                Some("only-a"),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(
            for_profile(&store, &profile, None).unwrap(),
            Some("only-a".into())
        );
        assert_eq!(
            for_profile(&store, &profile, Some("anthropic")).unwrap(),
            None
        );
        profile.source_json = serde_json::json!({"endpointID":"a","baseURL":"https://b.invalid/v1","wireProtocol":"responses"}).to_string();
        assert_eq!(for_profile(&store, &profile, None).unwrap(), None);
        profile.source_json = serde_json::json!({"endpointID":"b","baseURL":"https://a.invalid/v1","wireProtocol":"responses"}).to_string();
        assert_eq!(for_profile(&store, &profile, None).unwrap(), None);
        vault
            .write(
                "b",
                &identity("https://a.invalid/v1", "responses").unwrap(),
                0,
                None,
                || Ok(()),
            )
            .unwrap();
        assert_eq!(for_profile(&store, &profile, None).unwrap(), None);
    }

    #[test]
    fn profile_requires_current_endpoint_identity_and_protocol_even_without_a_key() {
        let directory = tempfile::tempdir().unwrap();
        let store =
            crate::store::NolaStore::open(&directory.path().join("canonical.sqlite")).unwrap();
        let mut profile = nola_application::ModelProfile {
            id: "model".into(),
            display_name: "Fixture".into(),
            provider: "openai-compatible".into(),
            model_name: "fixture".into(),
            runtime: "remote".into(),
            source_json: "{}".into(),
            parameters_json: "{}".into(),
            selected: false,
            enabled: true,
            revision: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
            deleted_at_ms: None,
        };
        for source in [
            serde_json::json!({"baseURL":"https://a.invalid/v1","wireProtocol":"responses"}),
            serde_json::json!({"endpointID":"a","baseURL":"https://a.invalid/v1"}),
        ] {
            profile.source_json = source.to_string();
            assert!(for_profile(&store, &profile, None).is_err());
        }
    }
}
