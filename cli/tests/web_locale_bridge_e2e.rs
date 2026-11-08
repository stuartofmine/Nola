use serde_json::Value;
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
        // This unique test-owned directory contains only fixture state.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Host {
    fn start() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nola-web-locale-{}-{}",
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
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", self.port, body.len()).unwrap();
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
            // HEAD may advertise GET's Transfer-Encoding but has no chunk
            // stream (not even the zero-length terminator).
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
}

struct Response {
    status: u16,
    headers: String,
    body: String,
}
impl Response {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
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
fn actual_workspace_serves_its_built_svelte_entry_and_assets() {
    let host = Host::start();
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
    }
}

#[test]
fn actual_locale_runtime_obeys_browser_cookie_storage_and_catalog_contracts() {
    let output = Command::new("node")
        .args(["--test", "src/lib/i18n.test.ts"])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/frontend"))
        .output()
        .expect("Node and web/frontend npm ci are required");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn workspace_locale_preferences_do_not_change_authentication_or_inject_shell_markup() {
    let host = Host::start();
    for language in ["zh-Hans", "en", "de", "es", "fr", "ja", "ru"] {
        let cookie = format!("unrelated=fixture; nola_ui_locale={language}");
        for path in ["/chat"] {
            let page = host.request(
                "GET",
                path,
                &[
                    ("Cookie", &cookie),
                    ("Accept-Language", "es-ES;q=0.9,en;q=0.8"),
                ],
                "",
            );
            page.assert_workspace_shell();
            assert!(!page
                .headers
                .to_ascii_lowercase()
                .contains("set-cookie: nola_ui_locale="));
        }
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
    host.request("GET", "/chat", &[], "")
        .assert_workspace_shell();

    // A display preference is never a mobile token or a browser session.
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
        &serde_json::json!({"token": token.trim()}).to_string(),
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
    assert_eq!(
        host.request("POST", "/api/web/logout", &[("Cookie", &combined)], "{}")
            .status,
        200
    );
    assert_eq!(
        host.request("GET", "/api/web/session", &[("Cookie", &combined)], "")
            .json()["sessionAuthenticated"],
        false
    );
}
