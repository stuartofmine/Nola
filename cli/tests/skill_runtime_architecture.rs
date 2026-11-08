// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reject process-global Skill source selection and cached grants.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources(directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read CLI source directory") {
        let entry = entry.expect("CLI source entry");
        let kind = entry.file_type().expect("CLI source entry type");
        let path = entry.path();
        if kind.is_dir() {
            rust_sources(&path, paths);
        } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
}

#[test]
fn skill_source_selection_has_no_process_global_registry_or_configurator() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rust_sources(&root, &mut paths);
    assert!(!paths.is_empty());
    paths.sort();
    for path in paths {
        let source = fs::read_to_string(&path).expect("read CLI Rust source");
        for forbidden in ["CONFIGURED_SKILL_DIRS", "configure_skill_dirs"] {
            assert!(
                !source.contains(forbidden),
                "{} retains {forbidden}; bind the source to the actual runtime instead",
                path.display()
            );
        }
    }
    let runtime = fs::read_to_string(root.join("skill_runtime.rs")).unwrap();
    assert!(
        !runtime.contains("std::env::"),
        "the owned source selection must not read ambient configuration"
    );
    assert!(
        !runtime.contains("OnceLock"),
        "the owned source selection must not be a singleton"
    );
}

#[test]
fn runtime_owned_skill_sources_do_not_cache_execution_manifest_authority() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/parts/contracts_execution.rs"),
    )
    .unwrap();
    let enabled = source
        .split_once("pub(crate) fn host_capability_enabled(")
        .expect("runtime-bound execution availability function")
        .1
        .split_once("struct HostCapabilityAvailability")
        .expect("execution check remains separate from captured availability")
        .0;
    assert!(enabled.contains("HostCapabilityAvailability::capture(&ctx.skills).enabled(name)"));
    let capture = source
        .split_once("fn capture(")
        .expect("availability capture")
        .1
        .split_once("fn from_captured(")
        .expect("pure manifest decoder")
        .0;
    assert!(capture.contains("std::env::var(\"NOLA_HOST_CAPABILITY_MANIFEST\")"));
    let skill = source
        .split_once("pub(crate) fn host_skill_enabled(")
        .expect("exact-id execution check")
        .1
        .split_once("const MAX_LINE_COUNT")
        .unwrap()
        .0;
    assert!(skill.contains("host_capability_enabled(ctx, \"Skill\")"));
    assert!(skill.contains("std::env::var(\"NOLA_HOST_CAPABILITY_MANIFEST\")"));
    assert!(skill.contains("enabled_skill_ids"));
}

#[test]
fn recovered_children_do_not_rebuild_a_managed_catalog_before_execution() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/task_runtime_parts/supervision.rs"),
    )
    .unwrap();
    let helper = source
        .split_once("fn recovered_agent_tool_schemas(")
        .expect("child-owned recovery catalog helper")
        .1
        .split_once("fn spawn_recovered_agent_task(")
        .unwrap()
        .0;
    assert!(helper.contains("child_config.tool_settings.skill_dirs.clone()"));
    assert!(helper.contains("skills.index()?"));
    assert!(helper.contains("tools::tool_schemas_with_skills(&skills)"));
    let spawn = source
        .split_once("fn spawn_recovered_agent_task(")
        .unwrap()
        .1
        .split_once("fn isolate_delegated_child_turn(")
        .unwrap()
        .0;
    assert!(spawn.contains("match recovered_agent_tool_schemas("));
    assert!(!spawn.contains("tools::tool_schemas()"));
    assert!(!spawn.contains("SkillRuntimeContext::default()"));
    let isolation = source
        .split_once("fn isolate_delegated_child_turn(")
        .unwrap()
        .1
        .split_once("#[cfg(test)]")
        .unwrap()
        .0;
    assert!(!isolation.contains("skill_dirs"));
}
