use crate::{
    error::{Error, Result},
    store::{RecordKind, Store},
    MAX_BYTES,
};
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path};

/// Check original offsets first, then count every overlapping exact byte match.
pub fn verify(store: &Store, id: &str) -> Result<(Value, bool)> {
    let (metadata, bytes) = store.get(id)?;
    if metadata.kind != RecordKind::Bytes {
        return Err(Error::new(
            "WRONG_KIND",
            4,
            "Use snapshot check for source manifests and logs for run records.",
        ));
    }
    if metadata.source == "<stdin>" {
        return Ok((
            json!({"reference": id, "status": "no_source", "semantic_position_verified": false}),
            false,
        ));
    }
    let path = Path::new(&metadata.source);
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((
                json!({"reference": id, "status": "source_missing", "semantic_position_verified": false}),
                false,
            ))
        }
        Err(e) => return Err(Error::io("Inspect reference source", e)),
    };
    if !meta.is_file() {
        return Err(Error::new(
            "UNSUPPORTED_SOURCE",
            4,
            "Reference source is no longer a regular file.",
        ));
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_BYTES as u64 + 1).read_to_end(&mut data))
        .map_err(|e| Error::io("Read reference source", e))?;
    if data.len() > MAX_BYTES {
        return Err(Error::new(
            "SIZE_LIMIT",
            9,
            "Source verification is limited to 128 MiB; status is unknown.",
        ));
    }
    let before = meta.modified().ok();
    let after = fs::metadata(path).map_err(|e| Error::io("Recheck reference source", e))?;
    if meta.len() != after.len() || before != after.modified().ok() {
        return Err(Error::new(
            "SOURCE_CHANGED",
            4,
            "Source changed during verification; retry.",
        ));
    }
    let original = data.get(metadata.span.range()) == Some(bytes.as_slice());
    let mut count = 0_usize;
    let mut candidates = Vec::new();
    let mut previous_start = 0;
    let mut line_start = 1;
    // Count overlapping binary matches without repeatedly comparing the whole snippet.
    for start in overlapping_matches(&data, &bytes) {
        count += 1;
        if candidates.len() < 20 {
            line_start += data[previous_start..start]
                .iter()
                .filter(|&&b| b == b'\n')
                .count();
            previous_start = start;
            let line_end = line_start
                + data[start..start + bytes.len() - 1]
                    .iter()
                    .filter(|&&b| b == b'\n')
                    .count();
            candidates.push(json!({"byte_start": start, "byte_end": start + bytes.len(), "line_start": line_start, "line_end": line_end}));
        }
    }
    let status = if original {
        "original_match"
    } else if count == 1 {
        "moved_unique"
    } else if count > 1 {
        "multiple_matches"
    } else {
        "changed"
    };
    Ok((
        json!({"reference": id, "status": status, "original_position_matches": original,
        "source": metadata.source, "original_span": metadata.span, "match_count": count, "candidates": candidates, "truncated": count > candidates.len(),
        "semantic_position_verified": false}),
        original,
    ))
}

fn overlapping_matches<'a>(data: &'a [u8], pattern: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    let finder = memchr::memmem::Finder::new(pattern);
    let mut cursor = 0;
    std::iter::from_fn(move || {
        let found = finder.find(&data[cursor..])? + cursor;
        cursor = found + 1;
        Some(found)
    })
}
