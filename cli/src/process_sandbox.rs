// SPDX-License-Identifier: Apache-2.0
//! Offline-preparable ASRT process adapter, not an authorization or OS attestation.
//! The owning Host must retain the prepared resources until physical settlement.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEPENDENCY: &str = include_str!("../sandbox-runtime/package.json");

/// Produced only by trusted Host composition from already admitted resources.
/// This adapter never discovers roots, credentials, grants or provider endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnthropicProcessPolicy {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub deny_write: Vec<PathBuf>,
    pub allowed_domains: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnthropicInstallation {
    node: PathBuf,
    cli: PathBuf,
    version: String,
    cli_sha256: String,
}

fn exact_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("Sandbox process paths must be exact and absolute".into());
    }
    Ok(())
}

impl AnthropicInstallation {
    /// Read-only capture: no package installation, --version process, OS probe or socket.
    pub(crate) fn capture(node: &Path, package: &Path) -> Result<Self, String> {
        exact_path(node)?;
        exact_path(package)?;
        let read = || -> Result<Self, Box<dyn std::error::Error>> {
            let manifest: Value =
                serde_json::from_slice(&std::fs::read(package.join("package.json"))?)?;
            let dependency: Value = serde_json::from_str(DEPENDENCY)?;
            let version = dependency["dependencies"]["@anthropic-ai/sandbox-runtime"]
                .as_str()
                .ok_or("Missing pinned ASRT dependency")?;
            if manifest["name"] != "@anthropic-ai/sandbox-runtime" || manifest["version"] != version
            {
                return Err("Installed ASRT does not match the pinned dependency".into());
            }
            let node = node.canonicalize()?;
            let cli = package.join("dist/cli.js").canonicalize()?;
            if !node.is_file() || !cli.is_file() {
                return Err("Sandbox programs must be regular files".into());
            }
            let cli_sha256 = format!("{:x}", Sha256::digest(std::fs::read(&cli)?));
            Ok(Self {
                node,
                cli,
                version: version.into(),
                cli_sha256,
            })
        };
        read().map_err(|error| format!("Required Anthropic process backend is unavailable: {error}; no unrestricted fallback"))
    }

    fn verify(&self, writable: &[PathBuf]) -> Result<(), String> {
        let package = self
            .cli
            .parent()
            .and_then(Path::parent)
            .ok_or("Invalid ASRT package root")?;
        if &Self::capture(&self.node, package)? != self {
            return Err("Frozen sandbox installation changed; execution refused".into());
        }
        if writable
            .iter()
            .any(|root| self.node.starts_with(root) || self.cli.starts_with(root))
        {
            return Err("Sandbox installation must be outside writable task roots".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProcessSandboxScope {
    NativeShell,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrozenProcessSandbox {
    pub scope: ProcessSandboxScope,
    pub installation: AnthropicInstallation,
    pub policy: AnthropicProcessPolicy,
}
impl FrozenProcessSandbox {
    pub(crate) fn validate_declaration(&self) -> Result<(), String> {
        if self.policy.environment.keys().any(|key| {
            !matches!(
                key.as_str(),
                "PATH" | "LANG" | "LC_ALL" | "LC_CTYPE" | "TERM"
            )
        }) {
            return Err("Process sandbox environment contains an unsupported variable".into());
        }
        if !self.policy.allowed_domains.is_empty() {
            return Err("Native shell sandbox currently requires network denial; Core network-grant mapping is unavailable".into());
        }
        for root in self
            .policy
            .read_roots
            .iter()
            .chain(&self.policy.write_roots)
            .chain(&self.policy.deny_read)
            .chain(&self.policy.deny_write)
        {
            exact_path(root)?;
        }
        Ok(())
    }
    pub(crate) fn validate_host_storage(&self, roots: &[PathBuf]) -> Result<(), String> {
        self.validate_declaration()?;
        exact_path(&self.installation.node)?;
        exact_path(&self.installation.cli)?;
        if roots.iter().any(|root| {
            let root = root.canonicalize().unwrap_or_else(|_| root.clone());
            self.installation.node.starts_with(&root) || self.installation.cli.starts_with(&root)
        }) {
            return Err("Required sandbox installation is inside an agent-addressable workspace; use trusted Host storage".into());
        }
        Ok(())
    }
    pub(crate) fn require_available(&self) -> Result<(), String> {
        self.validate_declaration()?;
        let roots = self
            .policy
            .write_roots
            .iter()
            .map(|root| root.canonicalize().map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        self.installation.verify(&roots)
    }
    pub(crate) fn for_worktree(
        &self,
        binding: &nola_domain::task_workspace::TaskWorktreeBinding,
    ) -> Result<Self, String> {
        let root = PathBuf::from(&binding.selection.root);
        if !self
            .policy
            .write_roots
            .iter()
            .any(|allowed| root.starts_with(allowed))
        {
            return Err("Sandbox policy does not authorize this execution workspace".into());
        }
        let mut scoped = self.clone();
        scoped.policy.write_roots = vec![root.clone()];
        for readable in &mut scoped.policy.read_roots {
            if root.starts_with(&*readable) {
                *readable = root.clone();
            }
        }
        scoped
            .policy
            .deny_write
            .push(PathBuf::from(&binding.selection.expected_common_directory));
        scoped.policy.deny_write.push(root.join(".git"));
        Ok(scoped)
    }
    pub(crate) fn prepare_shell(
        &self,
        cwd: &Path,
        command: &str,
        read_only: bool,
    ) -> Result<PreparedAnthropicCommand, String> {
        self.require_available()?;
        let mut prepared = prepare_anthropic_command(
            &self.installation,
            &self.policy,
            cwd,
            &[
                "/bin/bash".into(),
                "-o".into(),
                "pipefail".into(),
                if read_only { "-c".into() } else { "-lc".into() },
                crate::tools::shell_evidence_script(command),
            ],
        )?;
        let evidence = prepared
            .home
            .as_ref()
            .ok_or("Missing private sandbox home")?
            .path()
            .join("shell-exit-evidence");
        prepared.command.env("NOLA_SHELL_EVIDENCE_PATH", &evidence);
        prepared.inner_exit_evidence = Some(evidence);
        Ok(prepared)
    }
}

/// Trusted bookkeeping shell outside the backend: never resolve programs or
/// startup hooks from the caller's environment. Preserve only Host-set Git guard.
pub(crate) fn isolate_shell_supervisor_environment(command: &mut Command) {
    let git_guard = command.get_envs().find(|(key, _)| *key == "GIT_OPTIONAL_LOCKS")
        .and_then(|(_, value)| value.map(std::ffi::OsStr::to_os_string));
    command.env_clear().env("PATH", "/usr/bin:/bin").env("LANG", "C");
    if let Some(value) = git_guard { command.env("GIT_OPTIONAL_LOCKS", value); }
}

pub(crate) struct PreparedAnthropicCommand {
    pub command: Command,
    pub policy_sha256: String,
    // Deliberately owned until the caller has reaped/stopped the actual process.
    settings: Option<tempfile::NamedTempFile>,
    home: Option<tempfile::TempDir>,
    dispatched: bool,
    inner_exit_evidence: Option<PathBuf>,
}

impl PreparedAnthropicCommand {
    pub(crate) fn descriptor(&self) -> Value {
        json!({"scope":"native_shell","policy_sha256":self.policy_sha256,
            "settings_path":self.settings.as_ref().map(|file|file.path()),
            "private_home":self.home.as_ref().map(|home|home.path()),
            "dispatch_may_have_occurred":self.dispatched})
    }

    pub(crate) fn exit_evidence(&self, exit_code: i32) -> Option<crate::tools::ShellExitEvidence> {
        crate::tools::read_shell_exit_evidence(self.inner_exit_evidence.as_ref()?, exit_code)
    }

    pub(crate) fn shell_invocation(&self) -> Result<String, String> {
        fn quote(value: &std::ffi::OsStr) -> Result<String, String> {
            let value = value
                .to_str()
                .ok_or("Sandbox argv must be UTF-8 for the trusted shell wrapper")?;
            if value.contains('\0') {
                return Err("Invalid sandbox argv".into());
            }
            Ok(format!("'{}'", value.replace('\'', "'\\''")))
        }
        let mut parts = vec!["/usr/bin/env".to_owned(), "-i".into()];
        for (key, value) in self.command.get_envs() {
            if let Some(value) = value {
                let mut assignment = key.to_os_string();
                assignment.push("=");
                assignment.push(value);
                parts.push(quote(&assignment)?);
            }
        }
        parts.push(quote(self.command.get_program())?);
        for arg in self.command.get_args() {
            parts.push(quote(arg)?);
        }
        Ok(parts.join(" "))
    }
    pub(crate) fn mark_dispatched(&mut self) {
        self.dispatched = true;
    }
    pub(crate) fn settle(&mut self) {
        self.dispatched = false;
    }
}
impl Drop for PreparedAnthropicCommand {
    fn drop(&mut self) {
        if self.dispatched {
            if let Some(settings) = self.settings.take() {
                let _ = settings.keep();
            }
            if let Some(home) = self.home.take() {
                let _ = home.keep();
            }
        }
    }
}

/// Build argv/env for the upstream CLI, without starting it or claiming enforcement.
/// Both short and durable process adapters must use the same frozen policy when wired.
pub(crate) fn prepare_anthropic_command(
    installation: &AnthropicInstallation,
    policy: &AnthropicProcessPolicy,
    cwd: &Path,
    argv: &[String],
) -> Result<PreparedAnthropicCommand, String> {
    exact_path(cwd)?;
    if !matches!(std::env::consts::OS, "macos" | "linux") {
        return Err("Anthropic process integration is not enabled on this platform".into());
    }
    if argv.is_empty() || argv[0].is_empty() || argv.iter().any(|arg| arg.contains('\0')) {
        return Err("Sandbox command requires nonempty argv without NUL".into());
    }
    // An absolute executable cannot be mistaken for an SRT management subcommand.
    exact_path(Path::new(&argv[0]))?;
    for root in policy
        .read_roots
        .iter()
        .chain(&policy.write_roots)
        .chain(&policy.deny_read)
        .chain(&policy.deny_write)
    {
        exact_path(root)?;
    }
    if policy.read_roots.is_empty()
        || policy
            .read_roots
            .iter()
            .chain(&policy.write_roots)
            .any(|root| root.parent().is_none())
    {
        return Err("Sandbox cannot grant the whole filesystem".into());
    }
    if policy.environment.keys().any(|key| {
        !matches!(
            key.as_str(),
            "PATH" | "LANG" | "LC_ALL" | "LC_CTYPE" | "TERM"
        )
    }) {
        return Err(
            "Sandbox environment contains an unsupported variable; credentials are not inherited"
                .into(),
        );
    }
    // Avoid cwd/path aliases weakening the installation/write-root check.
    let readable = policy
        .read_roots
        .iter()
        .map(|root| {
            root.canonicalize()
                .map_err(|_| "Sandbox read roots must exist before preparation".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let writable = policy
        .write_roots
        .iter()
        .map(|root| {
            root.canonicalize()
                .map_err(|_| "Sandbox write roots must exist before preparation".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if readable
        .iter()
        .chain(&writable)
        .any(|root| root.parent().is_none())
    {
        return Err("Sandbox root alias would grant the whole filesystem".into());
    }
    let cwd = cwd.canonicalize().map_err(|error| error.to_string())?;
    if !readable.iter().any(|root| cwd.starts_with(root)) {
        return Err("Sandbox cwd is outside the frozen read roots".into());
    }
    installation.verify(&writable)?;
    let path = policy
        .environment
        .get("PATH")
        .ok_or("Sandbox Host requires an explicit trusted PATH")?;
    for entry in std::env::split_paths(path) {
        exact_path(&entry)?;
        let resolved = entry
            .canonicalize()
            .map_err(|_| "Sandbox PATH directory is unavailable")?;
        if !resolved.is_dir() || writable.iter().any(|root| resolved.starts_with(root)) {
            return Err(
                "Sandbox runtime helpers cannot be resolved from task-writable PATH entries".into(),
            );
        }
    }
    let policy_sha256 = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(installation, policy, &cwd, argv))
                .map_err(|error| error.to_string())?
        )
    );
    let home = tempfile::tempdir().map_err(|error| error.to_string())?;
    let home_path = home
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let literal = |path: &Path| json!({"path": path, "literal": true});
    let settings = json!({
        "network": {"allowedDomains": policy.allowed_domains, "deniedDomains": []},
        "filesystem": {
            "denyRead": std::iter::once(json!("/")).chain(policy.deny_read.iter().map(|p|literal(p))).collect::<Vec<_>>(),
            "allowRead": readable.iter().map(|p|literal(p)).chain([literal(&home_path)]).collect::<Vec<_>>(),
            "allowWrite": writable.iter().map(|p|literal(p)).chain([literal(&home_path)]).collect::<Vec<_>>(),
            "denyWrite": policy.deny_write.iter().map(|p|literal(p)).collect::<Vec<_>>()
        }
    });
    let bytes = serde_json::to_vec(&settings).map_err(|error| error.to_string())?;
    let mut file = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
    let settings_path = file
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if writable.iter().any(|root| settings_path.starts_with(root)) {
        return Err("Sandbox settings cannot be placed under a task-writable root".into());
    }
    std::io::Write::write_all(&mut file, &bytes).map_err(|error| error.to_string())?;
    let mut command = Command::new(&installation.node);
    command
        .arg(&installation.cli)
        .arg("--settings")
        .arg(file.path())
        .arg("--")
        .args(argv)
        .current_dir(cwd)
        .env_clear()
        .envs(&policy.environment)
        .env("HOME", &home_path)
        .env("TMPDIR", &home_path)
        .env("TMP", &home_path)
        .env("TEMP", &home_path);
    Ok(PreparedAnthropicCommand {
        command,
        policy_sha256,
        settings: Some(file),
        home: Some(home),
        dispatched: false,
        inner_exit_evidence: None,
    })
}

#[cfg(test)]
pub(crate) fn unavailable_process_sandbox_fixture(root: &Path) -> FrozenProcessSandbox {
    FrozenProcessSandbox {
        scope: ProcessSandboxScope::NativeShell,
        installation: AnthropicInstallation {
            node: std::env::current_exe().unwrap(),
            cli: root.join("uninstalled/dist/cli.js"),
            version: "0.0.81".into(),
            cli_sha256: "a".repeat(64),
        },
        policy: AnthropicProcessPolicy {
            read_roots: vec![root.to_path_buf()],
            write_roots: vec![root.to_path_buf()],
            deny_read: Vec::new(),
            deny_write: Vec::new(),
            allowed_domains: Vec::new(),
            environment: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn offline_plan_pins_installation_and_keeps_argv_env_and_settings_separate() {
        let mut supervisor = Command::new("/bin/bash");
        supervisor.env("BASH_ENV", "/untrusted/startup").env("LD_PRELOAD", "/untrusted/library")
            .env("GIT_OPTIONAL_LOCKS", "0");
        isolate_shell_supervisor_environment(&mut supervisor);
        assert_eq!(supervisor.get_program(), "/bin/bash");
        let env = supervisor.get_envs().map(|(key,value)| (key.to_str().unwrap(),value.unwrap().to_str().unwrap())).collect::<BTreeMap<_,_>>();
        assert_eq!(env, BTreeMap::from([("PATH","/usr/bin:/bin"),("LANG","C"),("GIT_OPTIONAL_LOCKS","0")]));
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let package = root.join("synthetic-package");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(package.join("dist")).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        // These files model metadata only. No fake or real runtime is executed.
        let dependency: Value = serde_json::from_str(DEPENDENCY).unwrap();
        std::fs::write(
            package.join("package.json"),
            serde_json::to_vec(&json!({
                "name":"@anthropic-ai/sandbox-runtime",
                "version":dependency["dependencies"]["@anthropic-ai/sandbox-runtime"]
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("dist/cli.js"),
            "offline fixture; never execute",
        )
        .unwrap();
        let node = std::env::current_exe().unwrap(); // Identity fixture, not a Node capability probe.
        assert!(AnthropicInstallation::capture(&node, &root.join("absent"))
            .unwrap_err()
            .contains("no unrestricted fallback"));
        let installation = AnthropicInstallation::capture(&node, &package).unwrap();
        let policy = AnthropicProcessPolicy {
            read_roots: vec![workspace.clone()],
            write_roots: vec![workspace.clone()],
            deny_read: vec![root.join("private")],
            deny_write: vec![root.join("shared.git")],
            allowed_domains: Vec::new(),
            environment: BTreeMap::from([
                ("LANG".into(), "C.UTF-8".into()),
                ("PATH".into(), "/usr/bin:/bin".into()),
            ]),
        };
        let argv = vec![
            "/bin/bash".into(),
            "-c".into(),
            "printf '%s' '$not_expanded; literal'".into(),
        ];
        let prepared =
            prepare_anthropic_command(&installation, &policy, &workspace, &argv).unwrap();
        let arguments = prepared
            .command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(arguments[1], "--settings");
        assert_eq!(arguments[3], "--");
        assert_eq!(&arguments[4..], &argv);
        let settings: Value =
            serde_json::from_slice(&std::fs::read(&arguments[2]).unwrap()).unwrap();
        assert_eq!(settings["filesystem"]["denyRead"][0], "/");
        assert_eq!(
            settings["filesystem"]["allowWrite"][0],
            json!({"path": workspace, "literal": true})
        );
        assert_eq!(settings["network"]["allowedDomains"], json!([]));
        assert_eq!(prepared.policy_sha256.len(), 64);
        let again = prepare_anthropic_command(&installation, &policy, &workspace, &argv).unwrap();
        assert_eq!(
            prepared.policy_sha256, again.policy_sha256,
            "private temp paths do not change the frozen policy"
        );
        assert!(prepared.command.get_envs().all(|(key, _)| matches!(
            key.to_str(),
            Some("PATH" | "LANG" | "HOME" | "TMPDIR" | "TMP" | "TEMP")
        )));
        let invocation = prepared.shell_invocation().unwrap();
        assert!(invocation.starts_with("/usr/bin/env -i "));
        assert!(invocation.contains("'$not_expanded; literal'".replace('\'', "'\\''").as_str()));
        let shell = FrozenProcessSandbox {
            scope: ProcessSandboxScope::NativeShell,
            installation: installation.clone(),
            policy: policy.clone(),
        }
        .prepare_shell(&workspace, "printf harmless", false)
        .unwrap();
        let evidence = shell.inner_exit_evidence.as_ref().unwrap();
        std::fs::write(evidence, "141\n141 0\n").unwrap();
        assert_eq!(
            shell.exit_evidence(141).unwrap().pipeline_exit_codes,
            vec![141, 0]
        );
        assert!(shell.exit_evidence(1).is_none());
        #[cfg(unix)]
        {
            std::fs::remove_file(evidence).unwrap();
            let private = root.join("private-evidence-fixture");
            std::fs::write(&private, "141\n141 0\n").unwrap();
            std::os::unix::fs::symlink(&private, evidence).unwrap();
            assert!(
                shell.exit_evidence(141).is_none(),
                "sandbox output cannot redirect the Host reader"
            );
        }
        let mut unknown =
            prepare_anthropic_command(&installation, &policy, &workspace, &argv).unwrap();
        let descriptor = unknown.descriptor();
        unknown.mark_dispatched();
        drop(unknown);
        let retained_settings = PathBuf::from(descriptor["settings_path"].as_str().unwrap());
        let retained_home = PathBuf::from(descriptor["private_home"].as_str().unwrap());
        assert!(
            retained_settings.exists() && retained_home.exists(),
            "unknown stop retains this attempt's resources"
        );
        // Remove only the resources just created by this no-process fixture.
        std::fs::remove_file(retained_settings).unwrap();
        std::fs::remove_dir_all(retained_home).unwrap();
        let mut unsafe_environment = policy.clone();
        unsafe_environment
            .environment
            .insert("OPENAI_API_KEY".into(), "synthetic-do-not-inherit".into());
        assert!(
            prepare_anthropic_command(&installation, &unsafe_environment, &workspace, &argv)
                .err()
                .unwrap()
                .contains("not inherited")
        );
        let mut writable_path = policy.clone();
        writable_path
            .environment
            .insert("PATH".into(), workspace.to_string_lossy().into_owned());
        assert!(
            prepare_anthropic_command(&installation, &writable_path, &workspace, &argv)
                .err()
                .unwrap()
                .contains("task-writable PATH")
        );
        std::fs::write(package.join("dist/cli.js"), "changed runtime fixture").unwrap();
        assert!(prepare_anthropic_command(&installation, &policy, &workspace, &argv).is_err());
    }
}
