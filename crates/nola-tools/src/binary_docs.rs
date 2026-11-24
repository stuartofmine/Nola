// SPDX-License-Identifier: Apache-2.0
//! Native binary document readers for `read_file`:
//! - Spreadsheets (`.xlsx`, `.xls`, `.ods`) via `calamine`
//! - PDFs (`.pdf`) via `lopdf` text extraction + optional page-render (`pdftoppm`/`mutool`/`gs`)
//! - Audio (`.wav`, `.mp3`, `.flac`, `.ogg`, `.m4a`) via `symphonia` + `rustfft` spectral analysis
//! - Images (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`) -> validated `MediaAttachment`

use std::fs;
use std::path::Path;
use std::process::Command;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use calamine::{open_workbook_auto, Reader};
use nola_types::{sha256_hex, MediaAttachment, ToolExecutionResult};
use rustfft::{num_complex::Complex, FftPlanner};
use serde_json::{json, Value};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Dispatch binary file extensions to rich native extractors; returns `None` for normal UTF-8 text.
pub fn try_read_binary_document(
    path: &Path,
    page: Option<usize>,
    max_rows: usize,
) -> Option<ToolExecutionResult> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())?;

    match ext.as_str() {
        "xlsx" | "xls" | "xlsm" | "xlsb" | "ods" => Some(read_spreadsheet(path, max_rows)),
        "pdf" => Some(read_pdf(path, page)),
        "wav" | "mp3" | "flac" | "ogg" | "m4a" | "aac" => Some(read_audio(path)),
        "png" | "jpg" | "jpeg" | "gif" | "webp" => Some(read_image(path, &ext)),
        _ => None,
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

fn read_audio(path: &Path) -> ToolExecutionResult {
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

fn read_image(path: &Path, ext: &str) -> ToolExecutionResult {
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
    let mime = match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
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
