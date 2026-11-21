// SPDX-License-Identifier: Apache-2.0
//! Host-side desktop engine discovery and explicit installation.
use super::Engine;
use crate::local_models;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

const LLAMA_GUIDE: &str = "https://github.com/ggml-org/llama.cpp/blob/master/docs/install.md";
const METAL_GUIDE: &str = "https://docs.vllm.ai/projects/vllm-metal/en/stable/installation/";
const VLLM_GUIDE: &str = "https://docs.vllm.ai/en/stable/getting_started/installation/";
static INSTALLING: Mutex<()> = Mutex::new(());

fn homebrew() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    // Use the platform installation, never an executable in the current project.
    let prefix = if cfg!(target_arch = "aarch64") {
        "/opt/homebrew"
    } else {
        "/usr/local"
    };
    let path = Path::new(prefix).join("bin/brew");
    local_models::executable_file(&path).then_some(path)
}

pub(super) fn metal_supported() -> bool {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return false;
    }
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().split('.').next()?.parse::<u32>().ok())
            .is_some_and(|v| v >= 15)
    })
}

fn status(engine: Engine, local: bool) -> Value {
    let (ready, override_name, mut guide) = match engine {
        Engine::Llama => (
            local_models::llama_server_path().is_some(),
            "NOLA_LLAMA_SERVER",
            LLAMA_GUIDE,
        ),
        Engine::Vllm => (
            local_models::vllm_path().is_some() && (!cfg!(target_os = "macos") || metal_supported()),
            "NOLA_VLLM_BIN",
            if cfg!(target_os = "macos") {
                METAL_GUIDE
            } else {
                VLLM_GUIDE
            },
        ),
        Engine::Mlx => (
            cfg!(all(target_os = "macos", target_arch = "aarch64")),
            "",
            "https://github.com/ml-explore/mlx-swift",
        ),
    };
    let reason = if ready {
        "ready"
    } else if !local {
        "install_on_host"
    } else if engine == Engine::Mlx
        || engine == Engine::Vllm && cfg!(target_os = "macos") && !metal_supported()
    {
        "unsupported"
    } else if !override_name.is_empty()
        && std::env::var_os(override_name).is_some_and(|s| !s.is_empty())
    {
        "invalid_override"
    } else if !cfg!(target_os = "macos") {
        "manual"
    } else if homebrew().is_some() {
        "installable"
    } else {
        guide = "https://brew.sh/";
        "homebrew_missing"
    };
    json!({"engineStatus":{"engine":engine.name(),"ready":ready,"reason":reason,"canInstall":reason=="installable","guideUrl":guide}})
}

pub(crate) fn engine_status(engine: &str, local: bool) -> Result<Value, String> {
    Ok(status(Engine::parse(engine)?, local))
}

pub(crate) fn setup_engine(engine: &str) -> Result<Value, String> {
    let engine = Engine::parse(engine)?;
    let before = status(engine, true);
    if before["engineStatus"]["ready"] == true {
        return Ok(before);
    }
    if before["engineStatus"]["canInstall"] != true {
        return Err("Open the installation guide on the Host, then check the engine again.".into());
    }
    let _guard = INSTALLING
        .try_lock()
        .map_err(|_| "An engine installation is already running on this Host.")?;
    let brew = homebrew().ok_or("Homebrew is unavailable on this Host.")?;
    install_using_brew(engine, &brew)?;
    let after = status(engine, true);
    if after["engineStatus"]["ready"] != true {
        return Err("Homebrew finished but the engine executable was not found. Check the installation guide and retry detection.".into());
    }
    Ok(after)
}

fn install_using_brew(engine: Engine, brew: &Path) -> Result<(), String> {
    let steps: &[&[&str]] = match engine {
        Engine::Llama => &[&["install", "llama.cpp"]],
        Engine::Vllm => &[
            &[
                "tap",
                "vllm-project/vllm-metal",
                "https://github.com/vllm-project/vllm-metal",
            ],
            &["install", "vllm-project/vllm-metal/vllm-metal"],
        ],
        Engine::Mlx => return Err("MLX is included in the Apple silicon app.".into()),
    };
    for args in steps {
        let mut child = Command::new(brew)
            .args(*args)
            .current_dir("/")
            .env("NONINTERACTIVE", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start Homebrew: {e}"))?;
        let mut tail = Vec::new();
        let mut buf = [0; 4096];
        let mut stderr = child.stderr.take().expect("piped stderr");
        loop {
            match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    tail.extend_from_slice(&buf[..n]);
                    if tail.len() > 4096 {
                        tail.drain(..tail.len() - 4096);
                    }
                }
            }
        }
        let exit = child
            .wait()
            .map_err(|e| format!("Could not read Homebrew exit status: {e}"))?;
        if !exit.success() {
            return Err(format!(
                "Homebrew failed ({exit}). {}",
                String::from_utf8_lossy(&tail).trim()
            ));
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn setup_runs_the_selected_recipe_and_stops_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("brew");
        let log = root.path().join("args");
        let fake = |code| {
            fs::write(
                &binary,
                format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$0.args\"\nexit {code}\n"),
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        };
        let args = binary.with_extension("args");
        fake(1);
        assert!(install_using_brew(Engine::Vllm, &binary).is_err());
        fs::rename(&args, &log).unwrap();
        assert_eq!(fs::read_to_string(log).unwrap().lines().count(), 1);
        fake(0);
        install_using_brew(Engine::Vllm, &binary).unwrap();
        install_using_brew(Engine::Llama, &binary).unwrap();
        assert_eq!(fs::read_to_string(args).unwrap(), "tap vllm-project/vllm-metal https://github.com/vllm-project/vllm-metal\ninstall vllm-project/vllm-metal/vllm-metal\ninstall llama.cpp\n");
        assert!(engine_status("sh", true).is_err());
        assert_eq!(
            status(Engine::Llama, false)["engineStatus"]["canInstall"],
            false
        );
    }
}
