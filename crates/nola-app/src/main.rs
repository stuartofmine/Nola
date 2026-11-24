// SPDX-License-Identifier: Apache-2.0
//! Entrypoint for the `nola` v2 unified binary.

use std::process::ExitCode;

fn main() -> ExitCode {
    match nola_app::cli::run_from_env() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("nola error: {err}");
            ExitCode::from(1)
        }
    }
}
