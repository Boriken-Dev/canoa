//! The latest lines this process logged, kept in memory for the Logs tab of the application's panel: the `canoa` program's `tracing`
//! output and, when the host runs inside an application, what that application's own log routing hears.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the log is the host's"
)]

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Mutex, PoisonError};

/// The most lines kept; the oldest go first.
pub const KEPT_LINES: usize = 2000;

/// The longest line kept, in bytes; a longer one is cut.
const MAX_LINE: usize = 4096;

/// Without the colour codes a terminal's log carries.
fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Lines kept, the oldest dropped first.
struct Ring {
    lines: VecDeque<String>,
    capacity: usize,
}

impl Ring {
    const fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            capacity,
        }
    }

    fn push(&mut self, line: &str) {
        let mut line = plain(line.trim_end_matches(['\r', '\n']));
        if line.len() > MAX_LINE {
            let mut end = MAX_LINE;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            line.truncate(end);
        }
        if self.lines.len() >= self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn tail(&self, count: usize) -> String {
        let from = self.lines.len().saturating_sub(count);
        let mut out = String::new();
        for line in self.lines.iter().skip(from) {
            out.push_str(line);
            out.push('\n');
        }
        out
    }
}

static LINES: Mutex<Ring> = Mutex::new(Ring::new(KEPT_LINES));

/// Keeps `line` as the latest.
pub fn record(line: &str) {
    LINES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(line);
}

/// The latest `count` lines, oldest first, each ended by a newline.
#[must_use]
pub fn tail(count: usize) -> String {
    LINES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .tail(count)
}

/// A writer for the `tracing` subscriber that writes to standard error and keeps each line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tee;

impl Write for Tee {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for line in String::from_utf8_lossy(bytes).lines() {
            record(line);
        }
        std::io::stderr().write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stderr().flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Tee {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_lines_are_kept_oldest_first_without_colour_codes_and_the_oldest_go_first() {
        let mut ring = Ring::new(3);
        ring.push("\u{1b}[32mINFO\u{1b}[0m one\n");
        ring.push("two");
        assert_eq!(ring.tail(5), "INFO one\ntwo\n");
        ring.push("three");
        ring.push("four");
        assert_eq!(ring.tail(10), "two\nthree\nfour\n");
        assert_eq!(ring.tail(1), "four\n");
        ring.push(&"x".repeat(MAX_LINE + 10));
        assert_eq!(ring.tail(1).len(), MAX_LINE + 1);
    }
}
