// SPDX-License-Identifier: Apache-2.0
//! Explicit composer attachment gestures. Text alone never grants file access.
use crate::external_execution_settings::HostAttachment;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

const MAX_MEDIA_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 512 * 1024;
const STAGED_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

fn staged() -> &'static Mutex<BTreeMap<String, HostAttachment>> {
    static STAGED: OnceLock<Mutex<BTreeMap<String, HostAttachment>>> = OnceLock::new();
    STAGED.get_or_init(Default::default)
}

pub(crate) fn attachments(text: &str) -> Vec<HostAttachment> {
    let mut matches = staged()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter_map(|(marker, attachment)| {
            text.find(marker).map(|offset| (offset, attachment.clone()))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(offset, _)| *offset);
    matches.into_iter().map(|(_, attachment)| attachment).collect()
}

pub(crate) fn evict_attachments(text: &str) {
    let mut guard = staged().lock().unwrap_or_else(|p| p.into_inner());
    guard.retain(|marker, _| !text.contains(marker.as_str()));
}

pub(crate) fn append_items(
    items: &mut Vec<nola_core::agent::AgentItem>,
    message: &str,
    text: &str,
) {
    use nola_core::agent::{AgentItem, ItemId};
    for (index, attachment) in attachments(text).into_iter().enumerate() {
        let uri: String = url::Url::from_file_path(&attachment.path)
            .map(String::from)
            .unwrap_or_else(|_| attachment.path.to_string_lossy().into_owned());
        let already_present = items.iter().any(|item| {
            matches!(item, AgentItem::Attachment { uri: existing, .. } if existing == &uri)
        });
        if already_present {
            continue;
        }
        items.push(AgentItem::Attachment {
            id: ItemId::from_string(format!("{message}:clipboard:{}", index + 1)),
            message_id: ItemId::from_string(message.to_owned()),
            uri,
            name: attachment.name,
            mime_type: Some(attachment.mime_type),
            byte_count: attachment.byte_count,
        });
    }
}

pub(crate) fn prune_old_staged_files() {
    let Some(home) = crate::platform_paths::home_dir() else {
        return;
    };
    prune_old_staged_files_in(&home.join(".nola").join("clipboard"), STAGED_MAX_AGE);
}

fn prune_old_staged_files_in(directory: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let stamp = metadata.modified().or_else(|_| metadata.created()).ok();
        let is_old = stamp
            .and_then(|time| now.duration_since(time).ok())
            .is_some_and(|age| age >= max_age);
        if is_old {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Tokenizes a POSIX shell-quoted string into unescaped words.
/// Returns `None` on unclosed quotes or a trailing lone backslash.
pub(crate) fn split_posix_shell_words(input: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '\'' {
                        closed = true;
                        break;
                    }
                    current.push(next);
                }
                if !closed {
                    return None;
                }
            }
            '"' => {
                in_word = true;
                let mut closed = false;
                while let Some(next) = chars.next() {
                    if next == '"' {
                        closed = true;
                        break;
                    }
                    if next == '\\' {
                        let Some(escaped) = chars.next() else {
                            return None;
                        };
                        match escaped {
                            '"' | '\\' | '$' | '`' => current.push(escaped),
                            '\n' => {}
                            other => {
                                current.push('\\');
                                current.push(other);
                            }
                        }
                    } else {
                        current.push(next);
                    }
                }
                if !closed {
                    return None;
                }
            }
            '\\' => {
                in_word = true;
                let Some(escaped) = chars.next() else {
                    return None;
                };
                if escaped != '\n' {
                    current.push(escaped);
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Some(words)
}

fn expand_tilde(path: &str) -> Option<PathBuf> {
    let home = crate::platform_paths::home_dir()?;
    if path == "~" {
        return Some(home);
    }
    path.strip_prefix("~/").map(|rest| home.join(rest))
}

fn resolve_token_path(token: &str, base_dir: Option<&Path>) -> Option<(PathBuf, bool)> {
    if token.is_empty() {
        return None;
    }
    if token.starts_with("file://") {
        let url = url::Url::parse(token).ok()?;
        let path = url.to_file_path().ok()?;
        return Some((path, true));
    }
    if token == "~" || token.starts_with("~/") {
        return expand_tilde(token).map(|path| (path, true));
    }
    let path = Path::new(token);
    if path.is_absolute() {
        if base_dir.is_none() {
            // Avoid mistaking slash commands like `/help` for absolute file paths.
            let rest = token.trim_start_matches('/');
            if !rest.contains('/') {
                return None;
            }
        }
        return Some((path.to_path_buf(), false));
    }
    base_dir.map(|dir| (dir.join(path), true))
}

fn image_mime_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn text_mime_for_extension(ext: Option<&str>) -> &'static str {
    match ext {
        Some("md" | "markdown") => "text/markdown",
        Some("json") => "application/json",
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("xml") => "application/xml",
        Some("csv") => "text/csv",
        Some("yaml" | "yml") => "text/yaml",
        _ => "text/plain",
    }
}

fn validate_image_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BYTES {
        return Err("image must be 1 byte..20 MiB".into());
    }
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = image::ImageReader::new(cursor)
        .with_guessed_format()
        .map_err(|e| format!("invalid image: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    reader
        .decode()
        .map(|_| ())
        .map_err(|e| format!("invalid image: {e}"))
}

fn validate_pdf_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BYTES {
        return Err("PDF must be 1 byte..20 MiB".into());
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err("file is not a valid PDF document".into());
    }
    Ok(())
}

fn validate_text_bytes(bytes: &[u8]) -> Result<&str, String> {
    if bytes.len() as u64 > MAX_TEXT_BYTES {
        return Err("text file exceeds 512 KiB".into());
    }
    if bytes.contains(&0) {
        return Err("unsupported binary file type".into());
    }
    std::str::from_utf8(bytes).map_err(|_| "unsupported non-UTF-8 binary file".to_string())
}

fn format_byte_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        let kb = ((bytes as f64) / 1024.0).round().max(1.0) as u64;
        format!("{kb} KB")
    } else {
        let mb = (bytes as f64) / (1024.0 * 1024.0);
        format!("{mb:.1} MB")
    }
}

fn make_unique_marker(
    guard: &BTreeMap<String, HostAttachment>,
    display_name: &str,
    byte_count: u64,
) -> String {
    let size = format_byte_size(byte_count);
    let base = format!("[{display_name} · {size}]");
    if !guard.contains_key(&base) {
        return base;
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("[{display_name} · {size} #{n}]");
        if !guard.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

fn stage_bytes_in(
    directory: &Path,
    bytes: &[u8],
    original_name: &str,
    extension: &str,
    mime_type: &str,
) -> Result<String, String> {
    use std::io::Write;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let suffix = if extension.is_empty() {
        String::new()
    } else {
        format!(".{extension}")
    };
    let mut file = tempfile::Builder::new()
        .prefix("attach-")
        .suffix(&suffix)
        .tempfile_in(directory)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    let (_, path) = file.keep().map_err(|e| e.to_string())?;
    let canonical = path.canonicalize().unwrap_or(path);
    let display_name = if original_name.trim().is_empty() {
        canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".to_string())
    } else {
        original_name.to_string()
    };
    let attachment = HostAttachment {
        name: display_name.clone(),
        path: canonical,
        mime_type: mime_type.to_string(),
        byte_count: bytes.len() as u64,
    };
    let mut guard = staged().lock().unwrap_or_else(|p| p.into_inner());
    let marker = make_unique_marker(&guard, &display_name, attachment.byte_count);
    guard.insert(marker.clone(), attachment);
    Ok(marker)
}

fn default_staging_dir() -> Result<PathBuf, String> {
    Ok(crate::platform_paths::home_dir()
        .ok_or("cannot locate Nola home")?
        .join(".nola")
        .join("clipboard"))
}

fn stage_file_in(directory: &Path, path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if metadata.is_dir() {
        return Err(format!("cannot attach a directory: {}", path.display()));
    }
    if !metadata.is_file() {
        return Err(format!("not a regular file: {}", path.display()));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let original_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_string());

    if let Some(mime) = ext.as_deref().and_then(image_mime_for_extension) {
        if metadata.len() > MAX_MEDIA_BYTES {
            return Err(format!("image exceeds 20 MiB: {original_name}"));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        validate_image_bytes(&bytes)?;
        return stage_bytes_in(
            directory,
            &bytes,
            &original_name,
            ext.as_deref().unwrap_or("png"),
            mime,
        );
    }

    if ext.as_deref() == Some("pdf") {
        if metadata.len() > MAX_MEDIA_BYTES {
            return Err(format!("PDF exceeds 20 MiB: {original_name}"));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        validate_pdf_bytes(&bytes)?;
        return stage_bytes_in(directory, &bytes, &original_name, "pdf", "application/pdf");
    }

    if metadata.len() > MAX_TEXT_BYTES {
        return Err(format!("file exceeds 512 KiB text limit: {original_name}"));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let _ = validate_text_bytes(&bytes)?;
    let mime = text_mime_for_extension(ext.as_deref());
    stage_bytes_in(
        directory,
        &bytes,
        &original_name,
        ext.as_deref().unwrap_or("txt"),
        mime,
    )
}

/// Validates `--context-file` paths, returning media/PDF `HostAttachment`s and
/// bounded inline UTF-8 text context.
pub(crate) fn resolve_context_files(
    paths: &[PathBuf],
) -> Result<(Vec<HostAttachment>, String), String> {
    let mut attachments = Vec::new();
    let mut inline_blocks = Vec::new();

    for raw in paths {
        let expanded = raw
            .to_str()
            .and_then(|s| {
                if s == "~" || s.starts_with("~/") {
                    expand_tilde(s)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| raw.clone());
        let metadata = std::fs::metadata(&expanded)
            .map_err(|_| format!("context file not found: {}", expanded.display()))?;
        if metadata.is_dir() {
            return Err(format!(
                "context file is a directory: {}",
                expanded.display()
            ));
        }
        if !metadata.is_file() {
            return Err(format!(
                "context file is not a regular file: {}",
                expanded.display()
            ));
        }
        let canonical = expanded.canonicalize().unwrap_or_else(|_| {
            if expanded.is_absolute() {
                expanded.clone()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&expanded))
                    .unwrap_or_else(|_| expanded.clone())
            }
        });
        let name = canonical
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| expanded.to_string_lossy().into_owned());
        let ext = canonical
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);

        if let Some(mime) = ext.as_deref().and_then(image_mime_for_extension) {
            if metadata.len() > MAX_MEDIA_BYTES {
                return Err(format!("context image exceeds 20 MiB: {}", expanded.display()));
            }
            let bytes = std::fs::read(&canonical).map_err(|e| e.to_string())?;
            validate_image_bytes(&bytes)?;
            attachments.push(HostAttachment {
                name,
                path: canonical,
                mime_type: mime.to_string(),
                byte_count: bytes.len() as u64,
            });
            continue;
        }

        if ext.as_deref() == Some("pdf") {
            if metadata.len() > MAX_MEDIA_BYTES {
                return Err(format!("context PDF exceeds 20 MiB: {}", expanded.display()));
            }
            let bytes = std::fs::read(&canonical).map_err(|e| e.to_string())?;
            validate_pdf_bytes(&bytes)?;
            attachments.push(HostAttachment {
                name,
                path: canonical,
                mime_type: "application/pdf".to_string(),
                byte_count: bytes.len() as u64,
            });
            continue;
        }

        if metadata.len() > MAX_TEXT_BYTES {
            return Err(format!(
                "context file exceeds 512 KiB: {}",
                expanded.display()
            ));
        }
        let bytes = std::fs::read(&canonical).map_err(|e| e.to_string())?;
        let text = validate_text_bytes(&bytes)
            .map_err(|e| format!("{}: {e}", expanded.display()))?;
        inline_blocks.push(format!(
            "<context_file path=\"{}\">\n{}\n</context_file>",
            canonical.display(),
            text
        ));
    }

    Ok((attachments, inline_blocks.join("\n\n")))
}

#[cfg(any(feature = "terminal-ui", test))]
pub(crate) fn paste_key(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    key.kind == KeyEventKind::Press
        && ((matches!(key.code, KeyCode::Char('v' | 'V'))
            && key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER))
            || (key.code == KeyCode::Insert && key.modifiers == KeyModifiers::SHIFT))
}

#[cfg(any(feature = "terminal-ui", test))]
pub(crate) fn paste_image() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let directory = default_staging_dir()?;
    if let Ok(files) = clipboard.get().file_list() {
        for path in files {
            if path.is_file() {
                if let Ok(marker) = stage_file_in(&directory, &path) {
                    return Ok(marker);
                }
            }
        }
    }
    let image = clipboard.get_image().map_err(|e| e.to_string())?;
    let width = u32::try_from(image.width).map_err(|_| "image width exceeds limit")?;
    let height = u32::try_from(image.height).map_err(|_| "image height exceeds limit")?;
    if u64::from(width) * u64::from(height) > 32 * 1024 * 1024 {
        return Err("clipboard image exceeds 32 megapixels".into());
    }
    let dynamic = image::DynamicImage::ImageRgba8(
        image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
            .ok_or("invalid clipboard image buffer")?,
    );
    let mut png = std::io::Cursor::new(Vec::new());
    dynamic
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let bytes = png.into_inner();
    if bytes.is_empty() || bytes.len() as u64 > MAX_MEDIA_BYTES {
        return Err("image must be 1 byte..20 MiB".into());
    }
    let name = format!(
        "clipboard-{}.png",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    stage_bytes_in(&directory, &bytes, &name, "png", "image/png")
}

/// Standalone pasted file path(s) (e.g. terminal drag-and-drop) are explicit
/// attachment gestures. Other pasted text remains text, even when it happens
/// to mention a file path.
#[cfg(any(feature = "terminal-ui", test))]
pub(crate) fn paste_path(text: &str) -> Result<Option<String>, String> {
    let directory = default_staging_dir()?;
    paste_path_in(&directory, text)
}

#[cfg(any(feature = "terminal-ui", test))]
fn paste_path_in(directory: &Path, text: &str) -> Result<Option<String>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let Some(tokens) = split_posix_shell_words(trimmed) else {
        return Ok(None);
    };
    if tokens.is_empty() {
        return Ok(None);
    }

    let mut resolved = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let Some((path, explicit)) = resolve_token_path(token, None) else {
            return Ok(None);
        };
        resolved.push((path, explicit));
    }

    for (path, explicit) in &resolved {
        if path.is_dir() {
            return Err(format!("cannot attach a directory: {}", path.display()));
        }
        if !path.exists() {
            if *explicit || path.extension().is_some() {
                return Err(format!("file not found: {}", path.display()));
            }
            return Ok(None);
        }
        if !path.is_file() {
            return Err(format!("not a regular file: {}", path.display()));
        }
    }

    let mut markers = Vec::with_capacity(resolved.len());
    for (path, _) in resolved {
        markers.push(stage_file_in(directory, &path)?);
    }
    Ok(Some(markers.join(" ")))
}

/// Explicitly attaches one or more files from `/attach <path> [path ...]`.
pub(crate) fn attach_path(raw_args: &str, base_dir: &Path) -> Result<String, String> {
    let directory = default_staging_dir()?;
    attach_path_in(&directory, raw_args, base_dir)
}

fn attach_path_in(directory: &Path, raw_args: &str, base_dir: &Path) -> Result<String, String> {
    let trimmed = raw_args.trim();
    if trimmed.is_empty() {
        return Err("missing file path".into());
    }
    let tokens = split_posix_shell_words(trimmed)
        .ok_or_else(|| "unclosed quote or invalid escape in path".to_string())?;
    if tokens.is_empty() {
        return Err("missing file path".into());
    }
    let mut markers = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let (path, _) = resolve_token_path(token, Some(base_dir))
            .ok_or_else(|| format!("invalid file path: {token}"))?;
        if !path.exists() {
            return Err(format!("file not found: {}", path.display()));
        }
        markers.push(stage_file_in(directory, &path)?);
    }
    Ok(markers.join(" "))
}

pub(crate) fn paste_error(error: &str) -> String {
    match crate::i18n::current_language() {
        crate::i18n::Language::ZhHans => format!(
            "图片粘贴失败：{error}。请先复制图片，再按 Ctrl+V（macOS 终端也可用）；终端的 Cmd+V 通常只粘贴文本。"
        ),
        crate::i18n::Language::ZhHant => format!(
            "圖片貼上失敗：{error}。請先複製圖片，再按 Ctrl+V（macOS 終端機也可用）；終端機的 Cmd+V 通常只貼上文字。"
        ),
        crate::i18n::Language::De => format!(
            "Einfügen des Bildes fehlgeschlagen: {error}. Kopieren Sie zuerst ein Bild und drücken Sie Strg+V; Cmd+V fügt im Terminal meist nur Text ein."
        ),
        crate::i18n::Language::Es => format!(
            "Error al pegar la imagen: {error}. Copia primero una imagen y pulsa Ctrl+V; Cmd+V en el terminal suele pegar solo texto."
        ),
        crate::i18n::Language::Fr => format!(
            "Échec du collage de l'image : {error}. Copiez d'abord une image puis appuyez sur Ctrl+V ; Cmd+V ne colle généralement que du texte dans le terminal."
        ),
        crate::i18n::Language::Ja => format!(
            "画像の貼り付けに失敗しました: {error}。画像をコピーしてから Ctrl+V を押してください（端末の Cmd+V は通常テキストのみ貼り付けます）。"
        ),
        crate::i18n::Language::Ko => format!(
            "이미지 붙여넣기 실패: {error}. 먼저 이미지를 복사한 뒤 Ctrl+V를 누르세요(터미널의 Cmd+V는 보통 텍스트만 붙여넣습니다)."
        ),
        crate::i18n::Language::Ru => format!(
            "Не удалось вставить изображение: {error}. Скопируйте изображение и нажмите Ctrl+V; Cmd+V в терминале обычно вставляет только текст."
        ),
        crate::i18n::Language::En => format!(
            "Image paste failed: {error}. Copy an image, then press Ctrl+V (also on macOS); terminal Cmd+V usually pastes text only."
        ),
    }
}

pub(crate) fn paste_path_error(error: &str) -> String {
    match crate::i18n::current_language() {
        crate::i18n::Language::ZhHans => format!("文件附加失败：{error}。"),
        crate::i18n::Language::ZhHant => format!("檔案附加失敗：{error}。"),
        crate::i18n::Language::De => format!("Datei konnte nicht angehängt werden: {error}."),
        crate::i18n::Language::Es => format!("No se pudo adjuntar el archivo: {error}."),
        crate::i18n::Language::Fr => format!("Impossible de joindre le fichier : {error}."),
        crate::i18n::Language::Ja => format!("ファイルの添付に失敗しました: {error}。"),
        crate::i18n::Language::Ko => format!("파일 첨부 실패: {error}."),
        crate::i18n::Language::Ru => format!("Не удалось прикрепить файл: {error}."),
        crate::i18n::Language::En => format!("File attachment failed: {error}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png_bytes() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([255, 0, 0, 255]),
        ));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn tiny_jpeg_bytes() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([0, 128, 255]),
        ));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        buf.into_inner()
    }

    #[test]
    fn cli_polish_image_gesture_is_attached_only_to_its_own_message() {
        let root = tempfile::tempdir().unwrap();
        let marker =
            stage_bytes_in(root.path(), b"fixture", "first.png", "png", "image/png").unwrap();
        assert!(attachments("a normal message mentioning /tmp/image.png").is_empty());
        let text = format!("inspect {marker}");
        let mut items = Vec::new();
        append_items(&mut items, "user-1", &text);
        let value = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(value["type"], "attachment");
        assert_eq!(value["message_id"], "user-1");
        assert_eq!(value["mime_type"], "image/png");
        assert_eq!(value["byte_count"], 7);
        assert!(attachments("deleted image placeholder").is_empty());
        let second =
            stage_bytes_in(root.path(), b"second", "second.png", "png", "image/png").unwrap();
        let ordered = attachments(&format!("{second} then {marker}"));
        assert_eq!(
            ordered.iter().map(|a| a.byte_count).collect::<Vec<_>>(),
            vec![6, 7]
        );
        evict_attachments(&format!("{second} then {marker}"));
        assert!(attachments(&format!("{second} then {marker}")).is_empty());
    }

    #[cfg(feature = "terminal-ui")]
    #[test]
    fn cli_polish_image_shortcut_never_consumes_submit_or_key_release() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        assert!(paste_key(&KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL
        )));
        assert!(!paste_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
        assert!(!paste_key(&KeyEvent::new_with_kind(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release
        )));
        assert!(paste_path("explain /tmp/example.png please")
            .unwrap()
            .is_none());
    }

    #[test]
    fn posix_shell_escaped_path_and_preserves_original_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("Screenshot 2026 (1).jpg");
        let jpeg = tiny_jpeg_bytes();
        std::fs::write(&file_path, &jpeg).unwrap();

        let escaped = file_path
            .to_string_lossy()
            .replace(' ', "\\ ")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let marker = paste_path_in(staging.path(), &escaped)
            .unwrap()
            .expect("should stage escaped image path");
        assert!(marker.starts_with("[Screenshot 2026 (1).jpg · "));
        let resolved = attachments(&marker);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].mime_type, "image/jpeg");
        assert_eq!(resolved[0].name, "Screenshot 2026 (1).jpg");
        assert_eq!(std::fs::read(&resolved[0].path).unwrap(), jpeg);
        evict_attachments(&marker);
    }

    #[test]
    fn multi_file_drag_drop_and_pdf_staging() {
        let dir = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("diagram.png");
        let pdf_path = dir.path().join("report.pdf");
        std::fs::write(&png_path, tiny_png_bytes()).unwrap();
        std::fs::write(&pdf_path, b"%PDF-1.4\n%fixture\n%%EOF\n").unwrap();

        let drop_text = format!("'{}' \"{}\"", png_path.display(), pdf_path.display());
        let markers = paste_path_in(staging.path(), &drop_text)
            .unwrap()
            .expect("should stage both files");
        assert!(markers.contains("diagram.png"));
        assert!(markers.contains("report.pdf"));

        let resolved = attachments(&markers);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].mime_type, "image/png");
        assert_eq!(resolved[1].mime_type, "application/pdf");
        evict_attachments(&markers);
    }

    #[test]
    fn file_uri_decoding_and_path_errors() {
        let dir = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let pdf_path = dir.path().join("My Report (Final).pdf");
        std::fs::write(&pdf_path, b"%PDF-1.7\n%%EOF\n").unwrap();
        let uri = url::Url::from_file_path(&pdf_path).unwrap().to_string();

        let marker = paste_path_in(staging.path(), &uri)
            .unwrap()
            .expect("should decode file:// URI");
        assert!(marker.contains("My Report (Final).pdf"));
        evict_attachments(&marker);

        // Directory should return explicit error
        let dir_err = paste_path_in(staging.path(), &dir.path().to_string_lossy()).unwrap_err();
        assert!(dir_err.contains("directory"));
        // Missing file with extension should return explicit error
        let missing = dir.path().join("missing.pdf");
        let missing_err = paste_path_in(staging.path(), &missing.to_string_lossy()).unwrap_err();
        assert!(missing_err.contains("not found"));
        // paste_path_error never mentions Ctrl+V
        assert!(!paste_path_error(&missing_err).contains("Ctrl+V"));
    }

    #[test]
    fn resolve_context_files_handles_media_text_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("shot.png");
        let pdf_path = dir.path().join("spec.pdf");
        let txt_path = dir.path().join("notes.rs");
        std::fs::write(&png_path, tiny_png_bytes()).unwrap();
        std::fs::write(&pdf_path, b"%PDF-1.4\n%%EOF\n").unwrap();
        std::fs::write(&txt_path, "fn main() {}\n").unwrap();

        let (attached, inline) =
            resolve_context_files(&[png_path, pdf_path, txt_path.clone()]).unwrap();
        assert_eq!(attached.len(), 2);
        assert_eq!(attached[0].mime_type, "image/png");
        assert_eq!(attached[1].mime_type, "application/pdf");
        assert!(inline.contains("fn main() {}"));
        assert!(inline.contains("notes.rs"));

        // Missing file fails explicitly
        assert!(resolve_context_files(&[dir.path().join("nope.txt")]).is_err());
        // Directory fails explicitly
        assert!(resolve_context_files(&[dir.path().to_path_buf()]).is_err());
    }
}
