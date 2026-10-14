//! Stream command logs into one immutable run record. Never infer success from test counts.
use crate::{
    context::Context,
    error::{Error, Result},
    snapshot::{self, Entry, Manifest},
    store::{Metadata, RecordKind, Store},
};
use command_group::CommandGroup;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tempfile::NamedTempFile;

const MAX_DESCRIPTOR: u64 = 4 * 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Counts {
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    pub stream: String,
    pub line: u64,
    pub byte_offset: u64,
    pub context: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub parser: String,
    pub parse_status: String,
    pub counts: Option<Counts>,
    pub observed_counts: Option<Counts>,
    pub failure_locations_total: u64,
    pub failures: Vec<Failure>,
    pub failure_locations_omitted: u64,
    pub context_truncated: bool,
    pub unclassified_failure_tail: Vec<Failure>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub path: String,
    pub status: String,
    pub observed: Option<Entry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema: u8,
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub started_at: u64,
    pub finished_at: u64,
    pub duration_ms: u64,
    pub command_exit: Option<i32>,
    pub signal: Option<i32>,
    pub wrapper_exit: u8,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub logs_complete: bool,
    #[serde(default)]
    pub limits: Option<RunLimits>,
    #[serde(default)]
    pub termination_reason: Option<String>,
    pub analysis: Analysis,
    pub snapshot: Option<String>,
    pub source_before: Option<Manifest>,
    pub source_after: Value,
    pub artifacts: Vec<Artifact>,
    pub artifact_relationship: String,
}

pub struct RunOptions {
    pub command: Vec<String>,
    pub parser: String,
    pub snapshot: Option<String>,
    pub inputs: Vec<PathBuf>,
    pub artifacts: Vec<PathBuf>,
    pub inherit_stdin: bool,
    pub limits: RunLimits,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub max_log_bytes: Option<u64>,
    pub timeout_seconds: Option<u64>,
    pub min_free_bytes: Option<u64>,
}
impl RunConfig {
    pub fn merge(self, fallback: Self) -> Self {
        Self {
            max_log_bytes: self.max_log_bytes.or(fallback.max_log_bytes),
            timeout_seconds: self.timeout_seconds.or(fallback.timeout_seconds),
            min_free_bytes: self.min_free_bytes.or(fallback.min_free_bytes),
        }
    }
    pub fn resolve(self) -> Result<RunLimits> {
        let limits = RunLimits {
            max_log_bytes: self.max_log_bytes.unwrap_or(64 * 1024 * 1024),
            timeout_seconds: self.timeout_seconds.unwrap_or(600),
            min_free_bytes: self.min_free_bytes.unwrap_or(512 * 1024 * 1024),
        };
        if !(1..=1024 * 1024 * 1024).contains(&limits.max_log_bytes)
            || !(1..=86400).contains(&limits.timeout_seconds)
        {
            return Err(Error::invalid("run max_log_bytes must be 1..1073741824 and timeout_seconds 1..86400; unbounded capture is not supported."));
        }
        Ok(limits)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLimits {
    pub max_log_bytes: u64,
    pub timeout_seconds: u64,
    pub min_free_bytes: u64,
}
struct CaptureState {
    bytes: AtomicU64,
    // 1: log cap, 2: I/O failure; first capture failure wins.
    reason: AtomicU8,
    cancel: AtomicBool,
}
impl CaptureState {
    fn stop(&self, reason: u8) {
        let _ = self
            .reason
            .compare_exchange(0, reason, Ordering::SeqCst, Ordering::SeqCst);
    }
}
fn capture(
    mut pipe: impl Read,
    mut file: File,
    state: Arc<CaptureState>,
    cap: u64,
) -> io::Result<()> {
    let result = (|| {
        let mut buffer = [0_u8; 64 * 1024];
        while !state.cancel.load(Ordering::SeqCst) {
            let n = match pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => return Err(e),
            };
            let previous = state
                .bytes
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |used| {
                    Some(used.saturating_add(n as u64).min(cap))
                })
                .expect("infallible byte reservation");
            let keep = (cap - previous).min(n as u64) as usize;
            file.write_all(&buffer[..keep])?;
            if keep < n {
                state.stop(1);
                break;
            }
        }
        file.sync_all()
    })();
    if result.is_err() {
        state.stop(2);
    }
    result
}
#[cfg(unix)]
fn cancellable(pipe: &impl std::os::fd::AsFd) -> Result<()> {
    let flags =
        rustix::fs::fcntl_getfl(pipe).map_err(|e| Error::io("Read pipe flags", e.into()))?;
    rustix::fs::fcntl_setfl(pipe, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|e| Error::io("Set cancellable pipe", e.into()))
}
fn missing_group(error: &io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) {
        return true;
    }
    error.kind() == io::ErrorKind::NotFound
}
fn incomplete(analysis: &mut Analysis) {
    analysis.counts = None;
    analysis.parse_status = "unknown".into();
}

fn stamp() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| Error::new("CLOCK_ERROR", 5, "Clock is before Unix epoch."))
}
fn bounded_text(bytes: &[u8], limit: usize) -> String {
    let n = bytes.len().min(limit);
    String::from_utf8_lossy(&bytes[..n])
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

struct Analyzer {
    counts: Counts,
    suites: u64,
    starts: u64,
    expected_tests: u64,
    reported_tests: u64,
    invalid: bool,
    locations: u64,
    failures: Vec<Failure>,
    offset: u64,
    limit: usize,
    budget: usize,
    context_truncated: bool,
    tail: Vec<Failure>,
    summary: Regex,
    running: Regex,
}
impl Analyzer {
    fn new(offset: u64, limit: usize, budget: usize) -> Self {
        Self { counts: Counts::default(), suites: 0, starts: 0, expected_tests: 0, reported_tests: 0, invalid: false,
            locations: 0, failures: Vec::new(), offset, limit, budget, context_truncated: false, tail: Vec::new(),
            summary: Regex::new(r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;").expect("static summary regex"),
            running: Regex::new(r"^running (\d+) tests?$").expect("static running regex") }
    }
    fn scan(&mut self, reader: impl Read, stream: &str) -> Result<()> {
        let mut reader = BufReader::new(reader);
        let mut previous: VecDeque<Vec<u8>> = VecDeque::new();
        let mut line = 0_u64;
        let mut byte_offset = 0_u64;
        loop {
            // Bound each allocation even for binary output with no newline.
            let mut bytes = Vec::new();
            let mut tail_bytes = Vec::new();
            let mut consumed = 0_u64;
            let mut overflow = false;
            loop {
                let available = reader
                    .fill_buf()
                    .map_err(|e| Error::io("Read saved log", e))?;
                if available.is_empty() {
                    break;
                }
                let take = available
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(available.len(), |i| i + 1);
                if take >= 512 {
                    tail_bytes.clear();
                    tail_bytes.extend_from_slice(&available[take - 512..take]);
                } else {
                    tail_bytes.extend_from_slice(&available[..take]);
                    if tail_bytes.len() > 512 {
                        tail_bytes.drain(..tail_bytes.len() - 512);
                    }
                }
                let room = MAX_LINE.saturating_sub(bytes.len());
                bytes.extend_from_slice(&available[..take.min(room)]);
                if take > room {
                    overflow = true;
                }
                let newline = available[take - 1] == b'\n';
                consumed += take as u64;
                reader.consume(take);
                if newline {
                    break;
                }
            }
            if consumed == 0 {
                break;
            }
            line += 1;
            let text = String::from_utf8_lossy(&bytes);
            let text = text.trim_end_matches(['\r', '\n']);
            if overflow {
                self.invalid = true;
            }
            if let Some(c) = self.running.captures(text) {
                self.starts += 1;
                if let Ok(n) = c[1].parse::<u64>() {
                    if let Some(total) = self.expected_tests.checked_add(n) {
                        self.expected_tests = total;
                    } else {
                        self.invalid = true;
                    }
                } else {
                    self.invalid = true;
                }
            }
            if let Some(c) = self.summary.captures(text) {
                let numbers = (2..=6)
                    .map(|i| c[i].parse::<u64>())
                    .collect::<std::result::Result<Vec<_>, _>>();
                match numbers {
                    Ok(n) => {
                        self.suites += 1;
                        if let (Some(passed), Some(failed), Some(skipped), Some(reported)) = (
                            self.counts.passed.checked_add(n[0]),
                            self.counts.failed.checked_add(n[1]),
                            self.counts.skipped.checked_add(n[2]),
                            self.reported_tests
                                .checked_add(n[0])
                                .and_then(|v| v.checked_add(n[1]))
                                .and_then(|v| v.checked_add(n[2])),
                        ) {
                            self.counts = Counts {
                                passed,
                                failed,
                                skipped,
                            };
                            self.reported_tests = reported;
                        } else {
                            self.invalid = true;
                        }
                        if n[3] != 0 || (c[1].eq("ok") && n[1] != 0) {
                            self.invalid = true;
                        }
                    }
                    Err(_) => self.invalid = true,
                }
            }
            let failure = text.contains("panicked at")
                || text.starts_with("error:")
                || text.starts_with("error[")
                || (text.starts_with("test ") && text.ends_with(" ... FAILED"))
                || (text.starts_with("---- ") && text.ends_with(" stdout ----"));
            // Include two subsequent lines in already selected excerpts.
            for entry in self.failures.iter_mut().rev().take(3) {
                if entry.stream == stream && line > entry.line && line <= entry.line + 2 {
                    let shown = bounded_text(&bytes, self.budget.min(512));
                    self.budget = self.budget.saturating_sub(shown.len());
                    if shown.len() < text.len() {
                        self.context_truncated = true;
                    }
                    if !shown.is_empty() {
                        entry.context.push(shown);
                    }
                }
            }
            if failure {
                let index = self.locations;
                self.locations += 1;
                if index >= self.offset && self.failures.len() < self.limit && self.budget > 0 {
                    let shown = bounded_text(&bytes, self.budget.min(512));
                    self.budget = self.budget.saturating_sub(shown.len());
                    if shown.len() < text.len() {
                        self.context_truncated = true;
                    }
                    let mut context = Vec::new();
                    for old in &previous {
                        let preceding = bounded_text(old, self.budget.min(256));
                        self.budget = self.budget.saturating_sub(preceding.len());
                        if !preceding.is_empty() {
                            context.push(preceding);
                        }
                    }
                    if !shown.is_empty() {
                        context.push(shown);
                    }
                    self.failures.push(Failure {
                        stream: stream.into(),
                        line,
                        byte_offset,
                        context,
                    });
                }
            }
            if !text.is_empty() {
                self.tail.push(Failure {
                    stream: stream.into(),
                    line,
                    byte_offset: byte_offset + consumed - tail_bytes.len() as u64,
                    context: vec![bounded_text(&tail_bytes, 512)],
                });
                if self.tail.iter().filter(|f| f.stream == stream).count() > 2 {
                    if let Some(index) = self.tail.iter().position(|f| f.stream == stream) {
                        self.tail.remove(index);
                    }
                }
            }
            previous.push_back(bytes[..bytes.len().min(256)].to_vec());
            if previous.len() > 2 {
                previous.pop_front();
            }
            byte_offset += consumed;
        }
        Ok(())
    }
    fn finish(self, parser: &str, command_exit: Option<i32>) -> Analysis {
        let parsed = parser == "cargo"
            && self.suites > 0
            && self.suites == self.starts
            && self.expected_tests == self.reported_tests
            && !self.invalid
            && command_exit.is_some_and(|code| code == 0 || self.counts.failed > 0);
        let omitted = self
            .locations
            .saturating_sub(self.offset.min(self.locations))
            .saturating_sub(self.failures.len() as u64);
        Analysis {
            parser: parser.into(),
            parse_status: if parsed { "parsed" } else { "unknown" }.into(),
            observed_counts: (parser == "cargo" && self.suites > 0).then_some(self.counts.clone()),
            counts: parsed.then_some(self.counts),
            failure_locations_total: self.locations,
            failures: self.failures,
            failure_locations_omitted: omitted,
            context_truncated: self.context_truncated,
            unclassified_failure_tail: if command_exit != Some(0)
                && self.locations == 0
                && self.offset == 0
            {
                let mut remaining = self.budget;
                self.tail
                    .into_iter()
                    .take(self.limit)
                    .filter_map(|mut failure| {
                        let text = failure.context.first()?;
                        let shown = bounded_text(text.as_bytes(), remaining.min(512));
                        remaining = remaining.saturating_sub(shown.len());
                        if shown.is_empty() {
                            None
                        } else {
                            failure.context = vec![shown];
                            Some(failure)
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            },
        }
    }
}

fn analyze(
    stdout: impl Read,
    stderr: impl Read,
    parser: &str,
    command_exit: Option<i32>,
    offset: u64,
    limit: usize,
    budget: usize,
) -> Result<Analysis> {
    let mut analyzer = Analyzer::new(offset, limit, budget);
    analyzer.scan(stdout, "stdout")?;
    analyzer.scan(stderr, "stderr")?;
    Ok(analyzer.finish(parser, command_exit))
}

fn preserve(file: NamedTempFile) -> Value {
    match file.keep() {
        Ok((_, path)) => json!({"path": path}),
        Err(e) => json!({"path": e.file.path(), "preservation_error": e.error.to_string()}),
    }
}

pub fn execute(context: &Context, options: RunOptions) -> Result<(Value, u8)> {
    if options.snapshot.is_some() && !options.inputs.is_empty() {
        return Err(Error::invalid("--snapshot and --input cannot be combined."));
    }
    if options.command.is_empty() {
        return Err(Error::invalid("run requires a command after --."));
    }
    let store = context.store()?;
    let parser = match options.parser.as_str() {
        "auto" => {
            if Path::new(&options.command[0])
                .file_stem()
                .is_some_and(|n| n == "cargo")
            {
                "cargo"
            } else {
                "none"
            }
        }
        "cargo" => "cargo",
        "none" => "none",
        _ => return Err(Error::invalid("Parser must be auto, cargo or none.")),
    };
    let source_before = if let Some(id) = &options.snapshot {
        let manifest = snapshot::load(&store, id)?;
        let changes = snapshot::check(&manifest)?;
        if !changes.is_empty() {
            return Err(Error::new("SNAPSHOT_CHANGED", 10, "Selected source no longer matches the snapshot; command was not started.").details(json!({"reference": id, "changed_count": changes.len(), "changes": changes.iter().take(20).collect::<Vec<_>>()})));
        }
        Some(manifest)
    } else if !options.inputs.is_empty() {
        Some(snapshot::capture(context, &options.inputs)?)
    } else {
        None
    };
    store.check_capacity(context.max_entries)?;
    let required_space = options
        .limits
        .min_free_bytes
        .saturating_add(options.limits.max_log_bytes.saturating_mul(3))
        .saturating_add(MAX_DESCRIPTOR * 2 + 1024 * 1024);
    let available = store.available_space()?;
    if available < required_space {
        return Err(Error::new("DISK_SPACE", 9, "Insufficient free space for bounded capture and record packing; command was not started.")
            .details(json!({"available_bytes": available, "required_bytes": required_space})));
    }
    let stdout = store.staging_file()?;
    let stderr = store.staging_file()?;
    let started_at = stamp()?;
    let timer = Instant::now();
    let mut command = Command::new(&options.command[0]);
    command
        .args(&options.command[1..])
        .current_dir(&context.cwd)
        .stdin(if options.inherit_stdin {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Clone files before spawning so all fallible setup precedes command execution.
    let out_file = stdout
        .as_file()
        .try_clone()
        .map_err(|e| Error::io("Clone stdout file", e))?;
    let err_file = stderr
        .as_file()
        .try_clone()
        .map_err(|e| Error::io("Clone stderr file", e))?;
    let mut child = command
        .group_spawn()
        .map_err(|e| Error::io("Start recorded command group", e))?;
    let out = child.inner().stdout.take().expect("piped stdout");
    let err = child.inner().stderr.take().expect("piped stderr");
    #[cfg(unix)]
    if let Err(error) = cancellable(&out).and_then(|_| cancellable(&err)) {
        let _ = child.kill();
        return Err(error);
    }
    let state = Arc::new(CaptureState {
        bytes: AtomicU64::new(0),
        reason: AtomicU8::new(0),
        cancel: AtomicBool::new(false),
    });
    let cap = options.limits.max_log_bytes;
    let out_state = Arc::clone(&state);
    let out_thread = std::thread::spawn(move || capture(out, out_file, out_state, cap));
    let err_state = Arc::clone(&state);
    let err_thread = std::thread::spawn(move || capture(err, err_file, err_state, cap));
    let mut status = None;
    let mut termination_reason = None;
    let mut last_space_check = Instant::now();
    let mut cleanup_error = None;
    loop {
        match child.try_wait() {
            Ok(value) => {
                if value.is_some() {
                    status = value;
                }
            }
            Err(error) => {
                cleanup_error = Some(error.to_string());
                termination_reason = Some("wait_failed".to_owned());
            }
        }
        let reason = state.reason.load(Ordering::SeqCst);
        if reason != 0 {
            termination_reason = Some(
                if reason == 1 {
                    "log_limit"
                } else {
                    "capture_failed"
                }
                .into(),
            );
        }
        // A complete EOF + exit wins over a deadline observed only after completion.
        if termination_reason.is_none()
            && status.is_some()
            && out_thread.is_finished()
            && err_thread.is_finished()
        {
            // A capture thread may have set the guard after the earlier load but before
            // is_finished. Re-read only after both completions, before declaring EOF complete.
            match state.reason.load(Ordering::SeqCst) {
                0 => break,
                1 => termination_reason = Some("log_limit".into()),
                _ => termination_reason = Some("capture_failed".into()),
            }
        }
        if termination_reason.is_none()
            && timer.elapsed() >= Duration::from_secs(options.limits.timeout_seconds)
        {
            termination_reason = Some("timeout".into());
        }
        if termination_reason.is_none() && last_space_check.elapsed() >= Duration::from_millis(250)
        {
            let required = options
                .limits
                .min_free_bytes
                .saturating_add(state.bytes.load(Ordering::SeqCst).saturating_mul(2))
                .saturating_add(MAX_DESCRIPTOR * 2 + 1024 * 1024);
            match store.available_space() {
                Ok(free) if free >= required => {}
                Ok(_) => termination_reason = Some("disk_space".into()),
                Err(_) => termination_reason = Some("disk_check_failed".into()),
            }
            last_space_check = Instant::now();
        }
        if termination_reason.is_some() {
            // Kill this process group/job only. Never wait indefinitely for inherited pipes.
            if let Err(error) = child.kill() {
                if !missing_group(&error) {
                    cleanup_error = Some(error.to_string());
                }
            }
            state.cancel.store(true, Ordering::SeqCst);
            let cleanup = Instant::now();
            while cleanup.elapsed() < Duration::from_secs(2) {
                if let Ok(Some(value)) = child.try_wait() {
                    status = Some(value);
                }
                if status.is_some() && out_thread.is_finished() && err_thread.is_finished() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let duration_ms = timer.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let finished_at = stamp();
    if status.is_none()
        || !out_thread.is_finished()
        || !err_thread.is_finished()
        || cleanup_error.is_some()
    {
        return Err(Error::new(
            "RECORDING_FAILED",
            5,
            "Command cleanup or pipe capture did not finish within the bounded grace period.",
        )
        .details(
            json!({"termination_reason": termination_reason, "cleanup_error": cleanup_error,
                "stdout": preserve(stdout), "stderr": preserve(stderr), "logs_complete": false}),
        ));
    }
    let out_result = out_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stdout capture panicked")));
    let err_result = err_thread
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("stderr capture panicked")));
    let status = status.expect("checked completed child");
    let command_exit = status.code();
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    let logs_complete = termination_reason.is_none();
    let wrapper_exit = match termination_reason.as_deref() {
        Some("timeout") => 124,
        Some(_) => 125,
        None => command_exit
            .unwrap_or_else(|| signal.map_or(1, |s| 128 + s))
            .clamp(0, 255) as u8,
    };
    if let Err(error) = out_result.and(err_result) {
        return Err(Error::new("RECORDING_FAILED", 5, format!("Command ended but log capture failed: {error}"))
            .details(json!({"command_exit": command_exit, "signal": signal, "stdout": preserve(stdout), "stderr": preserve(stderr), "logs_complete": false})));
    }
    let mut stdout = stdout;
    let mut stderr = stderr;
    // Every failure after execution preserves the full captured streams as local recovery material.
    let recorded = (|| -> Result<(Metadata, RunRecord)> {
        stdout.rewind().map_err(|e| Error::io("Rewind stdout", e))?;
        stderr.rewind().map_err(|e| Error::io("Rewind stderr", e))?;
        let mut analysis = analyze(
            stdout.as_file_mut(),
            stderr.as_file_mut(),
            parser,
            command_exit,
            0,
            5,
            4096,
        )?;
        if !logs_complete {
            incomplete(&mut analysis);
        }
        let source_after = if let Some(manifest) = &source_before {
            match snapshot::check(manifest) {
                Ok(changes) => {
                    json!({"status": if changes.is_empty() { "unchanged" } else { "changed" }, "changed_count": changes.len(), "changes": changes})
                }
                Err(error) => json!({"status": "unknown", "error": error}),
            }
        } else {
            json!({"status": "not_recorded"})
        };
        let mut artifacts = Vec::new();
        for path in &options.artifacts {
            let absolute = context.cwd.join(path);
            let text = snapshot::native_text(&absolute)?;
            let parent = absolute
                .parent()
                .ok_or_else(|| Error::invalid("Artifact needs a file path."))?;
            let name = snapshot::native_text(Path::new(
                absolute
                    .file_name()
                    .ok_or_else(|| Error::invalid("Artifact needs a file name."))?,
            ))?;
            match snapshot::hash_file(parent, &name) {
                Ok(Some(observed)) => artifacts.push(Artifact {
                    path: text,
                    status: "observed".into(),
                    observed: Some(observed),
                }),
                Ok(None) => artifacts.push(Artifact {
                    path: text,
                    status: "missing".into(),
                    observed: None,
                }),
                Err(_) => artifacts.push(Artifact {
                    path: text,
                    status: "unknown".into(),
                    observed: None,
                }),
            }
        }
        let record = RunRecord {
            schema: 1,
            command: options.command.clone(),
            cwd: context.cwd.clone(),
            started_at,
            finished_at: finished_at?,
            duration_ms,
            command_exit,
            signal,
            wrapper_exit,
            stdout_bytes: stdout
                .as_file()
                .metadata()
                .map_err(|e| Error::io("Read stdout size", e))?
                .len(),
            stderr_bytes: stderr
                .as_file()
                .metadata()
                .map_err(|e| Error::io("Read stderr size", e))?
                .len(),
            logs_complete,
            limits: Some(options.limits.clone()),
            termination_reason: termination_reason.clone(),
            analysis,
            snapshot: options.snapshot.clone(),
            source_before,
            source_after,
            artifacts,
            artifact_relationship:
                "declared_output_observed_after_command_not_proven_build_provenance".into(),
        };
        let descriptor = serde_json::to_vec(&record).map_err(|e| Error::invalid(e.to_string()))?;
        if descriptor.len() as u64 > MAX_DESCRIPTOR {
            return Err(Error::new(
                "SIZE_LIMIT",
                9,
                "Run descriptor exceeds 4 MiB; select fewer input files.",
            ));
        }
        let mut staged = store.staging_file()?;
        staged
            .write_all(&(descriptor.len() as u64).to_le_bytes())
            .and_then(|_| staged.write_all(&descriptor))
            .map_err(|e| Error::io("Write run descriptor", e))?;
        stdout
            .rewind()
            .and_then(|_| stderr.rewind())
            .map_err(|e| Error::io("Rewind captured logs", e))?;
        io::copy(stdout.as_file_mut(), &mut staged)
            .and_then(|_| io::copy(stderr.as_file_mut(), &mut staged))
            .map_err(|e| Error::io("Pack captured logs", e))?;
        let dependencies = options.snapshot.clone().into_iter().collect();
        let metadata = match store.put_record(
            staged.as_file_mut(),
            RecordKind::Run,
            dependencies,
            context.max_entries,
        ) {
            Ok(metadata) => metadata,
            Err(error) => {
                return Err(Error::new(error.code, error.exit, error.message).details(
                    json!({"cause_details": error.details, "staged_record": preserve(staged)}),
                ))
            }
        };
        Ok((metadata, record))
    })();
    match recorded {
        Ok((metadata, record)) => Ok((summary(&metadata.id, &record), wrapper_exit)),
        Err(error) => Err(Error::new(
            "RECORDING_FAILED",
            5,
            "Command ended but its run record could not be saved.",
        )
        .details(
            json!({"command_exit": command_exit, "signal": signal, "cause": error,
                "stdout": preserve(stdout), "stderr": preserve(stderr), "logs_complete": logs_complete}),
        )),
    }
}

pub fn summary(id: &str, record: &RunRecord) -> Value {
    let changes = record.source_after.get("changes").and_then(Value::as_array);
    let mut source_after = record.source_after.clone();
    if let Some(changes) = changes {
        source_after["changes"] = json!(changes.iter().take(20).collect::<Vec<_>>());
        source_after["omitted"] = json!(changes.len().saturating_sub(20));
    }
    json!({"reference": id, "kind": "run", "recording_status": "saved", "command_exit": record.command_exit,
        "signal": record.signal, "wrapper_exit": record.wrapper_exit, "logs_complete": record.logs_complete,
        "limits": record.limits, "termination_reason": record.termination_reason,
        "stdout_bytes": record.stdout_bytes, "stderr_bytes": record.stderr_bytes, "duration_ms": record.duration_ms, "started_at": record.started_at, "finished_at": record.finished_at,
        "analysis": record.analysis, "snapshot": record.snapshot, "source_after": source_after,
        "artifact_count": record.artifacts.len()})
}

pub struct LoadedRun {
    pub file: File,
    pub record: RunRecord,
    pub stdout_start: u64,
    pub stderr_start: u64,
}
pub fn load(store: &Store, id: &str) -> Result<LoadedRun> {
    let (mut file, metadata) = store.verified_file(id)?;
    if metadata.kind != RecordKind::Run {
        return Err(Error::new("WRONG_KIND", 4, "Expected a run reference."));
    }
    let payload_start = file
        .stream_position()
        .map_err(|e| Error::io("Read run offset", e))?;
    let mut prefix = [0_u8; 8];
    file.read_exact(&mut prefix)
        .map_err(|_| Error::new("CORRUPT_REF", 7, "Missing run descriptor prefix."))?;
    let len = u64::from_le_bytes(prefix);
    if len > MAX_DESCRIPTOR || len > metadata.bytes.saturating_sub(8) as u64 {
        return Err(Error::new("CORRUPT_REF", 7, "Run descriptor is too large."));
    }
    let mut bytes = vec![0; len as usize];
    file.read_exact(&mut bytes)
        .map_err(|e| Error::io("Read run descriptor", e))?;
    let record: RunRecord = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("CORRUPT_REF", 7, "Invalid run descriptor."))?;
    if record.schema != 1
        || record
            .stdout_bytes
            .checked_add(record.stderr_bytes)
            .and_then(|n| n.checked_add(8 + len))
            != Some(metadata.bytes as u64)
        || record.snapshot.clone().into_iter().collect::<Vec<_>>() != metadata.dependencies
    {
        return Err(Error::new("CORRUPT_REF", 7, "Inconsistent run record."));
    }
    Ok(LoadedRun {
        file,
        stdout_start: payload_start + 8 + len,
        stderr_start: payload_start + 8 + len + record.stdout_bytes,
        record,
    })
}

pub fn failed(run: &mut LoadedRun, offset: u64, limit: usize, budget: usize) -> Result<Analysis> {
    let mut analyzer = Analyzer::new(offset, limit, budget);
    run.file
        .seek(SeekFrom::Start(run.stdout_start))
        .map_err(|e| Error::io("Seek stdout", e))?;
    analyzer.scan((&mut run.file).take(run.record.stdout_bytes), "stdout")?;
    run.file
        .seek(SeekFrom::Start(run.stderr_start))
        .map_err(|e| Error::io("Seek stderr", e))?;
    analyzer.scan((&mut run.file).take(run.record.stderr_bytes), "stderr")?;
    let mut analysis = analyzer.finish(&run.record.analysis.parser, run.record.command_exit);
    if !run.record.logs_complete {
        incomplete(&mut analysis);
    }
    Ok(analysis)
}

pub fn stream_range(run: &LoadedRun, stream: &str) -> Result<(u64, u64)> {
    match stream {
        "stdout" => Ok((run.stdout_start, run.record.stdout_bytes)),
        "stderr" => Ok((run.stderr_start, run.record.stderr_bytes)),
        _ => Err(Error::invalid("Log stream must be stdout or stderr.")),
    }
}

/// Recheck observations without changing or reinterpreting the historical test result.
pub fn check(id: &str, record: &RunRecord, offset: u64, limit: usize) -> (Value, u8) {
    let mut changed = Vec::new();
    let mut unknown = Vec::new();
    let mut checked = 0;
    if let Some(manifest) = &record.source_before {
        checked += manifest.files.len();
        match snapshot::check(manifest) {
            Ok(changes) => changed.extend(
                changes
                    .iter()
                    .map(|c| json!({"kind": "source", "path": c.path, "status": c.status})),
            ),
            Err(error) => unknown.push(json!({"kind": "source", "error": error})),
        }
    }
    for artifact in &record.artifacts {
        if artifact.status != "observed" {
            unknown
                .push(json!({"kind": "artifact", "path": artifact.path, "status": "not_recorded"}));
            continue;
        }
        let path = Path::new(&artifact.path);
        let observed = path
            .parent()
            .zip(path.file_name())
            .ok_or_else(|| Error::invalid("Invalid recorded artifact path."))
            .and_then(|(parent, name)| {
                snapshot::native_text(Path::new(name))
                    .and_then(|name| snapshot::hash_file(parent, &name))
            });
        checked += 1;
        match observed {
            Ok(current) if current == artifact.observed => {},
            Ok(current) => changed.push(json!({"kind": "artifact", "path": artifact.path, "status": if current.is_some() { "modified" } else { "removed" }})),
            Err(error) => unknown.push(json!({"kind": "artifact", "path": artifact.path, "error": error})),
        }
    }
    let status = if !unknown.is_empty() {
        "unknown"
    } else if !changed.is_empty() {
        "changed"
    } else if checked == 0 {
        "not_recorded"
    } else {
        "unchanged"
    };
    let total = changed.len() + unknown.len();
    let skip = usize::try_from(offset).unwrap_or(usize::MAX);
    let details: Vec<_> = changed
        .iter()
        .chain(unknown.iter())
        .skip(skip)
        .take(limit)
        .collect();
    (
        json!({"reference": id, "status": status, "checked_files": checked, "changed_count": changed.len(), "unknown_count": unknown.len(),
        "details": details, "offset": offset, "omitted": total.saturating_sub(skip).saturating_sub(details.len()),
        "historical_command_exit": record.command_exit, "build_provenance_verified": false}),
        if status == "unchanged" { 0 } else { 10 },
    )
}

#[cfg(test)]
mod tests {
    use super::analyze;
    use std::io::Cursor;

    fn parse(stdout: &str, stderr: &str, exit: Option<i32>) -> super::Analysis {
        analyze(
            Cursor::new(stdout.as_bytes()),
            Cursor::new(stderr.as_bytes()),
            "cargo",
            exit,
            0,
            5,
            4096,
        )
        .unwrap()
    }

    #[test]
    fn multiple_suites_sum_pass_fail_and_ignored_without_counting_filtered_tests() {
        let log = "running 2 tests\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 4 filtered out; finished in 0s\nrunning 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n";
        let parsed = parse(log, "", Some(101));
        assert_eq!(parsed.parse_status, "parsed");
        let counts = parsed.counts.unwrap();
        assert_eq!((counts.passed, counts.failed, counts.skipped), (1, 1, 1));
    }

    #[test]
    fn unfinished_suite_and_late_command_error_keep_only_observed_counts() {
        let complete = "running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n";
        let late_error = parse(
            complete,
            "error: build failed for another target\n",
            Some(101),
        );
        assert!(late_error.counts.is_none());
        assert_eq!(late_error.observed_counts.unwrap().passed, 1);
        let unfinished = parse(&format!("{complete}running 3 tests\n"), "", Some(101));
        assert!(unfinished.counts.is_none());
        assert_eq!(unfinished.parse_status, "unknown");
    }

    #[test]
    fn oversized_lines_and_overflowed_counters_are_unknown() {
        let summary = "running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n";
        let oversized = parse(&format!("{}\n{summary}", "x".repeat(65537)), "", Some(0));
        assert!(oversized.counts.is_none());
        let huge = "running 18446744073709551615 tests\ntest result: ok. 18446744073709551615 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0s\n";
        assert!(parse(&format!("{huge}{huge}"), "", Some(0))
            .counts
            .is_none());
    }
}
