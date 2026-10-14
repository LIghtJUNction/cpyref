use crate::{
    cli::{Command, SnapshotCommand},
    print_json,
};
use cpyref::{
    context::Context,
    error::{Error, Result},
    handoff, runner,
    snapshot::{self, Change},
    store::write_stream_output,
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

pub fn execute(command: Command, context: &Context) -> Result<u8> {
    let store = context.store()?;
    match command {
        Command::Run(args) => {
            let options = runner::RunOptions {
                command: args.command,
                parser: args.parser,
                snapshot: args.snapshot,
                inputs: args.input,
                artifacts: args.artifact,
                inherit_stdin: args.inherit_stdin,
                limits: args.limits,
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
                    report["command"].as_str().unwrap_or_default()
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
