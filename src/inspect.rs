//! Bounded views of saved bytes; complete payload integrity is checked first.
use crate::{
    error::{Error, Result},
    store::{RecordKind, Store},
};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};

fn wrong_kind() -> Error {
    Error::new(
        "WRONG_KIND",
        4,
        "Read accepts byte references. Use cpyref logs, snapshot show, bundle show, or apply --dry-run for typed records.",
    )
}

/// Find inclusive one-based lines without retaining the complete saved body.
fn line_window(reader: &mut impl Read, total: u64, value: &str) -> Result<(u64, u64)> {
    let (first, last) = value.split_once(':').unwrap_or((value, value));
    let first = first
        .parse::<u64>()
        .map_err(|_| Error::invalid("Use N or START:END for a line range."))?;
    let last = last
        .parse::<u64>()
        .map_err(|_| Error::invalid("Use N or START:END for a line range."))?;
    if first == 0 || last < first {
        return Err(Error::invalid(
            "Line numbers start at 1; END must be at least START.",
        ));
    }

    let mut buffer = [0_u8; 65536];
    let mut position = 0_u64;
    let mut line = 1_u64;
    let mut start = (first == 1).then_some(0);
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| Error::io("Find saved lines", e))?;
        if n == 0 {
            break;
        }
        for (index, byte) in buffer[..n].iter().enumerate() {
            let next = position + index as u64 + 1;
            // A final newline belongs to its line and does not create a new line.
            if *byte == b'\n' && next < total {
                if line == last {
                    return Ok((start.unwrap_or(0), next));
                }
                line += 1;
                if line == first {
                    start = Some(next);
                }
            }
        }
        position += n as u64;
    }
    if last > line {
        return Err(
            Error::new("NO_MATCH", 2, "The requested saved lines do not exist.").details(
                json!({"reason": "LINE_OUT_OF_RANGE", "line_count": line, "requested": value}),
            ),
        );
    }
    Ok((start.unwrap_or(0), total))
}

/// Inspect a bounded portion of a byte reference without changing its stored bytes.
/// Offsets and returned continuation positions are relative to the saved payload.
pub fn read(
    store: &Store,
    id: &str,
    offset: u64,
    max_bytes: u32,
    lines: Option<&str>,
    tail: bool,
) -> Result<Value> {
    if !(1..=65536).contains(&max_bytes) {
        return Err(Error::invalid("--max-bytes must be between 1 and 65536."));
    }
    if tail && (lines.is_some() || offset != 0) {
        return Err(Error::invalid(
            "Cannot combine --tail with --lines or --offset.",
        ));
    }
    if store.info(id)?.kind != RecordKind::Bytes {
        return Err(wrong_kind());
    }
    let (mut file, metadata) = store.verified_file(id)?;
    if metadata.kind != RecordKind::Bytes {
        return Err(wrong_kind());
    }
    let payload_start = file
        .stream_position()
        .map_err(|e| Error::io("Read saved payload position", e))?;
    let total = metadata.bytes as u64;
    let (selection_start, end) = if let Some(lines) = lines {
        line_window(&mut (&mut file).take(total), total, lines)?
    } else if tail {
        (total.saturating_sub(u64::from(max_bytes)), total)
    } else {
        (0, total)
    };
    let start = offset.clamp(selection_start, end);
    file.seek(SeekFrom::Start(payload_start + start))
        .map_err(|e| Error::io("Seek saved excerpt", e))?;
    let mut bytes = Vec::new();
    file.take((end - start).min(u64::from(max_bytes)))
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("Read saved excerpt", e))?;
    let next = start + bytes.len() as u64;
    let mut report = json!({
        "reference": metadata.id, "sha256": metadata.sha256,
        "offset": start, "bytes": bytes.len(), "total_bytes": total,
        "next_offset": next, "truncated": next < end,
        "encoding": "utf-8-lossy", "content": String::from_utf8_lossy(&bytes)
    });
    if lines.is_some() {
        report["selection_start"] = json!(selection_start);
        report["selection_end"] = json!(end);
    }
    Ok(report)
}
