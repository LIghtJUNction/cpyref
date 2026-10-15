//! Locate literal, non-overlapping matches without expanding complete saved bodies.
use crate::{
    error::{Error, Result},
    store::{RecordKind, Store},
};
use regex::bytes::RegexBuilder;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub ignore_case: bool,
    /// Number of matches to skip, not a byte offset.
    pub offset: usize,
    pub limit: u32,
    /// Combined source-byte budget for all returned excerpts.
    pub max_bytes: u32,
    /// Requested context on each side of a match.
    pub context_bytes: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            ignore_case: false,
            offset: 0,
            limit: 20,
            max_bytes: 4096,
            context_bytes: 80,
        }
    }
}

/// Verify saved bytes, then count every non-overlapping literal match and show one page.
/// Line numbers refer to the saved payload; byte ends are exclusive. Excerpts are a
/// lossy UTF-8 display only, and their combined source bytes never exceed max_bytes.
pub fn find(store: &Store, id: &str, query: &str, options: &Options) -> Result<Value> {
    if query.is_empty() || query.len() > 4096 {
        return Err(Error::invalid(
            "Find query must contain 1-4096 UTF-8 bytes.",
        ));
    }
    if !(1..=1000).contains(&options.limit)
        || !(1..=65536).contains(&options.max_bytes)
        || options.context_bytes > 4096
    {
        return Err(Error::invalid(
            "Find requires limit 1-1000, max-bytes 1-65536, and context-bytes 0-4096.",
        ));
    }
    let matcher = RegexBuilder::new(&regex::escape(query))
        .case_insensitive(options.ignore_case)
        .size_limit(4 * 1024 * 1024)
        .build()
        .map_err(|e| Error::new("INVALID_ARGUMENT", 4, e.to_string()))?;
    require_bytes(store.info(id)?.kind)?;
    // get verifies the entire payload, including corruption outside matched excerpts.
    // Byte snapshots have a 128 MiB ceiling; do not retain a position for every hit.
    let (metadata, data) = store.get(id)?;
    require_bytes(metadata.kind)?;
    let mut matches = Vec::new();
    let mut total = 0;
    let mut remaining = options.max_bytes as usize;
    let mut content_truncated = false;
    let mut line_cursor = 0;
    let mut current_line = 1;
    for (index, found) in matcher.find_iter(&data).enumerate() {
        total = index + 1;
        if index < options.offset || matches.len() >= options.limit as usize {
            continue;
        }
        let start = found.start();
        let end = found.end();
        current_line += memchr::memchr_iter(b'\n', &data[line_cursor..start]).count();
        line_cursor = start;
        // A trailing newline belongs to its preceding line, as with copy selectors.
        let line_end = current_line + memchr::memchr_iter(b'\n', &data[start..end - 1]).count();
        let wanted_start = start.saturating_sub(options.context_bytes as usize);
        let wanted_end = end
            .saturating_add(options.context_bytes as usize)
            .min(data.len());
        let (content_start, content_end) =
            excerpt_range(start, end, wanted_start, wanted_end, remaining);
        let shown = content_end - content_start;
        let clipped = content_start != wanted_start || content_end != wanted_end;
        content_truncated |= clipped;
        remaining -= shown;
        matches.push(json!({
            "index": index, "byte_start": start, "byte_end": end,
            "line_start": current_line, "line_end": line_end,
            "content_start": content_start, "content_end": content_end,
            "content": String::from_utf8_lossy(&data[content_start..content_end]),
            "context_truncated": clipped,
        }));
    }
    let next_offset = options.offset.saturating_add(matches.len()).min(total);
    Ok(json!({
        "reference": metadata.id, "sha256": metadata.sha256,
        "total": total, "offset": options.offset, "limit": options.limit,
        "next_offset": next_offset, "truncated": next_offset < total,
        "encoding": "utf-8-lossy", "match_mode": "non_overlapping",
        "matches": matches, "content_bytes": options.max_bytes as usize - remaining,
        "content_truncated": content_truncated,
    }))
}

fn require_bytes(kind: RecordKind) -> Result<()> {
    if kind != RecordKind::Bytes {
        return Err(Error::new(
            "WRONG_KIND",
            4,
            "Find requires a bytes reference; use logs/snapshot/bundle commands for typed records.",
        ));
    }
    Ok(())
}

fn excerpt_range(
    start: usize,
    end: usize,
    wanted_start: usize,
    wanted_end: usize,
    budget: usize,
) -> (usize, usize) {
    if wanted_end - wanted_start <= budget {
        return (wanted_start, wanted_end);
    }
    // Spend scarce bytes on the match before its surroundings. Preserve hit positions
    // even after earlier excerpts have consumed the whole combined budget.
    let match_bytes = end - start;
    if budget <= match_bytes {
        return (start, start + budget);
    }
    let context_budget = budget - match_bytes;
    let left = (start - wanted_start).min(context_budget / 2);
    let right = (wanted_end - end).min(context_budget - left);
    let left = (start - wanted_start).min(context_budget - right);
    (start - left, end + right)
}
