// SPDX-License-Identifier: Apache-2.0
use crate::config_paths::expand_home;
use nola_update_host::{HttpsTransport, UpdateSource};
use std::path::PathBuf;

pub(crate) fn run_if_requested(args: &[String]) -> Option<Result<(), String>> {
    if args == ["--version"] || args == ["-V"] || args == ["version"] {
        return Some(
            UpdateSource::bundled()
                .map(|source| println!("Nola {} ({})", source.version, source.build))
                .map_err(|e| e.to_string()),
        );
    }
    if args.first().map(String::as_str) != Some("update") {
        return None;
    }
    Some(run(args).map_err(|e| e.to_string()))
}

fn resolve_download_dir(raw: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let expanded = expand_home(&PathBuf::from(raw));
    Ok(if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    })
}

fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let rest = args[1..].to_vec();
    if rest
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| arg == "--help" || arg == "-h")
    {
        println!("Usage:\n  nola update check [--source <absolute-public-source-json>]\n  nola update download <directory> [--source <absolute-public-source-json>]");
        return Ok(());
    }
    let mut source_path: Option<PathBuf> = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < rest.len() {
        let arg = &rest[index];
        if arg == "--" {
            positional.extend(rest[index + 1..].iter().cloned());
            break;
        }
        if arg == "--json" {
            // Both update operations already emit JSON.
            index += 1;
        } else if arg == "--source" {
            let value = rest
                .get(index + 1)
                .ok_or("--source requires an absolute public source JSON path")?;
            source_path = Some(PathBuf::from(value));
            index += 2;
        } else if let Some(value) = arg.strip_prefix("--source=") {
            source_path = Some(PathBuf::from(value));
            index += 1;
        } else {
            positional.push(arg.clone());
            index += 1;
        }
    }
    let source = if let Some(path) = source_path {
        if !path.is_absolute() {
            return Err("--source must be absolute".into());
        }
        let mut source = UpdateSource::parse(&std::fs::read_to_string(path)?)?;
        let installed = UpdateSource::bundled()?;
        // A public source override cannot claim that another build is installed.
        source.version = installed.version;
        source.build = installed.build;
        source
    } else {
        UpdateSource::bundled()?
    };
    let refs = positional.iter().map(String::as_str).collect::<Vec<_>>();
    if !matches!(refs.as_slice(), ["check"] | ["download", _]) {
        return Err("Usage: nola update check | download <directory> [--source <absolute-public-source-json>]".into());
    }
    let target = nola_update_host::native_target("cli");
    let checked = nola_update_host::check(&source, &target, &HttpsTransport)?;
    match refs.as_slice() {
        ["check"] => println!("{}", serde_json::to_string_pretty(&checked)?),
        ["download", directory] => {
            let download_dir = resolve_download_dir(directory)?;
            let offer = checked
                .release
                .ok_or("No downloadable release. Check status and publisher configuration first.")?;
            let package = nola_update_host::download(
                &source,
                &target,
                &offer.release_id,
                &download_dir,
                &HttpsTransport,
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "path": package,
                    "version": offer.version,
                    "build": offer.build,
                    "next_step": "Stop the affected Nola Host and its active tasks explicitly, then install the complete matching release bundle. No executable or user data has been replaced."
                })
            );
        }
        _ => {
            return Err("Usage: nola update check | download <directory> [--source <absolute-public-source-json>]".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn explicit_command_is_independent_of_model_configuration() {
        assert!(run_if_requested(&["ordinary update discussion".into()]).is_none());
        let mut source = UpdateSource::bundled().unwrap();
        source.feed_url = None; source.public_key = None;
        let temporary = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temporary.path(), serde_json::to_vec(&source).unwrap()).unwrap();
        assert!(run_if_requested(&["update".into(),"check".into(),"--source".into(),temporary.path().to_string_lossy().into_owned()]).unwrap().is_ok());
        assert!(run_if_requested(&["update".into(),"unknown".into()]).unwrap().is_err());
        assert!(run_if_requested(&["update".into(),"check".into(),"--source".into(),"relative".into()]).unwrap().is_err());
        assert!(run_if_requested(&["update".into(),"check".into(),"--source=relative".into()]).unwrap().is_err());
        assert!(run_if_requested(&["update".into(),"--help".into()]).unwrap().is_ok());
        assert!(run_if_requested(&["version".into()]).unwrap().is_ok());
        let resolved = resolve_download_dir("dist/updates").unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("dist/updates"));
    }
}
