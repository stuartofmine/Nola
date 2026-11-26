// SPDX-License-Identifier: Apache-2.0
//! Entrypoint for the `nola` v2 unified binary.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let debug = args.iter().any(|a| a == "--debug")
        || std::env::var("NOLA_DEBUG")
            .ok()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    match nola_app::cli::run_with_args(&args) {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            if debug {
                eprintln!("nola error [debug]: {err}");
            } else {
                eprintln!("nola error: {err}");
            }
            ExitCode::from(1)
        }
    }
}
