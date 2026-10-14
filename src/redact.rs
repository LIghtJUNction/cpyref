//! Exact-value redaction before storage. This is not automatic secret discovery.
use crate::error::{Error, Result};
use std::sync::Arc;

pub struct Prepared {
    pub names: Vec<String>,
    patterns: Arc<Vec<Vec<u8>>>,
}

impl Prepared {
    pub fn new(selected: &[String]) -> Result<Self> {
        let mut names = selected.to_vec();
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
        patterns.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        patterns.dedup();
        Ok(Self {
            names,
            patterns: Arc::new(patterns),
        })
    }

    pub fn stream(&self) -> Redactor {
        Redactor::new(self.patterns.clone())
    }

    pub fn command(&self, argv: &[String]) -> Vec<String> {
        argv.iter()
            .map(|arg| {
                String::from_utf8_lossy(&self.stream().feed(arg.as_bytes(), true)).into_owned()
            })
            .collect()
    }
}

pub struct Redactor {
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

    /// Keep enough original suffix to detect values split over reads. On truncation,
    /// drop the redactor instead of setting eof: uncompleted raw suffixes must not leak.
    pub fn feed(&mut self, bytes: &[u8], eof: bool) -> Vec<u8> {
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
    fn incomplete_suffix_is_not_emitted_before_eof() {
        let mut redactor = Redactor::new(Arc::new(vec![b"secret-value".to_vec()]));
        assert!(redactor.feed(b"secret-", false).is_empty());
        assert_eq!(redactor.feed(b"value", true), b"[REDACTED]");
    }

    #[test]
    fn no_rules_preserve_arbitrary_bytes() {
        let bytes = [0, 255, b'\r', b'\n'];
        let mut redactor = Redactor::new(Arc::new(Vec::new()));
        assert_eq!(redactor.feed(&bytes, true), bytes);
    }
}
