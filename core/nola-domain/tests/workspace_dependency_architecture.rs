// SPDX-License-Identifier: Apache-2.0
//! Compile-time-adjacent dependency rules for Architecture V2.
//!
//! These checks intentionally cover only boundaries that have already been
//! cut over. Client-to-Store debt is added here only after its old path is
//! deleted, so this suite never normalizes a temporary allowlist.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nola-domain lives under <workspace>/core")
        .to_path_buf()
}

#[test]
fn native_clients_cannot_construct_a_second_mutable_core_through_the_abi() {
    let root = workspace_root();
    assert!(!root.join("core/src/agent/ffi.rs").exists(),
        "the raw mutable Core C adapter must not be restored as a fallback");
    // These are shipping ABI surfaces, not reducer test fixtures. Pure context
    // and policy functions remain legal; canonical execution uses Application.
    for relative in [
        "core/src/lib.rs",
        "core/src/agent.rs",
        "core/nola-agent-engine/src/agent/mod.rs",
        "core/swift_shims/nola_core.h",
        "core/nola-mobile-bindings/src/lib.rs",
        "apps/ios/Services/AgentCoreRuntime.swift",
        "apps/android/app/src/main/java/com/nola/core/AgentCoreRuntime.kt",
        "apps/android/app/src/main/cpp/nola_core_jni.cpp",
    ] {
        let source = fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("missing protected ABI surface {relative}: {error}"));
        for forbidden in ["nola_core_agent_v2_", "NolaCoreAgentV2", "nativeV2Create",
            "nativeV2Restore", "nativeV2Submit", "pub mod ffi;", "agent::ffi"] {
            assert!(!source.contains(forbidden),
                "{relative} exposes the removed mutable Core path: {forbidden}");
        }
    }
}

fn workspace_metadata() -> &'static Value {
    static METADATA: OnceLock<Value> = OnceLock::new();
    METADATA.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .args([
                "metadata",
                "--no-deps",
                "--offline",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(workspace_root().join("Cargo.toml"))
            .output()
            .expect("execute Cargo metadata for the actual workspace");
        assert!(
            output.status.success(),
            "Cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("valid Cargo metadata")
    })
}

fn workspace_package(relative_manifest: &str) -> &'static Value {
    let metadata = workspace_metadata();
    let manifest = workspace_root().join(relative_manifest);
    metadata["packages"]
        .as_array()
        .expect("workspace packages")
        .iter()
        .find(|package| {
            package["manifest_path"]
                .as_str()
                .is_some_and(|path| Path::new(path) == manifest)
        })
        .unwrap_or_else(|| panic!("{} is absent from Cargo metadata", manifest.display()))
}

fn dependency_names(relative_manifest: &str) -> BTreeSet<String> {
    production_dependency_names(workspace_package(relative_manifest))
}

#[test]
fn native_host_manifest_does_not_require_terminal_frontend_dependencies() {
    let package = workspace_package("apps/cli/Cargo.toml");
    let features = package["features"].as_object().expect("declared features");
    assert!(features.contains_key("native-host"));
    assert_eq!(features["native-host"], serde_json::json!([]),
        "native-host must not activate terminal-ui through a feature alias");
    let terminal = features["terminal-ui"].as_array().expect("terminal-ui feature");
    for name in ["crossterm", "unicode-width", "unicode-segmentation"] {
        let dependency = package["dependencies"].as_array().unwrap().iter()
            .find(|dependency| dependency["name"] == name && dependency["kind"].is_null())
            .unwrap_or_else(|| panic!("missing declared terminal dependency {name}"));
        assert_eq!(dependency["optional"], true, "{name} must not be mandatory for Host");
        assert!(terminal.contains(&Value::String(format!("dep:{name}"))));
    }
    let targets = package["targets"].as_array().unwrap();
    let cli = targets.iter().find(|target| target["name"] == "nola").unwrap();
    assert_eq!(cli["required-features"], serde_json::json!(["terminal-ui"]));
    let host = targets.iter().find(|target| target["name"] == "nola-agent-host").unwrap();
    assert!(host.get("required-features").is_none_or(|value| {
        value.as_array().is_some_and(|features| !features.contains(&Value::String("terminal-ui".into())))
    }));
    // This checks manifest ownership only. The final no-default-features build
    // separately proves that production source does not reference gated UI.
}

fn production_dependency_names(package: &Value) -> BTreeSet<String> {
    package["dependencies"]
        .as_array()
        .expect("package dependencies")
        .iter()
        // Build dependencies also execute in the production build. Target and
        // optional dependencies count even when inactive on this developer's Mac.
        .filter(|dependency| dependency["kind"] != "dev")
        // Cargo's name is the package identity; rename is only a local alias.
        .map(|dependency| {
            dependency["name"]
                .as_str()
                .expect("dependency package name")
                .to_owned()
        })
        .collect()
}

#[test]
fn dependency_rules_cover_renames_targets_optional_and_build_dependencies() {
    let package = serde_json::json!({"dependencies": [
        {"name": "rusqlite", "rename": "innocent_name", "kind": null, "target": "cfg(windows)", "optional": true},
        {"name": "reqwest", "rename": null, "kind": "build", "target": null},
        {"name": "fixture_only", "rename": null, "kind": "dev", "target": null}
    ]});
    assert_eq!(
        production_dependency_names(&package),
        BTreeSet::from(["rusqlite".to_owned(), "reqwest".to_owned()])
    );
}

fn assert_excludes(relative_manifest: &str, forbidden: &[&str]) {
    let dependencies = dependency_names(relative_manifest);
    for name in forbidden {
        assert!(
            !dependencies.contains(*name),
            "{relative_manifest} crosses the Architecture V2 boundary through {name}"
        );
    }
}

fn source_files(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                result.push(path);
            }
        }
    }
    result
}

#[test]
fn domain_is_platform_and_infrastructure_free() {
    assert_excludes(
        "core/nola-domain/Cargo.toml",
        &[
            "nola_core",
            "nola_application",
            "nola_store",
            "nola_runtime_driver",
            "nola_service",
            "nola_service_client",
            "nola_host_protocol",
            "rusqlite",
            "reqwest",
            "tokio",
        ],
    );
}

#[test]
fn kernel_has_no_io_or_application_dependency() {
    assert_excludes(
        "core/nola-kernel/Cargo.toml",
        &[
            "nola_core",
            "nola_application",
            "nola_store",
            "nola_store_port",
            "nola_legacy_agent_adapter",
            "nola_runtime_driver",
            "nola_host_protocol",
            "rusqlite",
            "reqwest",
            "tokio",
        ],
    );
}

#[test]
fn wire_protocol_does_not_depend_on_runtime_implementations() {
    assert_excludes(
        "core/nola-protocol/Cargo.toml",
        &[
            "nola_core",
            "nola_application",
            "nola_store",
            "nola_runtime_driver",
            "nola_service",
            "nola_service_client",
            "nola_host_protocol",
            "rusqlite",
            "reqwest",
        ],
    );
}

#[test]
fn service_client_is_transport_only() {
    assert_excludes(
        "core/nola-service-client/Cargo.toml",
        &[
            "nola_core",
            "nola_application",
            "nola_store",
            "nola_runtime_driver",
            "nola_service",
            "rusqlite",
        ],
    );
}

#[test]
fn store_does_not_depend_on_application_or_runtime() {
    assert_excludes(
        "core/nola-store/Cargo.toml",
        &[
            "nola_core",
            "nola_application",
            "nola_runtime_driver",
            "nola_service",
            "nola_service_client",
            "nola_host_protocol",
            "reqwest",
        ],
    );
}

#[test]
fn update_host_does_not_depend_on_store_application_or_core() {
    assert_excludes(
        "core/nola-update-host/Cargo.toml",
        &["nola_store", "nola_application", "nola_core", "rusqlite"],
    );
}

#[test]
fn runtime_driver_does_not_own_store_or_application() {
    assert_excludes(
        "core/nola-runtime-driver/Cargo.toml",
        &[
            "nola_application",
            "nola_store",
            "nola_service",
            "nola_service_client",
            "rusqlite",
            "reqwest",
        ],
    );
}

#[test]
fn store_port_uses_domain_contracts_without_transport_or_runtime() {
    assert_excludes(
        "core/nola-store-port/Cargo.toml",
        &[
            "nola_host_protocol",
            "nola_core",
            "nola_application",
            "nola_store",
            "nola_runtime_driver",
            "nola_service",
            "rusqlite",
            "reqwest",
            "tokio",
        ],
    );
}

#[test]
fn application_uses_the_independent_agent_engine() {
    let dependencies = dependency_names("core/nola-application/Cargo.toml");
    assert!(
        dependencies.contains("nola_agent_engine"),
        "Application must consume the independently owned reducer implementation"
    );
    for forbidden in ["nola_core", "nola_store", "rusqlite"] {
        assert!(
            !dependencies.contains(forbidden),
            "Application production code must not depend directly on {forbidden}"
        );
    }
}

#[test]
fn runtime_driver_uses_only_opaque_host_transport_and_domain_values() {
    let dependencies = dependency_names("core/nola-runtime-driver/Cargo.toml");
    assert!(
        dependencies.contains("nola_host_protocol"),
        "Runtime Driver must consume the standalone Host transport contract"
    );
    for forbidden in ["nola_core", "nola_legacy_agent_adapter", "nola_application", "nola_store", "rusqlite"] {
        assert!(
            !dependencies.contains(forbidden),
            "Runtime Driver production code must not depend directly on {forbidden}"
        );
    }
}

#[test]
fn agent_engine_has_no_core_application_storage_or_platform_dependency() {
    let dependencies = dependency_names("core/nola-agent-engine/Cargo.toml");
    for forbidden in ["nola_core", "nola_application", "nola_store", "nola_store_port",
        "rusqlite", "tokio", "reqwest", "nola_runtime_driver"] {
        assert!(!dependencies.contains(forbidden), "Agent engine depends on {forbidden}");
    }
    assert!(!workspace_root().join("core/nola-legacy-agent-adapter/Cargo.toml").exists());
}

#[test]
fn new_application_and_runtime_sources_do_not_import_the_old_core_directly() {
    for root in ["core/nola-application/src", "core/nola-runtime-driver/src"] {
        for path in source_files(&workspace_root().join(root), "rs") {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert!(
                !source.contains("nola_core::") && !source.contains("use nola_core"),
                "{} bypasses the independent Agent engine",
                path.display()
            );
            if root == "core/nola-runtime-driver/src" {
                assert!(!source.contains("nola_legacy_agent_adapter"),
                    "{} reintroduces a concrete reducer through the retired Driver seam", path.display());
            }
        }
    }
}

#[test]
fn mac_views_do_not_bypass_the_application_client() {
    let ui_root = workspace_root().join("apps/macos/UI");
    for path in source_files(&ui_root, "swift") {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            !source.contains("MacApplicationPort.shared"),
            "{} bypasses the injected Application Client boundary",
            path.display()
        );
    }
}

#[test]
fn llm_facade_uses_real_module_boundaries_not_textual_inclusion() {
    let root = workspace_root();
    let facade = fs::read_to_string(root.join("apps/cli/src/llm.rs")).unwrap();
    for module in ["streaming", "provider_protocols", "responses"] {
        assert!(
            facade
                .lines()
                .any(|line| line.trim() == format!("mod {module};")),
            "{module} must remain a private implementation module behind the facade"
        );
    }
    for relative in [
        "apps/cli/src/llm.rs",
        "apps/cli/src/llm_parts/streaming.rs",
        "apps/cli/src/llm_parts/provider_protocols.rs",
        "apps/cli/src/llm_parts/responses.rs",
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        let compact: String = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        assert!(
            !compact.contains("include!("),
            "{relative} must not erase module privacy through textual inclusion"
        );
    }
}

#[test]
fn swift_clients_require_the_current_application_service_protocol() {
    let root = workspace_root();
    fn literal(source: &str, declaration: &str) -> u32 {
        let line = source.lines().find(|line| line.trim().starts_with(declaration))
            .unwrap_or_else(|| panic!("missing protocol declaration {declaration}"));
        line.split_once('=').expect("protocol initializer").1.trim()
            .trim_end_matches(';').parse().expect("literal protocol version")
    }
    let protocol = fs::read_to_string(root.join("core/nola-protocol/src/lib.rs")).unwrap();
    let current = literal(&protocol, "pub const APPLICATION_SERVICE_PROTOCOL_VERSION:");
    for (path, declaration) in [
        ("apps/ios/Services/IOSApplicationRuntime.swift", "static let expectedServiceProtocol:"),
        ("apps/macos/Services/MacApplicationServiceClient.swift", "static let protocolVersion ="),
    ] {
        let source = fs::read_to_string(root.join(path)).unwrap();
        assert_eq!(literal(&source, declaration), current,
            "{path} would reject the current runtime before opening its Store");
    }
}
