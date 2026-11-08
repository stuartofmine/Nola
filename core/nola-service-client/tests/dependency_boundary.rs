// SPDX-License-Identifier: Apache-2.0
//! The SDK's production workspace graph is Protocol -> Domain, without adapters.
use std::{
    collections::{BTreeMap, BTreeSet},
    process::Command,
};

#[test]
fn service_client_does_not_depend_on_application_or_store_implementations() {
    let output = Command::new(env!("CARGO"))
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
        .expect("read Cargo dependency metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|package| (package["name"].as_str().unwrap(), package))
        .collect::<BTreeMap<_, _>>();
    let mut pending = vec!["nola_service_client"];
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        assert!(
            matches!(
                name,
                "nola_service_client" | "nola_protocol" | "nola_domain"
            ),
            "SDK production dependency graph reaches implementation package {name}"
        );
        let package = packages
            .get(name)
            .expect("workspace dependency is in Cargo metadata");
        for dependency in package["dependencies"].as_array().unwrap() {
            // Include build and target-specific dependencies. Cargo returns the
            // actual package name, so aliases cannot hide implementation access.
            if dependency["kind"] == "dev" {
                continue;
            }
            let dependency_name = dependency["name"].as_str().unwrap();
            assert!(
                !matches!(dependency_name, "rusqlite" | "libsqlite3-sys"),
                "{name} introduces SQLite into the SDK production graph"
            );
            if packages.contains_key(dependency_name) {
                if name == "nola_service_client" {
                    assert_eq!(
                        dependency_name, "nola_protocol",
                        "SDK's only direct workspace dependency is Protocol"
                    );
                }
                pending.push(dependency_name);
            }
        }
    }
    assert!(visited.contains("nola_protocol"));
}
