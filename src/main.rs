mod cli;
mod workflows;

use clap::{error::ErrorKind, Parser};
use cli::{Cli, Command};
use cpyref::{
    context::Context,
    error::{Error, Result},
    select::preview,
    store::{write_output, Metadata},
    MAX_BYTES,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    fs::File,
    io::{self, IsTerminal, Read, Write},
    path::Path,
    process::ExitCode,
};

fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if limit == 0 || limit > MAX_BYTES {
        return Err(Error::invalid(
            "--max-bytes must be between 1 and 134217728.",
        ));
    }
    let reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(io::stdin())
    } else {
        let metadata = std::fs::metadata(path).map_err(|e| Error::io("Read input metadata", e))?;
        if !metadata.is_file() {
            return Err(Error::new(
                "UNSUPPORTED_SOURCE",
                4,
                "Input must be a regular file or - for stdin.",
            ));
        }
        Box::new(File::open(path).map_err(|e| Error::io("Open input file", e))?)
    };
    let mut data = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| Error::io("Read input", e))?;
    if data.len() > limit {
        return Err(Error::new(
            "SIZE_LIMIT",
            9,
            "Input exceeds --max-bytes; select a smaller input or raise the limit.",
        )
        .details(json!({"limit": limit})));
    }
    Ok(data)
}

fn print_json(value: &impl Serialize) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|e| Error::new("IO_ERROR", 5, e.to_string()))?;
    writeln!(out).map_err(|e| Error::io("Write stdout", e))
}

fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| {
            Error::new(
                "CLOCK_ERROR",
                5,
                "The system clock is before the Unix epoch.",
            )
        })
}

fn matches_filter(metadata: &Metadata, filter: &cli::FilterArgs) -> bool {
    filter
        .source
        .as_ref()
        .is_none_or(|s| metadata.source.contains(s))
        && filter
            .note
            .as_ref()
            .is_none_or(|s| metadata.note.as_deref().unwrap_or("").contains(s))
}

fn execute(command: Command, context: &Context) -> Result<()> {
    let store = context.store()?;
    match command {
        Command::Insert(args) => print_json(&cpyref::edit::execute(context, &args, false)?),
        Command::Replace(args) => print_json(&cpyref::edit::execute(context, &args, true)?),
        Command::Copy(args) => {
            let source_path = match args.source.as_deref() {
                Some(path) => path,
                None if !io::stdin().is_terminal() => Path::new("-"),
                None => return Err(Error::invalid("Provide an input file or pipe bytes into cpyref copy. Use copy - for explicit interactive stdin.")),
            };
            let data = read_input(source_path, args.max_bytes)?;
            let preview = preview(&data, &args.spec())?;
            if args.dry_run {
                let span = &preview.selected;
                let mut report = json!({"ok": true, "dry_run": true, "match_count": preview.match_count,
                    "candidates": preview.candidates, "truncated": preview.truncated,
                    "selected": span, "bytes": span.byte_end - span.byte_start});
                if args.show_content {
                    let bytes = &data[span.range()];
                    let shown = bytes.len().min(args.preview_bytes as usize);
                    report["content"] = json!(String::from_utf8_lossy(&bytes[..shown]));
                    report["content_bytes"] = json!(shown);
                    report["content_truncated"] = json!(shown < bytes.len());
                    report["content_encoding"] = json!("utf-8-lossy");
                }
                return print_json(&report);
            }
            let span = preview.selected;
            let range = span.range();
            let source = if source_path == Path::new("-") {
                "<stdin>".to_owned()
            } else {
                // The selected project does not change where relative input paths are opened.
                context.cwd.join(source_path).to_string_lossy().into_owned()
            };
            let metadata = store.put_with_options(
                &data[range],
                source,
                span,
                args.note,
                context.max_entries,
            )?;
            if args.json {
                print_json(&json!({"ok": true, "reference": metadata, "context": context.report()}))
            } else {
                writeln!(io::stdout().lock(), "{}", metadata.id)
                    .map_err(|e| Error::io("Write reference", e))
            }
        }
        Command::Paste { id, output, force } => {
            if store.info(&id)?.kind == cpyref::store::RecordKind::Run {
                return Err(Error::new(
                    "WRONG_KIND",
                    4,
                    "Use logs REF --stream stdout/stderr to retrieve a run's logs.",
                ));
            }
            let (_, data) = store.get(&id)?;
            if let Some(path) = output {
                write_output(&path, &data, force)
            } else {
                io::stdout()
                    .lock()
                    .write_all(&data)
                    .map_err(|e| Error::io("Write stdout", e))
            }
        }
        Command::Info { id } => print_json(&store.info(&id)?),
        Command::Remove { id } => store.remove(&id),
        Command::Context => print_json(&context.report()),
        Command::List {
            filter,
            limit,
            offset,
            json: as_json,
        } => {
            let entries = store.list()?;
            let entries: Vec<_> = entries
                .into_iter()
                .filter(|m| matches_filter(m, &filter))
                .collect();
            let total = entries.len();
            let shown: Vec<_> = entries
                .into_iter()
                .skip(offset)
                .take(limit as usize)
                .collect();
            if as_json {
                print_json(
                    &json!({"references": shown, "total": total, "offset": offset, "limit": limit, "context": context.report()}),
                )
            } else {
                let mut out = io::stdout().lock();
                writeln!(
                    out,
                    "REF\tSOURCE\tLINES\tBYTE_RANGE\tBYTES\tCREATED_UNIX\tNOTE"
                )
                .map_err(|e| Error::io("Write list", e))?;
                for m in shown {
                    // JSON-escape untrusted fields so notes/paths cannot inject rows or terminal controls.
                    writeln!(
                        out,
                        "{}\t{}\t{}:{}\t{}:{}\t{}\t{}\t{}",
                        m.id,
                        json!(m.source),
                        m.span.line_start,
                        m.span.line_end,
                        m.span.byte_start,
                        m.span.byte_end,
                        m.bytes,
                        m.created_at,
                        json!(m.note.unwrap_or_default())
                    )
                    .map_err(|e| Error::io("Write list", e))?;
                }
                Ok(())
            }
        }
        Command::Search {
            query,
            metadata_only,
            ignore_case,
            kind,
            filter,
            limit,
            offset,
            json: as_json,
        } => {
            let matcher = cpyref::search::Matcher::new(&query, ignore_case)?;
            let entries = store
                .list()?
                .into_iter()
                .filter(|m| matches_filter(m, &filter) && kind.is_none_or(|k| m.kind == k));
            let hits = cpyref::search::search(&store, entries, &matcher, metadata_only)?;
            let total = hits.len();
            let shown: Vec<_> = hits.into_iter().skip(offset).take(limit as usize).collect();
            if as_json {
                print_json(
                    &json!({"references": shown, "total": total, "offset": offset, "limit": limit,
                    "metadata_only": metadata_only, "payload_kinds": ["bytes"], "context": context.report()}),
                )
            } else {
                let mut out = io::stdout().lock();
                writeln!(
                    out,
                    "REF\tKIND\tSOURCE\tLINES\tBYTE_RANGE\tBYTES\tCREATED_UNIX\tNOTE\tMATCHED"
                )
                .map_err(|e| Error::io("Write search results", e))?;
                for hit in shown {
                    let m = hit.metadata;
                    writeln!(
                        out,
                        "{}\t{}\t{}\t{}:{}\t{}:{}\t{}\t{}\t{}\t{}",
                        m.id,
                        cpyref::search::kind_name(m.kind),
                        json!(m.source),
                        m.span.line_start,
                        m.span.line_end,
                        m.span.byte_start,
                        m.span.byte_end,
                        m.bytes,
                        m.created_at,
                        json!(m.note.unwrap_or_default()),
                        hit.matched_in.join(",")
                    )
                    .map_err(|e| Error::io("Write search results", e))?;
                }
                Ok(())
            }
        }
        Command::Stats => {
            let entries = store.list()?;
            let now = now()?;
            let mut ages = [0_usize; 4];
            let mut stored_bytes = 0_u64;
            for m in &entries {
                let age = now.saturating_sub(m.created_at);
                ages[if age < 86400 {
                    0
                } else if age < 7 * 86400 {
                    1
                } else if age < 30 * 86400 {
                    2
                } else {
                    3
                }] += 1;
                stored_bytes += store.stored_bytes(&m.id)?;
            }
            print_json(
                &json!({"count": entries.len(), "payload_bytes": entries.iter().map(|m| m.bytes as u64).sum::<u64>(),
                "stored_bytes": stored_bytes, "largest": entries.iter().max_by_key(|m| m.bytes),
                "oldest_at": entries.iter().map(|m| m.created_at).min(), "newest_at": entries.iter().map(|m| m.created_at).max(),
                "age_buckets": {"under_1_day": ages[0], "1_to_7_days": ages[1], "7_to_30_days": ages[2], "30_days_or_more": ages[3]},
                "context": context.report()}),
            )
        }
        Command::Clean {
            ids,
            filter,
            all,
            older_than,
            keep_latest,
            apply,
            confirm,
        } => {
            if filter.source.as_deref() == Some("") || filter.note.as_deref() == Some("") {
                return Err(Error::invalid(
                    "Cleanup source/note filters cannot be empty; use explicit --all instead.",
                ));
            }
            // Explicit IDs or --all select their own set and do not inherit cleanup defaults.
            let use_defaults = ids.is_empty() && !all;
            let older_than = older_than.or(if use_defaults {
                context.cleanup.older_than_days
            } else {
                None
            });
            let keep_latest = keep_latest.or(if use_defaults {
                context.cleanup.keep_latest
            } else {
                None
            });
            let filtered = filter.source.is_some()
                || filter.note.is_some()
                || older_than.is_some()
                || keep_latest.is_some();
            if ids.is_empty() && !all && !filtered {
                return Err(Error::invalid(
                    "Cleanup requires complete IDs, filters, --keep-latest, or explicit --all.",
                ));
            }
            if !ids.is_empty() && (all || filtered) {
                return Err(Error::invalid(
                    "Cleanup IDs cannot be combined with filters or --all.",
                ));
            }
            let entries = store.list()?;
            for id in &ids {
                if !entries.iter().any(|m| &m.id == id) {
                    store.info(id)?;
                }
            }
            let now = now()?;
            let selected: Vec<_> = entries
                .into_iter()
                .enumerate()
                .filter(|(index, m)| {
                    (ids.is_empty() || ids.contains(&m.id))
                        && matches_filter(m, &filter)
                        && keep_latest.is_none_or(|keep| *index >= keep)
                        && older_than.is_none_or(|days| {
                            now.saturating_sub(m.created_at) > u64::from(days) * 86400
                        })
                })
                .map(|(_, m)| m)
                .collect();
            store.check_removal(&selected)?;
            // Bind the preview to this exact space and record set, not merely a count or an age rule.
            let encoded =
                serde_json::to_vec(&(context.store_path.as_os_str().as_encoded_bytes(), &selected))
                    .map_err(|e| Error::invalid(e.to_string()))?;
            let token = format!("{:x}", Sha256::digest(encoded));
            if apply {
                if confirm.as_deref() != Some(&token) {
                    return Err(Error::new(
                        "PLAN_CHANGED",
                        4,
                        "Cleanup confirmation does not match this preview; preview again.",
                    ));
                }
                store.remove_batch(&selected)?;
            }
            print_json(&json!({"applied": apply, "count": selected.len(),
                "payload_bytes": selected.iter().map(|m| m.bytes as u64).sum::<u64>(),
                "references": selected, "confirm": token, "context": context.report()}))
        }
        _ => Err(Error::new(
            "INTERNAL_ERROR",
            5,
            "Workflow command reached legacy dispatcher.",
        )),
    }
}

fn run(cli: Cli) -> Result<u8> {
    if let Command::Receive {
        token,
        item,
        output,
        force,
    } = cli.command
    {
        let (context, mut metadata) = cpyref::handoff::receive(&token)?;
        if let Some(name) = item {
            metadata = cpyref::bundle::item(&context.store()?, &metadata.id, &name)
                .map_err(|e| context.annotate(e))?;
        }
        return workflows::receive_record(&context, metadata, output, force)
            .map_err(|e| context.annotate(e));
    }
    let context = Context::resolve(&cli.context)?;
    let result = match cli.command {
        command @ (Command::Run(_)
        | Command::Logs { .. }
        | Command::Snapshot(_)
        | Command::Verify { .. }
        | Command::Bundle { .. }
        | Command::Handoff { .. }) => workflows::execute(command, &context),
        command => execute(command, &context).map(|_| 0),
    };
    result.map_err(|error| context.annotate(error))
}

fn report(error: Error) -> ExitCode {
    let exit = error.exit;
    // Never mix diagnostics with copied bytes or the reference on stdout.
    let _ = writeln!(
        io::stderr().lock(),
        "{}",
        json!({"ok": false, "error": error})
    );
    ExitCode::from(exit)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return match error.print() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => report(Error::io("Write help", e)),
            };
        }
        Err(error) => return report(Error::invalid(error.to_string())),
    };
    match run(cli) {
        Ok(exit) => ExitCode::from(exit),
        Err(error) => report(error),
    }
}
