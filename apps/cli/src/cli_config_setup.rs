// SPDX-License-Identifier: Apache-2.0
//! Terminal-only first-run setup adapter. Never selected implicitly by Host configuration.
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crate::terminal_input::read;
use crate::i18n::{ui_text, UiText};
use crate::configuration_setup::{ConfigurationSetup, GeneratedConfiguration};
use serde_json::{Map, Value};

pub(crate) struct TerminalConfigurationSetup;
impl ConfigurationSetup for TerminalConfigurationSetup {
    fn configure_missing_model(&self) -> Result<Option<GeneratedConfiguration>, String> {
        if !io::stdin().is_terminal() || global_config_path().is_none_or(|path| path.exists()) {
            return Ok(None);
        }
        prompt_create_global_config()
    }
}

fn prompt_create_global_config() -> Result<Option<GeneratedConfiguration>, String> {
    let Some(path) = global_config_path() else {
        return Ok(None);
    };
    println!("{} {}.", ui_text(UiText::ConfigNotFound), path.display());
    let answer = prompt_config_line(ui_text(UiText::CreateConfigPrompt))?;
    if matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "n" | "no" | "0" | "false"
    ) {
        return Ok(None);
    }

    let endpoint = loop {
        let value = prompt_config_line(ui_text(UiText::EndpointPrompt))?;
        let value = value.trim().to_string();
        if !value.is_empty() {
            break value;
        }
        println!("{}", ui_text(UiText::EndpointRequired));
    };
    let model = prompt_config_line(ui_text(UiText::ModelPrompt))?
        .trim()
        .to_string();
    let api_key = prompt_config_secret(ui_text(UiText::ApiKeyPrompt))?
        .trim()
        .to_string();

    write_global_config(&path, &endpoint, &api_key, &model)?;
    println!("{} {}.", ui_text(UiText::ConfigCreated), path.display());
    Ok(Some(GeneratedConfiguration {
        endpoint,
        api_key,
        model,
    }))
}

fn prompt_config_line(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|err| format!("flush prompt failed: {err}"))?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|err| format!("read prompt failed: {err}"))?;
    Ok(value)
}

struct ConfigSecretRawModeGuard;

impl Drop for ConfigSecretRawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn prompt_config_secret(prompt: &str) -> Result<String, String> {
    if !io::stdin().is_terminal() {
        return prompt_config_line(prompt);
    }
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|err| format!("flush prompt failed: {err}"))?;
    enable_raw_mode().map_err(|err| format!("enable secure input failed: {err}"))?;
    let _raw_mode = ConfigSecretRawModeGuard;
    let mut value = String::new();
    loop {
        match read().map_err(|err| format!("read secure input failed: {err}"))? {
            Event::Key(key) => match edit_secret_key(&mut value, &key) {
                SecretKeyAction::Submit => {
                    print!("\r\n");
                    io::stdout().flush().map_err(|err| format!("flush prompt failed: {err}"))?;
                    return Ok(value);
                }
                SecretKeyAction::Cancel => {
                    print!("\r\n");
                    let _ = io::stdout().flush();
                    return Err("configuration input cancelled".to_string());
                }
                SecretKeyAction::Continue => {}
            },
            Event::Paste(text) => value.push_str(&text),
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SecretKeyAction { Continue, Submit, Cancel }

fn edit_secret_key(value: &mut String, key: &crossterm::event::KeyEvent) -> SecretKeyAction {
    if key.kind != KeyEventKind::Press { return SecretKeyAction::Continue; }
    // Treat terminal CR/LF variants as submission, never as literal m/j in a key.
    if crate::composer::is_submit(key) { return SecretKeyAction::Submit; }
    if key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c' | 'C')) {
        return SecretKeyAction::Cancel;
    }
    if key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('u' | 'U')) {
        value.clear();
        return SecretKeyAction::Continue;
    }
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) {
        return SecretKeyAction::Continue;
    }
    match key.code {
        KeyCode::Backspace | KeyCode::Delete => { value.pop(); }
        KeyCode::Char(ch) if !ch.is_control() => value.push(ch),
        _ => {}
    }
    SecretKeyAction::Continue
}

#[cfg(test)]
mod secret_input_tests {
    use super::*;
    use crossterm::event::KeyEvent;

    #[test]
    fn secret_return_variants_do_not_mutate_the_key() {
        for key in [KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
                    KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL)] {
            let mut value = String::new();
            assert_eq!(edit_secret_key(&mut value, &key), SecretKeyAction::Submit);
            assert!(value.is_empty());
        }
    }

    #[test]
    fn secret_shortcuts_cancel_clear_or_ignore_without_inserting_letters() {
        let mut value = "fixture".to_string();
        assert_eq!(edit_secret_key(&mut value, &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)), SecretKeyAction::Continue);
        assert_eq!(value, "fixture");
        assert_eq!(edit_secret_key(&mut value, &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), SecretKeyAction::Cancel);
        assert_eq!(edit_secret_key(&mut value, &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), SecretKeyAction::Cancel);
        edit_secret_key(&mut value, &KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(value.is_empty());
        edit_secret_key(&mut value, &KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(value, "A");
        edit_secret_key(&mut value, &KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release));
        assert_eq!(value, "A");
    }
}

fn write_global_config(
    path: &Path,
    endpoint: &str,
    api_key: &str,
    model: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create {} failed: {err}", parent.display()))?;
    }
    let mut llm = Map::new();
    llm.insert("endpoint".to_string(), Value::String(endpoint.to_string()));
    if !api_key.trim().is_empty() {
        llm.insert("api_key".to_string(), Value::String(api_key.to_string()));
    }
    if !model.trim().is_empty() {
        llm.insert("model".to_string(), Value::String(model.to_string()));
    }
    let mut root = Map::new();
    root.insert("llm".to_string(), Value::Object(llm));
    root.insert(
        "network_policy".to_string(),
        Value::String("allow".to_string()),
    );
    let text = serde_json::to_string_pretty(&Value::Object(root))
        .map_err(|err| format!("serialize config failed: {err}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_path = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let write_result = (|| -> Result<(), String> {
        let mut file = options
            .open(&temp_path)
            .map_err(|err| format!("create {} failed: {err}", temp_path.display()))?;
        file.write_all(format!("{text}\n").as_bytes())
            .map_err(|err| format!("write {} failed: {err}", temp_path.display()))?;
        file.sync_all()
            .map_err(|err| format!("sync {} failed: {err}", temp_path.display()))?;
        fs::rename(&temp_path, path).map_err(|err| {
            format!(
                "replace {} with {} failed: {err}",
                path.display(),
                temp_path.display()
            )
        })?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn global_config_path() -> Option<PathBuf> {
    crate::platform_paths::home_dir().map(|home| home.join(".nola").join("config.json"))
}
