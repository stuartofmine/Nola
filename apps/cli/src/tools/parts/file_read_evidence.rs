// SPDX-License-Identifier: Apache-2.0
// Same-open-file observations for the Run-local read-before-write ledger.
// These are content preconditions, never permission grants.

static FILE_READ_SCOPE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct FileReadEvidenceScope {
    key: String,
    run_id: Option<String>,
}

impl FileReadEvidenceScope {
    fn new(run_id: Option<String>) -> Self {
        let generation = FILE_READ_SCOPE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self { key: format!("file-read-{}-{generation:x}", std::process::id()), run_id }
    }
}

impl Drop for FileReadEvidenceScope {
    fn drop(&mut self) {
        if let Some(states) = FILE_ACCESS_STATE.get() {
            if let Ok(mut states) = states.lock() {
                states.retain(|(scope, _), _| scope != &self.key);
            }
        }
    }
}

fn current_file_read_scope_key() -> Option<String> {
    CURRENT_FILE_READ_SCOPE.with(|scope| scope.borrow().current().cloned())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileReadVersion {
    sha256: String,
    modified: SystemTime,
    total_lines: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FileReadCoverage {
    Full,
    Lines { first: usize, last: usize },
    None,
}

#[derive(Clone, Debug)]
struct FileReadObservation {
    path: PathBuf,
    scope: Option<String>,
    version: FileReadVersion,
    coverage: FileReadCoverage,
}

struct FileReadSelection {
    first_line: usize,
    line_count: Option<usize>,
    max_bytes: usize,
}

struct FileReadSnapshot {
    text: Option<String>,
    observation: Option<FileReadObservation>,
    file_bytes: u64,
    total_lines: usize,
    byte_truncated: bool,
}

struct ObservedFileText {
    text: String,
    /// Exact returned UTF-8 lines, without renderer headings or line numbers.
    raw_content: Option<String>,
    observation: Option<FileReadObservation>,
    next_start_line: Option<usize>,
    presentation_exhausted: bool,
}

#[derive(Clone, Copy)]
struct FileReadPresentationBudget {
    chars: usize,
    bytes: usize,
}

impl FileReadPresentationBudget {
    fn direct() -> Self { Self { chars: MAX_TOOL_RESULT_CHARS, bytes: usize::MAX } }
    fn fits(self, text: &str) -> bool {
        text.len() <= self.bytes && text.chars().count() <= self.chars
    }
}

fn file_read_metadata_matches(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.dev(), before.ino(), before.ctime(), before.ctime_nsec())
            != (after.dev(), after.ino(), after.ctime(), after.ctime_nsec()) {
            return false;
        }
    }
    true
}

/// Hash every byte from this descriptor while retaining only the requested
/// bounded text. Never reopen the path to attach a different version's hash.
fn read_file_snapshot(target: &Path, selection: FileReadSelection) -> Result<FileReadSnapshot, ToolOutcome> {
    use sha2::{Digest, Sha256};
    let path = file_state_key(target);
    let scope = current_file_read_scope_key();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(target).map_err(|err| error(format!("Read failed: {err}")))?;
    let before = file.metadata().map_err(|err| error(format!("Read failed: {err}")))?;
    if !before.is_file() {
        return Err(error("Read requires a regular file".into()));
    }
    let modified = before.modified().map_err(|err| error(format!("Read mtime failed: {err}")))?;
    let last_line = selection.line_count.map(|count|
        selection.first_line.saturating_add(count).saturating_sub(1)).unwrap_or(usize::MAX);
    let mut digest = Sha256::new();
    let mut data = Vec::new();
    let mut header = Vec::new();
    let mut buffer = [0u8; 65_536];
    let mut file_bytes = 0u64;
    let mut eligible_bytes = 0u64;
    let mut line = 1usize;
    let mut newlines = 0usize;
    let mut ends_with_newline = false;
    loop {
        let read = file.read(&mut buffer).map_err(|err| error(format!("Read failed: {err}")))?;
        if read == 0 { break; }
        let bytes = &buffer[..read];
        let header_remaining = 12usize.saturating_sub(header.len());
        header.extend(bytes.iter().take(header_remaining).copied());
        if binary_document_kind(&header) != BinaryDocumentKind::Unknown {
            // Extraction has its own bounded source reader and produces no
            // raw-text observation. Do not hash an arbitrarily large binary
            // before that reader can enforce its size limit.
            return Ok(FileReadSnapshot { text: None, observation: None,
                file_bytes: before.len(), total_lines: 0, byte_truncated: false });
        }
        digest.update(bytes);
        file_bytes = file_bytes.saturating_add(read as u64);
        ends_with_newline = bytes.last() == Some(&b'\n');
        for segment in bytes.split_inclusive(|byte| *byte == b'\n') {
            if line >= selection.first_line && line <= last_line {
                eligible_bytes = eligible_bytes.saturating_add(segment.len() as u64);
                let take = selection.max_bytes.saturating_sub(data.len()).min(segment.len());
                data.extend_from_slice(&segment[..take]);
            }
            if segment.last() == Some(&b'\n') {
                line = line.saturating_add(1);
                newlines = newlines.saturating_add(1);
            }
        }
    }
    let after = file.metadata().map_err(|err| error(format!("Read failed: {err}")))?;
    if !file_read_metadata_matches(&before, &after) || after.len() != file_bytes {
        return Err(error(format!("File changed during read. Read it again: {}", target.display())));
    }
    let total_lines = newlines.saturating_add(usize::from(file_bytes > 0 && !ends_with_newline));
    let byte_truncated = eligible_bytes > data.len() as u64;
    // A byte cap may split one UTF-8 scalar; return its complete prefix, not a
    // lossy replacement. Invalid source text and derived binary extraction do
    // not constitute raw-file text coverage.
    let text = match String::from_utf8(data) {
        Ok(text) => Some(text),
        Err(err) if byte_truncated && err.utf8_error().error_len().is_none() => {
            let valid = err.utf8_error().valid_up_to();
            let mut bytes = err.into_bytes();
            bytes.truncate(valid);
            Some(String::from_utf8(bytes).expect("validated UTF-8 prefix"))
        }
        Err(_) => None,
    };
    let observation = text.as_ref().map(|text| {
        let complete_lines = text.bytes().filter(|byte| *byte == b'\n').count()
            + usize::from(!text.is_empty() && !text.ends_with('\n') && !byte_truncated);
        let coverage = if selection.first_line == 1 && !byte_truncated && eligible_bytes == file_bytes {
            FileReadCoverage::Full
        } else if complete_lines > 0 {
            FileReadCoverage::Lines { first: selection.first_line,
                last: selection.first_line.saturating_add(complete_lines).saturating_sub(1) }
        } else {
            FileReadCoverage::None
        };
        FileReadObservation { path, scope, version: FileReadVersion {
            sha256: format!("{:x}", digest.finalize()), modified, total_lines,
        }, coverage }
    });
    Ok(FileReadSnapshot { text, observation, file_bytes, total_lines, byte_truncated })
}

/// Render complete raw lines within the actual model-output budget, before
/// generic result preservation/clamping. Coverage is derived from retained
/// source lines, never parsed back from presentation text or a saved artifact.
fn render_file_read_snapshot(
    target: &Path,
    mut snapshot: FileReadSnapshot,
    start_line: usize,
    show_line_numbers: bool,
    auto_large_preview: bool,
    budget: FileReadPresentationBudget,
) -> Result<ObservedFileText, ToolOutcome> {
    let selected = snapshot.text.as_deref().expect("raw text snapshot");
    let complete_count = match snapshot.observation.as_ref().map(|observation| &observation.coverage) {
        Some(FileReadCoverage::Full) => snapshot.total_lines,
        Some(FileReadCoverage::Lines { first, last }) => last.saturating_sub(*first).saturating_add(1),
        _ => 0,
    };
    let lines = selected.split_inclusive('\n').take(complete_count).collect::<Vec<_>>();
    let next_line = |kept: usize| {
        let next = start_line.saturating_add(kept);
        (next <= snapshot.total_lines).then_some(next)
    };
    let render = |kept: usize| {
        let end_line = start_line.saturating_add(kept).saturating_sub(1);
        let raw = lines[..kept].concat();
        let body = format_numbered_lines(&raw, start_line, show_line_numbers);
        let mut text = format!("# {}\nlines: {}-{} of {}\n\n{}",
            target.display(), start_line, end_line, snapshot.total_lines, body);
        if snapshot.file_bytes > MAX_WRITE_BYTES as u64 {
            text = format!("file_metadata: bytes={} large_file=true\nedit_strategy: read exact line ranges, then use edit_file_lines for observed ranges; existing files up to {MAX_EDITED_FILE_BYTES} bytes can be edited safely.\n\n{text}", snapshot.file_bytes);
        }
        if snapshot.byte_truncated { text.push_str("\n...[byte truncated]"); }
        if kept < complete_count { text.push_str("\n...[display truncated]"); }
        if auto_large_preview { text.push_str("\n...[large file preview: read focused original-file line ranges]"); }
        if let Some(next) = next_line(kept) {
            if kept == 0 {
                text.push_str(&format!("\nline_not_returned: {next}; complete line exceeds raw/display budget; no line coverage. Use bounded source inspection; do not skip this line or treat it as read."));
            }
            text.push_str(&format!("\nnext_start_line: {next}\nnext_action: read_file on the same original path with start_line={next} and a focused line_count; do not follow full_output_ref for file-read coverage."));
        }
        text
    };
    // Try all lines first: reaching EOF removes the continuation overhead.
    let all = render(lines.len());
    let (kept, text) = if budget.fits(&all) {
        (lines.len(), all)
    } else {
        let empty = render(0);
        if !budget.fits(&empty) {
            return Ok(ObservedFileText { text: String::new(), raw_content: None, observation: None,
                next_start_line: next_line(0), presentation_exhausted: true });
        }
        let mut kept = 0;
        let mut text = empty;
        for count in 1..=lines.len() {
            let candidate = render(count);
            if !budget.fits(&candidate) { break; }
            kept = count;
            text = candidate;
        }
        (kept, text)
    };
    let next_start_line = next_line(kept);
    let observation = if kept == 0 && snapshot.total_lines != 0 {
        None
    } else {
        snapshot.observation.take().map(|mut observation| {
            observation.coverage = if start_line == 1 && next_start_line.is_none() {
                FileReadCoverage::Full
            } else if kept > 0 {
                FileReadCoverage::Lines { first: start_line,
                    last: start_line.saturating_add(kept).saturating_sub(1) }
            } else { FileReadCoverage::None };
            observation
        })
    };
    Ok(ObservedFileText { text, raw_content: Some(lines[..kept].concat()), observation,
        next_start_line, presentation_exhausted: false })
}

fn record_file_read_observation(observation: FileReadObservation) {
    let Some(scope) = observation.scope else { return; };
    // A delayed observation cannot become evidence for a different Run.
    if current_file_read_scope_key().as_deref() != Some(scope.as_str()) { return; }
    let state = FILE_ACCESS_STATE.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut guard) = state.lock() else { return; };
    let entry = guard.entry((scope, observation.path)).or_insert_with(|| FileAccessState {
        version: observation.version.clone(), full_read: false, ranges: Vec::new(),
    });
    if entry.version != observation.version {
        entry.version = observation.version;
        entry.full_read = false;
        entry.ranges.clear();
    }
    match observation.coverage {
        FileReadCoverage::Full => { entry.full_read = true; entry.ranges.clear(); }
        FileReadCoverage::Lines { first, last } if !entry.full_read => {
            entry.ranges.push((first, last));
            entry.ranges.sort_unstable();
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for range in entry.ranges.drain(..) {
                if let Some(previous) = merged.last_mut() {
                    if range.0 <= previous.1.saturating_add(1) {
                        previous.1 = previous.1.max(range.1);
                        continue;
                    }
                }
                merged.push(range);
            }
            if merged.first() == Some(&(1, entry.version.total_lines)) {
                entry.full_read = true;
                merged.clear();
            }
            entry.ranges = merged;
        }
        _ => {}
    }
}

/// The generic tool boundary subsequently replaces oversized output with a
/// preview. Saved-but-unseen bodies are not raw-file read evidence. Commit only
/// whole, unmodified results; do not reverse-engineer coverage from preview JSON.
fn record_returned_file_observations(text: &str, observations: impl IntoIterator<Item = FileReadObservation>) {
    if text.chars().count() > MAX_TOOL_RESULT_CHARS
        || nola_domain::tool_evidence::ModelToolEvidence::text(text) != text {
        return;
    }
    for observation in observations { record_file_read_observation(observation); }
}
