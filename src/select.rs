use crate::error::{Error, Result};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::ops::Range;

const MAX_MATCHES: usize = 10_000;
const MAX_CANDIDATES: usize = 20;

#[derive(Debug, Default, Clone)]
pub struct Spec {
    pub lines: Option<String>,
    pub text: Option<String>,
    pub regex: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub markers_regex: bool,
    pub exclude_markers: bool,
    pub exclude_start: bool,
    pub exclude_end: bool,
    pub group: Option<String>,
    pub within: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub context_lines: usize,
    pub occurrence: Option<usize>,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dotall: bool,
}

/// Byte offsets are zero-based and half-open. Line numbers are one-based.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub selector: String,
}

impl Span {
    pub fn range(&self) -> Range<usize> {
        self.byte_start..self.byte_end
    }
}

/// Line starts exclude an artificial empty line after a final newline.
fn line_starts(data: &[u8]) -> Vec<usize> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut starts = vec![0];
    for (i, &byte) in data.iter().enumerate() {
        if byte == b'\n' && i + 1 < data.len() {
            starts.push(i + 1);
        }
    }
    starts
}

fn line_range(value: &str, starts: &[usize], length: usize) -> Result<Range<usize>> {
    let (first, last) = value.split_once(':').unwrap_or((value, value));
    let start = first.parse::<usize>().map_err(|_| Error::invalid("Use N or START:END for a line range."))?;
    let end = last.parse::<usize>().map_err(|_| Error::invalid("Use N or START:END for a line range."))?;
    if start == 0 || end < start {
        return Err(Error::invalid("Line numbers start at 1; END must be at least START."));
    }
    if end > starts.len() {
        return Err(Error::new("NO_MATCH", 2, "The requested lines do not exist.")
            .details(json!({"reason": "LINE_OUT_OF_RANGE", "line_count": starts.len(), "requested": value})));
    }
    Ok(starts[start - 1]..starts.get(end).copied().unwrap_or(length))
}

fn describe(range: Range<usize>, starts: &[usize], selector: &str) -> Span {
    Span {
        byte_start: range.start,
        byte_end: range.end,
        line_start: starts.partition_point(|&s| s <= range.start),
        line_end: starts.partition_point(|&s| s < range.end),
        selector: selector.to_owned(),
    }
}

fn compile(pattern: &str, spec: &Spec) -> Result<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(spec.ignore_case)
        .multi_line(spec.multiline)
        .dot_matches_new_line(spec.dotall)
        .size_limit(4 * 1024 * 1024)
        .build()
        .map_err(|e| Error::new("INVALID_REGEX", 4, e.to_string()))
}

fn push_match(ranges: &mut Vec<Range<usize>>, range: Range<usize>) -> Result<()> {
    if range.is_empty() {
        return Err(Error::new("EMPTY_SELECTION", 8, "A selector produced a zero-byte match; use a non-empty selector."));
    }
    if ranges.len() == MAX_MATCHES {
        return Err(Error::new("MATCH_LIMIT", 9, "Too many matches; narrow the search with --within or a more specific pattern.")
            .details(json!({"limit": MAX_MATCHES})));
    }
    ranges.push(range);
    Ok(())
}

/// Literal matches include overlaps, so a hidden overlapping match is not mistaken for uniqueness.
fn literal_matches(text: &str, pattern: &str, spec: &Spec) -> Result<Vec<Range<usize>>> {
    if pattern.is_empty() {
        return Err(Error::invalid("A text pattern cannot be empty."));
    }
    let insensitive = if spec.ignore_case { Some(compile(&regex::escape(pattern), spec)?) } else { None };
    let mut matches = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let found = if let Some(re) = &insensitive {
            re.find_at(text, cursor).map(|m| m.range())
        } else {
            text[cursor..].find(pattern).map(|i| cursor + i..cursor + i + pattern.len())
        };
        let Some(range) = found else { break };
        // Advance by one Unicode scalar, not by match length, to retain overlaps.
        cursor = range.start + text[range.start..].chars().next().map_or(1, char::len_utf8);
        push_match(&mut matches, range)?;
    }
    Ok(matches)
}

fn regex_matches(text: &str, pattern: &str, group: Option<&str>, spec: &Spec) -> Result<Vec<Range<usize>>> {
    let re = compile(pattern, spec)?;
    let index = match group {
        None => 0,
        Some(value) => {
            if let Ok(index) = value.parse::<usize>() {
                if index >= re.captures_len() {
                    return Err(Error::new("INVALID_GROUP", 4, "The regex capture group does not exist."));
                }
                index
            } else {
                re.capture_names().position(|name| name == Some(value))
                    .ok_or_else(|| Error::new("INVALID_GROUP", 4, "The named regex capture group does not exist."))?
            }
        }
    };
    let mut matches = Vec::new();
    // Regex matches follow the regex crate's leftmost, non-overlapping semantics.
    for (seen, captures) in re.captures_iter(text).enumerate() {
        if seen >= MAX_MATCHES {
            return Err(Error::new("MATCH_LIMIT", 9, "Too many regex matches; narrow the search with --within."));
        }
        if let Some(matched) = captures.get(index) {
            push_match(&mut matches, matched.range())?;
        }
    }
    Ok(matches)
}

fn marker_matches(text: &str, spec: &Spec) -> Result<Vec<Range<usize>>> {
    let start = spec.from.as_deref().unwrap_or_default();
    let end = spec.to.as_deref().unwrap_or_default();
    let (starts, ends) = if spec.markers_regex {
        (regex_matches(text, start, None, spec)?, regex_matches(text, end, None, spec)?)
    } else {
        (literal_matches(text, start, spec)?, literal_matches(text, end, spec)?)
    };
    if starts.is_empty() {
        return Err(Error::new("NO_MATCH", 2, "The start marker was not found.")
            .details(json!({"reason": "START_MARKER_NOT_FOUND"})));
    }
    let mut ranges = Vec::new();
    for start in starts {
        // Defined semantics: each start pairs with its nearest non-overlapping following end.
        let end_index = ends.partition_point(|end| end.start < start.end);
        if let Some(end) = ends.get(end_index) {
            let left = if spec.exclude_markers || spec.exclude_start { start.end } else { start.start };
            let right = if spec.exclude_markers || spec.exclude_end { end.start } else { end.end };
            let range = left..right;
            push_match(&mut ranges, range)?;
        }
    }
    if ranges.is_empty() {
        return Err(Error::new("NO_MATCH", 2, "No end marker follows a start marker.")
            .details(json!({"reason": "END_MARKER_NOT_FOUND"})));
    }
    Ok(ranges)
}

fn validate(spec: &Spec) -> Result<()> {
    let selectors = [spec.lines.is_some(), spec.text.is_some(), spec.regex.is_some(), spec.from.is_some()];
    if selectors.into_iter().filter(|x| *x).count() > 1 {
        return Err(Error::invalid("Choose only one of --lines, --text, --regex, or --from/--to."));
    }
    if spec.from.is_some() != spec.to.is_some() {
        return Err(Error::invalid("--from and --to must be used together."));
    }
    if (spec.markers_regex || spec.exclude_markers || spec.exclude_start || spec.exclude_end) && spec.from.is_none() {
        return Err(Error::invalid("Marker options require --from and --to."));
    }
    if spec.exclude_markers && (spec.exclude_start || spec.exclude_end) {
        return Err(Error::invalid("--exclude-markers cannot be combined with independent boundary options."));
    }
    if spec.group.is_some() && spec.regex.is_none() {
        return Err(Error::invalid("--group requires --regex."));
    }
    if (spec.multiline || spec.dotall) && spec.regex.is_none() && !spec.markers_regex {
        return Err(Error::invalid("--multiline and --dotall require a regex selector."));
    }
    if spec.ignore_case && spec.text.is_none() && spec.regex.is_none() && spec.from.is_none() {
        return Err(Error::invalid("--ignore-case requires a text, regex, or marker selector."));
    }
    if spec.occurrence == Some(0) {
        return Err(Error::invalid("--occurrence starts at 1."));
    }
    if spec.before.as_deref() == Some("") || spec.after.as_deref() == Some("") {
        return Err(Error::invalid("Context text cannot be empty."));
    }
    Ok(())
}

/// Select one original byte range. Nothing is written when this function fails.
pub fn preview(data: &[u8], spec: &Spec) -> Result<Preview> {
    validate(spec)?;
    let starts = line_starts(data);
    let scope = match &spec.within {
        Some(value) => line_range(value, &starts, data.len())?,
        None => 0..data.len(),
    };
    let (mut ranges, kind) = if let Some(value) = &spec.lines {
        (vec![line_range(value, &starts, data.len())?], "lines")
    } else if spec.text.is_some() || spec.regex.is_some() || spec.from.is_some() {
        let text = std::str::from_utf8(&data[scope.clone()])
            .map_err(|_| Error::new("INVALID_UTF8", 4, "Text matching requires UTF-8 in the selected scope; whole-file and line copying preserve arbitrary bytes."))?;
        let (local, kind) = if let Some(pattern) = &spec.text {
            (literal_matches(text, pattern, spec)?, "text")
        } else if let Some(pattern) = &spec.regex {
            (regex_matches(text, pattern, spec.group.as_deref(), spec)?, "regex")
        } else {
            (marker_matches(text, spec)?, "markers")
        };
        (local.into_iter().map(|r| r.start + scope.start..r.end + scope.start).collect::<Vec<_>>(), kind)
    } else {
        (vec![scope.clone()], "whole")
    };
    ranges.retain(|r| r.start >= scope.start && r.end <= scope.end);
    if spec.before.is_some() || spec.after.is_some() {
        ranges.retain(|r| {
            let first_line = starts.partition_point(|&s| s <= r.start).saturating_sub(1);
            let last_line = starts.partition_point(|&s| s < r.end).saturating_sub(1);
            let left = starts.get(first_line.saturating_sub(spec.context_lines)).copied().unwrap_or(0).max(scope.start);
            let right = starts.get(last_line.saturating_add(spec.context_lines).saturating_add(1)).copied().unwrap_or(data.len()).min(scope.end);
            let contains = |bytes: &[u8], pattern: &Option<String>| {
                pattern.as_ref().is_none_or(|p| bytes.windows(p.len()).any(|window| window == p.as_bytes()))
            };
            contains(&data[left..r.start], &spec.before) && contains(&data[r.end..right], &spec.after)
        });
    }
    if ranges.is_empty() {
        return Err(Error::new("NO_MATCH", 2, "No content satisfies the selector and context filters."));
    }
    let match_count = ranges.len();
    let candidates = ranges.iter().take(MAX_CANDIDATES).cloned().map(|r| describe(r, &starts, kind)).collect();
    let selected = if let Some(occurrence) = spec.occurrence {
        ranges.get(occurrence - 1).cloned().ok_or_else(|| {
            Error::new("NO_MATCH", 2, "The requested occurrence does not exist.")
                .details(json!({"reason": "OCCURRENCE_OUT_OF_RANGE", "requested": occurrence, "match_count": ranges.len()}))
        })?
    } else if ranges.len() == 1 {
        ranges.remove(0)
    } else {
        let candidates: Vec<_> = ranges.iter().take(MAX_CANDIDATES).enumerate().map(|(i, r)| {
            json!({"occurrence": i + 1, "bytes": r.end - r.start, "span": describe(r.clone(), &starts, kind)})
        }).collect();
        return Err(Error::new("AMBIGUOUS", 3, "Multiple matches found; narrow the selector or specify --occurrence.")
            .details(json!({"match_count": ranges.len(), "candidates": candidates, "truncated": ranges.len() > MAX_CANDIDATES})));
    };
    if selected.is_empty() {
        return Err(Error::new("EMPTY_SELECTION", 8, "The selected content is empty."));
    }
    Ok(Preview { match_count, candidates, truncated: match_count > MAX_CANDIDATES, selected: describe(selected, &starts, kind) })
}

#[derive(Debug, Serialize)]
pub struct Preview {
    pub match_count: usize,
    pub candidates: Vec<Span>,
    pub truncated: bool,
    pub selected: Span,
}

pub fn select(data: &[u8], spec: &Spec) -> Result<Span> {
    preview(data, spec).map(|p| p.selected)
}
