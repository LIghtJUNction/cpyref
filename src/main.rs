mod cli;

use clap::{error::ErrorKind, Parser};
use cli::{Cli, Command};
use cpyref::{error::{Error, Result}, select::select, store::{write_output, Store}, MAX_BYTES};
use serde::Serialize;
use serde_json::json;
use std::{fs::File, io::{self, Read, Write}, path::Path, process::ExitCode};

fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>> {
    if limit == 0 || limit > MAX_BYTES {
        return Err(Error::invalid("--max-bytes must be between 1 and 134217728."));
    }
    let reader: Box<dyn Read> = if path == Path::new("-") {
        Box::new(io::stdin())
    } else {
        let metadata = std::fs::metadata(path).map_err(|e| Error::io("Read input metadata", e))?;
        if !metadata.is_file() {
            return Err(Error::new("UNSUPPORTED_SOURCE", 4, "Input must be a regular file or - for stdin."));
        }
        Box::new(File::open(path).map_err(|e| Error::io("Open input file", e))?)
    };
    let mut data = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut data).map_err(|e| Error::io("Read input", e))?;
    if data.len() > limit {
        return Err(Error::new("SIZE_LIMIT", 9, "Input exceeds --max-bytes; select a smaller input or raise the limit.")
            .details(json!({"limit": limit})));
    }
    Ok(data)
}

fn print_json(value: &impl Serialize) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|e| Error::new("IO_ERROR", 5, e.to_string()))?;
    writeln!(out).map_err(|e| Error::io("Write stdout", e))
}

fn run(cli: Cli) -> Result<()> {
    let root = match cli.store {
        Some(path) => path,
        None => Store::default_root()?,
    };
    let store = Store::new(root);
    match cli.command {
        Command::Copy(args) => {
            let data = read_input(&args.source, args.max_bytes)?;
            let span = select(&data, &args.spec())?;
            let range = span.range();
            let source = if args.source == Path::new("-") {
                "<stdin>".to_owned()
            } else {
                // Record the path used to open the file, not a later re-resolved symlink target.
                std::env::current_dir().map_err(|e| Error::io("Read current directory", e))?
                    .join(&args.source).to_string_lossy().into_owned()
            };
            let metadata = store.put(&data[range], source, span)?;
            if args.json {
                print_json(&json!({"ok": true, "reference": metadata}))
            } else {
                writeln!(io::stdout().lock(), "{}", metadata.id).map_err(|e| Error::io("Write reference", e))
            }
        }
        Command::Paste { id, output, force } => {
            let (_, data) = store.get(&id)?;
            if let Some(path) = output {
                write_output(&path, &data, force)
            } else {
                io::stdout().lock().write_all(&data).map_err(|e| Error::io("Write stdout", e))
            }
        }
        Command::Info { id } => print_json(&store.info(&id)?),
        Command::Remove { id } => store.remove(&id),
    }
}

fn report(error: Error) -> ExitCode {
    let exit = error.exit;
    // Never mix diagnostics with copied bytes or the reference on stdout.
    let _ = writeln!(io::stderr().lock(), "{}", json!({"ok": false, "error": error}));
    ExitCode::from(exit)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if matches!(error.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            return match error.print() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => report(Error::io("Write help", e)),
            };
        }
        Err(error) => return report(Error::invalid(error.to_string())),
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(error),
    }
}
