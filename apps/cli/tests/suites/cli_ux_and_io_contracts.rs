// SPDX-License-Identifier: Apache-2.0
//! End-to-end subprocess contracts for CLI UX, argument routing, help non-mutation,
//! skills/mcp/memory administration, credential updates, and piped non-TTY SSE runs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

fn bounded_run(mut command: Command, stdin_bytes: Option<&[u8]>) -> Output {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if stdin_bytes.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().expect("spawn nola binary");
    if let Some(bytes) = stdin_bytes {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(bytes).expect("write stdin payload");
        }
    }
    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("nola subprocess did not exit within 15s");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    Output {
        status,
        stdout,
        stderr,
    }
}

fn nola_cmd(home: &Path, db_path: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nola"));
    cmd.current_dir(cwd)
        .env("NOLA_HOME", home)
        .env("NOLA_DB_PATH", db_path)
        .env("NOLA_SKIP_ONBOARDING", "1")
        .env("NOLA_CLI_INTERACTION_MODE", "never")
        .env("NO_COLOR", "1");
    cmd
}

#[test]
fn subcommand_help_exits_zero_without_creating_database_or_mutating_state() {
    let _lock = crate::env_lock();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = dir.path().join("unused.sqlite");

    for args in [
        &["--help"][..],
        &["room", "create", "--help"][..],
        &["session", "delete", "--help"][..],
        &["model", "--help"][..],
        &["plugin", "--help"][..],
        &["update", "--help"][..],
        &["memory", "--help"][..],
        &["skills", "--help"][..],
        &["mcp", "--help"][..],
        &["web", "--help"][..],
        &["--lang", "zh", "room", "create", "--help"][..],
    ] {
        let out = bounded_run(
            {
                let mut c = nola_cmd(&home, &db, dir.path());
                c.args(args);
                c
            },
            None,
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?} failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.trim().is_empty(),
            "{args:?} produced empty help stdout"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).trim().is_empty(),
            "{args:?} wrote unexpected stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !db.exists(),
            "{args:?} must not create SQLite database {:?}",
            db
        );
    }
}

#[test]
fn version_and_leading_admin_flags_do_not_require_llm_endpoint() {
    let _lock = crate::env_lock();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = dir.path().join("version-unused.sqlite");

    for args in [
        &["version"][..],
        &["--version"][..],
        &["-V"][..],
        &["--lang", "ko", "-V"][..],
        &["--json", "version"][..],
        &["--version", "--json"][..],
        &["version", "--json"][..],
        &["-V", "--lang=zh", "--json"][..],
    ] {
        let out = bounded_run(
            {
                let mut c = nola_cmd(&home, &db, dir.path());
                c.args(args);
                c
            },
            None,
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?} stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("Nola") || stdout.contains("\"version\""),
            "{args:?} stdout={stdout}"
        );
        if args.contains(&"--json") {
            assert!(serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["version"].is_string());
        }
        assert!(!db.exists(), "{args:?} must not create DB");
    }
}

#[test]
fn management_help_and_errors_stay_scoped_to_the_requested_command() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("unused.sqlite");
    for (args, code, expected) in [
        (&["--model", "session", "model", "--help"][..], 0, "nola model apply-config"),
        (&["--json", "plugin", "desktop-pages", "--help"], 0, "nola plugin desktop-pages"),
        (&["session", "--help"], 0, "nola session sources"),
        (&["model", "connections", "extra"], 2, "nola model --help"),
    ] {
        let mut command = nola_cmd(dir.path(), &db, dir.path());
        command.args(args);
        let out = bounded_run(command, None);
        assert_eq!(out.status.code(), Some(code), "{args:?}");
        let text = String::from_utf8_lossy(if code == 0 { &out.stdout } else { &out.stderr });
        assert!(text.contains(expected), "{text}");
        assert!(!text.contains("nola room"), "{text}");
        assert!(!db.exists());
    }
}

#[test]
fn missing_management_operands_never_mutate_state_and_delete_reports_absence() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("commands.sqlite");
    let invoke = |args: &[&str]| {
        let mut command = nola_cmd(dir.path(), &db, dir.path());
        command.args(args);
        bounded_run(command, None)
    };
    for args in [&["room", "create", "--json"][..], &["session", "delete", "--json"],
        &["plugin", "remove", "--json"], &["room", "send", "room", "--json"],
        &["room", "send", "room", "text", "--to", "--json"], &["model", "discover", "--json"],
        &["model", "chatgpt", "sign-in", "--json"], &["--project", "--json", "sessions"]] {
        assert_eq!(invoke(args).status.code(), Some(2), "{args:?}");
        assert!(!db.exists());
    }
    let absent = invoke(&["session", "delete", "missing", "--json"]);
    assert!(absent.status.success());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&absent.stdout).unwrap()["deleted"], false);
    let room = invoke(&["--json", "room", "create", "--", "--json"]);
    assert!(room.status.success());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&room.stdout).unwrap()["room"]["title"], "--json");
}

#[test]
fn skills_mcp_and_memory_admin_subcommands_persist_across_subprocesses() {
    let _lock = crate::env_lock();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = dir.path().join("admin.sqlite");

    // 1. Skills list + disable + enable
    let list_skills = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["--lang", "zh", "--json", "skills", "list"]);
            c
        },
        None,
    );
    assert_eq!(
        list_skills.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&list_skills.stderr)
    );
    let parsed_skills: serde_json::Value =
        serde_json::from_slice(&list_skills.stdout).expect("skills JSON");
    let first_id = parsed_skills["skills"][0]["id"]
        .as_str()
        .expect("at least one builtin skill")
        .to_string();

    let disable_skill = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["skills", "disable", &first_id]);
            c
        },
        None,
    );
    assert_eq!(disable_skill.status.code(), Some(0));

    let after_disable = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["skills", "list", "--json"]);
            c
        },
        None,
    );
    let after_disable_json: serde_json::Value =
        serde_json::from_slice(&after_disable.stdout).unwrap();
    assert_eq!(
        after_disable_json["skills"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == first_id)
            .unwrap()["enabled"],
        false
    );

    let enable_skill = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["skills", "enable", &first_id]);
            c
        },
        None,
    );
    assert_eq!(enable_skill.status.code(), Some(0));

    // 2. MCP add + list + disable + remove
    let add_mcp = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args([
                "mcp",
                "add",
                "demo-fs",
                "npx",
                "-y",
                "@modelcontextprotocol/server-filesystem",
                "/tmp",
            ]);
            c
        },
        None,
    );
    assert_eq!(
        add_mcp.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&add_mcp.stderr)
    );

    let disable_mcp = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["mcp", "disable", "demo-fs"]);
            c
        },
        None,
    );
    assert_eq!(disable_mcp.status.code(), Some(0));

    let list_mcp = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["--json", "mcp", "list"]);
            c
        },
        None,
    );
    let mcp_json: serde_json::Value = serde_json::from_slice(&list_mcp.stdout).unwrap();
    assert_eq!(mcp_json["servers"][0]["id"], "demo-fs");
    assert_eq!(mcp_json["servers"][0]["enabled"], false);

    let remove_mcp = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["mcp", "remove", "demo-fs"]);
            c
        },
        None,
    );
    assert_eq!(remove_mcp.status.code(), Some(0));

    // 3. Memory add + search + list (global & project scope)
    let mem_add = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args([
                "memory",
                "add",
                "Always keep four-space indentation",
                "--scope",
                "global",
            ]);
            c
        },
        None,
    );
    assert_eq!(
        mem_add.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&mem_add.stderr)
    );

    let mem_proj = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args([
                "memory",
                "add",
                "Workspace uses NolaStore epoch 1",
                "--scope=project",
            ]);
            c
        },
        None,
    );
    assert_eq!(
        mem_proj.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&mem_proj.stderr)
    );

    let mem_search = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["--json", "memory", "search", "epoch 1"]);
            c
        },
        None,
    );
    assert_eq!(mem_search.status.code(), Some(0));
    let search_json: serde_json::Value = serde_json::from_slice(&mem_search.stdout).unwrap();
    assert_eq!(search_json["records"].as_array().unwrap().len(), 1);
    assert_eq!(search_json["records"][0]["scope"], "workspace");
}

#[test]
fn mock_sse_subprocess_verifies_credential_rotation_yes_and_piped_io() {
    let _lock = crate::env_lock();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = dir.path().join("sse-runs.sqlite");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let (tx, rx) = mpsc::channel::<(String, String)>();

    let server = thread::spawn(move || {
        while !stop_flag.load(Ordering::Acquire) {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(err) => panic!("mock SSE accept failed: {err}"),
            };
            stream.set_nonblocking(false).unwrap();
            let _ = stream.set_nodelay(true);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            if request_line.trim().is_empty() {
                continue;
            }
            let mut content_length = 0usize;
            let mut auth_header = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(val) = lower.strip_prefix("content-length:") {
                    content_length = val.trim().parse().unwrap_or(0);
                }
                if lower.starts_with("authorization:") {
                    auth_header = line.trim().to_string();
                }
            }
            if content_length > 0 {
                let mut body = vec![0u8; content_length];
                reader.read_exact(&mut body).unwrap();
            }

            let is_chat = request_line.contains("/chat/completions");
            if is_chat {
                tx.send((request_line.trim().to_string(), auth_header))
                    .unwrap();
                let sse = concat!(
                    "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Mock SSE OK\"},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    sse.len(),
                    sse
                );
                stream.write_all(response.as_bytes()).unwrap();
            } else {
                let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                stream.write_all(response.as_bytes()).unwrap();
            }
            stream.flush().unwrap();
            let _ = stream.shutdown(Shutdown::Write);
            // Wait until the peer reads the response and closes its side so macOS
            // does not emit TCP RST before reqwest finishes reading the SSE body.
            let mut drain = [0u8; 64];
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            while let Ok(read_bytes) = reader.read(&mut drain) {
                if read_bytes == 0 {
                    break;
                }
            }
        }
    });

    let endpoint = format!("http://{addr}/v1");

    // Run 1: explicit -m, -y, piped stdin, first API key (saved session initializes HostCredentialVault)
    let first = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args([
                "--endpoint",
                &endpoint,
                "--api-key",
                "first-key",
                "-m",
                "mock-model",
                "-y",
                "--no-tools",
                "--session",
                "cred-rotate-session",
            ]);
            c
        },
        Some(b"Hello from pipe\n"),
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "first stderr={}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    assert!(
        first_stdout.contains("Mock SSE OK"),
        "expected Mock SSE OK in stdout, got: {first_stdout}"
    );

    // Run 2: second launch updates stored endpoint credential when --api-key differs,
    // and respects --permission-profile cautious over -y.
    let second = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args([
                "--endpoint",
                &endpoint,
                "--api-key",
                "rotated-key",
                "-m",
                "mock-model",
                "-y",
                "--permission-profile",
                "cautious",
                "--no-tools",
                "Second launch prompt",
            ]);
            c
        },
        None,
    );
    assert_eq!(
        second.status.code(),
        Some(0),
        "second stderr={}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        second_stdout.contains("Mock SSE OK"),
        "expected Mock SSE OK in stdout, got: {second_stdout}"
    );

    // Run 3: resume saved session WITHOUT passing --api-key; vault must supply rotated-key.
    let third = bounded_run(
        {
            let mut c = nola_cmd(&home, &db, dir.path());
            c.args(["--session", "cred-rotate-session", "--no-tools", "Third launch"]);
            c
        },
        None,
    );
    assert_eq!(
        third.status.code(),
        Some(0),
        "third stderr={}",
        String::from_utf8_lossy(&third.stderr)
    );
    let third_stdout = String::from_utf8_lossy(&third.stdout);
    assert!(
        third_stdout.contains("Mock SSE OK"),
        "expected Mock SSE OK in stdout, got: {third_stdout}"
    );

    let mut received = Vec::new();
    while let Ok(item) = rx.recv_timeout(Duration::from_millis(500)) {
        received.push(item);
    }
    stop.store(true, Ordering::Release);
    server.join().unwrap();

    assert_eq!(
        received.len(),
        3,
        "expected 3 requests across 3 launches, got: {received:?}"
    );
    assert!(
        received[0].1.to_ascii_lowercase().contains("bearer first-key"),
        "unexpected first auth header: {:?}",
        received[0]
    );
    assert!(
        received[1]
            .1
            .to_ascii_lowercase()
            .contains("bearer rotated-key"),
        "second launch must send rotated-key; got: {:?}",
        received[1]
    );
    assert!(
        received[2]
            .1
            .to_ascii_lowercase()
            .contains("bearer rotated-key"),
        "saved session without --api-key must read updated vault secret; got: {:?}",
        received[2]
    );
}
