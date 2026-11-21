use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../src/i18n.ts");
    println!("cargo:rerun-if-changed=../scripts/export-shell-strings.mjs");
    println!("cargo:rerun-if-changed=../src/startupPage.ts");
    println!("cargo:rerun-if-changed=../src/themeTokens.ts");
    let strings = Command::new("node")
        .arg("../scripts/export-shell-strings.mjs")
        .output()
        .expect("Building Windows desktop resources requires Node.js 24");
    assert!(
        strings.status.success(),
        "Could not compile desktop translations"
    );
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("locales.json"),
        strings.stdout,
    )
    .unwrap();
    let manifest = tauri_build::AppManifest::new().commands(&[
        "startup_status",
        "retry_startup",
        "open_diagnostics",
        "login_item",
        "set_login_item",
        "native_runtime",
        "set_theme",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("Build Tauri application resources");
}
