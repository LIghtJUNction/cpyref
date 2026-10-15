//! Read one reference, not payload bytes, from a CLI pipe.
use crate::{context::Context, error::{Error, Result}, store::valid_id};
use serde::Deserialize;
use serde_json::Value;
use std::io::{self, IsTerminal, Read};

const MAX_INPUT: u64 = 64 * 1024;

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    context: Value,
    reference: Option<Value>,
    plan: Option<String>,
}

fn invalid() -> Error {
    Error::new("REF_INPUT", 4,
        "Expected one complete reference, or a successful cpyref JSON result. Do not pipe payloads, errors, or multiple references.")
}

pub fn resolve(context: &Context, value: &str, plan: bool) -> Result<String> {
    if value != "-" {
        return Ok(value.to_owned());
    }
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err(Error::new("REF_INPUT", 4,
            "The '-' reference requires a pipe. Use cpyref copy FILE | cpyref paste - --output FILE."));
    }
    read(context, stdin.lock(), plan)
}

fn read(context: &Context, input: impl Read, plan: bool) -> Result<String> {
    let mut bytes = Vec::new();
    input.take(MAX_INPUT + 1).read_to_end(&mut bytes)
        .map_err(|e| Error::io("Read piped reference", e))?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(invalid());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?
        .trim_matches(|c: char| c.is_ascii_whitespace());
    if valid_id(text) {
        return Ok(text.to_owned());
    }
    let envelope: Envelope = serde_json::from_str(text).map_err(|_| invalid())?;
    if !envelope.ok || (!plan && envelope.plan.is_some()) {
        return Err(invalid());
    }
    let current = context.report();
    // JSON describes a space; it must never redirect the reader into that space.
    for key in ["scope", "namespace", "project_root", "store_base", "store_path"] {
        if envelope.context.get(key).is_none()
            || envelope.context.get(key) != current.get(key) {
            return Err(Error::new("REF_CONTEXT", 4,
                "Piped reference belongs to a different space. Keep the original project, namespace, and store; no automatic switch was made."));
        }
    }
    let id = if plan {
        envelope.plan.as_deref()
    } else {
        envelope.reference.as_ref().and_then(|r| r.get("id")).and_then(Value::as_str)
    }.filter(|id| valid_id(id)).ok_or_else(invalid)?;
    Ok(id.to_owned())
}
