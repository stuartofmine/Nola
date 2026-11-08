// SPDX-License-Identifier: Apache-2.0
//! Nola Application Service executable.

use std::path::PathBuf;

struct Arguments {
    database: PathBuf,
    socket: PathBuf,
    parent_pid: Option<u32>,
    recreate_incompatible_store: bool,
    archive_incompatible_store: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "{}",
            serde_json::json!({
                "type": "nola.service.startup_failure",
                "protocol_version": 1,
                "code": error.code,
                "detail": error.detail,
            })
        );
        std::process::exit(1);
    }
}

fn run() -> Result<(), nola_service::ServiceFailure> {
    let raw_arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if raw_arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("Usage: nola-service --database <path> --socket <path> [--parent-pid <pid>] [--recreate-incompatible-store | --archive-incompatible-store <directory>]");
        return Ok(());
    }
    let arguments =
        parse_arguments(raw_arguments).map_err(|detail| nola_service::ServiceFailure {
            code: "invalid_service_arguments",
            detail,
        })?;
    nola_service::serve_with_recovery_and_parent(
        &arguments.database,
        &arguments.socket,
        arguments.recreate_incompatible_store,
        arguments.archive_incompatible_store.as_deref(),
        arguments.parent_pid,
    )
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut database = None;
    let mut socket = None;
    let mut parent_pid = None;
    let mut recreate_incompatible_store = false;
    let mut archive_incompatible_store = None;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--database" => database = arguments.next().map(PathBuf::from),
            "--socket" => socket = arguments.next().map(PathBuf::from),
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
            "--recreate-incompatible-store" => recreate_incompatible_store = true,
            "--archive-incompatible-store" => {
                archive_incompatible_store = arguments.next().map(PathBuf::from)
            }
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    let database = database.ok_or_else(|| "--database is required".to_string())?;
    let socket = socket.ok_or_else(|| "--socket is required".to_string())?;
    if recreate_incompatible_store && archive_incompatible_store.is_some() {
        return Err(
            "--recreate-incompatible-store and --archive-incompatible-store are mutually exclusive"
                .into(),
        );
    }
    Ok(Arguments {
        database,
        socket,
        parent_pid,
        recreate_incompatible_store,
        archive_incompatible_store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_incompatible_store_recreation() {
        let arguments = parse_arguments([
            "--database".to_string(),
            "/tmp/nola.db".to_string(),
            "--socket".to_string(),
            "/tmp/nola.sock".to_string(),
            "--parent-pid".to_string(),
            "1234".to_string(),
            "--recreate-incompatible-store".to_string(),
        ])
        .unwrap();
        assert!(arguments.recreate_incompatible_store);
        assert!(arguments.archive_incompatible_store.is_none());
        assert_eq!(arguments.parent_pid, Some(1234));
    }

    #[test]
    fn parses_explicit_archiving_recovery() {
        let arguments = parse_arguments([
            "--database".to_string(),
            "/tmp/nola.db".to_string(),
            "--socket".to_string(),
            "/tmp/nola.sock".to_string(),
            "--archive-incompatible-store".to_string(),
            "/tmp/recovered-store".to_string(),
        ])
        .unwrap();
        assert!(!arguments.recreate_incompatible_store);
        assert_eq!(
            arguments.archive_incompatible_store.as_deref(),
            Some(std::path::Path::new("/tmp/recovered-store"))
        );
    }

    #[test]
    fn rejects_retired_provider_recovery_argument() {
        assert!(parse_arguments([
            "--database".into(),
            "/tmp/new.db".into(),
            "--socket".into(),
            "/tmp/nola.sock".into(),
            "--recover-provider-configuration-from".into(),
            "/tmp/old.db".into(),
        ])
        .err()
        .unwrap()
        .contains("unknown argument"));
    }
}
