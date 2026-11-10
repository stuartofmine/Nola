// SPDX-License-Identifier: Apache-2.0
//! Hidden same-build Application Service entry for CLI and Agent Host.

use std::path::PathBuf;

const MODE_ARGUMENT: &str = "--nola-application-service";

pub fn run_if_requested() -> Option<i32> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if !arguments.iter().any(|argument| argument == MODE_ARGUMENT) {
        return None;
    }
    Some(match run(arguments) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("nola application service: {error}");
            1
        }
    })
}

fn run(arguments: Vec<String>) -> Result<(), String> {
    let mut database = None;
    let mut socket = None;
    let mut parent_pid = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            MODE_ARGUMENT => {}
            "--database" => {
                database = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--database requires a value".to_string())?,
                ));
            }
            "--socket" => {
                socket = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--socket requires a value".to_string())?,
                ));
            }
            "--parent-pid" => {
                let raw = arguments
                    .next()
                    .ok_or_else(|| "--parent-pid requires a value".to_string())?;
                let parsed = raw
                    .parse::<u32>()
                    .map_err(|_| format!("invalid --parent-pid value: {raw}"))?;
                if parsed == 0 {
                    return Err("--parent-pid must be greater than zero".to_string());
                }
                parent_pid = Some(parsed);
            }
            other => return Err(format!("unknown application service argument: {other}")),
        }
    }
    let database = database.ok_or_else(|| "--database is required".to_string())?;
    let socket = socket.ok_or_else(|| "--socket is required".to_string())?;
    // Automatic service startup must never turn a schema mismatch into data
    // deletion. Explicit reset remains a separate operator action.
    nola_service::serve_with_options_and_parent(&database, &socket, false, parent_pid)
}

#[cfg(test)]
mod tests {
    use super::MODE_ARGUMENT;

    #[test]
    fn application_service_mode_is_explicit() {
        assert_eq!(MODE_ARGUMENT, "--nola-application-service");
    }
}
