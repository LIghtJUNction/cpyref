//! Literal search over scoped reference metadata and copied bytes, never body expansion.
use crate::{
    error::{Error, Result},
    store::{Metadata, RecordKind, Store},
};
use regex::bytes::{Regex, RegexBuilder};
use serde::Serialize;

pub struct Matcher {
    regex: Regex,
    overlap: usize,
}
impl Matcher {
    pub fn new(query: &str, ignore_case: bool) -> Result<Self> {
        if query.is_empty() || query.len() > 4096 {
            return Err(Error::invalid(
                "Search query must contain 1-4096 UTF-8 bytes.",
            ));
        }
        let regex = RegexBuilder::new(&regex::escape(query))
            .case_insensitive(ignore_case)
            .size_limit(4 * 1024 * 1024)
            .build()
            .map_err(|e| Error::new("INVALID_ARGUMENT", 4, e.to_string()))?;
        let longest = if ignore_case {
            query.chars().count() * 4
        } else {
            query.len()
        };
        Ok(Self {
            regex,
            overlap: longest.saturating_sub(1),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub metadata: Metadata,
    pub matched_in: Vec<&'static str>,
}

pub fn kind_name(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Bytes => "bytes",
        RecordKind::Snapshot => "snapshot",
        RecordKind::Run => "run",
        RecordKind::Bundle => "bundle",
    }
}

pub fn search(
    store: &Store,
    entries: impl IntoIterator<Item = Metadata>,
    matcher: &Matcher,
    metadata_only: bool,
) -> Result<Vec<Hit>> {
    let mut hits = Vec::new();
    for metadata in entries {
        let mut matched_in = Vec::new();
        for (name, text) in [
            ("id", metadata.id.as_str()),
            ("source", metadata.source.as_str()),
            ("note", metadata.note.as_deref().unwrap_or("")),
            ("kind", kind_name(metadata.kind)),
        ] {
            if matcher.regex.is_match(text.as_bytes()) {
                matched_in.push(name);
            }
        }
        // Typed manifests and run containers are not copied text. Use their own bounded views.
        if !metadata_only
            && metadata.kind == RecordKind::Bytes
            && store.contains_payload(&metadata.id, &matcher.regex, matcher.overlap)?
        {
            matched_in.push("content");
        }
        if !matched_in.is_empty() {
            hits.push(Hit {
                metadata,
                matched_in,
            });
        }
    }
    Ok(hits)
}
