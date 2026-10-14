use crate::{
    cli::{BundleCommand, Command, SnapshotCommand},
    print_json,
};
use cpyref::{
    bundle,
    context::Context,
    error::{Error, Result},
    handoff, runner,
    snapshot::{self, Change},
    store::{write_stream_output, Metadata, RecordKind},
    verify,
};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom, Write};

fn change_report(changes: &[Change], offset: usize, limit: usize) -> Value {
    let shown: Vec<_> = changes.iter().skip(offset).take(limit).collect();
    json!({"status": if changes.is_empty() { "unchanged" } else { "changed" }, "changed_count": changes.len(),
        "counts": {"added": changes.iter().filter(|c| c.status == "added").count(),
            "removed": changes.iter().filter(|c| c.status == "removed").count(), "modified": changes.iter().filter(|c| c.status == "modified").count()},
        "changes": shown, "offset": offset, "omitted": changes.len().saturating_sub(offset).saturating_sub(shown.len()),
        "semantic_position_verified": false})
}

fn show_bundle(context: &Context, id: &str, offset: usize, limit: usize) -> Result<()> {
    let manifest = bundle::load(&context.store()?, id)?;
    let items: Vec<_> = manifest
        .items
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(name, item)| {
            json!({"name": name, "reference": item.reference, "kind": item.kind,
            "bytes": item.bytes, "sha256": item.sha256})
        })
        .collect();
    print_json(
        &json!({"reference": id, "kind": "bundle", "item_count": manifest.items.len(),
        "items": items, "offset": offset, "omitted": manifest.items.len().saturating_sub(offset).saturating_sub(items.len())}),
    )
}

/// Receiving never executes stored commands or applies patches.
pub fn receive_record(
    context: &Context,
    metadata: Metadata,
    output: Option<std::path::PathBuf>,
    force: bool,
) -> Result<u8> {
    if metadata.kind == RecordKind::Bytes {
        let (mut file, actual) = context.store()?.verified_file(&metadata.id)?;
        if actual.sha256 != metadata.sha256 || actual.kind != metadata.kind {
            return Err(Error::new(
                "HASH_MISMATCH",
                7,
                "Record changed before restoration.",
            ));
        }
        if let Some(path) = output {
            write_stream_output(&path, &mut file, force)?;
        } else {
            std::io::copy(&mut file, &mut std::io::stdout().lock())
                .map_err(|e| Error::io("Write received bytes", e))?;
        }
    } else {
        if output.is_some() {
            return Err(Error::invalid("--output is only for byte references; select a byte item or use the typed export command."));
        }
        match metadata.kind {
            RecordKind::Bundle => show_bundle(context, &metadata.id, 0, 20)?,
            RecordKind::Run => {
                let loaded = runner::load(&context.store()?, &metadata.id)?;
                print_json(&runner::summary(&metadata.id, &loaded.record))?;
            }
            RecordKind::Snapshot => {
                let manifest = snapshot::load(&context.store()?, &metadata.id)?;
                print_json(
                    &json!({"reference": metadata.id, "kind": "snapshot", "root": manifest.root,
                    "file_count": manifest.files.len(), "content_saved": false, "sha256": metadata.sha256}),
                )?;
            }
            RecordKind::Bytes => unreachable!(),
        }
    }
    Ok(0)
}

pub fn execute(command: Command, context: &Context) -> Result<u8> {
    let store = context.store()?;
    match command {
        Command::Bundle { command } => match command {
            BundleCommand::Create {
                items,
                json: as_json,
            } => {
                let metadata = bundle::create(context, &items)?;
                if as_json {
                    print_json(&json!({"reference": metadata, "context": context.report()}))?;
                } else {
                    writeln!(std::io::stdout().lock(), "{}", metadata.id)
                        .map_err(|e| Error::io("Write bundle reference", e))?;
                }
                Ok(0)
            }
            BundleCommand::Show { id, offset, limit } => {
                show_bundle(context, &id, offset, limit as usize)?;
                Ok(0)
            }
            BundleCommand::Read {
                id,
                name,
                offset,
                max_bytes,
                output,
                force,
            } => {
                let metadata = bundle::item(&store, &id, &name)?;
                if metadata.kind != RecordKind::Bytes {
                    if output.is_some() || offset != 0 || max_bytes != 4096 {
                        return Err(Error::invalid("Typed items use their bounded summary; use logs/snapshot/bundle commands for additional views."));
                    }
                    return receive_record(context, metadata, None, false);
                }
                let (mut file, actual) = store.verified_file(&metadata.id)?;
                if actual.sha256 != metadata.sha256 || actual.kind != metadata.kind {
                    return Err(Error::new(
                        "HASH_MISMATCH",
                        7,
                        "Bundle item changed before reading.",
                    ));
                }
                if let Some(path) = output {
                    write_stream_output(&path, &mut file, force)?;
                    print_json(
                        &json!({"reference": metadata.id, "item": name, "output": path, "bytes": metadata.bytes, "sha256": metadata.sha256, "complete": true}),
                    )?;
                } else {
                    let offset = offset.min(metadata.bytes as u64);
                    file.seek(SeekFrom::Current(offset as i64))
                        .map_err(|e| Error::io("Seek bundle item", e))?;
                    let mut bytes = Vec::new();
                    file.take(u64::from(max_bytes))
                        .read_to_end(&mut bytes)
                        .map_err(|e| Error::io("Read bundle item", e))?;
                    print_json(
                        &json!({"reference": metadata.id, "item": name, "sha256": metadata.sha256,
                        "offset": offset, "bytes": bytes.len(), "total_bytes": metadata.bytes,
                        "next_offset": offset + bytes.len() as u64, "truncated": offset + (bytes.len() as u64) < metadata.bytes as u64,
                        "encoding": "utf-8-lossy", "content": String::from_utf8_lossy(&bytes)}),
                    )?;
                }
                Ok(0)
            }
        },
        Command::Run(args) => {
            let options = runner::RunOptions {
                command: args.command,
                parser: args.parser,
                snapshot: args.snapshot,
                inputs: args.input,
                artifacts: args.artifact,
                inherit_stdin: args.inherit_stdin,
                redact_env: args.redact_env,
                limits: runner::RunConfig {
                    max_log_bytes: args.max_log_bytes,
                    timeout_seconds: args.timeout,
                    min_free_bytes: args.min_free_bytes,
                }
                .merge(context.run.clone())
                .resolve()?,
            };
            let (report, exit) = runner::execute(context, options)?;
            print_json(&report)?;
            Ok(exit)
        }
        Command::Verify { id } => {
            let (report, original) = verify::verify(&store, &id)?;
            print_json(&report)?;
            Ok(if original { 0 } else { 10 })
        }
        Command::Handoff { id, format } => {
            let executable =
                std::env::current_exe().map_err(|e| Error::io("Resolve current executable", e))?;
            let report = handoff::handoff(context, &id, &format, &executable)?;
            if format == "json" {
                print_json(&report)?;
            } else {
                writeln!(
                    std::io::stdout().lock(),
                    "{}",
                    report[if format == "ref" { "token" } else { "command" }]
                        .as_str()
                        .unwrap_or_default()
                )
                .map_err(|e| Error::io("Write handoff command", e))?;
            }
            Ok(0)
        }
        Command::Snapshot(args) => match args.command {
            None => {
                let manifest = snapshot::capture(context, &args.paths)?;
                let metadata = snapshot::save(&store, &manifest, context.max_entries)?;
                print_json(
                    &json!({"reference": metadata.id, "kind": "snapshot", "root": manifest.root,
                    "file_count": manifest.files.len(), "source_bytes": manifest.files.values().map(|e| e.bytes).sum::<u64>(),
                    "content_saved": false, "dependency_trace": false}),
                )?;
                Ok(0)
            }
            Some(SnapshotCommand::Check { id, limit, offset }) => {
                let manifest = snapshot::load(&store, &id)?;
                let changes = snapshot::check(&manifest)?;
                let mut report = change_report(&changes, offset, limit as usize);
                report["reference"] = json!(id);
                print_json(&report)?;
                Ok(if changes.is_empty() { 0 } else { 10 })
            }
            Some(SnapshotCommand::Diff {
                before,
                after,
                limit,
                offset,
            }) => {
                let changes = snapshot::changes(
                    &snapshot::load(&store, &before)?,
                    &snapshot::load(&store, &after)?,
                )?;
                let mut report = change_report(&changes, offset, limit as usize);
                report["before"] = json!(before);
                report["after"] = json!(after);
                print_json(&report)?;
                Ok(if changes.is_empty() { 0 } else { 10 })
            }
            Some(SnapshotCommand::Show { id, limit, offset }) => {
                let manifest = snapshot::load(&store, &id)?;
                let files: Vec<_> = manifest.files.iter().skip(offset).take(limit as usize).map(|(path, entry)| json!({"path": path, "bytes": entry.bytes, "sha256": entry.sha256})).collect();
                print_json(
                    &json!({"reference": id, "kind": "snapshot", "root": manifest.root,
                    "file_count": manifest.files.len(), "selections": manifest.selections, "excluded": manifest.excluded,
                    "files": files, "offset": offset, "omitted": manifest.files.len().saturating_sub(offset).saturating_sub(files.len()),
                    "content_saved": false, "dependency_trace": false}),
                )?;
                Ok(0)
            }
        },
        Command::Logs {
            id,
            view,
            failed,
            check,
            stream,
            offset,
            limit,
            max_bytes,
            output,
            force,
        } => {
            let mut loaded = runner::load(&store, &id)?;
            if check {
                let (report, exit) = runner::check(&id, &loaded.record, offset, limit as usize);
                print_json(&report)?;
                return Ok(exit);
            } else if failed {
                let analysis =
                    runner::failed(&mut loaded, offset, limit as usize, max_bytes as usize)?;
                print_json(
                    &json!({"reference": id, "analysis": analysis, "offset": offset, "logs_complete": loaded.record.logs_complete}),
                )?;
            } else if let Some(stream) = stream {
                let (start, length) = runner::stream_range(&loaded, &stream)?;
                if let Some(path) = output {
                    if offset != 0 {
                        return Err(Error::invalid("Full log export cannot use --offset."));
                    }
                    loaded
                        .file
                        .seek(SeekFrom::Start(start))
                        .map_err(|e| Error::io("Seek log stream", e))?;
                    write_stream_output(&path, &mut loaded.file.take(length), force)?;
                    print_json(
                        &json!({"reference": id, "stream": stream, "output": path, "bytes": length, "complete": true}),
                    )?;
                } else {
                    let offset = offset.min(length);
                    loaded
                        .file
                        .seek(SeekFrom::Start(start + offset))
                        .map_err(|e| Error::io("Seek log excerpt", e))?;
                    let mut bytes = Vec::new();
                    loaded
                        .file
                        .take((length - offset).min(max_bytes as u64))
                        .read_to_end(&mut bytes)
                        .map_err(|e| Error::io("Read log excerpt", e))?;
                    print_json(
                        &json!({"reference": id, "stream": stream, "offset": offset, "bytes": bytes.len(),
                        "total_bytes": length, "next_offset": offset + bytes.len() as u64,
                        "truncated": offset + (bytes.len() as u64) < length, "encoding": "utf-8-lossy",
                        "content": String::from_utf8_lossy(&bytes)}),
                    )?;
                }
            } else if view == "inputs" {
                let manifest = loaded
                    .record
                    .source_before
                    .as_ref()
                    .ok_or_else(|| Error::invalid("This run has no recorded source inputs."))?;
                let skip = usize::try_from(offset).unwrap_or(usize::MAX);
                let files: Vec<_> = manifest.files.iter().skip(skip).take(limit as usize).map(|(path, entry)| json!({"path": path, "bytes": entry.bytes, "sha256": entry.sha256})).collect();
                print_json(
                    &json!({"reference": id, "root": manifest.root, "file_count": manifest.files.len(), "files": files,
                    "offset": offset, "omitted": manifest.files.len().saturating_sub(skip).saturating_sub(files.len()), "dependency_trace": false}),
                )?;
            } else if view == "artifacts" {
                let skip = usize::try_from(offset).unwrap_or(usize::MAX);
                let artifacts: Vec<_> = loaded
                    .record
                    .artifacts
                    .iter()
                    .skip(skip)
                    .take(limit as usize)
                    .collect();
                print_json(
                    &json!({"reference": id, "artifacts": artifacts, "artifact_count": loaded.record.artifacts.len(),
                    "offset": offset, "omitted": loaded.record.artifacts.len().saturating_sub(skip).saturating_sub(artifacts.len()),
                    "relationship": loaded.record.artifact_relationship}),
                )?;
            } else if view == "command" {
                let skip = usize::try_from(offset).unwrap_or(usize::MAX);
                let mut remaining = max_bytes as usize;
                let mut truncated = false;
                let argv: Vec<_> = loaded.record.command.iter().enumerate().skip(skip).take(limit as usize).map(|(index, arg)| {
                    let n = arg.len().min(remaining); remaining -= n;
                    let cut = n < arg.len(); truncated |= cut;
                    json!({"index": index, "value": String::from_utf8_lossy(&arg.as_bytes()[..n]), "truncated": cut})
                }).collect();
                print_json(
                    &json!({"reference": id, "argv": argv, "argument_count": loaded.record.command.len(), "cwd": loaded.record.cwd,
                    "started_at": loaded.record.started_at, "finished_at": loaded.record.finished_at, "offset": offset, "omitted": loaded.record.command.len().saturating_sub(skip).saturating_sub(argv.len()), "truncated": truncated}),
                )?;
            } else {
                print_json(&runner::summary(&id, &loaded.record))?;
            }
            Ok(0)
        }
        _ => Err(Error::new(
            "INTERNAL_ERROR",
            5,
            "Legacy command reached workflow dispatcher.",
        )),
    }
}
