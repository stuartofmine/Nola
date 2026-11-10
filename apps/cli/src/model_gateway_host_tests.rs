// SPDX-License-Identifier: Apache-2.0
use super::*;

fn profile(endpoint: &str) -> ModelProfile {
    ModelProfile {
        id: "gateway-fixture".into(), display_name: "Fixture".into(), model_name: "upstream-model".into(),
        provider: "openai-compatible".into(), runtime: "remote".into(),
        source_json: json!({"baseURL":endpoint,"endpointID":"fixture-endpoint","wireProtocol":"responses",
            "capabilities":["chat"],"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":false}}).to_string(),
        parameters_json: "{}".into(), selected: true, enabled: true, revision: 1,
        created_at_ms: 1, updated_at_ms: 1, deleted_at_ms: None,
    }
}

fn gateway_once(store_path: std::path::PathBuf) -> (String, std::thread::JoinHandle<()>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", server.server_addr());
    let gateway_origin = origin.clone();
    let worker = std::thread::spawn(move || {
        let store = NolaStore::open(&store_path).unwrap();
        let mut request = server
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let path = request.url().to_owned();
        let response = gateway_response(&mut request, &store, &path, &[gateway_origin]);
        request.respond(response).unwrap();
    });
    (origin, worker)
}

#[test]
fn gateway_catalog_and_direct_target_exclude_owned_siwc_authorization() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    let mut owned = profile("https://api.openai.com/v1");
    let mut source: Value = serde_json::from_str(&owned.source_json).unwrap();
    source["wireProtocol"] = json!("siwc_responses"); source["sourceKind"] = json!("nola_chatgpt");
    owned.source_json = source.to_string();
    assert!(!available(&owned));
    store.upsert_model_profile(owned).unwrap();
    assert!(target_profile(&store,"nola/gateway-fixture/1",ModelProtocol::SiwcResponses).is_err());
    let token = issue_access_token(&store,0).unwrap();
    let (url,worker) = gateway_once(store.path().to_owned());
    let catalog: Value = reqwest::blocking::Client::new().get(format!("{url}/gateway/v1/models"))
        .bearer_auth(token.token).send().unwrap().json().unwrap();
    assert_eq!(catalog["data"],json!([])); worker.join().unwrap();
}

#[test]
fn gateway_token_is_independent_revocable_and_never_in_catalog() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    store
        .upsert_model_profile(profile("https://fixture.invalid/v1"))
        .unwrap();
    let token = issue_access_token(&store, 0).unwrap();
    assert!(issue_access_token(&store, 0).is_err());
    let (url, worker) = gateway_once(store.path().to_owned());
    let response = reqwest::blocking::Client::new()
        .get(format!("{url}/gateway/v1/models"))
        .bearer_auth(&token.token)
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().unwrap();
    assert!(body.contains("nola/gateway-fixture/1"));
    assert!(!body.contains(&token.token));
    worker.join().unwrap();
    revoke_access_token(&store, token.revision).unwrap();
    let (url, worker) = gateway_once(store.path().to_owned());
    let response = reqwest::blocking::Client::new()
        .get(format!("{url}/gateway/v1/models"))
        .bearer_auth(&token.token)
        .send()
        .unwrap();
    assert_eq!(response.status(), 401);
    worker.join().unwrap();
}

#[test]
fn gateway_streams_protocol_bytes_without_executing_tools_or_forwarding_gateway_secret() {
    let upstream = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", upstream.server_addr());
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    store.upsert_model_profile(profile(&endpoint)).unwrap();
    HostCredentialVault::open(store.path())
        .unwrap()
        .write(
            "fixture-endpoint",
            &crate::host_credentials::identity(&endpoint, "responses").unwrap(),
            0,
            Some("fixture-provider-secret"),
            || Ok(()),
        )
        .unwrap();
    let token = issue_access_token(&store, 0).unwrap();
    let stream = "event: response.output_item.added\ndata: {\"type\":\"function_call\",\"name\":\"dangerous_tool\"}\n\nevent: response.completed\ndata: {\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}\n\n";
    let provider = std::thread::spawn(move || {
        let mut request = upstream
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(request.url(), "/v1/responses");
        assert_eq!(
            request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Authorization"))
                .unwrap()
                .value
                .as_str(),
            "Bearer fixture-provider-secret"
        );
        let body: Value = serde_json::from_reader(request.as_reader()).unwrap();
        assert_eq!(body["model"], "upstream-model");
        assert_eq!(body["previous_response_id"], "response-original");
        assert_eq!(body["tools"][0]["name"], "dangerous_tool");
        request
            .respond(
                Response::from_string(stream)
                    .with_header(Header::from_bytes("Content-Type", "text/event-stream").unwrap()),
            )
            .unwrap();
    });
    let (url, worker) = gateway_once(store.path().to_owned());
    let response = reqwest::blocking::Client::new().post(format!("{url}/gateway/v1/responses")).bearer_auth(&token.token)
        .json(&json!({"model":"nola/gateway-fixture/1","stream":true,"input":"fixture","previous_response_id":"response-original","tools":[{"type":"function","name":"dangerous_tool"}]})).send().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().unwrap(), stream);
    worker.join().unwrap();
    provider.join().unwrap();
    let attempts = crate::model_gateway_usage::list_gateway_attempts(&store, 20, 0).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].record.outcome, GatewayAttemptOutcome::Completed);
    assert_eq!(
        attempts[0].record.usage.as_ref().unwrap().input_tokens,
        Some(2)
    );
    assert_eq!(
        attempts[0].record.usage.as_ref().unwrap().output_tokens,
        Some(3)
    );
}

#[test]
fn changed_model_revision_and_protocol_fail_without_upstream_request() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    store
        .upsert_model_profile(profile("https://fixture.invalid/v1"))
        .unwrap();
    assert_eq!(
        target_profile(&store, "nola/gateway-fixture/99", ModelProtocol::Responses)
            .unwrap_err()
            .0,
        409
    );
    assert_eq!(
        target_profile(&store, "nola/gateway-fixture/1", ModelProtocol::Anthropic)
            .unwrap_err()
            .0,
        400
    );
    assert!(parse_model_id("upstream-model").is_err());
}
