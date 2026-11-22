// SPDX-License-Identifier: Apache-2.0
//! Terminal-only first-run setup adapter. Never selected implicitly by Host configuration.
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crate::terminal_input::read;
use crate::i18n::{ui_text, UiText};
use crate::configuration_setup::{ConfigurationSetup, GeneratedConfiguration};
use serde_json::{Map, Value};

pub(crate) struct TerminalConfigurationSetup;
impl ConfigurationSetup for TerminalConfigurationSetup {
    fn configure_missing_model(&self) -> Result<Option<GeneratedConfiguration>, String> {
        if !io::stdin().is_terminal() || global_config_path().is_none_or(|path| path.exists() || path.with_extension("jsonc").exists()) {
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
        if value.is_empty() {
            println!("{}", ui_text(UiText::EndpointRequired));
            continue;
        }
        if !is_valid_endpoint_url(&value) {
            println!("{}", ui_text(UiText::EndpointInvalidScheme));
            continue;
        }
        break value;
    };
    let model = loop {
        let value = prompt_config_line(ui_text(UiText::ModelPrompt))?;
        let value = value.trim().to_string();
        if !value.is_empty() {
            break value;
        }
        println!("{}", ui_text(UiText::ModelRequired));
    };
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

fn is_valid_endpoint_url(value: &str) -> bool {
    reqwest::Url::parse(value.trim()).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https") && url.host().is_some()
    })
}

fn prompt_config_line(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|err| format!("flush prompt failed: {err}"))?;
    let mut value = String::new();
    match io::stdin().read_line(&mut value) {
        Ok(0) => Err(io::Error::from(io::ErrorKind::UnexpectedEof).to_string()),
        Ok(_) => Ok(value),
        Err(error) => Err(format!("read prompt failed: {error}")),
    }
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
    let mut out = io::stdout();
    loop {
        match read().map_err(|err| format!("read secure input failed: {err}"))? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Enter => {
                    print!("\r\n");
                    out.flush()
                        .map_err(|err| format!("flush prompt failed: {err}"))?;
                    return Ok(value);
                }
                KeyCode::Backspace | KeyCode::Delete => {
                    if pop_secret_char(&mut value) {
                        erase_secret_stars(&mut out, 1)?;
                    }
                }
                KeyCode::Char(ch)
                    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(ch, 'c' | 'C') =>
                {
                    print!("\r\n");
                    let _ = out.flush();
                    return Err("configuration input cancelled".to_string());
                }
                KeyCode::Char(ch)
                    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(ch, 'u' | 'U') =>
                {
                    let removed = clear_secret_line(&mut value);
                    if removed > 0 {
                        erase_secret_stars(&mut out, removed)?;
                    }
                }
                KeyCode::Char(ch)
                    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(ch, 'w' | 'W') =>
                {
                    let removed = pop_secret_word(&mut value);
                    if removed > 0 {
                        erase_secret_stars(&mut out, removed)?;
                    }
                }
                KeyCode::Char(ch) if !ch.is_control() => {
                    value.push(ch);
                    write_secret_stars(&mut out, 1)?;
                }
                _ => {}
            },
            Event::Paste(text) => {
                let added = append_secret_paste(&mut value, &text);
                if added > 0 {
                    write_secret_stars(&mut out, added)?;
                }
            }
            _ => {}
        }
    }
}

fn pop_secret_char(value: &mut String) -> bool {
    value.pop().is_some()
}

fn clear_secret_line(value: &mut String) -> usize {
    let count = value.chars().count();
    value.clear();
    count
}

fn pop_secret_word(value: &mut String) -> usize {
    let mut removed = 0usize;
    while value.chars().next_back().is_some_and(char::is_whitespace) {
        value.pop();
        removed += 1;
    }
    while value.chars().next_back().is_some_and(|ch| !ch.is_whitespace()) {
        value.pop();
        removed += 1;
    }
    removed
}

fn append_secret_paste(value: &mut String, text: &str) -> usize {
    let mut added = 0usize;
    for ch in text.chars() {
        if !ch.is_control() {
            value.push(ch);
            added += 1;
        }
    }
    added
}

fn write_secret_stars(out: &mut impl Write, count: usize) -> Result<(), String> {
    let stars = "*".repeat(count);
    out.write_all(stars.as_bytes())
        .and_then(|_| out.flush())
        .map_err(|err| format!("flush prompt failed: {err}"))
}

fn erase_secret_stars(out: &mut impl Write, count: usize) -> Result<(), String> {
    let seq = "\x08 \x08".repeat(count);
    out.write_all(seq.as_bytes())
        .and_then(|_| out.flush())
        .map_err(|err| format!("flush prompt failed: {err}"))
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
    let directory = path.parent().ok_or("configuration needs a parent directory")?;
    let mut file = tempfile::NamedTempFile::new_in(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    writeln!(file, "{text}").and_then(|()| file.as_file().sync_all())
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.persist_noclobber(path).map_err(|error| format!("{}: {}", path.display(), error.error))?;
    Ok(())
}

fn global_config_path() -> Option<PathBuf> {
    crate::platform_paths::home_dir().map(|home| home.join(".nola").join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_creation_preserves_concurrently_created_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        write_global_config(&path, "https://fixture.invalid", "", "first").unwrap();
        let original = fs::read(&path).unwrap();
        assert!(write_global_config(&path, "https://different.invalid", "", "second").is_err());
        assert_eq!(fs::read(path).unwrap(), original);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn endpoint_url_validation_requires_http_or_https_with_host() {
        assert!(is_valid_endpoint_url("https://api.openai.com/v1"));
        assert!(is_valid_endpoint_url("http://127.0.0.1:11434/v1"));
        assert!(!is_valid_endpoint_url(""));
        assert!(!is_valid_endpoint_url("ftp://example.com/v1"));
        assert!(!is_valid_endpoint_url("http://"));
        assert!(!is_valid_endpoint_url("https://bad host/v1"));
        assert!(!is_valid_endpoint_url("http://localhost:invalid/v1"));
        assert!(!is_valid_endpoint_url("localhost:11434/v1"));
    }

    #[test]
    fn secret_line_editing_and_paste_filtering_behave_cleanly() {
        let mut secret = String::new();
        assert_eq!(append_secret_paste(&mut secret, "sk-\n1234\x07 56"), 10);
        assert_eq!(secret, "sk-1234 56");

        assert!(pop_secret_char(&mut secret));
        assert_eq!(secret, "sk-1234 5");

        assert_eq!(pop_secret_word(&mut secret), 1);
        assert_eq!(secret, "sk-1234 ");

        assert_eq!(pop_secret_word(&mut secret), 8);
        assert_eq!(secret, "");

        append_secret_paste(&mut secret, "token");
        assert_eq!(clear_secret_line(&mut secret), 5);
        assert!(secret.is_empty());
    }
}
