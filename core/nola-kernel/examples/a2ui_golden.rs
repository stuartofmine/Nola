// SPDX-License-Identifier: Apache-2.0
//! Explicit fixture generator; emits only synthetic, deterministic Core output.
#[path = "../tests/support/a2ui_golden.rs"]
mod fixture;
fn main() { println!("{}", serde_json::to_string_pretty(&fixture::generate()).unwrap()); }
