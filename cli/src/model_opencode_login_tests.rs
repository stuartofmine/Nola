// SPDX-License-Identifier: Apache-2.0
//! Isolated fake public provider API. No source executable or real credentials.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

#[test]
fn model_opencode_login_owner_prompts_callback_and_unknown_are_bound() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("fixture.sqlite")).unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = format!("http://{}", server.server_addr());
    let state = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (s, w, done) = (state.clone(), writes.clone(), stop.clone());
    let worker = std::thread::spawn(move || {
        while !done.load(Ordering::SeqCst) {
            let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap() else {
                continue;
            };
            assert!(request
                .headers()
                .iter()
                .any(|h| h.field.equiv("Authorization") && h.value.as_str().starts_with("Basic ")));
            let path = request.url().split('?').next().unwrap().to_owned();
            assert_ne!(request.method().as_str(), "DELETE");
            let phase = s.load(Ordering::SeqCst);
            let value = match path.as_str() {
                "/doc" => {
                    json!({"openapi":"3.1.0","paths":{"/path":{"get":{}},"/global/health":{"get":{}},"/provider/auth":{"get":{}},"/provider/{providerID}/oauth/authorize":{"post":{}},"/provider/{providerID}/oauth/callback":{"post":{}}}})
                }
                "/global/health" => json!({"healthy":true,"version":"fixture-release"}),
                "/path" => {
                    json!({"home":"/fixture/home","state":"/fixture/state","config":"/fixture/config","worktree":"/fixture/project","directory":if phase == 2 {"/changed"} else {"/fixture/project"}})
                }
                "/provider/auth" => {
                    json!({"fixture":[{"type":"oauth","label":if phase == 3 {"Changed"} else {"Login"},"prompts":[{"type":"select","key":"region","message":"Region","options":[{"label":"Fixture","value":"fixture"}]}]}]})
                }
                "/provider/fixture/oauth/authorize" => {
                    let mut body = String::new();
                    request.as_reader().read_to_string(&mut body).unwrap();
                    let body: Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(body, json!({"method":0,"inputs":{"region":"fixture"}}));
                    w.fetch_add(1, Ordering::SeqCst);
                    json!({"url":"https://example.invalid/fixture","method":"code","instructions":"Fixture only"})
                }
                "/provider/fixture/oauth/callback" => {
                    let mut body = String::new();
                    request.as_reader().read_to_string(&mut body).unwrap();
                    assert_eq!(
                        serde_json::from_str::<Value>(&body).unwrap(),
                        json!({"method":0,"code":"fixture-code"})
                    );
                    w.fetch_add(1, Ordering::SeqCst);
                    if phase == 1 {
                        json!({"error":"fixture failure"})
                    } else {
                        json!(true)
                    }
                }
                _ => panic!("unexpected fixture endpoint"),
            };
            request
                .respond(tiny_http::Response::from_string(value.to_string()))
                .unwrap();
        }
    });
    let connected = connect(
        &store,
        Connect {
            account_id: "opencode-fixture".into(),
            expected_revision: None,
            display_name: "Fixture".into(),
            origin: address,
            directory: "/fixture/project".into(),
            username: "fixture".into(),
            password: "not-a-real-secret".into(),
            acknowledge_local_service_access: true,
        },
    )
    .unwrap();
    assert!(!connected.inference_supported);
    assert_eq!(
        connected.account.account.state,
        ModelAccountState::Unavailable
    );
    let safe = serde_json::to_string(&owners(&store).unwrap()).unwrap();
    assert!(!safe.contains("not-a-real-secret") && !safe.contains("credential_ref"));
    let revision = connected.account.revision;
    let authorize_input = |request_id: String| Authorize {
        account_id: "opencode-fixture".into(),
        expected_revision: revision,
        request_id,
        provider: "fixture".into(),
        method: 0,
        method_binding: connected.methods[0].binding.clone(),
        inputs: BTreeMap::from([("region".into(), "fixture".into())]),
        acknowledge_provider_login_changes: true,
    };
    let callback_input = |request_id: String| Callback {
        account_id: "opencode-fixture".into(),
        expected_revision: revision,
        request_id,
        code: Some("fixture-code".into()),
        acknowledge_provider_login_changes: true,
    };
    let id = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        authorize(&store, authorize_input(id.clone()))
            .unwrap()
            .state,
        "pending"
    );
    assert!(authorize(&store, authorize_input(id.clone())).is_err());
    state.store(3, Ordering::SeqCst);
    assert!(callback(&store, callback_input(id.clone()), false).is_err());
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    state.store(2, Ordering::SeqCst);
    assert!(callback(&store, callback_input(id.clone()), false).is_err());
    state.store(0, Ordering::SeqCst);
    assert_eq!(
        callback(&store, callback_input(id), false).unwrap().state,
        "source_callback_completed"
    );
    let id = uuid::Uuid::new_v4().to_string();
    authorize(&store, authorize_input(id.clone())).unwrap();
    state.store(1, Ordering::SeqCst);
    assert!(callback(&store, callback_input(id.clone()), false).is_err());
    assert!(callback(&store, callback_input(id.clone()), false).is_err());
    assert_eq!(writes.load(Ordering::SeqCst), 4);
    assert_eq!(
        callback(&store, callback_input(id), true).unwrap().state,
        "stopped_locally"
    );
    assert_eq!(writes.load(Ordering::SeqCst), 4);
    stop.store(true, Ordering::SeqCst);
    worker.join().unwrap();
}

#[test]
fn model_opencode_login_rejects_unsafe_origins_paths_and_hidden_inputs() {
    for value in [
        "http://192.0.2.1:4096",
        "http://localhost:4096",
        "http://user:pass@127.0.0.1:4096",
        "http://127.0.0.1:4096/path",
    ] {
        assert!(origin(value).is_err());
    }
    assert!(provider_path("..", "callback").is_err());
    assert_eq!(
        provider_path("a/b", "callback").unwrap(),
        "/provider/a%2Fb/oauth/callback"
    );
    let method: Method = serde_json::from_value(json!({"type":"oauth","label":"Fixture","prompts":[{"type":"text","key":"extra","message":"Extra","when":{"key":"mode","op":"eq","value":"yes"}}]})).unwrap();
    assert!(inputs(&method, &BTreeMap::new()).is_ok());
    assert!(inputs(
        &method,
        &BTreeMap::from([("extra".into(), "hidden".into())])
    )
    .is_err());
}
