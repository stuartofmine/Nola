// SPDX-License-Identifier: Apache-2.0
use nola_update_host::{UpdateSource, HttpsTransport};
use std::path::PathBuf;

pub(crate) fn run_if_requested(args: &[String]) -> Option<Result<(), String>> {
    if args == ["--version"] || args == ["-V"] {
        return Some(UpdateSource::bundled().map(|source| println!("Nola {} ({})", source.version, source.build)).map_err(|e| e.to_string()));
    }
    if args.first().map(String::as_str) != Some("update") { return None; }
    Some(run(args).map_err(|e| e.to_string()))
}
fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = args[1..].to_vec();
    let source = if let Some(index) = args.iter().position(|s| s == "--source") {
        if index + 1 >= args.len() { return Err("--source requires an absolute public source JSON path".into()); }
        let path = PathBuf::from(&args[index + 1]);
        if !path.is_absolute() { return Err("--source must be absolute".into()); }
        let mut source = UpdateSource::parse(&std::fs::read_to_string(path)?)?;
        let installed = UpdateSource::bundled()?;
        // A public source override cannot claim that another build is installed.
        source.version = installed.version;
        source.build = installed.build;
        args.drain(index..=index + 1); source
    } else { UpdateSource::bundled()? };
    let valid = matches!(args.iter().map(String::as_str).collect::<Vec<_>>().as_slice(), ["check"] | ["download", _]);
    if !valid { return Err("Usage: nola update check | download <absolute-directory> [--source <absolute-public-source-json>]".into()); }
    let target = nola_update_host::native_target("cli");
    let checked = nola_update_host::check(&source, &target, &HttpsTransport)?;
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["check"] => println!("{}", serde_json::to_string_pretty(&checked)?),
        ["download", directory] => {
            let offer = checked.release.ok_or("No downloadable release. Check status and publisher configuration first.")?;
            let package = nola_update_host::download(&source, &target, &offer.release_id, &PathBuf::from(directory), &HttpsTransport)?;
            println!("{}", serde_json::json!({"path":package,"version":offer.version,"build":offer.build,
                "next_step":"Stop the affected Nola Host and its active tasks explicitly, then install the complete matching release bundle. No executable or user data has been replaced."}));
        },
        _ => return Err("Usage: nola update check | download <absolute-directory> [--source <absolute-public-source-json>]".into()),
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
    }
}
