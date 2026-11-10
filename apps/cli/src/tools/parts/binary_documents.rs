const MAX_BINARY_DOCUMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 500;
const MAX_WORKBOOK_SHEETS: usize = 128;
const MAX_WORKBOOK_CELLS: usize = 200_000;
const MAX_DECODED_AUDIO_SECONDS: usize = 15 * 60;
const AUDIO_FFT_SIZE: usize = 4096;

/// Explicit bounded visual read. No shell interpolation or implicit installer;
/// the same authorized path check as read_file precedes this operation.
fn render_pdf_page(target: &Path, page: u64, ctx: &ToolContext) -> ToolOutcome {
    let source = tool_try!(File::open(target).map_err(|err| error(err.to_string())));
    let mut bytes = Vec::new();
    tool_try!(source.take(MAX_BINARY_DOCUMENT_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|err| error(err.to_string())));
    if bytes.len() as u64 > MAX_BINARY_DOCUMENT_BYTES {
        return error("PDF visual read exceeds the 64 MiB source limit".into());
    }
    let directory = tool_try!(tempfile::tempdir().map_err(|err| error(err.to_string())));
    let input = directory.path().join("input.pdf");
    tool_try!(fs::write(&input, bytes).map_err(|err| error(err.to_string())));
    let mut command = Command::new("pdftoppm");
    command.args(["-f", &page.to_string(), "-l", &page.to_string(), "-singlefile",
        "-scale-to", "1600", "-png"]).arg(&input)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return missing_program_dependency(
            "pdftoppm", "pdf_page_rendering",
            json!({"apt-get":"poppler-utils","apk":"poppler-utils","dnf":"poppler-utils","brew":"poppler"}),
            json!({"tool":"read_file","arguments":{"path":target,"pdf_page":page}})),
        Err(err) => return error(format!("PDF renderer could not start: {err}. No page was inspected; installing a package may not resolve this error.")),
    };
    let output = tool_try!(wait_for_child_output(child, &ctx.cancellation,
        Duration::from_secs(20), "PDF page rendering"));
    if !output.status.success() || !output.stdout.starts_with(b"\x89PNG\r\n\x1a\n") {
        return error(format!("PDF page {page} could not be rendered: {}",
            String::from_utf8_lossy(&output.stderr).chars().take(1000).collect::<String>()));
    }
    if output.stdout.len() > 16 * 1024 * 1024 {
        return error("Rendered PDF page exceeds the 16 MiB image limit".into());
    }
    let snapshot = tool_try!(ctx.output_store.persist_media(&output.stdout)
        .map_err(|err| error(format!("PDF page snapshot failed: {err}"))));
    let mut outcome = ok(format!("Rendered page {page} of {} to a PNG bounded to 1600 pixels. Only this page is supplied; other pages are not inspected. Image input support is required.", target.display()),
        format!("Rendered PDF page {page}")).with_file_artifact(&snapshot, "read");
    if let Some(artifact) = outcome.artifacts.last_mut() {
        artifact.mime_type = Some("image/png".into());
        artifact.name = format!("page-{page}.png");
    }
    outcome
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinaryDocumentKind {
    Pdf,
    CompoundDocument,
    ZipContainer,
    Audio,
    Unknown,
}

fn binary_document_kind(bytes: &[u8]) -> BinaryDocumentKind {
    if bytes.starts_with(b"%PDF-") {
        BinaryDocumentKind::Pdf
    } else if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        BinaryDocumentKind::CompoundDocument
    } else if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        BinaryDocumentKind::ZipContainer
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        || bytes.starts_with(b"fLaC")
        || bytes.starts_with(b"OggS")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"))
        || bytes.get(4..8) == Some(b"ftyp")
    {
        BinaryDocumentKind::Audio
    } else {
        BinaryDocumentKind::Unknown
    }
}

fn binary_document_kind_from_file(target: &Path) -> Result<BinaryDocumentKind, ToolOutcome> {
    let mut header = [0u8; 12];
    let mut file = File::open(target).map_err(|err| error(format!("Read failed: {err}")))?;
    let count = file
        .read(&mut header)
        .map_err(|err| error(format!("Read failed: {err}")))?;
    Ok(binary_document_kind(&header[..count]))
}

fn extract_binary_document_text(
    target: &Path,
    max_output_bytes: usize,
    show_line_numbers: bool,
) -> Result<(String, usize), ToolOutcome> {
    let size = fs::metadata(target)
        .map_err(|err| error(format!("Read failed: {err}")))?
        .len();
    if size > MAX_BINARY_DOCUMENT_BYTES {
        return Err(error(format!(
            "Read failed: binary document is {size} bytes; the on-demand extraction limit is {MAX_BINARY_DOCUMENT_BYTES} bytes"
        )));
    }
    match binary_document_kind_from_file(target)? {
        BinaryDocumentKind::Pdf => {
            extract_pdf_document(target, max_output_bytes, show_line_numbers)
        }
        BinaryDocumentKind::CompoundDocument | BinaryDocumentKind::ZipContainer => {
            extract_workbook_document(target, max_output_bytes, show_line_numbers)
        }
        BinaryDocumentKind::Audio => {
            extract_audio_document(target, max_output_bytes, show_line_numbers)
        }
        BinaryDocumentKind::Unknown => Err(error(
            "Read failed: unsupported binary format (recognized formats: PDF, XLS, XLSX, XLSB, ODS, MP3, WAV, FLAC, Ogg/Vorbis, AAC/M4A)"
                .to_string(),
        )),
    }
}

fn extract_audio_document(
    target: &Path,
    max_output_bytes: usize,
    show_line_numbers: bool,
) -> Result<(String, usize), ToolOutcome> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let source = File::open(target).map_err(|err| error(format!("Read failed: {err}")))?;
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = target.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|err| {
            error(format!(
                "Read failed: detected audio but probing failed: {err}"
            ))
        })?;
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| {
        error("Read failed: audio container has no decodable default track".to_string())
    })?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|err| error(format!("Read failed: audio codec is unsupported: {err}")))?;
    let mut mono_samples = Vec::<f32>::new();
    let mut sample_rate = codec_params.sample_rate.unwrap_or(0);
    let mut channels = codec_params
        .channels
        .map(|value| value.count())
        .unwrap_or(0);
    let mut decoded_truncated = false;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => {
                return Err(error(format!(
                    "Read failed: audio packet decode failed: {err}"
                )))
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(err) => return Err(error(format!("Read failed: audio decode failed: {err}"))),
        };
        sample_rate = decoded.spec().rate;
        channels = decoded.spec().channels.count();
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        samples.copy_interleaved_ref(decoded);
        let decoded_frame_limit = (sample_rate as usize).saturating_mul(MAX_DECODED_AUDIO_SECONDS);
        for frame in samples.samples().chunks(channels.max(1)) {
            mono_samples.push(frame.iter().copied().sum::<f32>() / frame.len().max(1) as f32);
            if mono_samples.len() >= decoded_frame_limit {
                decoded_truncated = true;
                break;
            }
        }
        if decoded_truncated {
            break;
        }
    }
    if sample_rate == 0 || mono_samples.is_empty() {
        return Err(error(
            "Read failed: audio decoder produced no samples".to_string(),
        ));
    }
    let duration = mono_samples.len() as f64 / sample_rate as f64;
    let window_count = ((max_output_bytes / 110).clamp(8, 128)).min(mono_samples.len());
    let analysis_span = mono_samples.len().saturating_sub(AUDIO_FFT_SIZE).max(1);
    let hop = analysis_span.div_ceil(window_count).max(1);
    let mut text = format!(
        "codec: {:?}\nsample_rate_hz: {sample_rate}\nchannels: {channels}\ndecoded_duration_seconds: {duration:.6}\ndecoded_mono_frames: {}\ndecoded_truncated: {decoded_truncated}\nanalysis: Hann-window FFT; frequency candidates are strongest local spectral peaks and can include harmonics\nwindow_columns: center_seconds,rms,peak_frequencies_hz\n",
        codec_params.codec,
        mono_samples.len()
    );
    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(AUDIO_FFT_SIZE);
    for start in (0..analysis_span).step_by(hop).take(window_count) {
        let end = (start + AUDIO_FFT_SIZE).min(mono_samples.len());
        let samples = &mono_samples[start..end];
        let square_sum = samples.iter().map(|value| value * value).sum::<f32>();
        let rms = (square_sum / samples.len() as f32).sqrt();
        let mut bins = (0..AUDIO_FFT_SIZE)
            .map(|index| {
                let sample = samples.get(index).copied().unwrap_or(0.0);
                let hann = 0.5
                    - 0.5
                        * (std::f32::consts::TAU * index as f32 / (AUDIO_FFT_SIZE - 1) as f32)
                            .cos();
                rustfft::num_complex::Complex::new(sample * hann, 0.0)
            })
            .collect::<Vec<_>>();
        fft.process(&mut bins);
        let min_bin = ((40.0 * AUDIO_FFT_SIZE as f32 / sample_rate as f32) as usize).max(1);
        let max_bin = ((5_000.0 * AUDIO_FFT_SIZE as f32 / sample_rate as f32) as usize)
            .min(AUDIO_FFT_SIZE / 2 - 1);
        let magnitudes = bins.iter().map(|bin| bin.norm_sqr()).collect::<Vec<_>>();
        let mut peaks = (min_bin..=max_bin)
            .filter(|index| {
                magnitudes[*index] > magnitudes[index - 1]
                    && magnitudes[*index] >= magnitudes[index + 1]
            })
            .map(|index| (magnitudes[index], index))
            .collect::<Vec<_>>();
        peaks.sort_unstable_by(|left, right| right.0.total_cmp(&left.0));
        let frequencies = peaks
            .iter()
            .take(8)
            .map(|(_, index)| {
                format!(
                    "{:.2}",
                    *index as f32 * sample_rate as f32 / AUDIO_FFT_SIZE as f32
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        text.push_str(&format!(
            "{:.6},{rms:.8},{frequencies}\n",
            (start + samples.len() / 2) as f64 / sample_rate as f64
        ));
    }
    bounded_document_output(
        target,
        "audio/decoded-signal-summary",
        text,
        max_output_bytes,
        show_line_numbers,
    )
}

fn bounded_document_output(
    target: &Path,
    format: &str,
    text: String,
    max_output_bytes: usize,
    show_line_numbers: bool,
) -> Result<(String, usize), ToolOutcome> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut end = normalized.len().min(max_output_bytes);
    while end > 0 && !normalized.is_char_boundary(end) {
        end -= 1;
    }
    let selected = &normalized[..end];
    let truncated = end < normalized.len();
    let body = format_numbered_lines(selected, 1, show_line_numbers);
    let suffix = if truncated {
        "\n...[extracted text truncated; raise max_bytes up to 1000000 for more]"
    } else {
        ""
    };
    Ok((
        format!(
            "# {}\nformat: {format}\nextraction: on_demand\n\n{body}{suffix}",
            target.display()
        ),
        selected.len(),
    ))
}

fn extract_pdf_document(
    target: &Path,
    max_output_bytes: usize,
    show_line_numbers: bool,
) -> Result<(String, usize), ToolOutcome> {
    let document = lopdf::Document::load(target).map_err(|err| {
        error(format!(
            "Read failed: detected application/pdf but the document could not be opened: {err}"
        ))
    })?;
    let page_count = document.get_pages().len();
    if page_count > MAX_PDF_PAGES {
        return Err(error(format!(
            "Read failed: PDF has {page_count} pages; the on-demand extraction limit is {MAX_PDF_PAGES} pages"
        )));
    }
    // Decode one page at a time and stop when the caller's output budget is
    // already satisfied. Whole-document extraction made a small read request
    // pay the CPU and allocation cost of every page before truncating it.
    let mut text = String::new();
    let mut extracted_pages = 0usize;
    let mut page_errors = 0usize;
    let decode_budget = max_output_bytes.saturating_mul(2).max(4_096);
    for page_number in document.get_pages().keys().copied() {
        let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document.extract_text(&[page_number])
        }));
        match extracted {
            Ok(Ok(page_text)) => {
                extracted_pages += 1;
                if !page_text.trim().is_empty() {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(&format!("[PDF page {page_number}]\n{page_text}"));
                }
            }
            Ok(Err(_)) | Err(_) => page_errors += 1,
        }
        if text.len() >= decode_budget {
            text.push_str(&format!(
                "\n\n[PDF extraction stopped after {extracted_pages} of {page_count} pages because the requested output budget was reached]"
            ));
            break;
        }
    }
    if text.trim().is_empty() {
        return Err(error(format!(
            "Read failed: detected application/pdf but no selectable text was extracted from {extracted_pages} pages ({page_errors} page errors). The PDF may be image-only, encrypted, or malformed; OCR is required for image-only pages"
        )));
    }
    bounded_document_output(
        target,
        "application/pdf",
        text,
        max_output_bytes,
        show_line_numbers,
    )
}

fn extract_workbook_document(
    target: &Path,
    max_output_bytes: usize,
    show_line_numbers: bool,
) -> Result<(String, usize), ToolOutcome> {
    use calamine::{open_workbook_auto, Reader};

    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_workbook_auto(target)
    }))
    .map_err(|_| {
        error(
            "Read failed: detected an OLE/ZIP container but the workbook parser rejected malformed document structures"
                .to_string(),
        )
    })?;
    let mut workbook = opened.map_err(|err| {
        error(format!(
            "Read failed: detected an OLE/ZIP binary container but it is not a supported Excel/OpenDocument workbook: {err}"
        ))
    })?;
    let sheet_names = workbook.sheet_names().to_vec();
    let mut rendered = String::new();
    let mut cell_count = 0usize;
    for sheet_name in sheet_names.iter().take(MAX_WORKBOOK_SHEETS) {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str("## Sheet: ");
        rendered.push_str(sheet_name);
        rendered.push('\n');
        let range = workbook.worksheet_range(sheet_name).map_err(|err| {
            error(format!(
                "Read failed: could not read worksheet {sheet_name:?}: {err}"
            ))
        })?;
        for row in range.rows() {
            if cell_count >= MAX_WORKBOOK_CELLS || rendered.len() >= max_output_bytes {
                break;
            }
            let values = row
                .iter()
                .map(|cell| {
                    cell.to_string()
                        .replace('\t', " ")
                        .replace(['\r', '\n'], " ")
                })
                .collect::<Vec<_>>();
            cell_count = cell_count.saturating_add(values.len());
            rendered.push_str(&values.join("\t"));
            rendered.push('\n');
        }
        if cell_count >= MAX_WORKBOOK_CELLS || rendered.len() >= max_output_bytes {
            rendered.push_str("...[workbook extraction limit reached]\n");
            break;
        }
    }
    if sheet_names.len() > MAX_WORKBOOK_SHEETS {
        rendered.push_str(&format!(
            "...[{} worksheets omitted]\n",
            sheet_names.len() - MAX_WORKBOOK_SHEETS
        ));
    }
    bounded_document_output(
        target,
        "application/vnd.ms-excel-or-opendocument-spreadsheet",
        rendered,
        max_output_bytes,
        show_line_numbers,
    )
}

#[cfg(test)]
mod binary_document_tests {
    use super::*;
    #[test]
    fn recognizes_formats_by_signature_without_extension() {
        assert_eq!(binary_document_kind(b"%PDF-1.7"), BinaryDocumentKind::Pdf);
        assert_eq!(
            binary_document_kind(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]),
            BinaryDocumentKind::CompoundDocument
        );
        assert_eq!(
            binary_document_kind(b"PK\x03\x04anything"),
            BinaryDocumentKind::ZipContainer
        );
        assert_eq!(
            binary_document_kind(b"\x89PNG\r\n\x1a\n"),
            BinaryDocumentKind::Unknown
        );
        assert_eq!(
            binary_document_kind(b"RIFF\x24\0\0\0WAVE"),
            BinaryDocumentKind::Audio
        );
    }

    #[test]
    fn bounded_output_preserves_utf8_boundaries() {
        let path = Path::new("report.pdf");
        let (text, selected) =
            bounded_document_output(path, "application/pdf", "甲乙丙".into(), 4, false)
                .expect("bounded text");
        assert_eq!(selected, 3);
        assert!(text.contains("甲"));
        assert!(!text.contains("乙"));
        assert!(text.contains("truncated"));
    }

    fn fixture_file(bytes: &[u8], suffix: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::Builder::new()
            .suffix(suffix)
            .tempfile()
            .expect("temporary document");
        file.write_all(bytes).expect("write fixture");
        file
    }

    #[test]
    fn extracts_pdf_text_without_external_processes() {
        let file = fixture_file(include_bytes!("../test_fixtures/basic.pdf"), ".bin");
        let (text, _) =
            extract_binary_document_text(file.path(), 16_000, false).expect("extract PDF");
        assert!(text.contains("format: application/pdf"));
        assert!(text.contains("Nola PDF Text"));
    }

    #[test]
    fn pdf_signature_uses_extraction_even_when_container_bytes_are_utf8() {
        let file = fixture_file(include_bytes!("../test_fixtures/basic.pdf"), ".pdf");
        let args = Map::new();
        let observed = read_file_text(file.path(), &args, false, 16_000, FileReadPresentationBudget::direct())
            .expect("read PDF through public read path");
        assert!(observed.text.contains("Nola PDF Text"));
        assert!(!observed.text.contains("xref"));
        assert!(observed.observation.is_none(), "extracted text is not raw-file coverage");
    }

    #[test]
    fn extracts_xlsx_cells_without_trusting_extension() {
        let file = fixture_file(include_bytes!("../test_fixtures/basic.xlsx"), ".bin");
        let (text, _) =
            extract_binary_document_text(file.path(), 16_000, false).expect("extract workbook");
        assert!(text.contains("## Sheet: Metrics"));
        assert!(text.contains("Item\tValue"));
        assert!(text.contains("Alpha\t42"));
    }

    #[test]
    fn rejects_oversized_binary_documents_before_parsing() {
        let file = fixture_file(b"%PDF-1.7", ".pdf");
        file.as_file()
            .set_len(MAX_BINARY_DOCUMENT_BYTES + 1)
            .expect("create sparse oversized fixture");
        let outcome = extract_binary_document_text(file.path(), 16_000, false)
            .expect_err("oversized document must fail");
        assert!(outcome.text.contains("on-demand extraction limit"));
    }

    #[test]
    fn decodes_audio_to_bounded_signal_windows() {
        let sample_rate = 8_000u32;
        let samples = (0..sample_rate)
            .map(|index| {
                let phase = index as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32;
                (phase.sin() * i16::MAX as f32 * 0.25) as i16
            })
            .collect::<Vec<_>>();
        let data_bytes = (samples.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        let file = fixture_file(&wav, ".bin");
        let (text, _) =
            extract_binary_document_text(file.path(), 16_000, false).expect("decode WAV");
        assert!(text.contains("format: audio/decoded-signal-summary"));
        assert!(text.contains("sample_rate_hz: 8000"));
        assert!(text.contains("decoded_duration_seconds: 1.000000"));
        assert!(text.contains("window_columns: center_seconds"));
        assert!(text.contains("439.45"));
    }

    #[test]
    fn recognizes_supported_audio_containers_from_content() {
        for header in [
            b"ID3\x04\x00\x00\x00\x00\x00\x00\x00\x00".as_slice(),
            b"\xff\xfb\x90\x64\x00\x00\x00\x00\x00\x00\x00\x00".as_slice(),
            b"fLaC\x00\x00\x00\x22\x00\x00\x00\x00".as_slice(),
            b"OggS\x00\x02\x00\x00\x00\x00\x00\x00".as_slice(),
            b"RIFF\x24\x00\x00\x00WAVE".as_slice(),
            b"\x00\x00\x00\x18ftypM4A \x00\x00\x00\x00".as_slice(),
        ] {
            assert_eq!(binary_document_kind(header), BinaryDocumentKind::Audio);
        }
    }
}
