// SPDX-License-Identifier: Apache-2.0
//! Explicit operator transport entry, independent of Agent execution/admission.
//! Credentials/control remain in private stdio, never command-line parameters.
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(crate) fn normalize_windows_verbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

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
        || program
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("An absolute transport executable and exact SHA256 are required".into());
    }
    let canonical = program.canonicalize().map_err(|_| {
        "Tailcat transport executable is unavailable; no alternate program was selected"
    })?;
    if !canonical.is_file() {
        return Err("Transport executable must be an exact regular-file path".into());
    }
    let mut file =
        std::fs::File::open(&canonical).map_err(|_| "Cannot verify transport executable")?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).map_err(|_| "Cannot hash transport executable")?;
    if format!("{:x}", hash.finalize()) != expected.to_ascii_lowercase() {
        return Err("Tailcat transport executable changed; launch refused".into());
    }
    let exec_path = normalize_windows_verbatim(&canonical);
    let mut command = Command::new(exec_path);
    command.env_clear();
    #[cfg(not(windows))]
    command.env("PATH", "/usr/bin:/bin");
    #[cfg(windows)]
    {
        let system_root = std::env::var_os("SystemRoot")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows"));
        command
            .env("PATH", r"C:\Windows\System32;C:\Windows")
            .env("SystemRoot", system_root);
        for key in ["TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key).filter(|value| !value.is_empty()) {
                command.env(key, value);
            }
        }
    }
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
            .all(|(key, _)| matches!(key.to_str(), Some("PATH" | "SystemRoot" | "TEMP" | "TMP"))));
        std::fs::write(&program, b"changed").unwrap();
        assert!(prepare(&arguments).is_err());
        assert!(run_if_requested(&["run".into()]).is_none());
    }

    #[test]
    fn windows_verbatim_prefix_is_normalized_and_relative_paths_rejected() {
        assert_eq!(
            normalize_windows_verbatim(Path::new(r"\\?\C:\Nola\bin\nola-tailcat.exe")),
            PathBuf::from(r"C:\Nola\bin\nola-tailcat.exe")
        );
        assert_eq!(
            normalize_windows_verbatim(Path::new(r"\\?\UNC\server\share\nola-tailcat.exe")),
            PathBuf::from(r"\\server\share\nola-tailcat.exe")
        );
        assert_eq!(
            normalize_windows_verbatim(Path::new("/usr/local/bin/nola-tailcat")),
            PathBuf::from("/usr/local/bin/nola-tailcat")
        );
        let dummy_digest = "a".repeat(64);
        assert!(verified_command(Path::new("relative/nola-tailcat"), &dummy_digest).is_err());
        assert!(verified_command(Path::new("/tmp/../bin/nola-tailcat"), &dummy_digest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_absolute_helper_path_is_accepted_when_canonical_digest_matches() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().canonicalize().unwrap().join("real-helper");
        std::fs::write(&target, b"symlinked helper bytes").unwrap();
        let link = root.path().join("link-helper");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let digest = format!("{:x}", Sha256::digest(std::fs::read(&target).unwrap()));
        let command = verified_command(&link, &digest).unwrap();
        assert_eq!(command.get_program(), target);
    }
}
