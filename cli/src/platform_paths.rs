use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

pub(crate) fn home_dir() -> Option<PathBuf> {
    #[cfg(test)]
    return Some(std::env::var_os("NOLA_HOME").map(PathBuf::from).unwrap_or_else(|| {
        static TEST_APPLICATION_HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        TEST_APPLICATION_HOME.get_or_init(|| tempfile::tempdir().expect("isolated test application home")).path().to_path_buf()
    }));
    #[cfg(not(test))]
    home_dir_from(
        std::env::var_os("NOLA_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOMEDRIVE"),
        std::env::var_os("HOMEPATH"),
    )
}

/// Resolves the operating-system user home for shell-style paths such as
/// `~/Desktop`. `NOLA_HOME` is intentionally excluded: it identifies Nola's
/// application data root and must never change the meaning of `~`.
pub(crate) fn user_home_dir() -> Option<PathBuf> {
    home_dir_from(
        None,
        std::env::var_os("NOLA_EXTERNAL_EXECUTION_HOME").or_else(|| std::env::var_os("HOME")),
        std::env::var_os("USERPROFILE"),
        std::env::var_os("HOMEDRIVE"),
        std::env::var_os("HOMEPATH"),
    )
}

/// Resolves the one canonical Application Service database identity.
///
/// Clients may use the path to locate the service transport, but must never
/// open the database themselves. Keeping this outside `store` prevents thin
/// clients from depending on the persistence adapter merely to address the
/// Application Service.
pub(crate) fn application_database_path() -> Result<PathBuf, String> {
    application_database_path_from(
        std::env::var_os("NOLA_DB_PATH"),
        std::env::var_os("NOLA_MAC_APP_SUPPORT"),
        home_dir(),
    )
}

fn canonical_database_file_name() -> String {
    format!(
        "nola-v{}.db",
        nola_application_bootstrap::APPLICATION_STORE_SCHEMA_EPOCH
    )
}

pub(crate) fn memory_worker_inbox_directory(session_id: &str) -> Result<PathBuf, String> {
    if session_id.trim().is_empty() {
        return Err("memory worker inbox requires a session identity".to_string());
    }
    let database = application_database_path()?;
    let state_root = database
        .parent()
        .ok_or_else(|| "application database has no parent directory".to_string())?;
    Ok(state_root.join("MemoryWorker").join(session_id))
}

fn application_database_path_from(
    explicit_path: Option<OsString>,
    mac_support_root: Option<OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = non_empty(explicit_path) {
        return Ok(PathBuf::from(path));
    }
    if let Some(root) = non_empty(mac_support_root) {
        return Ok(PathBuf::from(root)
            .join("State")
            .join(canonical_database_file_name()));
    }
    let home = home.ok_or_else(|| "cannot resolve the user home directory".to_string())?;
    Ok(home.join(".nola").join(canonical_database_file_name()))
}

fn home_dir_from(
    nola_home: Option<OsString>,
    home: Option<OsString>,
    user_profile: Option<OsString>,
    home_drive: Option<OsString>,
    home_path: Option<OsString>,
) -> Option<PathBuf> {
    non_empty(nola_home)
        .or_else(|| non_empty(home))
        .or_else(|| non_empty(user_profile))
        .or_else(|| match (non_empty(home_drive), non_empty(home_path)) {
            (Some(drive), Some(path)) => {
                let mut combined = drive;
                combined.push(path);
                Some(combined)
            }
            _ => None,
        })
        .map(PathBuf::from)
}

fn non_empty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty() && value != OsStr::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_nola_home_wins_on_every_platform() {
        let resolved = home_dir_from(
            Some("D:\\NolaData".into()),
            Some("/Users/example".into()),
            Some("C:\\Users\\example".into()),
            None,
            None,
        );
        assert_eq!(resolved, Some(PathBuf::from("D:\\NolaData")));
    }

    #[test]
    fn user_home_does_not_use_nola_application_home() {
        let resolved = home_dir_from(
            None,
            Some("/Users/example".into()),
            Some("C:\\Users\\example".into()),
            None,
            None,
        );
        assert_eq!(resolved, Some(PathBuf::from("/Users/example")));
    }

    #[test]
    fn external_execution_home_has_priority_for_user_paths() {
        let resolved = home_dir_from(
            None,
            Some("/Users/real-user".into()),
            Some("C:\\Users\\fallback".into()),
            None,
            None,
        );
        assert_eq!(resolved, Some(PathBuf::from("/Users/real-user")));
    }

    #[test]
    fn windows_user_profile_is_used_without_home() {
        let resolved = home_dir_from(None, None, Some("C:\\Users\\example".into()), None, None);
        assert_eq!(resolved, Some(PathBuf::from("C:\\Users\\example")));
    }

    #[test]
    fn windows_drive_and_path_are_last_resort() {
        let resolved = home_dir_from(
            None,
            None,
            None,
            Some("C:".into()),
            Some("\\Users\\example".into()),
        );
        assert_eq!(resolved, Some(PathBuf::from("C:\\Users\\example")));
    }

    #[test]
    fn application_database_identity_prefers_explicit_then_mac_support() {
        assert_eq!(
            application_database_path_from(
                Some("/tmp/explicit.db".into()),
                Some("/tmp/Nola".into()),
                Some(PathBuf::from("/Users/example")),
            )
            .unwrap(),
            PathBuf::from("/tmp/explicit.db")
        );
        assert_eq!(
            application_database_path_from(
                None,
                Some("/tmp/Nola".into()),
                Some(PathBuf::from("/Users/example")),
            )
            .unwrap(),
            PathBuf::from(format!(
                "/tmp/Nola/State/nola-v{}.db",
                nola_application_bootstrap::APPLICATION_STORE_SCHEMA_EPOCH
            ))
        );

        assert_eq!(
            application_database_path_from(None, None, Some(PathBuf::from("/Users/example")),)
                .unwrap(),
            PathBuf::from(format!(
                "/Users/example/.nola/nola-v{}.db",
                nola_application_bootstrap::APPLICATION_STORE_SCHEMA_EPOCH
            ))
        );
    }
}

/// Native Bash, never the Windows WSL launcher named bash.exe on PATH.
pub(crate) fn bash_program() -> &'static str {
    #[cfg(windows)]
    { r"C:\Program Files\Git\bin\bash.exe" }
    #[cfg(not(windows))]
    { "/bin/bash" }
}

/// Host-installed executable roots; never consult a repository's PATH.
pub(crate) fn git_program() -> &'static str {
    if cfg!(windows) { "C:/Program Files/Git/cmd/git.exe" } else { "/usr/bin/git" }
}

pub(crate) fn system_utility(name: &str) -> Option<String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return None; }
    #[cfg(windows)]
    let candidates = vec![format!("C:/Program Files/Git/usr/bin/{name}.exe")];
    #[cfg(not(windows))]
    let candidates = vec![format!("/usr/bin/{name}"), format!("/bin/{name}")];
    candidates.into_iter().find(|path| std::path::Path::new(path).is_file())
}

/// MSYS/Git receive ordinary drive/UNC paths, not Win32 verbatim prefixes.
pub(crate) fn shell_path(path: &std::path::Path) -> String {
    let native = path.to_string_lossy();
    #[cfg(windows)]
    let native = native.strip_prefix(r"\\?\UNC\").map(|path| format!("//{path}"))
        .unwrap_or_else(|| native.strip_prefix(r"\\?\").unwrap_or(&native).to_owned());
    #[cfg(windows)]
    { native.replace('\\', "/") }
    #[cfg(not(windows))]
    { native.into_owned() }
}
