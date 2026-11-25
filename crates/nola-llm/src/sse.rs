// SPDX-License-Identifier: Apache-2.0
//! Streaming Server-Sent Events (SSE) parser.

use std::io::BufRead;

use crate::LlmError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Pull-based SSE reader over any `BufRead`.
pub struct SseReader<'a> {
    inner: &'a mut dyn BufRead,
    line_buf: String,
    saw_first_line: bool,
}

impl<'a> SseReader<'a> {
    pub fn new(inner: &'a mut dyn BufRead) -> Self {
        Self {
            inner,
            line_buf: String::new(),
            saw_first_line: false,
        }
    }

    pub fn next_event(&mut self) -> Result<Option<SseEvent>, LlmError> {
        let mut event_name: Option<String> = None;
        let mut data_lines: Vec<String> = Vec::new();
        loop {
            self.line_buf.clear();
            let n = self.inner.read_line(&mut self.line_buf)?;
            if n == 0 {
                if data_lines.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(SseEvent {
                    event: event_name,
                    data: data_lines.join("\n"),
                }));
            }
            let mut line = self.line_buf.trim_end_matches(&['\r', '\n'][..]);
            if !self.saw_first_line {
                self.saw_first_line = true;
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            if line.is_empty() {
                if !data_lines.is_empty() {
                    return Ok(Some(SseEvent {
                        event: event_name,
                        data: data_lines.join("\n"),
                    }));
                }
                event_name = None;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                event_name = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let value = rest.strip_prefix(' ').unwrap_or(rest);
                data_lines.push(value.to_string());
            }
        }
    }
}
