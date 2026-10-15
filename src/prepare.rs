//! Prepare notes and selected files as an immutable, named handoff bundle.
use crate::{
    bundle,
    context::Context,
    error::{Error, Result},
    select::{self, Span, Spec},
    store::Metadata,
    MAX_BYTES,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

struct Input {
    name: String,
    source: String,
    span: Span,
    data: Vec<u8>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

fn read_source(path: &Path, remaining: usize, limit: usize) -> Result<Vec<u8>> {
    let reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(io::stdin())
    } else {
        let metadata =
            std::fs::metadata(path).map_err(|e| Error::io("Read handoff input metadata", e))?;
        if !metadata.is_file() {
            return Err(Error::new(
                "UNSUPPORTED_SOURCE",
                4,
                "Handoff inputs must be regular files; only notes may use - for stdin.",
            ));
        }
        let file = File::open(path).map_err(|e| Error::io("Open handoff input", e))?;
        if !file
            .metadata()
            .map_err(|e| Error::io("Read opened handoff input metadata", e))?
            .is_file()
        {
            return Err(Error::new(
                "UNSUPPORTED_SOURCE",
                4,
                "Handoff inputs must be regular files.",
            ));
        }
        Box::new(file)
    };
    let mut bytes = Vec::new();
    reader
        .take(remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("Read handoff input", e))?;
    if bytes.len() > remaining {
        return Err(Error::new(
            "SIZE_LIMIT",
            9,
            "Combined notes and attachments exceed --max-bytes.",
        )
        .details(json!({"limit": limit, "remaining_bytes": remaining})));
    }
    Ok(bytes)
}

fn partial_failure(mut error: Error, created: &[Value], stage: &str) -> Error {
    if created.is_empty() {
        return error;
    }
    if !error.details.is_object() {
        error.details = json!({"original_details": error.details});
    }
    let details = error.details.as_object_mut().expect("details is an object");
    details.insert("created_references".into(), json!(created));
    details.insert("stage".into(), json!(stage));
    details.insert("side_effects".into(), json!(true));
    details.insert("retry_safe".into(), json!(false));
    error
}

/// Validate and read every input before saving anything. The byte budget applies
/// to notes and all attachments together, and excludes the small bundle manifest.
/// Publication still checks capacity under the store's lock for every record.
/// A later failure retains and reports completed references rather than deleting
/// data that another agent might already be using.
pub fn create(
    context: &Context,
    input: &Path,
    attachments: &[String],
    max_bytes: usize,
) -> Result<Metadata> {
    if max_bytes == 0 || max_bytes > MAX_BYTES {
        return Err(Error::invalid(
            "--max-bytes must be between 1 and 134217728.",
        ));
    }
    if attachments.len() > 99 {
        return Err(Error::invalid(
            "A handoff accepts notes and at most 99 attachments.",
        ));
    }
    let mut sources = vec![("notes".to_owned(), input.to_owned())];
    let mut names = BTreeSet::from(["notes".to_owned()]);
    for attachment in attachments {
        let (name, path) = attachment
            .split_once('=')
            .ok_or_else(|| Error::invalid("Expected --attach NAME=FILE."))?;
        if !valid_name(name) || !names.insert(name.to_owned()) {
            return Err(Error::invalid("Attachment names must be unique, 1-64 ASCII letters/digits/_.-, starting with a letter or digit; notes is reserved."));
        }
        if path.is_empty() || path == "-" {
            return Err(Error::invalid(
                "Attachments require a file path; - is only supported by --input.",
            ));
        }
        sources.push((name.to_owned(), PathBuf::from(path)));
    }

    let store = context.store()?;
    if context.max_entries != 0 {
        let count = store.list()?.len();
        let additional = sources.len() + 1;
        let required = count.saturating_add(additional);
        if required > context.max_entries {
            return Err(Error::new(
                "ENTRY_LIMIT",
                9,
                "This space does not have room for every handoff input and its bundle.",
            )
            .details(json!({"count": count, "additional_entries": additional,
                "required_entries": required, "max_entries": context.max_entries})));
        }
    }

    let mut inputs = Vec::with_capacity(sources.len());
    let mut total_bytes = 0;
    for (name, path) in sources {
        let path = if path == Path::new("-") {
            path
        } else {
            context.cwd.join(path)
        };
        let data = read_source(&path, max_bytes - total_bytes, max_bytes)?;
        let span = select::select(&data, &Spec::default())?;
        total_bytes += data.len();
        let source = if path == Path::new("-") {
            "<stdin>".to_owned()
        } else {
            path.to_string_lossy().into_owned()
        };
        inputs.push(Input {
            name,
            source,
            span,
            data,
        });
    }

    let mut created = Vec::with_capacity(inputs.len());
    let mut items = Vec::with_capacity(inputs.len());
    for input in inputs {
        let metadata = store
            .put_with_options(
                &input.data,
                input.source,
                input.span,
                None,
                context.max_entries,
            )
            .map_err(|error| partial_failure(error, &created, "save_input"))?;
        items.push(format!("{}={}", input.name, metadata.id));
        created.push(json!({"name": input.name, "id": metadata.id}));
    }
    bundle::create(context, &items)
        .map_err(|error| partial_failure(error, &created, "create_bundle"))
}

#[cfg(test)]
mod tests {
    use super::partial_failure;
    use crate::error::Error;
    use serde_json::json;

    #[test]
    fn partial_errors_keep_the_cause_and_completed_reference_receipts() {
        let error = Error::new("ENTRY_LIMIT", 9, "capacity changed")
            .details(json!({"count": 2, "max_entries": 2}));
        let created = [json!({"name": "notes", "id": "c123456789abc"})];
        let error = partial_failure(error, &created, "create_bundle");
        assert_eq!(error.code, "ENTRY_LIMIT");
        assert_eq!(error.exit, 9);
        assert_eq!(error.message, "capacity changed");
        assert_eq!(error.details["count"], 2);
        assert_eq!(error.details["created_references"], json!(created));
        assert_eq!(error.details["stage"], "create_bundle");
        assert_eq!(error.details["side_effects"], true);
        assert_eq!(error.details["retry_safe"], false);
    }

    #[test]
    fn failure_before_a_publication_does_not_claim_saved_references() {
        let error = Error::new("IO_ERROR", 5, "write failed").details(json!({"reason": 1}));
        let error = partial_failure(error, &[], "save_input");
        assert_eq!(error.details, json!({"reason": 1}));
    }
}
