// SPDX-License-Identifier: Apache-2.0
//! Keep canonical SQLite access out of the CLI and Agent Host production crates.

use std::process::Command;

fn workspace_metadata() -> serde_json::Value {
    let metadata = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()
        .expect("read resolved Cargo manifest metadata");
    assert!(
        metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    serde_json::from_slice(&metadata.stdout).unwrap()
}

#[test]
fn canonical_store_dependencies_are_only_available_to_test_fixtures() {
    let metadata = workspace_metadata();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "nola_cli")
        .expect("CLI package");
    // Cargo reports the original package name even for renamed dependencies,
    // and includes target-specific and build dependencies in this collection.
    for dependency in package["dependencies"].as_array().unwrap() {
        if dependency["kind"] == "dev" {
            continue;
        }
        let name = dependency["name"].as_str().unwrap();
        assert!(
            !matches!(name, "nola_store" | "rusqlite" | "libsqlite3-sys"),
            "CLI/Agent Host must submit Application commands; {name} is a direct production dependency"
        );
    }
}

// These are architectural capabilities, not a snapshot of every manifest.
// Only the pure value/decision layers have a complete external allowlist.
// Ports may use serialization helpers but cannot acquire a concrete backend.
fn boundary_violations(metadata: &serde_json::Value) -> Vec<String> {
    let packages = metadata["packages"].as_array().expect("metadata packages");
    let mut errors = Vec::new();
    for (name, allowed_workspace, pure_external) in [
        // UUID validates stable domain identities; the driver uses shared engine Goal contracts.
        // Neither edge grants transport, persistence, or Application dependencies.
        ("nola_domain", &[][..], Some(&["serde", "serde_json", "sha2", "url", "chrono", "uuid"][..])),
        ("nola_kernel", &["nola_domain"][..], Some(&["serde_json", "serde", "jsonschema", "sha2", "regex", "chrono"][..])),
        ("nola_agent_engine", &["nola_domain", "nola_kernel", "nola_host_protocol"][..],
            Some(&["serde", "serde_json", "chrono", "regex", "uuid", "sha2", "hex", "md5", "unicode-segmentation"][..])),
        ("nola_store_port", &["nola_domain"][..], None),
        ("nola_protocol", &["nola_domain"][..], None),
        ("nola_host_protocol", &["nola_domain"][..], None),
        ("nola_measurement", &[][..], Some(&["serde", "serde_json", "sha2", "uuid"][..])),
        ("nola_store", &["nola_domain", "nola_kernel", "nola_store_port", "nola_measurement"][..], None),
        ("nola_application", &["nola_domain", "nola_kernel", "nola_store_port",
            "nola_protocol", "nola_host_protocol", "nola_agent_engine", "nola_measurement"][..], None),
        ("nola_runtime_driver", &["nola_domain", "nola_host_protocol"][..], None),
        ("nola_update_host", &["nola_domain"][..], None),
    ] {
        let Some(package) = packages.iter().find(|package| package["name"] == name) else {
            errors.push(format!("protected package missing: {name}"));
            continue;
        };
        for dependency in package["dependencies"].as_array().expect("dependencies") {
            if dependency["kind"] == "dev" { continue; }
            let target = dependency["name"].as_str().expect("dependency package name");
            let is_workspace = packages.iter().any(|package| package["name"] == target);
            let backend_in_port = matches!(name, "nola_application" | "nola_runtime_driver"
                | "nola_store_port" | "nola_protocol" | "nola_host_protocol" | "nola_update_host")
                && matches!(target, "rusqlite" | "libsqlite3-sys" | "sqlx" | "diesel");
            if (is_workspace && !allowed_workspace.contains(&target))
                || (!is_workspace && pure_external.is_some_and(|allowed| !allowed.contains(&target)))
                || backend_in_port
            {
                errors.push(format!("forbidden production edge: {name} -> {target}"));
            }
        }
    }
    errors
}

#[test]
fn value_decision_ports_and_application_preserve_dependency_direction() {
    let violations = boundary_violations(&workspace_metadata());
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn guard_rejects_renamed_optional_target_and_build_backdoors_but_allows_fixtures() {
    let baseline = workspace_metadata();
    for (source, target, kind) in [
        ("nola_domain", "reqwest", serde_json::Value::Null),
        ("nola_kernel", "nola_application", serde_json::Value::Null),
        ("nola_application", "nola_store", serde_json::json!("build")),
        ("nola_store_port", "rusqlite", serde_json::Value::Null),
        ("nola_store", "nola_application", serde_json::Value::Null),
        ("nola_measurement", "nola_store", serde_json::Value::Null),
        ("nola_domain", "nola_measurement", serde_json::Value::Null),
        ("nola_runtime_driver", "nola_application", serde_json::Value::Null),
        ("nola_runtime_driver", "nola_agent_engine", serde_json::Value::Null),
        ("nola_agent_engine", "nola_core", serde_json::Value::Null),
        ("nola_agent_engine", "nola_store", serde_json::Value::Null),
        ("nola_runtime_driver", "nola_core", serde_json::Value::Null),
        ("nola_update_host", "nola_store", serde_json::Value::Null),
        ("nola_update_host", "rusqlite", serde_json::Value::Null),
    ] {
        let mut changed = baseline.clone();
        let package = changed["packages"].as_array_mut().unwrap().iter_mut()
            .find(|package| package["name"] == source).unwrap();
        package["dependencies"].as_array_mut().unwrap().push(serde_json::json!({
            "name": target, "rename": "innocent_alias", "optional": true,
            "target": "cfg(target_os = \"android\")", "kind": kind,
        }));
        assert!(boundary_violations(&changed).iter().any(|error|
            error == &format!("forbidden production edge: {source} -> {target}")));
        let dependency = changed["packages"].as_array_mut().unwrap().iter_mut()
            .find(|package| package["name"] == source).unwrap()["dependencies"]
            .as_array_mut().unwrap().last_mut().unwrap();
        dependency["kind"] = serde_json::json!("dev");
        assert!(boundary_violations(&changed).is_empty(), "test fixtures remain permitted");
    }
}
