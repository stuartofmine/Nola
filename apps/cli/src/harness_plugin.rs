// SPDX-License-Identifier: Apache-2.0
//! Explicit local management of Pi and DeepSeek Harness plugins using their native CLIs.
pub(crate) mod desktop;
use serde::{Deserialize, Serialize};
use std::{
    env, io,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Harness {
    Pi,
    Dsh,
    DshWeb,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Operation {
    Install(String),
    List,
    Pages,
    Open,
    Remove(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessPluginCommand {
    runtime: Harness,
    operation: Operation,
}

impl Harness {
    pub(crate) fn named(name: &str) -> Option<Self> {
        match name {
            "pi" => Some(Self::Pi),
            "dsh" => Some(Self::Dsh),
            _ => None,
        }
    }

    fn root(&self) -> Result<PathBuf, String> {
        std::path::absolute(crate::platform_paths::application_database_path()?)
            .map(|path| {
                path.with_extension("plugins")
                    .join("harness")
                    .join(self.program())
            })
            .map_err(|error| error.to_string())
    }

    fn profile(&self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Dsh => "tui",
            Self::DshWeb => "web",
        }
    }

    fn with_profile(self, profile: &str) -> Result<Self, String> {
        match (self, profile) {
            (Self::Pi, "" | "pi") => Ok(Self::Pi),
            (Self::Dsh, "" | "tui") => Ok(Self::Dsh),
            (Self::Dsh, "web") => Ok(Self::DshWeb),
            _ => Err("Unsupported runtime profile".into()),
        }
    }

    fn program(&self) -> &str {
        match self {
            Self::Pi => "pi",
            Self::Dsh | Self::DshWeb => "dsh",
        }
    }

    fn package(&self) -> &str {
        match self {
            Self::Pi => "@earendil-works/pi-coding-agent",
            Self::Dsh | Self::DshWeb => "@deepseek-ai/dsh",
        }
    }

    fn command(&self) -> io::Result<Command> {
        // npm's Windows launchers are batch files. Invoke their JS entry with Node
        // directly so a package source is always one OS argument, including on Windows.
        #[cfg(windows)]
        for dir in
            env::split_paths(&env::var_os("PATH").unwrap_or_default()).filter(|p| p.is_absolute())
        {
            let exe = dir.join(format!("{}.exe", self.program()));
            if exe.is_file() {
                return Ok(Command::new(exe));
            }
            if dir.join(format!("{}.cmd", self.program())).is_file() {
                let entry = match self {
                    Self::Pi => "dist/bundle/cli.js",
                    Self::Dsh | Self::DshWeb => "lib/bin.js",
                };
                for modules in [dir.join("node_modules"), dir.join("..")] {
                    let script = modules.join(self.package()).join(entry);
                    if script.is_file() {
                        let node = env::split_paths(&env::var_os("PATH").unwrap_or_default())
                            .filter(|path| path.is_absolute())
                            .map(|path| path.join("node.exe"))
                            .find(|path| path.is_file())
                            .ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::NotFound,
                                    "node.exe missing from PATH",
                                )
                            })?;
                        let mut command = Command::new(node);
                        command.arg(script);
                        return Ok(command);
                    }
                }
            }
        }
        #[cfg(windows)]
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            self.program().to_owned(),
        ));
        #[cfg(not(windows))]
        Ok(Command::new(self.program()))
    }
}

impl HarnessPluginCommand {
    pub(crate) fn parse(
        runtime: Harness,
        mut args: impl Iterator<Item = String>,
    ) -> Result<Self, String> {
        let usage = format!(
            "use nola plugin {} install <source> | list | pages | open | remove <source-or-name>",
            runtime.program()
        );
        let mut action = args.next();
        let runtime = if action.as_deref() == Some("--profile") {
            let profile = args.next().ok_or("Missing runtime profile")?;
            action = args.next();
            runtime.with_profile(&profile)?
        } else {
            runtime
        };
        let operation = match action.as_deref() {
            Some(action @ ("install" | "remove")) => {
                let source = args
                    .next()
                    .filter(|s| {
                        !s.trim().is_empty()
                            && !s.starts_with('-')
                            && !s.chars().any(char::is_control)
                    })
                    .ok_or_else(|| usage.clone())?;
                if action == "install" {
                    Operation::Install(source)
                } else {
                    Operation::Remove(source)
                }
            }
            Some("list") => Operation::List,
            Some("pages") => Operation::Pages,
            Some("open") => Operation::Open,
            _ => return Err(usage),
        };
        if args.next().is_some() {
            return Err(usage);
        }
        Ok(Self { runtime, operation })
    }

    fn arguments(&self, caller: &Path) -> Vec<String> {
        let mut args = Vec::new();
        if self.runtime != Harness::Pi {
            if !matches!(self.operation, Operation::Open) {
                args.push("plugin".into());
            }
            args.extend(["--profile".into(), self.runtime.profile().into()]);
        }
        match &self.operation {
            Operation::Open | Operation::Pages => {}
            Operation::List => {
                args.push("list".into());
                if self.runtime != Harness::Pi {
                    args.extend(["--depth".into(), "0".into()]);
                }
            }
            Operation::Install(source) | Operation::Remove(source) => {
                let verb = match (&self.operation, &self.runtime) {
                    (Operation::Install(_), Harness::Dsh | Harness::DshWeb) => "add",
                    (Operation::Install(_), Harness::Pi) => "install",
                    _ => "remove",
                };
                args.push(verb.into());
                let (prefix, local) = if self.runtime != Harness::Pi {
                    source
                        .strip_prefix("file:")
                        .map(|p| ("file:", p))
                        .or_else(|| source.strip_prefix("link:").map(|p| ("link:", p)))
                        .unwrap_or(("file:", source.as_str()))
                } else {
                    ("", source.as_str())
                };
                let path = Path::new(local);
                // Management runs away from the project's local configuration. Resolve
                // explicit local sources against the calling directory before changing cwd.
                if path.is_absolute()
                    || path.starts_with(".")
                    || path.starts_with("..")
                    || local != source
                {
                    let path = caller.join(path);
                    let path = path.to_string_lossy();
                    args.push(if self.runtime != Harness::Pi {
                        format!("{prefix}{path}")
                    } else {
                        path.into_owned()
                    });
                } else {
                    args.push(source.clone());
                }
            }
        }
        args
    }

    fn prepare(&self, project: &Path) -> Result<Command, String> {
        let caller = env::current_dir().map_err(|error| error.to_string())?;
        let root = self.runtime.root()?;
        let state = root.join("profile");
        let management = root.join("management");
        std::fs::create_dir_all(&management)
            .map_err(|error| format!("cannot create harness working directory: {error}"))?;
        let startup_error = |error: io::Error| {
            if error.kind() == io::ErrorKind::NotFound {
                format!("Could not launch {}. Install {} on this Host, make Node and the runtime available on PATH, and check the project directory.", self.runtime.program(), self.runtime.package())
            } else {
                format!("Could not start {}: {error}", self.runtime.program())
            }
        };
        let mut command = self.runtime.command().map_err(startup_error)?;
        command.env_clear();
        for key in [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "SHELL",
            "USERPROFILE",
            "SYSTEMROOT",
            "COMSPEC",
            "PATHEXT",
            "APPDATA",
            "LOCALAPPDATA",
            "TEMP",
            "TMP",
            "TMPDIR",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "TERM",
            "COLORTERM",
            "NO_COLOR",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "SSL_CERT_FILE",
            "NODE_EXTRA_CA_CERTS",
        ] {
            if let Some(value) = env::var_os(key) {
                command.env(key, value);
            }
        }
        let profile_key = match self.runtime {
            Harness::Pi => "PI_CODING_AGENT_DIR",
            Harness::Dsh | Harness::DshWeb => "DSH_HOME",
        };
        command.env(profile_key, state);
        if self.runtime != Harness::Pi {
            command.env("DSH_AGENTS_HOME", root.join("agents"));
        }
        let cwd = if matches!(self.operation, Operation::Open) {
            project
        } else {
            &management
        };
        command.current_dir(cwd).args(self.arguments(&caller));
        Ok(command)
    }

    pub(crate) fn run(&self, project: &Path) -> Result<(), String> {
        if self.operation == Operation::Pages {
            let packages = installed(&self.runtime)?;
            let catalog = pages(&self.runtime, &packages, "", true)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&catalog).map_err(|error| error.to_string())?
            );
            return Ok(());
        }
        let status = self.prepare(project)?.status().map_err(|error| {
            format!(
                "Could not launch {}. Install {} on this Host and check PATH: {error}",
                self.runtime.program(),
                self.runtime.package()
            )
        })?;
        if !status.success() {
            return Err(format!("{} exited with {status}", self.runtime.program()));
        }
        Ok(())
    }
}

/// Profile files are the native package managers' own persisted inventory.
fn installed(runtime: &Harness) -> Result<Vec<String>, String> {
    let path = runtime.root()?.join("profile").join(match runtime {
        Harness::Pi => "settings.json",
        Harness::Dsh => "profiles/tui/package.json",
        Harness::DshWeb => "profiles/web/package.json",
    });
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let value: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&data).map_err(|_| "Cannot read the native runtime inventory")?;
    let mut packages: Vec<String> = match runtime {
        Harness::Pi => match value.get("packages") {
            None => vec![],
            Some(value) => value
                .as_array()
                .ok_or("Invalid Pi inventory")?
                .iter()
                .map(|row| {
                    row.as_str()
                        .or_else(|| row.get("source")?.as_str())
                        .map(str::to_owned)
                        .ok_or("Invalid Pi package source")
                })
                .collect::<Result<_, _>>()?,
        },
        Harness::Dsh | Harness::DshWeb => match value.get("dependencies") {
            None => vec![],
            Some(value) => value
                .as_object()
                .ok_or("Invalid DSH inventory")?
                .keys()
                .cloned()
                .collect(),
        },
    };
    packages.sort();
    Ok(packages)
}

fn registry_source(source: &str) -> bool {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    source.len() <= 214 && PATTERN.get_or_init(|| regex::Regex::new(
        r"^(?:@[a-z0-9][a-z0-9._-]*/)?[a-z0-9][a-z0-9._-]*(?:@[0-9]+\.[0-9]+\.[0-9]+(?:[-+][a-zA-Z0-9.+-]+)?)?$"
    ).unwrap()).is_match(source)
}

fn available(runtime: &Harness) -> bool {
    let Ok(command) = runtime.command() else {
        return false;
    };
    let program = Path::new(command.get_program());
    if program.is_absolute() {
        return program.is_file();
    }
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .filter(|dir| dir.is_absolute())
        .any(|dir| {
            let Ok(metadata) = dir.join(program).metadata() else {
                return false;
            };
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                metadata.is_file()
            }
        })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PluginPage {
    package: String,
    title: String,
    url: String,
}

#[derive(Deserialize)]
struct PageCatalog {
    version: u32,
    pages: Vec<PluginPage>,
}

fn page_allowed(raw: &str, host: &str, local: bool) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    let Some(name) = url.host_str() else {
        return false;
    };
    let name = name.trim_end_matches('.');
    let loopback = name.eq_ignore_ascii_case("localhost")
        || name.ends_with(".localhost")
        || name
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    !name.eq_ignore_ascii_case(host.trim_end_matches('.'))
        && url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == "https" || (url.scheme() == "http" && loopback))
        && (local || !loopback)
}

fn pages(
    runtime: &Harness,
    installed: &[String],
    host: &str,
    local: bool,
) -> Result<Vec<PluginPage>, String> {
    let path = runtime.root()?.join(if runtime == &Harness::DshWeb {
        "web-pages.json"
    } else {
        "pages.json"
    });
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.to_string()),
    };
    let catalog: PageCatalog =
        serde_json::from_slice(&data).map_err(|_| "Invalid plugin page catalog")?;
    if catalog.version != 1 {
        return Err("Unsupported plugin page catalog version".into());
    }
    Ok(catalog
        .pages
        .into_iter()
        .filter(|p| {
            installed.contains(&p.package)
                && !p.title.trim().is_empty()
                && page_allowed(&p.url, host, local)
        })
        .collect())
}

pub(crate) fn manage(
    name: &str,
    profile: &str,
    action: &str,
    source: &str,
    host: &str,
    local: bool,
) -> Result<serde_json::Value, String> {
    manage_with_source(name, profile, action, source, host, local, false)
}

fn manage_with_source(
    name: &str,
    profile: &str,
    action: &str,
    source: &str,
    host: &str,
    local: bool,
    local_folder: bool,
) -> Result<serde_json::Value, String> {
    let runtime = Harness::named(name)
        .ok_or("Unknown runtime")?
        .with_profile(profile)?;
    let snapshot = |runtime: &Harness| -> Result<_, String> {
        let packages = installed(runtime)?;
        let (pages, page_error) = match pages(runtime, &packages, host, local) {
            Ok(entries) => (entries, None),
            Err(error) => (vec![], Some(error)),
        };
        Ok(
            serde_json::json!({"runtime":runtime.program(),"profile":runtime.profile(),"available":available(runtime),
            "packages":packages,"pages":pages,"pageError":page_error}),
        )
    };
    if action == "list" {
        return snapshot(&runtime);
    }
    static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE
        .try_lock()
        .map_err(|_| "Another plugin operation is in progress")?;
    let items = installed(&runtime)?;
    let operation = match action {
        "install"
            if local_folder
                && Path::new(source).is_absolute()
                && Path::new(source).join("package.json").is_file() =>
        {
            Operation::Install(source.into())
        }
        "install" if registry_source(source) => Operation::Install(if runtime == Harness::Pi {
            format!("npm:{source}")
        } else {
            source.into()
        }),
        "remove"
            if !source.starts_with('-')
                && !source.is_empty()
                && items.iter().any(|item| item == source) =>
        {
            if runtime == Harness::Pi
                && !source.starts_with("npm:")
                && !source.starts_with("git:")
                && !source.contains("://")
                && !source.starts_with("~/")
            {
                let path = desktop::pi_local_directory(&runtime.root()?.join("profile"), source);
                Operation::Remove(path.to_string_lossy().into_owned())
            } else {
                Operation::Remove(source.into())
            }
        }
        _ => return Err("Invalid registry package or unrecognized installed package".into()),
    };
    let task = HarnessPluginCommand { runtime, operation };
    let mut command = task.prepare(Path::new("."))?;
    use std::process::Stdio;
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "Cannot start the runtime. Check the Host installation and PATH.")?;
    if !status.success() {
        return Err(format!(
            "{} exited with {status}. Refresh the package list before retrying.",
            task.runtime.program()
        ));
    }
    snapshot(&task.runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CommandMode, Config};

    #[test]
    fn harness_plugin_pages_follow_client_and_origin() {
        assert!(page_allowed(
            "https://board.example/tasks",
            "nola.example",
            false
        ));
        assert!(page_allowed(
            "http://localhost:9000/tasks",
            "127.0.0.1",
            true
        ));
        for page in [
            "https://user:secret@board.example/",
            "javascript:void(0)",
            "https://nola.example:9001/",
            "https://nola.example./",
            "https://[::1]/",
            "http://board.example/",
            "http://localhost:9000/",
        ] {
            assert!(!page_allowed(page, "nola.example", false));
        }
        assert!(!page_allowed("http://127.0.0.1:9999", "127.0.0.1", true));
        for source in [
            "--global",
            "file:/tmp/test",
            "npm:other",
            "https://example.com",
            "a b",
        ] {
            assert!(!registry_source(source));
        }
        assert!(registry_source("@example/task-board@1.2.3-beta.1"));
    }

    #[test]
    fn harness_plugin_native_arguments_and_scope() {
        let caller = env::temp_dir();
        let parse = |words: &[&str]| {
            let parsed = Config::from_args(words.iter().map(|s| s.to_string())).unwrap();
            let CommandMode::HarnessPlugin(command) = parsed.command else {
                panic!("expected explicit harness management")
            };
            command.arguments(&caller)
        };
        assert_eq!(
            parse(&["plugin", "pi", "install", "npm:@example/extension@1"]),
            ["install", "npm:@example/extension@1"]
        );
        assert_eq!(
            parse(&["plugin", "dsh", "list"]),
            ["plugin", "--profile", "tui", "list", "--depth", "0"]
        );
        assert_eq!(
            parse(&["plugin", "dsh", "--profile", "web", "open"]),
            ["--profile", "web"]
        );
        assert_eq!(parse(&["plugin", "dsh", "open"]), ["--profile", "tui"]);
        assert_eq!(parse(&["plugin", "pi", "open"]), Vec::<String>::new());
        assert_eq!(
            parse(&["plugin", "dsh", "remove", "@demo/bundle"]),
            ["plugin", "--profile", "tui", "remove", "@demo/bundle"]
        );
        assert_eq!(
            parse(&["plugin", "pi", "install", "./a b"])[1],
            caller.join("./a b").to_string_lossy()
        );
    }

    #[test]
    fn harness_plugin_rejects_incomplete_and_extra_arguments() {
        for args in [
            vec!["plugin", "pi"],
            vec!["plugin", "pi", "install"],
            vec!["plugin", "dsh", "install", "--global"],
            vec!["plugin", "pi", "list", "extra"],
            vec!["plugin", "dsh", "exec", "anything"],
        ] {
            assert!(Config::from_args(args.into_iter().map(str::to_owned)).is_err());
        }
    }
}

/// A narrow native desktop control plane, shared by the CLI and the bundled Agent Host.
pub(crate) fn desktop_command(args: &[String]) -> Option<Result<(), String>> {
    let [prefix, action, rest @ ..] = args else {
        return None;
    };
    if prefix != "plugin" {
        return None;
    }
    let result = match action.as_str() {
        "desktop-library" => match rest {
            [] => Ok(desktop::library("", true)),
            _ => Err("Use plugin desktop-library".into()),
        },
        "desktop-pages" => match rest {
            [] => desktop::catalog(None),
            [id] => desktop::catalog(Some(id)),
            _ => Err("Use plugin desktop-pages [id]".into()),
        },
        "desktop-manage" => match rest {
            [runtime, profile, verb] if verb == "list" => {
                manage_with_source(runtime, profile, verb, "", "", true, true)
            }
            [runtime, profile, verb, source] if matches!(verb.as_str(), "install" | "remove") => {
                manage_with_source(runtime, profile, verb, source, "", true, true)
            }
            _ => Err(
                "Use plugin desktop-manage <runtime> <profile> list|install|remove [source]".into(),
            ),
        },
        _ => return None,
    };
    Some(result.map(|value| println!("{value}")))
}
