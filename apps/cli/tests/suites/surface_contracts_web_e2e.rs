// SPDX-License-Identifier: Apache-2.0
//! Unified real-HTTP Web Host coverage for SPA shell, locale bridge, auth sessions,
//! agent settings, messaging channel worker, data sources, and scheduled automations
//! on a single booted Web Host instance.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Host {
    root: PathBuf,
    process: Option<Child>,
    port: u16,
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(child) = &mut self.process {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Host {
    fn start() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nola-web-surface-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut host = Self {
            root,
            process: None,
            port,
        };
        let web_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("web");
        host.process = Some(
            Command::new(env!("CARGO_BIN_EXE_nola"))
                .args([
                    "web",
                    "--host",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                    "--web-root",
                    web_root.to_str().unwrap(),
                ])
                .current_dir(&host.root)
                .env("NOLA_DB_PATH", host.root.join("fixture.sqlite"))
                .env("NOLA_HOME", &host.root)
                .env("NOLA_CLI_LANG", "en")
                .env("NO_PROXY", "127.0.0.1,localhost")
                .env("no_proxy", "127.0.0.1,localhost")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        for _ in 0..1000 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return host;
            }
            if let Some(status) = host.process.as_mut().unwrap().try_wait().unwrap() {
                panic!("Fixture host exited before binding: {status}");
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("Fixture host did not bind within 30 seconds");
    }

    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Response {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.port,
            body.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(stream, "\r\n{body}").unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let split = bytes
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let headers = std::str::from_utf8(&bytes[..split]).unwrap();
        let payload = &bytes[split + 4..];
        let payload = if method == "HEAD" {
            assert!(payload.is_empty(), "HEAD must not carry a response body");
            Vec::new()
        } else if headers
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            let mut remaining = payload;
            let mut decoded = Vec::new();
            loop {
                let end = remaining
                    .windows(2)
                    .position(|part| part == b"\r\n")
                    .unwrap();
                let length = usize::from_str_radix(
                    std::str::from_utf8(&remaining[..end])
                        .unwrap()
                        .split(';')
                        .next()
                        .unwrap(),
                    16,
                )
                .unwrap();
                if length == 0 {
                    break;
                }
                remaining = &remaining[end + 2..];
                decoded.extend_from_slice(&remaining[..length]);
                assert_eq!(&remaining[length..length + 2], b"\r\n");
                remaining = &remaining[length + 2..];
            }
            decoded
        } else {
            payload.to_vec()
        };
        let body = String::from_utf8(payload)
            .unwrap_or_else(|_| panic!("Non-UTF-8 body from {method} {path}"));
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        Response {
            status,
            headers: headers.into(),
            body,
        }
    }

    fn json(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let payload = if body.is_null() && matches!(method, "GET" | "HEAD" | "DELETE") {
            String::new()
        } else {
            body.to_string()
        };
        let response = self.request(method, path, &[], &payload);
        (response.status, response.json())
    }
}

struct Response {
    status: u16,
    headers: String,
    body: String,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("Invalid JSON ({err}): {}", self.body))
    }

    fn assert_workspace_shell(&self) {
        assert_eq!(
            self.status, 200,
            "Build web/frontend before Web integration acceptance"
        );
        assert!(self
            .headers
            .to_ascii_lowercase()
            .contains("content-type: text/html"));
        assert!(self.body.contains("<div id=\"app\"></div>"));
        assert!(self.body.contains("type=\"module\""));
        assert!(self.body.contains("/ui/assets/"));
        assert!(!self.body.contains("NOLA_UI_LOCALE"));
        assert!(!self.body.contains("{{"));
        assert!(!self.body.contains("{%"));
    }
}

#[test]
fn workspace_shell_locale_settings_data_sources_and_automations_share_live_host() {
    let i18n_unit = Command::new("node")
        .args(["--test", "src/lib/i18n.test.ts"])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/frontend"))
        .output()
        .expect("Node and web/frontend npm ci are required");
    assert!(
        i18n_unit.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&i18n_unit.stdout),
        String::from_utf8_lossy(&i18n_unit.stderr)
    );

    let host = Host::start();

    // 1. Built Svelte SPA shell, JS module asset, and GET/HEAD across all workspace routes.
    let page = host.request("GET", "/chat", &[], "");
    page.assert_workspace_shell();
    let entry = page
        .body
        .split("src=\"")
        .nth(1)
        .and_then(|tail| tail.split('\"').next())
        .expect("compiled entry");
    assert!(entry.starts_with("/ui/assets/") && entry.ends_with(".js"));
    let module = host.request("GET", entry, &[], "");
    assert_eq!(module.status, 200);
    let headers = module.headers.to_ascii_lowercase();
    assert!(
        headers.contains("content-type: text/javascript")
            || headers.contains("content-type: application/javascript"),
        "{headers}"
    );
    assert!(!module.body.is_empty());
    let head = host.request("HEAD", entry, &[], "");
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
    for path in [
        "/models",
        "/model-test",
        "/capabilities",
        "/results",
        "/dashboard",
        "/data-sources",
        "/settings",
        "/scheduled-actions",
        "/login",
    ] {
        let response = host.request("GET", path, &[], "");
        response.assert_workspace_shell();
        assert_eq!(
            response.body, page.body,
            "route {path} must use the same real SPA shell"
        );
        let head_route = host.request("HEAD", path, &[], "");
        assert_eq!(head_route.status, 200, "HEAD {path}");
        assert!(head_route.body.is_empty());
    }

    // 2. Locale preferences, XSS/cookie isolation, mobile bearer token, and web session login/logout.
    for language in ["zh-Hans", "en", "de", "es", "fr", "ja", "ru"] {
        let cookie = format!("unrelated=fixture; nola_ui_locale={language}");
        let localized = host.request(
            "GET",
            "/chat",
            &[
                ("Cookie", &cookie),
                ("Accept-Language", "es-ES;q=0.9,en;q=0.8"),
            ],
            "",
        );
        localized.assert_workspace_shell();
        assert!(!localized
            .headers
            .to_ascii_lowercase()
            .contains("set-cookie: nola_ui_locale="));
    }
    for cookie in [
        "",
        "nola_ui_locale=system",
        "nola_ui_locale=",
        "nola_ui_locale=invalid",
        "nola_ui_locale=en-US",
        "nola_ui_locale=%65n",
        "nola_ui_locale=fr; nola_ui_locale=ja",
        "nola_ui_locale=de; nola_ui_locale=de",
        "nola_ui_locale=<script>fixture</script>",
    ] {
        let response = host.request(
            "GET",
            "/chat",
            &[("Cookie", cookie), ("Accept-Language", "de-DE,de;q=0.9")],
            "",
        );
        response.assert_workspace_shell();
        assert!(!response.body.contains("<script>fixture</script>"));
    }
    host.request("GET", "/chat", &[("Accept-Language", "ja-JP")], "")
        .assert_workspace_shell();

    for cookie in [
        "",
        "nola_ui_locale=fr",
        "nola_web_session=invalid; nola_ui_locale=ru",
    ] {
        assert_eq!(
            host.request("GET", "/api/mobile/me", &[("Cookie", cookie)], "")
                .status,
            401
        );
        let session = host.request("GET", "/api/web/session", &[("Cookie", cookie)], "");
        assert_eq!(session.status, 200);
        assert_eq!(session.json()["sessionAuthenticated"], false);
    }
    let token = std::fs::read_to_string(host.root.join(".nola/web/mobile-access-token")).unwrap();
    let login = host.request(
        "POST",
        "/api/web/login",
        &[("Cookie", "nola_ui_locale=fr")],
        &json!({"token": token.trim()}).to_string(),
    );
    assert_eq!(login.status, 200);
    let session_cookie = login
        .headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| {
            if key.eq_ignore_ascii_case("set-cookie")
                && value.trim().starts_with("nola_web_session=")
            {
                Some(value.trim().split(';').next().unwrap().to_string())
            } else {
                None
            }
        })
        .expect("Fixture login must issue its normal session cookie");
    assert!(login.headers.contains("HttpOnly"));
    assert!(!login.headers.lines().any(|line| line
        .to_ascii_lowercase()
        .starts_with("set-cookie: nola_ui_locale=")));
    let combined = format!("{session_cookie}; nola_ui_locale=ja");
    let session = host.request("GET", "/api/web/session", &[("Cookie", &combined)], "");
    assert_eq!(session.json()["sessionAuthenticated"], true);
    host.request(
        "GET",
        "/chat",
        &[("Cookie", &combined), ("Accept-Language", "en")],
        "",
    )
    .assert_workspace_shell();
    let bearer = format!("Bearer {}", token.trim());
    assert_eq!(
        host.request(
            "GET",
            "/api/mobile/me",
            &[("Authorization", &bearer), ("Cookie", "nola_ui_locale=ru")],
            ""
        )
        .status,
        200
    );
    let logout = host.request("POST", "/api/web/logout", &[("Cookie", &combined)], "{}");
    assert_eq!(logout.status, 200);
    assert!(logout.headers.contains("Max-Age=0"));
    let post_logout = host
        .request("GET", "/api/web/session", &[("Cookie", &combined)], "")
        .json();
    assert_eq!(post_logout["sessionAuthenticated"], false);
    assert_eq!(post_logout["localTrust"], true);
    assert_eq!(post_logout["authenticated"], true);

    // 3. Agent settings, permission profiles, and messaging-channel worker credential isolation.
    let (status, seeded) = host.json(
        "POST",
        "/api/agent/settings",
        json!({"agent_llm_model":"fixture-model","temperature":0.37,"custom_system_prompt":"preserve me"}),
    );
    assert_eq!(status, 200, "{seeded}");
    let (status, seeded) = host.json(
        "POST",
        "/api/agent/settings",
        json!({"mcp_baseline":[],"mcp_servers":[{"server_id":"settings-fixture","name":"Disabled fixture","enabled":false,"endpoint":"http://127.0.0.1:1"}]}),
    );
    assert_eq!(status, 200, "{seeded}");
    assert_eq!(seeded["mcp_servers"].as_array().unwrap().len(), 1);
    for policy in ["cautious", "allow_all"] {
        let (status, settings) = host.json(
            "POST",
            "/api/agent/settings",
            json!({"permission_profile":policy,"allowed_dirs":[&host.root],"bash_prefixes":["git status"],"host_tools_enabled":false}),
        );
        assert_eq!(status, 200, "{settings}");
        assert_eq!(settings["permission_profile"], policy);
        for field in [
            "agent_llm_model",
            "temperature",
            "custom_system_prompt",
            "mcp_servers",
        ] {
            assert_eq!(settings[field], seeded[field], "{field} changed");
        }
        assert_eq!(
            settings["network_policy"],
            if policy == "allow_all" { "allow" } else { "ask" }
        );
    }
    for invalid in ["moderate", "allow-all", "unknown", ""] {
        assert_eq!(
            host.json("POST", "/api/agent/settings", json!({"permission_profile":invalid})).0,
            422
        );
    }
    assert_eq!(
        host.json("POST", "/api/agent/settings", json!({"network_policy":"typo"})).0,
        422
    );
    let (_, settings) = host.json("GET", "/api/agent/settings", Value::Null);
    assert_eq!(settings["permission_profile"], "allow_all");

    let (_, initial_worker) = host.json("GET", "/api/messaging-channel-worker", Value::Null);
    assert_eq!(initial_worker["enabled"], false);
    assert_eq!(initial_worker["has_access_token"], false);
    let worker_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let worker_endpoint = format!("http://{}", worker_listener.local_addr().unwrap());
    let worker_server = std::thread::spawn(move || {
        worker_listener.set_nonblocking(true).unwrap();
        let mut requests = String::new();
        for (status, body) in [
            ("400 Bad Request", "secret-fixture-new-token provider sensitive response"),
            ("200 OK", r#"{"connection":{"id":"fixture","enabled":false}}"#),
        ] {
            let mut accepted = None;
            for _ in 0..1000 {
                if let Ok((stream, _)) = worker_listener.accept() {
                    accepted = Some(stream);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let mut stream = accepted.expect("mock connection request");
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut bytes = [0; 8192];
            let count = stream.read(&mut bytes).unwrap();
            requests.push_str(&String::from_utf8_lossy(&bytes[..count]));
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    let old_worker = "http://127.0.0.1:1";
    assert_eq!(
        host.json(
            "PUT",
            "/api/messaging-channel-worker",
            json!({"enabled":false,"backend_url":old_worker,"access_token":"secret-fixture-old-token","execution_backend":"nola"})
        )
        .0,
        200
    );
    assert_eq!(
        host.json(
            "PUT",
            "/api/messaging-channel-worker",
            json!({"enabled":false,"backend_url":worker_endpoint})
        )
        .0,
        400
    );
    let (_, unchanged_worker) = host.json("GET", "/api/messaging-channel-worker", Value::Null);
    assert_eq!(unchanged_worker["backend_url"], old_worker);
    assert_eq!(
        host.json(
            "PUT",
            "/api/messaging-channel-worker",
            json!({"clear_access_token":true,"access_token":"conflict"})
        )
        .0,
        400
    );
    let (status, updated_worker) = host.json(
        "PUT",
        "/api/messaging-channel-worker",
        json!({"enabled":false,"backend_url":worker_endpoint,"access_token":"secret-fixture-new-token"}),
    );
    assert_eq!(status, 200);
    assert_eq!(updated_worker["has_access_token"], true);
    assert!(updated_worker.get("access_token").is_none());
    let (status, failure) = host.json(
        "PUT",
        "/api/messaging-channels",
        json!({"id":"fixture","provider":"telegram","enabled":false}),
    );
    assert_eq!(status, 400);
    assert!(!failure.to_string().contains("secret-fixture"));
    assert!(!failure.to_string().contains("provider sensitive"));
    let (status, saved_channel) = host.json(
        "PUT",
        "/api/messaging-channels",
        json!({"id":"fixture","provider":"telegram","enabled":false}),
    );
    assert_eq!(status, 200);
    assert_eq!(saved_channel["connection"]["enabled"], false);
    let worker_requests = worker_server.join().unwrap();
    assert!(worker_requests.contains("secret-fixture-new-token"));
    assert!(!worker_requests.contains("secret-fixture-old-token"));
    let (status, cleared_worker) = host.json(
        "PUT",
        "/api/messaging-channel-worker",
        json!({"enabled":false,"clear_access_token":true,"backend_url":"http://127.0.0.1:2"}),
    );
    assert_eq!(status, 200);
    assert_eq!(cleared_worker["has_access_token"], false);
    assert_eq!(host.json("GET", "/api/messaging-channels", Value::Null).0, 400);

    // 4. Data sources CRUD, HTTP connector, conversation opt-in policy, and secret file isolation.
    let records_file = host.root.join("records.json");
    std::fs::write(
        &records_file,
        r#"[{"count":7,"privateValue":"fixture-never-return-content"}]"#,
    )
    .unwrap();
    let (status, empty_sources) = host.json("GET", "/api/data-sources", Value::Null);
    assert_eq!(status, 200);
    assert_eq!(empty_sources["sources"], json!([]));
    assert_eq!(empty_sources["localFirst"], true);
    let (status, usage) = host.json("GET", "/api/data-sources/usage", Value::Null);
    assert_eq!(status, 200);
    assert_eq!(usage["usageRecords"], json!([]));
    let source_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let source_location = format!("http://{}/records", source_listener.local_addr().unwrap());
    source_listener.set_nonblocking(true).unwrap();
    let source_provider = std::thread::spawn(move || {
        for _ in 0..1000 {
            if let Ok((mut stream, _)) = source_listener.accept() {
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                let body = r#"[{"value":1}]"#;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("HTTP source was never requested");
    });
    let (status, http_source) = host.json(
        "POST",
        "/api/data-sources",
        json!({"name":"HTTP fixture","kind":"http","location":source_location,"accessPolicy":"blocked"}),
    );
    source_provider.join().unwrap();
    assert_eq!(status, 201, "{http_source}");
    assert_eq!(
        http_source["source"]["validationContract"]["readRecipe"]["method"],
        "httpGet"
    );
    assert_eq!(
        host.json(
            "DELETE",
            &format!("/api/data-sources/{}", http_source["source"]["id"].as_str().unwrap()),
            Value::Null
        )
        .0,
        200
    );
    let mut source_draft = json!({
        "name":"Fixture",
        "kind":"localFile",
        "location":records_file,
        "accessPolicy":"conversationOptIn",
        "maximumAgeMinutes":60
    });
    let (status, created_source) = host.json("POST", "/api/data-sources", source_draft.clone());
    assert_eq!(status, 201, "{created_source}");
    assert_eq!(created_source["tested"], true);
    assert!(!created_source.to_string().contains("fixture-never-return-content"));
    let data_source_id = created_source["source"]["id"].as_str().unwrap();
    let data_source_path = format!("/api/data-sources/{data_source_id}");
    assert_eq!(
        created_source["source"]["validationContract"]["validationState"],
        "verified"
    );
    let (_, listed_sources) = host.json(
        "GET",
        "/api/data-sources?conversationId=fixture-conversation",
        Value::Null,
    );
    assert_eq!(listed_sources["sources"][0]["source"]["id"], data_source_id);
    assert_eq!(listed_sources["sources"][0]["conversationOverride"], Value::Null);
    assert!(listed_sources["sources"][0]["resolution"].is_object());
    let (status, source_conv) =
        host.json("POST", "/api/chat/conversations", json!({"title":"Scoped source"}));
    assert_eq!(status, 200, "{source_conv}");
    let source_conv_id = source_conv["conversationId"].as_str().unwrap();
    let scoped_path = format!("/api/chat/conversations/{source_conv_id}/data-sources/{data_source_id}");
    let list_path = format!("/api/data-sources?conversationId={source_conv_id}");
    assert_eq!(
        host.json(
            "PATCH",
            &format!("/api/chat/conversations/missing/data-sources/{data_source_id}"),
            json!({"enabled":true})
        )
        .0,
        404
    );
    let (_, inherited) = host.json("GET", &list_path, Value::Null);
    assert_eq!(inherited["sources"][0]["conversationOverride"], Value::Null);
    assert_eq!(
        inherited["sources"][0]["resolution"]["reason"],
        "conversation_opt_in_required"
    );
    let (status, enabled) = host.json("PATCH", &scoped_path, json!({"enabled":true}));
    assert_eq!(status, 200, "{enabled}");
    assert_eq!(enabled["source"]["conversationOverride"], true);
    assert_eq!(enabled["source"]["resolution"]["reason"], "ready");
    let (_, persisted_override) = host.json("GET", &list_path, Value::Null);
    assert_eq!(persisted_override["sources"][0]["conversationOverride"], true);
    let (status, restored) = host.json("PATCH", &scoped_path, json!({"enabled":null}));
    assert_eq!(status, 200, "{restored}");
    assert_eq!(restored["source"]["conversationOverride"], Value::Null);
    assert_eq!(
        restored["source"]["resolution"]["reason"],
        "conversation_opt_in_required"
    );
    for policy in ["defaultEnabled", "askWhenRelevant", "blocked", "conversationOptIn"] {
        source_draft["accessPolicy"] = json!(policy);
        let (status, updated) = host.json("PATCH", &data_source_path, source_draft.clone());
        assert_eq!(status, 200, "{updated}");
        assert_eq!(updated["source"]["accessPolicy"], policy);
        if policy == "blocked" {
            let (_, attempted) = host.json("PATCH", &scoped_path, json!({"enabled":true}));
            assert_eq!(attempted["source"]["resolution"]["reason"], "blocked");
            assert_eq!(attempted["source"]["resolution"]["usable"], false);
            assert_eq!(host.json("PATCH", &scoped_path, json!({"enabled":null})).0, 200);
        }
    }
    let (status, tested) = host.json("POST", &format!("{data_source_path}/test"), json!({}));
    assert_eq!(status, 200, "{tested}");
    assert_eq!(tested["tested"], true);
    for change in [
        json!({"accessPolicy":"allow_all"}),
        json!({"kind":"platformAPI"}),
        json!({"maximumAgeMinutes":-1}),
    ] {
        let mut invalid = source_draft.clone();
        for (key, value) in change.as_object().unwrap() {
            invalid[key] = value.clone();
        }
        assert_eq!(host.json("PATCH", &data_source_path, invalid).0, 400);
    }
    let (_, after_invalid) = host.json("GET", "/api/data-sources", Value::Null);
    assert_eq!(
        after_invalid["sources"][0]["source"]["accessPolicy"],
        "conversationOptIn"
    );
    assert_eq!(
        host.json("PATCH", "/api/data-sources/missing", source_draft.clone()).0,
        400
    );
    assert_eq!(
        host.json("POST", "/api/data-sources/missing/test", json!({})).0,
        400
    );
    assert_eq!(
        host.json("DELETE", "/api/data-sources/missing", Value::Null).0,
        404
    );
    for location in [
        host.root.join("absent").to_string_lossy().into_owned(),
        "https://example.invalid/".into(),
    ] {
        let mut invalid = source_draft.clone();
        invalid["location"] = json!(location);
        let (status, failure) = host.json("POST", "/api/data-sources", invalid);
        assert_eq!(status, 400);
        assert_eq!(failure["tested"], false);
    }
    let (_, after_failures) = host.json("GET", "/api/data-sources", Value::Null);
    assert!(after_failures["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["source"]["validationContract"]["validationState"] == "failed"));
    let mut retry = source_draft.clone();
    retry["id"] = json!("stable-retry-id");
    retry["location"] = json!(host.root.join("missing-retry"));
    assert_eq!(host.json("POST", "/api/data-sources", retry.clone()).0, 400);
    retry["location"] = json!(records_file);
    assert_eq!(host.json("POST", "/api/data-sources", retry).0, 201);
    let (_, after_retry) = host.json("GET", "/api/data-sources", Value::Null);
    let retried = after_retry["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["source"]["id"] == "stable-retry-id")
        .collect::<Vec<_>>();
    assert_eq!(retried.len(), 1);
    assert_eq!(
        retried[0]["source"]["validationContract"]["validationState"],
        "verified"
    );
    let vault = host.root.join(".nola-host-secrets");
    std::fs::create_dir_all(&vault).unwrap();
    let private_secret = vault.join("fixture-secret.txt");
    std::fs::write(&private_secret, "private-fixture-must-not-leak").unwrap();
    let mut forbidden = source_draft.clone();
    forbidden["location"] = json!(private_secret);
    let (status, response) = host.json("POST", "/api/data-sources", forbidden.clone());
    assert_eq!(status, 400);
    assert_eq!(response["tested"], false);
    assert!(!response.to_string().contains("private-fixture-must-not-leak"));
    #[cfg(unix)]
    {
        let alias = host.root.join("innocent.txt");
        std::os::unix::fs::symlink(&private_secret, &alias).unwrap();
        forbidden["location"] = json!(alias);
        assert_eq!(host.json("POST", "/api/data-sources", forbidden).0, 400);
    }
    let (status, deleted_source) = host.json("DELETE", &data_source_path, Value::Null);
    assert_eq!(status, 200);
    assert_eq!(deleted_source, json!({"deleted":true,"id":data_source_id}));
    assert_eq!(host.json("DELETE", &data_source_path, Value::Null).0, 404);
    let (_, remaining_sources) = host.json("GET", "/api/data-sources", Value::Null);
    assert!(remaining_sources["sources"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["source"]["id"] != data_source_id));

    // 5. Scheduled actions / automations CRUD, script approval CAS, and skill admission guard.
    let (_, action_list) = host.json("GET", "/api/scheduled-actions", Value::Null);
    assert_eq!(action_list["executionCapabilities"]["scheduledDispatch"], true);
    assert_eq!(action_list["executionCapabilities"]["promptAutomations"], true);
    assert_eq!(action_list["actions"], json!([]));
    let spec = json!({
        "kind":"agent",
        "spec":{
            "prompt":"Inspect the isolated fixture once",
            "execution_scope":{"kind":"local_workspace","root":&host.root},
            "permission_profile":"cautious",
            "model_selection":"use_current_default_at_admission",
            "context_refs":[],
            "skill_refs":[]
        }
    });
    let action_draft = json!({
        "name":"Fixture script",
        "format":"script",
        "cwd":&host.root,
        "script":"printf should-not-run",
        "frequency":"weekly",
        "weekday":2,
        "time":"09:15",
        "timezone":"local",
        "executionSpec":spec
    });
    for change in [
        json!({"time":"25:00"}),
        json!({"time":"9:00"}),
        json!({"time":"12:99"}),
        json!({"weekday":0}),
        json!({"weekday":8}),
        json!({"weekday":null}),
        json!({"timezone":"UTC"}),
        json!({"frequency":"hourly"}),
        json!({"format":"nola.mac.automation.v1"}),
    ] {
        let mut invalid = action_draft.clone();
        for (key, value) in change.as_object().unwrap() {
            invalid[key] = value.clone();
        }
        assert_eq!(host.json("POST", "/api/scheduled-actions", invalid).0, 400);
    }
    let (status, created_action) = host.json("POST", "/api/scheduled-actions", action_draft.clone());
    assert_eq!(status, 200, "{created_action}");
    let action_id = created_action["action"]["id"].as_str().unwrap();
    let action_path = format!("/api/scheduled-actions/{action_id}");
    assert_eq!(created_action["action"]["enabled"], false);
    assert_eq!(created_action["action"]["scriptApproved"], false);
    assert_eq!(created_action["action"]["timezone"], "local");
    assert_eq!(
        host.json(
            "PATCH",
            &action_path,
            json!({"enabled":true,"expectedRevision":created_action["action"]["revision"]})
        )
        .0,
        409
    );
    assert_eq!(
        host.json("PATCH", &action_path, json!({"enabled":false,"time":"10:00"})).0,
        400
    );
    assert_eq!(
        host.json("POST", &format!("{action_path}/run-now"), json!({})).0,
        400
    );
    assert_eq!(
        host.json(
            "POST",
            &format!("{action_path}/run-now"),
            json!({"occurrenceKey":"disabled-script","expectedRevision":created_action["action"]["revision"]})
        )
        .0,
        409
    );
    assert_eq!(
        host.json("POST", &format!("{action_path}/approve-script"), json!({"confirmSafety":false})).0,
        400
    );
    assert_eq!(
        host.json("POST", &format!("{action_path}/approve-script"), json!({"confirmSafety":true})).0,
        400
    );
    let reviewed_revision = created_action["action"]["revision"].as_i64().unwrap();
    let mut edited_action = action_draft.clone();
    edited_action["time"] = json!("10:30");
    let (_, updated_action) = host.json(
        "PATCH",
        &action_path,
        json!({"definition":edited_action,"expectedRevision":reviewed_revision}),
    );
    let current_revision = updated_action["action"]["revision"].as_i64().unwrap();
    assert!(current_revision > reviewed_revision);
    let (status, stale) = host.json(
        "POST",
        &format!("{action_path}/approve-script"),
        json!({"confirmSafety":true,"expectedRevision":reviewed_revision}),
    );
    assert_eq!(status, 409);
    assert_eq!(stale["code"], "automation_revision_conflict");
    assert_eq!(
        host.json("PATCH", &action_path, json!({"enabled":false,"expectedRevision":reviewed_revision})).0,
        409
    );
    let (status, approved) = host.json(
        "POST",
        &format!("{action_path}/approve-script"),
        json!({"confirmSafety":true,"expectedRevision":current_revision}),
    );
    assert_eq!(status, 200);
    assert_eq!(approved["action"]["scriptApproved"], true);
    assert_eq!(
        host.json(
            "POST",
            &format!("{action_path}/approve-script"),
            json!({"confirmSafety":true,"expectedRevision":current_revision})
        )
        .0,
        409
    );
    assert_eq!(
        host.json(
            "PATCH",
            &action_path,
            json!({"enabled":false,"expectedRevision":approved["action"]["revision"]})
        )
        .0,
        200
    );
    let (_, results) = host.json("GET", "/api/results", Value::Null);
    assert!(!results.to_string().contains("should-not-run"));
    assert_eq!(
        host.json(
            "POST",
            "/api/scheduled-actions/missing/run-now",
            json!({"occurrenceKey":"missing","expectedRevision":1})
        )
        .0,
        404
    );
    assert_eq!(
        host.json("PATCH", "/api/scheduled-actions/missing", json!({"enabled":false,"expectedRevision":1})).0,
        404
    );
    assert_eq!(host.json("DELETE", &action_path, Value::Null).0, 200);
    assert_eq!(host.json("DELETE", &action_path, Value::Null).0, 404);
    assert_eq!(
        host.json("GET", "/api/scheduled-actions", Value::Null).1["actions"],
        json!([])
    );

    let automation_file = host.root.join("source.txt");
    std::fs::write(&automation_file, "isolated automation fixture").unwrap();
    assert_eq!(
        host.json(
            "POST",
            "/api/agent/settings",
            json!({"enable_local_dev_tools":true,"allowed_dirs":[&host.root]})
        )
        .0,
        200
    );
    let mut unavailable = spec;
    unavailable["spec"]["skill_refs"] = json!(["fixture-skill-that-does-not-exist"]);
    let (_, file_action) = host.json(
        "POST",
        "/api/scheduled-actions",
        json!({"name":"Fixture file","format":"file","path":automation_file,"frequency":"daily","time":"23:59","executionSpec":unavailable}),
    );
    let file_action_id = file_action["action"]["id"].as_str().unwrap();
    let (status, run) = host.json(
        "POST",
        &format!("/api/scheduled-actions/{file_action_id}/run-now"),
        json!({"occurrenceKey":"unavailable-skill","expectedRevision":file_action["action"]["revision"]}),
    );
    assert_eq!(status, 409);
    assert_eq!(run["code"], "automation_skill_unavailable", "{run}");
    let (_, definitions) = host.json("GET", "/api/scheduled-actions", Value::Null);
    let definition = definitions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["id"] == file_action_id)
        .unwrap();
    assert_eq!(definition["revision"], file_action["action"]["revision"]);
    assert_eq!(definition["updatedAt"], file_action["action"]["updatedAt"]);
    let (_, results) = host.json("GET", "/api/results", Value::Null);
    assert!(!results["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|result| result["sourceId"] == file_action_id));
}
