// SPDX-License-Identifier: Apache-2.0
//! Explicitly confirmed, single-request text protocol probe. No automatic inference.
use crate::store::NolaStore;
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::{
    generation::ModelProtocol,
    model_connection::{ModelCapabilityEvidence, ModelConnectionSnapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeConfirmation {
    pub request_id: String,
    pub connection_id: String,
    pub expected_revision: i64,
    pub account_id: Option<String>,
    pub account_revision: Option<i64>,
    pub credential_binding: String,
    pub model_id: String,
    pub protocol: ModelProtocol,
    pub max_requests: u32,
    pub acknowledge_possible_charges: bool,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nola_domain::model_connection::*;
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap()
    }
    pub(crate) fn setup(
        protocol: ModelProtocol,
    ) -> (
        tempfile::TempDir,
        NolaStore,
        tiny_http::Server,
        ProbeConfirmation,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&directory.path().join("probe.sqlite")).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let connection = ModelConnection {
            id: "fixture".into(),
            display_name: "Fixture".into(),
            base_url: format!("http://{}/v1", server.server_addr()),
            protocols: vec![protocol],
            account_id: None,
            source: ModelSourceKind::Manual,
            source_id: None,
            enabled: true,
        };
        let saved: ModelConnectionSnapshot = execute(
            &store,
            ModelCommand::SaveConnection {
                input: connection.clone(),
                expected_revision: None,
                updated_at_ms: 1,
            },
        )
        .unwrap();
        let saved: ModelConnectionSnapshot = execute(
            &store,
            ModelCommand::RefreshConnectionCatalog {
                connection_id: connection.id.clone(),
                expected_revision: saved.revision,
                observed_at_ms: 2,
                models: vec![CatalogModel {
                    model_id: "fixture-model".into(),
                    display_name: "Fixture".into(),
                    protocol,
                    context_window: None,
                    output_limit: None,
                    modalities: vec![],
                    reasoning_levels: vec![], summary_supported: None,
                    execution_capabilities: None,
                }],
            },
        )
        .unwrap();
        let confirmation = ProbeConfirmation {
            request_id: uuid::Uuid::new_v4().to_string(),
            connection_id: connection.id.clone(),
            expected_revision: saved.revision,
            account_id: None,
            account_revision: None,
            credential_binding: credential_binding(&store, &connection, protocol).unwrap().0,
            model_id: "fixture-model".into(),
            protocol,
            max_requests: 1,
            acknowledge_possible_charges: true,
        };
        (directory, store, server, confirmation)
    }
    #[test]
    fn paid_probe_requires_exact_confirmation_and_cannot_replay_persisted_attempt() {
        let _serial = serial();
        let (_directory, store, server, confirmation) = setup(ModelProtocol::Responses);
        for change in 0..4 {
            let mut invalid = confirmation.clone();
            match change {
                0 => invalid.acknowledge_possible_charges = false,
                1 => invalid.max_requests = 2,
                2 => invalid.account_revision = Some(9),
                _ => invalid.credential_binding = "b".repeat(64),
            }
            assert!(probe(&store, invalid, &[]).is_err());
        }
        assert!(server.try_recv().unwrap().is_none());
        reserve_attempt(&store, &confirmation).unwrap();
        let reopened = NolaStore::open(store.path()).unwrap();
        assert!(reserve_attempt(&reopened, &confirmation).is_err());
        let attempt: Option<ModelProbeAttempt> = execute(
            &reopened,
            ModelCommand::GetModelProbeAttempt {
                request_id: confirmation.request_id,
            },
        )
        .unwrap();
        assert!(attempt.unwrap().text_confirmed.is_none());
    }
    #[test]
    fn only_protocol_text_success_records_evidence_without_changing_selection() {
        let _serial = serial();
        for protocol in [
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::Anthropic,
        ] {
            let (_directory, store, server, confirmation) = setup(protocol);
            let expected_path = match protocol {
                ModelProtocol::SiwcResponses => unreachable!("API probe fixture excludes subscription transport"),
                ModelProtocol::Responses => "/v1/responses",
                ModelProtocol::ChatCompletions => "/v1/chat/completions",
                ModelProtocol::Anthropic => "/v1/messages",
            };
            let worker = std::thread::spawn(move || {
                let mut request = server.recv().unwrap();
                assert_eq!(request.method(), &tiny_http::Method::Post);
                assert_eq!(request.url(), expected_path);
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let body: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(body["model"], "fixture-model");
                assert_eq!(body["stream"], false);
                assert!(body["tools"].is_null());
                let response = match protocol {
                    ModelProtocol::SiwcResponses => unreachable!("API probe fixture excludes subscription transport"),
                    ModelProtocol::Responses => {
                        json!({"output":[{"content":[{"type":"output_text","text":"OK"}]}]})
                    }
                    ModelProtocol::ChatCompletions => {
                        json!({"choices":[{"message":{"content":"OK"}}]})
                    }
                    ModelProtocol::Anthropic => json!({"content":[{"type":"text","text":"OK"}]}),
                };
                request
                    .respond(tiny_http::Response::from_string(response.to_string()))
                    .unwrap();
            });
            let observed = probe(&store, confirmation.clone(), &[]).unwrap();
            worker.join().unwrap();
            let assessment = &observed.document.capability_assessments.as_ref().unwrap()[0];
            assert!(assessment.evidence.as_ref().unwrap().text_generation);
            assert!(assessment.user_override.is_none());
            assert!(observed.document.catalog.as_ref().unwrap().models[0]
                .execution_capabilities
                .is_none());
            let profiles: Vec<Value> = execute(&store, ModelCommand::ListProfiles).unwrap();
            assert!(profiles.is_empty());
            assert!(probe(&store, confirmation, &[]).is_err());
        }
    }
    #[test]
    fn empty_or_error_envelopes_do_not_prove_text_support() {
        let _serial = serial();
        for response in [
            json!({}),
            json!({"error":{"message":"no"}}),
            json!({"status":"failed","output":[{"content":[{"type":"output_text","text":"partial"}]}]}),
            json!({"type":"error","output":[{"content":[{"type":"output_text","text":"partial"}]}]}),
            json!({"output":[{"content":[{"type":"output_text","text":""}]}]}),
        ] {
            let (_directory, store, server, confirmation) = setup(ModelProtocol::Responses);
            let worker = std::thread::spawn(move || {
                server
                    .recv()
                    .unwrap()
                    .respond(tiny_http::Response::from_string(response.to_string()))
                    .unwrap()
            });
            assert!(probe(&store, confirmation, &[]).is_err());
            worker.join().unwrap();
            let saved: Option<ModelConnectionSnapshot> = execute(
                &store,
                ModelCommand::GetConnection {
                    id: "fixture".into(),
                },
            )
            .unwrap();
            assert!(saved.unwrap().document.capability_assessments.is_none());
        }
    }

    #[test]
    fn cancelled_or_revision_changed_probe_cannot_commit_late_evidence() {
        let _serial = serial();
        for cancel in [true, false] {
            let (_directory, store, server, confirmation) = setup(ModelProtocol::Responses);
            let path = store.path().to_owned();
            let input = confirmation.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                sender
                    .send(probe(&NolaStore::open(&path).unwrap(), input, &[]))
                    .unwrap();
            });
            let request = server
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .unwrap();
            if cancel {
                assert!(crate::model_catalog_discovery::cancel(
                    &store,
                    &confirmation.request_id,
                    &confirmation.connection_id,
                    confirmation.expected_revision
                )
                .unwrap());
                assert!(receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                    .is_err());
                drop(request);
            } else {
                let saved: Option<ModelConnectionSnapshot> = execute(
                    &store,
                    ModelCommand::GetConnection {
                        id: "fixture".into(),
                    },
                )
                .unwrap();
                let mut connection = saved.unwrap().document.connection;
                connection.display_name = "Changed while waiting".into();
                let _: ModelConnectionSnapshot = execute(
                    &store,
                    ModelCommand::SaveConnection {
                        input: connection,
                        expected_revision: Some(confirmation.expected_revision),
                        updated_at_ms: 3,
                    },
                )
                .unwrap();
                request
                    .respond(tiny_http::Response::from_string(
                        json!({"output":[{"content":[{"type":"output_text","text":"OK"}]}]})
                            .to_string(),
                    ))
                    .unwrap();
                assert!(receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                    .is_err());
            }
            worker.join().unwrap();
            let saved: Option<ModelConnectionSnapshot> = execute(
                &store,
                ModelCommand::GetConnection {
                    id: "fixture".into(),
                },
            )
            .unwrap();
            assert!(saved.unwrap().document.capability_assessments.is_none());
            assert!(reserve_attempt(&store, &confirmation).is_err());
        }
    }

    #[test]
    fn independent_vault_rotation_invalidates_confirmation_without_network() {
        let _serial = serial();
        let (_directory, store, server, mut confirmation) = setup(ModelProtocol::Responses);
        let account = ModelAccount {
            id: "fixture-account".into(),
            display_name: "Fixture account".into(),
            source: ModelSourceKind::Manual,
            credential_ref: Some("fixture-ref".into()),
            state: ModelAccountState::Ready,
        };
        let account: ModelAccountSnapshot = execute(
            &store,
            ModelCommand::SaveAccount {
                input: account,
                expected_revision: None,
                updated_at_ms: 3,
            },
        )
        .unwrap();
        let current: Option<ModelConnectionSnapshot> = execute(
            &store,
            ModelCommand::GetConnection {
                id: "fixture".into(),
            },
        )
        .unwrap();
        let mut connection = current.unwrap().document.connection;
        connection.account_id = Some(account.account.id.clone());
        let saved: ModelConnectionSnapshot = execute(
            &store,
            ModelCommand::SaveConnection {
                input: connection.clone(),
                expected_revision: Some(confirmation.expected_revision),
                updated_at_ms: 4,
            },
        )
        .unwrap();
        // Route change intentionally clears the catalog; explicitly refresh this fixture.
        let saved: ModelConnectionSnapshot = execute(
            &store,
            ModelCommand::RefreshConnectionCatalog {
                connection_id: connection.id.clone(),
                expected_revision: saved.revision,
                observed_at_ms: 5,
                models: vec![CatalogModel {
                    model_id: confirmation.model_id.clone(),
                    display_name: "Fixture".into(),
                    protocol: confirmation.protocol,
                    context_window: None,
                    output_limit: None,
                    modalities: vec![],
                    reasoning_levels: vec![], summary_supported: None,
                    execution_capabilities: None,
                }],
            },
        )
        .unwrap();
        let bound = crate::host_credentials::identity(&connection.base_url, "responses").unwrap();
        let mut vault = crate::host_credentials::HostCredentialVault::open(store.path()).unwrap();
        let revision = vault
            .write(
                "fixture-ref:responses",
                &bound,
                0,
                Some("isolated-first"),
                || Ok(()),
            )
            .unwrap();
        confirmation.expected_revision = saved.revision;
        confirmation.account_id = connection.account_id.clone();
        confirmation.account_revision = Some(account.revision);
        confirmation.credential_binding =
            credential_binding(&store, &connection, confirmation.protocol)
                .unwrap()
                .0;
        vault
            .write(
                "fixture-ref:responses",
                &bound,
                revision,
                Some("isolated-second"),
                || Ok(()),
            )
            .unwrap();
        assert!(probe(&store, confirmation, &[])
            .unwrap_err()
            .contains("credential_confirmation_stale"));
        assert!(server.try_recv().unwrap().is_none());
    }
}
/// Opaque binding includes the exact vault slot revision, never its secret.
pub(crate) fn credential_binding(
    store: &NolaStore,
    connection: &nola_domain::model_connection::ModelConnection,
    protocol: ModelProtocol,
) -> Result<(String, Option<String>), String> {
    let mut binding =
        json!({"url":connection.base_url,"protocol":protocol,"account":connection.account_id});
    let secret = if let Some(id) = &connection.account_id {
        let snapshot: Option<nola_domain::model_connection::ModelAccountSnapshot> =
            execute(store, ModelCommand::GetAccount { id: id.clone() })?;
        let snapshot = snapshot.ok_or("model_account_not_found")?;
        if snapshot.account.state != nola_domain::model_connection::ModelAccountState::Ready {
            return Err("model_account_not_ready".into());
        }
        let reference = snapshot
            .account
            .credential_ref
            .ok_or("model_account_credential_missing")?;
        let record = crate::host_credentials::HostCredentialVault::open(store.path())?
            .record(&format!("{reference}:{}", protocol.as_str()))?
            .ok_or("model_account_credential_missing")?;
        if record.identity
            != crate::host_credentials::identity(&connection.base_url, protocol.as_str())?
        {
            return Err("model_account_credential_route_changed".into());
        }
        binding["account_revision"] = json!(snapshot.revision);
        binding["reference"] = json!(reference);
        binding["credential_revision"] = json!(record.revision);
        Some(record.secret.ok_or("model_account_credential_missing")?)
    } else {
        None
    };
    Ok((
        format!("{:x}", Sha256::digest(binding.to_string().as_bytes())),
        secret,
    ))
}

/// Durable before-send marker prevents UUID replay even after process restart.
/// Unknown outcomes intentionally consume the confirmation; nothing auto-retries.
fn reserve_attempt(store: &NolaStore, confirmation: &ProbeConfirmation) -> Result<(), String> {
    let canonical = uuid::Uuid::parse_str(&confirmation.request_id)
        .map_err(|_| "model_probe_request_id_invalid")?;
    if canonical.to_string() != confirmation.request_id {
        return Err("model_probe_request_id_invalid".into());
    }
    execute(
        store,
        ModelCommand::ReserveModelProbe {
            input: nola_domain::model_connection::ModelProbeAttempt {
                request_id: confirmation.request_id.clone(),
                connection_id: confirmation.connection_id.clone(),
                connection_revision: confirmation.expected_revision,
                account_id: confirmation.account_id.clone(),
                account_revision: confirmation.account_revision,
                credential_binding: confirmation.credential_binding.clone(),
                model_id: confirmation.model_id.clone(),
                protocol: confirmation.protocol,
                started_at_ms: chrono::Utc::now().timestamp_millis(),
                finished_at_ms: None,
                text_confirmed: None,
            },
        },
    )
}
fn execute<T: serde::de::DeserializeOwned>(
    store: &NolaStore,
    command: ModelCommand,
) -> Result<T, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent(store.path())?;
    serde_json::from_value(application.execute(ApplicationCommand::Model { command })?)
        .map_err(|_| "model_probe_invalid_result".into())
}
pub fn prepare(
    store: &NolaStore,
    connection_id: &str,
    expected_revision: i64,
    model_id: &str,
    protocol: ModelProtocol,
) -> Result<ProbeConfirmation, String> {
    prepare_with_credentials(
        store,
        connection_id,
        expected_revision,
        model_id,
        protocol,
        &mut |connection, protocol| credential_binding(store, connection, protocol),
    )
}

pub(crate) fn prepare_with_credentials(
    store: &NolaStore,
    connection_id: &str,
    expected_revision: i64,
    model_id: &str,
    protocol: ModelProtocol,
    resolve: &mut impl FnMut(
        &nola_domain::model_connection::ModelConnection,
        ModelProtocol,
    ) -> Result<(String, Option<String>), String>,
) -> Result<ProbeConfirmation, String> {
    let snapshot: Option<ModelConnectionSnapshot> = execute(
        store,
        ModelCommand::GetConnection {
            id: connection_id.into(),
        },
    )?;
    let snapshot = snapshot.ok_or("model_connection_not_found")?;
    let connection = &snapshot.document.connection;
    if snapshot.revision != expected_revision
        || !connection.enabled
        || !snapshot.document.catalog.as_ref().is_some_and(|catalog| {
            catalog
                .models
                .iter()
                .any(|entry| entry.model_id == model_id && entry.protocol == protocol)
        })
    {
        return Err("model_probe_route_unavailable".into());
    }
    let account_revision = if let Some(id) = &connection.account_id {
        let account: Option<nola_domain::model_connection::ModelAccountSnapshot> =
            execute(store, ModelCommand::GetAccount { id: id.clone() })?;
        Some(account.ok_or("model_account_not_found")?.revision)
    } else {
        None
    };
    Ok(ProbeConfirmation {
        request_id: uuid::Uuid::new_v4().to_string(),
        connection_id: connection_id.into(),
        expected_revision,
        account_id: connection.account_id.clone(),
        account_revision,
        credential_binding: resolve(connection, protocol)?.0,
        model_id: model_id.into(),
        protocol,
        max_requests: 1,
        acknowledge_possible_charges: false,
    })
}

pub fn probe(
    store: &NolaStore,
    confirmation: ProbeConfirmation,
    gateways: &[String],
) -> Result<ModelConnectionSnapshot, String> {
    probe_with_credentials(
        store,
        confirmation,
        gateways,
        &mut |connection, protocol| credential_binding(store, connection, protocol),
    )
}

pub(crate) fn probe_with_credentials(
    store: &NolaStore,
    confirmation: ProbeConfirmation,
    gateways: &[String],
    resolve: &mut impl FnMut(
        &nola_domain::model_connection::ModelConnection,
        ModelProtocol,
    ) -> Result<(String, Option<String>), String>,
) -> Result<ModelConnectionSnapshot, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    if !confirmation.acknowledge_possible_charges || confirmation.max_requests != 1 {
        return Err("model_probe_explicit_cost_confirmation_required".into());
    }
    if ACTIVE
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < 3).then_some(n + 1)
        })
        .is_err()
    {
        return Err("model_probe_busy".into());
    }
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _permit = Permit;
    let snapshot: Option<ModelConnectionSnapshot> = execute(
        store,
        ModelCommand::GetConnection {
            id: confirmation.connection_id.clone(),
        },
    )?;
    let snapshot = snapshot.ok_or("model_connection_not_found")?;
    let connection = &snapshot.document.connection;
    if snapshot.revision != confirmation.expected_revision
        || connection.account_id != confirmation.account_id
    {
        return Err("model_probe_confirmation_stale".into());
    }
    let account_revision = if let Some(id) = &connection.account_id {
        let account: Option<nola_domain::model_connection::ModelAccountSnapshot> =
            execute(store, ModelCommand::GetAccount { id: id.clone() })?;
        Some(account.ok_or("model_account_not_found")?.revision)
    } else {
        None
    };
    if account_revision != confirmation.account_revision {
        return Err("model_probe_account_confirmation_stale".into());
    }
    if !connection.enabled
        || !connection.protocols.contains(&confirmation.protocol)
        || !snapshot.document.catalog.as_ref().is_some_and(|catalog| {
            catalog
                .models
                .iter()
                .any(|m| m.model_id == confirmation.model_id && m.protocol == confirmation.protocol)
        })
    {
        return Err("model_probe_route_unavailable".into());
    }
    if crate::model_connection_host::is_gateway(&connection.base_url, gateways) {
        return Err("nola_gateway_loop".into());
    }
    let operation = crate::model_catalog_discovery::Operation::begin(
        store,
        &confirmation.request_id,
        &confirmation.connection_id,
        confirmation.expected_revision,
    )?;
    let (binding, secret) = resolve(connection, confirmation.protocol)?;
    if binding != confirmation.credential_binding {
        return Err("model_probe_credential_confirmation_stale".into());
    }
    let model = &confirmation.model_id;
    let (suffix, mut body) = match confirmation.protocol {
        ModelProtocol::SiwcResponses => return Err("siwc_probe_not_supported".into()),
        ModelProtocol::Responses => (
            "responses",
            json!({"model":model,"input":"Reply OK.","max_output_tokens":16,"stream":false,"store":false}),
        ),
        ModelProtocol::ChatCompletions => (
            "chat/completions",
            json!({"model":model,"messages":[{"role":"user","content":"Reply OK."}],"max_completion_tokens":16,"stream":false}),
        ),
        ModelProtocol::Anthropic => (
            "messages",
            json!({"model":model,"messages":[{"role":"user","content":"Reply OK."}],"max_tokens":16,"stream":false}),
        ),
    };
    if confirmation.protocol == ModelProtocol::ChatCompletions {
        let declaration = snapshot
            .document
            .catalog
            .as_ref()
            .and_then(|catalog| {
                catalog.models.iter().find(|entry| {
                    entry.model_id == confirmation.model_id
                        && entry.protocol == confirmation.protocol
                })
            })
            .and_then(|entry| entry.execution_capabilities);
        let capabilities = snapshot
            .document
            .capability_assessments
            .as_ref()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.model_id == confirmation.model_id && item.protocol == confirmation.protocol
                })
            })
            .and_then(|item| item.user_override)
            .or(declaration);
        if capabilities.is_some_and(|caps| {
            caps.chat_output_token_field == nola_application::ChatOutputTokenField::MaxTokens
        }) {
            body.as_object_mut()
                .expect("object request")
                .remove("max_completion_tokens");
            body["max_tokens"] = json!(16);
        }
    }
    reserve_attempt(store, &confirmation)?;
    let attempt_id = confirmation.request_id.clone();
    let bytes = operation.probe(
        &format!("{}/{suffix}", connection.base_url.trim_end_matches('/')),
        confirmation.protocol.as_str(),
        secret.as_deref(),
        &body,
    )?;
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| "model_probe_invalid_json")?;
    let text = match confirmation.protocol {
        ModelProtocol::SiwcResponses => return Err("siwc_probe_not_supported".into()),
        ModelProtocol::Responses => response["output"].as_array().is_some_and(|output| {
            output.iter().any(|item| {
                item["content"].as_array().is_some_and(|content| {
                    content.iter().any(|part| {
                        part["type"] == "output_text"
                            && part["text"].as_str().is_some_and(|s| !s.is_empty())
                    })
                })
            })
        }),
        ModelProtocol::ChatCompletions => response["choices"].as_array().is_some_and(|choices| {
            choices.iter().any(|choice| {
                choice["message"]["content"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
            })
        }),
        ModelProtocol::Anthropic => response["content"].as_array().is_some_and(|content| {
            content.iter().any(|part| {
                part["type"] == "text" && part["text"].as_str().is_some_and(|s| !s.is_empty())
            })
        }),
    };
    if !text
        || !response["error"].is_null()
        || response["type"] == "error"
        || matches!(response["status"].as_str(), Some("failed" | "cancelled"))
    {
        return Err("model_probe_text_not_confirmed".into());
    }
    // A credential/account change during the request cannot produce current evidence.
    if resolve(connection, confirmation.protocol)?.0 != binding {
        return Err("model_probe_credential_changed_during_request".into());
    }
    let result = operation.commit(|| {
        execute(
            store,
            ModelCommand::RecordConnectionCapabilityEvidence {
                connection_id: confirmation.connection_id,
                expected_revision: confirmation.expected_revision,
                model_id: confirmation.model_id,
                protocol: confirmation.protocol,
                evidence: ModelCapabilityEvidence {
                    request_id: confirmation.request_id,
                    connection_revision: confirmation.expected_revision,
                    account_revision,
                    credential_binding: binding,
                    observed_at_ms: chrono::Utc::now().timestamp_millis(),
                    requests_sent: 1,
                    text_generation: true,
                },
            },
        )
    });
    if result.is_ok() {
        let _: Result<(), String> = execute(
            store,
            ModelCommand::FinishModelProbe {
                request_id: attempt_id,
                text_confirmed: true,
                finished_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
    }
    result
}
