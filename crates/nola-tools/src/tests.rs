// SPDX-License-Identifier: Apache-2.0
//! Gate 2: Full Tool Projection & Receipt Invariant Suite.
//! Verifies EVERY built-in tool's `ToolModelPayload` fidelity and `VersionTransition` receipts.

use nola_types::{ToolModelPayload, VersionTransition};
use serde_json::json;

use crate::{path_within_workspace, validation_snapshot, SandboxMode, ToolContext, ToolRegistry};

#[test]
fn gate2_all_core_file_search_shell_and_code_tools_project_full_payload_and_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = ToolContext::new(dir.path());
    ctx.sandbox_mode = SandboxMode::Disabled;
    let reg = ToolRegistry::with_builtin_tools();

    // 1. write_file -> Created VersionTransition
    let w = reg.execute(
        "write_file",
        &ctx,
        json!({"path": "hello.py", "content": "def greet():\n    return 'hi'\n"}),
    );
    assert!(!w.is_error);
    assert!(w.has_changed_version());
    assert!(matches!(
        w.effects[0].transition,
        VersionTransition::Created { .. }
    ));

    // 2. read_file -> numbered content preserved verbatim in model_data
    let r = reg.execute("read_file", &ctx, json!({"path": "hello.py"}));
    assert!(!r.is_error);
    let r_val = r.payload.to_model_value();
    assert!(r_val["content"].as_str().unwrap().contains("1: def greet():"));

    // 3. edit_file (no expected_replacements needed!) -> Modified VersionTransition
    let e = reg.execute(
        "edit_file",
        &ctx,
        json!({"path": "hello.py", "old_text": "'hi'", "new_text": "'hello v2'"}),
    );
    assert!(!e.is_error);
    assert!(e.has_changed_version());
    assert!(matches!(
        e.effects[0].transition,
        VersionTransition::Modified { .. }
    ));

    // 4. apply_patch (pure Rust, dry_run defaults to false, zero git required!)
    let patch = "--- a/hello.py\n+++ b/hello.py\n@@ -1,2 +1,2 @@\n def greet():\n-    return 'hello v2'\n+    return 'patched v2'\n";
    let p = reg.execute("apply_patch", &ctx, json!({"patch": patch}));
    assert!(!p.is_error, "patch error: {:?}", p.payload);
    assert!(p.has_changed_version());
    assert!(std::fs::read_to_string(dir.path().join("hello.py"))
        .unwrap()
        .contains("patched v2"));

    // 5. grep -> MUST include full matched lines in model_data (never silently dropped!)
    let g = reg.execute("grep", &ctx, json!({"pattern": "patched v2"}));
    assert!(!g.is_error);
    let g_val = g.payload.to_model_value();
    assert_eq!(g_val["count"], 1);
    assert!(g_val["matches"][0]["text"]
        .as_str()
        .unwrap()
        .contains("patched v2"));

    // 6. glob & list_dir
    let gl = reg.execute("glob", &ctx, json!({"pattern": "*.py"}));
    assert_eq!(gl.payload.to_model_value()["count"], 1);
    let ld = reg.execute("list_dir", &ctx, json!({}));
    assert_eq!(ld.payload.to_model_value()["count"], 1);

    // 7. bash with output_artifacts -> Created VersionTransition!
    let b = reg.execute(
        "bash",
        &ctx,
        json!({
            "command": "echo generated > out.txt",
            "output_artifacts": ["out.txt"]
        }),
    );
    assert!(!b.is_error);
    assert!(b.has_changed_version());
    assert!(matches!(
        b.effects[0].transition,
        VersionTransition::Created { .. }
    ));

    // 8. background bash + bash_control returns full stdout/stderr
    let bg = reg.execute(
        "bash",
        &ctx,
        json!({"command": "echo bg_hello", "background": true}),
    );
    let job_id = bg.payload.to_model_value()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let st = reg.execute(
        "bash_control",
        &ctx,
        json!({"job_id": job_id, "action": "wait", "wait_ms": 2000}),
    );
    assert!(!st.is_error);
    assert!(st.payload.to_model_value()["stdout"]
        .as_str()
        .unwrap()
        .contains("bg_hello"));

    // 9. tool_search returns full schemas in model_data
    let ts = reg.execute("tool_search", &ctx, json!({"query": "web"}));
    assert_eq!(ts.payload.to_model_value()["tools"][0]["name"], "web_fetch");

    // 10. code_exec (QuickJS sandbox)
    let js = reg.execute("code_exec", &ctx, json!({"code": "return 6 * 7;"}));
    assert!(!js.is_error);
    assert_eq!(js.payload.to_model_value()["result"], 42);

    // 11. web_fetch blocks localhost SSRF
    let ssrf = reg.execute("web_fetch", &ctx, json!({"url": "http://127.0.0.1:8080"}));
    assert!(ssrf.is_error);
    assert!(matches!(ssrf.payload, ToolModelPayload::Structured { .. }));

    // 12. delete_file -> Deleted VersionTransition
    let d = reg.execute("delete_file", &ctx, json!({"path": "out.txt"}));
    assert!(!d.is_error);
    assert!(matches!(
        d.effects[0].transition,
        VersionTransition::Deleted { .. }
    ));
}

#[test]
fn symlink_escape_and_validation_snapshot_ignore_pycache() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = dir.path().join("escape_link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    #[cfg(unix)]
    assert!(!path_within_workspace(
        dir.path(),
        &link.join("new_secret.txt")
    ));

    std::fs::write(dir.path().join("main.py"), "x = 1\n").unwrap();
    let snap1 = validation_snapshot(dir.path());
    std::fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
    std::fs::write(dir.path().join("__pycache__/main.cpython-311.pyc"), b"123").unwrap();
    let snap2 = validation_snapshot(dir.path());
    assert_eq!(snap1, snap2, "__pycache__ must not invalidate validation_snapshot");
}
