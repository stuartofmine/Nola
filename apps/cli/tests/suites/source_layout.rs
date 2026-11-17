// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Prevent CLI Rust source files from growing beyond a reviewable size.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_RUST_LINES: usize = 5_000;

#[test]
fn rust_source_files_stay_below_the_line_limit() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut oversized = Vec::new();
    collect_oversized_rust_files(&crate_root.join("src"), &crate_root, &mut oversized);
    collect_oversized_rust_files(&crate_root.join("tests"), &crate_root, &mut oversized);
    oversized.sort();

    assert!(
        oversized.is_empty(),
        "Rust source files must not exceed {MAX_RUST_LINES} lines:\n{}",
        oversized.join("\n")
    );
}

fn collect_oversized_rust_files(directory: &Path, root: &Path, oversized: &mut Vec<String>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("read source directory entry").path();
        if path.is_dir() {
            collect_oversized_rust_files(&path, root, oversized);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let line_count = source.lines().count();
        if line_count > MAX_RUST_LINES {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            oversized.push(format!("{}: {line_count}", relative.display()));
        }
    }
}
