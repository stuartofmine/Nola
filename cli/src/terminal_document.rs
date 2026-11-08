// SPDX-License-Identifier: Apache-2.0
//! Source-backed transcript cells and the mutable Markdown tail. Terminal
//! geometry and ANSI output never become the source of conversation content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TranscriptCell {
    Text(String),
    Markdown(String),
}

impl TranscriptCell {
    pub(crate) fn source(&self) -> &str {
        match self { Self::Text(text) | Self::Markdown(text) => text }
    }
    pub(crate) fn render(&self, width: usize) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Markdown(source) => render_markdown(source, width),
        }
    }
}
impl From<String> for TranscriptCell { fn from(text: String) -> Self { Self::Text(text) } }
impl std::ops::Deref for TranscriptCell {
    type Target = str;
    fn deref(&self) -> &str { self.source() }
}
impl std::borrow::Borrow<str> for TranscriptCell { fn borrow(&self) -> &str { self.source() } }
impl PartialEq<&str> for TranscriptCell { fn eq(&self, other: &&str) -> bool { self.source() == *other } }
impl PartialEq<str> for TranscriptCell { fn eq(&self, other: &str) -> bool { self.source() == other } }

/// Commit only complete paragraphs outside fences. Unlike a byte/line quota,
/// this never cuts a table or fenced code block merely to animate the screen.
pub(crate) fn stable_boundary(source: &str) -> usize {
    let mut fence: Option<(char, usize)> = None;
    let mut offset = 0;
    let mut boundary = 0;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|ch| *ch == '`' || *ch == '~');
        if let Some(marker) = marker {
            let count = trimmed.chars().take_while(|ch| *ch == marker).count();
            if count >= 3 {
                if let Some((open, length)) = fence {
                    if open == marker && count >= length && trimmed[count..].trim().is_empty() { fence = None; }
                } else { fence = Some((marker, count)); }
            }
        }
        offset += line.len();
        if fence.is_none() && line.trim().is_empty() && !source[offset..].trim().is_empty() {
            boundary = offset;
        }
    }
    if boundary > 0 { return boundary; }
    // Ordinary complete lines cannot change each other's inline syntax. Keep
    // the last two lines mutable for a possible setext heading/table delimiter.
    let mut ends = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if !line.ends_with('\n') || line.contains(['`', '~', '*', '_', '[', ']', '|', '<', '>', '\\'])
            || line.trim_start().starts_with(['#', '-', '+', '=']) {
            break;
        }
        offset += line.len();
        ends.push(offset);
    }
    if ends.len() > 2 { ends[ends.len() - 3] } else { 0 }
}

/// Small immutable source cells allow animation to be scheduled, not slept.
pub(crate) fn stable_cells(source: String) -> Vec<TranscriptCell> {
    if source.contains(['`', '~', '*', '_', '[', ']', '|', '<', '>', '\\']) {
        return vec![TranscriptCell::Markdown(source)];
    }
    let mut cells = Vec::new();
    let mut chunk = String::new();
    for (index, line) in source.split_inclusive('\n').enumerate() {
        chunk.push_str(line);
        if index % 4 == 3 { cells.push(TranscriptCell::Markdown(std::mem::take(&mut chunk))); }
    }
    if !chunk.is_empty() { cells.push(TranscriptCell::Markdown(chunk)); }
    cells
}

pub(crate) fn render_markdown(source: &str, width: usize) -> String {
    crate::agent::render_markdown_document(source, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_never_commits_an_open_fence_or_partial_table() {
        assert_eq!(stable_boundary("intro\n\n```rust\n\nlet x = 1;\n"), 7);
        assert_eq!(stable_boundary("| A | B |\n| --- | --- |\n| 1 | 2 |\n"), 0);
        assert_eq!(stable_boundary("~~~\n\nbody\n~~~\n\nnext"), 15);
    }
    #[test]
    fn document_keeps_source_independent_of_rendering() {
        let cell = TranscriptCell::Markdown("**标题**\n\n中文 body".into());
        let _ = cell.render(30);
        let _ = cell.render(100);
        assert_eq!(cell.source(), "**标题**\n\n中文 body");
    }

    #[test]
    fn plain_long_stream_commits_ordered_cells_without_waiting_for_a_blank_line() {
        let source = (0..48).map(|index| format!("line {index}\n")).collect::<String>();
        let boundary = stable_boundary(&source);
        let cells = stable_cells(source[..boundary].to_owned());
        assert!(cells.len() > 1);
        assert!(cells.iter().all(|cell| cell.source().lines().count() <= 4));
        assert_eq!(cells.iter().map(TranscriptCell::source).collect::<String>() + &source[boundary..], source);
    }
}
