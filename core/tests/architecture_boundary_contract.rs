// SPDX-License-Identifier: Apache-2.0

use std::fs;
use std::path::Path;

#[test]
fn deterministic_core_has_no_file_network_process_or_database_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for forbidden in ["reqwest", "rusqlite", "tokio", "directories"] {
        assert!(
            !manifest.lines().any(|line| {
                line.trim_start().starts_with(&format!("{forbidden} "))
                    || line.trim_start().starts_with(&format!("{forbidden}="))
            }),
            "pure Core must not depend on {forbidden}"
        );
    }

    let mut violations = Vec::new();
    visit_rust_sources(&root.join("src"), &mut |path, source| {
        if path.file_name().and_then(|name| name.to_str()) == Some("lib_backup.rs") {
            return;
        }
        for forbidden in [
            "std::fs",
            "std::process::Command",
            "reqwest",
            "rusqlite",
            "TcpStream",
            "UdpSocket",
            "File::open",
            "File::create",
        ] {
            if source.contains(forbidden) {
                violations.push(format!("{} contains {forbidden}", path.display()));
            }
        }
    });
    assert!(
        violations.is_empty(),
        "pure Core contains Host IO:\n{}",
        violations.join("\n")
    );
}

fn visit_rust_sources(directory: &Path, visit: &mut dyn FnMut(&Path, &str)) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            visit_rust_sources(&path, visit);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            let source = fs::read_to_string(&path).unwrap();
            visit(&path, &source);
        }
    }
}
