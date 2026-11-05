//! What a service prints, kept and shown (FR-SV-05, docs/SERVICES.md "Logs", D-025).
//!
//! **The child writes straight into its log file**: `LUNA_HOME/logs/<service-id>.log`, opened for append and given
//! to the process as both stdout and stderr. lunaRun then *tails* the file. Compared with reading pipes this
//! means: the file survives a crash of lunaRun and an adopted service ([`Supervisor::adopt`](super::Supervisor::adopt))
//! simply keeps writing to it (a pipe whose reader died kills or silences its writer); a slow reader can never
//! make a service block on a full pipe; and nothing is lost between two runs of lunaRun. The price is that stdout
//! and stderr arrive as one stream in the order they were written (the source is `output`), and that a running
//! service cannot be rotated by renaming: at the size limit lunaRun **copies the file aside and truncates it**
//! (what `logrotate` calls `copytruncate`), and what the service writes between the copy and the truncate is lost.
//! A new run of a service starts by renaming the old files, which loses nothing.
//!
//! Every source (the service's own output, plus log files the service writes itself, such as MySQL's error log)
//! has its own ring buffer of the last lines, and one pump task per running service reads them all every tick and
//! sends **one batch per tick** to the [`LogEventSink`]: at most 10 events a second for a service, however much
//! it prints.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::paths::LunaHome;

/// The source name of a service's own stdout and stderr (one stream, see the module docs).
pub const OUTPUT: &str = "output";

/// How much of the log is kept and how it is sent. The defaults are the ones in SERVICES section 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    /// Size at which a running service's log file is copied aside and truncated (5 MB).
    pub max_bytes: u64,
    /// Files kept per service: the log and its rotated copies (`.log`, `.log.1`, `.log.2`).
    pub files: usize,
    /// Lines remembered per source for the viewer.
    pub ring_lines: usize,
    /// How often the files are read, which is how often a batch can be sent.
    pub interval: Duration,
    /// A line longer than this many bytes is cut (a service that prints a megabyte without a newline).
    pub max_line_bytes: usize,
    /// A last line that never got its newline (a prompt) is shown after the file has been quiet this long.
    pub partial_after: Duration,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            max_bytes: 5 * 1024 * 1024,
            files: 3,
            ring_lines: 2_000,
            interval: Duration::from_millis(100),
            max_line_bytes: 16 * 1024,
            partial_after: Duration::from_millis(500),
        }
    }
}

/// One line of a service's log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogLine {
    /// `output`, or the label of a log file the service writes itself (`MySQL error log`).
    pub source: String,
    /// Without the line ending. Bytes that are not UTF-8 are shown as the replacement character.
    pub line: String,
    /// When lunaRun read it, in milliseconds since 1970-01-01 UTC.
    #[ts(type = "number")]
    pub ts: u64,
}

/// The `service:log` event: what a service printed since the last one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogBatch {
    pub id: String,
    pub lines: Vec<LogLine>,
}

/// Receives batches of log lines. Closures work too.
pub trait LogEventSink: Send + Sync + 'static {
    fn emit(&self, batch: LogBatch);
}

impl<F: Fn(LogBatch) + Send + Sync + 'static> LogEventSink for F {
    fn emit(&self, batch: LogBatch) {
        self(batch);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

/// `LUNA_HOME/logs/<id>.log`.
pub(super) fn log_path(home: &LunaHome, id: &str) -> PathBuf {
    home.logs_dir().join(format!("{id}.log"))
}

fn numbered(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

/// Make room for the log to become `.1`: the oldest copy goes, the others move up by one. Afterwards `path.1` does
/// not exist (when `files > 1`). Deleting goes through `safe_remove`, like every deletion in `LUNA_HOME`.
fn shift_copies(home: &LunaHome, path: &Path, files: usize) -> io::Result<()> {
    if files <= 1 {
        return Ok(());
    }
    let last = files - 1;
    home.safe_remove(&numbered(path, last))
        .map_err(|err| io::Error::other(err.message))?;
    for n in (1..last).rev() {
        let from = numbered(path, n);
        if from.exists() {
            fs::rename(&from, numbered(path, n + 1))?;
        }
    }
    Ok(())
}

/// A new run of a service: the existing log becomes `.1`, `.1` becomes `.2`, and so on, and the oldest is deleted.
/// Nothing is lost that fits in the files kept. Not called for an automatic restart (a crash loop would push the
/// output of the crash out of the files in seconds) nor for an adoption.
pub(super) fn rotate_for_new_run(home: &LunaHome, id: &str, files: usize) -> io::Result<()> {
    let path = log_path(home, id);
    let Ok(meta) = fs::metadata(&path) else {
        return Ok(());
    };
    if meta.len() == 0 {
        return Ok(());
    }
    shift_copies(home, &path, files)?;
    if files > 1 {
        fs::rename(&path, numbered(&path, 1))
    } else {
        File::create(&path).map(drop)
    }
}

/// The log file, opened for append and created if needed: what a service process gets as stdout and stderr.
pub(super) fn open_output(home: &LunaHome, id: &str) -> io::Result<File> {
    fs::create_dir_all(home.logs_dir())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(home, id))
}

/// A line lunaRun writes into the log itself (`[lunaRun] started (pid 1234)`), so that the log explains why the
/// output stops. Best effort.
pub(super) fn note(home: &LunaHome, id: &str, text: &str) {
    if let Ok(mut file) = open_output(home, id) {
        let _ = writeln!(file, "[lunaRun] {text}");
    }
}

/// Copy a log that reached its size limit aside and empty it, while the service keeps it open for append. What the
/// service writes between the copy and the truncate is lost, which is the price of not asking it to reopen its
/// file (D-025).
fn copy_and_truncate(home: &LunaHome, path: &Path, files: usize) -> io::Result<()> {
    if files > 1 {
        shift_copies(home, path, files)?;
        fs::copy(path, numbered(path, 1))?;
    }
    OpenOptions::new().write(true).open(path)?.set_len(0)
}

// ---------------------------------------------------------------------------------------------
// Rings: the last lines of every source, for the viewer
// ---------------------------------------------------------------------------------------------

/// The last lines of every source of every service. Outlives the runs: the lines of a service that has stopped are
/// still there to look at.
pub(super) struct LogStore {
    ring_lines: usize,
    rings: Mutex<HashMap<String, HashMap<String, VecDeque<LogLine>>>>,
}

impl LogStore {
    pub(super) fn new(ring_lines: usize) -> Self {
        Self {
            ring_lines,
            rings: Mutex::new(HashMap::new()),
        }
    }

    /// Forget a service's lines (it starts a new run, with fresh files).
    pub(super) fn clear(&self, id: &str) {
        lock(&self.rings).remove(id);
    }

    pub(super) fn push(&self, id: &str, lines: &[LogLine]) {
        let mut rings = lock(&self.rings);
        let sources = rings.entry(id.to_owned()).or_default();
        for line in lines {
            let ring = sources.entry(line.source.clone()).or_default();
            ring.push_back(line.clone());
            while ring.len() > self.ring_lines {
                ring.pop_front();
            }
        }
    }

    /// Make sure a source exists even before it has a line, so a viewer can show its tab.
    pub(super) fn ensure_source(&self, id: &str, source: &str) {
        lock(&self.rings)
            .entry(id.to_owned())
            .or_default()
            .entry(source.to_owned())
            .or_default();
    }

    /// The last `max` lines of a source, oldest first.
    pub(super) fn lines(&self, id: &str, source: &str, max: usize) -> Vec<LogLine> {
        let rings = lock(&self.rings);
        let Some(ring) = rings.get(id).and_then(|sources| sources.get(source)) else {
            return Vec::new();
        };
        ring.iter()
            .skip(ring.len().saturating_sub(max))
            .cloned()
            .collect()
    }

    /// The sources a service has: `output` first, then the others by name.
    pub(super) fn sources(&self, id: &str) -> Vec<String> {
        let rings = lock(&self.rings);
        let mut names: Vec<String> = rings
            .get(id)
            .map(|sources| sources.keys().cloned().collect())
            .unwrap_or_default();
        names.sort_by(|a, b| (a != OUTPUT, a).cmp(&(b != OUTPUT, b)));
        names
    }
}

// ---------------------------------------------------------------------------------------------
// Tailing one file
// ---------------------------------------------------------------------------------------------

/// Most bytes read from one file in one tick, so a service that floods its log cannot keep the pump busy forever.
const MAX_READ_PER_TICK: u64 = 1024 * 1024;
/// How far back the last lines are looked for when a file is picked up (the lines it already has).
const SEED_BYTES: u64 = 512 * 1024;

fn to_text(bytes: &[u8], max: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= max {
        return text.into_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} ...[line cut]", &text[..end])
}

fn strip_line_ending(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\r").unwrap_or(bytes)
}

/// The last `max_lines` lines a file already has, without reading all of a big file.
pub(super) fn last_lines(path: &Path, max_lines: usize, max_line_bytes: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return Vec::new();
    };
    let start = len.saturating_sub(SEED_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(SEED_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let mut slice = bytes.as_slice();
    if start > 0 {
        // The first line is probably cut in half.
        slice = slice
            .iter()
            .position(|&b| b == b'\n')
            .map_or(&[][..], |at| &slice[at + 1..]);
    }
    let body = slice.strip_suffix(b"\n").unwrap_or(slice);
    if body.is_empty() && slice.is_empty() {
        return Vec::new();
    }
    let lines: Vec<String> = body
        .split(|&b| b == b'\n')
        .map(|line| to_text(strip_line_ending(line), max_line_bytes))
        .collect();
    let skip = lines.len().saturating_sub(max_lines);
    lines.into_iter().skip(skip).collect()
}

/// One file being followed.
struct Tailer {
    source: String,
    path: PathBuf,
    /// Copy aside and truncate at the size limit. Only for the service's own output, which lunaRun owns.
    rotate: bool,
    pos: u64,
    /// Bytes after the last line ending: a line that is not finished.
    partial: Vec<u8>,
    /// The line being read is longer than the limit: drop the rest of it.
    skipping: bool,
    quiet_since: Option<Instant>,
}

impl Tailer {
    /// Starts at the end of the file, so only what is written from now on is new. The lines it already has are
    /// returned for the ring (`seed`).
    fn new(source: &str, path: &Path, rotate: bool, config: &LogConfig) -> (Self, Vec<String>) {
        let pos = fs::metadata(path).map_or(0, |meta| meta.len());
        let seed = last_lines(path, config.ring_lines, config.max_line_bytes);
        (
            Self {
                source: source.to_owned(),
                path: path.to_path_buf(),
                rotate,
                pos,
                partial: Vec::new(),
                skipping: false,
                quiet_since: None,
            },
            seed,
        )
    }

    fn line(&self, bytes: &[u8], config: &LogConfig, ts: u64) -> LogLine {
        LogLine {
            source: self.source.clone(),
            line: to_text(strip_line_ending(bytes), config.max_line_bytes),
            ts,
        }
    }

    /// Read what was added since the last time. `finishing`: a line without an ending is shown now, not later.
    async fn poll(&mut self, home: &LunaHome, config: &LogConfig, finishing: bool) -> Vec<LogLine> {
        let mut out = Vec::new();
        let ts = now_millis();
        let Ok(meta) = tokio::fs::metadata(&self.path).await else {
            // Not there (yet, or any more): nothing to read.
            return out;
        };
        let len = meta.len();
        if len < self.pos {
            // Emptied behind our back (someone deleted its content): start again from the top.
            self.pos = 0;
            self.partial.clear();
            self.skipping = false;
        }
        let before = self.pos;
        if len > self.pos {
            self.read_new(len, config, ts, &mut out).await;
        }
        let grew = self.pos > before;

        // A last line that never gets its ending is shown once the file has been quiet for a moment since the last
        // write, or at once when the service ended.
        if grew {
            self.quiet_since = Some(Instant::now());
        }
        if self.partial.is_empty() {
            self.quiet_since = None;
        } else if finishing
            || self
                .quiet_since
                .is_some_and(|since| since.elapsed() >= config.partial_after)
        {
            let bytes = std::mem::take(&mut self.partial);
            out.push(self.line(&bytes, config, ts));
            self.quiet_since = None;
        }

        if self.rotate && self.pos >= len && len >= config.max_bytes {
            let (home, path, files) = (home.clone(), self.path.clone(), config.files);
            let rotated =
                tokio::task::spawn_blocking(move || copy_and_truncate(&home, &path, files))
                    .await
                    .is_ok_and(|result| result.is_ok());
            if rotated {
                self.pos = 0;
            }
        }
        out
    }

    async fn read_new(&mut self, len: u64, config: &LogConfig, ts: u64, out: &mut Vec<LogLine>) {
        let Ok(mut file) = tokio::fs::File::open(&self.path).await else {
            return;
        };
        if file.seek(SeekFrom::Start(self.pos)).await.is_err() {
            return;
        }
        let wanted = (len - self.pos).min(MAX_READ_PER_TICK);
        let mut chunk = Vec::new();
        let Ok(read) = (&mut file).take(wanted).read_to_end(&mut chunk).await else {
            return;
        };
        self.pos += read as u64;

        for &byte in &chunk {
            if byte == b'\n' {
                if self.skipping {
                    self.skipping = false;
                } else {
                    let bytes = std::mem::take(&mut self.partial);
                    out.push(self.line(&bytes, config, ts));
                }
            } else if !self.skipping {
                self.partial.push(byte);
                if self.partial.len() > config.max_line_bytes {
                    // Far too long: show what fits and drop the rest of this line.
                    let bytes = std::mem::take(&mut self.partial);
                    out.push(self.line(&bytes, config, ts));
                    self.skipping = true;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The pump: one task per running service
// ---------------------------------------------------------------------------------------------

/// What a pump needs to know about one source.
pub(super) struct Source {
    pub label: String,
    pub path: PathBuf,
    pub rotate: bool,
}

/// Reads all the log files of a running service every tick and sends one batch per tick.
pub(super) struct Pump {
    cancel: CancellationToken,
    flush: mpsc::UnboundedSender<oneshot::Sender<()>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Pump {
    /// Pick up the files, fill the rings with the lines they already have (so a viewer opened now shows them), and
    /// start reading what is written from now on.
    pub(super) fn start(
        runtime: &tokio::runtime::Handle,
        home: LunaHome,
        id: String,
        sources: Vec<Source>,
        config: LogConfig,
        store: Arc<LogStore>,
        sink: Arc<dyn LogEventSink>,
    ) -> Self {
        let mut tailers = Vec::new();
        for source in &sources {
            store.ensure_source(&id, &source.label);
            let (tailer, seed) = Tailer::new(&source.label, &source.path, source.rotate, &config);
            let seeded: Vec<LogLine> = seed
                .into_iter()
                .map(|line| LogLine {
                    source: source.label.clone(),
                    line,
                    ts: now_millis(),
                })
                .collect();
            store.push(&id, &seeded);
            tailers.push(tailer);
        }

        let cancel = CancellationToken::new();
        let (flush, mut flushes) = mpsc::unbounded_channel::<oneshot::Sender<()>>();
        let token = cancel.clone();
        let task = runtime.spawn(async move {
            let mut tailers = tailers;
            loop {
                let mut done = false;
                let mut waiting: Option<oneshot::Sender<()>> = None;
                tokio::select! {
                    biased;
                    () = token.cancelled() => done = true,
                    request = flushes.recv() => waiting = request,
                    () = tokio::time::sleep(config.interval) => {}
                }
                let finishing = done || waiting.is_some();
                let mut lines = Vec::new();
                for tailer in &mut tailers {
                    lines.extend(tailer.poll(&home, &config, finishing).await);
                }
                if !lines.is_empty() {
                    store.push(&id, &lines);
                    sink.emit(LogBatch {
                        id: id.clone(),
                        lines,
                    });
                }
                if let Some(waiting) = waiting {
                    let _ = waiting.send(());
                }
                if done {
                    break;
                }
            }
        });
        Self {
            cancel,
            flush,
            task: Mutex::new(Some(task)),
        }
    }

    /// Read everything that is in the files now, and return when it has been delivered. Used when the process has
    /// ended, so the last lines are there before anyone is told it ended.
    pub(super) async fn flush(&self) {
        let (done, wait) = oneshot::channel();
        if self.flush.send(done).is_ok() {
            let _ = wait.await;
        }
    }

    /// Stop reading, after one last read, and return when the reading task is gone. Safe to call twice. A service
    /// must not be startable again before this has returned: the old reader would read the new run's file too.
    pub(super) async fn shutdown(&self) {
        self.cancel.cancel();
        let task = lock(&self.task).take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn home() -> (TempDir, LunaHome) {
        let tmp = TempDir::new().unwrap();
        let home = LunaHome::at(tmp.path().join("lüna home")).unwrap();
        home.ensure_layout().unwrap();
        (tmp, home)
    }

    fn small() -> LogConfig {
        LogConfig {
            max_bytes: 1_000,
            files: 3,
            ring_lines: 5,
            interval: Duration::from_millis(10),
            max_line_bytes: 20,
            partial_after: Duration::from_millis(30),
        }
    }

    async fn read(tailer: &mut Tailer, home: &LunaHome, config: &LogConfig) -> Vec<String> {
        tailer
            .poll(home, config, false)
            .await
            .into_iter()
            .map(|l| l.line)
            .collect()
    }

    fn append(path: &Path, bytes: &[u8]) {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }

    #[test]
    fn long_lines_are_cut_on_a_character_boundary() {
        assert_eq!(to_text(b"short", 20), "short");
        let cut = to_text("ääääääääääääääääääää".as_bytes(), 7); // 2 bytes each
        assert!(
            cut.starts_with("äää") && cut.ends_with("...[line cut]"),
            "{cut}"
        );
        assert_eq!(to_text(b"abc\xff\xfedef", 20), "abc\u{fffd}\u{fffd}def");
    }

    #[tokio::test]
    async fn lines_are_split_whatever_the_chunks_and_the_endings() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("t.log");
        append(&path, b"");
        let config = small();
        let (mut tailer, seed) = Tailer::new("output", &path, false, &config);
        assert!(seed.is_empty());

        append(&path, b"one\r\ntwo\n\nthr");
        assert_eq!(read(&mut tailer, &home, &config).await, ["one", "two", ""]);
        append(&path, b"ee\nfour");
        assert_eq!(read(&mut tailer, &home, &config).await, ["three"]);
        append(&path, b"\n");
        assert_eq!(read(&mut tailer, &home, &config).await, ["four"]);
        assert!(read(&mut tailer, &home, &config).await.is_empty());
    }

    #[tokio::test]
    async fn a_character_split_between_two_reads_is_not_broken() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("t.log");
        append(&path, b"");
        let config = small();
        let (mut tailer, _) = Tailer::new("output", &path, false, &config);
        let bytes = "grüne\n".as_bytes();
        append(&path, &bytes[..3]); // the first byte of the umlaut
        assert!(read(&mut tailer, &home, &config).await.is_empty());
        append(&path, &bytes[3..]);
        assert_eq!(read(&mut tailer, &home, &config).await, ["grüne"]);
    }

    #[tokio::test]
    async fn a_line_without_an_ending_shows_after_a_quiet_moment_or_when_the_service_ends() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("t.log");
        append(&path, b"");
        let config = small();
        let (mut tailer, _) = Tailer::new("output", &path, false, &config);
        append(&path, b"mysql> ");
        assert!(
            read(&mut tailer, &home, &config).await.is_empty(),
            "too early"
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(read(&mut tailer, &home, &config).await, ["mysql> "]);

        append(&path, b"last words");
        let ended = tailer.poll(&home, &config, true).await;
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].line, "last words", "shown at once when finishing");
    }

    #[tokio::test]
    async fn a_line_that_is_far_too_long_is_cut_and_the_next_line_is_fine() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("t.log");
        append(&path, b"");
        let config = small();
        let (mut tailer, _) = Tailer::new("output", &path, false, &config);
        append(&path, &[b'x'; 500]);
        let first = read(&mut tailer, &home, &config).await;
        assert_eq!(first.len(), 1);
        assert!(first[0].ends_with("...[line cut]"), "{first:?}");
        append(&path, &[b'y'; 500]);
        append(&path, b"\nnext\n");
        assert_eq!(
            read(&mut tailer, &home, &config).await,
            ["next"],
            "the rest of the long line was dropped"
        );
    }

    #[tokio::test]
    async fn a_file_that_was_emptied_is_read_again_from_the_top() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("t.log");
        append(&path, b"old line\n");
        let config = small();
        let (mut tailer, seed) = Tailer::new("output", &path, false, &config);
        assert_eq!(seed, ["old line"]);
        fs::write(&path, b"").unwrap();
        append(&path, b"fresh\n");
        assert_eq!(read(&mut tailer, &home, &config).await, ["fresh"]);
    }

    #[tokio::test]
    async fn a_file_that_is_not_there_yields_nothing_and_appears_later() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("later.log");
        let config = small();
        let (mut tailer, seed) = Tailer::new("mysql", &path, false, &config);
        assert!(seed.is_empty());
        assert!(read(&mut tailer, &home, &config).await.is_empty());
        append(&path, b"hello\n");
        assert_eq!(read(&mut tailer, &home, &config).await, ["hello"]);
    }

    #[test]
    fn the_last_lines_of_a_file_are_found_without_reading_it_all() {
        let (_tmp, home) = home();
        let path = home.logs_dir().join("big.log");
        let mut text = String::new();
        for n in 0..100_000 {
            text.push_str(&format!("line {n}\r\n"));
        }
        fs::write(&path, &text).unwrap();
        let lines = last_lines(&path, 3, 100);
        assert_eq!(lines, ["line 99997", "line 99998", "line 99999"]);
        assert!(last_lines(&home.logs_dir().join("none.log"), 3, 100).is_empty());
        fs::write(&path, b"").unwrap();
        assert!(last_lines(&path, 3, 100).is_empty());
        fs::write(&path, b"only\n").unwrap();
        assert_eq!(last_lines(&path, 3, 100), ["only"]);
        fs::write(&path, b"no ending").unwrap();
        assert_eq!(last_lines(&path, 3, 100), ["no ending"]);
    }

    #[test]
    fn a_new_run_shifts_the_files_and_drops_the_oldest() {
        let (_tmp, home) = home();
        let path = log_path(&home, "db");
        let names = |home: &LunaHome| {
            let mut names: Vec<String> = fs::read_dir(home.logs_dir())
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let read = |p: &Path| fs::read_to_string(p).unwrap();

        for run in 1..=4 {
            rotate_for_new_run(&home, "db", 3).unwrap();
            fs::write(&path, format!("run {run}")).unwrap();
        }

        assert_eq!(names(&home), ["db.log", "db.log.1", "db.log.2"]);
        assert_eq!(read(&path), "run 4");
        assert_eq!(read(&numbered(&path, 1)), "run 3");
        assert_eq!(
            read(&numbered(&path, 2)),
            "run 2",
            "run 1 was the oldest and is gone"
        );

        // An empty log is not rotated into a copy (nothing to keep).
        fs::write(&path, b"").unwrap();
        rotate_for_new_run(&home, "db", 3).unwrap();
        assert_eq!(read(&numbered(&path, 1)), "run 3");
    }

    #[test]
    fn keeping_one_file_means_no_copies() {
        let (_tmp, home) = home();
        let path = log_path(&home, "db");
        fs::write(&path, "old").unwrap();
        rotate_for_new_run(&home, "db", 1).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert!(!numbered(&path, 1).exists());
    }

    #[tokio::test]
    async fn the_output_is_copied_aside_and_emptied_at_the_size_limit() {
        let (_tmp, home) = home();
        let path = log_path(&home, "db");
        let config = small(); // 1,000 bytes, 3 files
        // The service holds its log open for append, as a child does.
        let mut service = open_output(&home, "db").unwrap();
        let (mut tailer, _) = Tailer::new(OUTPUT, &path, true, &config);

        let mut all = Vec::new();
        for n in 0..40 {
            writeln!(service, "line number {n:03} with some padding").unwrap();
            all.extend(read(&mut tailer, &home, &config).await);
        }

        assert_eq!(all.len(), 40, "every line was seen, in order");
        assert!(
            fs::metadata(&path).unwrap().len() < 1_000 + 100,
            "the log never grows far past the limit"
        );
        assert!(numbered(&path, 1).exists(), "a copy was made");
        assert!(
            !numbered(&path, 3).exists(),
            "and no more than three files are kept"
        );
        // The service keeps writing into the same file after the truncate.
        writeln!(service, "after the rotation").unwrap();
        assert!(
            read(&mut tailer, &home, &config)
                .await
                .contains(&"after the rotation".to_owned())
        );
    }

    #[test]
    fn rings_keep_the_last_lines_per_source_and_list_the_sources_output_first() {
        let store = LogStore::new(3);
        let line = |source: &str, n: u32| LogLine {
            source: source.to_owned(),
            line: format!("{n}"),
            ts: 1,
        };
        store.push("db", &(0..5).map(|n| line("output", n)).collect::<Vec<_>>());
        store.push("db", &[line("error log", 9)]);
        store.ensure_source("db", "access log");

        let kept: Vec<String> = store
            .lines("db", "output", 100)
            .into_iter()
            .map(|l| l.line)
            .collect();
        assert_eq!(kept, ["2", "3", "4"], "the cap is per source");
        assert_eq!(
            store.lines("db", "output", 2).len(),
            2,
            "and the newest come last"
        );
        assert_eq!(store.sources("db"), ["output", "access log", "error log"]);
        assert!(
            store.lines("db", "nothing", 5).is_empty()
                && store.lines("other", "output", 5).is_empty()
        );

        store.clear("db");
        assert!(store.sources("db").is_empty());
    }
}
