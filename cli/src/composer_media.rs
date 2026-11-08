// SPDX-License-Identifier: Apache-2.0
//! Explicit composer attachment gestures. Text alone never grants file access.
use crate::external_execution_settings::HostAttachment;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

fn staged() -> &'static Mutex<BTreeMap<String, HostAttachment>> {
    static STAGED: OnceLock<Mutex<BTreeMap<String, HostAttachment>>> = OnceLock::new();
    STAGED.get_or_init(Default::default)
}

pub(crate) fn attachments(text: &str) -> Vec<HostAttachment> {
    let mut matches = staged().lock().unwrap_or_else(|p| p.into_inner()).iter()
        .filter_map(|(marker, attachment)| text.find(marker).map(|offset| (offset, attachment.clone())))
        .collect::<Vec<_>>();
    matches.sort_by_key(|(offset, _)| *offset);
    matches.into_iter().map(|(_, attachment)| attachment).collect()
}

pub(crate) fn append_items(items: &mut Vec<nola_core::agent::AgentItem>, message: &str, text: &str) {
    use nola_core::agent::{AgentItem, ItemId};
    for (index, attachment) in attachments(text).into_iter().enumerate() {
        items.push(AgentItem::Attachment {
            id: ItemId::from_string(format!("{message}:clipboard:{}", index + 1)),
            message_id: ItemId::from_string(message.to_owned()),
            uri: url::Url::from_file_path(&attachment.path).expect("absolute clipboard path").into(),
            name: attachment.name, mime_type: Some(attachment.mime_type), byte_count: attachment.byte_count,
        });
    }
}

#[cfg(feature = "terminal-ui")]
pub(crate) fn paste_key(key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
    key.kind == KeyEventKind::Press && ((matches!(key.code, KeyCode::Char('v' | 'V'))
        && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER))
        || (key.code == KeyCode::Insert && key.modifiers == KeyModifiers::SHIFT))
}

#[cfg(feature = "terminal-ui")]
pub(crate) fn paste_image() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let image = if let Some(image) = clipboard.get().file_list().unwrap_or_default().into_iter()
        .find_map(|path| read_image(&path).ok()) {
        image
    } else {
        let image = clipboard.get_image().map_err(|e| e.to_string())?;
        let width = u32::try_from(image.width).map_err(|_| "image width exceeds limit")?;
        let height = u32::try_from(image.height).map_err(|_| "image height exceeds limit")?;
        if u64::from(width) * u64::from(height) > 32 * 1024 * 1024 {
            return Err("clipboard image exceeds 32 megapixels".into());
        }
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
            .ok_or("invalid clipboard image buffer")?)
    };
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    stage_png(png.into_inner())
}

/// A standalone pasted image path is an explicit attachment gesture. Other
/// pasted text remains text, even when it happens to mention a file path.
#[cfg(feature = "terminal-ui")]
pub(crate) fn paste_path(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    let text = text.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
        .or_else(|| text.strip_prefix('"').and_then(|s| s.strip_suffix('"'))).unwrap_or(text);
    let path = text.replace("\\ ", " ");
    let path = std::path::Path::new(&path);
    if !path.is_absolute() || !path.is_file() { return Ok(None); }
    if !matches!(path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")) { return Ok(None); }
    let size = path.metadata().map_err(|e| e.to_string())?.len();
    if size > 20 * 1024 * 1024 { return Err("image exceeds 20 MiB".into()); }
    let image = read_image(path)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    stage_png(png.into_inner()).map(Some)
}

#[cfg(feature = "terminal-ui")]
fn read_image(path: &std::path::Path) -> Result<image::DynamicImage, String> {
    if path.metadata().map_err(|e| e.to_string())?.len() > 20 * 1024 * 1024 {
        return Err("image exceeds 20 MiB".into());
    }
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?
        .with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    reader.decode().map_err(|e| e.to_string())
}

#[cfg(feature = "terminal-ui")]
fn stage_png(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.is_empty() || bytes.len() > 20 * 1024 * 1024 { return Err("image must be 1 byte..20 MiB".into()); }
    let directory = crate::platform_paths::home_dir().ok_or("cannot locate Nola home")?
        .join(".nola").join("clipboard");
    stage_png_in(&directory, &bytes)
}

#[cfg(feature = "terminal-ui")]
fn stage_png_in(directory: &std::path::Path, bytes: &[u8]) -> Result<String, String> {
    use std::io::Write;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut file = tempfile::Builder::new().prefix("image-").suffix(".png").tempfile_in(directory)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    let (_, path) = file.keep().map_err(|e| e.to_string())?;
    let marker = format!("[Image {}]", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let attachment = HostAttachment { name: path.file_name().unwrap().to_string_lossy().into_owned(),
        path, mime_type: "image/png".into(), byte_count: bytes.len() as u64 };
    staged().lock().unwrap_or_else(|p| p.into_inner()).insert(marker.clone(), attachment);
    Ok(marker)
}

#[cfg(all(test, feature = "terminal-ui"))]
mod tests {
    use super::*;

    #[test]
    fn cli_polish_image_gesture_is_attached_only_to_its_own_message() {
        let root = tempfile::tempdir().unwrap();
        let marker = stage_png_in(root.path(), b"fixture").unwrap();
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
        let second = stage_png_in(root.path(), b"second").unwrap();
        let ordered = attachments(&format!("{second} then {marker}"));
        assert_eq!(ordered.iter().map(|a| a.byte_count).collect::<Vec<_>>(), vec![6, 7]);
    }

    #[test]
    fn cli_polish_image_shortcut_never_consumes_submit_or_key_release() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        assert!(paste_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)));
        assert!(!paste_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert!(!paste_key(&KeyEvent::new_with_kind(KeyCode::Char('v'), KeyModifiers::CONTROL, KeyEventKind::Release)));
        assert!(paste_path("explain /tmp/example.png please").unwrap().is_none());
    }
}

pub(crate) fn paste_error(error: &str) -> String {
    if crate::i18n::current_language() == crate::i18n::Language::ZhHans {
        format!("图片粘贴失败：{error}。请先复制图片，再按 Ctrl+V（macOS 终端也可用）；终端的 Cmd+V 通常只粘贴文本。")
    } else {
        format!("Image paste failed: {error}. Copy an image, then press Ctrl+V (also on macOS); terminal Cmd+V usually pastes text only.")
    }
}
