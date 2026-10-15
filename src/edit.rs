//! Exact-byte edits. Hash preconditions and a stable writer lock prevent stale cooperating writes.
use crate::{
    context::Context,
    error::{Error, Result},
    select::{preview, Spec},
    store::{RecordKind, Store},
    MAX_BYTES,
};
use clap::Args as ClapArgs;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::{Read, Write},
    ops::Range,
    path::{Component, Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, ClapArgs, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    /// Complete reference containing the bytes to insert or replace with.
    pub id: String,
    /// Existing ordinary file. Its permissions and unselected bytes are preserved.
    pub target: PathBuf,
    /// Replacement: inclusive one-based line range.
    #[arg(long)]
    pub lines: Option<String>,
    /// Replacement: uniquely matching literal text.
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long)]
    pub before_line: Option<usize>,
    #[arg(long)]
    pub after_line: Option<usize>,
    #[arg(long)]
    pub before_text: Option<String>,
    #[arg(long)]
    pub after_text: Option<String>,
    #[arg(long)]
    pub at_start: bool,
    #[arg(long)]
    pub at_end: bool,
    /// Preview positions, sizes and hashes without writing or expanding content.
    #[arg(long)]
    pub dry_run: bool,
    /// Save a guarded edit plan without changing the target; use cpyref apply PLAN_ID next.
    #[arg(long, conflicts_with_all = ["dry_run", "expect_sha256"])]
    pub plan: bool,
    /// Required for direct writing: target SHA-256 from the approved preview or read.
    #[arg(long)]
    pub expect_sha256: Option<String>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn ordinary(metadata: &Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::new(
            "UNSAFE_OUTPUT",
            5,
            "Editing requires an ordinary file, not a link or a special file.",
        ));
    }
    if metadata.permissions().readonly() {
        return Err(Error::new(
            "PERMISSION_DENIED",
            5,
            "The edit target is read-only.",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::new(
                "UNSAFE_OUTPUT",
                5,
                "Editing refuses reparse points.",
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.nlink() != 1 || metadata.permissions().mode() & 0o6000 != 0 {
            return Err(Error::new(
                "UNSAFE_OUTPUT",
                5,
                "Editing refuses hard-linked or set-ID files.",
            ));
        }
    }
    Ok(())
}

fn checked_path(path: &Path) -> Result<PathBuf> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if matches!(component, Component::Normal(_)) {
            let metadata =
                fs::symlink_metadata(&prefix).map_err(|e| Error::io("Inspect edit path", e))?;
            if metadata.file_type().is_symlink() {
                return Err(Error::new(
                    "UNSAFE_OUTPUT",
                    5,
                    "Edit paths must not traverse symbolic links.",
                ));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(Error::new(
                        "UNSAFE_OUTPUT",
                        5,
                        "Edit paths must not traverse reparse points.",
                    ));
                }
            }
        }
    }
    fs::canonicalize(path).map_err(|e| Error::io("Resolve edit target", e))
}

fn read_target(path: &Path) -> Result<(Vec<u8>, Metadata)> {
    let metadata = fs::symlink_metadata(path).map_err(|e| Error::io("Inspect edit target", e))?;
    ordinary(&metadata)?;
    if metadata.len() > MAX_BYTES as u64 {
        return Err(Error::new("SIZE_LIMIT", 9, "Edit target exceeds 128 MiB."));
    }
    let file = File::open(path).map_err(|e| Error::io("Open edit target", e))?;
    let before = file
        .metadata()
        .map_err(|e| Error::io("Inspect open target", e))?;
    ordinary(&before)?;
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("Read edit target", e))?;
    let after = file
        .metadata()
        .map_err(|e| Error::io("Recheck open target", e))?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::new("SIZE_LIMIT", 9, "Edit target exceeds 128 MiB."));
    }
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != after.len()
    {
        return Err(Error::new(
            "TARGET_CHANGED",
            10,
            "The target changed while it was read.",
        ));
    }
    Ok((bytes, after))
}

fn selection(bytes: &[u8], lines: Option<String>, text: Option<String>) -> Result<Range<usize>> {
    Ok(preview(
        bytes,
        &Spec {
            lines,
            text,
            ..Spec::default()
        },
    )?
    .selected
    .range())
}

fn validate(args: &Args, replace: bool) -> Result<()> {
    if args.plan && (args.dry_run || args.expect_sha256.is_some()) {
        return Err(Error::invalid(
            "Use --plan by itself, or the existing --dry-run / --expect-sha256 route.",
        ));
    }
    let replacement = usize::from(args.lines.is_some()) + usize::from(args.text.is_some());
    let insertion = [
        args.before_line.is_some(),
        args.after_line.is_some(),
        args.before_text.is_some(),
        args.after_text.is_some(),
        args.at_start,
        args.at_end,
    ]
    .into_iter()
    .filter(|v| *v)
    .count();
    if (replace && (replacement != 1 || insertion != 0))
        || (!replace && (insertion != 1 || replacement != 0))
    {
        return Err(Error::invalid(if replace {
            "replace requires exactly one of --lines or --text; insert position options are not accepted."
        } else {
            "insert requires exactly one of --before-line, --after-line, --before-text, --after-text, --at-start or --at-end."
        }));
    }
    if let Some(hash) = &args.expect_sha256 {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::invalid(
                "--expect-sha256 must be a complete 64-character SHA-256 value.",
            ));
        }
    } else if !args.dry_run && !args.plan {
        return Err(Error::new(
            "PRECONDITION_REQUIRED",
            4,
            "Use --plan, then cpyref apply PLAN_ID. For direct writes, preview with --dry-run and provide --expect-sha256.",
        ));
    }
    Ok(())
}

pub fn execute(context: &Context, args: &Args, replace: bool) -> Result<Value> {
    execute_checked(context, args, replace, None)
}

pub(crate) fn execute_checked(
    context: &Context,
    args: &Args,
    replace: bool,
    expected_source: Option<&str>,
) -> Result<Value> {
    validate(args, replace)?;
    let (reference, inserted) = context.store()?.get(&args.id)?;
    if reference.kind != RecordKind::Bytes {
        return Err(Error::new(
            "WRONG_KIND",
            4,
            "Edits require a bytes reference, not a run or source manifest.",
        ));
    }
    if expected_source.is_some_and(|expected| expected != reference.sha256.as_str()) {
        return Err(Error::new("REF_CHANGED", 10,
            "The saved bytes no longer match this plan. Nothing was written; create a new plan from trusted input."));
    }
    let requested = context.cwd.join(&args.target);
    let path = checked_path(&requested)?;
    // Stable lock objects live outside source folders and are shared across namespaces.
    // Different storage bases and non-cooperating programs are not covered by this lock.
    let _lock = if args.dry_run || args.plan {
        None
    } else {
        let key = digest(path.as_os_str().as_encoded_bytes());
        let store = Store::scoped(
            context.store_base.clone(),
            context.store_base.join("edit-locks").join(key),
        )?;
        Some(store.edit_lock()?)
    };
    let (before, metadata) = read_target(&path)?;
    let before_sha256 = digest(&before);
    if args
        .expect_sha256
        .as_ref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&before_sha256))
    {
        return Err(Error::new(
            "TARGET_CHANGED",
            10,
            "Target hash does not match the approved version; nothing was written.",
        )
        .details(json!({"actual_sha256": before_sha256})));
    }
    let range = if replace {
        selection(&before, args.lines.clone(), args.text.clone())?
    } else {
        let position = if args.at_start {
            0
        } else if args.at_end {
            before.len()
        } else if let Some(line) = args.before_line {
            selection(&before, Some(line.to_string()), None)?.start
        } else if let Some(line) = args.after_line {
            selection(&before, Some(line.to_string()), None)?.end
        } else if let Some(text) = &args.before_text {
            selection(&before, None, Some(text.clone()))?.start
        } else {
            selection(&before, None, args.after_text.clone())?.end
        };
        position..position
    };
    let size = before
        .len()
        .checked_sub(range.len())
        .and_then(|n| n.checked_add(inserted.len()))
        .filter(|n| *n <= MAX_BYTES)
        .ok_or_else(|| Error::new("SIZE_LIMIT", 9, "Edited output would exceed 128 MiB."))?;
    let mut after = Vec::with_capacity(size);
    after.extend_from_slice(&before[..range.start]);
    after.extend_from_slice(&inserted);
    after.extend_from_slice(&before[range.end..]);
    let after_sha256 = digest(&after);
    if !args.dry_run && !args.plan {
        let parent = path
            .parent()
            .ok_or_else(|| Error::invalid("Target has no parent directory."))?;
        let mut temp =
            NamedTempFile::new_in(parent).map_err(|e| Error::io("Stage edited file", e))?;
        temp.write_all(&after)
            .map_err(|e| Error::io("Write edited file", e))?;
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| Error::io("Preserve target permissions", e))?;
        temp.as_file()
            .sync_all()
            .map_err(|e| Error::io("Sync edited file", e))?;
        if checked_path(&requested)? != path {
            return Err(Error::new(
                "TARGET_CHANGED",
                10,
                "Target path changed before publication.",
            ));
        }
        let (current, current_metadata) = read_target(&path)?;
        if current != before || current_metadata.permissions() != metadata.permissions() {
            return Err(Error::new(
                "TARGET_CHANGED",
                10,
                "Target changed before publication; nothing was written.",
            ));
        }
        // No in-place truncation. A failed publication leaves the old target intact.
        temp.persist(&path)
            .map_err(|e| Error::io("Publish edited file", e.error))?;
    }
    let report = json!({"ok": true, "reference": args.id, "target": path, "operation": if replace { "replace" } else { "insert" },
        "applied": !args.dry_run && !args.plan, "byte_start": range.start, "byte_end": range.end,
        "bytes_removed": range.len(), "bytes_inserted": inserted.len(), "output_bytes": after.len(),
        "before_sha256": before_sha256, "after_sha256": after_sha256, "context": context.report()});
    if args.plan {
        crate::plan::save(context, args, replace, &reference.sha256, &report)
    } else {
        Ok(report)
    }
}
