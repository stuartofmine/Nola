// SPDX-License-Identifier: Apache-2.0
//! Entirely isolated HTTP fixtures; never launches or contacts a real Kimi owner.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct Fixture {
    _root: tempfile::TempDir,
    store: NolaStore,
    url: String,
    state: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let state = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, w, done) = (state.clone(), writes.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap()
                else {
                    continue;
                };
                assert!(request
                    .headers()
                    .iter()
                    .any(|header| header.field.equiv("Authorization")
                        && header.value.as_str() == "Bearer fixture-server-only"));
                let method = request.method().as_str().to_owned();
                let path = request.url().to_owned();
                assert!(
                    !path.contains("sessions")
                        && !path.contains("usage")
                        && !path.contains("userinfo")
                        && !path.contains("logout")
                );
                let value = if path == "/openapi.json" {
                    json!({"openapi":"3.1.0","paths":{"/api/v1/meta":{"get":{}},"/api/v1/auth":{"get":{}},"/api/v1/oauth/login":{"get":{},"post":{},"delete":{}}}})
                } else {
                    let state = s.load(Ordering::SeqCst);
                    let data = if path == "/api/v1/meta" {
                        json!({"server_id":if state == 4 {"changed-server"} else {"fixture-server"},"dangerous_bypass_auth":false})
                    } else if path == "/api/v1/auth" {
                        json!({"models_ready":true,"managed_provider":{"name":PROVIDER,"status":"unauthenticated"}})
                    } else if method == "POST" {
                        let mut body = String::new();
                        request.as_reader().read_to_string(&mut body).unwrap();
                        assert_eq!(
                            serde_json::from_str::<Value>(&body).unwrap()["provider"],
                            PROVIDER
                        );
                        w.fetch_add(1, Ordering::SeqCst);
                        s.store(1, Ordering::SeqCst);
                        json!({"flow_id":"fixture-flow","provider":PROVIDER,"status":"pending","verification_uri_complete":"https://auth.kimi.com/fixture","user_code":"FIXTURE","interval":1})
                    } else if method == "DELETE" {
                        w.fetch_add(1, Ordering::SeqCst);
                        s.store(2, Ordering::SeqCst);
                        json!({"cancelled":true,"status":"cancelled"})
                    } else if state == 0 {
                        Value::Null
                    } else {
                        json!({"flow_id":if state == 3 {"other-client-flow"} else {"fixture-flow"},"provider":PROVIDER,
                        "status":if state == 2 {"cancelled"} else {"pending"},"verification_uri_complete":"https://auth.kimi.com/fixture","user_code":"FIXTURE","interval":1})
                    };
                    json!({"code":if state == 5 {40110} else {0},"data":data})
                };
                request
                    .respond(tiny_http::Response::from_string(value.to_string()))
                    .unwrap();
            }
        });
        Self {
            _root: root,
            store,
            url,
            state,
            writes,
            stop,
            worker: Some(worker),
        }
    }
    fn connect(&self) -> Status {
        connect(
            &self.store,
            Connect {
                account_id: "kimi-fixture".into(),
                expected_revision: None,
                display_name: "Fixture Kimi".into(),
                origin: self.url.clone(),
                server_bearer: "fixture-server-only".into(),
                acknowledge_local_service_access: true,
            },
        )
        .unwrap()
    }
    fn request(value: &Status) -> Operation {
        Operation {
            account_id: value.account.account.id.clone(),
            expected_revision: value.account.revision,
            server_id: value.server_id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            flow_id: value.flow_id.clone(),
            region: None,
            acknowledge_provider_login_changes: true,
        }
    }
}
#[test]
fn explicit_owner_login_status_and_provider_cancel_never_enable_inference() {
    let fixture = Fixture::new();
    let connected = fixture.connect();
    assert_eq!(connected.state, "unauthenticated");
    assert_eq!(
        connected.account.account.state,
        ModelAccountState::Unavailable
    );
    let listed = owners(&fixture.store).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!serde_json::to_string(&listed)
        .unwrap()
        .contains("fixture-server-only"));
    let started = operate(&fixture.store, "start", Fixture::request(&connected)).unwrap();
    assert_eq!(started.state, "pending");
    assert!(!started.inference_supported);
    assert!(!serde_json::to_string(&started)
        .unwrap()
        .contains("fixture-server-only"));
    let pending = operate(&fixture.store, "status", Fixture::request(&started)).unwrap();
    assert_eq!(pending.flow_id, started.flow_id);
    let reopened = operate(&fixture.store, "status", Fixture::request(&connected)).unwrap();
    assert_eq!(reopened.flow_id, started.flow_id);
    assert_eq!(fixture.writes.load(Ordering::SeqCst), 1); // observations never cancel
    let mut denied = Fixture::request(&started);
    denied.acknowledge_provider_login_changes = false;
    assert!(operate(&fixture.store, "cancel", denied).is_err());
    assert_eq!(fixture.writes.load(Ordering::SeqCst), 1);
    let cancelled = operate(&fixture.store, "cancel", Fixture::request(&started)).unwrap();
    assert_eq!(cancelled.cancellation_confirmed, Some(true));
    assert_eq!(fixture.writes.load(Ordering::SeqCst), 2);
    assert_eq!(
        current(
            &fixture.store,
            "kimi-fixture",
            Some(connected.account.revision)
        )
        .unwrap()
        .unwrap()
        .account
        .state,
        ModelAccountState::Unavailable
    );
}
#[test]
fn changed_flow_server_or_account_never_cancels_a_replacement() {
    let fixture = Fixture::new();
    let connected = fixture.connect();
    let started = operate(&fixture.store, "start", Fixture::request(&connected)).unwrap();
    assert!(operate(&fixture.store, "start", Fixture::request(&connected)).is_err());
    fixture.state.store(3, Ordering::SeqCst);
    assert!(operate(&fixture.store, "cancel", Fixture::request(&started)).is_err());
    fixture.state.store(4, Ordering::SeqCst);
    assert!(operate(&fixture.store, "cancel", Fixture::request(&started)).is_err());
    fixture.state.store(5, Ordering::SeqCst); // HTTP 200 with error envelope is not success
    assert!(operate(&fixture.store, "status", Fixture::request(&started)).is_err());
    let _: ModelAccountSnapshot = fixture
        .store
        .application()
        .unwrap()
        .execute(ApplicationCommand::Model {
            command: ModelCommand::SaveAccount {
                input: ModelAccount {
                    state: ModelAccountState::Disconnected,
                    ..connected.account.account
                },
                expected_revision: Some(connected.account.revision),
                updated_at_ms: 2,
            },
        })
        .unwrap();
    assert!(operate(&fixture.store, "cancel", Fixture::request(&started)).is_err());
    assert_eq!(fixture.writes.load(Ordering::SeqCst), 1);
}
#[test]
fn endpoint_and_verification_origins_are_not_inferred_or_redirected() {
    for value in [
        "https://api.kimi.com",
        "http://localhost:58627",
        "http://127.0.0.1:58627/path",
        "http://127.0.0.1:58627?token=secret",
        "http://user@127.0.0.1:58627",
    ] {
        assert!(origin(value).is_err());
    }
    assert!(origin("http://127.0.0.1:58627").is_ok());
    assert!(origin("http://[::1]:58627").is_ok());
    assert!(verification_url(&json!("https://auth.kimi.com.evil.invalid/a")).is_err());
    assert!(verification_url(&json!("http://auth.kimi.com/a")).is_err());
}
