// SPDX-License-Identifier: Apache-2.0
//! Native binary document readers for `read_file` and attachment parsing:
//! - Magic-byte format sniffing (PDF, ZIP-based DOCX vs XLSX, PNG/JPEG/GIF/WEBP, WAV/MP3/FLAC/OGG/MP4)
//! - Word documents (`.docx`) via `zip` + `quick-xml` (`word/document.xml` paragraphs & tables)
//! - Spreadsheets (`.xlsx`, `.xls`, `.ods`) via `calamine`
//! - PDFs (`.pdf`) via `lopdf` text extraction + optional page-render (`pdftoppm`)
//! - Audio (`.wav`, `.mp3`, `.flac`, `.ogg`, `.m4a`, `.aac`) via `symphonia` + `rustfft` spectral analysis
//! - Images (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`) -> validated `MediaAttachment`
//! - `extract_document_text_from_bytes` for in-memory attachment parsing (UTF-8, PDF, DOCX, XLSX)

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Command;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use calamine::{open_workbook_auto, open_workbook_auto_from_rs, Reader};
use nola_types::{sha256_hex, MediaAttachment, ToolExecutionResult};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use rustfft::{num_complex::Complex, FftPlanner};
use serde_json::{json, Value};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const MAX_SNIFF_READ_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DOCX_TEXT_CHARS: usize = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinaryFormat {
    Pdf,
    Docx,
    Spreadsheet,
    Audio(&'static str),
    Image(&'static str),
}

fn sniff_zip_kind(bytes: &[u8]) -> Option<BinaryFormat> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut has_word = false;
    let mut has_xl = false;
    let mut is_ods = false;
    for i in 0..archive.len().min(256) {
        if let Ok(entry) = archive.by_index(i) {
            let name = entry.name();
            if name == "word/document.xml" {
                has_word = true;
                break;
            }
            if name == "xl/workbook.xml" {
                has_xl = true;
                break;
            }
            if name == "mimetype" {
                let mut mt = String::new();
                let mut limited = entry.take(256);
                if limited.read_to_string(&mut mt).is_ok()
                    && mt.contains("opendocument.spreadsheet")
                {
                    is_ods = true;
                }
            }
        }
    }
    if has_word {
        Some(BinaryFormat::Docx)
    } else if has_xl || is_ods {
        Some(BinaryFormat::Spreadsheet)
    } else {
        None
    }
}

/// Sniff binary format from magic bytes first, then fall back to file extension / MIME type.
pub(crate) fn sniff_binary_format(
    bytes: &[u8],
    ext_hint: Option<&str>,
    mime_hint: Option<&str>,
) -> Option<BinaryFormat> {
    // 1. Magic bytes
    if bytes.starts_with(b"%PDF-") {
        return Some(BinaryFormat::Pdf);
    }
    if bytes.len() >= 4
        && (bytes.starts_with(b"PK\x03\x04")
            || bytes.starts_with(b"PK\x05\x06")
            || bytes.starts_with(b"PK\x07\x08"))
    {
        if let Some(kind) = sniff_zip_kind(bytes) {
            return Some(kind);
        }
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(BinaryFormat::Image("image/png"));
    }
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some(BinaryFormat::Image("image/jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(BinaryFormat::Image("image/gif"));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(BinaryFormat::Image("image/webp"));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some(BinaryFormat::Audio("wav"));
    }
    if bytes.starts_with(b"fLaC") {
        return Some(BinaryFormat::Audio("flac"));
    }
    if bytes.starts_with(b"OggS") {
        return Some(BinaryFormat::Audio("ogg"));
    }
    if bytes.starts_with(b"ID3")
        || (bytes.len() >= 4
            && bytes[0] == 0xFF
            && (bytes[1] & 0xE0) == 0xE0
            && (bytes[1] & 0x06) != 0x00
            && (bytes[2] & 0xF0) != 0xF0)
    {
        return Some(BinaryFormat::Audio("mp3"));
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return Some(BinaryFormat::Audio("m4a"));
    }

    // 2. Fallback to extension hint
    if let Some(ext) = ext_hint.map(|s| s.to_ascii_lowercase()) {
        match ext.as_str() {
            "pdf" => return Some(BinaryFormat::Pdf),
            "docx" => return Some(BinaryFormat::Docx),
            "xlsx" | "xls" | "xlsm" | "xlsb" | "ods" => return Some(BinaryFormat::Spreadsheet),
            "wav" => return Some(BinaryFormat::Audio("wav")),
            "mp3" => return Some(BinaryFormat::Audio("mp3")),
            "flac" => return Some(BinaryFormat::Audio("flac")),
            "ogg" => return Some(BinaryFormat::Audio("ogg")),
            "m4a" | "mp4" => return Some(BinaryFormat::Audio("m4a")),
            "aac" => return Some(BinaryFormat::Audio("aac")),
            "png" => return Some(BinaryFormat::Image("image/png")),
            "jpg" | "jpeg" => return Some(BinaryFormat::Image("image/jpeg")),
            "gif" => return Some(BinaryFormat::Image("image/gif")),
            "webp" => return Some(BinaryFormat::Image("image/webp")),
            _ => {}
        }
    }

    // 3. Fallback to MIME hint
    if let Some(mime) = mime_hint.map(|s| s.to_ascii_lowercase()) {
        if mime == "application/pdf" {
            return Some(BinaryFormat::Pdf);
        }
        if mime.contains("wordprocessingml.document") || mime == "application/docx" {
            return Some(BinaryFormat::Docx);
        }
        if mime.contains("spreadsheetml.sheet")
            || mime.contains("opendocument.spreadsheet")
            || mime == "application/vnd.ms-excel"
        {
            return Some(BinaryFormat::Spreadsheet);
        }
        if mime == "image/png" {
            return Some(BinaryFormat::Image("image/png"));
        }
        if mime == "image/jpeg" || mime == "image/jpg" {
            return Some(BinaryFormat::Image("image/jpeg"));
        }
        if mime == "image/gif" {
            return Some(BinaryFormat::Image("image/gif"));
        }
        if mime == "image/webp" {
            return Some(BinaryFormat::Image("image/webp"));
        }
        if mime.starts_with("audio/") {
            return Some(BinaryFormat::Audio("wav"));
        }
    }

    None
}

/// Dispatch binary files (via magic-byte sniffing first, then extension) to rich native extractors;
/// returns `None` for normal UTF-8 text.
pub fn try_read_binary_document(
    path: &Path,
    page: Option<usize>,
    max_rows: usize,
) -> Option<ToolExecutionResult> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());

    // Read file header (or full bytes if ZIP-based and reasonably sized) for magic-byte sniffing.
    let mut header = [0u8; 16];
    let header_len = fs::File::open(path)
        .ok()
        .and_then(|mut f| f.read(&mut header).ok())
        .unwrap_or(0);
    let sniff_slice: Vec<u8> = if header_len >= 4
        && (header[..4] == *b"PK\x03\x04"
            || header[..4] == *b"PK\x05\x06"
            || header[..4] == *b"PK\x07\x08")
        && fs::metadata(path)
            .map(|m| m.len() <= MAX_SNIFF_READ_BYTES)
            .unwrap_or(false)
    {
        fs::read(path).unwrap_or_else(|_| header[..header_len].to_vec())
    } else {
        header[..header_len].to_vec()
    };

    let fmt = sniff_binary_format(&sniff_slice, ext.as_deref(), None)?;
    match fmt {
        BinaryFormat::Spreadsheet => Some(read_spreadsheet(path, max_rows)),
        BinaryFormat::Docx => Some(read_docx(path)),
        BinaryFormat::Pdf => Some(read_pdf(path, page)),
        BinaryFormat::Audio(audio_ext) => Some(read_audio(path, audio_ext)),
        BinaryFormat::Image(mime) => Some(read_image(path, mime)),
    }
}

/// Extract plain text from in-memory bytes for attachment parsing (`web_server.rs /api/attachments/parse`).
/// Supports UTF-8 text, PDF (`lopdf`), DOCX (`zip` + `quick-xml`), and XLSX/XLS/ODS (`calamine`).
pub fn extract_document_text_from_bytes(name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    let mime_opt = (!mime.trim().is_empty()).then_some(mime);

    match sniff_binary_format(bytes, ext.as_deref(), mime_opt) {
        Some(BinaryFormat::Pdf) => extract_pdf_text_from_bytes(bytes).ok(),
        Some(BinaryFormat::Docx) => extract_docx_text_from_bytes(bytes).ok(),
        Some(BinaryFormat::Spreadsheet) => extract_spreadsheet_text_from_bytes(bytes, 500).ok(),
        Some(BinaryFormat::Image(_)) | Some(BinaryFormat::Audio(_)) => None,
        None => {
            if bytes.iter().take(8192).any(|&b| b == 0) {
                return None;
            }
            std::str::from_utf8(bytes).ok().map(ToString::to_string)
        }
    }
}

/// Extract paragraphs and tables from a `.docx` archive (`word/document.xml`).
pub fn extract_docx_text_from_bytes(bytes: &[u8]) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("failed to open docx zip archive: {e}"))?;
    let mut doc_entry = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("docx missing word/document.xml: {e}"))?;
    let mut xml = String::new();
    doc_entry
        .read_to_string(&mut xml)
        .map_err(|e| format!("failed to read word/document.xml: {e}"))?;

    let mut reader = XmlReader::from_str(&xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut in_table = false;
    let mut in_cell = false;
    let mut current_row_cells: Vec<String> = Vec::new();
    let mut current_cell_buf = String::new();
    let mut current_para_buf = String::new();
    let mut in_text = false;

    let push_capped = |dst: &mut String, s: &str| {
        if dst.len() < MAX_DOCX_TEXT_CHARS {
            let rem = MAX_DOCX_TEXT_CHARS - dst.len();
            if s.len() <= rem {
                dst.push_str(s);
            } else {
                let mut end = rem;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                dst.push_str(&s[..end]);
            }
        }
    };

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"tbl" => {
                        in_table = true;
                    }
                    b"tr" => {
                        current_row_cells.clear();
                    }
                    b"tc" => {
                        in_cell = true;
                        current_cell_buf.clear();
                    }
                    b"p" => {
                        current_para_buf.clear();
                    }
                    b"t" => {
                        in_text = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"tab" => current_para_buf.push('\t'),
                    b"br" | b"cr" => current_para_buf.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if in_text {
                    if let Ok(decoded) = e.decode() {
                        let unescaped = quick_xml::escape::unescape(&decoded)
                            .map(|c| c.into_owned())
                            .unwrap_or_else(|_| decoded.into_owned());
                        current_para_buf.push_str(&unescaped);
                    }
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if in_text {
                    if let Ok(name) = e.decode() {
                        match name.as_ref() {
                            "amp" => current_para_buf.push('&'),
                            "lt" => current_para_buf.push('<'),
                            "gt" => current_para_buf.push('>'),
                            "quot" => current_para_buf.push('"'),
                            "apos" => current_para_buf.push('\''),
                            other => {
                                if let Ok(Some(ch)) = e.resolve_char_ref() {
                                    current_para_buf.push(ch);
                                } else {
                                    current_para_buf.push('&');
                                    current_para_buf.push_str(other);
                                    current_para_buf.push(';');
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"t" => {
                        in_text = false;
                    }
                    b"p" => {
                        let trimmed = current_para_buf.trim_end();
                        if in_cell {
                            if !trimmed.is_empty() {
                                if !current_cell_buf.is_empty() {
                                    current_cell_buf.push(' ');
                                }
                                current_cell_buf.push_str(trimmed);
                            }
                        } else if !trimmed.is_empty() {
                            if !out.is_empty() {
                                push_capped(&mut out, "\n");
                            }
                            push_capped(&mut out, trimmed);
                        }
                        current_para_buf.clear();
                    }
                    b"tc" => {
                        current_row_cells.push(current_cell_buf.trim().to_string());
                        current_cell_buf.clear();
                        in_cell = false;
                    }
                    b"tr" => {
                        if !current_row_cells.is_empty()
                            && current_row_cells.iter().any(|c| !c.is_empty())
                        {
                            if !out.is_empty() {
                                push_capped(&mut out, "\n");
                            }
                            push_capped(&mut out, &current_row_cells.join(" | "));
                        }
                        current_row_cells.clear();
                    }
                    b"tbl" => {
                        in_table = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("failed to parse word/document.xml: {e}")),
            _ => {}
        }
    }
    let _ = in_table;
    Ok(out)
}

fn extract_pdf_text_from_bytes(bytes: &[u8]) -> Result<String, String> {
    let doc = lopdf::Document::load_mem(bytes).map_err(|e| format!("failed to parse PDF: {e}"))?;
    let total_pages = doc.get_pages().len();
    let max_pages = total_pages.min(50) as u32;
    let mut parts = Vec::new();
    for pn in 1..=max_pages {
        if let Ok(text) = doc.extract_text(&[pn]) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    Ok(parts.join("\n\n"))
}

fn extract_spreadsheet_text_from_bytes(bytes: &[u8], max_rows: usize) -> Result<String, String> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e| format!("failed to open spreadsheet: {e}"))?;
    let names = wb.sheet_names().to_vec();
    let mut out = String::new();
    let row_cap = max_rows.clamp(1, 500);
    for name in &names {
        if let Ok(range) = wb.worksheet_range(name) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("# Sheet: {name}\n"));
            for row in range.rows().take(row_cap) {
                let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
                out.push_str(&cells.join("\t"));
                out.push('\n');
            }
        }
    }
    Ok(out.trim_end().to_string())
}

fn read_docx(path: &Path) -> ToolExecutionResult {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "docx_read_error",
                format!("cannot read docx file: {e}"),
                None,
            )
        }
    };
    let digest = sha256_hex(&bytes);
    match extract_docx_text_from_bytes(&bytes) {
        Ok(text) => {
            let lines = text.lines().count();
            ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "format": "docx",
                    "sha256": digest,
                    "lines": lines,
                    "chars": text.chars().count(),
                    "content": text
                }),
                format!(
                    "Read DOCX {} ({lines} lines, {} chars)",
                    path.display(),
                    text.chars().count()
                ),
            )
        }
        Err(e) => ToolExecutionResult::err_structured("docx_parse_error", e, None),
    }
}

fn read_spreadsheet(path: &Path, max_rows: usize) -> ToolExecutionResult {
    let mut wb = match open_workbook_auto(path) {
        Ok(w) => w,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "spreadsheet_read_error",
                format!("failed to open spreadsheet: {e}"),
                None,
            )
        }
    };
    let names = wb.sheet_names().to_vec();
    let mut sheets_out = Vec::new();
    for name in &names {
        if let Ok(range) = wb.worksheet_range(name) {
            let (total_rows, total_cols) = range.get_size();
            let rows: Vec<Vec<String>> = range
                .rows()
                .take(max_rows.clamp(1, 500))
                .map(|r| r.iter().map(|c| c.to_string()).collect())
                .collect();
            sheets_out.push(json!({
                "name": name,
                "total_rows": total_rows,
                "total_cols": total_cols,
                "preview_rows": rows
            }));
        }
    }
    ToolExecutionResult::ok_structured(
        json!({
            "path": path.display().to_string(),
            "format": "spreadsheet",
            "sheets": sheets_out
        }),
        format!("Read spreadsheet {} ({} sheets)", path.display(), names.len()),
    )
}

fn read_pdf(path: &Path, page: Option<usize>) -> ToolExecutionResult {
    let doc = match lopdf::Document::load(path) {
        Ok(d) => d,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "pdf_read_error",
                format!("failed to parse PDF: {e}"),
                None,
            )
        }
    };
    let total_pages = doc.get_pages().len();
    let page_numbers: Vec<u32> = match page {
        Some(p) if p >= 1 && p <= total_pages => vec![p as u32],
        _ => (1..=(total_pages.min(10) as u32)).collect(),
    };
    let mut pages_text = Vec::new();
    for pn in &page_numbers {
        let text = doc.extract_text(&[*pn]).unwrap_or_default();
        pages_text.push(json!({"page": pn, "text": text.trim()}));
    }

    // Best-effort render requested page to PNG attachment when pdftoppm is installed
    let mut media = Vec::new();
    if let Some(p) = page.filter(|&p| p >= 1 && p <= total_pages) {
        if let Some(att) = try_render_pdf_page_png(path, p) {
            media.push(att);
        }
    }

    ToolExecutionResult::ok_structured(
        json!({
            "path": path.display().to_string(),
            "format": "pdf",
            "total_pages": total_pages,
            "extracted_pages": pages_text,
            "rendered_page_images": media.len()
        }),
        format!("Read PDF {} ({total_pages} pages)", path.display()),
    )
    .with_media(media)
}

fn try_render_pdf_page_png(path: &Path, page: usize) -> Option<MediaAttachment> {
    let tmp = std::env::temp_dir().join(format!(
        "nola-pdf-{}-p{page}",
        nola_types::new_id()
    ));
    let prefix = tmp.to_string_lossy().into_owned();
    let ok = Command::new("pdftoppm")
        .args([
            "-png",
            "-r",
            "144",
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-singlefile",
        ])
        .arg(path)
        .arg(&prefix)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        return None;
    }
    let png_path = format!("{prefix}.png");
    let bytes = fs::read(&png_path).ok()?;
    let _ = fs::remove_file(&png_path);
    Some(MediaAttachment {
        mime_type: "image/png".into(),
        filename: format!("page_{page}.png"),
        sha256: sha256_hex(&bytes),
        raw_bytes_len: bytes.len(),
        bytes_base64: B64.encode(&bytes),
    })
}

fn read_audio(path: &Path, sniffed_ext: &str) -> ToolExecutionResult {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "audio_open_error",
                format!("cannot open audio file: {e}"),
                None,
            )
        }
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    } else if !sniffed_ext.is_empty() {
        hint.with_extension(sniffed_ext);
    }
    let probed = match symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "audio_probe_error",
                format!("unsupported audio format: {e}"),
                None,
            )
        }
    };
    let mut format = probed.format;
    let Some(track) = format.default_track() else {
        return ToolExecutionResult::err_structured("audio_no_track", "no audio track found", None);
    };
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = match symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
    {
        Ok(d) => d,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "audio_decoder_error",
                format!("decoder init failed: {e}"),
                None,
            )
        }
    };

    let mut mono: Vec<f32> = Vec::new();
    let max_samples = (sample_rate as usize).saturating_mul(10); // cap 10s for spectral summary
    let mut total_frames: u64 = 0;

    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        if let Ok(decoded) = decoder.decode(&packet) {
            let frames = decoded.frames() as u64;
            total_frames += frames;
            if mono.len() < max_samples {
                let ch = decoded.spec().channels.count().max(1);
                let mut sbuf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                sbuf.copy_interleaved_ref(decoded);
                for frame in sbuf.samples().chunks(ch) {
                    if mono.len() >= max_samples {
                        break;
                    }
                    let sum: f32 = frame.iter().copied().sum();
                    mono.push(sum / ch as f32);
                }
            }
        }
    }

    let duration_secs = if sample_rate > 0 {
        total_frames as f64 / sample_rate as f64
    } else {
        0.0
    };
    let rms = if mono.is_empty() {
        0.0
    } else {
        (mono.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / mono.len() as f64).sqrt()
    };

    // Compute dominant frequency via FFT on up to 4096 samples
    let dominant_hz = if mono.len() >= 256 {
        let n = mono.len().min(4096).next_power_of_two().min(4096);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let mut buf: Vec<Complex<f32>> = mono
            .iter()
            .take(n)
            .map(|&s| Complex::new(s, 0.0))
            .collect();
        buf.resize(n, Complex::new(0.0, 0.0));
        fft.process(&mut buf);
        let (peak_bin, _) = buf
            .iter()
            .take(n / 2)
            .enumerate()
            .skip(1)
            .max_by(|a, b| a.1.norm().total_cmp(&b.1.norm()))
            .unwrap_or((0, &Complex::new(0.0, 0.0)));
        peak_bin as f64 * sample_rate as f64 / n as f64
    } else {
        0.0
    };

    ToolExecutionResult::ok_structured(
        json!({
            "path": path.display().to_string(),
            "format": "audio",
            "sample_rate_hz": sample_rate,
            "channels": channels,
            "duration_secs": (duration_secs * 1000.0).round() / 1000.0,
            "rms_amplitude": (rms * 10_000.0).round() / 10_000.0,
            "dominant_frequency_hz": (dominant_hz * 10.0).round() / 10.0,
        }),
        format!(
            "Analyzed audio {} ({:.2}s, {} Hz, {} ch)",
            path.display(),
            duration_secs,
            sample_rate,
            channels
        ),
    )
}

fn read_image(path: &Path, mime: &str) -> ToolExecutionResult {
    if let Ok(meta) = fs::metadata(path) {
        if meta.len() > 20 * 1024 * 1024 {
            return ToolExecutionResult::err_structured(
                "image_too_large",
                format!("image exceeds 20 MiB limit ({} bytes)", meta.len()),
                None,
            );
        }
    }
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return ToolExecutionResult::err_structured(
                "image_read_error",
                format!("cannot read image: {e}"),
                None,
            )
        }
    };
    let dims: Value = match image::load_from_memory(&bytes) {
        Ok(img) => json!({"width": img.width(), "height": img.height()}),
        Err(_) => Value::Null,
    };
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("image.png")
        .to_string();
    let att = MediaAttachment {
        mime_type: mime.into(),
        filename: filename.clone(),
        sha256: sha256_hex(&bytes),
        raw_bytes_len: bytes.len(),
        bytes_base64: B64.encode(&bytes),
    };
    ToolExecutionResult::ok_structured(
        json!({
            "path": path.display().to_string(),
            "format": "image",
            "mime_type": mime,
            "bytes": bytes.len(),
            "dimensions": dims
        }),
        format!("Loaded image {} ({} bytes)", filename, bytes.len()),
    )
    .with_media(vec![att])
}
