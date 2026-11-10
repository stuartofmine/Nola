// SPDX-License-Identifier: Apache-2.0
//! Explicit operator transport entry, independent of Agent execution/admission.
//! Credentials/control remain in private stdio, never command-line parameters.
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

fn prepare(arguments: &[String]) -> Result<Command, String> {
    if arguments.len() != 3 {
        return Err("Usage: nola tailcat-bridge /absolute/nola-tailcat SHA256 (private control over stdin/stdout)".into());
    }
    let program = Path::new(&arguments[1]);
    let expected = &arguments[2];
    if !program.is_absolute()
        || expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("An absolute transport executable and exact SHA256 are required".into());
    }
    verified_command(program, expected)
}

pub(crate) fn verified_command(program: &Path, expected: &str) -> Result<Command, String> {
    if !program.is_absolute()
        || expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("An absolute transport executable and exact SHA256 are required".into());
    }
    let canonical = program.canonicalize().map_err(|_| {
        "Tailcat transport executable is unavailable; no alternate program was selected"
    })?;
    if canonical != program || !canonical.is_file() {
        return Err("Transport executable must be an exact regular-file path".into());
    }
    let mut file =
        std::fs::File::open(&canonical).map_err(|_| "Cannot verify transport executable")?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).map_err(|_| "Cannot hash transport executable")?;
    if format!("{:x}", hash.finalize()) != expected.to_ascii_lowercase() {
        return Err("Tailcat transport executable changed; launch refused".into());
    }
    let mut command = Command::new(canonical);
    command.env_clear();
    #[cfg(not(windows))]
    command.env("PATH", "/usr/bin:/bin");
    #[cfg(windows)]
    command
        .env("PATH", r"C:\Windows\System32;C:\Windows")
        .env("SystemRoot", r"C:\Windows");
    // Inherited pipes are the explicitly selected operator IPC channel. No
    // credentials, default Tailcat keys or account environment is inherited.
    Ok(command)
}

pub(crate) fn run_if_requested(arguments: &[String]) -> Option<Result<(), String>> {
    if arguments.first().map(String::as_str) != Some("tailcat-bridge") {
        return None;
    }
    Some((|| {
        let mut command = prepare(arguments)?;
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Same process/stdio ownership, no background child or detached daemon.
            let _ = command.exec();
            Err("Tailcat transport launch failed; no fallback was started".into())
        }
        #[cfg(not(unix))]
        {
            let status = command
                .status()
                .map_err(|_| "Tailcat transport launch failed")?;
            if status.success() {
                Ok(())
            } else {
                Err("Tailcat transport failed; no Nola Run stop ACK is implied".into())
            }
        }
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_transport_program_and_empty_environment_are_required_without_execution() {
        let root = tempfile::tempdir().unwrap();
        let program = root
            .path()
            .canonicalize()
            .unwrap()
            .join("transport-metadata");
        std::fs::write(&program, b"not an executable; metadata fixture only").unwrap();
        let digest = format!("{:x}", Sha256::digest(std::fs::read(&program).unwrap()));
        let arguments = vec![
            "tailcat-bridge".into(),
            program.to_string_lossy().into_owned(),
            digest,
        ];
        let command = prepare(&arguments).unwrap();
        assert_eq!(command.get_program(), program);
        assert_eq!(command.get_args().count(), 0, "no address or key in argv");
        assert!(command
            .get_envs()
            .all(|(key, _)| matches!(key.to_str(), Some("PATH" | "SystemRoot"))));
        std::fs::write(&program, b"changed").unwrap();
        assert!(prepare(&arguments).is_err());
        assert!(run_if_requested(&["run".into()]).is_none());
    }
}
