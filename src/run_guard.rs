//! Bounded process-group capture. Redaction is explicit and happens before bytes reach disk.
use crate::error::{Error, Result};
use clap::Args;
use command_group::{CommandGroup, GroupChild};
use serde_json::json;
use std::{
    fs::File,
    io::{self, Read, Write},
    process::{Command, ExitStatus},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Args)]
pub struct Limits {
    /// Maximum seconds for the process and output pipes; 0 explicitly disables the deadline.
    #[arg(long, default_value_t = 900)]
    pub timeout: u64,
    /// Combined stdout/stderr byte budget, before and after redaction; 0 means unlimited.
    #[arg(long, default_value_t = 67_108_864)]
    pub max_log_bytes: u64,
    /// Redact this environment variable's exact value from logs and recorded argv, before saving.
    #[arg(long)]
    pub redact_env: Vec<String>,
}

pub struct Prepared {
    timeout: u64,
    max_bytes: u64,
    pub redacted_env: Vec<String>,
    patterns: Arc<Vec<Vec<u8>>>,
}

impl Prepared {
    pub fn new(limits: &Limits) -> Result<Self> {
        let mut names = limits.redact_env.clone();
        names.sort();
        names.dedup();
        if names.len() > 32 {
            return Err(Error::invalid(
                "At most 32 --redact-env names may be supplied.",
            ));
        }
        let mut patterns = Vec::new();
        for name in &names {
            let valid = !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !name.as_bytes()[0].is_ascii_digit();
            if !valid {
                return Err(Error::new(
                    "REDACTION_CONFIG",
                    4,
                    "Redaction requires an environment variable name, not a secret value.",
                ));
            }
            let value = std::env::var(name).map_err(|_| {
                Error::new(
                    "REDACTION_CONFIG",
                    4,
                    "A selected redaction variable is missing or is not UTF-8.",
                )
            })?;
            if value.is_empty() || value.len() > 4096 {
                return Err(Error::new(
                    "REDACTION_CONFIG",
                    4,
                    "Each redaction value must contain 1 to 4096 UTF-8 bytes.",
                ));
            }
            patterns.push(value.into_bytes());
        }
        patterns.sort_by_key(|p| std::cmp::Reverse(p.len()));
        patterns.dedup();
        Ok(Self {
            timeout: limits.timeout,
            max_bytes: limits.max_log_bytes,
            redacted_env: names,
            patterns: Arc::new(patterns),
        })
    }

    pub fn command(&self, argv: &[String]) -> Vec<String> {
        argv.iter()
            .map(|arg| {
                let mut redactor = Redactor::new(self.patterns.clone());
                String::from_utf8_lossy(&redactor.feed(arg.as_bytes(), true)).into_owned()
            })
            .collect()
    }

    pub fn capture(
        &self,
        command: &mut Command,
        stdout: NamedTempFile,
        stderr: NamedTempFile,
    ) -> Result<Captured> {
        // Reopen before spawn so a file-open error cannot leave a started child behind.
        let out_file = stdout
            .reopen()
            .map_err(|e| Error::io("Open capture output", e))?;
        let err_file = stderr
            .reopen()
            .map_err(|e| Error::io("Open capture error output", e))?;
        let mut child = KillOnDrop(
            command
                .group_spawn()
                .map_err(|e| Error::io("Start recorded process group", e))?,
        );
        let out = child
            .0
            .inner()
            .stdout
            .take()
            .ok_or_else(|| Error::new("INTERNAL_ERROR", 5, "stdout was not piped."))?;
        let err = child
            .0
            .inner()
            .stderr
            .take()
            .ok_or_else(|| Error::new("INTERNAL_ERROR", 5, "stderr was not piped."))?;
        let state = Arc::new(State::default());
        let out_thread = capture_thread(
            out,
            out_file,
            self.patterns.clone(),
            state.clone(),
            self.max_bytes,
        );
        let err_thread = capture_thread(
            err,
            err_file,
            self.patterns.clone(),
            state.clone(),
            self.max_bytes,
        );
        let started = Instant::now();
        let mut stopped_at = None;
        let mut reason = None;
        let status = loop {
            let status = match child.0.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    state.abort.store(true, Ordering::SeqCst);
                    return Err(recovery(
                        &format!("Could not inspect the recorded process: {error}"),
                        stdout,
                        stderr,
                        None,
                    ));
                }
            };
            // Check limits before accepting completion: a fast child can exceed the budget and exit.
            if reason.is_none() {
                reason = if state.failed.load(Ordering::SeqCst) {
                    Some("capture_error")
                } else if state.limited.load(Ordering::SeqCst) {
                    Some("log_limit")
                } else if self.timeout != 0
                    && started.elapsed() >= Duration::from_secs(self.timeout)
                {
                    Some("timeout")
                } else {
                    None
                };
                if reason.is_some() {
                    state.abort.store(true, Ordering::SeqCst);
                    let _ = child.0.kill();
                    stopped_at = Some(Instant::now());
                }
            }
            if out_thread.is_finished() && err_thread.is_finished() {
                if let Some(status) = status {
                    break status;
                }
            }
            // Escaped groups, stuck devices and uninterruptible children are not a sandbox guarantee.
            // Never join a blocked reader indefinitely. The named files stay available for recovery.
            if stopped_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)) {
                return Err(recovery(
                    "Process or output pipes did not close after termination.",
                    stdout,
                    stderr,
                    status,
                ));
            }
            thread::sleep(Duration::from_millis(20));
        };
        let out_result = out_thread
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("stdout capture panicked")));
        let err_result = err_thread
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("stderr capture panicked")));
        let (out_eof, err_eof) = match (out_result, err_result) {
            (Ok(out_eof), Ok(err_eof)) => (out_eof, err_eof),
            _ => {
                return Err(recovery(
                    "Command ended but log capture failed.",
                    stdout,
                    stderr,
                    Some(status),
                ))
            }
        };
        // A final EOF flush may discover a post-redaction size limit just before a thread ends.
        if reason.is_none() && state.limited.load(Ordering::SeqCst) {
            reason = Some("log_limit");
        }
        Ok(Captured {
            stdout,
            stderr,
            status,
            stop_reason: reason.map(str::to_owned),
            logs_complete: reason.is_none() && out_eof && err_eof,
        })
    }
}

pub struct Captured {
    pub stdout: NamedTempFile,
    pub stderr: NamedTempFile,
    pub status: ExitStatus,
    pub stop_reason: Option<String>,
    pub logs_complete: bool,
}

struct KillOnDrop(GroupChild);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn recovery(
    message: &str,
    stdout: NamedTempFile,
    stderr: NamedTempFile,
    status: Option<ExitStatus>,
) -> Error {
    fn keep(file: NamedTempFile) -> serde_json::Value {
        match file.keep() {
            Ok((_, path)) => json!({"path": path}),
            Err(e) => json!({"path": e.file.path(), "preservation_error": e.error.to_string()}),
        }
    }
    Error::new("RECORDING_FAILED", 5, message).details(json!({
        "stdout": keep(stdout), "stderr": keep(stderr), "logs_complete": false,
        "command_exit": status.and_then(|s| s.code()), "termination_unconfirmed": true
    }))
}

#[derive(Default)]
struct State {
    raw: AtomicU64,
    stored: AtomicU64,
    limited: AtomicBool,
    abort: AtomicBool,
    failed: AtomicBool,
}
impl State {
    fn reserve(&self, counter: &AtomicU64, limit: u64, requested: usize) -> usize {
        if limit == 0 {
            return requested;
        }
        let mut used = counter.load(Ordering::SeqCst);
        loop {
            let allowed = (limit.saturating_sub(used)).min(requested as u64);
            match counter.compare_exchange(used, used + allowed, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    if allowed < requested as u64 {
                        self.limited.store(true, Ordering::SeqCst);
                    }
                    return allowed as usize;
                }
                Err(current) => used = current,
            }
        }
    }
}

fn capture_thread(
    reader: impl Read + Send + 'static,
    mut file: File,
    patterns: Arc<Vec<Vec<u8>>>,
    state: Arc<State>,
    limit: u64,
) -> thread::JoinHandle<io::Result<bool>> {
    thread::spawn(move || {
        let result = capture(reader, &mut file, patterns, &state, limit)
            .and_then(|eof| file.sync_all().map(|_| eof));
        if result.is_err() {
            state.failed.store(true, Ordering::SeqCst);
        }
        result
    })
}

fn capture(
    mut reader: impl Read,
    file: &mut File,
    patterns: Arc<Vec<Vec<u8>>>,
    state: &State,
    limit: u64,
) -> io::Result<bool> {
    let mut redactor = Redactor::new(patterns);
    let mut buffer = [0_u8; 8192];
    loop {
        if state.abort.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let count = match reader.read(&mut buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if state.abort.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let allowed = state.reserve(&state.raw, limit, count);
        let bytes = redactor.feed(&buffer[..allowed], count == 0);
        let stored = state.reserve(&state.stored, limit, bytes.len());
        file.write_all(&bytes[..stored])?;
        if allowed < count || stored < bytes.len() || state.limited.load(Ordering::SeqCst) {
            // Pending raw suffixes are deliberately not flushed on truncation.
            return Ok(false);
        }
        if count == 0 {
            return Ok(true);
        }
    }
}

struct Redactor {
    patterns: Arc<Vec<Vec<u8>>>,
    pending: Vec<u8>,
    longest: usize,
}
impl Redactor {
    fn new(patterns: Arc<Vec<Vec<u8>>>) -> Self {
        let longest = patterns.iter().map(Vec::len).max().unwrap_or(1);
        Self {
            patterns,
            pending: Vec::new(),
            longest,
        }
    }
    fn feed(&mut self, bytes: &[u8], eof: bool) -> Vec<u8> {
        if self.patterns.is_empty() {
            return bytes.to_vec();
        }
        self.pending.extend_from_slice(bytes);
        let safe = if eof {
            self.pending.len()
        } else {
            self.pending.len().saturating_sub(self.longest - 1)
        };
        let mut cursor = 0;
        let mut output = Vec::new();
        while cursor < safe {
            if let Some(pattern) = self
                .patterns
                .iter()
                .find(|pattern| self.pending[cursor..].starts_with(pattern))
            {
                output.extend_from_slice(b"[REDACTED]");
                cursor += pattern.len();
            } else {
                output.push(self.pending[cursor]);
                cursor += 1;
            }
        }
        self.pending.drain(..cursor);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redaction_handles_every_split_and_prefers_longer_values() {
        let input = b"before abcdef and abc after";
        for split in 0..=input.len() {
            let mut redactor = Redactor::new(Arc::new(vec![b"abcdef".to_vec(), b"abc".to_vec()]));
            let mut output = redactor.feed(&input[..split], false);
            output.extend(redactor.feed(&input[split..], true));
            assert_eq!(output, b"before [REDACTED] and [REDACTED] after");
        }
    }
    #[test]
    fn shared_budget_never_exceeds_the_limit() {
        let state = State::default();
        assert_eq!(state.reserve(&state.stored, 10, 7), 7);
        assert_eq!(state.reserve(&state.stored, 10, 7), 3);
        assert_eq!(state.reserve(&state.stored, 10, 1), 0);
        assert!(state.limited.load(Ordering::SeqCst));
    }
    #[test]
    fn incomplete_redaction_suffix_is_not_emitted_before_eof() {
        let mut redactor = Redactor::new(Arc::new(vec![b"secret-value".to_vec()]));
        assert!(redactor.feed(b"secret-", false).is_empty());
        assert_eq!(redactor.feed(b"value", true), b"[REDACTED]");
    }
}
