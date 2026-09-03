use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

pub(crate) const MAX_LOG_LINES: usize = 2_000;
pub(crate) const MAX_LOG_BYTES: usize = 1024 * 1024;
const TRUNCATION_MARKER: &str = " …[truncated]";

#[derive(Clone, Debug)]
pub struct LogStore {
    inner: Arc<Mutex<LogStoreState>>,
}

#[derive(Debug)]
struct LogStoreState {
    lines: VecDeque<String>,
    rendered: String,
    bytes: usize,
    revision: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LogSnapshot {
    pub revision: u64,
    pub text: String,
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogStoreState {
                lines: VecDeque::new(),
                rendered: String::new(),
                bytes: 0,
                revision: 0,
            })),
        }
    }

    pub fn revision(&self) -> u64 {
        self.inner.lock().map(|state| state.revision).unwrap_or(0)
    }

    pub fn snapshot(&self) -> LogSnapshot {
        self.inner
            .lock()
            .map(|state| LogSnapshot {
                revision: state.revision,
                text: state.rendered.clone(),
            })
            .unwrap_or_default()
    }

    fn append_rendered_line(&self, line: String) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };

        let previous = state.rendered.clone();
        state.bytes += line.len();
        state.rendered.push_str(&line);
        state.lines.push_back(line);

        while state.lines.len() > MAX_LOG_LINES || state.bytes > MAX_LOG_BYTES {
            let Some(oldest) = state.lines.pop_front() else {
                break;
            };
            state.bytes = state.bytes.saturating_sub(oldest.len());
            state.rendered.replace_range(..oldest.len(), "");
        }

        if state.rendered != previous {
            state.revision = state.revision.wrapping_add(1);
        }
    }

    #[cfg(test)]
    fn push_line(&self, line: &str) {
        self.append_rendered_line(render_line(line.as_bytes(), false));
    }
}

pub struct CapturingWriter<W> {
    store: LogStore,
    output: W,
    fragment: Vec<u8>,
    fragment_truncated: bool,
}

impl<W> CapturingWriter<W> {
    pub(crate) fn new(store: LogStore, output: W) -> Self {
        Self {
            store,
            output,
            fragment: Vec::new(),
            fragment_truncated: false,
        }
    }

    fn append_fragment_byte(&mut self, byte: u8) {
        if self.fragment_truncated {
            return;
        }
        if self.fragment.len() < MAX_LOG_BYTES {
            self.fragment.push(byte);
        } else {
            self.fragment_truncated = true;
        }
    }

    fn finish_fragment(&mut self) {
        if self.fragment.last() == Some(&b'\r') {
            self.fragment.pop();
        }
        let line = render_line(&self.fragment, self.fragment_truncated);
        self.store.append_rendered_line(line);
        self.fragment.clear();
        self.fragment_truncated = false;
    }
}

impl<W: Write + Send> Write for CapturingWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish_fragment();
            } else {
                self.append_fragment_byte(byte);
            }
        }

        // Capturing is deliberately independent from the best-effort tee. A GUI
        // process commonly has no stderr handle, so an output failure is ignored.
        let _ = self.output.write_all(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.output.flush();
        Ok(())
    }
}

fn render_line(bytes: &[u8], force_truncated: bool) -> String {
    let text = String::from_utf8_lossy(bytes);
    let needs_truncation = force_truncated || text.len().saturating_add(1) > MAX_LOG_BYTES;
    if !needs_truncation {
        let mut rendered = text.into_owned();
        rendered.push('\n');
        return rendered;
    }

    let content_limit = MAX_LOG_BYTES
        .saturating_sub(TRUNCATION_MARKER.len())
        .saturating_sub(1);
    let prefix = utf8_prefix(&text, content_limit);
    let mut rendered = String::with_capacity(prefix.len() + TRUNCATION_MARKER.len() + 1);
    rendered.push_str(prefix);
    rendered.push_str(TRUNCATION_MARKER);
    rendered.push('\n');
    rendered
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn init_logging() -> LogStore {
    let store = LogStore::new();
    let writer = CapturingWriter::new(store.clone(), io::stderr());
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "warn,companion=info,lovense=debug,buttplug_client=info,buttplug_server=info",
    ))
    .format_timestamp_millis()
    .format_module_path(true)
    .target(env_logger::Target::Pipe(Box::new(writer)))
    .init();
    store
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[derive(Default)]
    struct RecordingWriter {
        bytes: Vec<u8>,
        fail: bool,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail {
                return Err(io::Error::other("tee failed"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct PartialWriter {
        sender: std::sync::mpsc::Sender<Vec<u8>>,
    }

    impl Write for PartialWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(1);
            self.sender
                .send(bytes[..count].to_vec())
                .map_err(|_| io::Error::other("tee receiver dropped"))?;
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn snapshots_start_empty_and_revision_changes_only_for_visible_content() {
        let store = LogStore::new();
        assert_eq!(store.snapshot(), LogSnapshot::default());
        store.push_line("first");
        let first = store.snapshot();
        assert_eq!(first.text, "first\n");
        assert_eq!(first.revision, 1);
        assert_eq!(store.snapshot().revision, first.revision);
    }
    #[test]
    fn revision_does_not_change_when_eviction_leaves_visible_text_unchanged() {
        let store = LogStore::new();
        for _ in 0..=MAX_LOG_LINES {
            store.push_line("same");
        }
        let revision = store.revision();
        store.push_line("same");
        assert_eq!(store.revision(), revision);
        assert_eq!(store.snapshot().text.lines().count(), MAX_LOG_LINES);
    }

    #[test]
    fn complete_lines_are_ordered_and_oldest_lines_are_evicted() {
        let store = LogStore::new();
        for index in 0..=MAX_LOG_LINES {
            store.push_line(&format!("line-{index}"));
        }
        let snapshot = store.snapshot();
        assert!(!snapshot.text.contains("line-0\n"));
        assert!(snapshot.text.starts_with("line-1\n"));
        assert!(snapshot.text.ends_with(&format!("line-{MAX_LOG_LINES}\n")));
        assert_eq!(snapshot.text.lines().count(), MAX_LOG_LINES);
    }

    #[test]
    fn byte_bound_evicts_oldest_complete_lines() {
        let store = LogStore::new();
        let line = "x".repeat(1023);
        let count = MAX_LOG_BYTES / (line.len() + 1) + 2;
        for index in 0..count {
            store.push_line(&format!("{line}{index}"));
        }
        let snapshot = store.snapshot();
        assert!(snapshot.text.len() <= MAX_LOG_BYTES);
        assert!(snapshot.text.lines().count() < count);
        assert!(snapshot.text.contains(&format!("{line}{}", count - 1)));
    }

    #[test]
    fn oversized_multibyte_line_is_valid_utf8_and_marked() {
        let store = LogStore::new();
        store.push_line(&"界".repeat(MAX_LOG_BYTES));
        let snapshot = store.snapshot();
        assert!(snapshot.text.len() <= MAX_LOG_BYTES);
        assert!(snapshot.text.contains("[truncated]"));
        assert!(std::str::from_utf8(snapshot.text.as_bytes()).is_ok());
    }

    #[test]
    fn writer_assembles_fragments_and_multiple_crlf_records() {
        let store = LogStore::new();
        let mut writer = CapturingWriter::new(store.clone(), RecordingWriter::default());
        writer.write_all(b"one\r\ntw").unwrap();
        assert_eq!(store.snapshot().text, "one\n");
        writer.write_all(b"o\r\nthree\nfour").unwrap();
        assert_eq!(store.snapshot().text, "one\ntwo\nthree\n");
        writer.write_all(b"\n").unwrap();
        assert_eq!(store.snapshot().text, "one\ntwo\nthree\nfour\n");
    }

    #[test]
    fn writer_bounds_incomplete_fragment_and_waits_for_newline() {
        let store = LogStore::new();
        let mut writer = CapturingWriter::new(store.clone(), RecordingWriter::default());
        writer.write_all(&vec![b'a'; MAX_LOG_BYTES + 32]).unwrap();
        assert!(store.snapshot().text.is_empty());
        writer.write_all(b"\n").unwrap();
        let snapshot = store.snapshot();
        assert!(snapshot.text.contains("[truncated]"));
    }

    #[test]
    fn writer_capture_survives_failing_tee() {
        let store = LogStore::new();
        let mut writer = CapturingWriter::new(
            store.clone(),
            RecordingWriter {
                fail: true,
                ..Default::default()
            },
        );
        assert_eq!(writer.write(b"captured\n").unwrap(), 9);
        assert_eq!(store.snapshot().text, "captured\n");
    }

    #[test]
    fn writer_tees_every_original_byte_even_when_output_writes_partially() {
        let store = LogStore::new();
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut writer = CapturingWriter::new(store.clone(), PartialWriter { sender });
        let bytes = b"one\r\ntwo\n";
        assert_eq!(writer.write(bytes).unwrap(), bytes.len());
        drop(writer);
        let output: Vec<u8> = receiver.into_iter().flatten().collect();
        assert_eq!(output, bytes);
        assert_eq!(store.snapshot().text, "one\ntwo\n");
    }
}
